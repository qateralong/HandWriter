use std::fs;
use std::path::Path;

use handwriter_core::geometry::Point;
use handwriter_core::skeleton::{self, SkeletonParams};
use serde_json::{Value, json};

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

fn lines(v: &Value) -> Vec<Vec<Point>> {
    v.as_array().unwrap().iter().map(|c| c.as_array().unwrap().iter().map(|p| (f(&p[0]), f(&p[1]))).collect()).collect()
}

fn lj(v: &[Vec<Point>]) -> Value {
    json!(v.iter().map(|c| c.iter().map(|p| json!([p.0, p.1])).collect::<Vec<_>>()).collect::<Vec<_>>())
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
        _ => (a != b).then(|| format!("{path}: rust {a} vs python {b}")),
    }
}

fn rle(data: &[bool]) -> Value {
    let mut runs = Vec::new();
    let (mut cur, mut n) = (false, 0u64);
    for &v in data {
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

fn cases() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden/skeleton/cases.json");
    serde_json::from_str(&fs::read_to_string(path).expect("run python tools/make_golden.py")).unwrap()
}

#[test]
fn skeleton_primitives_match_python() {
    let c = cases();
    let mut failures = Vec::new();
    for (i, p) in c["prims"].as_array().unwrap().iter().enumerate() {
        let (w, h) = (p["w"].as_u64().unwrap() as usize, p["h"].as_u64().unwrap() as usize);
        let mask = skeleton::rasterize(&lines(&p["contours_px"]), w, h);
        let on: Vec<usize> = (0..w * h).filter(|&k| mask.data[k]).collect();
        if let Some(d) = first_diff(&format!("prim {i} mask"), &rle(&mask.data), &p["mask"]) {
            failures.push(d);
            continue;
        }
        let sk = skeleton::skeletonize(&mask);
        if let Some(d) = first_diff(&format!("prim {i} skel"), &rle(&sk.data), &p["skel"]) {
            failures.push(d);
        }
        let dt = skeleton::distance_transform(&mask);
        let dt_on: Vec<f64> = on.iter().map(|&k| dt[k]).take(4000).collect();
        if let Some(d) = first_diff(&format!("prim {i} dt"), &json!(dt_on), &p["dt_on"]) {
            failures.push(d);
        }
        if let Some(d) = first_diff(&format!("prim {i} ref"), &json!(skeleton::reference_px(&mask)), &p["reference_px"])
        {
            failures.push(d);
        }
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn skeleton_strokes_match_python() {
    let c = cases();
    let mut failures = Vec::new();
    for g in c["glyphs"].as_array().unwrap() {
        let name = g["name"].as_str().unwrap();
        let pv = &g["params"];
        let params = SkeletonParams {
            px_per_em: f(&pv["px_per_em"]),
            prune: f(&pv["prune"]),
            extend: f(&pv["extend"]),
            smooth: f(&pv["smooth"]),
            simplify: f(&pv["simplify"]),
            junction_merge: f(&pv["junction_merge"]),
        };
        assert_eq!(json!(params.key()), g["key"]);
        let r = skeleton::skeleton_strokes(
            &lines(&g["contours"]),
            f(&g["x_height"]),
            &params,
            g["want_raw"].as_bool().unwrap(),
        );
        for (what, got, want) in [
            ("raw", lj(&r.raw), &g["raw"]),
            ("strokes", lj(&r.strokes), &g["strokes"]),
            ("closed", json!(r.closed), &g["closed"]),
        ] {
            if let Some(d) = first_diff(&format!("{name} {what}"), &got, want) {
                failures.push(d);
                break;
            }
        }
    }
    for (i, fc) in c["fills"].as_array().unwrap().iter().enumerate() {
        let got = lj(&skeleton::fill_centerlines(&lines(&fc["contours"])));
        if let Some(d) = first_diff(&format!("fill {i}"), &got, &fc["lines"]) {
            failures.push(d);
        }
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
}
