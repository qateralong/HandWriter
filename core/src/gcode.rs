//! gcode для карандаша: без нагрева и без G28, ноль — там, где стоит карандаш при запуске.

use crate::checks::{to_machine, travel_box};
use crate::geometry::{Point, polyline_length};
use crate::numeric;
use crate::settings::Settings;

/// Число с двумя знаками после точки, без «-0.00».
pub fn fmt(v: f64) -> String {
    let s = format!("{v:.2}");
    if s == "-0.00" { "0.00".into() } else { s }
}

fn feed(v: f64) -> String {
    // round() в Python округляет половины к чётному
    format!("{}", v.round_ties_even() as i64)
}

fn round_stroke(points: &[Point], s: &Settings) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for &p in points {
        let (mx, my) = to_machine(p, s);
        let q = (fmt(mx), fmt(my));
        if out.last() != Some(&q) {
            out.push(q);
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub draw_mm: f64,
    pub travel_mm: f64,
    pub strokes: usize,
    pub lifts: usize,
    pub time_s: f64,
}

/// Переезды с поднятым карандашом: от нуля к первому штриху, между штрихами и обратно в ноль.
pub fn travel_moves(strokes: &[Vec<Point>]) -> Vec<(Point, Point)> {
    let mut moves = Vec::new();
    let mut cur = (0.0, 0.0);
    for st in strokes {
        let (Some(&first), Some(&last)) = (st.first(), st.last()) else { continue };
        if first != cur {
            moves.push((cur, first));
        }
        cur = last;
    }
    if cur != (0.0, 0.0) {
        moves.push((cur, (0.0, 0.0)));
    }
    moves
}

pub fn compute_stats(strokes: &[Vec<Point>], s: &Settings) -> Stats {
    let pr = &s.printer;
    let strokes: Vec<Vec<Point>> = strokes.iter().filter(|st| !st.is_empty()).cloned().collect();
    let draw = numeric::sum(strokes.iter().map(|st| polyline_length(st)));
    let travel = numeric::sum(travel_moves(&strokes).into_iter().map(|(a, b)| numeric::dist(a, b)));
    let dz = (pr.pen_up_z - pr.pen_down_z).abs();
    let n = strokes.len();
    let mut t = 0.0;
    if pr.feed_draw > 0.0 && pr.feed_travel > 0.0 && pr.feed_z > 0.0 {
        t = (draw / pr.feed_draw + travel / pr.feed_travel + 2.0 * dz * n as f64 / pr.feed_z
            + 2.0 * pr.pen_up_z.abs() / pr.feed_z)
            * 60.0;
    }
    Stats { draw_mm: draw, travel_mm: travel, strokes: n, lifts: n, time_s: t }
}

/// Текст для комментария gcode: всё не-ASCII — как `\xNN`, `\uNNNN`, `\UNNNNNNNN`.
pub fn ascii(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        let c = ch as u32;
        match c {
            0..=0x7f => out.push(ch),
            0x80..=0xff => out.push_str(&format!("\\x{c:02x}")),
            0x100..=0xffff => out.push_str(&format!("\\u{c:04x}")),
            _ => out.push_str(&format!("\\U{c:08x}")),
        }
    }
    out
}

/// gcode из штрихов в миллиметрах листа. `header` — строки комментария о задании,
/// `info` — о листе и шрифте (если `None`, собирается из настроек почерка).
pub fn generate_gcode(strokes: &[Vec<Point>], s: &Settings, header: &[String], info: Option<&[String]>) -> String {
    let pr = &s.printer;
    let (up, down) = (fmt(pr.pen_up_z), fmt(pr.pen_down_z));
    let (fz, fd, ft) = (feed(pr.feed_z), feed(pr.feed_draw), feed(pr.feed_travel));
    let bx = travel_box(s);
    let st = compute_stats(strokes, s);
    let mut lines: Vec<String> = vec!["; HandWriter gcode".into()];
    lines.extend(header.iter().map(|h| format!("; {}", ascii(h))));
    let default_info;
    let info = match info {
        Some(i) => i,
        None => {
            let (sh, ty) = (&s.sheet, &s.typography);
            default_info = [
                format!(
                    "sheet {}x{} mm, margins L{} R{}, first line {}, bottom {}, pitch {}",
                    fmt(sh.width),
                    fmt(sh.height),
                    fmt(sh.margin_left),
                    fmt(sh.margin_right),
                    fmt(sh.first_line_top),
                    fmt(sh.bottom_limit),
                    fmt(sh.line_pitch)
                ),
                format!(
                    "size {} mm, baseline shift {}, dx {} dy {}, rotation {} deg",
                    fmt(ty.size_mm),
                    fmt(ty.baseline_shift),
                    fmt(ty.dx),
                    fmt(ty.dy),
                    fmt(ty.rotation_deg)
                ),
            ];
            &default_info[..]
        }
    };
    lines.extend(info.iter().map(|i| format!("; {}", ascii(i))));
    let measured = if bx.measured {
        String::new()
    } else {
        format!(" (NOT MEASURED, {} used)", if pr.use_work_area { "printer work area" } else { "sheet size" })
    };
    lines.extend([
        format!(
            "; pen up Z{up} down Z{down}, feed draw {fd} travel {ft} z {fz}, simplify {}",
            fmt(pr.simplify_tol)
        ),
        format!(
            "; travel X{}..{} Y{}..{}{measured}, flip_x {} flip_y {}",
            fmt(bx.x_min),
            fmt(bx.x_max),
            fmt(bx.y_min),
            fmt(bx.y_max),
            pr.flip_x as i32,
            pr.flip_y as i32
        ),
        format!(
            "; strokes {}, draw {:.0} mm, travel {:.0} mm, est {:.1} min",
            st.strokes,
            st.draw_mm,
            st.travel_mm,
            st.time_s / 60.0
        ),
    ]);
    lines.extend(
        ["G21", "G90", "M104 S0", "M140 S0", "M420 S0", "M211 S0", "G92 X0 Y0 Z0"].map(String::from),
    );
    lines.push(format!("G0 Z{up} F{fz}"));
    for stroke in strokes {
        let pts = round_stroke(stroke, s);
        let Some((x, y)) = pts.first() else { continue };
        lines.push(format!("G0 X{x} Y{y} F{ft}"));
        lines.push(format!("G1 Z{down} F{fz}"));
        for (i, (x, y)) in pts[1..].iter().enumerate() {
            lines.push(if i == 0 { format!("G1 X{x} Y{y} F{fd}") } else { format!("G1 X{x} Y{y}") });
        }
        lines.push(format!("G0 Z{up} F{fz}"));
    }
    lines.push(format!("G0 Z{up} F{fz}"));
    lines.push(format!("G0 X0.00 Y0.00 F{ft}"));
    lines.push("M400".into());
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_and_ascii() {
        assert_eq!(fmt(-0.001), "0.00");
        assert_eq!(fmt(1.0), "1.00");
        assert_eq!(feed(2.5), "2");
        assert_eq!(ascii("лист é ✓ 𝄞"), "\\u043b\\u0438\\u0441\\u0442 \\xe9 \\u2713 \\U0001d11e");
    }
}
