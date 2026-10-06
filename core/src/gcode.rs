use crate::checks::{to_machine, travel_box};
use crate::geometry::{Point, make_transform, polyline_length};
use crate::layout::line_baselines;
use crate::numeric;
use crate::settings::Settings;

pub const END_PAD: [&str; 8] = ["G4 P100"; 8];

pub fn fmt(v: f64) -> String {
    let s = format!("{v:.2}");
    if s == "-0.00" { "0.00".into() } else { s }
}

fn feed(v: f64) -> String {
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
    moves
}

impl Stats {
    pub fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "draw_mm": numeric::round_to(self.draw_mm, 1),
            "travel_mm": numeric::round_to(self.travel_mm, 1),
            "strokes": self.strokes,
            "lifts": self.lifts,
            "time_s": self.time_s.round_ties_even() as i64,
        })
    }
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
        t = (draw / pr.feed_draw
            + travel / pr.feed_travel
            + 2.0 * dz * n as f64 / pr.feed_z
            + (pr.pen_up_z.abs() + pr.end_lift) / pr.feed_z)
            * 60.0;
    }
    Stats { draw_mm: draw, travel_mm: travel, strokes: n, lifts: n, time_s: t }
}

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
            "; pen up Z{up} down Z{down}, end Z{}, feed draw {fd} travel {ft} z {fz}, simplify {}",
            fmt(pr.pen_up_z + pr.end_lift),
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
    lines.extend(["G21", "G90", "M104 S0", "M140 S0", "M420 S0", "M211 S0", "G92 X0 Y0 Z0"].map(String::from));
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
    lines.push(format!("G0 Z{} F{fz}", fmt(pr.pen_up_z + pr.end_lift)));
    lines.push("M400".into());
    lines.extend(END_PAD.iter().map(|s| s.to_string()));
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

pub fn test_pattern(s: &Settings) -> Vec<Vec<Point>> {
    let (sh, ty) = (&s.sheet, &s.typography);
    let t = make_transform(ty.rotation_deg, ty.dx, ty.dy);
    let (x0, x1) = (sh.margin_left, sh.width - sh.margin_right);
    let bases = line_baselines(s);
    let y_top = sh.height - sh.first_line_top;
    let y_bot = bases.last().copied().unwrap_or(sh.bottom_limit);
    let rect = [(x0, y_bot), (x1, y_bot), (x1, y_top), (x0, y_top), (x0, y_bot)];
    let mut out = vec![rect.iter().map(|&p| t(p)).collect::<Vec<_>>()];
    if bases.len() > 2 {
        for b in &bases[1..bases.len() - 1] {
            let y = b + ty.baseline_shift;
            out.push(vec![t((x0, y)), t((x0 + 3.0, y))]);
        }
    }
    let o = s.printer.test_mark_offset;
    let (ax, ay, head) = (40.0, 20.0, 3.0);
    out.push(vec![(o, o), (o + ax, o)]);
    out.push(vec![(o + ax - head, o + head * 0.6), (o + ax, o), (o + ax - head, o - head * 0.6)]);
    out.push(vec![(o, o), (o, o + ay)]);
    out.push(vec![(o - head * 0.6, o + ay - head), (o, o + ay), (o + head * 0.6, o + ay - head)]);
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
