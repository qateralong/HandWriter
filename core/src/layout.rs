use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::geometry::Point;
use crate::glyphs::{GlyphProvider, ShapedGlyph};
use crate::numeric;
use crate::rand::{pick, urnd};
use crate::settings::{Randomness, Settings};
use crate::text::{ProcessedText, Word, break_positions};
use crate::writing::build_paths;

const EPS: f64 = 1e-6;
const PLACEHOLDER_EM: f64 = 0.5;

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedGlyph {
    pub word: i64,
    pub letter: i64,
    pub line: i64,
    pub ch: char,
    pub glyph: Option<String>,
    pub x: f64,
    pub y: f64,
    pub advance: f64,
    pub missing: bool,
    pub hyphen: bool,
    pub segment: i64,
    pub adv_em: f64,
    pub dx_em: f64,
    pub dy_em: f64,
    pub size: f64,
    pub slant: f64,
    pub voff: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrawnStroke {
    pub points: Vec<Point>,
    pub word: i64,
    pub letter: i64,
    pub line: i64,
    pub hyphen: bool,
    pub tags: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutResult {
    pub strokes: Vec<DrawnStroke>,
    pub glyphs: Vec<PlacedGlyph>,
    pub baselines: Vec<f64>,
    pub used_lines: i64,
    pub scale: f64,
    pub first_word: Option<i64>,
    pub last_word: Option<i64>,
    pub last_letter: Option<i64>,
    pub next_word: Option<i64>,
    pub next_letter: Option<i64>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

pub fn line_baselines(s: &Settings) -> Vec<f64> {
    let sh = &s.sheet;
    let mut out = Vec::new();
    let mut y = sh.height - sh.first_line_top;
    if sh.line_pitch <= 0.0 {
        return if y >= sh.bottom_limit - EPS { vec![y] } else { out };
    }
    while y >= sh.bottom_limit - EPS {
        out.push(y);
        y -= sh.line_pitch;
    }
    out
}

pub struct RandomSource<'a> {
    r: Randomness,
    on: bool,
    seed: i64,
    prov: &'a dyn GlyphProvider,
    variants: RefCell<HashMap<i64, BTreeMap<i64, String>>>,
    size_mean: f64,
}

impl<'a> RandomSource<'a> {
    pub fn new(s: &Settings, pt: &ProcessedText, prov: &'a dyn GlyphProvider) -> Self {
        let r = s.randomness.clone();
        let mut size_mean = 0.0;
        if r.enabled && r.size > 0.0 {
            let us: Vec<f64> = pt
                .words
                .iter()
                .flat_map(|w| (0..w.len()).map(move |i| urnd(r.seed, "size", &[w.index, i as i64 + 1])))
                .collect();
            if !us.is_empty() {
                size_mean = numeric::sum(us.iter().copied()) / us.len() as f64;
            }
        }
        Self { on: r.enabled, seed: r.seed, r, prov, variants: RefCell::new(HashMap::new()), size_mean }
    }

    pub fn size(&self, w: i64, l: i64) -> f64 {
        if !self.on || self.r.size <= 0.0 {
            return 1.0;
        }
        1.0 + self.r.size / 100.0 * (urnd(self.seed, "size", &[w, l]) - self.size_mean)
    }

    pub fn slant(&self, w: i64, l: i64) -> f64 {
        if self.on { self.r.slant * urnd(self.seed, "slant", &[w, l]) } else { 0.0 }
    }

    pub fn voff(&self, w: i64, l: i64) -> f64 {
        if self.on { self.r.offset * urnd(self.seed, "voff", &[w, l]) } else { 0.0 }
    }

    pub fn letter_spacing(&self, w: i64, l: i64) -> f64 {
        if self.on { 1.0 + self.r.letter_spacing / 100.0 * urnd(self.seed, "ls", &[w, l]) } else { 1.0 }
    }

    pub fn word_spacing(&self, w: i64) -> f64 {
        if self.on { 1.0 + self.r.word_spacing / 100.0 * urnd(self.seed, "ws", &[w]) } else { 1.0 }
    }

    pub fn line_start(&self, line: i64) -> f64 {
        if self.on { self.r.line_start * urnd(self.seed, "lstart", &[line]) } else { 0.0 }
    }

    pub fn right_edge(&self, line: i64) -> f64 {
        if self.on { self.r.right_edge * urnd(self.seed, "redge", &[line]) } else { 0.0 }
    }

    pub fn variants(&self, word: &Word) -> BTreeMap<i64, String> {
        if !(self.on && self.r.variants) {
            return BTreeMap::new();
        }
        if let Some(got) = self.variants.borrow().get(&word.index) {
            return got.clone();
        }
        let mut got = BTreeMap::new();
        let mut last: HashMap<char, i64> = HashMap::new();
        for (i, &ch) in word.chars.iter().enumerate() {
            let pool = self.prov.variant_pool(&ch.to_string());
            let n = pool.len() as i64;
            if n < 2 {
                continue;
            }
            let keys = [word.index, i as i64 + 1];
            let mut k = pick(self.seed, "variant", n, &keys);
            if last.get(&ch) == Some(&k) {
                k = (k + 1 + pick(self.seed, "variant2", n - 1, &keys)) % n;
            }
            last.insert(ch, k);
            got.insert(i as i64 + 1, pool[k as usize].clone());
        }
        self.variants.borrow_mut().insert(word.index, got.clone());
        got
    }
}

#[derive(Debug, Clone)]
struct Run {
    i: usize,
    name: Option<String>,
    adv_em: f64,
    dx_em: f64,
    dy_em: f64,
    width: f64,
}

struct Measure<'a> {
    prov: &'a dyn GlyphProvider,
    scale: f64,
    skipped: &'a BTreeSet<char>,
    rs: &'a RandomSource<'a>,
    cache: RefCell<HashMap<(i64, usize, usize), Vec<Run>>>,
    hyphen: Option<ShapedGlyph>,
    hyphen_w: f64,
}

impl<'a> Measure<'a> {
    fn new(prov: &'a dyn GlyphProvider, scale: f64, skipped: &'a BTreeSet<char>, rs: &'a RandomSource<'a>) -> Self {
        let hyphen = prov.shape("-").into_iter().next();
        let hyphen_w = hyphen.as_ref().map_or(0.0, |h| h.advance * scale);
        Self { prov, scale, skipped, rs, cache: RefCell::new(HashMap::new()), hyphen, hyphen_w }
    }

    fn runs(&self, word: &Word, a: usize, b: usize) -> Vec<Run> {
        let key = (word.index, a, b);
        if let Some(got) = self.cache.borrow().get(&key) {
            return got.clone();
        }
        let text = word.slice(a, b);
        let n = b - a;
        let chosen = self.rs.variants(word);
        let mut by_cluster: BTreeMap<usize, Vec<ShapedGlyph>> = BTreeMap::new();
        for g in self.prov.shape(&text) {
            by_cluster.entry(g.cluster).or_default().push(g);
        }
        let clusters: Vec<usize> = by_cluster.keys().copied().collect();
        let mut covered = BTreeSet::new();
        for (k, &c) in clusters.iter().enumerate() {
            let end = clusters.get(k + 1).copied().unwrap_or(n);
            covered.extend(c..end);
        }
        let mut out = Vec::new();
        for i in 0..n {
            let ch = word.chars[a + i];
            let letter = (a + i + 1) as i64;
            if let Some(gs) = by_cluster.get(&i) {
                let f = self.rs.size(word.index, letter) * self.rs.letter_spacing(word.index, letter);
                for g in gs {
                    let (mut name, mut adv) = (g.name.clone(), g.advance);
                    if let Some(alt) = chosen.get(&letter)
                        && gs.len() == 1
                        && self.prov.variant_pool(&ch.to_string()).first() == Some(&name)
                        && *alt != name
                    {
                        name = alt.clone();
                        adv = self.prov.advance(alt).expect("variant glyph exists");
                    }
                    out.push(Run {
                        i: a + i,
                        name: Some(name),
                        adv_em: adv,
                        dx_em: g.x_offset,
                        dy_em: g.y_offset,
                        width: adv * self.scale * f,
                    });
                }
            } else if !covered.contains(&i) || !self.prov.has_char(&ch.to_string()) {
                let w = if self.skipped.contains(&ch) { 0.0 } else { PLACEHOLDER_EM * self.scale };
                out.push(Run { i: a + i, name: None, adv_em: 0.0, dx_em: 0.0, dy_em: 0.0, width: w });
            }
        }
        self.cache.borrow_mut().insert(key, out.clone());
        out
    }

    fn width(&self, word: &Word, a: usize, b: usize) -> f64 {
        numeric::sum(self.runs(word, a, b).iter().map(|r| r.width))
    }
}

struct State {
    line: i64,
    x: f64,
    empty: bool,
    segment: i64,
}

struct Ctx<'a> {
    s: &'a Settings,
    pt: &'a ProcessedText,
    rs: &'a RandomSource<'a>,
    meas: &'a Measure<'a>,
    bases: Vec<f64>,
    n_lines: i64,
    space_w: f64,
    x_left: f64,
    x_right: f64,
    st: State,
    res: LayoutResult,
}

impl Ctx<'_> {
    fn right(&self, line: i64) -> f64 {
        self.x_right + self.rs.right_edge(line)
    }

    fn gap(&self, word: &Word) -> f64 {
        if self.st.empty { 0.0 } else { self.space_w * self.rs.word_spacing(word.index) }
    }

    fn place_segment(&mut self, word: &Word, a: usize, b: usize, with_hyphen: bool) {
        let line = self.st.line;
        let base = self.bases[(line - 1) as usize] + self.s.typography.baseline_shift;
        let mut x = self.st.x + self.gap(word);
        let seg = self.st.segment;
        self.st.segment += 1;
        for r in self.meas.runs(word, a, b) {
            let ch = word.chars[r.i];
            let letter = r.i as i64 + 1;
            self.res.glyphs.push(PlacedGlyph {
                word: word.index,
                letter,
                line,
                ch,
                missing: r.name.is_none() && !self.pt.skipped.contains(&ch),
                glyph: r.name,
                x,
                y: base,
                advance: r.width,
                hyphen: false,
                segment: seg,
                adv_em: r.adv_em,
                dx_em: r.dx_em,
                dy_em: r.dy_em,
                size: self.rs.size(word.index, letter),
                slant: self.rs.slant(word.index, letter),
                voff: self.rs.voff(word.index, letter),
            });
            x += r.width;
        }
        self.res.last_word = Some(word.index);
        self.res.last_letter = Some(b as i64);
        if with_hyphen && let Some(hg) = &self.meas.hyphen {
            self.res.glyphs.push(PlacedGlyph {
                word: word.index,
                letter: b as i64,
                line,
                ch: '-',
                glyph: Some(hg.name.clone()),
                x,
                y: base,
                advance: self.meas.hyphen_w,
                missing: false,
                hyphen: true,
                segment: seg,
                adv_em: hg.advance,
                dx_em: 0.0,
                dy_em: 0.0,
                size: 1.0,
                slant: 0.0,
                voff: 0.0,
            });
            x += self.meas.hyphen_w;
        }
        self.st.x = x;
        self.st.empty = false;
        self.res.used_lines = self.res.used_lines.max(line);
    }

    fn new_line(&mut self, skip: i64) -> bool {
        self.st.line += 1 + skip;
        self.st.x = self.x_left + self.rs.line_start(self.st.line);
        self.st.empty = true;
        self.st.line <= self.n_lines
    }

    fn overflow(&mut self, word: &Word, a: usize) {
        self.res.next_word = Some(word.index);
        self.res.next_letter = Some(a as i64 + 1);
    }
}

fn best_break(word: &Word, a: usize, breaks: &BTreeMap<usize, bool>, avail: f64, meas: &Measure) -> Option<usize> {
    let mut best = None;
    for (&b, &need_h) in breaks {
        if b <= a || b >= word.len() {
            continue;
        }
        let w = meas.width(word, a, b) + if need_h { meas.hyphen_w } else { 0.0 };
        if w <= avail + EPS {
            best = Some(b);
        }
    }
    best
}

fn forced_break(word: &Word, a: usize, avail: f64, meas: &Measure) -> Option<usize> {
    let mut best = None;
    for b in a + 1..word.len() {
        if meas.width(word, a, b) + meas.hyphen_w <= avail + EPS {
            best = Some(b);
        } else {
            break;
        }
    }
    best
}

pub fn layout(s: &Settings, prov: &dyn GlyphProvider, pt: &ProcessedText) -> LayoutResult {
    let mut res = LayoutResult::default();
    let (sh, ty) = (&s.sheet, &s.typography);
    let m = prov.metrics();
    if m.x_height <= 0.0 {
        res.errors.push("У шрифта нулевая высота строчной буквы".into());
        return res;
    }
    let scale = ty.size_mm / m.x_height;
    res.scale = scale;
    let bases = line_baselines(s);
    res.baselines = bases.iter().map(|b| b + ty.baseline_shift).collect();
    let n_lines = bases.len() as i64;
    if n_lines == 0 {
        res.errors.push("На листе не помещается ни одной строки: проверь первую строку и нижний предел".into());
        return res;
    }
    if pt.words.is_empty() {
        return res;
    }

    let rs = RandomSource::new(s, pt, prov);
    let meas = Measure::new(prov, scale, &pt.skipped, &rs);
    if meas.hyphen.is_none() {
        res.warnings.push("В шрифте нет дефиса «-»: переносы будут без дефиса".into());
    }
    let space_w = prov.space_advance() * scale;
    let x_left = sh.margin_left;
    let x_right = sh.width - sh.margin_right;

    let n_words = pt.words.len() as i64;
    let start_word = s.text_options.start_word;
    let start = 1.max(start_word.min(n_words));
    if start_word > n_words {
        res.warnings.push(format!("В тексте только {n_words} слов, начинаю со слова {start}"));
    }
    res.first_word = Some(start);

    let first_x = x_left + rs.line_start(1);
    let mut c = Ctx {
        s,
        pt,
        rs: &rs,
        meas: &meas,
        bases,
        n_lines,
        space_w,
        x_left,
        x_right,
        st: State { line: 1, x: first_x, empty: true, segment: 0 },
        res,
    };
    let mut placed_any = false;

    'paragraphs: for p in &pt.paragraphs {
        let words: Vec<&Word> = p.words.iter().map(|&i| &pt.words[i]).filter(|w| w.index >= start).collect();
        let Some(&first) = words.first() else { continue };
        let paragraph_start = first.index == pt.words[p.words[0]].index;
        if placed_any {
            if !c.new_line(if paragraph_start { p.blank_before } else { 0 }) {
                c.overflow(first, 0);
                break;
            }
        } else if paragraph_start && start == 1 && p.blank_before != 0 {
            c.new_line(p.blank_before - 1);
            if c.st.line > c.n_lines {
                c.overflow(first, 0);
                break;
            }
        }
        if paragraph_start && p.indent {
            c.st.x += sh.indent;
        }
        placed_any = true;
        for word in words {
            let mut a = 0;
            let breaks = break_positions(word, ty.hyphenate);
            loop {
                let avail = c.right(c.st.line) - c.st.x - c.gap(word);
                if meas.width(word, a, word.len()) <= avail + EPS {
                    c.place_segment(word, a, word.len(), false);
                    break;
                }
                let last_line = c.st.line >= c.n_lines;
                let b = if last_line { None } else { best_break(word, a, &breaks, avail, &meas) };
                if let Some(b) = b {
                    c.place_segment(word, a, b, breaks[&b]);
                    a = b;
                    if !c.new_line(0) {
                        c.overflow(word, a);
                        break 'paragraphs;
                    }
                    continue;
                }
                if !c.st.empty {
                    if !c.new_line(0) {
                        c.overflow(word, a);
                        break 'paragraphs;
                    }
                    continue;
                }
                let Some(b) = forced_break(word, a, avail, &meas) else {
                    c.res
                        .errors
                        .push(format!("Строка слишком узкая: не помещается даже одна буква слова {}", word.index));
                    c.overflow(word, a);
                    break 'paragraphs;
                };
                c.place_segment(word, a, b, true);
                c.res.warnings.push(format!("Слово {} длиннее строки, разбито принудительно", word.index));
                a = b;
                if !c.new_line(0) {
                    c.overflow(word, a);
                    break 'paragraphs;
                }
            }
        }
    }

    let mut res = c.res;
    res.strokes = build_paths(s, prov, &res.glyphs, scale);
    res
}
