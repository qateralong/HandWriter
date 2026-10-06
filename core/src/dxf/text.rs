use std::sync::LazyLock;

use regex::Regex;

static CARET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\^([^\n])").unwrap());

fn caret_decode(text: &str) -> String {
    CARET
        .replace_all(text, |c: &regex::Captures| {
            let ch = c[1].chars().next().expect("char") as i64;
            let v = (ch - 64).rem_euclid(126) as u32;
            char::from_u32(v).unwrap_or('\u{FFFD}').to_string()
        })
        .into_owned()
}

pub fn fast_plain_mtext(text: &str) -> String {
    let mut chars: Vec<char> = Vec::new();
    let mut raw: Vec<char> = caret_decode(text).chars().rev().collect();
    while let Some(ch) = raw.pop() {
        if ch == '\\' {
            let Some(c) = raw.pop() else { break };
            if "\\{}".contains(c) {
                chars.push(c);
            } else if "PNLlOoKkX".contains(c) {
                if c == 'P' {
                    chars.push('\n');
                } else if c == 'N' {
                    chars.push(' ');
                }
            } else {
                let stacking = c == 'S';
                let first = c;
                let mut search = raw.clone();
                let mut cur = c;
                let mut found = true;
                let mut buf = Vec::new();
                while cur != ';' {
                    match search.pop() {
                        Some(x) => {
                            cur = x;
                            if stacking && x != ';' {
                                buf.push(x);
                            }
                        }
                        None => {
                            found = false;
                            break;
                        }
                    }
                }
                chars.extend(buf);
                if found {
                    raw = search;
                } else {
                    chars.push('\\');
                    chars.push(first);
                }
            }
        } else if ch == '{' || ch == '}' {
        } else if ch == '%' {
            if raw.last() == Some(&'%') {
                raw.pop();
                if let Some(code) = raw.pop() {
                    match code.to_lowercase().collect::<String>().as_str() {
                        "c" => chars.push('Ø'),
                        "d" => chars.push('°'),
                        "p" => chars.push('±'),
                        _ => chars.extend(['%', '%', code]),
                    }
                }
            } else {
                chars.push(ch);
            }
        } else {
            chars.push(ch);
        }
    }
    chars.into_iter().collect()
}
