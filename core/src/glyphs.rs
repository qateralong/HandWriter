use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

use crate::geometry::Point;
use crate::numeric;
use crate::svgparse::{
    SKIP_TAGS, local_tag, mat_mul, num, parse_transform, parse_viewbox, path_d_to_strokes, walk_strokes,
};
use crate::xml;

#[derive(Debug, Clone, PartialEq)]
pub struct FontMetrics {
    pub x_height: f64,
    pub cap_height: f64,
    pub ascent: f64,
    pub descent: f64,
    pub x_height_source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    pub name: String,
    pub strokes: Vec<Vec<Point>>,
    pub advance: f64,
}

impl Glyph {
    pub fn bbox(&self) -> Option<(f64, f64, f64, f64)> {
        let mut it = self.strokes.iter().flatten();
        let &(x, y) = it.next()?;
        let (mut x0, mut y0, mut x1, mut y1) = (x, y, x, y);
        for &(x, y) in it {
            x0 = numeric::min(x0, x);
            y0 = numeric::min(y0, y);
            x1 = numeric::max(x1, x);
            y1 = numeric::max(y1, y);
        }
        Some((x0, y0, x1, y1))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShapedGlyph {
    pub name: String,
    pub cluster: usize,
    pub advance: f64,
    pub x_offset: f64,
    pub y_offset: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontInfo {
    pub name: String,
    pub mode: String,
    pub source: String,
    pub glyph_count: usize,
    pub chars: String,
    pub variants: IndexMap<String, Vec<String>>,
    pub metrics: Option<FontMetrics>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontError(pub String);

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FontError {}

impl From<crate::svgpath::PathError> for FontError {
    fn from(e: crate::svgpath::PathError) -> Self {
        FontError(e.0)
    }
}

pub trait GlyphProvider {
    fn mode(&self) -> &'static str;
    fn name(&self) -> &str;
    fn metrics(&self) -> &FontMetrics;
    fn glyph(&self, name: &str) -> Option<Glyph>;
    fn glyph_names_for_char(&self, ch: &str) -> Vec<String>;
    fn info(&self) -> FontInfo;

    fn has_char(&self, ch: &str) -> bool {
        !self.glyph_names_for_char(ch).is_empty()
    }

    fn glyph_names(&self) -> Vec<String> {
        let names: BTreeSet<String> =
            self.info().chars.chars().flat_map(|c| self.glyph_names_for_char(&c.to_string())).collect();
        names.into_iter().collect()
    }

    fn variants(&self) -> IndexMap<String, Vec<String>> {
        IndexMap::new()
    }

    fn advance(&self, name: &str) -> Option<f64> {
        self.glyph(name).map(|g| g.advance)
    }

    fn variant_pool(&self, ch: &str) -> Vec<String> {
        self.glyph_names_for_char(ch)
    }

    fn space_advance(&self) -> f64 {
        if let Some(first) = self.glyph_names_for_char(" ").first()
            && let Some(g) = self.glyph(first)
            && g.advance > 0.0
        {
            return g.advance;
        }
        0.3
    }

    fn shape(&self, text: &str) -> Vec<ShapedGlyph> {
        let mut out = Vec::new();
        for (i, ch) in text.chars().enumerate() {
            let names = self.glyph_names_for_char(&ch.to_string());
            let Some(g) = names.first().and_then(|n| self.glyph(n)) else { continue };
            out.push(ShapedGlyph { name: g.name, cluster: i, advance: g.advance, x_offset: 0.0, y_offset: 0.0 });
        }
        out
    }
}

const FLATTEN_TOL_EM: f64 = 0.0005;

static ALIASES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    HashMap::from([
        ("space", " "),
        ("exclam", "!"),
        ("quotedbl", "\""),
        ("numbersign", "#"),
        ("dollar", "$"),
        ("percent", "%"),
        ("ampersand", "&"),
        ("quotesingle", "'"),
        ("parenleft", "("),
        ("parenright", ")"),
        ("asterisk", "*"),
        ("plus", "+"),
        ("comma", ","),
        ("hyphen", "-"),
        ("period", "."),
        ("slash", "/"),
        ("colon", ":"),
        ("semicolon", ";"),
        ("less", "<"),
        ("equal", "="),
        ("greater", ">"),
        ("question", "?"),
        ("at", "@"),
        ("bracketleft", "["),
        ("backslash", "\\"),
        ("bracketright", "]"),
        ("underscore", "_"),
        ("braceleft", "{"),
        ("bar", "|"),
        ("braceright", "}"),
        ("asciitilde", "~"),
        ("numero", "№"),
        ("guillemotleft", "«"),
        ("guillemotright", "»"),
        ("emdash", "—"),
        ("endash", "–"),
        ("ellipsis", "…"),
        ("degree", "°"),
    ])
});

static UNI_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:uni|u\+|u)([0-9a-f]{4,6})$").expect("valid regex"));
static VARIANT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(.*)\.([0-9]+|alt[0-9]*)$").expect("valid regex"));

fn nfc(s: &str) -> String {
    s.nfc().collect()
}

pub fn char_from_stem(stem: &str) -> Option<String> {
    let stem = nfc(stem);
    if stem.chars().count() == 1 {
        return Some(stem);
    }
    if let Some(c) = UNI_RE.captures(&stem) {
        return u32::from_str_radix(&c[1], 16).ok().and_then(char::from_u32).map(|c| c.to_string());
    }
    ALIASES.get(stem.as_str()).map(|s| s.to_string())
}

fn default_glyph_name(ch: &str) -> String {
    format!("uni{:04X}", ch.chars().next().map_or(0, u32::from))
}

fn variant_key(n: &str) -> (u8, u64, String) {
    match VARIANT_RE.captures(n) {
        None => (0, 0, n.to_string()),
        Some(c) => {
            let v = &c[2];
            let k = if v.bytes().all(|b| b.is_ascii_digit()) { v.parse().unwrap_or(u64::MAX) } else { 999 };
            (1, k, n.to_string())
        }
    }
}

fn primary_first(names: &[String]) -> Vec<String> {
    let mut uniq: Vec<String> = Vec::new();
    for n in names {
        if !uniq.contains(n) {
            uniq.push(n.clone());
        }
    }
    uniq.sort_by_cached_key(|n| variant_key(n));
    uniq
}

fn freeze(strokes: Vec<Vec<Point>>) -> Vec<Vec<Point>> {
    strokes.into_iter().filter(|s| !s.is_empty()).collect()
}

fn top_of(glyphs: &IndexMap<String, Glyph>, cmap: &IndexMap<String, Vec<String>>, chars: &str) -> Option<(f64, char)> {
    for ch in chars.chars() {
        if let Some(names) = cmap.get(&ch.to_string())
            && let Some(first) = names.first()
            && let Some((_, _, _, top)) = glyphs[first].bbox()
            && top > 0.0
        {
            return Some((top, ch));
        }
    }
    None
}

fn truthy(v: Option<f64>) -> Option<f64> {
    v.filter(|&x| x != 0.0)
}

fn make_metrics(
    glyphs: &IndexMap<String, Glyph>,
    cmap: &IndexMap<String, Vec<String>>,
    x_height: Option<f64>,
    cap_height: Option<f64>,
    ascent: Option<f64>,
    descent: Option<f64>,
) -> FontMetrics {
    let mut src = "из файла шрифта".to_string();
    let x_height = match truthy(x_height) {
        Some(x) => x,
        None => match top_of(glyphs, cmap, "хxzvwuо") {
            Some((top, ch)) => {
                src = format!("по глифу «{ch}»");
                top
            }
            None => {
                src = "не найдена, принято 0.5 em".into();
                0.5
            }
        },
    };
    let cap_height = match truthy(cap_height) {
        Some(c) => c,
        None => top_of(glyphs, cmap, "НHXТIЕE").map_or(x_height * 1.4, |t| t.0),
    };
    let ys: Vec<f64> = glyphs.values().flat_map(|g| g.strokes.iter().flatten().map(|p| p.1)).collect();
    let ascent = ascent.unwrap_or_else(|| ys.iter().copied().reduce(numeric::max).unwrap_or(cap_height));
    let mut descent =
        descent.unwrap_or_else(|| ys.iter().copied().reduce(numeric::min).map_or(-0.25, |m| numeric::min(m, 0.0)));
    if descent > 0.0 {
        descent = -descent;
    }
    FontMetrics { x_height, cap_height, ascent, descent, x_height_source: src }
}

fn read_text(path: &Path) -> Result<String, FontError> {
    let bytes = fs::read(path).map_err(|e| FontError(format!("{}: {e}", path.display())))?;
    xml::decode(&bytes).map_err(|e| FontError(format!("{}: {e}", path.display())))
}

fn py_float(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn py_truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

#[derive(Debug, Clone)]
pub struct StrokeGlyphProvider {
    name: String,
    glyphs: IndexMap<String, Glyph>,
    cmap: IndexMap<String, Vec<String>>,
    metrics: FontMetrics,
    pub source: String,
    pub notes: Vec<String>,
}

impl GlyphProvider for StrokeGlyphProvider {
    fn mode(&self) -> &'static str {
        "strokes"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn metrics(&self) -> &FontMetrics {
        &self.metrics
    }

    fn glyph(&self, name: &str) -> Option<Glyph> {
        self.glyphs.get(name).cloned()
    }

    fn glyph_names_for_char(&self, ch: &str) -> Vec<String> {
        self.cmap.get(ch).cloned().unwrap_or_default()
    }

    fn variants(&self) -> IndexMap<String, Vec<String>> {
        self.cmap.iter().filter(|(_, v)| v.len() > 1).map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    fn info(&self) -> FontInfo {
        let mut chars: Vec<&String> = self.cmap.keys().collect();
        chars.sort();
        FontInfo {
            name: self.name.clone(),
            mode: self.mode().into(),
            source: self.source.clone(),
            glyph_count: self.glyphs.len(),
            chars: chars.into_iter().map(String::as_str).collect(),
            variants: self.variants(),
            metrics: Some(self.metrics.clone()),
            notes: self.notes.clone(),
        }
    }
}

impl StrokeGlyphProvider {
    pub fn glyph_order(&self) -> impl Iterator<Item = &str> {
        self.glyphs.keys().map(String::as_str)
    }

    pub fn cmap(&self) -> &IndexMap<String, Vec<String>> {
        &self.cmap
    }

    pub fn from_path(path: &Path) -> Result<Self, FontError> {
        if path.is_dir() {
            return Self::from_svg_folder(path);
        }
        if path.extension().is_some_and(|e| e.to_string_lossy().to_lowercase() == "svg") {
            return Self::from_svg_font(path);
        }
        Err(FontError(format!("Режим «Штрихи» принимает SVG-шрифт или папку с SVG: {}", path.display())))
    }

    pub fn from_svg_font(path: &Path) -> Result<Self, FontError> {
        let text = read_text(path)?;
        let doc = xml::parse(&text).map_err(|e| FontError(format!("{}: {e}", path.display())))?;
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(font) = doc.root_element().descendants().find(|n| n.is_element() && local_tag(*n) == "font") else {
            return Err(FontError(format!("В файле нет элемента <font>, это не SVG-шрифт: {file_name}")));
        };
        let face = font.descendants().find(|n| n.is_element() && local_tag(*n) == "font-face");
        let fa = |name: &str| face.and_then(|f| f.attribute(name));
        let mut upm = num(fa("units-per-em"), 1000.0);
        if upm == 0.0 {
            upm = 1000.0;
        }
        let default_adv = num(font.attribute("horiz-adv-x"), upm / 2.0);
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let name = [fa("font-family"), font.attribute("id")]
            .into_iter()
            .flatten()
            .find(|s| !s.is_empty())
            .map(String::from)
            .unwrap_or(stem);

        let s = 1.0 / upm;
        let scale = [s, 0.0, 0.0, s, 0.0, 0.0];
        let mut glyphs: IndexMap<String, Glyph> = IndexMap::new();
        let mut cmap: IndexMap<String, Vec<String>> = IndexMap::new();
        let mut unnamed: Vec<String> = Vec::new();
        let mut notes = Vec::new();
        for el in font.children().filter(|n| n.is_element()) {
            if local_tag(el) != "glyph" {
                continue;
            }
            let adv = num(el.attribute("horiz-adv-x"), default_adv) * s;
            let m = mat_mul(scale, parse_transform(el.attribute("transform")));
            let mut strokes = Vec::new();
            if let Some(d) = el.attribute("d").filter(|d| !d.is_empty()) {
                strokes.extend(path_d_to_strokes(d, m, FLATTEN_TOL_EM)?);
            }
            for child in el.children().filter(|n| n.is_element()) {
                strokes.extend(walk_strokes(child, m, FLATTEN_TOL_EM, SKIP_TAGS)?);
            }
            let uni = el.attribute("unicode").map(|u| if u.is_empty() { String::new() } else { nfc(u) });
            if let Some(u) = &uni
                && u.chars().count() != 1
            {
                notes.push(format!("Глиф для «{u}» (лигатура) пропущен: в режиме «Штрихи» лигатур нет"));
                continue;
            }
            let mut gname = match el.attribute("glyph-name").filter(|g| !g.is_empty()) {
                Some(g) => g.to_string(),
                None => {
                    let Some(u) = &uni else { continue };
                    let base = default_glyph_name(u);
                    if glyphs.contains_key(&base) {
                        format!("{base}.{}", cmap.get(u).map_or(0, Vec::len) + 1)
                    } else {
                        base
                    }
                }
            };
            while glyphs.contains_key(&gname) {
                gname.push('_');
            }
            glyphs.insert(gname.clone(), Glyph { name: gname.clone(), strokes: freeze(strokes), advance: adv });
            match uni {
                Some(u) => cmap.entry(u).or_default().push(gname),
                None => unnamed.push(gname),
            }
        }
        let mut by_name: HashMap<String, String> = HashMap::new();
        for (ch, names) in &cmap {
            for n in names {
                by_name.insert(n.clone(), ch.clone());
            }
        }
        for gname in unnamed {
            if let Some(c) = VARIANT_RE.captures(&gname)
                && let Some(ch) = by_name.get(&c[1])
            {
                cmap.get_mut(ch).expect("char from cmap").push(gname.clone());
            }
        }
        for names in cmap.values_mut() {
            *names = primary_first(names);
        }
        let em = |name: &str| fa(name).filter(|v| !v.is_empty()).map(|v| num(Some(v), 0.0) / upm);
        let metrics = make_metrics(&glyphs, &cmap, em("x-height"), em("cap-height"), em("ascent"), em("descent"));
        Ok(Self { name, glyphs, cmap, metrics, source: path.display().to_string(), notes })
    }

    pub fn from_svg_folder(folder: &Path) -> Result<Self, FontError> {
        let io_err = |e: std::io::Error| FontError(format!("{}: {e}", folder.display()));
        let cfg_path = folder.join("font.json");
        let cfg: serde_json::Map<String, Value> = if cfg_path.exists() {
            let text = fs::read_to_string(&cfg_path).map_err(io_err)?;
            match serde_json::from_str(&text).map_err(|e| FontError(format!("{}: {e}", cfg_path.display())))? {
                Value::Object(o) => o,
                _ => serde_json::Map::new(),
            }
        } else {
            serde_json::Map::new()
        };
        let mut files: Vec<(String, std::path::PathBuf)> = fs::read_dir(folder)
            .map_err(io_err)?
            .filter_map(|e| e.ok())
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .filter(|(n, _)| if cfg!(windows) { n.to_lowercase().ends_with(".svg") } else { n.ends_with(".svg") })
            .collect();
        files.sort();
        if files.is_empty() {
            return Err(FontError(format!("В папке нет SVG-файлов: {}", folder.display())));
        }
        let upm_cfg = cfg.get("units_per_em").filter(|v| py_truthy(Some(v))).and_then(py_float);
        let base_cfg = cfg.get("baseline").filter(|v| !v.is_null());
        let mut glyphs: IndexMap<String, Glyph> = IndexMap::new();
        let mut cmap: IndexMap<String, Vec<String>> = IndexMap::new();
        let mut notes = Vec::new();
        let mut upm_seen: Option<f64> = None;
        for (fname, path) in &files {
            let mut stem = fname[..fname.len() - 4].to_string();
            let mut variant = 1u64;
            if let Some(c) = VARIANT_RE.captures(&stem)
                && char_from_stem(&c[1]).is_some()
                && char_from_stem(&stem).is_none()
            {
                let v = c[2].to_string();
                stem = c[1].to_string();
                variant = if v.bytes().all(|b| b.is_ascii_digit()) { v.parse().unwrap_or(u64::MAX) } else { 2 };
            }
            let Some(ch) = char_from_stem(&stem) else {
                notes.push(format!("Файл {fname}: не понял, какой это символ, пропущен"));
                continue;
            };
            let text = read_text(path)?;
            let doc = xml::parse(&text).map_err(|e| FontError(format!("{}: {e}", path.display())))?;
            let root = doc.root_element();
            let Some((vx, vy, vw, vh)) = parse_viewbox(root) else {
                notes.push(format!("Файл {fname}: нет viewBox/width/height, пропущен"));
                continue;
            };
            let upm = upm_cfg.unwrap_or(vh);
            if upm_seen.is_none_or(|u| u == 0.0) {
                upm_seen = Some(upm);
            }
            let baseline = match base_cfg {
                Some(b) => py_float(b).ok_or_else(|| FontError(format!("font.json: baseline {b}")))?,
                None => vy + vh * 0.8,
            };
            let s = 1.0 / upm;
            let m = [s, 0.0, 0.0, -s, -vx * s, baseline * s];
            let strokes = walk_strokes(root, m, FLATTEN_TOL_EM, SKIP_TAGS)?;
            let adv_attr = [root.attribute("data-advance"), root.attribute("horiz-adv-x")]
                .into_iter()
                .flatten()
                .find(|a| !a.is_empty());
            let adv = adv_attr.map_or(vw, |a| num(Some(a), 0.0)) * s;
            let mut gname = default_glyph_name(&ch) + &if variant > 1 { format!(".{variant}") } else { String::new() };
            while glyphs.contains_key(&gname) {
                gname.push('_');
            }
            glyphs.insert(gname.clone(), Glyph { name: gname.clone(), strokes: freeze(strokes), advance: adv });
            cmap.entry(ch).or_default().push(gname);
        }
        for names in cmap.values_mut() {
            *names = primary_first(names);
        }
        let u = upm_cfg.or(upm_seen.filter(|&u| u != 0.0)).unwrap_or(1.0);
        let cfg_em = |key: &str| match cfg.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => py_float(v).map(|x| Some(x / u)).ok_or_else(|| FontError(format!("font.json: {key} {v}"))),
        };
        let metrics = make_metrics(
            &glyphs,
            &cmap,
            cfg_em("x_height")?,
            cfg_em("cap_height")?,
            cfg_em("ascent")?,
            cfg_em("descent")?,
        );
        if base_cfg.is_none() {
            notes.push("Нет font.json: базовая линия принята на 80% высоты viewBox сверху".into());
        }
        let name = match cfg.get("name") {
            Some(Value::String(s)) if !s.is_empty() => s.clone(),
            _ => folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        };
        Ok(Self { name, glyphs, cmap, metrics, source: folder.display().to_string(), notes })
    }
}
