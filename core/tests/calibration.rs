use std::fs;
use std::path::Path;

use handwriter_core::calibration::{check_errors, make_reach_check_gcode, make_zero_gcode, reach_corners};
use handwriter_core::settings::Settings;
use serde_json::{Value, json};

#[test]
fn calibration_matches_python() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden/calibration.json");
    let cases: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("run python tools/make_golden.py")).unwrap();
    let mut n = 0;
    for (i, c) in cases.as_array().unwrap().iter().enumerate() {
        let s = Settings::from_json(&c["settings"].to_string()).unwrap();
        let (errors, warnings) = check_errors(&s);
        let corners: Vec<Value> = if s.printer.travel.is_some() && errors.is_empty() {
            reach_corners(&s).iter().map(|p| json!([p.0, p.1])).collect()
        } else {
            Vec::new()
        };
        assert_eq!(json!({"errors": errors, "warnings": warnings, "corners": corners}), c["info"], "case {i}");
        match make_reach_check_gcode(&s) {
            Ok(g) => assert_eq!(json!(g), c["check"], "case {i} check"),
            Err(e) => assert_eq!(json!(e), c["check_refused"], "case {i} check"),
        }
        match make_zero_gcode(&s) {
            Ok(g) => assert_eq!(json!(g), c["zero"], "case {i} zero"),
            Err(e) => assert_eq!(json!(e), c["zero_refused"], "case {i} zero"),
        }
        n += 1;
    }
    assert_eq!(n, 80);
}
