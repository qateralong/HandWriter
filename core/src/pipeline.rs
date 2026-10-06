use std::collections::BTreeSet;

use crate::checks::{check_bounds, check_printer, check_settings, check_sheet};
use crate::gcode::{generate_gcode, test_pattern};
use crate::geometry::Point;
use crate::glyphs::GlyphProvider;
use crate::layout::{LayoutResult, layout};
use crate::numeric::repr;
use crate::settings::{Mode, Settings};
use crate::text::{ProcessedText, process_text};

#[derive(Debug, Clone, PartialEq)]
pub struct Resume {
    pub word: i64,
    pub letter: i64,
    pub path: usize,
    pub point: usize,
    pub connected: bool,
}

#[derive(Debug, Clone)]
pub struct Composition {
    pub settings: Settings,
    pub font_name: Option<String>,
    pub text: Option<ProcessedText>,
    pub layout: Option<LayoutResult>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub resume: Option<Resume>,
}

impl Composition {
    pub fn all_strokes(&self) -> Vec<Vec<Point>> {
        self.layout.as_ref().map(|l| l.strokes.iter().map(|s| s.points.clone()).collect()).unwrap_or_default()
    }

    pub fn strokes(&self) -> Vec<Vec<Point>> {
        let paths = self.all_strokes();
        match &self.resume {
            None => paths,
            Some(r) => {
                let mut out = vec![paths[r.path][r.point..].to_vec()];
                out.extend_from_slice(&paths[r.path + 1..]);
                out
            }
        }
    }
}

fn opt(v: Option<i64>) -> String {
    v.map_or_else(|| "None".to_string(), |x| x.to_string())
}

pub fn find_resume(lay: &LayoutResult, word: i64, letter: i64) -> Result<Resume, String> {
    let targets: BTreeSet<usize> = lay
        .glyphs
        .iter()
        .enumerate()
        .filter(|(_, g)| g.word == word && g.letter == letter && !g.hyphen)
        .map(|(i, _)| i)
        .collect();
    if targets.is_empty() {
        if !lay.glyphs.iter().any(|g| g.word == word) {
            return Err(format!(
                "Продолжить с: слова {word} нет на этом листе (здесь слова {}–{})",
                opt(lay.first_word),
                opt(lay.last_word)
            ));
        }
        return Err(format!("Продолжить с: в слове {word} нет буквы {letter}"));
    }
    for (pi, st) in lay.strokes.iter().enumerate() {
        for (k, t) in st.tags.iter().enumerate() {
            if targets.contains(t) {
                let connected = k > 0;
                return Ok(Resume { word, letter, path: pi, point: if connected { k - 1 } else { 0 }, connected });
            }
        }
    }
    Err(format!("Продолжить с: у буквы {letter} слова {word} нет штрихов (пропущенный символ?)"))
}

pub fn compose_without_font(s: &Settings, font_error: &str) -> Composition {
    let (mut errors, mut warnings) = check_sheet(s);
    let (pe, pw) = check_printer(s);
    errors.extend(pe);
    warnings.extend(pw);
    errors.push(format!("Шрифт не загрузился: {font_error}"));
    Composition { settings: s.clone(), font_name: None, text: None, layout: None, errors, warnings, resume: None }
}

pub fn compose(s: &Settings, prov: &dyn GlyphProvider) -> Composition {
    let (sheet_errors, mut warnings) = check_sheet(s);
    let (printer_errors, printer_warnings) = check_printer(s);
    let sheet_ok = sheet_errors.is_empty();
    let mut errors = sheet_errors;
    errors.extend(printer_errors);
    warnings.extend(printer_warnings);
    let has_char = |c: &str| prov.has_char(c);
    let pt = process_text(&s.text, &has_char, &s.text_options.replacements, &s.text_options.missing);
    if !pt.unresolved.is_empty() {
        let chars: Vec<String> = pt.unresolved.iter().map(|c| format!("«{c}»")).collect();
        errors.push(format!("Нет в шрифте: {}. Выбери для них «пропустить» или «заменить»", chars.join(" ")));
    }
    let mut c = Composition {
        settings: s.clone(),
        font_name: Some(prov.name().to_string()),
        text: None,
        layout: None,
        errors: Vec::new(),
        warnings: Vec::new(),
        resume: None,
    };
    if sheet_ok {
        let start = s.text_options.start_word;
        let mut names: Vec<String> = Vec::new();
        for w in pt.words.iter().filter(|w| w.index >= start) {
            names.extend(prov.shape(&w.text).into_iter().map(|g| g.name));
        }
        names.extend(prov.shape("-").into_iter().map(|g| g.name));
        if s.randomness.enabled && s.randomness.variants {
            for w in pt.words.iter().filter(|w| w.index >= start) {
                for ch in w.chars.iter() {
                    names.extend(prov.variant_pool(&ch.to_string()));
                }
            }
        }
        prov.prepare(&names);
        let lay = layout(s, prov, &pt);
        errors.extend(lay.errors.iter().cloned());
        warnings.extend(lay.warnings.iter().cloned());
        if let Some(next) = lay.next_word {
            let letter = match lay.next_letter {
                Some(l) if l > 1 => format!(", буквы {l}"),
                _ => String::new(),
            };
            warnings.push(format!("Текст не поместился: следующий лист начинать со слова {next}{letter}"));
        }
        if s.text_options.resume_word != 0 {
            match find_resume(&lay, s.text_options.resume_word, s.text_options.resume_letter) {
                Ok(r) => c.resume = Some(r),
                Err(e) => errors.push(e),
            }
        }
        c.layout = Some(lay);
        c.text = Some(pt);
        errors.extend(check_bounds(&c.strokes(), s, 5));
    } else {
        c.text = Some(pt);
    }
    c.errors = errors;
    c.warnings = warnings;
    c
}

pub fn make_gcode(c: &Composition) -> Result<String, Vec<String>> {
    if !c.errors.is_empty() {
        return Err(c.errors.clone());
    }
    let strokes = c.strokes();
    if strokes.is_empty() {
        return Err(vec!["Нечего писать: текст пустой".into()]);
    }
    let s = &c.settings;
    let lay = c.layout.as_ref().expect("layout exists when there are strokes");
    let r = &s.randomness;
    let rand = format!("seed: {}", r.seed)
        + &if r.enabled {
            format!(
                " (size {}% slant {} offset {} letters {}% words {}% drift {} line start {} right edge {} jitter {} variants {})",
                repr(r.size),
                repr(r.slant),
                repr(r.offset),
                repr(r.letter_spacing),
                repr(r.word_spacing),
                repr(r.drift),
                repr(r.line_start),
                repr(r.right_edge),
                repr(r.jitter),
                r.variants as i32
            )
        } else {
            " (randomness off)".into()
        };
    let mode = match s.mode {
        Mode::Strokes => "strokes",
        Mode::Outlines => "outlines",
    };
    let next = match lay.next_word {
        Some(w) if w != 0 => {
            let letter = if lay.next_letter.unwrap_or(1) > 1 {
                format!(", letter {}", opt(lay.next_letter))
            } else {
                String::new()
            };
            format!("word {w}{letter}")
        }
        _ => "text finished".into(),
    };
    let mut header = vec![
        format!("font: {} (mode {mode})", c.font_name.as_deref().unwrap_or("None")),
        rand,
        format!(
            "connections: {}",
            if s.connections.enabled { format!("on, distance {}", repr(s.connections.distance)) } else { "off".into() }
        ),
        format!("sheet start: word {}, letter 1", opt(lay.first_word)),
        format!(
            "words: {}..{} (last written: word {}, letter {})",
            opt(lay.first_word),
            opt(lay.last_word),
            opt(lay.last_word),
            opt(lay.last_letter)
        ),
        format!("next sheet: {next}"),
    ];
    if let Some(r) = &c.resume {
        header.push(
            format!("RESUME from word {}, letter {}", r.word, r.letter)
                + if r.connected { " (starts at the connection point)" } else { "" },
        );
    }
    Ok(generate_gcode(&strokes, s, &header, None))
}

pub fn make_test_gcode(s: &Settings) -> Result<(String, Vec<Vec<Point>>), Vec<String>> {
    let (mut errors, _) = check_settings(s);
    let strokes = test_pattern(s);
    errors.extend(check_bounds(&strokes, s, 5));
    if !errors.is_empty() {
        return Err(errors);
    }
    let header = ["TEST: writing area rectangle + axis arrows (X long, Y short)".to_string()];
    Ok((generate_gcode(&strokes, s, &header, None), strokes))
}

pub fn compose_with_font(s: &Settings, paths: &crate::glyphs::FontPaths) -> Composition {
    match crate::glyphs::load_provider(&s.font, s.mode, &s.outline, paths) {
        Ok(prov) => compose(s, prov.as_ref()),
        Err(e) => compose_without_font(s, &e),
    }
}

fn r3(v: f64) -> f64 {
    crate::numeric::round_to(v, 3)
}

fn r2p(p: &Point) -> serde_json::Value {
    serde_json::json!([r3(p.0), r3(p.1)])
}

pub fn char_name(c: char) -> String {
    unicode_names2::name(c).map_or_else(|| "?".to_string(), |n| n.to_string())
}

pub fn preview_payload(c: &Composition, prov: Option<&dyn GlyphProvider>) -> serde_json::Value {
    use serde_json::{Value, json};
    let s = &c.settings;
    let bx = crate::checks::travel_box(s);
    let strokes = c.strokes();
    let mut out = serde_json::Map::new();
    out.insert("errors".into(), json!(c.errors));
    out.insert("warnings".into(), json!(c.warnings));
    out.insert("sheet".into(), serde_json::to_value(&s.sheet).expect("sheet"));
    out.insert(
        "travel_box".into(),
        json!({"x_min": bx.x_min, "x_max": bx.x_max, "y_min": bx.y_min, "y_max": bx.y_max, "measured": bx.measured}),
    );
    out.insert("flip_x".into(), json!(s.printer.flip_x));
    out.insert("flip_y".into(), json!(s.printer.flip_y));
    out.insert("safety_margin".into(), json!(s.printer.safety_margin));
    for k in ["strokes", "travel", "glyphs", "baselines"] {
        out.insert(k.into(), json!([]));
    }
    out.insert("stats".into(), crate::gcode::compute_stats(&strokes, s).as_json());
    out.insert("missing".into(), json!([]));
    for k in ["font", "end", "resume"] {
        out.insert(k.into(), Value::Null);
    }
    if let Some(p) = prov {
        let info = p.info();
        let m = info.metrics.as_ref();
        out.insert(
            "font".into(),
            json!({
                "name": info.name, "mode": info.mode, "glyph_count": info.glyph_count, "chars": info.chars,
                "variants": info.variants, "notes": info.notes,
                "x_height": m.map(|m| m.x_height), "x_height_source": m.map_or("", |m| m.x_height_source.as_str()),
                "features": info.features, "gpos_features": info.gpos_features,
                "variant_sources": info.variant_sources,
                "ligatures": info.ligatures.iter().map(|(c, g, f)| json!({"chars": c, "glyph": g, "feature": f})).collect::<Vec<_>>(),
            }),
        );
    }
    if let Some(t) = &c.text {
        let missing: Vec<Value> = t
            .missing
            .iter()
            .map(|mc| {
                json!({
                    "char": mc.ch.to_string(), "code": mc.code(), "name": char_name(mc.ch), "count": mc.count,
                    "positions": mc.positions.iter()
                        .map(|p| json!({"line": p.line, "word": p.word, "letter": p.letter}))
                        .collect::<Vec<_>>(),
                    "resolved": !t.unresolved.contains(&mc.ch),
                })
            })
            .collect();
        out.insert("missing".into(), json!(missing));
        let replaced: serde_json::Map<String, Value> =
            t.replaced.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
        out.insert("replaced".into(), Value::Object(replaced));
        out.insert("word_count".into(), json!(t.words.len()));
    }
    if let Some(lay) = &c.layout {
        let r = c.resume.as_ref();
        let item = |st: &crate::layout::DrawnStroke, pts: &[Point], done: bool| {
            json!({"p": pts.iter().map(r2p).collect::<Vec<_>>(), "w": st.word, "l": st.letter, "n": st.line,
                   "h": st.hyphen, "d": done})
        };
        let mut items = Vec::new();
        for (pi, st) in lay.strokes.iter().enumerate() {
            match r {
                Some(r) if pi == r.path => {
                    if r.point > 0 {
                        items.push(item(st, &st.points[..=r.point], true));
                    }
                    items.push(item(st, &st.points[r.point..], false));
                }
                Some(r) if pi < r.path => items.push(item(st, &st.points, true)),
                _ => items.push(item(st, &st.points, false)),
            }
        }
        out.insert("strokes".into(), json!(items));
        let travel: Vec<Value> =
            crate::gcode::travel_moves(&strokes).iter().map(|(a, b)| json!([r2p(a), r2p(b)])).collect();
        out.insert("travel".into(), json!(travel));
        let mut boxes: std::collections::HashMap<usize, [f64; 4]> = std::collections::HashMap::new();
        for st in &lay.strokes {
            for (&(x, y), &t) in st.points.iter().zip(&st.tags) {
                let b = boxes.entry(t).or_insert([x, y, x, y]);
                *b = [
                    crate::numeric::min(b[0], x),
                    crate::numeric::min(b[1], y),
                    crate::numeric::max(b[2], x),
                    crate::numeric::max(b[3], y),
                ];
            }
        }
        let glyphs: Vec<Value> = lay
            .glyphs
            .iter()
            .enumerate()
            .map(|(i, g)| {
                json!({"w": g.word, "l": g.letter, "n": g.line, "c": g.ch.to_string(), "x": r3(g.x), "y": r3(g.y),
                       "adv": r3(g.advance), "missing": g.missing, "h": g.hyphen,
                       "b": boxes.get(&i).map(|b| b.iter().map(|v| r3(*v)).collect::<Vec<_>>())})
            })
            .collect();
        out.insert("glyphs".into(), json!(glyphs));
        out.insert("baselines".into(), json!(lay.baselines));
        out.insert("scale".into(), json!(lay.scale));
        out.insert(
            "end".into(),
            json!({"first_word": lay.first_word, "last_word": lay.last_word, "last_letter": lay.last_letter,
                   "next_word": lay.next_word, "next_letter": lay.next_letter}),
        );
        if let Some(r) = r {
            out.insert("resume".into(), json!({"word": r.word, "letter": r.letter, "connected": r.connected}));
        }
    }
    Value::Object(out)
}
