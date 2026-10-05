use std::fs;
use std::path::{Path, PathBuf};

use handwriter_core::drawing::pipeline::{compose_drawing, gcode_filename, make_all_files, preview_payload};
use handwriter_core::drawing::sources::load_drawing;
use handwriter_core::settings::{DrawingImport, Settings};
use serde_json::{Value, json};

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden")
}

fn first_diff(path: &str, a: &Value, b: &Value) -> Option<String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => (x.as_f64().unwrap().to_bits() != y.as_f64().unwrap().to_bits())
            .then(|| format!("{path}: rust {a} vs python {b}")),
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{path}: length {} vs {}", x.len(), y.len()));
            }
            x.iter().zip(y).enumerate().find_map(|(i, (p, q))| first_diff(&format!("{path}[{i}]"), p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x {
                match y.get(k) {
                    None => return Some(format!("{path}.{k}: missing in python")),
                    Some(w) => {
                        if let Some(d) = first_diff(&format!("{path}.{k}"), v, w) {
                            return Some(d);
                        }
                    }
                }
            }
            y.keys().find(|k| !x.contains_key(*k)).map(|k| format!("{path}.{k}: missing in rust"))
        }
        _ => (a != b).then(|| format!("{path}: rust {a} vs python {b}")),
    }
}

fn gcode_diff(a: &str, b: &str) -> String {
    for (i, (x, y)) in a.lines().zip(b.lines()).enumerate() {
        if x != y {
            return format!("line {}: rust `{x}` python `{y}`", i + 1);
        }
    }
    format!("line count {} vs {}", a.lines().count(), b.lines().count())
}

#[test]
fn drawing_pipeline_matches_python() {
    let dir = golden().join("drawing_pipeline");
    let input = dir.join("input");
    let loader = |spec: &str, imp: &DrawingImport, tol: f64| load_drawing(spec, imp, tol, &input);
    let cases: Value = serde_json::from_str(&fs::read_to_string(dir.join("cases.json")).unwrap()).unwrap();
    let mut failures = Vec::new();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let s = Settings::from_json(&case["settings"].to_string()).unwrap();
        let c = compose_drawing(&s, &loader);
        if let Some(d) = first_diff("preview", &preview_payload(&c), &case["preview"]) {
            failures.push(format!("{name}: {d}"));
            continue;
        }
        let extra = json!({"frame_fitted": c.frame_fitted, "dashed_solid": c.dashed_solid,
                           "source_strokes": c.source_strokes.len()});
        for k in ["frame_fitted", "dashed_solid", "source_strokes"] {
            if extra[k] != case[k] {
                failures.push(format!("{name}: {k} {} vs {}", extra[k], case[k]));
            }
        }
        match make_all_files(&c, true) {
            Ok(files) => {
                let want = case["files"].as_array().map(|v| v.len()).unwrap_or(0);
                if files.len() != want {
                    failures.push(format!("{name}: {} files vs {want}", files.len()));
                    continue;
                }
                if json!(gcode_filename(&c)) != case["gcode_filename"] {
                    failures.push(format!("{name}: gcode filename {}", gcode_filename(&c)));
                }
                for (f, w) in files.iter().zip(case["files"].as_array().unwrap()) {
                    let wg = w["gcode"].as_str().unwrap();
                    if f.filename != w["filename"].as_str().unwrap()
                        || f.gcode != wg
                        || f.test != w["test"].as_bool().unwrap()
                    {
                        failures.push(format!("{name}: {} {}", f.filename, gcode_diff(&f.gcode, wg)));
                    }
                }
            }
            Err(e) => {
                if json!(e) != case["refused"] {
                    failures.push(format!("{name}: refused {e:?} vs {}", case["refused"]));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn original_drawing_golden_files_match() {
    let mut checked = 0;
    for case in [
        "drawing_builtin_a4",
        "drawing_builtin_frame_weights",
        "drawing_marked_a4",
        "drawing_a3_four_runs",
        "drawing_split_areas",
        "drawing_svg",
    ] {
        let dir = golden().join(case);
        let input = dir.join("input");
        let loader = |spec: &str, imp: &DrawingImport, tol: f64| load_drawing(spec, imp, tol, &input);
        let s = Settings::from_json(&fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap();
        let c = compose_drawing(&s, &loader);
        let files = make_all_files(&c, true).unwrap_or_else(|e| panic!("{case}: {e:?}"));
        let recorded: Vec<Value> = serde_json::from_str(&fs::read_to_string(dir.join("files.json")).unwrap()).unwrap();
        let tests = recorded.iter().any(|f| f["filename"].as_str().unwrap().ends_with("_test.gcode"));
        let files: Vec<_> = files.into_iter().filter(|f| tests || !f.test).collect();
        assert_eq!(files.len(), recorded.len(), "{case}");
        for f in files {
            let want = fs::read_to_string(dir.join(&f.filename)).unwrap_or_else(|_| panic!("{case}: {}", f.filename));
            assert!(f.gcode == want, "{case}/{}: {}", f.filename, gcode_diff(&f.gcode, &want));
            checked += 1;
        }
    }
    assert!(checked >= 18, "{checked}");
}
