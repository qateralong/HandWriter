use std::fs;
use std::path::Path;

use handwriter_core::drawing::passes;
use handwriter_core::drawing::place::Rect;
use handwriter_core::geometry::Point;
use handwriter_core::settings::{DrawingSettings, Printer};
use serde_json::{Value, json};

fn cases() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden/plan/cases.json");
    serde_json::from_str(&fs::read_to_string(path).expect("run python tools/make_golden.py")).unwrap()
}

fn first_diff(path: &str, a: &Value, b: &Value) -> Option<String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => (x.as_f64().unwrap().to_bits() != y.as_f64().unwrap().to_bits())
            .then(|| format!("{path}: rust {a} vs python {b}")),
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{path}: rust {a} vs python {b}"));
            }
            x.iter().zip(y).enumerate().find_map(|(i, (p, q))| first_diff(&format!("{path}[{i}]"), p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            if x.len() != y.len() {
                return Some(format!("{path}: keys differ"));
            }
            x.iter().find_map(|(k, v)| {
                y.get(k).map_or(Some(format!("{path}.{k}: missing")), |w| first_diff(&format!("{path}.{k}"), v, w))
            })
        }
        _ => (a != b).then(|| format!("{path}: rust {a} vs python {b}")),
    }
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

fn pt(v: &Value) -> Point {
    (f(&v[0]), f(&v[1]))
}

fn rect(v: &Value) -> Rect {
    (f(&v[0]), f(&v[1]), f(&v[2]), f(&v[3]))
}

fn rj(r: Rect) -> Value {
    json!([r.0, r.1, r.2, r.3])
}

fn rot_rects(rr: &passes::RotationRects) -> Value {
    json!([rr.0, rr.1.iter().map(|(r, rc)| json!([r, rj(*rc)])).collect::<Vec<_>>(), rr.2])
}

#[test]
fn pass_planning_matches_python() {
    let c = cases();
    let mut failures = Vec::new();
    let mut check = |what: String, got: Value, want: &Value| {
        if let Some(d) = first_diff(&what, &got, want) {
            failures.push(d);
        }
    };
    for (i, x) in c["covered"].as_array().unwrap().iter().enumerate() {
        let strokes: Vec<Vec<Point>> =
            x[0].as_array().unwrap().iter().map(|s| s.as_array().unwrap().iter().map(pt).collect()).collect();
        let rects: Vec<Rect> = x[1].as_array().unwrap().iter().map(rect).collect();
        let mask = passes::covered_mask(&passes::segments(&strokes), &rects);
        check(format!("covered[{i}]"), json!([x[0], x[1], mask, passes::lines_covered(&strokes, &rects)]), x);
    }
    for (i, x) in c["plans"].as_array().unwrap().iter().enumerate() {
        let pr: Printer = serde_json::from_value(x["printer"].clone()).unwrap();
        let pl = passes::plan_sheet(f(&x["W"]), f(&x["H"]), rect(&x["target"]), &pr, x["what"].as_str().unwrap());
        let got = json!({
            "allowed": pl.allowed, "rects": pl.allowed.iter().map(|r| json!([r, rj(pl.rects[r])])).collect::<Vec<_>>(),
            "rotations": pl.rotations, "uncovered": pl.uncovered.iter().map(|&r| rj(r)).collect::<Vec<_>>(),
            "message": pl.message, "notes": pl.notes, "ok": pl.ok(), "describe": pl.describe(None),
            "describe_all": pl.describe(Some(&pl.allowed)),
        });
        check(format!("plan[{i}]"), got, &x["plan"]);
    }
    for (i, x) in c["marked"].as_array().unwrap().iter().enumerate() {
        let ds: DrawingSettings = serde_json::from_value(x["drawing"].clone()).unwrap();
        let (w, h) = (f(&x["W"]), f(&x["H"]));
        let pts: Vec<Point> = x["pts"].as_array().unwrap().iter().map(pt).collect();
        let aff: Vec<Value> = [0, 180]
            .iter()
            .map(|&r| {
                let m = passes::marked_affine(&ds.marked, w, h, r);
                let ap: Vec<Value> = pts
                    .iter()
                    .map(|&q| {
                        let a = passes::apply_affine(m, q);
                        json!([a.0, a.1])
                    })
                    .collect();
                let iv: Vec<Value> = pts
                    .iter()
                    .map(|&q| {
                        let a = passes::invert_affine(m, q);
                        json!([a.0, a.1])
                    })
                    .collect();
                json!([r, m, ap, iv])
            })
            .collect();
        check(format!("marked[{i}].affine"), json!(aff), &x["affine"]);
        check(format!("marked[{i}].rects"), rot_rects(&passes::marked_rects(&ds.marked, w, h)), &x["rects"]);
    }
    for (i, x) in c["a3"].as_array().unwrap().iter().enumerate() {
        let ds: DrawingSettings = serde_json::from_value(x["drawing"].clone()).unwrap();
        let (w, h) = (f(&x["W"]), f(&x["H"]));
        let pts: Vec<Point> = x["pts"].as_array().unwrap().iter().map(pt).collect();
        let aff: Vec<Value> = passes::A3_RUNS
            .iter()
            .map(|&r| {
                let m = passes::a3_affine(&ds.a3, w, h, r);
                let ap: Vec<Value> = pts
                    .iter()
                    .map(|&q| {
                        let a = passes::apply_affine(m, q);
                        json!([a.0, a.1])
                    })
                    .collect();
                json!([r, m, ap])
            })
            .collect();
        check(format!("a3[{i}].affine"), json!(aff), &x["affine"]);
        check(format!("a3[{i}].rects"), rot_rects(&passes::a3_rects(&ds.a3, w, h)), &x["rects"]);
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
}
