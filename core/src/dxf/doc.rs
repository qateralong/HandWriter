use std::collections::HashMap;
use std::f64::consts::TAU;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use regex::bytes::Regex as BRegex;

use super::geom::*;

#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    S(String),
    I(i64),
    F(f64),
    P(V3),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    pub code: i32,
    pub val: Val,
}

impl Tag {
    pub fn s(&self) -> &str {
        match &self.val {
            Val::S(s) => s,
            _ => "",
        }
    }

    pub fn f(&self) -> f64 {
        match self.val {
            Val::F(v) => v,
            Val::I(v) => v as f64,
            Val::P(p) => p.x,
            Val::S(_) => 0.0,
        }
    }

    pub fn i(&self) -> i64 {
        match self.val {
            Val::I(v) => v,
            Val::F(v) => v as i64,
            _ => 0,
        }
    }

    pub fn p(&self) -> V3 {
        match self.val {
            Val::P(p) => p,
            Val::F(v) => v3(v, 0.0, 0.0),
            _ => NULLVEC,
        }
    }
}

const POINT_CODES: [i32; 20] =
    [10, 11, 12, 13, 14, 15, 16, 17, 18, 110, 111, 112, 210, 211, 212, 213, 1010, 1011, 1012, 1013];

fn is_point_code(c: i32) -> bool {
    POINT_CODES.contains(&c)
}

fn is_invalid_code(c: i32) -> bool {
    POINT_CODES.iter().any(|&p| c == p + 10 || (c == p + 20 && c != 38))
}

enum Kind {
    Str,
    Int,
    Float,
    Binary,
}

fn code_type(c: i32) -> Kind {
    let r = |a: i32, b: i32| (a..=b).contains(&c);
    if r(10, 59) || r(110, 149) || r(210, 239) || r(460, 469) || r(1010, 1059) {
        Kind::Float
    } else if r(60, 79)
        || r(90, 99)
        || r(160, 179)
        || r(270, 299)
        || r(370, 389)
        || r(400, 409)
        || r(420, 429)
        || r(440, 459)
        || r(1060, 1071)
    {
        Kind::Int
    } else if r(310, 319) || c == 1004 {
        Kind::Binary
    } else {
        Kind::Str
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Enc {
    Utf8,
    Rs(&'static encoding_rs::Encoding),
}

fn to_encoding(cp: &str) -> Enc {
    let table: [(&str, &'static encoding_rs::Encoding); 14] = [
        ("874", encoding_rs::WINDOWS_874),
        ("932", encoding_rs::SHIFT_JIS),
        ("936", encoding_rs::GBK),
        ("949", encoding_rs::EUC_KR),
        ("950", encoding_rs::BIG5),
        ("1250", encoding_rs::WINDOWS_1250),
        ("1251", encoding_rs::WINDOWS_1251),
        ("1252", encoding_rs::WINDOWS_1252),
        ("1253", encoding_rs::WINDOWS_1253),
        ("1254", encoding_rs::WINDOWS_1254),
        ("1255", encoding_rs::WINDOWS_1255),
        ("1256", encoding_rs::WINDOWS_1256),
        ("1257", encoding_rs::WINDOWS_1257),
        ("1258", encoding_rs::WINDOWS_1258),
    ];
    for (k, e) in table {
        if cp.ends_with(k) {
            return Enc::Rs(e);
        }
    }
    Enc::Rs(encoding_rs::WINDOWS_1252)
}

fn decode(b: &[u8], enc: Enc) -> String {
    match enc {
        Enc::Utf8 => String::from_utf8_lossy(b).into_owned(),
        Enc::Rs(e) => e.decode_without_bom_handling(b).0.into_owned(),
    }
}

fn decode_strict_ok(b: &[u8], enc: Enc) -> bool {
    match enc {
        Enc::Utf8 => std::str::from_utf8(b).is_ok(),
        Enc::Rs(e) => !e.decode_without_bom_handling(b).1,
    }
}

static UNI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\U\+[A-Fa-f0-9]{4}").unwrap());
static MIF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\M\+[1-5][A-Fa-f0-9]{4}").unwrap());
static INT_RE: LazyLock<BRegex> = LazyLock::new(|| BRegex::new(r"[+-]?\d+").unwrap());
static FLOAT_RE: LazyLock<BRegex> = LazyLock::new(|| BRegex::new(r"[+-]?\d+(:?\.\d*)?(:?[eE][+-]?\d+)?").unwrap());

fn decode_mif(s: &str) -> String {
    let enc = match &s[3..4] {
        "1" => Some(encoding_rs::SHIFT_JIS),
        "2" => Some(encoding_rs::BIG5),
        "3" => Some(encoding_rs::EUC_KR),
        "5" => Some(encoding_rs::GBK),
        _ => None,
    };
    let Some(enc) = enc else { return s.to_string() };
    let hex = &s[4..];
    let bytes: Vec<u8> =
        (0..hex.len() / 2).filter_map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()).collect();
    let (out, had_err) = enc.decode_without_bom_handling(&bytes);
    if had_err { s.to_string() } else { out.into_owned() }
}

fn decode_special(s: String) -> String {
    if UNI.is_match(&s) {
        UNI.replace_all(&s, |c: &regex::Captures| {
            let v = u32::from_str_radix(&c[0][3..], 16).unwrap_or(0xFFFD);
            char::from_u32(v).unwrap_or('\u{FFFD}').to_string()
        })
        .into_owned()
    } else if MIF.is_match(&s) {
        MIF.replace_all(&s, |c: &regex::Captures| decode_mif(&c[0])).into_owned()
    } else {
        s
    }
}

fn trim_ascii(b: &[u8]) -> &[u8] {
    let s = b.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(b.len());
    let e = b.iter().rposition(|c| !c.is_ascii_whitespace()).map_or(s, |e| e + 1);
    &b[s..e]
}

fn parse_int(b: &[u8]) -> Option<i64> {
    let t = trim_ascii(b);
    if let Some(v) = std::str::from_utf8(t).ok().and_then(|s| s.replace('_', "").parse::<i64>().ok()) {
        return Some(v);
    }
    let m = INT_RE.find(b)?;
    std::str::from_utf8(m.as_bytes()).ok()?.parse::<i64>().ok()
}

fn parse_float(b: &[u8]) -> Option<f64> {
    let t = trim_ascii(b);
    if let Some(v) = std::str::from_utf8(t).ok().and_then(|s| s.parse::<f64>().ok()) {
        return Some(v);
    }
    let stripped: Vec<u8> = b.iter().copied().filter(|c| !c.is_ascii_whitespace()).collect();
    let m = FLOAT_RE.find(&stripped)?;
    std::str::from_utf8(m.as_bytes()).ok()?.parse::<f64>().ok()
}

fn raw_tags(data: &[u8]) -> Result<Vec<(i32, Vec<u8>)>, String> {
    let mut lines = data.split_inclusive(|&c| c == b'\n');
    let mut out = Vec::new();
    let mut line = 1;
    while let Some(code) = lines.next() {
        let code_s = trim_ascii(code);
        let c = match std::str::from_utf8(code_s).ok().and_then(|s| s.parse::<i32>().ok()) {
            Some(c) => c,
            None => match INT_RE.find(code).and_then(|m| std::str::from_utf8(m.as_bytes()).ok()?.parse::<i32>().ok()) {
                Some(c) => c,
                None => {
                    let shown = String::from_utf8_lossy(code).replace('\u{FFFD}', "");
                    return Err(format!(
                        "Invalid group code \"{}\" at line {line}.",
                        shown.trim_end_matches(['\r', '\n'])
                    ));
                }
            },
        };
        let Some(value) = lines.next() else { break };
        let mut v = value;
        while let Some((&last, rest)) = v.split_last() {
            if last == b'\n' || last == b'\r' { v = rest } else { break }
        }
        line += 2;
        let eof = c == 0 && v == b"EOF";
        if c != 999 {
            out.push((c, v.to_vec()));
        }
        if eof {
            break;
        }
    }
    Ok(out)
}

fn detect_encoding(tags: &[(i32, Vec<u8>)]) -> Enc {
    let mut encoding: Option<Enc> = None;
    let mut version: Option<String> = None;
    let mut next = "";
    for (c, v) in tags {
        if *c == 9 {
            if v == b"$DWGCODEPAGE" {
                next = "cp";
            } else if v == b"$ACADVER" {
                next = "ver";
            }
        } else if *c == 3 && next == "cp" {
            encoding = Some(to_encoding(&decode(v, Enc::Rs(encoding_rs::WINDOWS_1252))));
            next = "";
        } else if *c == 1 && next == "ver" {
            version = Some(decode(v, Enc::Rs(encoding_rs::WINDOWS_1252)));
            next = "";
        }
        if let (Some(e), Some(ver)) = (encoding, &version) {
            return if ver.as_str() >= "AC1021" { Enc::Utf8 } else { e };
        }
    }
    Enc::Rs(encoding_rs::WINDOWS_1252)
}

fn filter_point_codes(tags: Vec<(i32, Vec<u8>)>) -> Vec<(i32, Vec<u8>)> {
    let mut out = Vec::with_capacity(tags.len());
    let mut expected = -1;
    let mut z_code = 0;
    let mut point: Vec<(i32, Vec<u8>)> = Vec::new();
    for tag in tags {
        let code = tag.0;
        if !point.is_empty() && code != expected {
            if point.len() > 1 {
                out.append(&mut point);
            }
            point.clear();
        }
        if is_point_code(code) {
            expected = code + 10;
            z_code = code + 20;
            point.push(tag);
        } else if code == expected {
            point.push(tag);
            expected += 10;
            if expected > z_code {
                expected = -1;
            }
        } else if !is_invalid_code(code) {
            out.push(tag);
        }
    }
    if point.len() > 1 {
        out.append(&mut point);
    }
    out
}

fn compile(tags: Vec<(i32, Vec<u8>)>, enc: Enc) -> Result<(Vec<Tag>, usize), String> {
    let mut out = Vec::with_capacity(tags.len());
    let mut errors = 0;
    let mut i = 0;
    let strict_float = |v: &[u8]| std::str::from_utf8(trim_ascii(v)).ok().and_then(|s| s.parse::<f64>().ok());
    let strict_int =
        |v: &[u8]| std::str::from_utf8(trim_ascii(v)).ok().and_then(|s| s.replace('_', "").parse::<i64>().ok());
    while i < tags.len() {
        let (code, ref v) = tags[i];
        i += 1;
        if is_point_code(code) {
            let Some((yc, yv)) = tags.get(i) else { break };
            i += 1;
            if *yc != code + 10 {
                return Err(format!("Missing required y-coordinate near line: {}.", 2 * i));
            }
            let Some((zc, zv)) = tags.get(i) else { break };
            let has_z = *zc == code + 20;
            let line = 2 * (i + 1);
            let mut vals = vec![v.as_slice(), yv.as_slice()];
            if has_z {
                vals.push(zv.as_slice());
                i += 1;
            }
            let strict: Option<Vec<f64>> = vals.iter().map(|b| strict_float(b)).collect();
            let nums = match strict {
                Some(n) => n,
                None => {
                    let rec: Option<Vec<f64>> = vals.iter().map(|b| parse_float(b)).collect();
                    match rec {
                        Some(n) => {
                            errors += vals.len();
                            n
                        }
                        None => return Err(format!("Invalid floating point values near line: {line}.")),
                    }
                }
            };
            out.push(Tag { code, val: Val::P(v3(nums[0], nums[1], if has_z { nums[2] } else { 0.0 })) });
            continue;
        }
        let line = 2 * i;
        let bad = || format!("Invalid tag ({code}, \"{}\") near line: {line}.", decode(v, enc));
        let val = match code_type(code) {
            Kind::Binary => Val::S(String::new()),
            Kind::Int => match strict_int(v) {
                Some(n) => Val::I(n),
                None => {
                    let n = parse_int(v).ok_or_else(bad)?;
                    errors += 1;
                    Val::I(n)
                }
            },
            Kind::Float => match strict_float(v) {
                Some(n) => Val::F(n),
                None => {
                    let n = parse_float(v).ok_or_else(bad)?;
                    errors += 1;
                    Val::F(n)
                }
            },
            Kind::Str => {
                let raw = if code == 0 { trim_ascii(v) } else { v.as_slice() };
                if !decode_strict_ok(raw, enc) {
                    errors += 1;
                }
                if code == 0 {
                    Val::S(decode(raw, enc).to_uppercase())
                } else {
                    Val::S(decode_special(decode(raw, enc)))
                }
            }
        };
        out.push(Tag { code, val });
    }
    Ok((out, errors))
}

pub fn read_tags(data: &[u8]) -> Result<(Vec<Tag>, usize), String> {
    let raw = raw_tags(data)?;
    let enc = detect_encoding(&raw);
    compile(filter_point_codes(raw), enc)
}

#[derive(Debug, Clone, Default)]
pub struct Common {
    pub layer: Option<String>,
    pub linetype: Option<String>,
    pub lineweight: Option<i64>,
    pub color: Option<i64>,
    pub ltscale: Option<f64>,
    pub invisible: Option<i64>,
    pub paperspace: Option<i64>,
    pub owner: Option<String>,
    pub extrusion: Option<V3>,
    pub thickness: Option<f64>,
}

impl Common {
    pub fn graphic(&self) -> Common {
        Common {
            layer: self.layer.clone(),
            linetype: self.linetype.clone(),
            lineweight: self.lineweight,
            color: self.color,
            ltscale: self.ltscale,
            ..Default::default()
        }
    }

    pub fn extrusion(&self) -> V3 {
        self.extrusion.unwrap_or(Z_AXIS)
    }
}

#[derive(Debug, Clone)]
pub struct Vertex {
    pub location: V3,
    pub start_width: Option<f64>,
    pub end_width: Option<f64>,
    pub bulge: Option<f64>,
    pub flags: i64,
}

#[derive(Debug, Clone)]
pub enum Edge {
    Line {
        start: V2,
        end: V2,
    },
    Arc {
        center: V2,
        radius: f64,
        start: f64,
        end: f64,
    },
    Ellipse {
        center: V2,
        major: V2,
        ratio: f64,
        start: f64,
        end: f64,
    },
    Spline {
        degree: i64,
        knots: Vec<f64>,
        weights: Vec<f64>,
        cps: Vec<V2>,
        fits: Vec<V2>,
        st: Option<V2>,
        et: Option<V2>,
    },
}

#[derive(Debug, Clone)]
pub enum BPath {
    Poly { vertices: Vec<(f64, f64, f64)>, closed: bool },
    Edges(Vec<Edge>),
}

#[derive(Debug, Clone)]
pub enum Body {
    Line {
        start: V3,
        end: V3,
    },
    Circle {
        center: V3,
        radius: f64,
    },
    Arc {
        center: V3,
        radius: f64,
        start: f64,
        end: f64,
    },
    Ellipse {
        center: V3,
        major: V3,
        ratio: f64,
        start: f64,
        end: f64,
    },
    LwPolyline {
        pts: Vec<[f64; 5]>,
        flags: i64,
        const_width: Option<f64>,
        elevation: Option<f64>,
    },
    Polyline {
        flags: i64,
        elevation: Option<V3>,
        vertices: Vec<Vertex>,
    },
    Spline {
        degree: i64,
        knot_tol: f64,
        cps: Vec<V3>,
        fits: Vec<V3>,
        knots: Vec<f64>,
        weights: Vec<f64>,
        st: Option<V3>,
        et: Option<V3>,
    },
    Insert {
        name: String,
        insert: Option<V3>,
        sx: f64,
        sy: f64,
        sz: f64,
        rotation: f64,
    },
    Dimension {
        geometry: Option<String>,
        text_midpoint: Option<V3>,
        insert: Option<V3>,
        content: Option<Vec<Entity>>,
    },
    Leader {
        vertices: Vec<V3>,
        dimstyle: String,
        path_type: i64,
        annotation_type: i64,
        has_hookline: i64,
        hookline_direction: i64,
        has_arrowhead: i64,
        text_width: f64,
        horizontal_direction: V3,
    },
    Text {
        text: String,
        insert: Option<V3>,
        align_point: Option<V3>,
        rotation: f64,
        oblique: f64,
        width: f64,
        height: f64,
    },
    MText {
        text: String,
        insert: Option<V3>,
        text_direction: Option<V3>,
        rotation: Option<f64>,
        char_height: f64,
        width: Option<f64>,
    },
    Solid {
        vtx: [Option<V3>; 4],
    },
    Hatch {
        solid_fill: i64,
        elevation: V3,
        paths: Vec<BPath>,
    },
    Image {
        insert: Option<V3>,
    },
    Point {
        location: V3,
    },
    Other,
}

#[derive(Debug, Clone)]
pub struct Entity {
    pub dxftype: String,
    pub c: Common,
    pub body: Body,
}

#[derive(Debug, Clone)]
pub struct Layer {
    pub name: String,
    pub flags: i64,
    pub color: i64,
    pub linetype: Option<String>,
    pub lineweight: Option<i64>,
}

impl Layer {
    pub fn is_off(&self) -> bool {
        self.color < 0
    }

    pub fn is_frozen(&self) -> bool {
        self.flags & 1 != 0
    }
}

#[derive(Debug, Clone, Default)]
pub struct Linetype {
    pub total: f64,
    pub elements: Vec<f64>,
    pub complex: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DimStyle {
    pub vars: HashMap<i32, Val>,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub name: String,
    pub base_point: V3,
    pub entities: Vec<Entity>,
}

#[derive(Debug, Default)]
pub struct Doc {
    pub errors: usize,
    pub version: String,
    pub header: HashMap<String, Val>,
    pub layers: IndexMap<String, Layer>,
    pub linetypes: IndexMap<String, Linetype>,
    pub dimstyles: IndexMap<String, DimStyle>,
    pub blocks: HashMap<String, Block>,
    pub msp: Vec<Entity>,
    pub layouts: Vec<(String, Vec<Entity>)>,
}

pub fn key(name: &str) -> String {
    name.to_lowercase()
}

impl Doc {
    pub fn block(&self, name: &str) -> Option<&Block> {
        self.blocks.get(&key(name))
    }

    pub fn layer(&self, name: &str) -> Option<&Layer> {
        self.layers.get(&key(name))
    }

    pub fn header_f(&self, name: &str) -> Option<f64> {
        self.header.get(name).map(|v| match v {
            Val::F(f) => *f,
            Val::I(i) => *i as f64,
            _ => 0.0,
        })
    }
}

struct Split<'a> {
    common: Vec<&'a Tag>,
    spec: Vec<&'a Tag>,
    segs: Vec<(String, Vec<&'a Tag>)>,
}

fn split_entity(tags: &[Tag]) -> Split<'_> {
    let mut clean: Vec<&Tag> = Vec::new();
    let mut in_app = false;
    for t in &tags[1..] {
        if t.code == 101 || t.code == 1001 {
            break;
        }
        if t.code == 102 {
            in_app = t.s().starts_with('{');
            continue;
        }
        if in_app {
            continue;
        }
        clean.push(t);
    }
    let mut segs: Vec<(String, Vec<&Tag>)> = vec![(String::new(), Vec::new())];
    for t in clean {
        if t.code == 100 {
            segs.push((t.s().to_string(), Vec::new()));
        } else {
            segs.last_mut().expect("seg").1.push(t);
        }
    }
    let mut common = Vec::new();
    let mut spec = Vec::new();
    if segs.len() == 1 {
        common = segs[0].1.clone();
        spec = segs[0].1.clone();
    } else {
        for (i, (name, ts)) in segs.iter().enumerate() {
            if i == 0 || name == "AcDbEntity" {
                common.extend(ts.iter().copied());
            }
            if i == 0 {
                spec.extend(ts.iter().copied().filter(|t| t.code != 330 && t.code != 5));
            } else if name != "AcDbEntity" {
                spec.extend(ts.iter().copied());
            }
        }
    }
    Split { common, spec, segs }
}

fn last<'a>(ts: &[&'a Tag], code: i32) -> Option<&'a Tag> {
    ts.iter().rev().find(|t| t.code == code).copied()
}

fn lf(ts: &[&Tag], code: i32) -> Option<f64> {
    last(ts, code).map(|t| t.f())
}

fn li(ts: &[&Tag], code: i32) -> Option<i64> {
    last(ts, code).map(|t| t.i())
}

fn lp(ts: &[&Tag], code: i32) -> Option<V3> {
    last(ts, code).map(|t| t.p())
}

fn ls(ts: &[&Tag], code: i32) -> Option<String> {
    last(ts, code).map(|t| t.s().to_string())
}

const PATH_CODES: [i32; 18] = [10, 11, 12, 13, 40, 42, 50, 51, 72, 73, 74, 92, 93, 94, 95, 96, 97, 330];

fn group_by<'a>(tags: &[&'a Tag], split: i32) -> Vec<Vec<&'a Tag>> {
    let mut out: Vec<Vec<&Tag>> = Vec::new();
    for t in tags {
        if t.code == split {
            out.push(vec![*t]);
        } else if let Some(g) = out.last_mut() {
            g.push(*t);
        }
    }
    out
}

fn pop_source_objects(tags: &mut Vec<&Tag>) {
    let mut n = tags.len();
    while n > 0 && (tags[n - 1].code == 97 || tags[n - 1].code == 330) {
        if tags[n - 1].code == 97 {
            tags.truncate(n - 1);
            return;
        }
        n -= 1;
    }
}

fn load_edge(kind: i64, tags: &[&Tag]) -> Edge {
    let v2 = |t: &Tag| V2::of(t.p());
    match kind {
        1 => {
            let (mut start, mut end) = (V2 { x: 0.0, y: 0.0 }, V2 { x: 0.0, y: 0.0 });
            for t in tags {
                match t.code {
                    10 => start = v2(t),
                    11 => end = v2(t),
                    _ => {}
                }
            }
            Edge::Line { start, end }
        }
        2 | 3 => {
            let mut center = V2 { x: 0.0, y: 0.0 };
            let mut major = V2 { x: 1.0, y: 0.0 };
            let (mut radius, mut ratio, mut s, mut e, mut ccw) = (1.0, 1.0, 0.0, 0.0, true);
            for t in tags {
                match t.code {
                    10 => center = v2(t),
                    11 if kind == 3 => major = v2(t),
                    40 if kind == 2 => radius = t.f(),
                    40 => ratio = t.f(),
                    50 => s = t.f(),
                    51 => e = t.f(),
                    73 => ccw = t.i() != 0,
                    _ => {}
                }
            }
            let (s, e) = if ccw { (s, e) } else { (360.0 - e, 360.0 - s) };
            if kind == 2 {
                Edge::Arc { center, radius, start: s, end: e }
            } else {
                Edge::Ellipse { center, major, ratio, start: s, end: e }
            }
        }
        _ => {
            let (mut degree, mut knots, mut weights, mut cps, mut fits, mut st, mut et) =
                (3, Vec::new(), Vec::new(), Vec::new(), Vec::new(), None, None);
            for t in tags {
                match t.code {
                    94 => degree = t.i(),
                    40 => knots.push(t.f()),
                    42 => weights.push(t.f()),
                    10 => cps.push(v2(t)),
                    11 => fits.push(v2(t)),
                    12 => st = Some(v2(t)),
                    13 => et = Some(v2(t)),
                    _ => {}
                }
            }
            Edge::Spline { degree, knots, weights, cps, fits, st, et }
        }
    }
}

fn load_paths(tags: &[&Tag]) -> Vec<BPath> {
    let mut out = Vec::new();
    for mut g in group_by(tags, 92) {
        let flags = g[0].i();
        pop_source_objects(&mut g);
        if flags & 2 != 0 {
            let mut vertices: Vec<(f64, f64, f64)> = Vec::new();
            let mut closed = false;
            for t in &g {
                match t.code {
                    10 => {
                        let p = t.p();
                        vertices.push((p.x, p.y, 0.0));
                    }
                    42 => {
                        if let Some(v) = vertices.last_mut() {
                            v.2 = t.f();
                        }
                    }
                    73 => closed = t.i() != 0,
                    _ => {}
                }
            }
            out.push(BPath::Poly { vertices, closed });
        } else {
            let edges = group_by(&g, 72)
                .into_iter()
                .filter_map(|et| {
                    let k = et[0].i();
                    (0 < k && k < 5).then(|| load_edge(k, &et[1..]))
                })
                .collect();
            out.push(BPath::Edges(edges));
        }
    }
    out
}

fn load_hatch(sp: &Split) -> Body {
    let tags: Vec<&Tag> =
        sp.segs.iter().find(|(n, _)| n == "AcDbHatch").map(|(_, t)| t.clone()).unwrap_or_else(|| sp.spec.clone());
    let Some(start) = tags.iter().position(|t| t.code == 91) else {
        return Body::Other;
    };
    let n = tags[start + 1..].iter().take_while(|t| PATH_CODES.contains(&t.code)).count();
    let paths = load_paths(&tags[start + 1..start + 1 + n]);
    let mut rest: Vec<&Tag> = tags[..start].to_vec();
    rest.extend_from_slice(&tags[start + 1 + n..]);
    let mut cut = rest.len();
    for (i, t) in rest.iter().enumerate() {
        if t.code == 98 || t.code == 450 || t.code == 78 {
            cut = cut.min(i);
        }
    }
    let rest = &rest[..cut];
    Body::Hatch { solid_fill: li(rest, 70).unwrap_or(1), elevation: lp(rest, 10).unwrap_or(NULLVEC), paths }
}

fn parse_entity(tags: &[Tag]) -> Entity {
    let dxftype = tags[0].s().to_string();
    let sp = split_entity(tags);
    let c = &sp.common;
    let s = &sp.spec;
    let common = Common {
        layer: ls(c, 8),
        linetype: ls(c, 6),
        lineweight: li(c, 370),
        color: li(c, 62),
        ltscale: lf(c, 48),
        invisible: li(c, 60),
        paperspace: li(c, 67),
        owner: ls(c, 330),
        extrusion: lp(s, 210),
        thickness: lf(s, 39),
    };
    let body = match dxftype.as_str() {
        "LINE" => Body::Line { start: lp(s, 10).unwrap_or(NULLVEC), end: lp(s, 11).unwrap_or(NULLVEC) },
        "CIRCLE" => Body::Circle { center: lp(s, 10).unwrap_or(NULLVEC), radius: lf(s, 40).unwrap_or(1.0) },
        "ARC" => Body::Arc {
            center: lp(s, 10).unwrap_or(NULLVEC),
            radius: lf(s, 40).unwrap_or(1.0),
            start: lf(s, 50).unwrap_or(0.0),
            end: lf(s, 51).unwrap_or(360.0),
        },
        "ELLIPSE" => Body::Ellipse {
            center: lp(s, 10).unwrap_or(NULLVEC),
            major: lp(s, 11).unwrap_or(X_AXIS),
            ratio: lf(s, 40).unwrap_or(1.0),
            start: lf(s, 41).unwrap_or(0.0),
            end: lf(s, 42).unwrap_or(TAU),
        },
        "LWPOLYLINE" => {
            let mut pts: Vec<[f64; 5]> = Vec::new();
            let mut rest: Vec<&Tag> = Vec::new();
            for t in s.iter() {
                match t.code {
                    10 => {
                        let p = t.p();
                        pts.push([p.x, p.y, 0.0, 0.0, 0.0]);
                    }
                    40..=42 => {
                        if let Some(v) = pts.last_mut() {
                            v[(t.code - 38) as usize] = t.f();
                        }
                    }
                    _ => rest.push(t),
                }
            }
            Body::LwPolyline {
                pts,
                flags: li(&rest, 70).unwrap_or(0),
                const_width: lf(&rest, 43),
                elevation: lf(&rest, 38),
            }
        }
        "POLYLINE" => Body::Polyline { flags: li(s, 70).unwrap_or(0), elevation: lp(s, 10), vertices: Vec::new() },
        "VERTEX" => Body::Polyline {
            flags: li(s, 70).unwrap_or(0),
            elevation: None,
            vertices: vec![Vertex {
                location: lp(s, 10).unwrap_or(NULLVEC),
                start_width: lf(s, 40),
                end_width: lf(s, 41),
                bulge: lf(s, 42),
                flags: li(s, 70).unwrap_or(0),
            }],
        },
        "SPLINE" => {
            let (mut cps, mut fits, mut knots, mut weights) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let mut rest: Vec<&Tag> = Vec::new();
            for t in s.iter() {
                match t.code {
                    10 => cps.push(t.p()),
                    11 => fits.push(t.p()),
                    40 => knots.push(t.f()),
                    41 => weights.push(t.f()),
                    12 | 13 if NULLVEC.isclose(t.p()) => {}
                    _ => rest.push(t),
                }
            }
            Body::Spline {
                degree: li(&rest, 71).unwrap_or(3),
                knot_tol: lf(&rest, 42).unwrap_or(1e-10),
                cps,
                fits,
                knots,
                weights,
                st: lp(&rest, 12),
                et: lp(&rest, 13),
            }
        }
        "INSERT" => Body::Insert {
            name: ls(s, 2).unwrap_or_default(),
            insert: lp(s, 10),
            sx: lf(s, 41).unwrap_or(1.0),
            sy: lf(s, 42).unwrap_or(1.0),
            sz: lf(s, 43).unwrap_or(1.0),
            rotation: lf(s, 50).unwrap_or(0.0),
        },
        "DIMENSION" | "ARC_DIMENSION" | "LARGE_RADIAL_DIMENSION" => {
            Body::Dimension { geometry: ls(s, 2), text_midpoint: lp(s, 11), insert: lp(s, 12), content: None }
        }
        "LEADER" => Body::Leader {
            vertices: s.iter().filter(|t| t.code == 10).map(|t| t.p()).collect(),
            dimstyle: ls(s, 3).unwrap_or_else(|| "Standard".into()),
            path_type: li(s, 72).unwrap_or(0),
            annotation_type: li(s, 73).unwrap_or(3),
            has_hookline: li(s, 75).unwrap_or(1),
            hookline_direction: li(s, 74).unwrap_or(1),
            has_arrowhead: li(s, 71).unwrap_or(1),
            text_width: lf(s, 41).unwrap_or(1.0),
            horizontal_direction: lp(s, 211).unwrap_or(X_AXIS),
        },
        "TEXT" | "ATTRIB" | "ATTDEF" => Body::Text {
            text: ls(s, 1).unwrap_or_default(),
            insert: lp(s, 10),
            align_point: lp(s, 11),
            rotation: lf(s, 50).unwrap_or(0.0),
            oblique: lf(s, 51).unwrap_or(0.0),
            width: lf(s, 41).unwrap_or(1.0),
            height: lf(s, 40).unwrap_or(2.5),
        },
        "MTEXT" => {
            let mut parts = String::new();
            let mut tail = String::new();
            for t in s.iter() {
                match t.code {
                    1 => tail = t.s().to_string(),
                    3 => parts.push_str(t.s()),
                    _ => {}
                }
            }
            parts.push_str(&tail);
            Body::MText {
                text: parts.replace('\r', "").replace('\n', "\\P"),
                insert: lp(s, 10),
                text_direction: lp(s, 11),
                rotation: lf(s, 50),
                char_height: lf(s, 40).unwrap_or(2.5),
                width: lf(s, 41),
            }
        }
        "SOLID" | "TRACE" | "3DFACE" => Body::Solid { vtx: [lp(s, 10), lp(s, 11), lp(s, 12), lp(s, 13)] },
        "HATCH" => load_hatch(&sp),
        "IMAGE" => Body::Image { insert: lp(s, 10) },
        "POINT" => Body::Point { location: lp(s, 10).unwrap_or(NULLVEC) },
        _ => Body::Other,
    };
    Entity { dxftype, c: common, body }
}

fn link(raw: Vec<Vec<Tag>>) -> Vec<(Vec<Tag>, Entity)> {
    let mut out: Vec<(Vec<Tag>, Entity)> = Vec::new();
    let mut main: Option<(Vec<Tag>, Entity, &str)> = None;
    for tags in raw {
        let e = parse_entity(&tags);
        if let Some((mt, mut me, expected)) = main.take() {
            if e.dxftype == "SEQEND" {
                out.push((mt, me));
                continue;
            }
            if e.dxftype == expected {
                if let (Body::Polyline { vertices, .. }, Body::Polyline { vertices: v, .. }) = (&mut me.body, e.body) {
                    vertices.extend(v);
                }
                main = Some((mt, me, expected));
                continue;
            }
            out.push((mt, me));
        }
        match e.dxftype.as_str() {
            "POLYLINE" => main = Some((tags, e, "VERTEX")),
            "INSERT" if last(&split_entity(&tags).spec, 66).is_some_and(|t| t.i() != 0) => {
                main = Some((tags, e, "ATTRIB"))
            }
            _ => out.push((tags, e)),
        }
    }
    if let Some((mt, me, _)) = main {
        out.push((mt, me));
    }
    out
}

fn group0(tags: &[Tag]) -> Vec<Vec<Tag>> {
    let mut out: Vec<Vec<Tag>> = Vec::new();
    for t in tags {
        if t.code == 0 {
            out.push(vec![t.clone()]);
        } else if let Some(g) = out.last_mut() {
            g.push(t.clone());
        }
    }
    out
}

fn table_entries(groups: &[Vec<Tag>], kind: &str) -> Vec<Vec<Tag>> {
    groups.iter().filter(|g| g[0].s() == kind).cloned().collect()
}

pub fn load(data: &[u8]) -> Result<Doc, String> {
    let (tags, errors) = read_tags(data)?;
    let mut sections: IndexMap<String, Vec<Tag>> = IndexMap::new();
    let mut cur: Option<Vec<Tag>> = None;
    for t in tags {
        if t.code == 0 {
            match t.s() {
                "SECTION" => {
                    if let Some(s) = cur.take() {
                        push_section(&mut sections, s);
                    }
                    cur = Some(vec![t]);
                    continue;
                }
                "ENDSEC" | "EOF" => {
                    if let Some(s) = cur.take() {
                        push_section(&mut sections, s);
                    }
                    continue;
                }
                _ => {}
            }
        }
        if let Some(s) = cur.as_mut() {
            s.push(t);
        }
    }
    if let Some(s) = cur.take() {
        push_section(&mut sections, s);
    }
    let mut doc = Doc { errors, ..Default::default() };
    let header = sections.get("HEADER").cloned().unwrap_or_default();
    let mut i = 0;
    while i < header.len() {
        if header[i].code == 9 {
            let name = header[i].s().to_string();
            if let Some(v) = header.get(i + 1).filter(|t| t.code != 9) {
                doc.header.insert(name, v.val.clone());
            }
        }
        i += 1;
    }
    doc.version = match doc.header.get("$ACADVER") {
        Some(Val::S(v)) if Regex::new(r"^AC[0-9]{4}$").unwrap().is_match(v.trim()) => v.trim().to_string(),
        _ => "AC1009".into(),
    };
    let tables = group0(sections.get("TABLES").map_or(&[][..], |v| &v[..]));
    let mut block_records: HashMap<String, String> = HashMap::new();
    for g in table_entries(&tables, "LAYER") {
        let sp = split_entity(&g);
        let s = &sp.spec;
        let name = ls(s, 2).unwrap_or_default();
        doc.layers.insert(
            key(&name),
            Layer {
                name,
                flags: li(s, 70).unwrap_or(0),
                color: li(s, 62).unwrap_or(7),
                linetype: ls(s, 6),
                lineweight: li(s, 370),
            },
        );
    }
    for g in table_entries(&tables, "LTYPE") {
        let sp = split_entity(&g);
        let s = &sp.spec;
        let name = ls(s, 2).unwrap_or_default();
        let lt = Linetype {
            total: lf(s, 40).unwrap_or(0.0),
            elements: s.iter().filter(|t| t.code == 49).map(|t| t.f()).collect(),
            complex: s.iter().any(|t| t.code == 340),
        };
        doc.linetypes.insert(key(&name), lt);
    }
    for g in table_entries(&tables, "DIMSTYLE") {
        let sp = split_entity(&g);
        let name = ls(&sp.spec, 2).unwrap_or_default();
        let vars = sp.spec.iter().map(|t| (t.code, t.val.clone())).collect();
        doc.dimstyles.insert(key(&name), DimStyle { vars });
    }
    for g in table_entries(&tables, "BLOCK_RECORD") {
        let sp = split_entity(&g);
        if let (Some(h), Some(n)) =
            (ls(&sp.common, 5).or_else(|| g.iter().find(|t| t.code == 5).map(|t| t.s().into())), ls(&sp.spec, 2))
        {
            block_records.insert(h, n);
        }
    }
    let blocks_raw = group0(sections.get("BLOCKS").map_or(&[][..], |v| &v[..]));
    let mut current: Option<Block> = None;
    let mut body: Vec<Vec<Tag>> = Vec::new();
    for g in blocks_raw.into_iter().skip(1) {
        match g[0].s() {
            "BLOCK" => {
                let sp = split_entity(&g);
                let name = ls(&sp.spec, 2).or_else(|| ls(&sp.spec, 3)).unwrap_or_default();
                current = Some(Block { name, base_point: lp(&sp.spec, 10).unwrap_or(NULLVEC), entities: Vec::new() });
                body.clear();
            }
            "ENDBLK" => {
                if let Some(mut b) = current.take() {
                    b.entities = link(std::mem::take(&mut body)).into_iter().map(|(_, e)| e).collect();
                    doc.blocks.insert(key(&b.name), b);
                }
            }
            _ => {
                if current.is_some() {
                    body.push(g);
                }
            }
        }
    }
    let ents_raw: Vec<Vec<Tag>> =
        group0(sections.get("ENTITIES").map_or(&[][..], |v| &v[..])).into_iter().skip(1).collect();
    let msp_handle = block_records.iter().find(|(_, n)| n.eq_ignore_ascii_case("*Model_Space")).map(|(h, _)| h.clone());
    let psp_handle = block_records.iter().find(|(_, n)| n.eq_ignore_ascii_case("*Paper_Space")).map(|(h, _)| h.clone());
    let mut psp: Vec<Entity> = Vec::new();
    for (_, e) in link(ents_raw) {
        let owner = e.c.owner.clone();
        let paper = if owner.is_some() && owner == msp_handle {
            false
        } else if owner.is_some() && owner == psp_handle {
            true
        } else {
            e.c.paperspace.unwrap_or(0) != 0
        };
        if paper { psp.push(e) } else { doc.msp.push(e) }
    }
    if let Some(b) = doc.blocks.get(&key("*Model_Space")) {
        doc.msp.extend(b.entities.iter().cloned());
    }
    let objects = group0(sections.get("OBJECTS").map_or(&[][..], |v| &v[..]));
    let mut layouts: Vec<(i64, String, String)> = Vec::new();
    for g in table_entries(&objects, "LAYOUT") {
        let sp = split_entity(&g);
        let lay = sp.segs.iter().find(|(n, _)| n == "AcDbLayout").map(|(_, t)| t.clone()).unwrap_or_default();
        let name = ls(&lay, 1).unwrap_or_default();
        let br = ls(&lay, 330).unwrap_or_default();
        layouts.push((li(&lay, 71).unwrap_or(0), name, block_records.get(&br).cloned().unwrap_or_default()));
    }
    if layouts.is_empty() {
        layouts.push((1, "Layout1".into(), "*Paper_Space".into()));
    }
    for (_, name, brn) in layouts {
        if brn.eq_ignore_ascii_case("*Model_Space") {
            continue;
        }
        let mut ents = Vec::new();
        if brn.eq_ignore_ascii_case("*Paper_Space") {
            ents.append(&mut psp);
        }
        if let Some(b) = doc.blocks.get(&key(&brn)) {
            ents.extend(b.entities.iter().cloned());
        }
        doc.layouts.push((name, ents));
    }
    let names: std::collections::HashSet<String> = doc.blocks.keys().cloned().collect();
    let valid = |e: &Entity| !matches!(&e.body, Body::Insert { name, .. } if !names.contains(&key(name)));
    doc.msp.retain(valid);
    for (_, ents) in doc.layouts.iter_mut() {
        ents.retain(valid);
    }
    for b in doc.blocks.values_mut() {
        b.entities.retain(valid);
    }
    Ok(doc)
}

fn push_section(sections: &mut IndexMap<String, Vec<Tag>>, s: Vec<Tag>) {
    let Some(name) = s.get(1).filter(|t| t.code == 2).map(|t| t.s().to_string()) else { return };
    match sections.get_mut(&name) {
        Some(v) => v.extend(s.into_iter().skip(2)),
        None => {
            sections.insert(name, s);
        }
    }
}
