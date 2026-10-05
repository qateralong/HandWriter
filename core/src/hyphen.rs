use std::collections::HashMap;
use std::sync::LazyLock;

const RU_RU: &str = include_str!("../resources/hyphen/hyph_ru_RU.dic");
const EN_US: &str = include_str!("../resources/hyphen/hyph_en_US.dic");

const IGNORED: &[&str] =
    &["%", "#", "LEFTHYPHENMIN", "RIGHTHYPHENMIN", "COMPOUNDLEFTHYPHENMIN", "COMPOUNDRIGHTHYPHENMIN"];

pub struct Dictionary {
    patterns: HashMap<String, (usize, Vec<u32>)>,
    maxlen: usize,
    left: usize,
    right: usize,
}

fn replace_hex(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if i + 3 < chars.len()
            && chars[i] == '^'
            && chars[i + 1] == '^'
            && chars[i + 2..i + 4].iter().all(|c| c.is_ascii_digit() || ('a'..='f').contains(c))
        {
            let hex: String = chars[i + 2..i + 4].iter().collect();
            out.push(char::from_u32(u32::from_str_radix(&hex, 16).expect("hex digits")).expect("byte value"));
            i += 4;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn split_pattern(pattern: &str) -> (String, Vec<u32>) {
    let chars: Vec<char> = pattern.chars().collect();
    let mut tags = String::new();
    let mut values = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let mut v = 0;
        if let Some(d) = chars[i].to_digit(10).filter(|_| chars[i].is_ascii_digit()) {
            v = d;
            i += 1;
        }
        if i < chars.len() && !chars[i].is_ascii_digit() {
            tags.push(chars[i]);
            i += 1;
        }
        values.push(v);
    }
    values.push(0);
    (tags, values)
}

impl Dictionary {
    pub fn parse(text: &str, left: usize, right: usize) -> Self {
        let mut patterns = HashMap::new();
        for line in text.split('\n').skip(1) {
            let line = line.trim();
            if line.is_empty() || IGNORED.iter().any(|p| line.starts_with(p)) {
                continue;
            }
            let pattern = replace_hex(line);
            let (tags, values) = split_pattern(&pattern);
            if values.iter().all(|&v| v == 0) {
                continue;
            }
            let start = values.iter().position(|&v| v != 0).expect("non-zero value");
            let end = values.iter().rposition(|&v| v != 0).expect("non-zero value") + 1;
            patterns.insert(tags, (start, values[start..end].to_vec()));
        }
        let maxlen = patterns.keys().map(|k| k.chars().count()).max().unwrap_or(0);
        Self { patterns, maxlen, left, right }
    }

    fn raw_positions(&self, word: &str) -> Vec<usize> {
        let pointed: Vec<char> = format!(".{}.", word.to_lowercase()).chars().collect();
        let n = pointed.len();
        let mut refs = vec![0u32; n + 1];
        let mut key = String::new();
        for i in 0..n.saturating_sub(1) {
            let stop = (i + self.maxlen).min(n) + 1;
            for j in i + 1..stop {
                key.clear();
                key.extend(&pointed[i..j]);
                let Some((offset, values)) = self.patterns.get(&key) else { continue };
                let from = i + offset;
                for (k, &v) in values.iter().enumerate() {
                    if let Some(r) = refs.get_mut(from + k) {
                        *r = (*r).max(v);
                    }
                }
            }
        }
        refs.iter().enumerate().filter(|(_, r)| *r % 2 == 1).map(|(i, _)| i.wrapping_sub(1)).collect()
    }

    pub fn positions(&self, word: &str) -> Vec<usize> {
        let len = word.chars().count() as i64;
        let right = len - self.right as i64;
        self.raw_positions(word)
            .into_iter()
            .filter(|&i| {
                let i = i as i64;
                self.left as i64 <= i && i <= right
            })
            .collect()
    }
}

pub static RUSSIAN: LazyLock<Dictionary> = LazyLock::new(|| Dictionary::parse(RU_RU, 2, 2));
pub static ENGLISH: LazyLock<Dictionary> = LazyLock::new(|| Dictionary::parse(EN_US, 2, 2));
