use std::fs;
use std::path::{Path, PathBuf};

use handwriter_core::drawing::model::ImportResult;
use handwriter_core::drawing::pipeline::{compose_drawing, make_all_files, preview_payload};
use handwriter_core::drawing::raster_import::import_raster;
use handwriter_core::drawing::sources::load_drawing;
use handwriter_core::settings::{DrawingImport, Settings};
use serde_json::{Value, json};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden/raster")
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

fn rle(r: &ImportResult) -> Value {
    let Some(m) = &r.fill_mask else { return Value::Null };
    let mut runs = Vec::new();
    let (mut cur, mut n) = (false, 0u64);
    for &v in &m.data {
        if v == cur {
            n += 1;
        } else {
            runs.push(n);
            cur = v;
            n = 1;
        }
    }
    runs.push(n);
    json!(runs)
}

fn dump(r: &ImportResult) -> Value {
    let pts = |v: &[(f64, f64)]| json!(v.iter().map(|p| json!([p.0, p.1])).collect::<Vec<_>>());
    json!({
        "kind": r.kind, "name": r.name,
        "paths": r.paths.iter().map(|p| json!({"points": pts(&p.points), "closed": p.closed, "width": p.width,
            "layer": p.layer, "dash": p.dash, "dash_offset": p.dash_offset})).collect::<Vec<_>>(),
        "texts": [], "warnings": r.warnings, "errors": r.errors, "units": r.units, "units_note": r.units_note,
        "pages": r.pages, "page": r.page, "layers": {}, "bbox": r.bbox().map(|b| json!([b.0, b.1, b.2, b.3])),
        "info": r.info, "fill_px": r.fill_px, "fill_mask": rle(r),
        "fill_bbox": r.fill_bbox().map(|b| json!([b.0, b.1, b.2, b.3])),
    })
}

#[test]
fn raster_imports_match_python() {
    let cases: Value = serde_json::from_str(&fs::read_to_string(dir().join("cases.json")).unwrap()).unwrap();
    let mut failures = Vec::new();
    for c in cases["imports"].as_array().unwrap() {
        let file = c["file"].as_str().unwrap();
        if cfg!(windows) && file.ends_with(".jpg") {
            continue;
        }
        let opts: DrawingImport = serde_json::from_value(c["opts"].clone()).unwrap();
        let data = fs::read(dir().join("input").join(file)).unwrap();
        let r = import_raster(&data, file, &opts);
        let mut want = c["result"].clone();
        if file == "broken.png" {
            assert!(r.errors[0].starts_with("Картинка не читается: "), "{:?}", r.errors);
            want["errors"] = json!(r.errors);
        }
        if let Some(d) = first_diff(&format!("{file} {}", c["opts"]), &dump(&r), &want) {
            failures.push(d);
        }
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn raster_drawings_match_python() {
    let cases: Value = serde_json::from_str(&fs::read_to_string(dir().join("cases.json")).unwrap()).unwrap();
    let input = dir().join("input");
    let loader = |spec: &str, imp: &DrawingImport, tol: f64| load_drawing(spec, imp, tol, &input);
    let mut failures = Vec::new();
    for c in cases["drawings"].as_array().unwrap() {
        if cfg!(windows) && c["file"].as_str().is_some_and(|f| f.ends_with(".jpg")) {
            continue;
        }
        let s = Settings::from_json(&c["settings"].to_string()).unwrap();
        let comp = compose_drawing(&s, &loader);
        if let Some(d) = first_diff(&format!("{} preview", c["file"]), &preview_payload(&comp), &c["preview"]) {
            failures.push(d);
            continue;
        }
        let files = make_all_files(&comp, true).unwrap();
        let want = c["files"].as_array().unwrap();
        assert_eq!(files.len(), want.len());
        for (f, w) in files.iter().zip(want) {
            if f.gcode != w["gcode"].as_str().unwrap() {
                failures.push(format!("{} {} gcode differs", c["file"], f.filename));
            }
        }
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn svg_small_fills_become_centerlines() {
    let cases: Value = serde_json::from_str(&fs::read_to_string(dir().join("cases.json")).unwrap()).unwrap();
    for c in cases["svg_fills"].as_array().unwrap() {
        let opts = DrawingImport {
            fill_centerlines: true,
            fill_centerline_max: c["max"].as_f64().unwrap(),
            ..Default::default()
        };
        let r = handwriter_core::drawing::svg_import::import_svg(
            c["svg"].as_str().unwrap().as_bytes(),
            "f.svg",
            &opts,
            0.05,
        );
        let mut got = dump(&r);
        for k in ["info", "fill_px", "fill_mask", "fill_bbox"] {
            got.as_object_mut().unwrap().remove(k);
        }
        let mut want = c["result"].clone();
        want["texts"] = json!([]);
        if let Some(d) = first_diff(&format!("max {}", c["max"]), &got, &want) {
            panic!("{d}");
        }
    }
}
