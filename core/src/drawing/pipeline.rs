use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Value, json};

use crate::checks::{check_bounds, check_printer, to_machine};
use crate::drawing::model::{DPath, ImportResult, Mask};
use crate::drawing::ops::{dash_polyline, dedupe, expand_passes, join_paths, order_paths, outside_parts};
use crate::drawing::passes::{
    self, A3_RUNS, Affine, Plan, a3_affine, a3_rects, apply_affine, corner, corner_name, corner_point, covered_mask,
    invert_affine, make_table, marked_affine, marked_rects, pass_dims, plan_sheet, rotation_rects, segments, to_pass,
    uncovered,
};
use crate::drawing::place::{
    AnchorKind, Placement, Rect, SheetLayout, anchor_of, best_fit, fixed_mode, place, reach_areas, scale_label,
    sheet_layout, shrink,
};
use crate::drawing::sources::BUILTIN_TEST;
use crate::drawing::split::{CutStats, Geometry, Mark, Node, control_marks, cut_stroke, mark_strokes, solve};
use crate::gcode::{compute_stats, fmt, generate_gcode};
use crate::geometry::{Point, rdp};
use crate::numeric::{self, format_g, round_to};
use crate::settings::{DrawingImport, FillDir, LayerWeight, Orientation, Settings, SheetFormat, Travel};

pub const MIN_DASH_PERIOD: f64 = 0.5;
pub const TEXT_LIST_LIMIT: usize = 12;

pub type Loader<'a> = dyn Fn(&str, &DrawingImport, f64) -> ImportResult + 'a;

fn corner_en(c: &str) -> &'static str {
    match c {
        "A" => "bottom left",
        "B" => "bottom right",
        "C" => "top right",
        _ => "top left",
    }
}

fn enum_name<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|x| x.as_str().map(String::from)).unwrap_or_default()
}

fn degrees(rs: &[i64]) -> String {
    rs.iter().map(|r| format!("{r}°")).collect::<Vec<_>>().join(", ")
}

#[derive(Debug, Clone, PartialEq)]
pub enum SheetMap {
    Identity,
    Pass { rotation: i64, width: f64, height: f64 },
    Affine(crate::drawing::passes::Affine),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MachineMap {
    pub flip_x: bool,
    pub flip_y: bool,
    pub dx: f64,
    pub dy: f64,
    pub map: SheetMap,
}

impl MachineMap {
    pub fn for_text(s: &Settings) -> MachineMap {
        MachineMap { flip_x: s.printer.flip_x, flip_y: s.printer.flip_y, dx: 0.0, dy: 0.0, map: SheetMap::Identity }
    }

    pub fn to_sheet(&self, (x, y): Point) -> Point {
        let q = (if self.flip_x { -x } else { x } - self.dx, if self.flip_y { -y } else { y } - self.dy);
        match self.map {
            SheetMap::Identity => q,
            SheetMap::Pass { rotation, width, height } => crate::drawing::passes::from_pass(q, rotation, width, height),
            SheetMap::Affine(m) => crate::drawing::passes::invert_affine(m, q),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PassPart {
    pub index: usize,
    pub rotation: i64,
    pub region: Rect,
    pub strokes: Vec<Vec<Point>>,
    pub thick: Vec<bool>,
    pub marks: usize,
    pub dx: f64,
    pub dy: f64,
    pub errors: Vec<String>,
}

impl PassPart {
    pub fn corner(&self) -> &'static str {
        corner(self.rotation)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextOnSheet {
    pub x: f64,
    pub y: f64,
    pub text: String,
    pub kind: String,
}

#[derive(Debug, Clone)]
pub struct DrawingComposition {
    pub settings: Settings,
    pub sheet_settings: Option<Settings>,
    pub imp: Option<ImportResult>,
    pub layout: Option<SheetLayout>,
    pub placement: Option<Placement>,
    pub reach_measured: bool,
    pub allowed: Vec<i64>,
    pub pass_rects: IndexMap<i64, Rect>,
    pub rotation_notes: Vec<String>,
    pub sheet_plan: Option<Plan>,
    pub frame_fitted: bool,
    pub drawing_passes: Option<Vec<i64>>,
    pub parts: Vec<PassPart>,
    pub split: Option<Node>,
    pub marks: Vec<Mark>,
    pub cut_stats: Option<CutStats>,
    pub source_strokes: Vec<Vec<Point>>,
    pub strokes: Vec<Vec<Point>>,
    pub thick: Vec<bool>,
    pub stroke_part: Vec<usize>,
    pub unreachable: Vec<Vec<Point>>,
    pub texts: Vec<TextOnSheet>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub reach_percent: Option<f64>,
    pub passes_percent: Option<f64>,
    pub dashed_solid: usize,
}

impl DrawingComposition {
    fn new(settings: Settings) -> Self {
        Self {
            settings,
            sheet_settings: None,
            imp: None,
            layout: None,
            placement: None,
            reach_measured: false,
            allowed: Vec::new(),
            pass_rects: IndexMap::new(),
            rotation_notes: Vec::new(),
            sheet_plan: None,
            frame_fitted: false,
            drawing_passes: None,
            parts: Vec::new(),
            split: None,
            marks: Vec::new(),
            cut_stats: None,
            source_strokes: Vec::new(),
            strokes: Vec::new(),
            thick: Vec::new(),
            stroke_part: Vec::new(),
            unreachable: Vec::new(),
            texts: Vec::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            reach_percent: None,
            passes_percent: None,
            dashed_solid: 0,
        }
    }

    pub fn rotation(&self) -> Option<i64> {
        (self.parts.len() == 1).then(|| self.parts[0].rotation)
    }

    pub fn reach(&self) -> Option<Rect> {
        self.pass_rects.get(&self.rotation().unwrap_or(0)).copied()
    }

    pub fn marked(&self) -> bool {
        fixed_mode(&self.settings.drawing)
    }

    fn layout(&self) -> &SheetLayout {
        self.layout.as_ref().expect("layout is set")
    }

    pub fn affine(&self, r: i64) -> Affine {
        let ds = &self.settings.drawing;
        let lay = self.layout();
        if ds.a3.enabled {
            a3_affine(&ds.a3, lay.width, lay.height, r)
        } else {
            marked_affine(&ds.marked, lay.width, lay.height, r)
        }
    }

    pub fn to_machine(&self, p: Point, r: i64) -> Point {
        if self.marked() {
            return apply_affine(self.affine(r), p);
        }
        let lay = self.layout();
        to_pass(p, r, lay.width, lay.height)
    }

    pub fn start_point(&self, r: i64) -> Point {
        if self.marked() {
            return invert_affine(self.affine(r), (0.0, 0.0));
        }
        let lay = self.layout();
        corner_point(r, lay.width, lay.height)
    }

    pub fn machine_map(&self, part: &PassPart) -> MachineMap {
        let s = self.part_settings(part.rotation);
        let lay = self.layout();
        let map = if self.marked() {
            SheetMap::Affine(self.affine(part.rotation))
        } else {
            SheetMap::Pass { rotation: part.rotation, width: lay.width, height: lay.height }
        };
        MachineMap { flip_x: s.printer.flip_x, flip_y: s.printer.flip_y, dx: part.dx, dy: part.dy, map }
    }

    pub fn part_strokes(&self, part: &PassPart, strokes: Option<&[Vec<Point>]>) -> Vec<Vec<Point>> {
        strokes
            .unwrap_or(&part.strokes)
            .iter()
            .map(|st| {
                st.iter()
                    .map(|&p| {
                        let q = self.to_machine(p, part.rotation);
                        (q.0 + part.dx, q.1 + part.dy)
                    })
                    .collect()
            })
            .collect()
    }

    pub fn pass_strokes(&self) -> Vec<Vec<Point>> {
        if self.parts.len() == 1 { self.part_strokes(&self.parts[0], None) } else { self.strokes.clone() }
    }

    pub fn part_settings(&self, rotation: i64) -> Settings {
        let mut s = self.sheet_settings.clone().expect("sheet settings are set");
        let lay = self.layout();
        (s.sheet.width, s.sheet.height) = pass_dims(rotation, lay.width, lay.height);
        s.printer.use_work_area = true;
        if self.marked() {
            let ds = &self.settings.drawing;
            let (x_min, x_max, y_min, y_max) = if ds.a3.enabled {
                (ds.a3.x_min, ds.a3.x_max, ds.a3.y_min, ds.a3.y_max)
            } else {
                (ds.marked.x_min, ds.marked.x_max, ds.marked.y_min, ds.marked.y_max)
            };
            s.printer.safety_margin = 0.0;
            s.printer.travel = Some(Travel {
                x_min: numeric::min(x_min, 0.0),
                x_max: numeric::max(x_max, 0.0),
                y_min: numeric::min(y_min, 0.0),
                y_max: numeric::max(y_max, 0.0),
            });
        }
        s
    }

    pub fn pass_settings(&self) -> Settings {
        match self.rotation() {
            Some(r) => self.part_settings(r),
            None => self.sheet_settings.clone().expect("sheet settings are set"),
        }
    }
}

pub fn is_thick(p: &DPath, s: &Settings) -> bool {
    let w = &s.drawing.weights;
    if !w.enabled {
        return false;
    }
    let mode = if p.layer.is_empty() { LayerWeight::Auto } else { w.layers.get(&p.layer).copied().unwrap_or_default() };
    if mode != LayerWeight::Auto {
        return mode == LayerWeight::Thick;
    }
    p.width.is_some_and(|width| width > w.threshold + 1e-9)
}

pub fn field_rect(lay: &SheetLayout, s: &Settings) -> Rect {
    if let Some(inner) = lay.inner {
        return inner;
    }
    let m = s.drawing.placement.margin;
    (m, m, lay.width - m, lay.height - m)
}

fn interp(x: f64, xp: &[f64], fp: &[f64]) -> f64 {
    let n = xp.len();
    if x < xp[0] {
        return fp[0];
    }
    if x > xp[n - 1] {
        return fp[n - 1];
    }
    if x == xp[n - 1] {
        return fp[n - 1];
    }
    let j = xp.partition_point(|&v| v <= x) - 1;
    if j == n - 1 || xp[j] == x {
        return fp[j];
    }
    let slope = (fp[j + 1] - fp[j]) / (xp[j + 1] - xp[j]);
    let mut r = slope * (x - xp[j]) + fp[j];
    if r.is_nan() {
        r = slope * (x - xp[j + 1]) + fp[j + 1];
        if r.is_nan() && fp[j] == fp[j + 1] {
            r = fp[j];
        }
    }
    r
}

fn band_map(lo: f64, hi: f64, busy: &[(f64, f64)], target: f64) -> Option<(Vec<f64>, Vec<f64>)> {
    let mut spans: Vec<(f64, f64)> = busy
        .iter()
        .filter(|(a, b)| *b > lo && *a < hi)
        .map(|&(a, b)| (numeric::max(a, lo), numeric::min(b, hi)))
        .collect();
    spans.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (a, b) in spans {
        match merged.last_mut() {
            Some(m) if a <= m.1 => m.1 = numeric::max(m.1, b),
            _ => merged.push((a, b)),
        }
    }
    let empty = (hi - lo) - numeric::sum(merged.iter().map(|(a, b)| b - a));
    let room = empty + target - (hi - lo);
    if empty <= 1e-6 || room < 0.0 {
        return None;
    }
    let k = room / empty;
    let (mut src, mut dst) = (vec![lo], vec![0.0]);
    let (mut cur, mut pos) = (lo, 0.0);
    for (a, b) in merged {
        pos += (a - cur) * k;
        src.extend([a, b]);
        dst.extend([pos, pos + b - a]);
        pos += b - a;
        cur = b;
    }
    src.push(hi);
    dst.push(pos + (hi - cur) * k);
    Some((src, dst))
}

fn min_max(v: impl Iterator<Item = f64> + Clone) -> (f64, f64) {
    (v.clone().reduce(numeric::min).expect("non-empty"), v.reduce(numeric::max).expect("non-empty"))
}

pub fn fit_frame_to_margins(imp: &ImportResult, tw: f64, th: f64) -> Option<ImportResult> {
    let (x0, y0, x1, y1) = imp.bbox()?;
    if imp.fill_mask.is_some() || imp.paths.is_empty() {
        return None;
    }
    let eps = 0.002 * numeric::max(x1 - x0, y1 - y0);
    let on_edge = |p: &DPath| {
        p.points.iter().all(|&(x, _)| numeric::min((x - x0).abs(), (x - x1).abs()) < eps)
            || p.points.iter().all(|&(_, y)| numeric::min((y - y0).abs(), (y - y1).abs()) < eps)
    };
    let inner: Vec<&DPath> = imp.paths.iter().filter(|p| !on_edge(p)).collect();
    if inner.len() == imp.paths.len() {
        return None;
    }
    let bx: Vec<(f64, f64)> = inner.iter().map(|p| min_max(p.points.iter().map(|q| q.0))).collect();
    let by: Vec<(f64, f64)> = inner.iter().map(|p| min_max(p.points.iter().map(|q| q.1))).collect();
    let (sx, dx) = band_map(x0, x1, &bx, tw)?;
    let (sy, dy) = band_map(y0, y1, &by, th)?;
    let fx = |v: f64| interp(v, &sx, &dx);
    let fy = |v: f64| interp(v, &sy, &dy);
    let mut out = imp.clone();
    for p in &mut out.paths {
        p.points = p.points.iter().map(|&(x, y)| (fx(x), fy(y))).collect();
    }
    for t in &mut out.texts {
        t.x = fx(t.x);
        t.y = fy(t.y);
    }
    Some(out)
}

fn file_segments(imp: &ImportResult) -> Vec<(Point, Point)> {
    let mut lines: Vec<Vec<Point>> = imp.paths.iter().map(|p| p.points.clone()).collect();
    if let Some((x0, y0, x1, y1)) = imp.fill_bbox() {
        lines.push(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]);
    }
    segments(&lines)
}

fn combinations<T: Clone>(items: &[T], k: usize) -> Vec<Vec<T>> {
    if k == 0 {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        for mut rest in combinations(&items[i + 1..], k - 1) {
            rest.insert(0, items[i].clone());
            out.push(rest);
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub fn fit_to_passes(
    imp: &ImportResult,
    bbox: Rect,
    area: Rect,
    s_max: f64,
    rects: &[Rect],
    pad: f64,
    dx: f64,
    dy: f64,
    anchor: AnchorKind,
    areas: i64,
) -> f64 {
    let rects: Vec<Rect> = rects.iter().map(|&q| shrink(q, pad)).filter(|r| r.2 > r.0 && r.3 > r.1).collect();
    if rects.is_empty() || s_max <= 0.0 {
        return 0.0;
    }
    let groups: Vec<Vec<Rect>> = if 0 < areas && (areas as usize) < rects.len() {
        combinations(&rects, areas as usize)
    } else {
        vec![rects.clone()]
    };
    let segs = file_segments(imp);
    let (cxa, cya) = ((area.0 + area.2) / 2.0, (area.1 + area.3) / 2.0);
    let (cxb, cyb) = ((bbox.0 + bbox.2) / 2.0, (bbox.1 + bbox.3) / 2.0);
    let ok = |sc: f64| {
        let t = match anchor {
            AnchorKind::Zero => (-bbox.0 * sc + pad + dx, -bbox.1 * sc + pad + dy),
            AnchorKind::Left => (area.0 - bbox.0 * sc + dx, cya - cyb * sc + dy),
            AnchorKind::Center => (cxa - cxb * sc + dx, cya - cyb * sc + dy),
        };
        let moved: Vec<(Point, Point)> =
            segs.iter().map(|&(a, b)| ((a.0 * sc + t.0, a.1 * sc + t.1), (b.0 * sc + t.0, b.1 * sc + t.1))).collect();
        groups.iter().any(|g| covered_mask(&moved, g).into_iter().all(|c| c))
    };
    if ok(s_max) {
        return s_max;
    }
    let (mut lo, mut hi) = (0.0, s_max);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if ok(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo < s_max * 1e-4 {
            break;
        }
    }
    if lo > 0.0 && ok(lo) { lo } else { 0.0 }
}

pub fn hatch_mask(mask: Option<&Mask>, px: f64, pl: &Placement, step: f64, dir: FillDir) -> Vec<Vec<Point>> {
    let Some(mask) = mask.filter(|m| m.any()) else { return Vec::new() };
    if pl.scale <= 0.0 {
        return Vec::new();
    }
    let horizontal = match dir {
        FillDir::Horizontal => true,
        FillDir::Vertical => false,
        FillDir::Auto => {
            let (rows, cols) = (mask.rows, mask.cols);
            let mut sh = 0i64;
            for r in 0..rows {
                for c in 0..cols {
                    if mask.get(r, c) && (c == 0 || !mask.get(r, c - 1)) {
                        sh += 1;
                    }
                }
            }
            let mut sv = 0i64;
            for c in 0..cols {
                for r in 0..rows {
                    if mask.get(r, c) && (r == 0 || !mask.get(r - 1, c)) {
                        sv += 1;
                    }
                }
            }
            let starts_h = sh as f64 / rows as f64;
            let starts_v = sv as f64 / cols as f64;
            starts_h * rows as f64 <= starts_v * cols as f64
        }
    };
    let m = if horizontal { mask.clone() } else { mask.transposed() };
    let n_lines = m.rows as f64;
    let step_px = numeric::max(step / pl.scale / px, 1e-6);
    let mut out = Vec::new();
    let mut k = 0;
    let mut pos = step_px / 2.0;
    while pos < n_lines {
        let row = pos as usize;
        let mut segs: Vec<Vec<Point>> = Vec::new();
        let mut c = 0;
        while c < m.cols {
            if !m.get(row, c) {
                c += 1;
                continue;
            }
            let a = c;
            while c < m.cols && m.get(row, c) {
                c += 1;
            }
            let b = c;
            let (p0, p1) = if horizontal {
                let y = -pos * px;
                (pl.apply((a as f64 * px, y)), pl.apply((b as f64 * px, y)))
            } else {
                let x = pos * px;
                (pl.apply((x, (-(a as i64)) as f64 * px)), pl.apply((x, (-(b as i64)) as f64 * px)))
            };
            segs.push(vec![p0, p1]);
        }
        if !segs.is_empty() {
            if k % 2 == 1 {
                segs.reverse();
                for s in &mut segs {
                    s.reverse();
                }
            }
            out.extend(segs);
            k += 1;
        }
        pos += step_px;
    }
    out
}

fn short(t: &str, n: usize) -> String {
    if t.chars().count() <= n { t.to_string() } else { t.chars().take(n - 1).collect::<String>() + "…" }
}

struct Setup {
    lay: SheetLayout,
    allowed: Vec<i64>,
    rects: IndexMap<i64, Rect>,
    notes: Vec<String>,
    pl: Placement,
}

fn setup(s: &Settings, imp: &ImportResult, bbox: Rect, pad: f64) -> Setup {
    let ds = &s.drawing;
    let lay = sheet_layout(ds, Some(bbox));
    let (allowed, rects, notes) = if ds.a3.enabled {
        a3_rects(&ds.a3, lay.width, lay.height)
    } else if ds.marked.enabled {
        marked_rects(&ds.marked, lay.width, lay.height)
    } else {
        rotation_rects(lay.width, lay.height, &s.printer)
    };
    let mut windows: Vec<Rect> = allowed.iter().map(|r| rects[r]).collect();
    if anchor_of(ds) == AnchorKind::Zero
        && let Some(&w0) = rects.get(&0)
    {
        windows = vec![w0];
    }
    let mut pl = place(ds, bbox, &lay, &windows, pad);
    let p = &ds.placement;
    if p.scale_mode == crate::settings::ScaleMode::FitPasses && pl.errors.is_empty() {
        let area = pl.area.expect("placement has an area");
        let anchor = anchor_of(ds);
        let sc = fit_to_passes(imp, bbox, area, pl.scale, &windows, pad, p.dx, p.dy, anchor, ds.split.areas);
        if sc <= 0.0 {
            pl.errors.push(
                "Подобрать масштаб под проходы не получается: середина поля листа не достаётся ни в одном допустимом \
                 проходе. Сдвинь чертёж (dx, dy), разреши другую сторону или проверь окно достижимости"
                    .into(),
            );
        } else if sc < pl.scale && anchor == AnchorKind::Left {
            let cy = (area.1 + area.3) / 2.0 - (bbox.1 + bbox.3) / 2.0 * sc;
            pl = Placement { scale: sc, tx: area.0 - bbox.0 * sc + p.dx, ty: cy + p.dy, ..pl };
        } else if sc < pl.scale && anchor == AnchorKind::Zero {
            pl = Placement { scale: sc, tx: -bbox.0 * sc + pad + p.dx, ty: -bbox.1 * sc + pad + p.dy, ..pl };
        } else if sc < pl.scale {
            let k = sc / pl.scale;
            let (cx, cy) = ((area.0 + area.2) / 2.0, (area.1 + area.3) / 2.0);
            pl = Placement {
                scale: sc,
                tx: cx + (pl.tx - cx) * k + p.dx * (1.0 - k),
                ty: cy + (pl.ty - cy) * k + p.dy * (1.0 - k),
                ..pl
            };
        }
    }
    Setup { lay, allowed, rects, notes, pl }
}

pub fn compose_drawing(s: &Settings, loader: &Loader) -> DrawingComposition {
    let mut s = s.clone();
    if s.drawing.a3.enabled {
        s.drawing.marked.enabled = false;
        s.drawing.split.areas = 0;
        let sh = &mut s.drawing.sheet;
        (sh.format, sh.orientation, sh.width, sh.height) = (SheetFormat::Custom, Orientation::Landscape, 420.0, 297.0);
        s.printer.travel = None;
    } else if s.drawing.marked.enabled {
        let (length, span) =
            (s.drawing.marked.length, numeric::max(s.drawing.marked.x_max - s.drawing.marked.x_min, 1.0));
        s.drawing.split.areas = 0;
        let sh = &mut s.drawing.sheet;
        (sh.format, sh.orientation) = (SheetFormat::Custom, Orientation::Landscape);
        s.printer.travel = None;
        (sh.width, sh.height) = (length, span);
    }
    let ds = s.drawing.clone();
    let mut c = DrawingComposition::new(s.clone());
    let tol = ds.paths.curve_tol;
    let mut imp = loader(&ds.file, &ds.imp, tol);
    c.warnings.extend(imp.warnings.iter().cloned());
    if !imp.errors.is_empty() {
        c.errors.extend(imp.errors.iter().cloned());
        c.imp = Some(imp);
        return c;
    }
    let mut bbox = imp.bbox();
    if bbox.is_some()
        && fixed_mode(&ds)
        && !ds.frame.enabled
        && matches!(ds.placement.scale_mode, crate::settings::ScaleMode::Fit | crate::settings::ScaleMode::FitPasses)
    {
        let f = &ds.frame;
        let (pw, ph) =
            if ds.a3.enabled { (420.0, 297.0) } else { (ds.marked.length, ds.marked.x_max - ds.marked.x_min) };
        if let Some(fitted) = fit_frame_to_margins(&imp, pw - f.left - f.right, ph - f.top - f.bottom) {
            bbox = fitted.bbox();
            imp = fitted;
            c.frame_fitted = true;
        }
    }
    let Some(mut bbox) = bbox else {
        let n_img = imp.texts.iter().filter(|t| t.kind == "image").count();
        let n_txt = imp.texts.len() - n_img;
        let mut extra = Vec::new();
        if n_txt > 0 {
            extra.push(format!("текст не в кривых: {n_txt}"));
        }
        if n_img > 0 {
            extra.push(format!("встроенные картинки: {n_img} (скан? загрузи его как PNG/JPG)"));
        }
        let tail = if extra.is_empty() { String::new() } else { format!(" ({})", extra.join("; ")) };
        c.errors.push(format!("В файле нет линий, которые можно нарисовать{tail}"));
        c.imp = Some(imp);
        return c;
    };

    let w = &ds.weights;
    let pad = if w.enabled { (w.passes - 1) as f64 / 2.0 * w.step * 2.0 } else { 0.0 };
    let mut st = setup(&s, &imp, bbox, pad);
    if st.pl.scale > 1.0001 && imp.kind != "raster" {
        let k = 2f64.powi(st.pl.scale.log2().ceil() as i32);
        let imp2 = loader(&ds.file, &ds.imp, tol / k);
        if imp2.errors.is_empty()
            && let Some(b2) = imp2.bbox()
        {
            imp = imp2;
            bbox = b2;
            st = setup(&s, &imp, bbox, pad);
        }
    }
    let Setup { lay, allowed, rects, notes, pl } = st;
    let (big_w, big_h) = (lay.width, lay.height);
    let mut s2 = s.clone();
    (s2.sheet.width, s2.sheet.height) = (big_w, big_h);
    c.allowed = allowed.clone();
    c.pass_rects = rects.clone();
    c.rotation_notes = notes;
    c.reach_measured = s.printer.travel.is_some();
    if ds.marked.enabled {
        let mk = &ds.marked;
        if mk.x_max <= mk.x_min || mk.y_max <= mk.y_min {
            c.errors.push("Рабочая зона: «до» должно быть больше «от» по X и по Y".into());
        }
    }
    let (e, warns) = check_printer(&s2);
    c.errors.extend(e);
    c.warnings.extend(warns.into_iter().filter(|m| !(fixed_mode(&ds) && m.contains("не измерен"))));
    if ds.a3.enabled && (ds.a3.x_max <= ds.a3.x_min || ds.a3.y_max <= ds.a3.y_min) {
        c.errors.push("Рабочая зона: «до» должно быть больше «от» по X и по Y".into());
    }
    c.errors.extend(pl.errors.iter().cloned());
    c.warnings.extend(pl.warnings.iter().cloned());
    c.sheet_plan = if fixed_mode(&ds) {
        None
    } else {
        Some(plan_sheet(big_w, big_h, field_rect(&lay, &s), &s.printer, "рабочее поле листа"))
    };
    c.sheet_settings = Some(s2);
    c.layout = Some(lay.clone());
    c.placement = Some(pl.clone());
    if !pl.errors.is_empty() {
        c.imp = Some(imp);
        return c;
    }

    let windows: Vec<Rect> = allowed.iter().map(|r| rects[r]).collect();
    let (s_reach, _) = best_fit(bbox, &reach_areas(&lay, &windows, pad));
    c.reach_percent = (s_reach > 0.0).then_some(s_reach * 100.0);

    let mut groups: BTreeMap<bool, Vec<Vec<Point>>> = BTreeMap::from([(false, Vec::new()), (true, Vec::new())]);
    for p in &imp.paths {
        let pts = dedupe(&p.points.iter().map(|&q| pl.apply(q)).collect::<Vec<_>>(), 1e-9);
        let thick = is_thick(p, &s);
        let mut pieces = vec![pts.clone()];
        if let Some(dash) = &p.dash {
            let pat: Vec<f64> = dash.iter().map(|v| v * pl.scale).collect();
            if numeric::sum(pat.iter().copied()) < MIN_DASH_PERIOD {
                c.dashed_solid += 1;
            } else {
                pieces = dash_polyline(&pts, &pat, p.dash_offset * pl.scale);
            }
        }
        groups.get_mut(&thick).expect("both groups").extend(pieces.into_iter().filter(|q| !q.is_empty()));
    }
    if c.dashed_solid > 0 {
        c.warnings.push(format!(
            "Штриховые линии с узором мельче {} мм на бумаге нарисованы сплошными: {}",
            format_g(MIN_DASH_PERIOD, 6),
            c.dashed_solid
        ));
    }
    for fl in &lay.frame {
        groups.get_mut(&(ds.weights.enabled && fl.thick)).expect("both groups").push(fl.points.clone());
    }
    if imp.fill_mask.is_some() {
        groups.get_mut(&false).expect("both groups").extend(hatch_mask(
            imp.fill_mask.as_ref(),
            imp.fill_px,
            &pl,
            ds.imp.fill_step,
            ds.imp.fill_dir,
        ));
    }

    let tol_rdp = s.printer.simplify_tol;
    let mut final_strokes: Vec<Vec<Point>> = Vec::new();
    let mut flags: Vec<bool> = Vec::new();
    for thick in [true, false] {
        for pts in join_paths(&groups[&thick], ds.paths.join_tol) {
            let pts = if pts.len() > 2 { rdp(&pts, tol_rdp) } else { pts };
            let closed = pts.len() > 2 && pts[0] == pts[pts.len() - 1];
            let passes =
                if thick { expand_passes(&pts, closed, ds.weights.passes, ds.weights.step) } else { vec![pts] };
            for q in passes {
                final_strokes.push(q);
                flags.push(thick);
            }
        }
    }

    for t in &imp.texts {
        let (x, y) = pl.apply((t.x, t.y));
        c.texts.push(TextOnSheet { x, y, text: t.text.clone(), kind: t.kind.clone() });
    }
    let txt: Vec<&TextOnSheet> = c.texts.iter().filter(|t| t.kind == "text").collect();
    let n_img = c.texts.iter().filter(|t| t.kind == "image").count();
    if !txt.is_empty() {
        let listed: Vec<String> = txt
            .iter()
            .take(TEXT_LIST_LIMIT)
            .map(|t| format!("«{}» (X {:.0}, Y {:.0})", short(&t.text, 40), t.x, t.y))
            .collect();
        let more = if txt.len() > TEXT_LIST_LIMIT {
            format!(" и ещё {}", txt.len() - TEXT_LIST_LIMIT)
        } else {
            String::new()
        };
        c.warnings.push(format!(
            "Текст не в кривых не рисуется ({} мест, координаты на листе, мм): {}{more}. Преврати текст в кривые в \
             редакторе, если он нужен на бумаге",
            txt.len(),
            listed.join("; ")
        ));
    }
    if n_img > 0 {
        c.warnings.push(format!("Встроенные картинки не рисуются: {n_img}"));
    }

    let off_sheet = final_strokes
        .iter()
        .flatten()
        .filter(|&&(x, y)| !(-1e-6 <= x && x <= big_w + 1e-6 && -1e-6 <= y && y <= big_h + 1e-6))
        .count();
    if off_sheet > 0 {
        c.errors.push(format!("Чертёж выходит за край листа ({off_sheet} точек): уменьши масштаб или сдвиг"));
    }

    c.source_strokes = final_strokes.clone();
    c.imp = Some(imp);
    let segs = segments(&final_strokes);
    if covered_mask(&segs, &windows).into_iter().all(|v| v) && !final_strokes.is_empty() {
        split_parts(&mut c, &final_strokes, &flags, big_w, big_h);
    } else {
        let ordered = order_paths(&final_strokes, ds.paths.long_path, (0.0, 0.0));
        c.thick = ordered.iter().map(|(i, _)| flags[*i]).collect();
        c.strokes = ordered.into_iter().map(|(_, q)| q).collect();
        c.stroke_part = vec![0; c.strokes.len()];
        if !final_strokes.is_empty() {
            c.unreachable = outside_all(&c.strokes, &windows);
            let msg = unreachable_message(&mut c, &s, bbox, pad, big_w, big_h, &lay);
            c.errors.push(msg);
        }
    }

    if c.errors.is_empty() {
        let mut all_errors = Vec::new();
        for i in 0..c.parts.len() {
            let part = c.parts[i].clone();
            let be = check_bounds(&c.part_strokes(&part, None), &c.part_settings(part.rotation), 5);
            if !be.is_empty() {
                let mut errs: Vec<String> =
                    be.iter().map(|m| format!("Проход {} ({}°): {m}", part.index, part.rotation)).collect();
                if part.dx != 0.0 || part.dy != 0.0 {
                    errs.push(format!(
                        "Проход {}: поправка dx {}, dy {} выводит линии за окно достижимости — уменьши поправку или масштаб",
                        part.index,
                        format_g(part.dx, 6),
                        format_g(part.dy, 6)
                    ));
                }
                c.parts[i].errors = errs.clone();
                all_errors.extend(errs);
            }
        }
        c.errors.extend(all_errors);
    }
    if final_strokes.is_empty() {
        c.errors.push("Нечего рисовать".into());
    }
    c
}

fn split_parts(c: &mut DrawingComposition, final_strokes: &[Vec<Point>], flags: &[bool], big_w: f64, big_h: f64) {
    let ds = c.settings.drawing.clone();
    let sp = &ds.split;
    let rects = c.pass_rects.clone();
    let segs = segments(final_strokes);
    let geo = Geometry::new(final_strokes);
    let sheet = (0.0, 0.0, big_w, big_h);
    let mut root: Option<Node> = None;
    let mut used_slack = true;
    let counts: Vec<usize> = if sp.areas != 0 { vec![sp.areas as usize] } else { (1..=c.allowed.len()).collect() };
    'outer: for k in counts {
        for combo in combinations(&c.allowed, k) {
            let rs: Vec<Rect> = combo.iter().map(|r| rects[r]).collect();
            if !covered_mask(&segs, &rs).into_iter().all(|v| v) {
                continue;
            }
            for margin in [sp.overlap + sp.slack, sp.overlap] {
                root = solve(&geo, &rects, &combo, sheet, sheet, margin, sp.overlap);
                if root.is_some() {
                    used_slack = margin > sp.overlap || sp.slack == 0.0;
                    break;
                }
            }
            if root.is_some() {
                break 'outer;
            }
        }
    }
    let Some(root) = root else {
        let k = (sp.areas as usize).min(c.allowed.len());
        let none_covers = !combinations(&c.allowed, k).iter().any(|combo| {
            let rs: Vec<Rect> = combo.iter().map(|r| rects[r]).collect();
            covered_mask(&segs, &rs).into_iter().all(|v| v)
        });
        if sp.areas != 0 && none_covers {
            c.errors.push(format!(
                "Выбрано рабочих областей: {} — их не хватает, чтобы достать весь чертёж. Поставь «авто» или больше \
                 областей, либо уменьши масштаб",
                sp.areas
            ));
        } else {
            c.errors.push(
                "Проходы вместе достают весь чертёж, но провести между ними прямые швы не получилось. Уменьши масштаб \
                 («Подобрать масштаб под проходы» с запасом) или сдвинь чертёж"
                    .into(),
            );
        }
        let ordered = order_paths(final_strokes, ds.paths.long_path, (0.0, 0.0));
        c.thick = ordered.iter().map(|(i, _)| flags[*i]).collect();
        c.strokes = ordered.into_iter().map(|(_, q)| q).collect();
        c.stroke_part = vec![0; c.strokes.len()];
        return;
    };
    if !used_slack {
        c.warnings.push(format!(
            "Запас у шва под сдвиг нуля ({} мм) не поместился: шов проведён только с нахлёстом {} мм, ставь ноль \
             особенно точно",
            format_g(sp.slack, 6),
            format_g(sp.overlap, 6)
        ));
    }
    let leaves: IndexMap<i64, Rect> =
        root.leaves().into_iter().map(|lf| (lf.rotation.expect("leaf"), lf.core)).collect();
    let mut stats = CutStats::default();
    let mut pieces: IndexMap<i64, Vec<(Vec<Point>, bool)>> = leaves.keys().map(|&r| (r, Vec::new())).collect();
    for (st, &th) in final_strokes.iter().zip(flags) {
        for (rot, piece) in cut_stroke(st, &root, sp.overlap, Some(&mut stats)) {
            pieces.get_mut(&rot).expect("leaf rotation").push((piece, th));
        }
    }
    c.cut_stats = Some(stats);
    let use_marks = sp.marks && leaves.len() > 1;
    c.marks = if use_marks {
        control_marks(&root, &geo, &rects, sp.mark_size, sp.mark_count as usize, 1.0)
    } else {
        Vec::new()
    };
    if use_marks && c.marks.is_empty() {
        c.warnings.push(
            "Контрольные крестики не поместились: у швов нет места, которое достают оба прохода вдали от линий".into(),
        );
    }
    let run_order: Vec<i64> = if ds.a3.enabled {
        A3_RUNS.iter().copied().filter(|r| pieces.contains_key(r)).collect()
    } else {
        let mut v: Vec<i64> = pieces.keys().copied().collect();
        v.sort();
        v
    };
    let mut idx = 0;
    for rot in run_order {
        let mk: Vec<Vec<Point>> = c
            .marks
            .iter()
            .filter(|m| m.passes.0 == rot || m.passes.1 == rot)
            .flat_map(|m| mark_strokes(m, sp.mark_size))
            .collect();
        if pieces[&rot].is_empty() && mk.is_empty() {
            continue;
        }
        let mut strokes: Vec<Vec<Point>> = Vec::new();
        let mut th: Vec<bool> = Vec::new();
        for thick in [true, false] {
            let group: Vec<Vec<Point>> =
                pieces[&rot].iter().filter(|(_, t)| *t == thick).map(|(p, _)| p.clone()).collect();
            for pts in join_paths(&group, ds.paths.join_tol) {
                strokes.push(pts);
                th.push(thick);
            }
        }
        let ordered = order_paths(&strokes, ds.paths.long_path, c.start_point(rot));
        idx += 1;
        let (dx, dy) = sp.offsets.get(&rot.to_string()).copied().unwrap_or((0.0, 0.0));
        let n_mk = mk.len();
        let mut part_strokes = mk;
        let mut part_thick = vec![false; n_mk];
        for (i, q) in ordered {
            part_thick.push(th[i]);
            part_strokes.push(q);
        }
        c.parts.push(PassPart {
            index: idx,
            rotation: rot,
            region: leaves[&rot],
            strokes: part_strokes,
            thick: part_thick,
            marks: n_mk,
            dx,
            dy,
            errors: Vec::new(),
        });
    }
    let mut leaf_rots: Vec<i64> = leaves.keys().copied().collect();
    leaf_rots.sort();
    let present: Vec<i64> = c.parts.iter().map(|p| p.rotation).collect();
    let empty: Vec<i64> = leaf_rots.into_iter().filter(|r| !present.contains(r)).collect();
    if !empty.is_empty() {
        c.warnings.push(format!(
            "Областей по раскладке: {}, но в {} из них нет линий (поворот {}) — файлов будет {}",
            leaves.len(),
            empty.len(),
            degrees(&empty),
            c.parts.len()
        ));
    }
    c.split = Some(root);
    c.drawing_passes = Some(c.parts.iter().map(|p| p.rotation).collect());
    let mut strokes = Vec::new();
    let mut thick = Vec::new();
    let mut part_of = Vec::new();
    for p in &c.parts {
        strokes.extend(p.strokes.iter().cloned());
        thick.extend(p.thick.iter().copied());
        part_of.extend(std::iter::repeat_n(p.index, p.strokes.len()));
    }
    c.strokes.extend(strokes);
    c.thick.extend(thick);
    c.stroke_part.extend(part_of);
}

fn outside_all(strokes: &[Vec<Point>], rects: &[Rect]) -> Vec<Vec<Point>> {
    let mut out = Vec::new();
    for st in strokes {
        let mut pieces = vec![st.clone()];
        for &r in rects {
            pieces = pieces.iter().flat_map(|p| outside_parts(p, r, 1e-9)).collect();
            if pieces.is_empty() {
                break;
            }
        }
        out.extend(pieces);
    }
    out
}

fn unreachable_message(
    c: &mut DrawingComposition,
    s: &Settings,
    bbox: Rect,
    pad: f64,
    big_w: f64,
    big_h: f64,
    lay: &SheetLayout,
) -> String {
    let ds = &s.drawing;
    let tail = if c.allowed.is_empty() {
        format!("; допустимых поворотов нет: {}", c.rotation_notes.join("; "))
    } else {
        format!("; допустимые повороты: {}", degrees(&c.allowed))
    };
    let mut msg = format!("Часть чертежа не достаётся ни в одном допустимом проходе (красным{tail}). ");
    let windows: Vec<Rect> = c.allowed.iter().map(|r| c.pass_rects[r]).collect();
    if !lay.frame.is_empty() && !c.allowed.is_empty() {
        let fl: Vec<Vec<Point>> = lay.frame.iter().map(|f| f.points.clone()).collect();
        if !covered_mask(&segments(&fl), &windows).into_iter().all(|v| v) {
            msg +=
                "Рамка ГОСТ не покрывается проходами: масштаб чертежа тут не поможет, выключи рамку или смени лист. ";
        }
    }
    if !c.allowed.is_empty() && ds.placement.scale_mode != crate::settings::ScaleMode::FitPasses {
        let area = c.placement.as_ref().and_then(|p| p.area);
        let areas: Vec<Rect> = lay.areas.iter().map(|&a| shrink(a, pad)).collect();
        let (s_fit, fit_area) = best_fit(bbox, &areas);
        let zero = anchor_of(ds) == AnchorKind::Zero && c.pass_rects.contains_key(&0);
        let rects: Vec<Rect> = if zero { vec![c.pass_rects[&0]] } else { windows.clone() };
        let sc = fit_to_passes(
            c.imp.as_ref().expect("import is set"),
            bbox,
            fit_area.or(area).expect("an area"),
            s_fit,
            &rects,
            pad,
            ds.placement.dx,
            ds.placement.dy,
            anchor_of(ds),
            ds.split.areas,
        );
        if sc > 0.0 {
            c.passes_percent = Some(sc * 100.0);
            msg += &format!(
                "Уменьши масштаб до {}% — кнопка «Подобрать масштаб под проходы». ",
                format_g((sc * 1000.0).floor() / 10.0, 6)
            );
        }
    }
    for (is_x, name) in [(true, "+X"), (false, "+Y")] {
        let allowed_now = if is_x { s.printer.table.overhang_x } else { s.printer.table.overhang_y };
        if !allowed_now {
            let mut p2 = s.printer.clone();
            if is_x {
                p2.table.overhang_x = true;
            } else {
                p2.table.overhang_y = true;
            }
            let (allowed2, rects2, _) = rotation_rects(big_w, big_h, &p2);
            let rs: Vec<Rect> = allowed2.iter().map(|r| rects2[r]).collect();
            if allowed2 != c.allowed && covered_mask(&segments(&c.strokes), &rs).into_iter().all(|v| v) {
                msg += &format!("Или включи «лист может выступать в сторону {name}». ");
            }
        }
    }
    msg.trim().to_string()
}

static SAFE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"[<>:"/\\|?*\x00-\x1f\s]+"#).expect("valid regex"));

pub fn file_stem(c: &DrawingComposition) -> String {
    let ds = &c.settings.drawing;
    let stem = if ds.file == BUILTIN_TEST {
        "test".to_string()
    } else {
        let name = c.imp.as_ref().map(|i| i.name.clone()).unwrap_or_default();
        let st = Path::new(&name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let safe = SAFE.replace_all(&st, "_").into_owned();
        if safe.is_empty() { "drawing".into() } else { safe }
    };
    let lay = c.layout();
    let fmt_name = if ds.sheet.format == SheetFormat::Custom {
        format!("{}x{}", format_g(lay.width, 6), format_g(lay.height, 6))
    } else {
        enum_name(&ds.sheet.format)
    };
    let o = if lay.orientation.as_str() == "portrait" { "P" } else { "L" };
    let scale = c.placement.as_ref().map_or(0.0, |p| p.scale);
    format!("drawing_{stem}_{fmt_name}{o}_{:.0}pct", scale * 100.0)
}

pub fn part_filename(c: &DrawingComposition, part: &PassPart, test: bool) -> String {
    format!("{}_pass{}_rot{}{}.gcode", file_stem(c), part.index, part.rotation, if test { "_test" } else { "" })
}

pub fn gcode_filename(c: &DrawingComposition) -> String {
    match c.parts.first() {
        Some(p) => part_filename(c, p, false),
        None => file_stem(c) + ".gcode",
    }
}

fn seams_text(c: &DrawingComposition) -> Vec<String> {
    match &c.split {
        None => Vec::new(),
        Some(root) => {
            root.seams().iter().map(|n| format!("{} = {:.2}", if n.axis == 0 { "x" } else { "y" }, n.s)).collect()
        }
    }
}

pub fn part_header(c: &DrawingComposition, part: &PassPart, test: bool) -> (Vec<String>, Vec<String>) {
    let ds = &c.settings.drawing;
    let pl = c.placement.as_ref().expect("placement is set");
    let lay = c.layout();
    let imp = c.imp.as_ref().expect("import is set");
    let (w, fr, sp) = (&ds.weights, &ds.frame, &ds.split);
    let r = part.rotation;
    let n = c.parts.len();
    let source = if ds.file == BUILTIN_TEST { ds.file.clone() } else { imp.name.clone() };
    let page = if imp.kind == "pdf" { format!(", page {}", imp.page) } else { String::new() };
    let pass_line = if c.marked() {
        format!("PASS {}/{n}: sheet on the printer marks, rotated {r} deg, pencil on the zero line", part.index)
    } else {
        format!(
            "PASS {}/{n}: rotate sheet {r} deg CCW, corner {} ({} in layout) at the stops = zero",
            part.index,
            part.corner(),
            corner_en(part.corner())
        )
    };
    let mut header = vec![
        format!("DRAWING: {source} ({}{page})", imp.kind),
        format!("{}{pass_line}", if test { "TEST FILE for " } else { "" }),
        format!(
            "run order: {}",
            c.parts
                .iter()
                .map(|p| format!("{}) {}", p.index, part_filename(c, p, test)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        format!("zero correction dx {} dy {} mm (pass coordinates)", fmt(part.dx), fmt(part.dy)),
        format!(
            "scale {} ({:.2}%), mode {}",
            scale_label(pl.scale),
            pl.scale * 100.0,
            enum_name(&ds.placement.scale_mode)
        ),
    ];
    if n > 1 {
        header.push(format!(
            "seams (layout mm): {}; overlap {} mm along lines",
            seams_text(c).join("; "),
            fmt(sp.overlap)
        ));
    }
    if part.marks > 0 {
        header.push(format!("control crosses: {}, size {} mm, drawn first", part.marks / 2, fmt(sp.mark_size)));
    }
    let (wp, hp) = pass_dims(r, lay.width, lay.height);
    let tb = make_table(&c.settings.printer, lay.width, lay.height);
    let frame = if fr.enabled {
        format!(
            "frame GOST 2.104 form 1: L{} R{} T{} B{}{}",
            fmt(fr.left),
            fmt(fr.right),
            fmt(fr.top),
            fmt(fr.bottom),
            if fr.title_block {
                format!(", title block {}x{}", fmt(fr.tb_width), fmt(fr.tb_height))
            } else {
                ", no title block".into()
            }
        )
    } else {
        "frame: off".into()
    };
    let weights = if w.enabled {
        format!("line weights: thick > {} mm drawn in {} passes, step {} mm", fmt(w.threshold), w.passes, fmt(w.step))
    } else {
        "line weights: off (single pass for all lines)".into()
    };
    let info = vec![
        format!(
            "sheet {} {} {}x{} mm (in this pass {}x{} along X x Y), drawing origin offset X{} Y{}",
            enum_name(&ds.sheet.format),
            lay.orientation.as_str(),
            fmt(lay.width),
            fmt(lay.height),
            fmt(wp),
            fmt(hp),
            fmt(pl.tx),
            fmt(pl.ty)
        ),
        format!(
            "table: sheet may overhang +X {} +Y {}; table edge X{} Y{}{}",
            tb.allow_x as i32,
            tb.allow_y as i32,
            fmt(tb.table_x),
            fmt(tb.table_y),
            if tb.table_given { "" } else { " (= reach window edge)" }
        ),
        frame,
        weights,
        format!(
            "curves {} mm, join {} mm, long paths first >= {} mm",
            fmt(ds.paths.curve_tol),
            fmt(ds.paths.join_tol),
            fmt(ds.paths.long_path)
        ),
    ];
    (header, info)
}

pub fn make_part_gcode(c: &DrawingComposition, part: &PassPart) -> Result<String, Vec<String>> {
    if !c.errors.is_empty() {
        return Err(c.errors.clone());
    }
    let (header, info) = part_header(c, part, false);
    Ok(generate_gcode(&c.part_strokes(part, None), &c.part_settings(part.rotation), &header, Some(&info)))
}

pub fn make_drawing_gcode(c: &DrawingComposition, index: usize) -> Result<String, Vec<String>> {
    if !c.errors.is_empty() {
        return Err(c.errors.clone());
    }
    if c.parts.is_empty() {
        return Err(vec!["Нет проходов".into()]);
    }
    make_part_gcode(c, &c.parts[index - 1])
}

fn rect_points(c: &DrawingComposition, q: Rect, r: i64) -> Vec<Point> {
    [(q.0, q.1), (q.2, q.1), (q.2, q.3), (q.0, q.3), (q.0, q.1)].iter().map(|&p| c.to_machine(p, r)).collect()
}

pub fn part_test_strokes(c: &DrawingComposition, part: &PassPart) -> Vec<Vec<Point>> {
    let lay = c.layout();
    let (w, h) = (lay.width, lay.height);
    let r = part.rotation;
    let mut out: Vec<Vec<Point>> = Vec::new();
    let mut fixed: Vec<Vec<Point>> = Vec::new();
    let reach = c.pass_rects.get(&r).copied();
    if let Some(rc) = reach {
        fixed.push(rect_points(c, rc, r));
    }
    if c.parts.len() > 1 {
        let reg = part.region;
        let cell = (numeric::max(reg.0, 0.0), numeric::max(reg.1, 0.0), numeric::min(reg.2, w), numeric::min(reg.3, h));
        let cell = match reach {
            Some(rc) => passes::intersect(cell, rc),
            None => Some(cell),
        };
        if let Some(cell) = cell {
            out.extend(dash_polyline(&rect_points(c, cell, r), &[4.0, 3.0], 0.0));
        }
    }
    let o = c.settings.printer.test_mark_offset;
    let head = 2.0;
    out.push(vec![(o, o), (o + 20.0, o)]);
    out.push(vec![(o + 20.0 - head, o + head * 0.6), (o + 20.0, o), (o + 20.0 - head, o - head * 0.6)]);
    out.push(vec![(o, o), (o, o + 10.0)]);
    out.push(vec![(o - head * 0.6, o + 10.0 - head), (o, o + 10.0), (o + head * 0.6, o + 10.0 - head)]);
    for i in 0..part.index {
        let x = o + 25.0 + 2.5 * i as f64;
        out.push(vec![(x, o - 1.5), (x, o + 1.5)]);
    }
    let marks: Vec<Vec<Point>> =
        part.strokes[..part.marks].iter().map(|st| st.iter().map(|&p| c.to_machine(p, r)).collect()).collect();
    let shifted = out
        .into_iter()
        .chain(marks)
        .map(|st| st.into_iter().map(|p| (p.0 + part.dx, p.1 + part.dy)).collect::<Vec<_>>());
    fixed.into_iter().chain(shifted).collect()
}

pub fn make_part_test_gcode(c: &DrawingComposition, part: &PassPart) -> Result<String, Vec<String>> {
    let strokes = part_test_strokes(c, part);
    let errors = check_bounds(&strokes, &c.part_settings(part.rotation), 5);
    if !errors.is_empty() {
        return Err(errors.into_iter().map(|m| format!("Тестовый файл прохода {}: {m}", part.index)).collect());
    }
    let (mut header, info) = part_header(c, part, true);
    header.insert(
        2,
        "TEST: available part of the sheet (solid), this pass cell (dashed), corner mark X long / Y short with pass \
         number ticks, control crosses"
            .into(),
    );
    Ok(generate_gcode(&strokes, &c.part_settings(part.rotation), &header, Some(&info)))
}

#[derive(Debug, Clone, PartialEq)]
pub struct GcodeFile {
    pub filename: String,
    pub gcode: String,
    pub pass_index: usize,
    pub rotation: i64,
    pub test: bool,
}

pub fn make_all_files(c: &DrawingComposition, tests: bool) -> Result<Vec<GcodeFile>, Vec<String>> {
    if !c.errors.is_empty() {
        return Err(c.errors.clone());
    }
    let mut out = Vec::new();
    for p in &c.parts {
        out.push(GcodeFile {
            filename: part_filename(c, p, false),
            gcode: make_part_gcode(c, p)?,
            pass_index: p.index,
            rotation: p.rotation,
            test: false,
        });
        if tests {
            out.push(GcodeFile {
                filename: part_filename(c, p, true),
                gcode: make_part_test_gcode(c, p)?,
                pass_index: p.index,
                rotation: p.rotation,
                test: true,
            });
        }
    }
    Ok(out)
}

fn travel_on_sheet(strokes: &[Vec<Point>], start: Point) -> Vec<(Point, Point)> {
    let mut moves = Vec::new();
    let mut cur = start;
    for st in strokes {
        if let Some(&first) = st.first() {
            if first != cur {
                moves.push((cur, first));
            }
            cur = st[st.len() - 1];
        }
    }
    moves
}

fn r2(p: Point) -> Value {
    json!([round_to(p.0, 3), round_to(p.1, 3)])
}

fn rr(q: &[f64]) -> Value {
    json!(q.iter().map(|&v| round_to(v, 2)).collect::<Vec<_>>())
}

fn rect_list(r: Rect) -> [f64; 4] {
    [r.0, r.1, r.2, r.3]
}

fn opt_rect(r: Option<Rect>) -> Value {
    r.map_or(Value::Null, |r| json!(rect_list(r)))
}

pub fn preview_payload(c: &DrawingComposition) -> Value {
    let s2 = c.sheet_settings.as_ref().unwrap_or(&c.settings);
    let mut out = serde_json::Map::new();
    out.insert("errors".into(), json!(c.errors));
    out.insert("warnings".into(), json!(c.warnings));
    out.insert("strokes".into(), json!([]));
    out.insert("travel".into(), json!([]));
    out.insert("unreachable".into(), json!([]));
    out.insert("texts".into(), json!([]));
    out.insert("frame".into(), json!([]));
    out.insert("stats".into(), compute_stats(&[], s2).as_json());
    out.insert("parts".into(), json!([]));
    out.insert("seams".into(), json!([]));
    out.insert("marks".into(), json!([]));
    out.insert("marked".into(), json!(c.marked()));
    if let Some(imp) = &c.imp {
        let bb = imp.bbox();
        let mut layers: Vec<(&String, &crate::drawing::model::LayerInfo)> = imp.layers.iter().collect();
        layers.sort_by(|a, b| a.0.cmp(b.0));
        out.insert(
            "import".into(),
            json!({
                "kind": imp.kind, "name": imp.name, "units": imp.units, "units_note": imp.units_note,
                "pages": imp.pages, "page": imp.page, "info": imp.info,
                "size": bb.map(|b| json!([round_to(b.2 - b.0, 2), round_to(b.3 - b.1, 2)])),
                "paths": imp.paths.len(),
                "layers": layers.iter().map(|(n, v)| json!({"name": n, "count": v.count, "width": v.width})).collect::<Vec<_>>(),
                "has_widths": imp.paths.iter().any(|p| p.width.is_some()),
            }),
        );
    }
    let Some(lay) = &c.layout else { return Value::Object(out) };
    let pl = c.placement.as_ref();
    let (w, h) = (lay.width, lay.height);
    out.insert("sheet".into(), json!({"width": w, "height": h, "orientation": lay.orientation.as_str()}));
    out.insert(
        "frame".into(),
        json!(
            lay.frame
                .iter()
                .map(|fl| json!({"p": fl.points.iter().map(|&p| r2(p)).collect::<Vec<_>>(), "t": fl.thick}))
                .collect::<Vec<_>>()
        ),
    );
    out.insert("title_block".into(), opt_rect(lay.title_block));
    out.insert("areas".into(), json!(lay.areas.iter().map(|&a| rect_list(a)).collect::<Vec<_>>()));
    out.insert(
        "reach".into(),
        json!({"rect": opt_rect(c.reach()), "measured": c.reach_measured, "margin": s2.printer.safety_margin}),
    );
    let sp = c.sheet_plan.as_ref();
    let show: Vec<i64> = match sp {
        Some(p) if p.ok() => p.rotations.clone().unwrap_or_default(),
        _ => c.allowed.clone(),
    };
    let t0 = make_table(&c.settings.printer, w, h);
    let mut windows = serde_json::Map::new();
    for r in [0, 90, 180, 270] {
        let t = make_table(&c.settings.printer, w, h);
        windows.insert(r.to_string(), json!({"raw": rr(&rect_list(t.raw)), "safe": rr(&rect_list(t.safe))}));
    }
    let show_rects: Vec<Rect> = show.iter().map(|r| c.pass_rects[r]).collect();
    let rotation = c.rotation();
    out.insert(
        "passes".into(),
        json!({
            "allowed": c.allowed, "notes": c.rotation_notes,
            "rects": c.pass_rects.iter().map(|(r, q)| (r.to_string(), rr(&rect_list(*q)))).collect::<serde_json::Map<_, _>>(),
            "corners": {"A": [0, 0], "B": [w, 0], "C": [w, h], "D": [0, h]},
            "corner_of": {"0": "A", "90": "D", "180": "C", "270": "B"},
            "map": show,
            "map_uncovered": uncovered((0.0, 0.0, w, h), &show_rects).iter().map(|&q| rr(&rect_list(q))).collect::<Vec<_>>(),
            "field": rr(&rect_list(field_rect(lay, &c.settings))),
            "sheet_plan": {
                "rotations": sp.and_then(|p| p.rotations.clone()),
                "describe": sp.filter(|p| p.ok()).map_or(String::new(), |p| p.describe(None)),
                "message": sp.map_or(String::new(), |p| p.message.clone()),
            },
            "drawing": c.drawing_passes, "rotation": rotation,
            "rotation_text": rotation.map_or(String::new(), |r| format!("{r}°, в упоре угол {} ({})", corner(r), corner_name(corner(r)))),
            "table": {"allow_x": t0.allow_x, "allow_y": t0.allow_y, "table_x": t0.table_x, "table_y": t0.table_y,
                      "given": t0.table_given, "raw": rr(&rect_list(t0.raw)), "safe": rr(&rect_list(t0.safe))},
            "windows": windows,
        }),
    );
    if let Some(pl) = pl.filter(|p| p.errors.is_empty()) {
        let mut scale = json!({
            "value": pl.scale, "label": scale_label(pl.scale), "percent": round_to(pl.scale * 100.0, 2),
            "area": opt_rect(pl.area),
            "reach_percent": c.reach_percent.filter(|&v| v != 0.0).map(|v| round_to(v, 2)),
            "passes_percent": c.passes_percent.filter(|&v| v != 0.0).map(|v| round_to(v, 2)),
        });
        if let Some(bb) = c.imp.as_ref().and_then(|i| i.bbox()) {
            scale["paper_size"] = json!([round_to((bb.2 - bb.0) * pl.scale, 1), round_to((bb.3 - bb.1) * pl.scale, 1)]);
        }
        out.insert("scale".into(), scale);
    }
    let mut travel: Vec<Value> = Vec::new();
    let mut parts_json = Vec::new();
    let (mut draw, mut trav, mut strokes_n, mut lifts, mut time_s) = (0.0, 0.0, 0usize, 0usize, 0i64);
    for p in &c.parts {
        let st = compute_stats(&c.part_strokes(p, None), &c.part_settings(p.rotation));
        let sj = st.as_json();
        draw += sj["draw_mm"].as_f64().expect("number");
        trav += sj["travel_mm"].as_f64().expect("number");
        strokes_n += st.strokes;
        lifts += st.lifts;
        time_s += sj["time_s"].as_i64().expect("integer");
        let corner_pt = c.start_point(p.rotation);
        travel.extend(travel_on_sheet(&p.strokes, corner_pt).into_iter().map(|(a, b)| json!([r2(a), r2(b), p.index])));
        let (pw, ph) = pass_dims(p.rotation, w, h);
        parts_json.push(json!({
            "index": p.index, "rotation": p.rotation, "corner": p.corner(), "corner_name": corner_name(p.corner()),
            "region": rr(&rect_list(p.region)), "stats": sj, "marks": p.marks / 2, "dx": p.dx, "dy": p.dy,
            "filename": part_filename(c, p, false), "test_filename": part_filename(c, p, true),
            "errors": p.errors, "pass_size": [pw, ph],
            "m": if c.marked() { json!(c.affine(p.rotation).iter().map(|&v| round_to(v, 6)).collect::<Vec<_>>()) } else { Value::Null },
        }));
    }
    out.insert("parts".into(), json!(parts_json));
    if c.parts.is_empty() {
        out.insert("stats".into(), compute_stats(&c.strokes, s2).as_json());
        travel = travel_on_sheet(&c.strokes, (0.0, 0.0)).into_iter().map(|(a, b)| json!([r2(a), r2(b), 0])).collect();
    } else {
        out.insert(
            "stats".into(),
            json!({"draw_mm": round_to(draw, 1), "travel_mm": round_to(trav, 1), "strokes": strokes_n, "lifts": lifts,
                   "time_s": time_s}),
        );
    }
    out.insert(
        "strokes".into(),
        json!(
            c.strokes
                .iter()
                .zip(&c.thick)
                .zip(&c.stroke_part)
                .map(|((st, t), k)| json!({
            "p": st.iter().map(|&p| r2(p)).collect::<Vec<_>>(), "t": t, "k": k}))
                .collect::<Vec<_>>()
        ),
    );
    out.insert("travel".into(), json!(travel));
    out.insert(
        "seams".into(),
        json!(c.split.as_ref().map_or(Vec::new(), |root| {
            root.seams()
                .iter()
                .map(|n| {
                    let span = [rect_list(n.core)[1 - n.axis], rect_list(n.core)[3 - n.axis]];
                    json!({"axis": n.axis, "s": round_to(n.s, 3), "span": rr(&span)})
                })
                .collect()
        })),
    );
    out.insert(
        "marks".into(),
        json!(
            c.marks
                .iter()
                .map(|m| json!({"x": round_to(m.x, 3), "y": round_to(m.y, 3), "passes": [m.passes.0, m.passes.1]}))
                .collect::<Vec<_>>()
        ),
    );
    out.insert("overlap".into(), json!(c.settings.drawing.split.overlap));
    out.insert(
        "unreachable".into(),
        json!(c.unreachable.iter().map(|st| st.iter().map(|&p| r2(p)).collect::<Vec<_>>()).collect::<Vec<_>>()),
    );
    out.insert(
        "texts".into(),
        json!(
            c.texts
                .iter()
                .map(|t| json!({"x": round_to(t.x, 2), "y": round_to(t.y, 2), "text": t.text, "kind": t.kind}))
                .collect::<Vec<_>>()
        ),
    );
    out.insert("flip".into(), json!([s2.printer.flip_x, s2.printer.flip_y]));
    let mo = to_machine((0.0, 0.0), s2);
    out.insert("machine_origin".into(), json!([mo.0, mo.1]));
    Value::Object(out)
}
