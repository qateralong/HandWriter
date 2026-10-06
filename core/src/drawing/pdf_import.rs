use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

use indexmap::IndexMap;
use mupdf::pdf::{PdfDocument, PdfPage};
use mupdf::text_page::{TextBlockType, TextPageFlags};
use mupdf::{
    ColorParams, Colorspace, Context, Device, Font, Matrix, NativeDevice, Path, PathWalker, StrokeState, TextPage,
};
use mupdf_sys::{fz_context, fz_font_name, fz_new_stext_page, fz_rect, fz_stext_line, fz_stext_page};
use regex::Regex;

use crate::drawing::model::{DPath, ImportResult, LayerInfo, TextMark};
use crate::drawing::svg_import::add_filled;
use crate::geometry::Point;
use crate::numeric;
use crate::settings::DrawingImport;
use crate::svgparse::{FlattenPen, IDENTITY};
use crate::svgpath::Pen;

const PT: f64 = 25.4 / 72.0;

type StartFn<'a> = dyn Fn(&mut Option<FlattenPen>, &mut Vec<Vec<Point>>, (f64, f64)) + 'a;
const INF: f32 = 2147483520.0;

type P = (f32, f32);

#[derive(Debug, Clone, PartialEq)]
enum Item {
    Line(P, P),
    Curve(P, P, P, P),
    Re([f32; 4]),
    Qu([P; 4]),
}

#[derive(Debug, Clone)]
struct PathDict {
    items: Vec<Item>,
    typ: &'static str,
    close_path: Option<bool>,
    color: Option<Vec<f32>>,
    fill: Option<Vec<f32>>,
    width: Option<f32>,
    dashes: Option<String>,
    layer: String,
}

#[derive(Default)]
struct LineArt {
    out: Vec<PathDict>,
    layer: String,
}

struct Walker {
    ctm: Matrix,
    fill: bool,
    items: Vec<Item>,
    close_path: Option<bool>,
    last: P,
    first: P,
    have_move: bool,
    line_count: usize,
}

fn tp(x: f32, y: f32, m: &Matrix) -> P {
    (x * m.a + y * m.c + m.e, x * m.b + y * m.d + m.f)
}

impl Walker {
    fn check_quad(&mut self) {
        let n = self.items.len();
        let mut f = [0.0f32; 8];
        let mut lp = (0.0, 0.0);
        for i in 0..4 {
            let Item::Line(a, b) = self.items[n - 4 + i] else { return };
            f[i * 2] = a.0;
            f[i * 2 + 1] = a.1;
            lp = b;
        }
        if lp.0 != f[0] || lp.1 != f[1] {
            return;
        }
        self.line_count = 0;
        let q = [(f[0], f[1]), (f[6], f[7]), (f[2], f[3]), (f[4], f[5])];
        self.items.truncate(n - 4);
        self.items.push(Item::Qu(q));
    }

    fn check_rect(&mut self) -> bool {
        self.line_count = 0;
        let n = self.items.len();
        let (Item::Line(ll, lr), Item::Line(ur, ul)) = (&self.items[n - 3], &self.items[n - 1]) else { return false };
        let (ll, lr, ur, ul) = (*ll, *lr, *ur, *ul);
        if ll.1 != lr.1 || ll.0 != ul.0 || ur.1 != ul.1 || ur.0 != lr.0 {
            return false;
        }
        let r = if ul.1 < lr.1 { [ul.0, ul.1, lr.0, lr.1] } else { [ll.0, ll.1, ur.0, ur.1] };
        self.items.truncate(n - 3);
        self.items.push(Item::Re(r));
        true
    }
}

impl PathWalker for Walker {
    fn move_to(&mut self, x: f32, y: f32) {
        self.last = tp(x, y, &self.ctm);
        self.first = self.last;
        self.have_move = true;
        self.line_count = 0;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p1 = tp(x, y, &self.ctm);
        self.items.push(Item::Line(self.last, p1));
        self.last = p1;
        self.line_count += 1;
        if self.line_count == 4 && !self.fill {
            self.check_quad();
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x3: f32, y3: f32) {
        self.line_count = 0;
        let (p1, p2, p3) = (tp(x1, y1, &self.ctm), tp(x2, y2, &self.ctm), tp(x3, y3, &self.ctm));
        self.items.push(Item::Curve(self.last, p1, p2, p3));
        self.last = p3;
    }

    fn close(&mut self) {
        if self.line_count == 3 && self.check_rect() {
            return;
        }
        self.line_count = 0;
        if self.have_move {
            if self.first != self.last {
                self.items.push(Item::Line(self.last, self.first));
                self.last = self.first;
            }
            self.have_move = false;
            self.close_path = Some(false);
        } else {
            self.close_path = Some(true);
        }
    }
}

fn walk(path: &Path, ctm: Matrix, fill: bool) -> Option<(Vec<Item>, Option<bool>)> {
    let mut w = Walker {
        ctm,
        fill,
        items: Vec::new(),
        close_path: None,
        last: (0.0, 0.0),
        first: (0.0, 0.0),
        have_move: false,
        line_count: 0,
    };
    path.walk(&mut w).ok()?;
    (!w.items.is_empty()).then_some((w.items, w.close_path))
}

fn rgb(cs: &Colorspace, color: &[f32]) -> Option<Vec<f32>> {
    cs.convert_color(color, &Colorspace::device_rgb(), None, ColorParams::default()).ok().map(|c| c[..3].to_vec())
}

fn fmt_g(f: f32) -> String {
    if f.is_nan() || f == 0.0 {
        return if f.is_sign_negative() && !f.is_nan() { "-0".into() } else { "0".into() };
    }
    let f = if f.is_infinite() { f32::MAX.copysign(f) } else { f };
    let s = format!("{:e}", f.abs());
    let (mant, exp) = s.split_once('e').expect("exponent");
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let exp: i32 = exp.parse().expect("exponent");
    let nd = digits.len() as i32;
    let point = exp + 1;
    let mut out = String::new();
    if f < 0.0 {
        out.push('-');
    }
    if point <= 0 {
        out.push('.');
        for _ in 0..-point {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        for (i, c) in digits.chars().enumerate() {
            out.push(c);
            if i as i32 + 1 == point && (i as i32 + 1) < nd {
                out.push('.');
            }
        }
        for _ in nd..point {
            out.push('0');
        }
    }
    out
}

impl LineArt {
    fn append_merge(&mut self, d: PathDict) {
        if d.typ == "s"
            && let Some(prev) = self.out.last_mut()
            && prev.typ == "f"
            && prev.items == d.items
        {
            prev.typ = "fs";
            if prev.close_path.is_none() {
                prev.close_path = d.close_path;
            }
            prev.color = d.color;
            prev.width = d.width;
            prev.dashes = d.dashes;
            return;
        }
        self.out.push(d);
    }
}

impl NativeDevice for LineArt {
    fn fill_path(
        &mut self,
        path: &Path,
        _even_odd: bool,
        ctm: Matrix,
        cs: &Colorspace,
        color: &[f32],
        _alpha: f32,
        _cp: ColorParams,
    ) {
        let Some((items, close_path)) = walk(path, ctm, true) else { return };
        let d = PathDict {
            items,
            typ: "f",
            close_path,
            color: None,
            fill: rgb(cs, color),
            width: None,
            dashes: None,
            layer: self.layer.clone(),
        };
        self.append_merge(d);
    }

    fn stroke_path(
        &mut self,
        path: &Path,
        stroke: &StrokeState,
        ctm: Matrix,
        cs: &Colorspace,
        color: &[f32],
        _alpha: f32,
        _cp: ColorParams,
    ) {
        let factor = (ctm.a * ctm.d - ctm.b * ctm.c).abs().sqrt();
        let Some((items, close_path)) = walk(path, ctm, false) else { return };
        let dl = stroke.dashes();
        let dashes = if dl.is_empty() {
            "[] 0".to_string()
        } else {
            let mut s = "[ ".to_string();
            for v in dl {
                s.push_str(&fmt_g(factor * v));
                s.push(' ');
            }
            s.push_str(&format!("] {}", fmt_g(factor * stroke.dash_phase())));
            s
        };
        let d = PathDict {
            items,
            typ: "s",
            close_path: Some(close_path.unwrap_or(false)),
            color: rgb(cs, color),
            fill: None,
            width: Some(factor * stroke.line_width()),
            dashes: Some(dashes),
            layer: self.layer.clone(),
        };
        self.append_merge(d);
    }

    fn begin_layer(&mut self, name: &str) {
        self.layer = name.to_string();
    }

    fn end_layer(&mut self) {
        self.layer.clear();
    }
}

fn white(c: &Option<Vec<f32>>) -> bool {
    c.as_ref()
        .is_some_and(|c| c.len() >= 3 && c[..3].iter().map(|&v| v as f64).fold(f64::INFINITY, numeric::min) > 0.95)
}

static DASH_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\[([^\]]*)\]\s*([-+\d.eE]*)").unwrap());

fn dashes(s: Option<&str>, k: f64) -> (Option<Vec<f64>>, f64) {
    let Some(s) = s.filter(|s| !s.is_empty()) else { return (None, 0.0) };
    let Some(m) = DASH_RE.captures(s) else { return (None, 0.0) };
    let vals: Vec<f64> = m[1].split_whitespace().map(|v| v.parse::<f64>().unwrap_or(0.0).abs() * k).collect();
    let off =
        m.get(2).map(|g| g.as_str()).filter(|g| !g.is_empty()).map_or(0.0, |g| g.parse::<f64>().unwrap_or(0.0)) * k;
    let ok = !vals.is_empty() && numeric::sum(vals.iter().copied()) > 0.0;
    (ok.then_some(vals), off)
}

fn norm_rotation(mut a: i32) -> i32 {
    while a < 0 {
        a += 360;
    }
    while a >= 360 {
        a -= 360;
    }
    if a % 90 != 0 { 0 } else { a }
}

fn ctx_ptr() -> *mut fz_context {
    let ctx = Context::get();
    unsafe { std::mem::transmute_copy::<Context, *mut fz_context>(&ctx) }
}

fn new_text_page(rect: fz_rect) -> Option<TextPage> {
    let raw: *mut fz_stext_page = unsafe { fz_new_stext_page(ctx_ptr(), rect) };
    let nn = std::ptr::NonNull::new(raw)?;
    Some(unsafe { std::mem::transmute::<std::ptr::NonNull<fz_stext_page>, TextPage>(nn) })
}

#[derive(Clone, Copy)]
struct R4 {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl R4 {
    const EMPTY: R4 = R4 { x0: INF, y0: INF, x1: -INF, y1: -INF };

    fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    fn is_infinite(&self) -> bool {
        self.x0 == -INF && self.x1 == INF && self.y0 == -INF && self.y1 == INF
    }

    fn union(self, b: R4) -> R4 {
        if !(b.x0 <= b.x1 && b.y0 <= b.y1) {
            return self;
        }
        if !(self.x0 <= self.x1 && self.y0 <= self.y1) {
            return b;
        }
        R4 { x0: self.x0.min(b.x0), y0: self.y0.min(b.y0), x1: self.x1.max(b.x1), y1: self.y1.max(b.y1) }
    }

    fn overlaps(&self, b: &R4) -> bool {
        !(self.x0 >= b.x1 || self.y0 >= b.y1 || self.x1 <= b.x0 || self.y1 <= b.y0)
    }

    fn intersect_empty(&self, b: &R4) -> bool {
        let r = R4 { x0: self.x0.max(b.x0), y0: self.y0.max(b.y0), x1: self.x1.min(b.x1), y1: self.y1.min(b.y1) };
        r.is_empty()
    }

    fn contains(&self, b: &R4) -> bool {
        if self.is_empty() || b.is_empty() {
            return false;
        }
        b.x0 >= self.x0 && b.x1 <= self.x1 && b.y0 >= self.y0 && b.y1 <= self.y1
    }
}

fn mat_t(m: [f32; 6], p: P) -> P {
    (p.0 * m[0] + p.1 * m[2] + m[4], p.0 * m[1] + p.1 * m[3] + m[5])
}

fn rect_from_quad(q: [P; 4]) -> R4 {
    let xs = [q[0].0, q[1].0, q[2].0, q[3].0];
    let ys = [q[0].1, q[1].1, q[2].1, q[3].1];
    let mn = |v: [f32; 4]| v[0].min(v[1]).min(v[2]).min(v[3]);
    let mx = |v: [f32; 4]| v[0].max(v[1]).max(v[2]).max(v[3]);
    R4 { x0: mn(xs), y0: mn(ys), x1: mx(xs), y1: mx(ys) }
}

fn char_bbox(dir: P, wmode: bool, c: i32, origin: P, size: f32, quad: [P; 4], font: Option<&Font>) -> R4 {
    let corrected = (|| {
        if wmode {
            return None;
        }
        let font = font?;
        let mut asc = font.ascender();
        let mut dsc = font.descender();
        let mut asc_dsc = asc - dsc + f32::EPSILON;
        if asc_dsc >= 1.0 {
            return None;
        }
        if asc < 1e-3 {
            dsc = -0.1;
            asc = 0.9;
            asc_dsc = 1.0;
        }
        if asc_dsc < 1.0 {
            dsc /= asc_dsc;
            asc /= asc_dsc;
        }
        asc_dsc = asc - dsc;
        asc = asc * size / asc_dsc;
        dsc = dsc * size / asc_dsc;
        let (c0, s0) = dir;
        let mut trm1 = [c0, -s0, s0, c0, 0.0, 0.0];
        let mut trm2 = [c0, s0, -s0, c0, 0.0, 0.0];
        if c0 == -1.0 {
            trm1[3] = 1.0;
            trm2[3] = 1.0;
        }
        let x1 = [1.0, 0.0, 0.0, 1.0, -origin.0, -origin.1];
        let x2 = [1.0, 0.0, 0.0, 1.0, origin.0, origin.1];
        let mut q = quad.map(|p| mat_t(trm1, mat_t(x1, p)));
        if c0 == 1.0 && q[0].1 > 0.0 {
            q[0].1 = asc;
            q[1].1 = asc;
            q[2].1 = dsc;
            q[3].1 = dsc;
        } else {
            q[0].1 = -asc;
            q[1].1 = -asc;
            q[2].1 = -dsc;
            q[3].1 = -dsc;
        }
        if q[2].0 < 0.0 {
            q[2].0 = 0.0;
            q[0].0 = 0.0;
        }
        let cwidth = q[3].0 - q[2].0;
        if cwidth < f32::EPSILON
            && let Ok(glyph) = font.encode_character(c)
            && glyph != 0
            && let Ok(fw) = font.advance_glyph_with_wmode(glyph, wmode)
        {
            q[3].0 = q[2].0 + fw * size;
            q[1].0 = q[3].0;
        }
        Some(q.map(|p| mat_t(x2, mat_t(trm2, p))))
    })();
    let r = rect_from_quad(corrected.unwrap_or(quad));
    if !wmode {
        return r;
    }
    let mut r = r;
    if r.y1 < r.y0 + size {
        r.y0 = r.y1 - size;
    }
    r
}

fn text_marks(page: &PdfPage, tr: &dyn Fn(f64, f64) -> Point, res: &mut ImportResult) -> Result<(), mupdf::Error> {
    let b = page.bounds()?;
    let mediabox = fz_rect { x0: b.x0, y0: b.y0, x1: b.x1, y1: b.y1 };
    let Some(tpage) = new_text_page(mediabox) else { return Ok(()) };
    {
        let dev = Device::from_text_page(&tpage, TextPageFlags::from_bits_truncate(199))?;
        page.run(&dev, &Matrix::IDENTITY)?;
    }
    let tp_rect = R4 { x0: b.x0, y0: b.y0, x1: b.x1, y1: b.y1 };
    let infinite = tp_rect.is_infinite();
    for block in tpage.blocks() {
        let bb = block.bounds();
        let br = R4 { x0: bb.x0, y0: bb.y0, x1: bb.x1, y1: bb.y1 };
        match block.r#type() {
            TextBlockType::Image => {
                if tp_rect.contains(&br) || infinite {
                    let (x, y) = tr(bb.x0 as f64, bb.y1 as f64);
                    res.texts.push(TextMark {
                        x, y, text: "встроенная картинка".into(), kind: "image".into()
                    });
                }
            }
            TextBlockType::Text => {
                if !(tp_rect.overlaps(&br) || infinite) {
                    continue;
                }
                for line in block.lines() {
                    let lb = line.bounds();
                    let lr = R4 { x0: lb.x0, y0: lb.y0, x1: lb.x1, y1: lb.y1 };
                    if tp_rect.intersect_empty(&lr) && !infinite {
                        continue;
                    }
                    let raw: &fz_stext_line = unsafe { std::mem::transmute_copy(&line) };
                    let dir = (raw.dir.x, raw.dir.y);
                    let wmode = raw.wmode != 0;
                    let first_y = if raw.first_char.is_null() { 0.0 } else { unsafe { (*raw.first_char).origin.y } };
                    let mut text = String::new();
                    let mut span_text = String::new();
                    let mut line_rect = R4::EMPTY;
                    let mut span_rect = R4::EMPTY;
                    let mut old_style: Option<(f32, u32, u32, Vec<u8>, u32)> = None;
                    for ch in line.chars() {
                        let q = ch.quad();
                        let quad = [(q.ul.x, q.ul.y), (q.ur.x, q.ur.y), (q.ll.x, q.ll.y), (q.lr.x, q.lr.y)];
                        let o = ch.origin();
                        let raw_c: &mupdf_sys::fz_stext_char = unsafe { std::mem::transmute_copy(&ch) };
                        let font = ch.font();
                        let r = char_bbox(dir, wmode, raw_c.c, (o.x, o.y), ch.size(), quad, font.as_ref());
                        if !tp_rect.overlaps(&r) && !infinite {
                            continue;
                        }
                        let mut flags = u32::from(!wmode && dir == (1.0, 0.0) && o.y < first_y - ch.size() * 0.1);
                        let fname: Vec<u8> = if raw_c.font.is_null() {
                            Vec::new()
                        } else {
                            let n = unsafe { std::ffi::CStr::from_ptr(fz_font_name(ctx_ptr(), raw_c.font)) }.to_bytes();
                            match n.iter().position(|&b| b == b'+') {
                                Some(6) => n[7..].to_vec(),
                                _ => n.to_vec(),
                            }
                        };
                        if let Some(f) = font.as_ref() {
                            flags += u32::from(f.is_italic()) * 2
                                + u32::from(f.is_serif()) * 4
                                + u32::from(f.is_monospaced()) * 8
                                + u32::from(f.is_bold()) * 16;
                        }
                        let style = (ch.size(), flags, (raw_c.flags as u32) & !4, fname, raw_c.argb);
                        if old_style.as_ref() != Some(&style) {
                            if old_style.is_some() {
                                text.push_str(&span_text);
                                line_rect = line_rect.union(span_rect);
                            }
                            span_text.clear();
                            old_style = Some(style);
                            span_rect = r;
                        }
                        span_rect = span_rect.union(r);
                        span_text.push(ch.char().unwrap_or('\u{FFFD}'));
                    }
                    if old_style.is_some() && !span_rect.is_empty() {
                        text.push_str(&span_text);
                        line_rect = line_rect.union(span_rect);
                    }
                    let t = text
                        .split(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
                        .filter(|w| !w.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !t.is_empty() {
                        let (x, y) = tr(line_rect.x0 as f64, line_rect.y1 as f64);
                        res.texts.push(TextMark { x, y, text: t, kind: "text".into() });
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn import_pdf(data: &[u8], name: &str, opts: &DrawingImport, tol_mm: f64) -> ImportResult {
    let mut res = ImportResult::new("pdf", name);
    res.units = "pt".into();
    res.units_note = "PDF: пункты (1/72 дюйма), размер как на бумаге".into();
    if data.is_empty() {
        res.errors.push("PDF не читается: Cannot open empty stream.".into());
        return res;
    }
    let doc = match PdfDocument::from_bytes(data) {
        Ok(d) => d,
        Err(_) => {
            res.errors.push("PDF не читается: Failed to open stream".into());
            return res;
        }
    };
    let count = doc.page_count().unwrap_or(0);
    res.pages = count as i64;
    if count == 0 {
        res.errors.push("В PDF нет страниц".into());
        return res;
    }
    let page_no = opts.pdf_page.clamp(1, count as i64);
    if page_no != opts.pdf_page {
        res.warnings.push(format!("В PDF {count} стр., взята страница {page_no}"));
    }
    res.page = page_no;
    let Ok(page) = doc.load_page(page_no as i32 - 1).and_then(PdfPage::try_from) else {
        res.errors.push("PDF не читается: Failed to open stream".into());
        return res;
    };
    let mut page = page;
    let rotation = norm_rotation(page.rotation().unwrap_or(0));
    if rotation != 0 {
        let _ = page.set_rotation(0);
    }
    let unrot = page.bounds().ok();
    let recorder = Rc::new(RefCell::new(LineArt::default()));
    if let Ok(dev) = Device::from_native(recorder.clone()) {
        let _ = page.run(&dev, &Matrix::IDENTITY);
    }
    let rot: [f32; 6] = match (rotation, unrot) {
        (90 | 180 | 270, Some(b)) => {
            let (w, h) = ((b.x1 - b.x0).abs(), (b.y1 - b.y0).abs());
            match rotation {
                90 => [0.0, 1.0, -1.0, 0.0, h, 0.0],
                180 => [-1.0, 0.0, 0.0, -1.0, w, h],
                _ => [0.0, -1.0, 1.0, 0.0, 0.0, w],
            }
        }
        _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    };
    let tr = |x: f64, y: f64| -> Point {
        let (x, y) = (x as f32, y as f32);
        let qx = x * rot[0] + y * rot[2] + rot[4];
        let qy = x * rot[1] + y * rot[3] + rot[5];
        (qx as f64 * PT, -(qy as f64) * PT)
    };
    let drawings = std::mem::take(&mut recorder.borrow_mut().out);
    let mut whites = 0;
    let mut fills_small = 0;
    let pt = |p: P| (p.0 as f64, p.1 as f64);
    for d in drawings {
        let mut subpaths: Vec<Vec<Point>> = Vec::new();
        let mut pen: Option<FlattenPen> = None;
        let start = |pen: &mut Option<FlattenPen>, subpaths: &mut Vec<Vec<Point>>, p: (f64, f64)| {
            if let Some(old) = pen.take() {
                subpaths.extend(old.finish());
            }
            let mut np = FlattenPen::new(IDENTITY, tol_mm);
            np.move_to(tr(p.0, p.1));
            *pen = Some(np);
        };
        let mut last: Option<(f64, f64)> = None;
        let far = |a: (f64, f64), last: Option<(f64, f64)>| {
            last.is_none_or(|l| (a.0 - l.0).abs() > 1e-6 || (a.1 - l.1).abs() > 1e-6)
        };
        for it in &d.items {
            match it {
                Item::Line(a, b) => {
                    let (a, b) = (pt(*a), pt(*b));
                    if far(a, last) {
                        start(&mut pen, &mut subpaths, a);
                    }
                    pen.as_mut().expect("pen").line_to(tr(b.0, b.1));
                    last = Some(b);
                }
                Item::Curve(a, c1, c2, b) => {
                    let (a, c1, c2, b) = (pt(*a), pt(*c1), pt(*c2), pt(*b));
                    if far(a, last) {
                        start(&mut pen, &mut subpaths, a);
                    }
                    pen.as_mut().expect("pen").curve_to(tr(c1.0, c1.1), tr(c2.0, c2.1), tr(b.0, b.1));
                    last = Some(b);
                }
                Item::Re(r) => {
                    let (mut x0, mut y0, mut x1, mut y1) = (r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64);
                    if x1 < x0 {
                        std::mem::swap(&mut x0, &mut x1);
                    }
                    if y1 < y0 {
                        std::mem::swap(&mut y0, &mut y1);
                    }
                    let q = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
                    quad_path(&mut pen, &mut subpaths, &start, &tr, q);
                    last = None;
                }
                Item::Qu(q) => {
                    let q = q.map(pt);
                    quad_path(&mut pen, &mut subpaths, &start, &tr, q);
                    last = None;
                }
            }
        }
        if let Some(mut p) = pen.take() {
            if d.close_path == Some(true) {
                p.close_path();
            }
            subpaths.extend(p.finish());
        }
        subpaths.retain(|p| !p.is_empty());
        if subpaths.is_empty() {
            continue;
        }
        if d.typ.contains('s') {
            if white(&d.color) {
                whites += 1;
                continue;
            }
            let width = d.width.map_or(0.0, |w| w as f64) * PT;
            let (dash, off) = dashes(d.dashes.as_deref(), PT);
            for p in subpaths {
                let closed = p.len() > 2 && p[0] == p[p.len() - 1];
                res.paths.push(DPath {
                    points: p,
                    closed,
                    width: Some(width),
                    layer: d.layer.clone(),
                    dash: dash.clone(),
                    dash_offset: off,
                });
            }
        } else if d.typ.contains('f') {
            if white(&d.fill) {
                whites += 1;
                continue;
            }
            if add_filled(&subpaths, &mut res, opts, &d.layer, true) {
                fills_small += 1;
            }
        }
    }
    let _ = text_marks(&page, &tr, &mut res);
    if rotation != 0 {
        let _ = page.set_rotation(rotation);
    }
    if whites > 0 {
        res.warnings.push(format!("Белые заливки и обводки (фон) пропущены: {whites}"));
    }
    if fills_small > 0 {
        res.warnings.push(format!("Мелкие закрашенные фигуры превращены в центральные линии: {fills_small}"));
    }
    let mut layers: IndexMap<String, LayerInfo> = IndexMap::new();
    for p in &res.paths {
        if !p.layer.is_empty() {
            layers
                .entry(p.layer.clone())
                .or_insert(LayerInfo { count: 0, width: None, linetype: String::new() })
                .count += 1;
        }
    }
    res.layers = layers;
    res
}

fn quad_path(
    pen: &mut Option<FlattenPen>,
    subpaths: &mut Vec<Vec<Point>>,
    start: &StartFn,
    tr: &dyn Fn(f64, f64) -> Point,
    q: [(f64, f64); 4],
) {
    let [ul, ur, ll, lr] = q;
    start(pen, subpaths, ul);
    let p = pen.as_mut().expect("pen");
    for c in [ur, lr, ll, ul] {
        p.line_to(tr(c.0, c.1));
    }
    p.close_path();
}
