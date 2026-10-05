use crate::geometry::Point;
use crate::numeric::{self, format_g};
use crate::settings::{Anchor, DrawingSettings, Orientation, ScaleMode, SheetFormat};

pub type Rect = (f64, f64, f64, f64);

pub const TITLE_BLOCK_W: f64 = 185.0;
pub const TITLE_BLOCK_H: f64 = 55.0;
pub const TITLE_BLOCK_LINES: &[(f64, f64, f64, f64, bool)] = &[
    (0.0, 0.0, 0.0, 55.0, true),
    (0.0, 55.0, 185.0, 55.0, true),
    (7.0, 30.0, 7.0, 55.0, true),
    (17.0, 0.0, 17.0, 55.0, true),
    (40.0, 0.0, 40.0, 55.0, true),
    (55.0, 0.0, 55.0, 55.0, true),
    (65.0, 0.0, 65.0, 55.0, true),
    (0.0, 5.0, 65.0, 5.0, false),
    (0.0, 10.0, 65.0, 10.0, false),
    (0.0, 15.0, 65.0, 15.0, false),
    (0.0, 20.0, 65.0, 20.0, false),
    (0.0, 25.0, 65.0, 25.0, false),
    (0.0, 30.0, 65.0, 30.0, true),
    (0.0, 35.0, 65.0, 35.0, true),
    (0.0, 40.0, 65.0, 40.0, false),
    (0.0, 45.0, 65.0, 45.0, false),
    (0.0, 50.0, 65.0, 50.0, false),
    (65.0, 40.0, 185.0, 40.0, true),
    (65.0, 15.0, 185.0, 15.0, true),
    (135.0, 0.0, 135.0, 40.0, true),
    (135.0, 35.0, 185.0, 35.0, true),
    (135.0, 20.0, 185.0, 20.0, true),
    (150.0, 20.0, 150.0, 40.0, true),
    (167.0, 20.0, 167.0, 40.0, true),
    (140.0, 20.0, 140.0, 35.0, false),
    (145.0, 20.0, 145.0, 35.0, false),
    (155.0, 15.0, 155.0, 20.0, true),
];

#[derive(Debug, Clone, PartialEq)]
pub struct FrameLine {
    pub points: Vec<Point>,
    pub thick: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetOrientation {
    Portrait,
    Landscape,
}

impl SheetOrientation {
    pub fn as_str(self) -> &'static str {
        match self {
            SheetOrientation::Portrait => "portrait",
            SheetOrientation::Landscape => "landscape",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SheetLayout {
    pub width: f64,
    pub height: f64,
    pub orientation: SheetOrientation,
    pub frame: Vec<FrameLine>,
    pub inner: Option<Rect>,
    pub title_block: Option<Rect>,
    pub areas: Vec<Rect>,
}

pub fn sheet_size(ds: &DrawingSettings, bbox: Option<Rect>) -> (f64, f64, SheetOrientation) {
    use SheetOrientation::{Landscape, Portrait};
    let sh = &ds.sheet;
    let (w, h) = match sh.format {
        SheetFormat::A4 => (210.0, 297.0),
        SheetFormat::A3 => (297.0, 420.0),
        SheetFormat::Custom => (sh.width, sh.height),
    };
    let (short, long) = (numeric::min(w, h), numeric::max(w, h));
    let o = match sh.orientation {
        Orientation::Portrait => Portrait,
        Orientation::Landscape => Landscape,
        Orientation::Auto if sh.format == SheetFormat::Custom => {
            return (sh.width, sh.height, if sh.width <= sh.height { Portrait } else { Landscape });
        }
        Orientation::Auto => {
            if bbox.is_some_and(|b| (b.2 - b.0) > (b.3 - b.1)) {
                Landscape
            } else {
                Portrait
            }
        }
    };
    if o == Portrait { (short, long, o) } else { (long, short, o) }
}

pub fn fixed_mode(ds: &DrawingSettings) -> bool {
    ds.marked.enabled || ds.a3.enabled
}

pub fn sheet_layout(ds: &DrawingSettings, bbox: Option<Rect>) -> SheetLayout {
    let (w, h, o) = sheet_size(ds, bbox);
    let mut lay = SheetLayout {
        width: w,
        height: h,
        orientation: o,
        frame: Vec::new(),
        inner: None,
        title_block: None,
        areas: Vec::new(),
    };
    let f = &ds.frame;
    if !f.enabled {
        let m = ds.placement.margin;
        lay.areas =
            vec![if fixed_mode(ds) { (f.left, f.bottom, w - f.right, h - f.top) } else { (m, m, w - m, h - m) }];
        return lay;
    }
    let (x0, y0, x1, y1) = (f.left, f.bottom, w - f.right, h - f.top);
    lay.inner = Some((x0, y0, x1, y1));
    lay.frame.push(FrameLine { points: vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)], thick: true });
    if f.title_block {
        let (tw, th) = (numeric::min(f.tb_width, x1 - x0), numeric::min(f.tb_height, y1 - y0));
        let (tx0, ty0) = (x1 - tw, y0);
        lay.title_block = Some((tx0, ty0, x1, ty0 + th));
        let (kx, ky) = (tw / TITLE_BLOCK_W, th / TITLE_BLOCK_H);
        for &(a, b, c, d, thick) in TITLE_BLOCK_LINES {
            lay.frame
                .push(FrameLine { points: vec![(tx0 + a * kx, ty0 + b * ky), (tx0 + c * kx, ty0 + d * ky)], thick });
        }
        lay.areas = vec![(x0, ty0 + th, x1, y1), (x0, y0, tx0, y1)];
    } else {
        lay.areas = vec![(x0, y0, x1, y1)];
    }
    lay
}

fn fit_scale(bbox: Rect, area: Rect) -> f64 {
    let (bw, bh) = (bbox.2 - bbox.0, bbox.3 - bbox.1);
    let (aw, ah) = (area.2 - area.0, area.3 - area.1);
    if aw <= 0.0 || ah <= 0.0 {
        return 0.0;
    }
    let sx = if bw > 1e-9 { aw / bw } else { f64::INFINITY };
    let sy = if bh > 1e-9 { ah / bh } else { f64::INFINITY };
    let s = numeric::min(sx, sy);
    if s == f64::INFINITY { 0.0 } else { s }
}

pub fn shrink(a: Rect, d: f64) -> Rect {
    (a.0 + d, a.1 + d, a.2 - d, a.3 - d)
}

pub fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let r = (numeric::max(a.0, b.0), numeric::max(a.1, b.1), numeric::min(a.2, b.2), numeric::min(a.3, b.3));
    (r.2 > r.0 && r.3 > r.1).then_some(r)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub scale: f64,
    pub tx: f64,
    pub ty: f64,
    pub area: Option<Rect>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Placement {
    fn failed(msg: &str) -> Self {
        Self { scale: 0.0, tx: 0.0, ty: 0.0, area: None, errors: vec![msg.into()], warnings: Vec::new() }
    }

    pub fn apply(&self, p: Point) -> Point {
        (p.0 * self.scale + self.tx, p.1 * self.scale + self.ty)
    }
}

pub fn best_fit(bbox: Rect, areas: &[Rect]) -> (f64, Option<Rect>) {
    let (mut best, mut best_area) = (0.0, None);
    for &a in areas {
        let s = fit_scale(bbox, a);
        if s > best + 1e-12 {
            best = s;
            best_area = Some(a);
        }
    }
    (best, best_area)
}

pub fn reach_areas(lay: &SheetLayout, reach: &[Rect], pad: f64) -> Vec<Rect> {
    reach
        .iter()
        .flat_map(|&w| lay.areas.iter().filter_map(move |&a| intersect(shrink(a, pad), shrink(w, pad))))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorKind {
    Center,
    Zero,
    Left,
}

pub fn anchor_of(ds: &DrawingSettings) -> AnchorKind {
    if fixed_mode(ds) && !ds.frame.enabled {
        return AnchorKind::Left;
    }
    match ds.placement.anchor {
        Anchor::Center => AnchorKind::Center,
        Anchor::Zero => AnchorKind::Zero,
    }
}

pub fn place(ds: &DrawingSettings, bbox: Rect, lay: &SheetLayout, reach: &[Rect], pad: f64) -> Placement {
    let pl = &ds.placement;
    let mode = pl.scale_mode;
    let mut areas: Vec<Rect> = lay.areas.iter().map(|&a| shrink(a, pad)).collect();
    if mode == ScaleMode::FitReach {
        areas = reach_areas(lay, reach, pad);
        if areas.is_empty() {
            return Placement::failed(
                "Окно достижимости не пересекается с полем листа ни в одном допустимом повороте: вписать некуда \
                 (проверь окно, стол и лист)",
            );
        }
    }
    let (s_fit, area) = best_fit(bbox, &areas);
    let Some(area) = area else {
        return Placement::failed("Чертёж пустой или поле листа слишком маленькое");
    };
    let mut warnings = Vec::new();
    let s = match mode {
        ScaleMode::Fit | ScaleMode::FitReach | ScaleMode::FitPasses => s_fit,
        ScaleMode::OneToOne => 1.0,
        ScaleMode::Percent => pl.percent / 100.0,
    };
    if matches!(mode, ScaleMode::OneToOne | ScaleMode::Percent) {
        let (bw, bh) = ((bbox.2 - bbox.0) * s, (bbox.3 - bbox.1) * s);
        let (aw, ah) = (area.2 - area.0, area.3 - area.1);
        if bw > aw + 1e-6 || bh > ah + 1e-6 {
            let what =
                if mode == ScaleMode::OneToOne { "1:1".to_string() } else { format!("{}%", format_g(pl.percent, 6)) };
            warnings.push(format!(
                "При масштабе {what} чертёж {bw:.1}×{bh:.1} мм больше поля листа {aw:.1}×{ah:.1} мм; «вписать» дало бы {:.1}%",
                s_fit * 100.0
            ));
        }
    }
    let anchor = anchor_of(ds);
    if anchor == AnchorKind::Zero {
        return Placement {
            scale: s,
            tx: -bbox.0 * s + pad + pl.dx,
            ty: -bbox.1 * s + pad + pl.dy,
            area: Some(area),
            errors: Vec::new(),
            warnings,
        };
    }
    let mut cx = (area.0 + area.2) / 2.0 - (bbox.0 + bbox.2) / 2.0 * s;
    if anchor == AnchorKind::Left {
        cx = area.0 - bbox.0 * s;
    }
    let cy = (area.1 + area.3) / 2.0 - (bbox.1 + bbox.3) / 2.0 * s;
    Placement { scale: s, tx: cx + pl.dx, ty: cy + pl.dy, area: Some(area), errors: Vec::new(), warnings }
}

fn label_num(v: f64) -> String {
    if (v - v.round_ties_even()).abs() < 0.005 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

pub fn scale_label(s: f64) -> String {
    if s <= 0.0 {
        return "—".into();
    }
    if (s - 1.0).abs() < 1e-9 {
        return "1:1".into();
    }
    if s < 1.0 { format!("1:{}", label_num(1.0 / s)) } else { format!("{}:1", label_num(s)) }
}
