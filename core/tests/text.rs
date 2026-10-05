use std::fs;
use std::path::{Path, PathBuf};

use handwriter_core::glyphs::StrokeGlyphProvider;
use handwriter_core::hyphen::{ENGLISH, RUSSIAN};
use handwriter_core::layout::LayoutResult;
use handwriter_core::pipeline::{compose, make_gcode, make_test_gcode};
use handwriter_core::settings::Settings;
use handwriter_core::text::{ProcessedText, break_positions};
use serde_json::{Value, json};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn cases() -> Value {
    let path = root().join("tests/golden/text/cases.json");
    serde_json::from_str(&fs::read_to_string(path).expect("run python tools/make_golden.py")).unwrap()
}

fn bits_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64().unwrap().to_bits() == y.as_f64().unwrap().to_bits(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| bits_equal(p, q)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| bits_equal(v, w)))
        }
        _ => a == b,
    }
}

fn first_diff(path: &str, a: &Value, b: &Value) -> Option<String> {
    match (a, b) {
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{path}: length {} vs {}", x.len(), y.len()));
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
        _ => (!bits_equal(a, b)).then(|| format!("{path}: rust {a} vs python {b}")),
    }
}

fn dump_text(pt: &ProcessedText) -> Value {
    json!({
        "paragraphs": pt.paragraphs.iter().map(|p| json!({
            "index": p.index, "indent": p.indent, "blank_before": p.blank_before,
            "words": p.words.iter().map(|&i| pt.words[i].index).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "words": pt.words.iter().map(|w| json!({
            "index": w.index, "text": w.text, "paragraph": w.paragraph, "text_line": w.text_line,
            "soft_breaks": w.soft_breaks.iter().collect::<Vec<_>>(),
            "breaks": break_positions(w, true).into_iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
            "breaks_manual": break_positions(w, false).into_iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "missing": pt.missing.iter().map(|m| json!({"char": m.ch.to_string(), "count": m.count,
            "positions": m.positions.iter().map(|p| json!({"line": p.line, "word": p.word, "letter": p.letter})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "skipped": pt.skipped.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "unresolved": pt.unresolved.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "replaced": pt.replaced.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
    })
}

fn dump_layout(l: &LayoutResult) -> Value {
    json!({
        "glyphs": l.glyphs.iter().map(|g| json!({
            "word": g.word, "letter": g.letter, "line": g.line, "char": g.ch.to_string(), "glyph": g.glyph,
            "x": g.x, "y": g.y, "advance": g.advance, "missing": g.missing, "hyphen": g.hyphen, "segment": g.segment,
            "adv_em": g.adv_em, "dx_em": g.dx_em, "dy_em": g.dy_em, "size": g.size, "slant": g.slant, "voff": g.voff,
        })).collect::<Vec<_>>(),
        "baselines": l.baselines, "used_lines": l.used_lines, "scale": l.scale, "first_word": l.first_word,
        "last_word": l.last_word, "last_letter": l.last_letter, "next_word": l.next_word, "next_letter": l.next_letter,
        "warnings": l.warnings, "errors": l.errors,
        "strokes": l.strokes.iter().map(|s| json!({
            "points": s.points.iter().map(|p| json!([p.0, p.1])).collect::<Vec<_>>(), "tags": s.tags,
            "word": s.word, "letter": s.letter, "line": s.line, "hyphen": s.hyphen})).collect::<Vec<_>>(),
    })
}

#[test]
fn hyphenation_matches_pyphen() {
    for w in cases()["hyphenation"].as_array().unwrap() {
        let word = w[0].as_str().unwrap();
        assert_eq!(json!(RUSSIAN.positions(word)), w[1], "ru {word}");
        assert_eq!(json!(ENGLISH.positions(word)), w[2], "en {word}");
    }
}

#[test]
fn text_pipeline_matches_python() {
    let prov = StrokeGlyphProvider::from_path(&root().join("handwriter/fonts/hershey_cyrillic.svg")).unwrap();
    let all = cases();
    let mut failures = Vec::new();
    for case in all["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let s = Settings::from_json(&case["settings"].to_string()).unwrap();
        let c = compose(&s, &prov);
        let mut check = |what: &str, got: Value, want: &Value| {
            if let Some(d) = first_diff(what, &got, want) {
                failures.push(format!("{name}: {d}"));
            }
        };
        check("text", dump_text(c.text.as_ref().unwrap()), &case["text"]);
        check("errors", json!(c.errors), &case["errors"]);
        check("warnings", json!(c.warnings), &case["warnings"]);
        match (&c.layout, case.get("layout")) {
            (Some(l), Some(want)) => check("layout", dump_layout(l), want),
            (None, None) => {}
            (got, _) => check("layout presence", json!(got.is_some()), &json!(!got.is_some())),
        }
        let resume = c.resume.as_ref().map(
            |r| json!({"word": r.word, "letter": r.letter, "path": r.path, "point": r.point, "connected": r.connected}),
        );
        check("resume", json!(resume), case.get("resume").unwrap_or(&Value::Null));
        match make_gcode(&c) {
            Ok(g) => check("gcode", json!(g), case.get("gcode").unwrap_or(&Value::Null)),
            Err(e) => check("refused", json!(e), case.get("refused").unwrap_or(&Value::Null)),
        }
        match make_test_gcode(&s) {
            Ok((g, _)) => check("test_gcode", json!(g), case.get("test_gcode").unwrap_or(&Value::Null)),
            Err(e) => check("test_refused", json!(e), case.get("test_refused").unwrap_or(&Value::Null)),
        }
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
}
