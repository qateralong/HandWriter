use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;
use unicode_general_category::{GeneralCategory, get_general_category};
use unicode_normalization::UnicodeNormalization;

use crate::hyphen;
use crate::settings::{MissingAction, MissingChoice};

pub const SOFT_HYPHEN: char = '\u{ad}';

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub index: i64,
    pub text: String,
    pub chars: Vec<char>,
    pub paragraph: i64,
    pub text_line: i64,
    pub soft_breaks: BTreeSet<usize>,
}

impl Word {
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn slice(&self, a: usize, b: usize) -> String {
        self.chars[a..b].iter().collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph {
    pub index: i64,
    pub indent: bool,
    pub blank_before: i64,
    pub words: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    pub line: i64,
    pub word: i64,
    pub letter: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MissingChar {
    pub ch: char,
    pub count: usize,
    pub positions: Vec<Position>,
}

impl MissingChar {
    pub fn code(&self) -> String {
        format!("U+{:04X}", u32::from(self.ch))
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProcessedText {
    pub paragraphs: Vec<Paragraph>,
    pub words: Vec<Word>,
    pub missing: Vec<MissingChar>,
    pub skipped: BTreeSet<char>,
    pub unresolved: BTreeSet<char>,
    pub replaced: IndexMap<char, String>,
}

pub fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n").nfc().collect()
}

fn single_char(s: &str) -> Option<char> {
    let mut it = s.chars();
    let c = it.next()?;
    it.next().is_none().then_some(c)
}

pub fn apply_replacements(
    text: &str,
    has_char: &dyn Fn(&str) -> bool,
    replacements: &[(String, String)],
    missing_choices: &IndexMap<String, MissingChoice>,
) -> (String, IndexMap<char, String>) {
    let mut table: BTreeMap<char, String> = BTreeMap::new();
    for (src, dst) in replacements {
        if let Some(c) = single_char(src)
            && !has_char(src)
        {
            table.insert(c, dst.clone());
        }
    }
    for (ch, choice) in missing_choices {
        if let Some(c) = single_char(ch)
            && choice.action == MissingAction::Replace
            && !has_char(ch)
        {
            table.insert(c, choice.replacement.nfc().collect());
        }
    }
    let mut used = IndexMap::new();
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match table.get(&ch) {
            Some(dst) => {
                used.insert(ch, dst.clone());
                out.push_str(dst);
            }
            None => out.push(ch),
        }
    }
    (out, used)
}

pub fn strip_soft_hyphens(token: &str) -> (String, BTreeSet<usize>) {
    let mut out: Vec<char> = Vec::new();
    let mut breaks = BTreeSet::new();
    for ch in token.chars() {
        if ch == SOFT_HYPHEN {
            if !out.is_empty() {
                breaks.insert(out.len());
            }
        } else {
            out.push(ch);
        }
    }
    let n = out.len();
    (out.into_iter().collect(), breaks.into_iter().filter(|&b| 0 < b && b < n).collect())
}

fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

pub fn process_text(
    text: &str,
    has_char: &dyn Fn(&str) -> bool,
    replacements: &[(String, String)],
    missing_choices: &IndexMap<String, MissingChoice>,
) -> ProcessedText {
    let text = normalize(text);
    let (text, used) = apply_replacements(&text, has_char, replacements, missing_choices);

    let mut paragraphs: Vec<Paragraph> = Vec::new();
    let mut words: Vec<Word> = Vec::new();
    let mut blank = 0;
    for (line_no, line) in text.split('\n').enumerate() {
        let line_no = line_no as i64 + 1;
        let body = line.trim_start_matches(is_blank);
        let stripped = body.trim_matches(is_blank);
        if stripped.is_empty() {
            blank += 1;
            continue;
        }
        let lead = &line[..line.len() - body.len()];
        let mut p = Paragraph {
            index: paragraphs.len() as i64 + 1,
            indent: lead.contains('\t'),
            blank_before: blank,
            words: Vec::new(),
        };
        blank = 0;
        for token in stripped.split(is_blank).filter(|t| !t.is_empty()) {
            let (visible, soft) = strip_soft_hyphens(token);
            if visible.is_empty() {
                continue;
            }
            let w = Word {
                index: words.len() as i64 + 1,
                chars: visible.chars().collect(),
                text: visible,
                paragraph: p.index,
                text_line: line_no,
                soft_breaks: soft,
            };
            p.words.push(words.len());
            words.push(w);
        }
        if !p.words.is_empty() {
            paragraphs.push(p);
        }
    }

    let mut missing: IndexMap<char, MissingChar> = IndexMap::new();
    for w in &words {
        for (i, &ch) in w.chars.iter().enumerate() {
            if has_char(&ch.to_string()) {
                continue;
            }
            let mc = missing.entry(ch).or_insert_with(|| MissingChar { ch, count: 0, positions: Vec::new() });
            mc.count += 1;
            if mc.positions.len() < 20 {
                mc.positions.push(Position { line: w.text_line, word: w.index, letter: i as i64 + 1 });
            }
        }
    }
    let skipped: BTreeSet<char> = missing
        .keys()
        .copied()
        .filter(|ch| missing_choices.get(&ch.to_string()).is_some_and(|c| c.action == MissingAction::Skip))
        .collect();
    let unresolved = missing.keys().copied().filter(|ch| !skipped.contains(ch)).collect();
    ProcessedText { paragraphs, words, missing: missing.into_values().collect(), skipped, unresolved, replaced: used }
}

fn is_letter_run_char(c: char) -> bool {
    use GeneralCategory::*;
    matches!(
        get_general_category(c),
        UppercaseLetter | LowercaseLetter | TitlecaseLetter | ModifierLetter | OtherLetter | LetterNumber | OtherNumber
    )
}

fn is_cyrillic(c: char) -> bool {
    ('\u{400}'..='\u{4ff}').contains(&c)
}

pub fn break_positions(word: &Word, auto: bool) -> BTreeMap<usize, bool> {
    let chars = &word.chars;
    let mut res: BTreeMap<usize, bool> = word.soft_breaks.iter().map(|&b| (b, true)).collect();
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '-' && 0 < i && i + 1 < chars.len() {
            res.insert(i + 1, false);
        }
    }
    if auto {
        let mut i = 0;
        while i < chars.len() {
            if !is_letter_run_char(chars[i]) {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && is_letter_run_char(chars[i]) {
                i += 1;
            }
            let run = &chars[start..i];
            if run.len() < 4 {
                continue;
            }
            let dict = if run.iter().any(|&c| is_cyrillic(c)) { &*hyphen::RUSSIAN } else { &*hyphen::ENGLISH };
            let run: String = run.iter().collect();
            for pos in dict.positions(&run) {
                res.entry(start + pos).or_insert(true);
            }
        }
    }
    res
}
