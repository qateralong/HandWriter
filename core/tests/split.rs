use std::fs;
use std::path::Path;

use handwriter_core::drawing::place::Rect;
use handwriter_core::drawing::split::{self, CutStats, Geometry, Node};
use handwriter_core::geometry::Point;
use indexmap::IndexMap;
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

fn rj(r: Rect) -> Value {
    json!([r.0, r.1, r.2, r.3])
}

fn pts(v: &[Point]) -> Value {
    json!(v.iter().map(|p| json!([p.0, p.1])).collect::<Vec<_>>())
}

fn node(n: Option<&Node>) -> Value {
    match n {
        None => Value::Null,
        Some(n) => json!({"core": rj(n.core), "ext": rj(n.ext), "rotation": n.rotation,
            "axis": if n.rotation.is_some() { Value::Null } else { json!(n.axis) }, "s": n.s, "cost": n.cost,
            "low": node(n.low.as_deref()), "high": node(n.high.as_deref())}),
    }
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
        (Value::Object(x), Value::Object(y)) => x.iter().find_map(|(k, v)| {
            y.get(k).map_or(Some(format!("{path}.{k}: missing")), |w| first_diff(&format!("{path}.{k}"), v, w))
        }),
        _ => (a != b).then(|| format!("{path}: rust {a} vs python {b}")),
    }
}

#[test]
fn split_matches_python() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden/split/split.json");
    let all: Value = serde_json::from_str(&fs::read_to_string(path).expect("run python tools/make_golden.py")).unwrap();
    let mut failures = Vec::new();
    let mut check = |what: String, got: Value, want: &Value| {
        if let Some(d) = first_diff(&what, &got, want) {
            failures.push(d);
        }
    };
    for (i, c) in all["cases"].as_array().unwrap().iter().enumerate() {
        let strokes: Vec<Vec<Point>> =
            c["strokes"].as_array().unwrap().iter().map(|s| s.as_array().unwrap().iter().map(pt).collect()).collect();
        let rects: IndexMap<i64, Rect> =
            c["rects"].as_array().unwrap().iter().map(|x| (x[0].as_i64().unwrap(), rect(&x[1]))).collect();
        let allowed: Vec<i64> = c["allowed"].as_array().unwrap().iter().map(|x| x.as_i64().unwrap()).collect();
        let (w, h) = (f(&c["W"]), f(&c["H"]));
        let geo = Geometry::new(&strokes);
        let sheet = (0.0, 0.0, w, h);
        let ov = f(&c["ov"]);
        let root = split::solve(&geo, &rects, &allowed, sheet, sheet, f(&c["margin"]), ov);
        check(format!("case {i} root"), node(root.as_ref()), &c["root"]);
        let Some(root) = root else { continue };
        let mut stats = CutStats::default();
        let cuts: Vec<Value> = strokes
            .iter()
            .map(|st| {
                json!(
                    split::cut_stroke(st, &root, ov, Some(&mut stats))
                        .iter()
                        .map(|(r, p)| json!([r, pts(p)]))
                        .collect::<Vec<_>>()
                )
            })
            .collect();
        check(format!("case {i} cuts"), json!(cuts), &c["cuts"]);
        check(
            format!("case {i} stats"),
            json!({"length_in": stats.length_in, "length_out": stats.length_out,
            "cuts": stats.cuts, "extension": stats.extension}),
            &c["stats"],
        );
        let (size, count) = (f(&c["mark_size"]), c["mark_count"].as_u64().unwrap() as usize);
        let marks: Vec<Value> = split::control_marks(&root, &geo, &rects, size, count, 1.0)
            .iter()
            .map(|m| {
                let st: Vec<Value> = split::mark_strokes(m, size).iter().map(|s| pts(s)).collect();
                json!([m.x, m.y, [m.passes.0, m.passes.1], st])
            })
            .collect();
        check(format!("case {i} marks"), json!(marks), &c["marks"]);
    }
    for (i, x) in all["extract"].as_array().unwrap().iter().enumerate() {
        let p: Vec<Point> = x[0].as_array().unwrap().iter().map(pt).collect();
        let closed = x[1].as_bool().unwrap();
        let (a, b) = (f(&x[2]), f(&x[3]));
        let cum = split::cum_lengths(&p);
        let axis = x[7].as_u64().unwrap() as usize;
        let at = split::point_at(&p, &cum, a);
        let got = json!([
            x[0],
            closed,
            a,
            b,
            cum,
            pts(&split::extract(&p, &cum, a, b, closed)),
            [at.0, at.1],
            axis,
            x[8],
            split::crossings(&p, &cum, axis, f(&x[8]))
        ]);
        check(format!("extract[{i}]"), got, x);
    }
    for x in all["bisect"].as_array().unwrap() {
        let (lo, hi, thr, want) = (f(&x[0]), f(&x[1]), f(&x[2]), x[3].as_bool().unwrap());
        let got =
            if want { split::bisect(|s| s <= thr, lo, hi, true) } else { split::bisect(|s| s >= thr, lo, hi, false) };
        check(format!("bisect {x}"), json!(got), &x[4]);
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
}
