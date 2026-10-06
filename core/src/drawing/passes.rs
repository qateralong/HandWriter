use indexmap::IndexMap;

use crate::drawing::place::Rect;
use crate::geometry::Point;
use crate::numeric::{self, format_g};
use crate::settings::{A3Sheet, MarkedSheet, Printer};

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

pub fn segments(strokes: &[Vec<Point>]) -> Vec<(Point, Point)> {
    let mut out = Vec::new();
    for st in strokes {
        if st.len() == 1 {
            out.push((st[0], st[0]));
        }
        out.extend(st.windows(2).map(|w| (w[0], w[1])));
    }
    out
}

fn np_max(a: f64, b: f64) -> f64 {
    if a >= b || a.is_nan() { a } else { b }
}

fn np_min(a: f64, b: f64) -> f64 {
    if a <= b || a.is_nan() { a } else { b }
}

fn segment_covered(a: Point, b: Point, rects: &[Rect], eps: f64) -> bool {
    let d = (b.0 - a.0, b.1 - a.1);
    let mut spans: Vec<(f64, f64)> = Vec::with_capacity(rects.len());
    for &(x0, y0, x1, y1) in rects {
        let (mut t0, mut t1) = (0.0, 1.0);
        let mut valid = true;
        for (p, q) in [(-d.0, a.0 - x0), (d.0, x1 - a.0), (-d.1, a.1 - y0), (d.1, y1 - a.1)] {
            let q = q + eps;
            let zero = p.abs() < 1e-15;
            if zero && q < 0.0 {
                valid = false;
            }
            let r = if zero { 0.0 } else { q / p };
            if p < 0.0 && !zero {
                t0 = np_max(t0, r);
            }
            if p > 0.0 && !zero {
                t1 = np_min(t1, r);
            }
        }
        valid &= t0 <= t1 + 1e-12;
        spans.push(if valid { (t0, t1) } else { (f64::INFINITY, f64::NEG_INFINITY) });
    }
    spans.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or_else(|| x.0.is_nan().cmp(&y.0.is_nan())));
    let mut reach = 0.0;
    for (t0, t1) in spans {
        if t0 <= reach + 1e-9 {
            reach = np_max(reach, t1);
        }
    }
    reach >= 1.0 - 1e-9
}

pub fn covered_mask(segs: &[(Point, Point)], rects: &[Rect]) -> Vec<bool> {
    if rects.is_empty() {
        return vec![false; segs.len()];
    }
    segs.iter().map(|&(a, b)| segment_covered(a, b, rects, 1e-7)).collect()
}

pub fn lines_covered(strokes: &[Vec<Point>], rects: &[Rect]) -> bool {
    covered_mask(&segments(strokes), rects).into_iter().all(|c| c)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub w: f64,
    pub h: f64,
    pub allowed: Vec<i64>,
    pub rects: IndexMap<i64, Rect>,
    pub rotations: Option<Vec<i64>>,
    pub uncovered: Vec<Rect>,
    pub message: String,
    pub notes: Vec<String>,
}

impl Plan {
    pub fn ok(&self) -> bool {
        self.rotations.is_some()
    }

    pub fn describe(&self, rotations: Option<&[i64]>) -> String {
        let rs = rotations.or(self.rotations.as_deref()).unwrap_or(&[]);
        rs.iter()
            .enumerate()
            .map(|(i, &r)| format!("{}) {r}°, в упоре угол {} ({})", i + 1, corner(r), corner_name(corner(r))))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

pub type RotationRects = (Vec<i64>, IndexMap<i64, Rect>, Vec<String>);

pub type PassOffset<'a> = &'a dyn Fn(i64) -> (f64, f64);

fn no_offset(_: i64) -> (f64, f64) {
    (0.0, 0.0)
}

pub fn rotation_rects(w: f64, h: f64, printer: &Printer) -> RotationRects {
    rotation_rects_off(w, h, printer, &no_offset)
}

pub fn rotation_rects_off(w: f64, h: f64, printer: &Printer, off: PassOffset) -> RotationRects {
    let (mut allowed, mut rects, mut notes) = (Vec::new(), IndexMap::new(), Vec::new());
    for r in ROTATIONS {
        let tb = make_table(printer, w, h);
        let (ok, why) = rotation_allowed(r, w, h, &tb);
        if !ok {
            notes.push(format!("{r}°: {why}"));
            continue;
        }
        let (dx, dy) = off(r);
        let safe = (tb.safe.0 - dx, tb.safe.1 - dy, tb.safe.2 - dx, tb.safe.3 - dy);
        let Some(rc) = pass_rect(r, w, h, safe) else {
            notes.push(format!("{r}°: окно достижимости не заходит на лист"));
            continue;
        };
        allowed.push(r);
        rects.insert(r, rc);
    }
    (allowed, rects, notes)
}

fn combinations(items: &[i64], k: usize) -> Vec<Vec<i64>> {
    if k == 0 {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        for mut rest in combinations(&items[i + 1..], k - 1) {
            rest.insert(0, items[i]);
            out.push(rest);
        }
    }
    out
}

pub fn min_cover(allowed: &[i64], covers: impl Fn(&[i64]) -> bool) -> Option<Vec<i64>> {
    for k in 1..=allowed.len() {
        for combo in combinations(allowed, k) {
            if covers(&combo) {
                return Some(combo);
            }
        }
    }
    None
}

fn covers_target(target: Rect, rects: &IndexMap<i64, Rect>, rs: &[i64]) -> bool {
    let list: Vec<Rect> = rs.iter().map(|r| rects[r]).collect();
    uncovered(target, &list).is_empty()
}

pub fn plan_sheet(w: f64, h: f64, target: Rect, printer: &Printer, what: &str) -> Plan {
    let (allowed, rects, notes) = rotation_rects(w, h, printer);
    let mut plan = Plan {
        w,
        h,
        allowed: allowed.clone(),
        rects: rects.clone(),
        rotations: None,
        uncovered: Vec::new(),
        message: String::new(),
        notes: notes.clone(),
    };
    if let Some(combo) = min_cover(&allowed, |rs| covers_target(target, &rects, rs)) {
        let list: Vec<Rect> = combo.iter().map(|r| rects[r]).collect();
        plan.uncovered = uncovered((0.0, 0.0, w, h), &list);
        plan.rotations = Some(combo);
        return plan;
    }
    let all: Vec<Rect> = allowed.iter().map(|r| rects[r]).collect();
    plan.uncovered = uncovered(target, &all);
    plan.message = explain(w, h, target, printer, &allowed, &notes, what);
    plan
}

fn grow(printer: &Printer, side: &str, d: f64) -> Printer {
    let mut p = printer.clone();
    let tb = make_table(printer, 1.0, 1.0);
    if p.table.table_x.is_none() {
        p.table.table_x = Some(tb.table_x);
    }
    if p.table.table_y.is_none() {
        p.table.table_y = Some(tb.table_y);
    }
    let axis_x = side.starts_with('x');
    let flip = if axis_x { p.flip_x } else { p.flip_y };
    let grow_max = side.ends_with('+') != flip;
    let t = p.travel.as_mut().expect("deficit needs a measured travel");
    let delta = if grow_max { d } else { -d };
    match (axis_x, grow_max) {
        (true, true) => t.x_max += delta,
        (true, false) => t.x_min += delta,
        (false, true) => t.y_max += delta,
        (false, false) => t.y_min += delta,
    }
    p
}

fn covers_with(printer: &Printer, w: f64, h: f64, target: Rect, allowed_fixed: Option<&[i64]>) -> Option<Vec<i64>> {
    let (mut allowed, rects, _) = rotation_rects(w, h, printer);
    if let Some(fixed) = allowed_fixed {
        allowed.retain(|r| fixed.contains(r));
    }
    min_cover(&allowed, |rs| covers_target(target, &rects, rs))
}

fn deficit(printer: &Printer, w: f64, h: f64, target: Rect, allowed: &[i64], side: &str, limit: f64) -> Option<f64> {
    if printer.travel.is_none() || allowed.is_empty() {
        return None;
    }
    covers_with(&grow(printer, side, limit), w, h, target, Some(allowed))?;
    let (mut lo, mut hi) = (0.0, limit);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if covers_with(&grow(printer, side, mid), w, h, target, Some(allowed)).is_none() {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo < 0.05 {
            break;
        }
    }
    Some(hi)
}

fn capitalize(s: &str) -> String {
    let mut it = s.chars();
    match it.next() {
        Some(c) => c.to_uppercase().chain(it.flat_map(char::to_lowercase)).collect(),
        None => String::new(),
    }
}

fn degrees(rs: &[i64]) -> String {
    rs.iter().map(|r| format!("{r}°")).collect::<Vec<_>>().join(", ")
}

fn explain(w: f64, h: f64, target: Rect, printer: &Printer, allowed: &[i64], notes: &[String], what: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let size = format!("{}×{} мм", format_g(w, 6), format_g(h, 6));
    let what = capitalize(what);
    if allowed.is_empty() {
        parts.push(format!("{what} {size} не встаёт на стол ни в одном повороте: {}.", notes.join("; ")));
        if printer.table.table_x.is_none() || printer.table.table_y.is_none() {
            parts.push(
                "Размер стола не задан, край стола считается по краю окна достижимости; если стол больше, введи его \
                 размер от упора."
                    .into(),
            );
        }
    } else {
        parts.push(format!("{what} {size} не покрывается проходами (допустимые повороты: {}).", degrees(allowed)));
        let lim = w + h;
        let mut best: IndexMap<char, (f64, &str)> = IndexMap::new();
        for side in ["x+", "x-", "y+", "y-"] {
            if let Some(d) = deficit(printer, w, h, target, allowed, side, lim) {
                let ax = side.chars().next().expect("axis letter");
                if best.get(&ax).is_none_or(|b| d < b.0) {
                    best.insert(ax, (d, side));
                }
            }
        }
        if !best.is_empty() {
            let texts: Vec<String> = ['x', 'y']
                .iter()
                .filter_map(|ax| best.get(ax).map(|&(d, side)| (ax, d, side)))
                .map(|(ax, d, side)| {
                    let up = ax.to_ascii_uppercase();
                    let sign = if side.ends_with('+') { '+' } else { '−' };
                    format!("по оси {up} не хватает {d:.1} мм окна (в сторону {sign}{up})")
                })
                .collect();
            parts.push(format!("Не хватает: {}.", texts.join(" или ")));
        } else if printer.travel.is_some() {
            parts.push("Одним расширением окна по одной оси не исправить.".into());
        }
    }
    let tb = &printer.table;
    for (is_x, name) in [(true, "+X"), (false, "+Y")] {
        let allowed_now = if is_x { tb.overhang_x } else { tb.overhang_y };
        if !allowed_now {
            let mut p = printer.clone();
            if is_x {
                p.table.overhang_x = true;
            } else {
                p.table.overhang_y = true;
            }
            if let Some(combo) = covers_with(&p, w, h, target, None) {
                parts.push(format!(
                    "Включи «лист может выступать в сторону {name}» — будет проходов: {} ({}).",
                    combo.len(),
                    degrees(&combo)
                ));
            }
        }
    }
    if !tb.overhang_x && !tb.overhang_y {
        let mut p = printer.clone();
        p.table.overhang_x = true;
        p.table.overhang_y = true;
        if let Some(combo) = covers_with(&p, w, h, target, None)
            && !parts.iter().any(|s| s.contains("Включи"))
        {
            parts.push(format!("Включи обе стороны +X и +Y — будет проходов: {}.", combo.len()));
        }
    }
    parts.push(
        "Или уменьши: формат листа, рабочее поле (поля, рамку) или чертёж (кнопка «Подобрать масштаб под проходы»)."
            .into(),
    );
    parts.join(" ")
}

pub type Affine = [f64; 6];

pub fn marked_affine(mk: &MarkedSheet, w: f64, h: f64, r: i64) -> Affine {
    if w > h {
        let [a, b, c, d, e, f] = marked_base(mk, h, w, r);
        return [-b, a, -d, c, b * w + e, d * w + f];
    }
    marked_base(mk, w, h, r)
}

fn marked_base(mk: &MarkedSheet, w: f64, h: f64, r: i64) -> Affine {
    if r == 180 {
        return [-1.0, 0.0, 0.0, -1.0, mk.x_min + w, mk.y_max];
    }
    [1.0, 0.0, 0.0, 1.0, mk.x_min, mk.y_max - h]
}

pub const A3_RUNS: [i64; 4] = [0, 180, 90, 270];

pub fn a3_affine(z: &A3Sheet, w: f64, h: f64, r: i64) -> Affine {
    let (c, s): (i64, i64) = [(1, 0), (0, 1), (-1, 0), (0, -1)][(((r + 270) % 360) / 90) as usize];
    let (cf, sf) = (c as f64, s as f64);
    let pts = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(u, v): (f64, f64)| (cf * u - sf * v, sf * u + cf * v));
    let mx = pts.iter().map(|p| p.0).reduce(numeric::max).expect("four corners");
    let my = pts.iter().map(|p| p.1).reduce(numeric::max).expect("four corners");
    [c as f64, (-s) as f64, s as f64, c as f64, z.x_max - mx, z.y_max - my]
}

pub fn a3_rects(z: &A3Sheet, w: f64, h: f64) -> RotationRects {
    a3_rects_off(z, w, h, &no_offset)
}

pub fn a3_rects_off(z: &A3Sheet, w: f64, h: f64, off: PassOffset) -> RotationRects {
    let (mut allowed, mut rects, mut notes) = (Vec::new(), IndexMap::new(), Vec::new());
    for (i, r) in A3_RUNS.into_iter().enumerate() {
        let m = a3_affine(z, w, h, r);
        let (dx, dy) = off(r);
        let q0 = invert_affine(m, (z.x_min - dx, z.y_min - dy));
        let q1 = invert_affine(m, (z.x_max - dx, z.y_max - dy));
        let bx =
            (numeric::min(q0.0, q1.0), numeric::min(q0.1, q1.1), numeric::max(q0.0, q1.0), numeric::max(q0.1, q1.1));
        let Some(rc) = intersect(bx, (0.0, 0.0, w, h)) else {
            notes.push(format!("заход {}: зона не заходит на лист", i + 1));
            continue;
        };
        allowed.push(r);
        rects.insert(r, rc);
    }
    (allowed, rects, notes)
}

pub fn apply_affine(m: Affine, p: Point) -> Point {
    (m[0] * p.0 + m[1] * p.1 + m[4], m[2] * p.0 + m[3] * p.1 + m[5])
}

pub fn invert_affine(m: Affine, q: Point) -> Point {
    let det = m[0] * m[3] - m[1] * m[2];
    let (x, y) = (q.0 - m[4], q.1 - m[5]);
    ((m[3] * x - m[1] * y) / det, (-m[2] * x + m[0] * y) / det)
}

pub fn marked_rects(mk: &MarkedSheet, w: f64, h: f64) -> RotationRects {
    marked_rects_off(mk, w, h, &no_offset)
}

pub fn marked_rects_off(mk: &MarkedSheet, w: f64, h: f64, off: PassOffset) -> RotationRects {
    let (mut allowed, mut rects, mut notes) = (Vec::new(), IndexMap::new(), Vec::new());
    for r in [0, 180] {
        let m = marked_affine(mk, w, h, r);
        let (_, dy) = off(r);
        let (a, b, c) = (m[2], m[3], m[5] - (mk.y_min - dy));
        let rc = if b.abs() >= a.abs() {
            let bounds = [0.0, w].map(|u| (-c - a * u) / b);
            if b > 0.0 {
                (0.0, numeric::max(0.0, numeric::max(bounds[0], bounds[1])), w, h)
            } else {
                (0.0, 0.0, w, numeric::min(h, numeric::min(bounds[0], bounds[1])))
            }
        } else {
            let bounds = [0.0, h].map(|v| (-c - b * v) / a);
            if a > 0.0 {
                (numeric::max(0.0, numeric::max(bounds[0], bounds[1])), 0.0, w, h)
            } else {
                (0.0, 0.0, numeric::min(w, numeric::min(bounds[0], bounds[1])), h)
            }
        };
        if rc.3 - rc.1 <= EPS || rc.2 - rc.0 <= EPS {
            notes.push(format!("{r}°: лист не заходит выше линии нуля"));
            continue;
        }
        allowed.push(r);
        rects.insert(r, rc);
    }
    (allowed, rects, notes)
}
