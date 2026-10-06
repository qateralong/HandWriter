use std::sync::{LazyLock, OnceLock, RwLock};

use serde_json::Value;

pub const LANGS: [&str; 2] = ["ru", "en"];

const EN: &str = include_str!("../../handwriter/locale/en.json");

const TR_KEYS: [&str; 11] = [
    "errors",
    "warnings",
    "detail",
    "notes",
    "message",
    "describe",
    "rotation_text",
    "label",
    "units_note",
    "corner_name",
    "x_height_source",
];

static LANG: RwLock<Option<String>> = RwLock::new(None);

struct Entry {
    key: String,
    value: String,
    cyr_start: bool,
    cyr_end: bool,
}

static EXTRA_EN: OnceLock<&'static str> = OnceLock::new();

static EN_TABLE: LazyLock<Vec<Entry>> = LazyLock::new(|| table(&[Some(EN), EXTRA_EN.get().copied()]));

pub fn add_table(code: &str, json: &'static str) {
    if code == "en" {
        let _ = EXTRA_EN.set(json);
    }
}

fn is_cyr(c: char) -> bool {
    ('\u{400}'..='\u{4ff}').contains(&c)
}

fn table(sources: &[Option<&str>]) -> Vec<Entry> {
    let mut words: serde_json::Map<String, Value> = serde_json::Map::new();
    for src in sources.iter().flatten() {
        let m: serde_json::Map<String, Value> = serde_json::from_str(src).expect("locale json");
        words.extend(m);
    }
    let mut out: Vec<Entry> = words
        .into_iter()
        .filter_map(|(k, v)| {
            let v = v.as_str()?.to_string();
            if v.is_empty() || k.is_empty() {
                return None;
            }
            let cyr_start = k.chars().next().is_some_and(is_cyr);
            let cyr_end = k.chars().last().is_some_and(is_cyr);
            Some(Entry { key: k, value: v, cyr_start, cyr_end })
        })
        .collect();
    out.sort_by(|a, b| b.key.chars().count().cmp(&a.key.chars().count()).then_with(|| a.key.cmp(&b.key)));
    out
}

pub fn lang() -> String {
    if let Some(l) = LANG.read().expect("lang lock").as_ref() {
        return l.clone();
    }
    let mut v = std::env::var("HANDWRITER_LANG").unwrap_or_default().trim().to_lowercase();
    if v.is_empty() {
        let file = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("lang.txt")));
        if let Some(text) = file.and_then(|f| std::fs::read_to_string(f).ok()) {
            v = text.trim().to_lowercase();
        }
    }
    let v = if LANGS.contains(&v.as_str()) { v } else { "ru".to_string() };
    *LANG.write().expect("lang lock") = Some(v.clone());
    v
}

pub fn set_lang(value: &str) {
    let v = if LANGS.contains(&value) { value } else { "ru" };
    *LANG.write().expect("lang lock") = Some(v.to_string());
}

fn replace_bounded(text: &str, e: &Entry) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    let mut copied = 0;
    while let Some(i) = text[pos..].find(&e.key) {
        let at = pos + i;
        let end = at + e.key.len();
        let before_ok = !e.cyr_start || !text[..at].chars().next_back().is_some_and(is_cyr);
        let after_ok = !e.cyr_end || !text[end..].chars().next().is_some_and(is_cyr);
        if before_ok && after_ok {
            out.push_str(&text[copied..at]);
            out.push_str(&e.value);
            copied = end;
            pos = end;
        } else {
            pos = at + text[at..].chars().next().map_or(1, char::len_utf8);
        }
        if pos >= text.len() {
            break;
        }
    }
    out.push_str(&text[copied..]);
    out
}

pub fn tr_to(text: &str, code: &str) -> String {
    if code == "ru" || text.is_empty() || !text.chars().any(is_cyr) {
        return text.to_string();
    }
    let table: &[Entry] = match code {
        "en" => &EN_TABLE,
        _ => return text.to_string(),
    };
    let mut text = text.to_string();
    for e in table {
        if text.contains(&e.key) {
            text = replace_bounded(&text, e);
            if !text.chars().any(is_cyr) {
                break;
            }
        }
    }
    text
}

pub fn tr(text: &str) -> String {
    tr_to(text, &lang())
}

pub fn tr_static(text: &str) -> String {
    let code = lang();
    if code == "ru" {
        return text.to_string();
    }
    tr_to(text, &code).replace("<html lang=\"ru\">", &format!("<html lang=\"{code}\">"))
}

pub fn tr_json(v: Value) -> Value {
    let code = lang();
    if code == "ru" {
        return v;
    }
    tr_json_key(v, None, &code)
}

fn tr_json_key(v: Value, key: Option<&str>, code: &str) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .map(|(k, v)| {
                    let t = tr_json_key(v, Some(&k), code);
                    (k, t)
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.into_iter().map(|x| tr_json_key(x, key, code)).collect()),
        Value::String(s) if key.is_some_and(|k| TR_KEYS.contains(&k)) => Value::String(tr_to(&s, code)),
        other => other,
    }
}
