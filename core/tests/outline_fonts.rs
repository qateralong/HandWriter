use std::fs;
use std::path::{Path, PathBuf};

use handwriter_core::glyphs::GlyphProvider;
use handwriter_core::outline::OutlineGlyphProvider;
use handwriter_core::pipeline::{compose, make_gcode};
use handwriter_core::settings::Settings;
use handwriter_core::skeleton::SkeletonParams;
use serde_json::{Value, json};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn first_diff(path: &str, a: &Value, b: &Value) -> Option<String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => (x.as_f64().unwrap().to_bits() != y.as_f64().unwrap().to_bits())
            .then(|| format!("{path}: rust {a} vs python {b}")),
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{path}: length {} vs {}: rust {a} python {b}", x.len(), y.len()));
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

fn cases() -> Value {
    serde_json::from_str(&fs::read_to_string(root().join("tests/golden/outline_fonts/cases.json")).unwrap()).unwrap()
}

fn pts(v: &[(f64, f64)]) -> Value {
    json!(v.iter().map(|p| json!([p.0, p.1])).collect::<Vec<_>>())
}

#[test]
fn outline_fonts_match_fonttools() {
    let c = cases();
    let mut failures = Vec::new();
    for (fname, want) in c["fonts"].as_object().unwrap() {
        let prov = OutlineGlyphProvider::from_path(&root().join("tests/fonts").join(fname)).unwrap();
        let d = &prov.data;
        let info = prov.info();
        let a = &d.analysis;
        let mut got = serde_json::Map::new();
        got.insert("name".into(), json!(prov.name()));
        got.insert("upem".into(), json!(d.upem));
        got.insert("order".into(), json!(d.order));
        got.insert(
            "cmap".into(),
            json!(d.cmap.iter().map(|(k, v)| json!([k, d.order[*v as usize]])).collect::<Vec<_>>()),
        );
        let m = &d.metrics;
        got.insert(
            "metrics".into(),
            json!({"x_height": m.x_height, "cap_height": m.cap_height, "ascent": m.ascent,
            "descent": m.descent, "x_height_source": m.x_height_source}),
        );
        got.insert("analysis".into(), json!({"gsub_features": a.gsub_features, "gpos_features": a.gpos_features,
            "variants": a.variants, "ligatures": a.ligatures.iter().map(|(c, g, f)| json!({"chars": c, "glyph": g, "feature": f})).collect::<Vec<_>>()}));
        got.insert("chars".into(), json!(info.chars));
        got.insert("variants".into(), json!(info.variants));
        let mut nfc = serde_json::Map::new();
        let mut vp = serde_json::Map::new();
        for ch in want["names_for_char"].as_object().unwrap().keys() {
            nfc.insert(ch.clone(), json!(prov.glyph_names_for_char(ch)));
            vp.insert(ch.clone(), json!(prov.variant_pool(ch)));
        }
        got.insert("names_for_char".into(), Value::Object(nfc));
        got.insert("variant_pool".into(), Value::Object(vp));
        got.insert("space_advance".into(), json!(prov.space_advance()));
        got.insert(
            "advances".into(),
            json!(
                d.order.iter().take(80).map(|n| (n.clone(), json!(prov.advance(n)))).collect::<serde_json::Map<_, _>>()
            ),
        );
        let shapes: Vec<Value> = want["shape"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| {
                let t = x[0].as_str().unwrap();
                json!([
                    t,
                    prov.shape(t)
                        .iter()
                        .map(|g| json!([g.name, g.cluster, g.advance, g.x_offset, g.y_offset]))
                        .collect::<Vec<_>>()
                ])
            })
            .collect();
        got.insert("shape".into(), json!(shapes));
        let mut contours = serde_json::Map::new();
        for n in want["contours"].as_object().unwrap().keys() {
            let gid = d.order.iter().position(|o| o == n).unwrap() as u16;
            contours.insert(n.clone(), json!(d.contours(gid).iter().map(|c| pts(c)).collect::<Vec<_>>()));
        }
        got.insert("contours".into(), Value::Object(contours));
        if let Some(diff) = first_diff(fname, &Value::Object(got), want) {
            failures.push(diff);
        }
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn outline_handwriting_matches_python() {
    let c = cases();
    let mut failures = Vec::new();
    for case in c["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let s = Settings::from_json(&case["settings"].to_string()).unwrap();
        let base =
            OutlineGlyphProvider::from_path(&root().join("tests/fonts").join(case["font"].as_str().unwrap())).unwrap();
        let prov = base.with_params(SkeletonParams::from(&s.outline));
        let comp = compose(&s, &prov);
        let lay = comp.layout.as_ref().unwrap();
        let glyphs: Vec<Value> = lay
            .glyphs
            .iter()
            .map(|g| json!([g.word, g.letter, g.ch.to_string(), g.glyph, g.x, g.y, g.advance]))
            .collect();
        let strokes: Vec<Value> = lay.strokes.iter().map(|st| pts(&st.points)).collect();
        for (what, got, want) in [
            ("errors", json!(comp.errors), &case["errors"]),
            ("warnings", json!(comp.warnings), &case["warnings"]),
            ("glyphs", json!(glyphs), &case["glyphs"]),
            ("strokes", json!(strokes), &case["strokes"]),
            ("gcode", json!(make_gcode(&comp).ok()), &case["gcode"]),
        ] {
            if let Some(d) = first_diff(&format!("{name} {what}"), &got, want) {
                failures.push(d);
                break;
            }
        }
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn fonts_load_from_settings_like_python() {
    use handwriter_core::glyphs::{FontPaths, load_provider};
    use handwriter_core::pipeline::compose_with_font;
    use handwriter_core::settings::{Mode, OutlineOptions};
    let paths = FontPaths { builtin_dir: root().join("handwriter/fonts"), user_fonts_dir: root().join("tests/fonts") };
    let dir = root().join("tests/golden/text_outline_badscript");
    let s = Settings::from_json(&fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap();
    let c = compose_with_font(&s, &paths);
    assert_eq!(make_gcode(&c).unwrap(), fs::read_to_string(dir.join("text.gcode")).unwrap());
    let o = OutlineOptions::default();
    let missing = load_provider("nope.ttf", Mode::Outlines, &o, &paths).err().unwrap();
    assert_eq!(missing, format!("Шрифт не найден: {}", paths.user_fonts_dir.join("nope.ttf").display()));
    let wrong = load_provider("BadScript-Regular.ttf", Mode::Strokes, &o, &paths).err().unwrap();
    assert_eq!(wrong, "Для шрифта BadScript-Regular.ttf нужен режим «Контуры»");
    let wrong2 = load_provider("builtin:hershey_cyrillic.svg", Mode::Outlines, &o, &paths).err().unwrap();
    assert_eq!(wrong2, "Для шрифта hershey_cyrillic.svg нужен режим «Штрихи»");
    assert_eq!(
        load_provider("builtin:hershey_cyrillic.svg", Mode::Strokes, &o, &paths).unwrap().name(),
        "Hershey Complex Cyrillic"
    );
}
