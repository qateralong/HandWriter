use std::fs;
use std::path::{Path, PathBuf};

use handwriter_core::gcode::{compute_stats, generate_gcode};
use handwriter_core::geometry::Point;
use handwriter_core::numeric;
use handwriter_core::settings::Settings;
use serde::Deserialize;
use serde_json::Value;

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden")
}

fn cases() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(golden_dir())
        .expect("tests/golden is missing: run python tools/make_golden.py")
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("files.json").exists())
        .collect();
    v.sort();
    assert!(!v.is_empty());
    v
}

#[derive(Deserialize)]
struct StatsJson {
    draw_mm: f64,
    travel_mm: f64,
    time_s: f64,
}

#[derive(Deserialize)]
struct FileCase {
    filename: String,
    strokes: Vec<Vec<Point>>,
    settings: Value,
    header: Option<Vec<String>>,
    info: Option<Vec<String>>,
    stats: StatsJson,
}

fn first_diff(a: &str, b: &str) -> String {
    for (i, (x, y)) in a.lines().zip(b.lines()).enumerate() {
        if x != y {
            return format!("line {}:\n  rust:   {x}\n  python: {y}", i + 1);
        }
    }
    format!("line count differs: rust {}, python {}", a.lines().count(), b.lines().count())
}

fn part_settings(v: &Value) -> Settings {
    let mut s = Settings::from_json(&v.to_string()).expect("pass settings parse");
    s.printer.use_work_area = v["printer"]["_use_work_area"].as_bool().unwrap_or(false);
    s
}

#[test]
fn settings_roundtrip_matches_python() {
    for case in cases() {
        let text = fs::read_to_string(case.join("settings.json")).unwrap();
        let py: Value = serde_json::from_str(&text).unwrap();
        let s = Settings::from_json(&text).unwrap_or_else(|e| panic!("{}: {e}", case.display()));
        let mut rs: Value = serde_json::from_str(&s.to_json()).unwrap();
        rs["printer"].as_object_mut().unwrap().remove("end_lift");
        rs["drawing"].as_object_mut().unwrap().remove("test_files");
        assert_eq!(rs, py, "{}", case.display());
    }
}

#[test]
fn default_settings_match_python() {
    let text = fs::read_to_string(golden_dir().join("text_plain/settings.json")).unwrap();
    let mut py: Value = serde_json::from_str(&text).unwrap();
    py["printer"]["travel"] = Value::Null;
    py["randomness"]["enabled"] = Value::Bool(true);
    py["connections"]["enabled"] = Value::Bool(true);
    let mut rs: Value = serde_json::from_str(&Settings::default().to_json()).unwrap();
    rs["printer"].as_object_mut().unwrap().remove("end_lift");
    rs["drawing"].as_object_mut().unwrap().remove("test_files");
    assert_eq!(rs, py);
}

#[test]
fn gcode_matches_python() {
    let mut checked = 0;
    for case in cases() {
        let files: Vec<FileCase> = serde_json::from_str(&fs::read_to_string(case.join("files.json")).unwrap()).unwrap();
        for f in files {
            let s = part_settings(&f.settings);
            let name = format!("{}/{}", case.file_name().unwrap().to_string_lossy(), f.filename);
            let st = compute_stats(&f.strokes, &s);
            assert_eq!(
                (st.draw_mm, st.travel_mm, st.time_s),
                (f.stats.draw_mm, f.stats.travel_mm, f.stats.time_s),
                "{name}: stats"
            );
            let got = generate_gcode(&f.strokes, &s, f.header.as_deref().unwrap_or(&[]), f.info.as_deref());
            let want = fs::read_to_string(case.join(&f.filename)).unwrap();
            assert!(got == want, "{name}: {}", first_diff(&got, &want));
            checked += 1;
        }
    }
    assert!(checked >= 20, "files checked: {checked}");
}

#[derive(Deserialize)]
struct NumericJson {
    dist: Vec<(Point, Point, f64)>,
    sum: Vec<(Vec<f64>, f64)>,
    repr: Vec<(f64, String)>,
    fmt2: Vec<(f64, String)>,
}

#[test]
fn numeric_matches_python() {
    let n: NumericJson = serde_json::from_str(&fs::read_to_string(golden_dir().join("numeric.json")).unwrap()).unwrap();
    for (p, q, d) in n.dist {
        assert_eq!(numeric::dist(p, q).to_bits(), d.to_bits(), "dist({p:?}, {q:?})");
    }
    for (xs, s) in n.sum {
        assert_eq!(numeric::sum(xs.iter().copied()).to_bits(), s.to_bits(), "sum({xs:?})");
    }
    for (x, r) in n.repr {
        assert_eq!(numeric::repr(x), r);
    }
    for (x, r) in n.fmt2 {
        assert_eq!(format!("{x:.2}"), r, "{x:?}");
    }
}
