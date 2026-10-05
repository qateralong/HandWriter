use std::fs;
use std::path::Path;

use handwriter_core::drawing::passes;
use handwriter_core::drawing::place::{self, Rect};
use handwriter_core::settings::{Printer, Settings};
use serde_json::{Value, json};

fn cases() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/golden/placement/cases.json");
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

fn rect(v: &Value) -> Rect {
    (v[0].as_f64().unwrap(), v[1].as_f64().unwrap(), v[2].as_f64().unwrap(), v[3].as_f64().unwrap())
}

fn opt_rect(v: Option<Rect>) -> Value {
    v.map_or(Value::Null, |r| json!([r.0, r.1, r.2, r.3]))
}

fn rects(v: &[Rect]) -> Value {
    json!(v.iter().map(|&r| opt_rect(Some(r))).collect::<Vec<_>>())
}

#[test]
fn placement_matches_python() {
    let c = cases();
    let mut failures = Vec::new();
    let mut check = |what: String, got: Value, want: &Value| {
        if let Some(d) = first_diff(&what, &got, want) {
            failures.push(d);
        }
    };
    let layouts = c["layouts"].as_array().unwrap();
    let mut computed = Vec::new();
    for (i, case) in layouts.iter().enumerate() {
        let s = Settings::from_json(&case["settings"].to_string()).unwrap();
        let bbox = (!case["bbox"].is_null()).then(|| rect(&case["bbox"]));
        let lay = place::sheet_layout(&s.drawing, bbox);
        let got = json!({
            "width": lay.width, "height": lay.height, "orientation": lay.orientation.as_str(),
            "frame": lay.frame.iter().map(|f| json!([f.points.iter().map(|p| json!([p.0, p.1])).collect::<Vec<_>>(), f.thick])).collect::<Vec<_>>(),
            "inner": opt_rect(lay.inner), "title_block": opt_rect(lay.title_block), "areas": rects(&lay.areas),
        });
        check(format!("layout[{i}]"), got, &case["layout"]);
        computed.push((s, bbox, lay));
    }
    for p in c["places"].as_array().unwrap() {
        let i = p["index"].as_u64().unwrap() as usize;
        let (s, bbox, lay) = &computed[i];
        let reach: Vec<Rect> = p["reach"].as_array().unwrap().iter().map(rect).collect();
        let pl = place::place(&s.drawing, bbox.unwrap(), lay, &reach, p["pad"].as_f64().unwrap());
        let a = pl.apply((12.5, -3.0));
        let got = json!({"scale": pl.scale, "tx": pl.tx, "ty": pl.ty, "area": opt_rect(pl.area), "errors": pl.errors,
                         "warnings": pl.warnings, "applied": [a.0, a.1]});
        check(format!("place[{i}]"), got, &p["placement"]);
    }
    for l in c["labels"].as_array().unwrap() {
        check(format!("label {}", l[0]), json!(place::scale_label(l[0].as_f64().unwrap())), &l[1]);
    }
    let ps = &c["passes"];
    for (i, m) in ps["maps"].as_array().unwrap().iter().enumerate() {
        let (w, h, r) = (m[0].as_f64().unwrap(), m[1].as_f64().unwrap(), m[2].as_i64().unwrap());
        let pt = (m[3][0].as_f64().unwrap(), m[3][1].as_f64().unwrap());
        let t = passes::to_pass(pt, r, w, h);
        let f = passes::from_pass(pt, r, w, h);
        let k = passes::corner_point(r, w, h);
        let d = passes::pass_dims(r, w, h);
        let got = json!([
            w,
            h,
            r,
            [pt.0, pt.1],
            [t.0, t.1],
            [f.0, f.1],
            [k.0, k.1],
            [d.0, d.1],
            m[8],
            opt_rect(Some(passes::rect_from_pass(rect(&m[8]), r, w, h)))
        ]);
        check(format!("maps[{i}]"), got, m);
    }
    for (i, t) in ps["tables"].as_array().unwrap().iter().enumerate() {
        let pr: Printer = serde_json::from_value(t["printer"].clone()).unwrap();
        let (w, h) = (t["W"].as_f64().unwrap(), t["H"].as_f64().unwrap());
        let tb = passes::make_table(&pr, w, h);
        let got = json!({"raw": opt_rect(Some(tb.raw)), "safe": opt_rect(Some(tb.safe)), "measured": tb.measured,
            "allow_x": tb.allow_x, "allow_y": tb.allow_y, "table_x": tb.table_x, "table_y": tb.table_y,
            "table_given": tb.table_given});
        check(format!("table[{i}]"), got, &t["table"]);
        let per_r: Vec<Value> = passes::ROTATIONS
            .iter()
            .map(|&r| {
                let o = passes::overhang(r, w, h, &tb);
                let (ok, why) = passes::rotation_allowed(r, w, h, &tb);
                json!([r, [o.0, o.1], [ok, why], opt_rect(passes::pass_rect(r, w, h, tb.safe))])
            })
            .collect();
        check(format!("table[{i}].rotations"), json!(per_r), &t["rotations"]);
    }
    for (i, u) in ps["uncovered"].as_array().unwrap().iter().enumerate() {
        let target = rect(&u[0]);
        let rs: Vec<Rect> = u[1].as_array().unwrap().iter().map(rect).collect();
        let un = passes::uncovered(target, &rs);
        let inter = rs.first().map_or(Value::Null, |&q| opt_rect(passes::intersect(target, q)));
        check(format!("uncovered[{i}]"), json!([u[0], u[1], rects(&un), passes::area(&un), inter]), u);
    }
    assert!(failures.is_empty(), "{} differences:\n{}", failures.len(), failures.join("\n"));
}
