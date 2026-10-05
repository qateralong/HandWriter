use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use indexmap::IndexMap;
use rayon::prelude::*;
use regex::Regex;
use rustybuzz::ttf_parser::{self, GlyphId, PlatformId, Tag};
use unicode_general_category::{GeneralCategory, get_general_category};

use crate::geometry::Point;
use crate::glyphs::{FontError, FontInfo, FontMetrics, Glyph, GlyphProvider, ShapedGlyph};
use crate::pyset::{PySet, hash_int};
use crate::skeleton::{SkeletonParams, skeleton_strokes};
use crate::svgparse::{cubic_flat, mid};

const VARIANT_FEATURES_BASE: &[&str] =
    &["calt", "salt", "rand", "swsh", "cswh", "init", "medi", "fina", "isol", "hist"];
const LIGATURE_FEATURES: &[&str] = &["liga", "clig", "rlig", "dlig", "calt"];
const POSITIONAL: &[&str] = &["init", "medi", "fina", "isol", "calt", "swsh", "cswh", "hist"];
pub const SHAPING_FEATURES: &[&str] = &["calt", "liga", "clig", "rlig", "locl", "kern"];

static TECH_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:lf|tf|osf|onum|lnum|pnum|tnum|sups|subs|sinf|numr|dnom|sc|c2sc|smcp|case|cy|locl.*|ordn|frac|dflt|null|notdef|superior|inferior)$",
    )
    .expect("valid regex")
});
static NAME_SPLIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.+?)\.(.+)$").expect("valid regex"));

fn is_variant_feature(tag: &str) -> bool {
    VARIANT_FEATURES_BASE.contains(&tag) || is_numbered(tag, "ss", 1, 20) || is_numbered(tag, "cv", 1, 99)
}

fn is_numbered(tag: &str, prefix: &str, lo: u32, hi: u32) -> bool {
    tag.len() == 4
        && tag.starts_with(prefix)
        && tag[2..].bytes().all(|b| b.is_ascii_digit())
        && tag[2..].parse::<u32>().is_ok_and(|n| (lo..=hi).contains(&n))
}

fn is_free_feature(tag: &str) -> bool {
    tag == "salt" || tag == "rand" || is_numbered(tag, "ss", 1, 20) || is_numbered(tag, "cv", 1, 99)
}

pub fn is_free_variant(source: &str) -> bool {
    if is_free_feature(source) {
        return true;
    }
    if let Some(rest) = source.strip_prefix("суффикс .") {
        let suffix = rest.split('.').next().unwrap_or("").to_lowercase();
        return !POSITIONAL.contains(&suffix.as_str()) && !TECH_SUFFIX.is_match(&suffix);
    }
    false
}

struct Be<'a>(&'a [u8]);

impl Be<'_> {
    fn u16(&self, o: usize) -> Option<u16> {
        self.0.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    fn i16(&self, o: usize) -> Option<i16> {
        self.u16(o).map(|v| v as i16)
    }
    fn u32(&self, o: usize) -> Option<u32> {
        self.0.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

fn coverage(t: &Be, o: usize) -> Vec<u16> {
    let mut out = Vec::new();
    match t.u16(o) {
        Some(1) => {
            let n = t.u16(o + 2).unwrap_or(0) as usize;
            for i in 0..n {
                if let Some(g) = t.u16(o + 4 + 2 * i) {
                    out.push(g);
                }
            }
        }
        Some(2) => {
            let n = t.u16(o + 2).unwrap_or(0) as usize;
            for i in 0..n {
                let r = o + 4 + 6 * i;
                if let (Some(s), Some(e)) = (t.u16(r), t.u16(r + 2)) {
                    out.extend(s..=e);
                }
            }
        }
        _ => {}
    }
    out
}

fn records(t: &Be, o: usize, count: usize, out: &mut BTreeSet<u16>) {
    for i in 0..count {
        if let Some(li) = t.u16(o + 4 * i + 2) {
            out.insert(li);
        }
    }
}

fn nested_lookups(t: &Be, st: usize, ltype: u16) -> Vec<u16> {
    let mut found: Vec<u16> = Vec::new();
    let mut add = |set: BTreeSet<u16>| found.extend(set);
    let fmt = t.u16(st).unwrap_or(0);
    let mut s = BTreeSet::new();
    match (ltype, fmt) {
        (5, 1) | (5, 2) => {
            let base = if fmt == 1 { st + 4 } else { st + 6 };
            let n = t.u16(base).unwrap_or(0) as usize;
            for i in 0..n {
                let off = t.u16(base + 2 + 2 * i).unwrap_or(0) as usize;
                if off == 0 {
                    continue;
                }
                let set = st + off;
                let rn = t.u16(set).unwrap_or(0) as usize;
                for j in 0..rn {
                    let r = set + t.u16(set + 2 + 2 * j).unwrap_or(0) as usize;
                    let gc = t.u16(r).unwrap_or(0) as usize;
                    let sc = t.u16(r + 2).unwrap_or(0) as usize;
                    records(t, r + 4 + 2 * gc.saturating_sub(1), sc, &mut s);
                }
            }
        }
        (5, 3) => {
            let gc = t.u16(st + 2).unwrap_or(0) as usize;
            let sc = t.u16(st + 4).unwrap_or(0) as usize;
            records(t, st + 6 + 2 * gc, sc, &mut s);
        }
        (6, 1) | (6, 2) => {
            let base = if fmt == 1 { st + 4 } else { st + 10 };
            let n = t.u16(base).unwrap_or(0) as usize;
            for i in 0..n {
                let off = t.u16(base + 2 + 2 * i).unwrap_or(0) as usize;
                if off == 0 {
                    continue;
                }
                let set = st + off;
                let rn = t.u16(set).unwrap_or(0) as usize;
                for j in 0..rn {
                    let mut r = set + t.u16(set + 2 + 2 * j).unwrap_or(0) as usize;
                    let bc = t.u16(r).unwrap_or(0) as usize;
                    r += 2 + 2 * bc;
                    let ic = t.u16(r).unwrap_or(0) as usize;
                    r += 2 + 2 * ic.saturating_sub(1);
                    let lc = t.u16(r).unwrap_or(0) as usize;
                    r += 2 + 2 * lc;
                    let sc = t.u16(r).unwrap_or(0) as usize;
                    records(t, r + 2, sc, &mut s);
                }
            }
        }
        (6, 3) => {
            let mut r = st + 2;
            let bc = t.u16(r).unwrap_or(0) as usize;
            r += 2 + 2 * bc;
            let ic = t.u16(r).unwrap_or(0) as usize;
            r += 2 + 2 * ic;
            let lc = t.u16(r).unwrap_or(0) as usize;
            r += 2 + 2 * lc;
            let sc = t.u16(r).unwrap_or(0) as usize;
            records(t, r + 2, sc, &mut s);
        }
        _ => {}
    }
    add(s);
    found
}

fn feature_tags(t: &Be) -> Vec<(String, Vec<u16>)> {
    let Some(fl) = t.u16(6).map(|v| v as usize).filter(|&v| v != 0) else { return Vec::new() };
    let n = t.u16(fl).unwrap_or(0) as usize;
    let mut out = Vec::new();
    for i in 0..n {
        let r = fl + 2 + 6 * i;
        let Some(tag) = t.0.get(r..r + 4) else { break };
        let tag = String::from_utf8_lossy(tag).into_owned();
        let f = fl + t.u16(r + 4).unwrap_or(0) as usize;
        let cnt = t.u16(f + 2).unwrap_or(0) as usize;
        let idx = (0..cnt).filter_map(|k| t.u16(f + 4 + 2 * k)).collect();
        out.push((tag, idx));
    }
    out
}

fn lookup_subtables(t: &Be, li: usize) -> Vec<(u16, usize)> {
    let Some(ll) = t.u16(8).map(|v| v as usize).filter(|&v| v != 0) else { return Vec::new() };
    let lookup = ll + t.u16(ll + 2 + 2 * li).unwrap_or(0) as usize;
    let ltype = t.u16(lookup).unwrap_or(0);
    let n = t.u16(lookup + 4).unwrap_or(0) as usize;
    let mut out = Vec::new();
    for i in 0..n {
        let st = lookup + t.u16(lookup + 6 + 2 * i).unwrap_or(0) as usize;
        if ltype == 7 {
            let ext_type = t.u16(st + 2).unwrap_or(0);
            let off = t.u32(st + 4).unwrap_or(0) as usize;
            out.push((ext_type, st + off));
        } else {
            out.push((ltype, st));
        }
    }
    out
}

fn lookup_count(t: &Be) -> usize {
    match t.u16(8).map(|v| v as usize).filter(|&v| v != 0) {
        Some(ll) => t.u16(ll).unwrap_or(0) as usize,
        None => 0,
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Analysis {
    pub gsub_features: Vec<String>,
    pub gpos_features: Vec<String>,
    pub variants: IndexMap<String, IndexMap<String, Vec<String>>>,
    pub ligatures: Vec<(String, String, String)>,
}

fn analyze(raw: &ttf_parser::RawFace, cmap: &BTreeMap<u32, u16>, order: &[String]) -> Analysis {
    let mut glyph_to_char: HashMap<String, char> = HashMap::new();
    for (&code, &gid) in cmap {
        if let Some(ch) = char::from_u32(code) {
            glyph_to_char.entry(order[gid as usize].clone()).or_insert(ch);
        }
    }
    let mut res = Analysis::default();
    let mut variants: IndexMap<char, IndexMap<String, BTreeSet<String>>> = IndexMap::new();
    let name_of = |g: u16| order.get(g as usize).cloned().unwrap_or_default();
    let add_variant =
        |variants: &mut IndexMap<char, IndexMap<String, BTreeSet<String>>>, src: &str, dst: &str, source: &str| {
            let ch = glyph_to_char
                .get(src)
                .copied()
                .or_else(|| variants.iter().find(|(_, gl)| gl.contains_key(src)).map(|(c, _)| *c));
            let Some(ch) = ch else { return };
            if cmap.get(&(ch as u32)).map(|&g| name_of(g)).as_deref() == Some(dst) {
                return;
            }
            use GeneralCategory::*;
            let cat_ok = matches!(
                get_general_category(ch),
                UppercaseLetter
                    | LowercaseLetter
                    | TitlecaseLetter
                    | ModifierLetter
                    | OtherLetter
                    | DecimalNumber
                    | LetterNumber
                    | OtherNumber
                    | ConnectorPunctuation
                    | DashPunctuation
                    | OpenPunctuation
                    | ClosePunctuation
                    | InitialPunctuation
                    | FinalPunctuation
                    | OtherPunctuation
            );
            if !cat_ok {
                return;
            }
            if dst.split('.').skip(1).any(|part| TECH_SUFFIX.is_match(part)) {
                return;
            }
            variants.entry(ch).or_default().entry(dst.to_string()).or_default().insert(source.to_string());
        };

    if let Some(gpos) = raw.table(Tag::from_bytes(b"GPOS")) {
        let tags: BTreeSet<String> = feature_tags(&Be(gpos)).into_iter().map(|(t, _)| t).collect();
        res.gpos_features = tags.into_iter().collect();
    }
    if let Some(gsub) = raw.table(Tag::from_bytes(b"GSUB")) {
        let t = Be(gsub);
        let n_lookups = lookup_count(&t);
        let mut feats: BTreeMap<String, PySet<(i64, i64)>> = BTreeMap::new();
        for (tag, idx) in feature_tags(&t) {
            let set = feats.entry(tag).or_default();
            for i in idx {
                set.add((i as i64, 0), hash_int(i as i64));
            }
        }
        res.gsub_features = feats.keys().cloned().collect();
        let mut ligs: BTreeSet<(String, String, String)> = BTreeSet::new();
        for (tag, idxs) in &feats {
            let is_var = is_variant_feature(tag);
            let is_lig = LIGATURE_FEATURES.contains(&tag.as_str());
            if !is_var && !is_lig {
                continue;
            }
            let mut todo: Vec<usize> = idxs.iter().map(|(i, _)| i as usize).collect();
            let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
            while let Some(li) = todo.pop() {
                if seen.contains(&li) || li >= n_lookups {
                    continue;
                }
                seen.insert(li);
                for (ltype, st) in lookup_subtables(&t, li) {
                    let fmt = t.u16(st).unwrap_or(0);
                    match ltype {
                        1 if is_var => {
                            let cov = coverage(&t, st + t.u16(st + 2).unwrap_or(0) as usize);
                            for (k, &g) in cov.iter().enumerate() {
                                let dst = if fmt == 1 {
                                    (g as i32 + t.i16(st + 4).unwrap_or(0) as i32).rem_euclid(65536) as u16
                                } else {
                                    t.u16(st + 6 + 2 * k).unwrap_or(0)
                                };
                                add_variant(&mut variants, &name_of(g), &name_of(dst), tag);
                            }
                        }
                        3 if is_var => {
                            let cov = coverage(&t, st + t.u16(st + 2).unwrap_or(0) as usize);
                            for (k, &g) in cov.iter().enumerate() {
                                let set = st + t.u16(st + 6 + 2 * k).unwrap_or(0) as usize;
                                let n = t.u16(set).unwrap_or(0) as usize;
                                for j in 0..n {
                                    let b = t.u16(set + 2 + 2 * j).unwrap_or(0);
                                    add_variant(&mut variants, &name_of(g), &name_of(b), tag);
                                }
                            }
                        }
                        4 if is_lig => {
                            let cov = coverage(&t, st + t.u16(st + 2).unwrap_or(0) as usize);
                            for (k, &first) in cov.iter().enumerate() {
                                let set = st + t.u16(st + 6 + 2 * k).unwrap_or(0) as usize;
                                let n = t.u16(set).unwrap_or(0) as usize;
                                for j in 0..n {
                                    let lig = set + t.u16(set + 2 + 2 * j).unwrap_or(0) as usize;
                                    let lg = t.u16(lig).unwrap_or(0);
                                    let cc = t.u16(lig + 2).unwrap_or(0) as usize;
                                    let mut comps = vec![first];
                                    comps.extend((0..cc.saturating_sub(1)).filter_map(|c| t.u16(lig + 4 + 2 * c)));
                                    let chars: String = comps
                                        .iter()
                                        .map(|&c| glyph_to_char.get(&name_of(c)).copied().unwrap_or('?'))
                                        .collect();
                                    ligs.insert((chars, name_of(lg), tag.clone()));
                                }
                            }
                        }
                        5 | 6 => {
                            let mut nested = PySet::new();
                            for li2 in nested_lookups(&t, st, ltype) {
                                nested.add((li2 as i64, 0), hash_int(li2 as i64));
                            }
                            todo.extend(nested.iter().map(|(i, _)| i as usize));
                        }
                        8 if is_var => {
                            let cov = coverage(&t, st + t.u16(st + 2).unwrap_or(0) as usize);
                            let bc = t.u16(st + 4).unwrap_or(0) as usize;
                            let la = st + 6 + 2 * bc;
                            let lc = t.u16(la).unwrap_or(0) as usize;
                            let sub = la + 2 + 2 * lc;
                            let gc = t.u16(sub).unwrap_or(0) as usize;
                            for (k, &g) in cov.iter().enumerate().take(gc) {
                                let b = t.u16(sub + 2 + 2 * k).unwrap_or(0);
                                add_variant(&mut variants, &name_of(g), &name_of(b), tag);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        res.ligatures = ligs.into_iter().collect();
    }
    for name in order {
        let Some(c) = NAME_SPLIT.captures(name) else { continue };
        if name.starts_with('.') {
            continue;
        }
        let base = &c[1];
        if glyph_to_char.contains_key(base) {
            add_variant(&mut variants, base, name, &format!("суффикс .{}", &c[2]));
        }
    }
    let mut chars: Vec<char> = variants.keys().copied().collect();
    chars.sort();
    res.variants = chars
        .into_iter()
        .map(|ch| {
            let gl = &variants[&ch];
            (ch.to_string(), gl.iter().map(|(g, s)| (g.clone(), s.iter().cloned().collect())).collect())
        })
        .collect();
    res
}

struct ContourPen {
    tol: f64,
    contours: Vec<Vec<Point>>,
    cur: Option<Vec<Point>>,
}

impl ContourPen {
    fn flush(&mut self) {
        if let Some(c) = self.cur.take()
            && c.len() > 1
        {
            self.contours.push(c);
        }
    }

    fn move_to(&mut self, p: Point) {
        self.flush();
        self.cur = Some(vec![p]);
    }

    fn line_to(&mut self, p: Point) {
        self.cur.get_or_insert_with(Vec::new).push(p);
    }

    fn cubic(&mut self, p0: Point, p1: Point, p2: Point, p3: Point, depth: u32) {
        if depth > 12 || cubic_flat(p0, p1, p2, p3, self.tol) {
            self.cur.get_or_insert_with(Vec::new).push(p3);
            return;
        }
        let (p01, p12, p23) = (mid(p0, p1), mid(p1, p2), mid(p2, p3));
        let (p012, p123) = (mid(p01, p12), mid(p12, p23));
        let m = mid(p012, p123);
        self.cubic(p0, p01, p012, m, depth + 1);
        self.cubic(m, p123, p23, p3, depth + 1);
    }

    fn last(&self) -> Point {
        *self.cur.as_ref().and_then(|c| c.last()).expect("current point")
    }

    fn qcurve_one(&mut self, p1: Point, p2: Point) {
        let p0 = self.last();
        let c1 = (p0.0 + 2.0 / 3.0 * (p1.0 - p0.0), p0.1 + 2.0 / 3.0 * (p1.1 - p0.1));
        let c2 = (p2.0 + 2.0 / 3.0 * (p1.0 - p2.0), p2.1 + 2.0 / 3.0 * (p1.1 - p2.1));
        self.cubic(p0, c1, c2, p2, 0);
    }

    fn curve_one(&mut self, p1: Point, p2: Point, p3: Point) {
        let p0 = self.last();
        self.cubic(p0, p1, p2, p3, 0);
    }

    fn qcurve_to(&mut self, points: &[Option<Point>]) {
        let mut pts: Vec<Point> = points.iter().flatten().copied().collect();
        if points.last() == Some(&None) {
            let (x, y) = pts[pts.len() - 1];
            let (nx, ny) = pts[0];
            let implied = (0.5 * (x + nx), 0.5 * (y + ny));
            self.move_to(implied);
            pts.push(implied);
        }
        if pts.len() > 1 {
            let n = pts.len() - 1;
            for i in 0..n.saturating_sub(1) {
                let (x, y) = pts[i];
                let (nx, ny) = pts[i + 1];
                self.qcurve_one(pts[i], (0.5 * (x + nx), 0.5 * (y + ny)));
            }
            self.qcurve_one(pts[n - 1], pts[n]);
        } else if let Some(&p) = pts.first() {
            self.line_to(p);
        }
    }

    fn close_path(&mut self) {
        if let Some(c) = self.cur.as_mut()
            && let (Some(&a), Some(&b)) = (c.first(), c.last())
            && a != b
        {
            c.push(a);
        }
        self.flush();
    }
}

type Affine = [f64; 6];

fn apply(t: &Affine, p: Point) -> Point {
    (t[0] * p.0 + t[2] * p.1 + t[4], t[1] * p.0 + t[3] * p.1 + t[5])
}

fn compose(outer: &Affine, inner: &Affine) -> Affine {
    let [xx1, xy1, yx1, yy1, dx1, dy1] = *inner;
    let [xx2, xy2, yx2, yy2, dx2, dy2] = *outer;
    [
        xx1 * xx2 + xy1 * yx2,
        xx1 * xy2 + xy1 * yy2,
        yx1 * xx2 + yy1 * yx2,
        yx1 * xy2 + yy1 * yy2,
        xx2 * dx1 + yx2 * dy1 + dx2,
        xy2 * dx1 + yy2 * dy1 + dy2,
    ]
}

struct Glyf<'a> {
    glyf: Be<'a>,
    loca: Vec<usize>,
}

impl<'a> Glyf<'a> {
    fn new(raw: &ttf_parser::RawFace<'a>, num_glyphs: usize) -> Option<Self> {
        let glyf = raw.table(Tag::from_bytes(b"glyf"))?;
        let loca = Be(raw.table(Tag::from_bytes(b"loca"))?);
        let head = Be(raw.table(Tag::from_bytes(b"head"))?);
        let long = head.i16(50)? == 1;
        let loca =
            (0..=num_glyphs)
                .map(|i| {
                    if long { loca.u32(4 * i).unwrap_or(0) as usize } else { loca.u16(2 * i).unwrap_or(0) as usize * 2 }
                })
                .collect();
        Some(Self { glyf: Be(glyf), loca })
    }

    fn data(&self, gid: usize) -> Option<usize> {
        let (a, b) = (*self.loca.get(gid)?, *self.loca.get(gid + 1)?);
        (b > a).then_some(a)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_glyf(face: &ttf_parser::Face, g: &Glyf, gid: usize, t: Option<&Affine>, pen: &mut ContourPen, depth: u32) {
    if depth > 16 {
        return;
    }
    let Some(o) = g.data(gid) else { return };
    let d = &g.glyf;
    let n_contours = d.i16(o).unwrap_or(0);
    let x_min = d.i16(o + 2).unwrap_or(0) as f64;
    let tp = |p: Point| match t {
        Some(t) => apply(t, p),
        None => p,
    };
    if n_contours < 0 {
        let mut p = o + 10;
        loop {
            let flags = d.u16(p).unwrap_or(0);
            let cid = d.u16(p + 2).unwrap_or(0) as usize;
            p += 4;
            let (x, y) = if flags & 0x0001 != 0 {
                let v = (d.i16(p).unwrap_or(0) as f64, d.i16(p + 2).unwrap_or(0) as f64);
                p += 4;
                v
            } else {
                let b = &d.0;
                let v = (b.get(p).map_or(0, |&v| v as i8) as f64, b.get(p + 1).map_or(0, |&v| v as i8) as f64);
                p += 2;
                v
            };
            let f2 = |o: usize| d.i16(o).unwrap_or(0) as f64 / 16384.0;
            let ct: Affine = if flags & 0x0008 != 0 {
                let s = f2(p);
                p += 2;
                [s, 0.0, 0.0, s, x, y]
            } else if flags & 0x0040 != 0 {
                let (sx, sy) = (f2(p), f2(p + 2));
                p += 4;
                [sx, 0.0, 0.0, sy, x, y]
            } else if flags & 0x0080 != 0 {
                let (a, b, c, e) = (f2(p), f2(p + 2), f2(p + 4), f2(p + 6));
                p += 8;
                [a, b, c, e, x, y]
            } else {
                [1.0, 0.0, 0.0, 1.0, x, y]
            };
            let combined = match t {
                Some(outer) => compose(outer, &ct),
                None => ct,
            };
            draw_glyf(face, g, cid, Some(&combined), pen, depth + 1);
            if flags & 0x0020 == 0 {
                break;
            }
        }
        return;
    }
    let n = n_contours as usize;
    let ends: Vec<usize> = (0..n).map(|i| d.u16(o + 10 + 2 * i).unwrap_or(0) as usize).collect();
    let n_pts = ends.last().map_or(0, |e| e + 1);
    let ins_len = d.u16(o + 10 + 2 * n).unwrap_or(0) as usize;
    let mut p = o + 12 + 2 * n + ins_len;
    let b = &d.0;
    let mut flags = Vec::with_capacity(n_pts);
    while flags.len() < n_pts {
        let f = b.get(p).copied().unwrap_or(0);
        p += 1;
        flags.push(f);
        if f & 0x08 != 0 {
            let r = b.get(p).copied().unwrap_or(0);
            p += 1;
            for _ in 0..r {
                flags.push(f);
            }
        }
    }
    flags.truncate(n_pts);
    let mut read_coords = |short: u8, same: u8| -> Vec<f64> {
        let mut v = 0i32;
        let mut out = Vec::with_capacity(n_pts);
        for &f in &flags {
            if f & short != 0 {
                let dv = b.get(p).copied().unwrap_or(0) as i32;
                p += 1;
                v += if f & same != 0 { dv } else { -dv };
            } else if f & same == 0 {
                v += d.i16(p).unwrap_or(0) as i32;
                p += 2;
            }
            out.push(v as f64);
        }
        out
    };
    let xs = read_coords(0x02, 0x10);
    let ys = read_coords(0x04, 0x20);
    let lsb = face.glyph_hor_side_bearing(GlyphId(gid as u16)).unwrap_or(0) as f64;
    let offset = lsb - x_min;
    let mut start = 0;
    for &end in &ends {
        let end = end + 1;
        let contour: Vec<Point> =
            (start..end.min(n_pts)).map(|i| tp((if offset != 0.0 { xs[i] + offset } else { xs[i] }, ys[i]))).collect();
        let on: Vec<bool> = (start..end.min(n_pts)).map(|i| flags[i] & 1 != 0).collect();
        start = end;
        if contour.is_empty() {
            pen.close_path();
            continue;
        }
        if !on.contains(&true) {
            let mut pts: Vec<Option<Point>> = contour.iter().map(|&p| Some(p)).collect();
            pts.push(None);
            pen.qcurve_to(&pts);
        } else {
            let first_on = on.iter().position(|&v| v).expect("on-curve point") + 1;
            let mut c: Vec<Point> = contour[first_on..].iter().chain(&contour[..first_on]).copied().collect();
            let mut f: Vec<bool> = on[first_on..].iter().chain(&on[..first_on]).copied().collect();
            pen.move_to(c[c.len() - 1]);
            while !c.is_empty() {
                let next_on = f.iter().position(|&v| v).expect("ends on-curve") + 1;
                if next_on == 1 {
                    if c.len() > 1 {
                        pen.line_to(c[0]);
                    }
                } else {
                    let seg: Vec<Option<Point>> = c[..next_on].iter().map(|&p| Some(p)).collect();
                    pen.qcurve_to(&seg);
                }
                c.drain(..next_on);
                f.drain(..next_on);
            }
        }
        pen.close_path();
    }
}

struct CffBuilder<'p> {
    pen: &'p mut ContourPen,
}

impl ttf_parser::OutlineBuilder for CffBuilder<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pen.move_to((x as f64, y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.pen.line_to((x as f64, y as f64));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.pen.qcurve_one((x1 as f64, y1 as f64), (x as f64, y as f64));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.pen.curve_one((x1 as f64, y1 as f64), (x2 as f64, y2 as f64), (x as f64, y as f64));
    }
    fn close(&mut self) {
        self.pen.close_path();
    }
}

const MAC_ROMAN: [char; 128] = [
    'Ä', 'Å', 'Ç', 'É', 'Ñ', 'Ö', 'Ü', 'á', 'à', 'â', 'ä', 'ã', 'å', 'ç', 'é', 'è', 'ê', 'ë', 'í', 'ì', 'î', 'ï', 'ñ',
    'ó', 'ò', 'ô', 'ö', 'õ', 'ú', 'ù', 'û', 'ü', '†', '°', '¢', '£', '§', '•', '¶', 'ß', '®', '©', '™', '´', '¨', '≠',
    'Æ', 'Ø', '∞', '±', '≤', '≥', '¥', 'µ', '∂', '∑', '∏', 'π', '∫', 'ª', 'º', 'Ω', 'æ', 'ø', '¿', '¡', '¬', '√', 'ƒ',
    '≈', '∆', '«', '»', '…', '\u{a0}', 'À', 'Ã', 'Õ', 'Œ', 'œ', '–', '—', '“', '”', '‘', '’', '÷', '◊', 'ÿ', 'Ÿ', '⁄',
    '€', '‹', '›', 'ﬁ', 'ﬂ', '‡', '·', '‚', '„', '‰', 'Â', 'Ê', 'Á', 'Ë', 'È', 'Í', 'Î', 'Ï', 'Ì', 'Ó', 'Ô',
    '\u{f8ff}', 'Ò', 'Ú', 'Û', 'Ù', 'ı', 'ˆ', '˜', '¯', '˘', '˙', '˚', '¸', '˝', '˛', 'ˇ',
];

fn decode_name(n: &ttf_parser::name::Name) -> Option<String> {
    match n.platform_id {
        PlatformId::Unicode | PlatformId::Windows => {
            if n.platform_id == PlatformId::Windows && !matches!(n.encoding_id, 0 | 1 | 10) {
                return None;
            }
            let units: Vec<u16> = n.name.as_chunks::<2>().0.iter().map(|&c| u16::from_be_bytes(c)).collect();
            String::from_utf16(&units).ok()
        }
        PlatformId::Macintosh if n.encoding_id == 0 => {
            Some(n.name.iter().map(|&b| if b < 128 { b as char } else { MAC_ROMAN[(b - 128) as usize] }).collect())
        }
        _ => None,
    }
}

fn debug_name(face: &ttf_parser::Face, id: u16) -> Option<String> {
    let mut some = None;
    for n in face.names() {
        if n.name_id != id {
            continue;
        }
        let Some(s) = decode_name(&n) else { continue };
        let plat = match n.platform_id {
            PlatformId::Macintosh => 1,
            PlatformId::Windows => 3,
            PlatformId::Unicode => 0,
            _ => 99,
        };
        if (plat, n.language_id) == (1, 0) || (plat, n.language_id) == (3, 0x409) {
            return Some(s);
        }
        some = Some(s);
    }
    some
}

fn best_cmap(face: &ttf_parser::Face) -> BTreeMap<u32, u16> {
    let prefs = [(3, 10), (0, 6), (0, 4), (3, 1), (0, 3), (0, 2), (0, 1), (0, 0)];
    let Some(cmap) = face.tables().cmap else { return BTreeMap::new() };
    for (pid, eid) in prefs {
        for st in cmap.subtables {
            let p = match st.platform_id {
                PlatformId::Unicode => 0,
                PlatformId::Macintosh => 1,
                PlatformId::Windows => 3,
                _ => 9,
            };
            if p != pid || st.encoding_id != eid {
                continue;
            }
            let mut m = BTreeMap::new();
            st.codepoints(|c| {
                if let Some(g) = st.glyph_index(c) {
                    m.insert(c, g.0);
                }
            });
            return m;
        }
    }
    BTreeMap::new()
}

fn glyph_order(face: &ttf_parser::Face, cmap: &BTreeMap<u32, u16>) -> Vec<String> {
    let n = face.number_of_glyphs() as usize;
    let mut by_gid: HashMap<u16, u32> = HashMap::new();
    for (&c, &g) in cmap {
        by_gid.entry(g).or_insert(c);
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    (0..n)
        .map(|i| {
            let base = match face.glyph_name(GlyphId(i as u16)) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ if i == 0 => ".notdef".into(),
                _ => match by_gid.get(&(i as u16)) {
                    Some(&c) if c <= 0xFFFF => format!("uni{c:04X}"),
                    Some(&c) => format!("u{c:X}"),
                    None => format!("glyph{i:05}"),
                },
            };
            let k = seen.entry(base.clone()).or_insert(0);
            let name = if *k == 0 { base.clone() } else { format!("{base}#{k}") };
            *k += 1;
            name
        })
        .collect()
}

pub struct FontData {
    pub path: PathBuf,
    data: Arc<Vec<u8>>,
    pub upem: f64,
    pub order: Vec<String>,
    gid_of: HashMap<String, u16>,
    pub cmap: BTreeMap<u32, u16>,
    advances: Vec<u16>,
    pub analysis: Analysis,
    pub name: String,
    pub metrics: FontMetrics,
    contour_cache: Mutex<HashMap<u16, Arc<Vec<Vec<Point>>>>>,
    glyph_cache: Mutex<HashMap<(u16, String), Glyph>>,
}

impl FontData {
    pub fn open(path: &Path) -> Result<Self, FontError> {
        let data = std::fs::read(path).map_err(|e| FontError(format!("{}: {e}", path.display())))?;
        Self::from_bytes(path, data)
    }

    pub fn from_bytes(path: &Path, data: Vec<u8>) -> Result<Self, FontError> {
        let data = Arc::new(data);
        let face = ttf_parser::Face::parse(&data, 0).map_err(|e| FontError(format!("{}: {e}", path.display())))?;
        let upem = face.units_per_em() as f64;
        let cmap = best_cmap(&face);
        let order = glyph_order(&face, &cmap);
        let gid_of = order.iter().enumerate().map(|(i, n)| (n.clone(), i as u16)).collect();
        let advances = (0..order.len()).map(|i| face.glyph_hor_advance(GlyphId(i as u16)).unwrap_or(0)).collect();
        let analysis = analyze(face.raw_face(), &cmap, &order);
        let name = debug_name(&face, 4)
            .or_else(|| debug_name(&face, 1))
            .unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
        let mut fd = FontData {
            path: path.to_path_buf(),
            data: data.clone(),
            upem,
            order,
            gid_of,
            cmap,
            advances,
            analysis,
            name,
            metrics: FontMetrics {
                x_height: 0.0,
                cap_height: 0.0,
                ascent: 0.0,
                descent: 0.0,
                x_height_source: String::new(),
            },
            contour_cache: Mutex::new(HashMap::new()),
            glyph_cache: Mutex::new(HashMap::new()),
        };
        fd.metrics = fd.compute_metrics(&face);
        Ok(fd)
    }

    fn face(&self) -> ttf_parser::Face<'_> {
        ttf_parser::Face::parse(&self.data, 0).expect("font parsed before")
    }

    pub fn contours(&self, gid: u16) -> Arc<Vec<Vec<Point>>> {
        if let Some(c) = self.contour_cache.lock().expect("cache lock").get(&gid) {
            return c.clone();
        }
        let face = self.face();
        let mut pen = ContourPen { tol: self.upem * 0.0002, contours: Vec::new(), cur: None };
        match Glyf::new(face.raw_face(), self.order.len()) {
            Some(g) => draw_glyf(&face, &g, gid as usize, None, &mut pen, 0),
            None => {
                let mut b = CffBuilder { pen: &mut pen };
                let _ = face.outline_glyph(GlyphId(gid), &mut b);
            }
        }
        pen.flush();
        let s = 1.0 / self.upem;
        let c: Vec<Vec<Point>> =
            pen.contours.iter().map(|ct| ct.iter().map(|&(x, y)| (x * s, y * s)).collect()).collect();
        let c = Arc::new(c);
        self.contour_cache.lock().expect("cache lock").insert(gid, c.clone());
        c
    }

    fn top(&self, ch: char) -> Option<f64> {
        let gid = *self.cmap.get(&(ch as u32))?;
        self.contours(gid).iter().flatten().map(|p| p.1).reduce(crate::numeric::max)
    }

    fn compute_metrics(&self, face: &ttf_parser::Face) -> FontMetrics {
        let os2 = face.tables().os2;
        let sx = os2.and_then(|o| o.x_height()).map_or(0.0, |v| v as f64 / self.upem);
        let mut found = None;
        for ch in ['х', 'x'] {
            if let Some(t) = self.top(ch).filter(|&t| t > 0.0) {
                let extra = if sx != 0.0 { format!(", sxHeight {sx:.3}") } else { String::new() };
                found = Some((t, format!("по глифу «{ch}»{extra}")));
                break;
            }
        }
        let (xh, src) = found.unwrap_or_else(|| {
            if sx > 0.0 {
                (sx, "OS/2.sxHeight".into())
            } else {
                (0.5, "не найдена, принято 0.5 em".into())
            }
        });
        let mut cap = os2.and_then(|o| o.capital_height()).map_or(0.0, |v| v as f64 / self.upem);
        if cap == 0.0 {
            cap =
                self.top('Н').filter(|&v| v != 0.0).or_else(|| self.top('H').filter(|&v| v != 0.0)).unwrap_or(xh * 1.4);
        }
        let hhea = face.tables().hhea;
        FontMetrics {
            x_height: xh,
            cap_height: cap,
            ascent: hhea.ascender as f64 / self.upem,
            descent: hhea.descender as f64 / self.upem,
            x_height_source: src,
        }
    }

    fn resolve(&self, name: &str) -> Option<u16> {
        if let Some(num) = name.strip_prefix('#')
            && !num.is_empty()
            && num.bytes().all(|b| b.is_ascii_digit())
        {
            return num.parse::<usize>().ok().filter(|&i| i < self.order.len()).map(|i| i as u16);
        }
        self.gid_of.get(name).copied()
    }

    fn advance_of(&self, gid: u16) -> f64 {
        self.advances[gid as usize] as f64 / self.upem
    }
}

#[derive(Clone)]
pub struct OutlineGlyphProvider {
    pub data: Arc<FontData>,
    pub params: SkeletonParams,
}

impl OutlineGlyphProvider {
    pub fn from_path(path: &Path) -> Result<Self, FontError> {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ext != "ttf" && ext != "otf" {
            return Err(FontError(format!(
                "Режим «Контуры» принимает .ttf или .otf: {}",
                path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            )));
        }
        Ok(Self { data: Arc::new(FontData::open(path)?), params: SkeletonParams::default() })
    }

    pub fn with_params(&self, params: SkeletonParams) -> Self {
        Self { data: self.data.clone(), params }
    }

    fn compute(&self, gid: u16) -> Glyph {
        let key = self.params.key();
        if let Some(g) = self.data.glyph_cache.lock().expect("cache lock").get(&(gid, key.clone())) {
            return g.clone();
        }
        let res = skeleton_strokes(&self.data.contours(gid), self.data.metrics.x_height, &self.params, false);
        let g = Glyph {
            name: self.data.order[gid as usize].clone(),
            strokes: res.strokes,
            advance: self.data.advance_of(gid),
        };
        self.data.glyph_cache.lock().expect("cache lock").insert((gid, key), g.clone());
        g
    }

    pub fn debug_glyph(&self, name: &str) -> Option<serde_json::Value> {
        let gid = self.data.resolve(name)?;
        let t = std::time::Instant::now();
        let outline = self.data.contours(gid);
        let res = skeleton_strokes(&outline, self.data.metrics.x_height, &self.params, true);
        let pts = |v: &[Vec<Point>]| {
            serde_json::json!(v.iter().map(|s| s.iter().map(|p| [p.0, p.1]).collect::<Vec<_>>()).collect::<Vec<_>>())
        };
        Some(serde_json::json!({
            "name": self.data.order[gid as usize], "outline": pts(&outline), "strokes": pts(&res.strokes),
            "raw": pts(&res.raw), "closed": res.closed, "advance": self.data.advance_of(gid),
            "ms": (t.elapsed().as_secs_f64() * 1000.0).round() as i64,
        }))
    }
}

impl GlyphProvider for OutlineGlyphProvider {
    fn mode(&self) -> &'static str {
        "outlines"
    }

    fn name(&self) -> &str {
        &self.data.name
    }

    fn metrics(&self) -> &FontMetrics {
        &self.data.metrics
    }

    fn glyph(&self, name: &str) -> Option<Glyph> {
        Some(self.compute(self.data.resolve(name)?))
    }

    fn glyph_names_for_char(&self, ch: &str) -> Vec<String> {
        let mut it = ch.chars();
        let (Some(c), None) = (it.next(), it.next()) else { return Vec::new() };
        let Some(&gid) = self.data.cmap.get(&(c as u32)) else { return Vec::new() };
        let primary = self.data.order[gid as usize].clone();
        let mut out = vec![primary.clone()];
        if let Some(v) = self.data.analysis.variants.get(ch) {
            out.extend(v.keys().filter(|n| **n != primary).cloned());
        }
        out
    }

    fn info(&self) -> FontInfo {
        let mut chars: Vec<char> =
            self.data.cmap.keys().filter(|&&c| c > 31).filter_map(|&c| char::from_u32(c)).collect();
        chars.sort();
        let a = &self.data.analysis;
        FontInfo {
            name: self.data.name.clone(),
            mode: "outlines".into(),
            source: self.data.path.display().to_string(),
            glyph_count: self.data.order.len(),
            chars: chars.into_iter().collect(),
            variants: self.variants(),
            metrics: Some(self.data.metrics.clone()),
            notes: Vec::new(),
            features: a.gsub_features.clone(),
            gpos_features: a.gpos_features.clone(),
            variant_sources: a.variants.clone(),
            ligatures: a.ligatures.clone(),
        }
    }

    fn has_char(&self, ch: &str) -> bool {
        let mut it = ch.chars();
        matches!((it.next(), it.next()), (Some(c), None) if self.data.cmap.contains_key(&(c as u32)))
    }

    fn glyph_names(&self) -> Vec<String> {
        self.data.order.clone()
    }

    fn variants(&self) -> IndexMap<String, Vec<String>> {
        self.data.analysis.variants.keys().map(|ch| (ch.clone(), self.glyph_names_for_char(ch))).collect()
    }

    fn advance(&self, name: &str) -> Option<f64> {
        self.data.resolve(name).map(|g| self.data.advance_of(g))
    }

    fn variant_pool(&self, ch: &str) -> Vec<String> {
        let names = self.glyph_names_for_char(ch);
        if names.len() < 2 {
            return names;
        }
        let empty = IndexMap::new();
        let src = self.data.analysis.variants.get(ch).unwrap_or(&empty);
        let mut out = vec![names[0].clone()];
        out.extend(
            names[1..].iter().filter(|n| src.get(*n).is_some_and(|s| s.iter().any(|x| is_free_variant(x)))).cloned(),
        );
        out
    }

    fn space_advance(&self) -> f64 {
        self.data.cmap.get(&32).map_or(0.3, |&g| self.data.advance_of(g))
    }

    fn shape(&self, text: &str) -> Vec<ShapedGlyph> {
        let Some(face) = rustybuzz::Face::from_slice(&self.data.data, 0) else { return Vec::new() };
        let mut buf = rustybuzz::UnicodeBuffer::new();
        for (i, ch) in text.chars().enumerate() {
            buf.add(ch, i as u32);
        }
        buf.guess_segment_properties();
        if let Ok(lang) = "ru".parse::<rustybuzz::Language>() {
            buf.set_language(lang);
        }
        let features: Vec<rustybuzz::Feature> = SHAPING_FEATURES
            .iter()
            .map(|t| rustybuzz::Feature::new(Tag::from_bytes(t.as_bytes().try_into().expect("4-byte tag")), 1, ..))
            .collect();
        let out = rustybuzz::shape(&face, &features, buf);
        let s = 1.0 / self.data.upem;
        out.glyph_infos()
            .iter()
            .zip(out.glyph_positions())
            .filter(|(info, _)| info.glyph_id != 0)
            .map(|(info, pos)| ShapedGlyph {
                name: self.data.order[info.glyph_id as usize].clone(),
                cluster: info.cluster as usize,
                advance: pos.x_advance as f64 * s,
                x_offset: pos.x_offset as f64 * s,
                y_offset: pos.y_offset as f64 * s,
            })
            .collect()
    }

    fn prepare(&self, names: &[String]) {
        let key = self.params.key();
        let cached: std::collections::HashSet<u16> = self
            .data
            .glyph_cache
            .lock()
            .expect("cache lock")
            .keys()
            .filter(|(_, k)| *k == key)
            .map(|(g, _)| *g)
            .collect();
        let todo: BTreeSet<u16> =
            names.iter().filter_map(|n| self.data.resolve(n)).filter(|g| !cached.contains(g)).collect();
        if todo.len() < 2 {
            return;
        }
        todo.into_par_iter().for_each(|g| {
            self.compute(g);
        });
    }
}
