use std::fs;
use std::path::Path;

use handwriter_core::checks::{check_bounds, check_settings};
use handwriter_core::geometry::{Point, make_transform, polyline_length, rdp, rdp_indices};
use handwriter_core::settings::Settings;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct CheckCase {
    settings: Value,
    strokes: Vec<Vec<Point>>,
    errors: Vec<String>,
    warnings: Vec<String>,
    bounds: Vec<String>,
    bounds2: Vec<String>,
}

#[derive(Deserialize)]
struct PolylineCase {
    points: Vec<Point>,
    tol: f64,
    must_keep: Vec<usize>,
    rdp: Vec<Point>,
    indices: Vec<usize>,
    length: f64,
}

#[derive(Deserialize)]
struct GeometryCases {
    transforms: Vec<(f64, f64, f64, Point, Point)>,
    polylines: Vec<PolylineCase>,
}

#[derive(Deserialize)]
struct Conformance {
    constraints: Vec<(String, Value, bool)>,
    checks: Vec<CheckCase>,
    geometry: GeometryCases,
}

fn load() -> Conformance {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden/conformance.json");
    serde_json::from_str(&fs::read_to_string(path).expect("run python tools/make_golden.py")).unwrap()
}

fn settings_with_flags(v: &Value) -> Settings {
    let mut s = Settings::from_json(&v.to_string()).unwrap();
    s.printer.use_work_area = v["printer"]["_use_work_area"].as_bool().unwrap_or(false);
    s
}

#[test]
fn field_constraints_match_pydantic() {
    let c = load();
    assert_eq!(c.constraints.len(), 285);
    let mut fields: Vec<&str> = c.constraints.iter().map(|(f, _, _)| f.as_str()).collect();
    fields.dedup();
    assert_eq!(fields.len(), 49);
    for (field, data, ok) in &c.constraints {
        let got = Settings::from_json(&data.to_string());
        assert_eq!(got.is_ok(), *ok, "{field}: {data} -> {got:?}");
    }
}

#[test]
fn checks_match_python() {
    for (i, case) in load().checks.iter().enumerate() {
        let s = settings_with_flags(&case.settings);
        let (errors, warnings) = check_settings(&s);
        assert_eq!(errors, case.errors, "case {i}");
        assert_eq!(warnings, case.warnings, "case {i}");
        assert_eq!(check_bounds(&case.strokes, &s, 5), case.bounds, "case {i}");
        assert_eq!(check_bounds(&case.strokes, &s, 2), case.bounds2, "case {i}");
    }
}

#[test]
fn geometry_matches_python() {
    let g = load().geometry;
    for (rot, dx, dy, p, want) in g.transforms {
        let got = make_transform(rot, dx, dy)(p);
        assert_eq!((got.0.to_bits(), got.1.to_bits()), (want.0.to_bits(), want.1.to_bits()), "{rot} {dx} {dy} {p:?}");
    }
    for c in g.polylines {
        assert_eq!(rdp(&c.points, c.tol), c.rdp);
        assert_eq!(rdp_indices(&c.points, c.tol, &c.must_keep), c.indices);
        assert_eq!(polyline_length(&c.points).to_bits(), c.length.to_bits());
    }
}
