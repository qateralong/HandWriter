use std::collections::HashMap;

use handwriter_core::checks::to_machine;
use handwriter_core::drawing::align::{AlignReading, align_correction, align_marks, make_align_gcode};
use handwriter_core::drawing::pipeline::{MachineMap, compose_drawing};
use handwriter_core::drawing::sources::load_drawing;
use handwriter_core::settings::{Settings, Travel};

fn loader(
    spec: &str,
    imp: &handwriter_core::settings::DrawingImport,
    tol: f64,
) -> handwriter_core::drawing::model::ImportResult {
    load_drawing(spec, imp, tol, std::path::Path::new("."))
}

fn variants() -> Vec<(&'static str, Settings)> {
    let mut out = Vec::new();
    out.push(("default", Settings::default()));
    let mut s = Settings::default();
    s.drawing.a3.enabled = true;
    out.push(("a3", s));
    let mut s = Settings::default();
    s.drawing.a3.enabled = true;
    s.drawing.marked.enabled = true;
    s.drawing.a3.x_max = 239.0;
    s.drawing.a3.y_max = 190.0;
    s.drawing.sheet.format = handwriter_core::settings::SheetFormat::A3;
    out.push(("a3 like the user", s));
    let mut s = Settings::default();
    s.drawing.marked.enabled = true;
    out.push(("marked", s));
    let mut s = Settings::default();
    s.printer.flip_x = true;
    s.printer.flip_y = true;
    s.printer.travel = Some(Travel { x_min: -200.0, x_max: 3.0, y_min: -215.0, y_max: 3.0 });
    s.drawing.split.areas = 4;
    out.push(("flipped 4 areas", s));
    out
}

fn drawn_on_sheet(s: &Settings, p: (f64, f64), errors: &HashMap<i64, (f64, f64)>) -> Vec<(usize, (f64, f64))> {
    let c = compose_drawing(s, &loader);
    c.parts
        .iter()
        .map(|part| {
            let ps = c.part_settings(part.rotation);
            let q = c.to_machine(p, part.rotation);
            let real = to_machine((q.0 + part.dx, q.1 + part.dy), &ps);
            let nominal = MachineMap { dx: 0.0, dy: 0.0, ..c.machine_map(part) };
            let back = nominal.to_sheet(real);
            let e = errors[&part.rotation];
            (part.index, (back.0 + e.0, back.1 + e.1))
        })
        .collect()
}

#[test]
fn alignment_test_measures_and_corrects_pass_offsets() {
    let mut checked = 0;
    for (name, s) in variants() {
        let c = compose_drawing(&s, &loader);
        assert!(c.errors.is_empty(), "{name}: {:?}", c.errors);
        if c.parts.len() < 2 {
            continue;
        }
        let marks = align_marks(&c);
        assert!(!marks.is_empty(), "{name}: no marks");
        for part in &c.parts {
            let g = make_align_gcode(&c, &marks, part).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert!(g.contains("ALIGNMENT TEST"), "{name}");
        }
        let truth: HashMap<i64, (f64, f64)> = c
            .parts
            .iter()
            .enumerate()
            .map(|(i, p)| (p.rotation, [(0.4, -0.3), (1.2, -2.0), (0.5, 0.7), (-1.0, 1.5)][i % 4]))
            .collect();
        let readings: Vec<AlignReading> = marks
            .iter()
            .map(|m| {
                let (a, b) = (truth[&m.scale_rot], truth[&m.pointer_rot]);
                AlignReading { mark: m.id, x: b.0 - a.0, y: b.1 - a.1 }
            })
            .collect();
        let offsets = align_correction(&c, &marks, &readings).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let mut fixed = s.clone();
        fixed.drawing.split.offsets = offsets;
        let p = (marks[0].x, marks[0].y);
        let before = drawn_on_sheet(&s, p, &truth);
        let spread = |v: &[(usize, (f64, f64))]| {
            v.iter().map(|(_, q)| (q.0 - v[0].1.0).hypot(q.1 - v[0].1.1)).fold(0.0, f64::max)
        };
        assert!(spread(&before) > 1.0, "{name}: simulated error too small");
        let after = drawn_on_sheet(&fixed, p, &truth);
        assert!(spread(&after) < 0.02, "{name}: passes still apart after correction: {after:?}");
        let again = compose_drawing(&fixed, &loader);
        let readings2: Vec<AlignReading> = marks.iter().map(|m| AlignReading { mark: m.id, x: 0.0, y: 0.0 }).collect();
        let same = align_correction(&again, &marks, &readings2).unwrap();
        assert_eq!(same, fixed.drawing.split.offsets, "{name}: zero readings must keep the offsets");
        checked += 1;
    }
    assert!(checked >= 3, "only {checked} multi-pass variants");
}
