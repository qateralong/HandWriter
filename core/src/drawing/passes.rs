use crate::drawing::place::Rect;
use crate::geometry::Point;
use crate::numeric;
use crate::settings::Printer;

pub const ROTATIONS: [i64; 4] = [0, 90, 180, 270];
pub const OVERHANG_TOL: f64 = 0.5;
pub const EPS: f64 = 1e-6;

pub fn corner(r: i64) -> &'static str {
    match r {
        0 => "A",
        90 => "D",
        180 => "C",
        _ => "B",
    }
}

pub fn corner_name(c: &str) -> &'static str {
    match c {
        "A" => "левый нижний",
        "B" => "правый нижний",
        "C" => "правый верхний",
        _ => "левый верхний",
    }
}

pub fn pass_dims(r: i64, w: f64, h: f64) -> (f64, f64) {
    if r == 0 || r == 180 { (w, h) } else { (h, w) }
}

pub fn to_pass((u, v): Point, r: i64, w: f64, h: f64) -> Point {
    match r {
        0 => (u, v),
        90 => (h - v, u),
        180 => (w - u, h - v),
        270 => (v, w - u),
        _ => panic!("поворот {r}"),
    }
}

pub fn from_pass((x, y): Point, r: i64, w: f64, h: f64) -> Point {
    match r {
        0 => (x, y),
        90 => (y, h - x),
        180 => (w - x, h - y),
        270 => (w - y, x),
        _ => panic!("поворот {r}"),
    }
}

pub fn corner_point(r: i64, w: f64, h: f64) -> Point {
    from_pass((0.0, 0.0), r, w, h)
}

pub fn rect_from_pass(rc: Rect, r: i64, w: f64, h: f64) -> Rect {
    let a = from_pass((rc.0, rc.1), r, w, h);
    let b = from_pass((rc.2, rc.3), r, w, h);
    (numeric::min(a.0, b.0), numeric::min(a.1, b.1), numeric::max(a.0, b.0), numeric::max(a.1, b.1))
}

pub fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let r = (numeric::max(a.0, b.0), numeric::max(a.1, b.1), numeric::min(a.2, b.2), numeric::min(a.3, b.3));
    (r.2 > r.0 + EPS && r.3 > r.1 + EPS).then_some(r)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub raw: Rect,
    pub safe: Rect,
    pub measured: bool,
    pub allow_x: bool,
    pub allow_y: bool,
    pub table_x: f64,
    pub table_y: f64,
    pub table_given: bool,
}

fn sorted2(a: f64, b: f64) -> (f64, f64) {
    if b < a { (b, a) } else { (a, b) }
}

pub fn make_table(printer: &Printer, _w: f64, _h: f64) -> Table {
    let m = printer.safety_margin;
    let (raw, measured) = match printer.travel {
        None => ((0.0, 0.0, printer.work_w, printer.work_h), false),
        Some(t) => {
            let fx = if printer.flip_x { -1.0 } else { 1.0 };
            let fy = if printer.flip_y { -1.0 } else { 1.0 };
            let xs = sorted2(t.x_min * fx, t.x_max * fx);
            let ys = sorted2(t.y_min * fy, t.y_max * fy);
            ((xs.0, ys.0, xs.1, ys.1), true)
        }
    };
    let tb = &printer.table;
    let safe = (
        numeric::min(raw.0 + m, 0.0),
        numeric::min(raw.1 + m, 0.0),
        numeric::max(raw.2 - m, 0.0),
        numeric::max(raw.3 - m, 0.0),
    );
    Table {
        raw,
        safe,
        measured,
        allow_x: tb.overhang_x,
        allow_y: tb.overhang_y,
        table_x: tb.table_x.unwrap_or(raw.2),
        table_y: tb.table_y.unwrap_or(raw.3),
        table_given: tb.table_x.is_some() && tb.table_y.is_some(),
    }
}

pub fn overhang(r: i64, w: f64, h: f64, table: &Table) -> (f64, f64) {
    let (wp, hp) = pass_dims(r, w, h);
    (numeric::max(0.0, wp - table.table_x), numeric::max(0.0, hp - table.table_y))
}

pub fn rotation_allowed(r: i64, w: f64, h: f64, table: &Table) -> (bool, String) {
    if !table.table_given {
        return (true, String::new());
    }
    let (ox, oy) = overhang(r, w, h, table);
    let mut bad = Vec::new();
    if ox > OVERHANG_TOL && !table.allow_x {
        bad.push(format!("выступает за стол на {ox:.0} мм в +X"));
    }
    if oy > OVERHANG_TOL && !table.allow_y {
        bad.push(format!("выступает за стол на {oy:.0} мм в +Y"));
    }
    (bad.is_empty(), bad.join(" и "))
}

pub fn pass_rect(r: i64, w: f64, h: f64, safe: Rect) -> Option<Rect> {
    let (wp, hp) = pass_dims(r, w, h);
    intersect(safe, (0.0, 0.0, wp, hp)).map(|rc| rect_from_pass(rc, r, w, h))
}

fn sorted_unique(values: impl IntoIterator<Item = f64>) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    for v in values {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    out
}

pub fn uncovered(target: Rect, rects: &[Rect]) -> Vec<Rect> {
    let rects: Vec<Rect> = rects.iter().filter_map(|&q| intersect(target, q)).collect();
    let xs = sorted_unique([target.0, target.2].into_iter().chain(rects.iter().flat_map(|r| [r.0, r.2])));
    let ys = sorted_unique([target.1, target.3].into_iter().chain(rects.iter().flat_map(|r| [r.1, r.3])));
    let mut out = Vec::new();
    for yw in ys.windows(2) {
        let (y0, y1) = (yw[0], yw[1]);
        if y1 - y0 <= EPS {
            continue;
        }
        let cy = (y0 + y1) / 2.0;
        let mut run: Option<Rect> = None;
        for xw in xs.windows(2) {
            let (x0, x1) = (xw[0], xw[1]);
            if x1 - x0 <= EPS {
                continue;
            }
            let cx = (x0 + x1) / 2.0;
            let hit = rects.iter().any(|r| r.0 <= cx && cx <= r.2 && r.1 <= cy && cy <= r.3);
            if hit {
                if let Some(rn) = run.take() {
                    out.push(rn);
                }
            } else {
                run = Some(match run {
                    Some(rn) => (rn.0, y0, x1, y1),
                    None => (x0, y0, x1, y1),
                });
            }
        }
        if let Some(rn) = run {
            out.push(rn);
        }
    }
    out
}

pub fn area(rects: &[Rect]) -> f64 {
    numeric::sum(rects.iter().map(|r| (r.2 - r.0) * (r.3 - r.1)))
}
