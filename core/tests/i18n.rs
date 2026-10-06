use std::fs;
use std::path::Path;

use handwriter_core::i18n::{set_lang, tr_json, tr_static, tr_to};
use serde_json::Value;

#[test]
fn translations_match_python() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden/i18n/cases.json");
    let cases: Value = serde_json::from_str(&fs::read_to_string(path).expect("run make_golden.py")).unwrap();
    let mut bad = Vec::new();
    for pair in cases["tr"].as_array().unwrap() {
        let (src, want) = (pair[0].as_str().unwrap(), pair[1].as_str().unwrap());
        let got = tr_to(src, "en");
        if got != want {
            bad.push(format!("{src:?}\n rust {got:?}\n python {want:?}"));
        }
    }
    set_lang("en");
    for pair in cases["static"].as_array().unwrap() {
        if tr_static(pair[0].as_str().unwrap()) != pair[1].as_str().unwrap() {
            bad.push(format!("static {:.60}", pair[0].as_str().unwrap()));
        }
    }
    let j = &cases["json"];
    if tr_json(j[0].clone()) != j[1] {
        bad.push(format!("json {}", tr_json(j[0].clone())));
    }
    set_lang("ru");
    assert!(
        bad.is_empty(),
        "{} differences:\n{}",
        bad.len(),
        bad.iter().take(10).cloned().collect::<Vec<_>>().join("\n")
    );
}
