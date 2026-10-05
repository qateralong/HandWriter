use std::fs;
use std::path::Path;

use handwriter_core::drawing::place::Rect;
use handwriter_core::drawing::split::{Geometry, clip_segments};
use handwriter_core::geometry::Point;
use serde_json::{Value, json};

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

fn pt(v: &Value) -> Point {
    (f(&v[0]), f(&v[1]))
}

fn rect(v: &Value) -> Rect {
    (f(&v[0]), f(&v[1]), f(&v[2]), f(&v[3]))
}

fn pts(v: &[Point]) -> Value {
    json!(v.iter().map(|p| json!([p.0, p.1])).collect::<Vec<_>>())
}

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64().unwrap().to_bits() == y.as_f64().unwrap().to_bits(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        _ => a == b,
    }
}

#[test]
fn split_geometry_matches_python() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden/split/geometry.json");
    let cases: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("run python tools/make_golden.py")).unwrap();
    let mut failures = Vec::new();
    for (i, c) in cases.as_array().unwrap().iter().enumerate() {
        let strokes: Vec<Vec<Point>> =
            c["strokes"].as_array().unwrap().iter().map(|s| s.as_array().unwrap().iter().map(pt).collect()).collect();
        let geo = Geometry::new(&strokes);
        let q = rect(&c["Q"]);
        let rects: Vec<Rect> = c["rects"].as_array().unwrap().iter().map(rect).collect();
        let cands: Vec<f64> = c["cands"].as_array().unwrap().iter().map(f).collect();
        let p: Vec<Point> = c["P"].as_array().unwrap().iter().map(pt).collect();
        let (ca, cb, idx) = clip_segments(&geo.a, &geo.b, q, 1e-9);
        let checks = [
            ("A", pts(&geo.a), &c["geo"]["A"]),
            ("B", pts(&geo.b), &c["geo"]["B"]),
            ("seg_path", json!(geo.seg_path), &c["geo"]["seg_path"]),
            ("curved", json!(geo.curved), &c["geo"]["curved"]),
            ("seg_len", json!(geo.seg_len), &c["geo"]["seg_len"]),
            ("path_len", json!(geo.path_len), &c["geo"]["path_len"]),
            ("clip", json!([pts(&ca), pts(&cb), idx]), &c["clip"]),
            ("covered", json!(geo.covered(q, &rects)), &c["covered"]),
            (
                "seam_cost",
                json!(geo.seam_cost(q, c["axis"].as_u64().unwrap() as usize, &cands, f(&c["ov"]))),
                &c["seam_cost"],
            ),
            (
                "distance",
                json!(
                    geo.distance_to(&p)
                        .iter()
                        .map(|&d| if d.is_finite() { json!(d) } else { json!("inf") })
                        .collect::<Vec<_>>()
                ),
                &c["distance"],
            ),
        ];
        for (name, got, want) in checks {
            if !same(&got, want) {
                failures.push(format!("case {i} {name}:\n  rust   {got}\n  python {want}"));
            }
        }
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
}
