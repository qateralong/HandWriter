use std::fs;
use std::path::{Path, PathBuf};

use handwriter_core::drawing::model::ImportResult;
use handwriter_core::drawing::ops;
use handwriter_core::drawing::svg_import::import_svg;
use handwriter_core::geometry::Point;
use handwriter_core::settings::{DrawingImport, Units};
use serde_json::{Value, json};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden/drawing_import")
}

fn cases() -> Value {
    serde_json::from_str(&fs::read_to_string(dir().join("cases.json")).expect("run python tools/make_golden.py"))
        .unwrap()
}

fn first_diff(path: &str, a: &Value, b: &Value) -> Option<String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => (x.as_f64().unwrap().to_bits() != y.as_f64().unwrap().to_bits())
            .then(|| format!("{path}: rust {a} vs python {b}")),
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{path}: length {} vs {}\n rust {a}\n python {b}", x.len(), y.len()));
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

fn pts(v: &[Point]) -> Value {
    json!(v.iter().map(|p| json!([p.0, p.1])).collect::<Vec<_>>())
}

fn many(v: &[Vec<Point>]) -> Value {
    json!(v.iter().map(|p| pts(p)).collect::<Vec<_>>())
}

fn dump(r: &ImportResult) -> Value {
    json!({
        "kind": r.kind, "name": r.name,
        "paths": r.paths.iter().map(|p| json!({"points": pts(&p.points), "closed": p.closed, "width": p.width,
            "layer": p.layer, "dash": p.dash, "dash_offset": p.dash_offset})).collect::<Vec<_>>(),
        "texts": r.texts.iter().map(|t| json!({"x": t.x, "y": t.y, "text": t.text, "kind": t.kind})).collect::<Vec<_>>(),
        "warnings": r.warnings, "errors": r.errors, "units": r.units, "units_note": r.units_note,
        "pages": r.pages, "page": r.page,
        "layers": r.layers.iter().map(|(k, l)| (k.clone(), json!({"count": l.count, "width": l.width, "linetype": l.linetype}))).collect::<serde_json::Map<_, _>>(),
        "bbox": r.bbox().map(|b| json!([b.0, b.1, b.2, b.3])),
    })
}

fn p(v: &Value) -> Point {
    (v[0].as_f64().unwrap(), v[1].as_f64().unwrap())
}

fn pl(v: &Value) -> Vec<Point> {
    v.as_array().unwrap().iter().map(p).collect()
}

fn pls(v: &Value) -> Vec<Vec<Point>> {
    v.as_array().unwrap().iter().map(pl).collect()
}

#[test]
fn svg_imports_match_python() {
    let mut failures = Vec::new();
    for case in cases()["imports"].as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let units = match case["units"].as_str().unwrap() {
            "mm" => Units::Mm,
            "in" => Units::In,
            _ => Units::Auto,
        };
        let opts = DrawingImport { units, ..DrawingImport::default() };
        let data = fs::read(dir().join("input").join(file)).unwrap();
        let got = dump(&import_svg(&data, file, &opts, case["tol"].as_f64().unwrap()));
        let mut want = case["result"].clone();
        if file == "broken.svg" {
            let e = got["errors"][0].as_str().unwrap();
            assert!(e.starts_with("SVG не читается: "), "{e}");
            want["errors"] = got["errors"].clone();
        }
        if let Some(d) = first_diff(&format!("{file} {units:?} {}", case["tol"]), &got, &want) {
            failures.push(d);
        }
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn drawing_ops_match_python() {
    let c = cases();
    let o = &c["ops"];
    let mut failures = Vec::new();
    let mut check = |what: String, got: Value, want: &Value| {
        if let Some(d) = first_diff(&what, &got, want) {
            failures.push(d);
        }
    };
    for (i, x) in o["dash"].as_array().unwrap().iter().enumerate() {
        let pat: Vec<f64> = x[1].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        check(format!("dash[{i}]"), many(&ops::dash_polyline(&pl(&x[0]), &pat, x[2].as_f64().unwrap())), &x[3]);
    }
    for (i, x) in o["dedupe"].as_array().unwrap().iter().enumerate() {
        check(format!("dedupe[{i}]"), pts(&ops::dedupe(&pl(&x[0]), 1e-9)), &x[1]);
    }
    for (i, x) in o["offset"].as_array().unwrap().iter().enumerate() {
        let got = ops::offset_polyline(&pl(&x[0]), x[1].as_f64().unwrap(), x[2].as_bool().unwrap());
        check(format!("offset[{i}]"), pts(&got), &x[3]);
    }
    for (i, x) in o["expand"].as_array().unwrap().iter().enumerate() {
        let got =
            ops::expand_passes(&pl(&x[0]), x[1].as_bool().unwrap(), x[2].as_i64().unwrap(), x[3].as_f64().unwrap());
        check(format!("expand[{i}]"), many(&got), &x[4]);
    }
    for (i, x) in o["join"].as_array().unwrap().iter().enumerate() {
        check(format!("join[{i}]"), many(&ops::join_paths(&pls(&x[0]), x[1].as_f64().unwrap())), &x[2]);
    }
    for (i, x) in o["order"].as_array().unwrap().iter().enumerate() {
        let got = ops::order_paths(&pls(&x[0]), x[1].as_f64().unwrap(), p(&x[2]));
        let got = json!(got.iter().map(|(k, s)| json!([k, pts(s)])).collect::<Vec<_>>());
        check(format!("order[{i}]"), got, &x[3]);
    }
    for (i, x) in o["outside"].as_array().unwrap().iter().enumerate() {
        let b = &x[1];
        let bx = (b[0].as_f64().unwrap(), b[1].as_f64().unwrap(), b[2].as_f64().unwrap(), b[3].as_f64().unwrap());
        check(format!("outside[{i}]"), many(&ops::outside_parts(&pl(&x[0]), bx, 1e-9)), &x[2]);
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
}
