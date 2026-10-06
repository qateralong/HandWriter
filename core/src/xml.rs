use std::sync::LazyLock;

use encoding_rs::{Encoding, UTF_8};
use regex::bytes::Regex;

static DECL_ENCODING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*<\?xml[^>]*?encoding\s*=\s*["']([A-Za-z0-9._:-]+)["']"#).expect("valid regex"));

pub fn decode(bytes: &[u8]) -> Result<String, String> {
    if let Some((enc, bom)) = Encoding::for_bom(bytes) {
        let (text, had_errors) = enc.decode_without_bom_handling(&bytes[bom..]);
        return if had_errors { Err(format!("invalid {} text", enc.name())) } else { Ok(text.into_owned()) };
    }
    let enc = DECL_ENCODING
        .captures(bytes)
        .and_then(|c| Encoding::for_label(&c[1]))
        .map(|e| e.output_encoding())
        .unwrap_or(UTF_8);
    let (text, had_errors) = enc.decode_without_bom_handling(bytes);
    if had_errors { Err(format!("invalid {} text", enc.name())) } else { Ok(text.into_owned()) }
}

pub fn options<'a>() -> roxmltree::ParsingOptions<'a> {
    roxmltree::ParsingOptions { allow_dtd: true, ..roxmltree::ParsingOptions::default() }
}

pub fn parse(text: &str) -> Result<roxmltree::Document<'_>, String> {
    roxmltree::Document::parse_with_options(text, options()).map_err(|e| e.to_string())
}
