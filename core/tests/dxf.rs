use std::fs;
use std::path::{Path, PathBuf};

use handwriter_core::drawing::model::ImportResult;
use handwriter_core::dxf::import_dxf;
use handwriter_core::geometry::Point;
use handwriter_core::settings::{DrawingImport, Units};
use serde_json::{Value, json};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden/dxf")
}

fn first_diff(path: &str, a: &Value, b: &Value, tol: f64) -> Option<String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (p, q) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            let bad = if tol == 0.0 { p.to_bits() != q.to_bits() } else { (p - q).abs() > tol * (1.0 + q.abs()) };
            bad.then(|| format!("{path}: rust {a} vs python {b}"))
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{path}: length {} vs {}\n rust {a}\n python {b}", x.len(), y.len()));
            }
            x.iter().zip(y).enumerate().find_map(|(i, (p, q))| first_diff(&format!("{path}[{i}]"), p, q, tol))
        }
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x {
                match y.get(k) {
                    None => return Some(format!("{path}.{k}: missing in python")),
                    Some(w) => {
                        if let Some(d) = first_diff(&format!("{path}.{k}"), v, w, tol) {
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

#[test]
fn dxf_imports_match_python() {
    let cases: Value =
        serde_json::from_str(&fs::read_to_string(dir().join("cases.json")).expect("run python tools/make_golden.py"))
            .unwrap();
    let mut failures = Vec::new();
    for c in cases["imports"].as_array().unwrap() {
        let file = c["file"].as_str().unwrap();
        let data = fs::read(dir().join("input").join(file)).unwrap();
        let units = match c["units"].as_str().unwrap() {
            "mm" => Units::Mm,
            "in" => Units::In,
            _ => Units::Auto,
        };
        let imp = DrawingImport { units, fill_centerlines: c["fills"].as_bool().unwrap(), ..Default::default() };
        let got = dump(&import_dxf(&data, file, &imp, c["tol"].as_f64().unwrap()));
        let tol = if file.starts_with("approx_") { 1e-9 } else { 0.0 };
        if let Some(d) = first_diff("", &got, &c["result"], tol) {
            failures.push(format!("{file} units={} tol={} fills={}: {d}", c["units"], c["tol"], c["fills"]));
        }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}
