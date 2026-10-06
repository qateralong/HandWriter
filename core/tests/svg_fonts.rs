use std::fs;
use std::path::{Path, PathBuf};

use handwriter_core::geometry::Point;
use handwriter_core::glyphs::{GlyphProvider, StrokeGlyphProvider};
use handwriter_core::rand::{pick, rnd, urnd, vnoise};
use handwriter_core::svgparse::{SKIP_TAGS, parse_transform, parse_viewbox, path_d_to_strokes, walk_strokes};
use handwriter_core::xml;
use serde_json::{Value, json};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn cases() -> Value {
    let path = root().join("tests/golden/svg_fonts/cases.json");
    serde_json::from_str(&fs::read_to_string(path).expect("run python tools/make_golden.py")).unwrap()
}

fn bits_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64().unwrap().to_bits() == y.as_f64().unwrap().to_bits(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| bits_equal(p, q)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|((k1, v1), (k2, v2))| k1 == k2 && bits_equal(v1, v2))
        }
        _ => a == b,
    }
}

fn strokes_json(strokes: &[Vec<Point>]) -> Value {
    json!(strokes.iter().map(|s| s.iter().map(|p| json!([p.0, p.1])).collect::<Vec<_>>()).collect::<Vec<_>>())
}

fn dump(p: &StrokeGlyphProvider) -> Value {
    let info = p.info();
    let m = p.metrics();
    let mut glyphs = serde_json::Map::new();
    for name in p.glyph_order() {
        let g = p.glyph(name).unwrap();
        glyphs.insert(name.into(), json!({"strokes": strokes_json(&g.strokes), "advance": g.advance}));
    }
    json!({
        "name": p.name(), "glyph_count": info.glyph_count, "chars": info.chars, "variants": info.variants,
        "notes": info.notes, "cmap": p.cmap().iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        "glyph_order": p.glyph_order().collect::<Vec<_>>(),
        "metrics": {"x_height": m.x_height, "cap_height": m.cap_height, "ascent": m.ascent, "descent": m.descent,
                    "x_height_source": m.x_height_source},
        "space_advance": p.space_advance(), "glyphs": glyphs,
    })
}

#[test]
fn stroke_fonts_match_python() {
    let input = root().join("tests/golden/svg_fonts/input");
    std::fs::create_dir_all(input.join("empty")).unwrap();
    for case in cases()["fonts"].as_array().unwrap() {
        let label = case["font"].as_str().unwrap();
        if cfg!(windows) && label == "input/mixed" {
            continue;
        }
        let path = if label == "builtin" {
            root().join("handwriter/fonts/hershey_cyrillic.svg")
        } else {
            root().join("tests/golden/svg_fonts").join(label)
        };
        match (StrokeGlyphProvider::from_path(&path), case.get("result")) {
            (Ok(p), Some(want)) => {
                let got = dump(&p);
                for key in want.as_object().unwrap().keys() {
                    assert!(
                        bits_equal(&got[key], &want[key]),
                        "{label}: {key}\n got: {}\nwant: {}",
                        got[key],
                        want[key]
                    );
                }
            }
            (Err(e), None) => {
                let msg = e.0.replace(&input.display().to_string(), "<input>");
                assert_eq!(msg, case["error"][1].as_str().unwrap(), "{label}");
            }
            (got, _) => panic!("{label}: got {got:?}, want {case}"),
        }
    }
}

#[test]
fn paths_match_python() {
    let mut errors = 0;
    for case in cases()["paths"].as_array().unwrap() {
        let d = case[0].as_str().unwrap();
        let m: [f64; 6] = serde_json::from_value(case[1].clone()).unwrap();
        let tol = case[2].as_f64().unwrap();
        let want = &case[3];
        let want_error = want.get(0).is_some_and(Value::is_string);
        match path_d_to_strokes(d, m, tol) {
            Ok(s) => {
                assert!(!want_error, "{d:?}: python raised {want}");
                assert!(
                    bits_equal(&strokes_json(&s), want),
                    "{d:?} {m:?} {tol}\n got: {}\nwant: {want}",
                    strokes_json(&s)
                );
            }
            Err(e) => {
                assert!(want_error, "{d:?}: rust error {e}");
                errors += 1;
            }
        }
    }
    assert_eq!(errors, 48);
}

#[test]
fn transforms_and_documents_match_python() {
    let c = cases();
    for t in c["transforms"].as_array().unwrap() {
        let s = t[0].as_str().unwrap();
        let got = parse_transform(Some(s));
        assert!(bits_equal(&json!(got), &t[1]), "{s:?}: {got:?} vs {}", t[1]);
    }
    for doc in c["documents"].as_array().unwrap() {
        let text = doc["svg"].as_str().unwrap();
        let d = xml::parse(text).unwrap();
        let root = d.root_element();
        let vb = parse_viewbox(root).map(|v| json!([v.0, v.1, v.2, v.3])).unwrap_or(Value::Null);
        assert!(bits_equal(&vb, &doc["viewbox"]), "{}", doc["name"]);
        let s = walk_strokes(root, [1.0, 0.0, 0.0, -1.0, 0.0, 100.0], 0.01, SKIP_TAGS).unwrap();
        assert!(
            bits_equal(&strokes_json(&s), &doc["strokes"]),
            "{}\n got: {}\nwant: {}",
            doc["name"],
            strokes_json(&s),
            doc["strokes"]
        );
    }
}

#[test]
fn rand_matches_python() {
    let c = cases();
    let r = &c["rand"];
    let keys = |v: &Value| -> Vec<i64> { v.as_array().unwrap().iter().map(|k| k.as_i64().unwrap()).collect() };
    for x in r["rnd"].as_array().unwrap() {
        let got = rnd(x[0].as_i64().unwrap(), x[1].as_str().unwrap(), &keys(&x[2]));
        assert_eq!(got.to_bits(), x[3].as_f64().unwrap().to_bits(), "{x}");
    }
    for x in r["urnd"].as_array().unwrap() {
        let got = urnd(x[0].as_i64().unwrap(), x[1].as_str().unwrap(), &keys(&x[2]));
        assert_eq!(got.to_bits(), x[3].as_f64().unwrap().to_bits(), "{x}");
    }
    for x in r["pick"].as_array().unwrap() {
        let got = pick(x[0].as_i64().unwrap(), x[1].as_str().unwrap(), x[2].as_i64().unwrap(), &keys(&x[3]));
        assert_eq!(got, x[4].as_i64().unwrap(), "{x}");
    }
    for x in r["vnoise"].as_array().unwrap() {
        let got = vnoise(x[0].as_i64().unwrap(), x[1].as_str().unwrap(), x[2].as_f64().unwrap(), &keys(&x[3]));
        assert_eq!(got.to_bits(), x[4].as_f64().unwrap().to_bits(), "{x}");
    }
}
