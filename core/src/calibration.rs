use crate::checks::check_printer;
use crate::gcode::{ascii, fmt};
use crate::geometry::Point;
use crate::numeric::format_g;
use crate::settings::Settings;

pub const PREAMBLE: [&str; 7] = ["G21", "G90", "M104 S0", "M140 S0", "M420 S0", "M211 S0", "G92 X0 Y0 Z0"];

pub fn reach_corners(s: &Settings) -> Vec<Point> {
    let t = s.printer.travel.expect("measured travel");
    let m = s.printer.safety_margin;
    vec![(t.x_min + m, t.y_min + m), (t.x_max - m, t.y_min + m), (t.x_max - m, t.y_max - m), (t.x_min + m, t.y_max - m)]
}

pub fn check_errors(s: &Settings) -> (Vec<String>, Vec<String>) {
    let pr = &s.printer;
    let (mut errors, warnings) = check_printer(s);
    let mut warnings: Vec<String> = warnings.into_iter().filter(|w| !w.contains("не измерен")).collect();
    let Some(t) = pr.travel else {
        errors.push("Окно достижимости не введено: сначала запиши замеры".into());
        return (errors, warnings);
    };
    let m = pr.safety_margin;
    if t.x_max - t.x_min <= 2.0 * m || t.y_max - t.y_min <= 2.0 * m {
        errors.push(format!("Окно достижимости меньше двух запасов ({} мм) по одной из осей", format_g(2.0 * m, 6)));
    }
    if errors.is_empty() {
        for (i, (x, y)) in reach_corners(s).into_iter().enumerate() {
            if x < 0.0 || y < 0.0 {
                warnings.push(format!(
                    "Угол {} (X{x:.1} Y{y:.1}) за краем листа со стороны упоров: карандаш коснётся стола или упора — \
                     подложи туда бумагу или проверь, что там можно касаться",
                    i + 1
                ));
            }
        }
    }
    (errors, warnings)
}

fn feed(v: f64) -> String {
    (v.round_ties_even() as i64).to_string()
}

pub fn make_reach_check_gcode(s: &Settings) -> Result<String, Vec<String>> {
    let (errors, _) = check_errors(s);
    if !errors.is_empty() {
        return Err(errors);
    }
    let pr = &s.printer;
    let tb = &pr.table;
    let (up, down) = (fmt(pr.pen_up_z), fmt(pr.pen_down_z));
    let (fz, ft) = (feed(pr.feed_z), feed(pr.feed_travel));
    let t = pr.travel.expect("checked above");
    let touch_ms = (tb.touch_s * 1000.0).round_ties_even() as i64;
    let pause_ms = (tb.pause_s * 1000.0).round_ties_even() as i64;
    let mut lines: Vec<String> = vec![
        "; HandWriter gcode".into(),
        "; REACH CHECK: pencil visits the 4 corners of the reach window (inset by the safety margin),".into(),
        format!(
            "; touches the paper for {} s in each corner, pause {} s between corners",
            format_g(tb.touch_s, 6),
            format_g(tb.pause_s, 6)
        ),
        format!(
            "; reach window X{}..{} Y{}..{}, margin {} mm",
            fmt(t.x_min),
            fmt(t.x_max),
            fmt(t.y_min),
            fmt(t.y_max),
            fmt(pr.safety_margin)
        ),
        format!("; {}", ascii(&format!("sheet may overhang +X {} +Y {}", tb.overhang_x as i32, tb.overhang_y as i32))),
        format!("; pen up Z{up} down Z{down}, feed travel {ft} z {fz}"),
    ];
    lines.extend(PREAMBLE.iter().map(|s| s.to_string()));
    lines.push(format!("G0 Z{up} F{fz}"));
    let corners = reach_corners(s);
    for (i, (x, y)) in corners.iter().enumerate() {
        lines.extend([
            format!("; corner {}", i + 1),
            format!("G0 X{} Y{} F{ft}", fmt(*x), fmt(*y)),
            format!("G1 Z{down} F{fz}"),
            format!("G4 P{touch_ms}"),
            format!("G0 Z{up} F{fz}"),
        ]);
        if i + 1 < corners.len() {
            lines.push(format!("G4 P{pause_ms}"));
        }
    }
    lines.extend([format!("G0 Z{} F{fz}", fmt(pr.pen_up_z + pr.end_lift)), "M400".into()]);
    Ok(lines.join("\n") + "\n")
}

pub fn make_zero_gcode(s: &Settings) -> Result<String, Vec<String>> {
    let (errors, _) = check_printer(s);
    let errors: Vec<String> = errors.into_iter().filter(|e| !e.contains("Ход карандаша")).collect();
    if !errors.is_empty() {
        return Err(errors);
    }
    let pr = &s.printer;
    let (up, fz) = (fmt(pr.pen_up_z), feed(pr.feed_z));
    let mut lines: Vec<String> = vec![
        "; HandWriter gcode".into(),
        "; ZERO: run with the pencil touching the paper in the sheet corner at the stops.".into(),
        "; Sets X0 Y0 Z0 here (G92), lifts the pencil, software endstops off (M211 S0).".into(),
    ];
    lines.extend(PREAMBLE.iter().map(|s| s.to_string()));
    lines.extend([format!("G0 Z{up} F{fz}"), "M117 Zero set, pen up".into(), "M400".into()]);
    Ok(lines.join("\n") + "\n")
}
