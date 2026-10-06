use handwriter_core::checks::to_machine;
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

#[test]
fn machine_points_map_back_to_the_sheet() {
    let mut variants = Vec::new();
    variants.push(Settings::default());
    let mut s = Settings::default();
    s.drawing.a3.enabled = true;
    variants.push(s);
    let mut s = Settings::default();
    s.drawing.marked.enabled = true;
    variants.push(s);
    let mut s = Settings::default();
    s.printer.flip_x = true;
    s.printer.flip_y = true;
    s.printer.travel = Some(Travel { x_min: -200.0, x_max: 3.0, y_min: -215.0, y_max: 3.0 });
    s.drawing.split.areas = 4;
    variants.push(s);
    let mut checked = 0;
    for s in variants {
        let c = compose_drawing(&s, &loader);
        assert!(!c.parts.is_empty(), "{:?}", c.errors);
        for part in &c.parts {
            let ps = c.part_settings(part.rotation);
            let map = c.machine_map(part);
            for (orig, mach) in part.strokes.iter().zip(c.part_strokes(part, None)) {
                for (&p, q) in orig.iter().zip(mach) {
                    let back = map.to_sheet(to_machine(q, &ps));
                    assert!((back.0 - p.0).abs() < 1e-9 && (back.1 - p.1).abs() < 1e-9, "{p:?} -> {back:?}");
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 100);
    let mut s = Settings::default();
    s.printer.flip_y = true;
    let m = MachineMap::for_text(&s);
    assert_eq!(m.to_sheet(to_machine((12.5, 30.0), &s)), (12.5, 30.0));
}
