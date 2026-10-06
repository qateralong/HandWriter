#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use handwriter_core::calibration;
use handwriter_core::drawing::model::ImportResult;
use handwriter_core::drawing::passes::{corner, corner_name};
use handwriter_core::drawing::pipeline as dp;
use handwriter_core::drawing::sources;
use handwriter_core::gcode::test_pattern;
use handwriter_core::glyphs::{BUILTIN_PREFIX, SharedProvider, load_provider, resolve_font_path};
use handwriter_core::i18n::{lang, tr, tr_json, tr_static};
use handwriter_core::numeric::round_to;
use handwriter_core::pipeline::{compose, compose_without_font, make_gcode, make_test_gcode, preview_payload};
use handwriter_core::settings::DrawingImport;
use handwriter_core::settings::{Mode, OutlineOptions, Settings, default_profiles, load_settings, save_settings};
use handwriter_core::text::normalize;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::assets::{static_file, ui_file};
use crate::logs;
use crate::paths::{font_paths, log_path, settings_path, user_drawings_dir, user_fonts_dir};
use crate::printer::{self, PRINTER};

pub struct Activity {
    pub started: Instant,
    pub last_ping_ms: AtomicU64,
    pub token: String,
}

impl Activity {
    pub fn new() -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let token = format!("{:016x}", (nanos as u64) ^ u64::from(std::process::id()).rotate_left(32));
        Activity { started: Instant::now(), last_ping_ms: AtomicU64::new(0), token }
    }

    pub fn ping(&self) {
        let ms = self.started.elapsed().as_millis() as u64;
        self.last_ping_ms.store(ms.max(1), Ordering::Relaxed);
    }

    pub fn last_ping(&self) -> Option<f64> {
        let ms = self.last_ping_ms.load(Ordering::Relaxed);
        (ms > 0).then_some(ms as f64 / 1000.0)
    }
}

type St = Arc<Activity>;

fn json_resp(status: StatusCode, v: Value) -> Response {
    let body = serde_json::to_vec(&tr_json(v)).expect("json");
    (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

fn ok(v: Value) -> Response {
    json_resp(StatusCode::OK, v)
}

fn detail(status: StatusCode, msg: &str) -> Response {
    json_resp(status, json!({"detail": msg}))
}

fn refused(errors: Vec<String>) -> Response {
    json_resp(StatusCode::UNPROCESSABLE_ENTITY, json!({"errors": errors}))
}

const FIELD_NAMES: [(&str, &str); 8] = [
    ("sheet", "Лист"),
    ("typography", "Размер и подгонка"),
    ("printer", "Принтер"),
    ("randomness", "Случайность"),
    ("connections", "Слитное письмо"),
    ("outline", "Контуры → линии"),
    ("text_options", "Текст"),
    ("drawing", "Чертёж"),
];

fn where_text(path: &str) -> String {
    let parts: Vec<&str> = path.split('.').filter(|p| !p.is_empty() && *p != "?").collect();
    if parts.is_empty() {
        return "запрос".into();
    }
    let first = FIELD_NAMES.iter().find(|(k, _)| *k == parts[0]).map_or(parts[0], |(_, v)| *v);
    std::iter::once(first).chain(parts[1..].iter().copied()).collect::<Vec<_>>().join(" → ")
}

fn invalid(path: &str, msgs: Vec<String>) -> Response {
    let text = format!("Неверные настройки. {}", msgs.join("; "));
    logs::warn("handwriter.server", &format!("Неверные данные {path}: {}", msgs.join("; ")));
    json_resp(StatusCode::UNPROCESSABLE_ENTITY, json!({"detail": text, "errors": [text]}))
}

fn parse_body<T: for<'de> Deserialize<'de>>(path: &str, body: &[u8]) -> Result<T, Response> {
    let mut de = serde_json::Deserializer::from_slice(body);
    match serde_path_to_error::deserialize::<_, T>(&mut de) {
        Ok(v) => Ok(v),
        Err(e) => {
            let inner = e.inner().to_string();
            let what = if inner.contains("missing field") {
                "не заполнено"
            } else if inner.contains("expected f64")
                || inner.contains("expected i64")
                || inner.contains("expected u")
                || inner.contains("integer")
                || inner.contains("float")
            {
                "нужно число"
            } else {
                "неверное значение"
            };
            Err(invalid(path, vec![format!("{}: {what}", where_text(&e.path().to_string()))]))
        }
    }
}

fn settings_from(path: &str, body: &[u8]) -> Result<Settings, Response> {
    let s: Settings = parse_body(path, body)?;
    let bad = s.invalid_fields();
    if bad.is_empty() {
        Ok(s)
    } else {
        let msgs = bad.iter().take(5).map(|f| format!("{}: значение вне допустимых пределов", where_text(f))).collect();
        Err(invalid(path, msgs))
    }
}

macro_rules! body_settings {
    ($path:expr, $body:expr) => {
        match settings_from($path, &$body) {
            Ok(s) => s,
            Err(r) => return r,
        }
    };
}

async fn blocking<F: FnOnce() -> Response + Send + 'static>(path: String, f: F) -> Response {
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r,
        Err(e) => {
            let msg = if e.is_panic() {
                let p = e.into_panic();
                p.downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into())
            } else {
                e.to_string()
            };
            logs::error("handwriter.server", &format!("Ошибка при обработке {path}: {msg}"));
            let text =
                format!("Внутренняя ошибка программы ({msg}). Подробности записаны в журнал: {}", log_path().display());
            json_resp(StatusCode::INTERNAL_SERVER_ERROR, json!({"detail": text, "errors": [text]}))
        }
    }
}

fn ui_page() -> Response {
    let data = ui_file("index.html").expect("embedded ui");
    let text = tr_static(std::str::from_utf8(data).expect("utf8 page"));
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-store")], text).into_response()
}

async fn ui_handler(UrlPath(name): UrlPath<String>, uri: Uri) -> Response {
    match ui_file(&name) {
        Some(data) => asset_response(&name, data),
        None => detail(StatusCode::NOT_FOUND, &format!("Не найдено: {}", uri.path())),
    }
}

fn page(name: &str) -> Response {
    let data = static_file(name).expect("embedded page");
    let text =
        tr_static(std::str::from_utf8(data).expect("utf8 page")).replace("<a href=\"/\"", "<a href=\"/classic\"");
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-store")], text).into_response()
}

async fn static_handler(UrlPath(name): UrlPath<String>, uri: Uri) -> Response {
    match static_file(&name) {
        Some(data) => asset_response(&name, data),
        None => detail(StatusCode::NOT_FOUND, &format!("Не найдено: {}", uri.path())),
    }
}

fn asset_response(name: &str, data: &'static [u8]) -> Response {
    let ext = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("");
    let ctype = match ext {
        "js" => "text/javascript; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "png" => "image/png",
        "css" => "text/css; charset=utf-8",
        _ => "application/octet-stream",
    };
    if matches!(ext, "js" | "html") {
        let text = std::str::from_utf8(data).expect("utf8");
        let body = if lang() != "ru" { tr_static(text) } else { text.to_string() };
        return ([(header::CONTENT_TYPE, ctype), (header::CACHE_CONTROL, "no-store")], body).into_response();
    }
    ([(header::CONTENT_TYPE, ctype)], data).into_response()
}

fn mode_for_path(p: &Path) -> Mode {
    match p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()) {
        Some(e) if e == "ttf" || e == "otf" => Mode::Outlines,
        _ => Mode::Strokes,
    }
}

fn mode_str(m: Mode) -> &'static str {
    match m {
        Mode::Strokes => "strokes",
        Mode::Outlines => "outlines",
    }
}

fn provider(spec: &str, mode: Mode, outline: &OutlineOptions) -> Result<SharedProvider, String> {
    load_provider(spec, mode, outline, &font_paths())
}

static SAFE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"[<>:"/\\|?*\x00-\x1f]"#).unwrap());

fn base_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

fn safe(name: &str) -> String {
    SAFE.replace_all(name, "_").into_owned()
}

fn loader(spec: &str, imp: &DrawingImport, tol: f64) -> ImportResult {
    sources::load_drawing(spec, imp, tol, &user_drawings_dir())
}

fn qbool(q: &HashMap<String, String>, k: &str, default: bool) -> Result<bool, Response> {
    match q.get(k).map(|v| v.to_lowercase()) {
        None => Ok(default),
        Some(v) if ["1", "true", "yes", "on", "t", "y"].contains(&v.as_str()) => Ok(true),
        Some(v) if ["0", "false", "no", "off", "f", "n"].contains(&v.as_str()) => Ok(false),
        Some(_) => Err(invalid("query", vec![format!("{k}: неверное значение")])),
    }
}

fn file_json(f: &dp::GcodeFile) -> Value {
    json!({"filename": f.filename, "gcode": f.gcode, "pass": f.pass_index, "rotation": f.rotation, "test": f.test})
}

#[derive(Deserialize)]
struct UploadFile {
    filename: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    content_b64: Option<String>,
}

#[derive(Deserialize)]
struct FontUpload {
    name: String,
    files: Vec<UploadFile>,
}

#[derive(Deserialize)]
struct DrawingUpload {
    filename: String,
    content_b64: String,
}

#[derive(Deserialize)]
struct DebugRequest {
    settings: Settings,
    #[serde(default)]
    text: String,
    #[serde(default)]
    glyph: String,
}

fn font_reply(spec: &str) -> Response {
    let mode = mode_for_path(&resolve_font_path(spec, &font_paths()));
    match provider(spec, mode, &OutlineOptions::default()) {
        Ok(p) => {
            let info = p.info();
            ok(json!({"spec": spec, "mode": mode_str(mode), "name": info.name, "glyph_count": info.glyph_count,
                      "notes": info.notes}))
        }
        Err(e) => detail(StatusCode::BAD_REQUEST, &format!("Не удалось прочитать шрифт: {e}")),
    }
}

fn upload_font(up: FontUpload) -> Response {
    let base = user_fonts_dir();
    let lower = |f: &UploadFile| f.filename.to_lowercase();
    if let Some(f) = up.files.iter().find(|f| lower(f).ends_with(".ttf") || lower(f).ends_with(".otf")) {
        let Some(b64) = f.content_b64.as_deref().filter(|b| !b.is_empty()) else {
            return detail(StatusCode::BAD_REQUEST, "TTF/OTF нужно передавать в content_b64");
        };
        let Ok(data) = B64.decode(b64) else {
            return detail(StatusCode::BAD_REQUEST, "Файл повреждён при передаче");
        };
        let name = safe(base_name(&f.filename));
        if let Err(e) = std::fs::write(base.join(&name), data) {
            return detail(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
        }
        return font_reply(&name);
    }
    let svgs: Vec<&UploadFile> =
        up.files.iter().filter(|f| lower(f).ends_with(".svg") || lower(f).ends_with(".json")).collect();
    if svgs.is_empty() {
        return detail(StatusCode::BAD_REQUEST, "Нужен файл .ttf, .otf или .svg");
    }
    let spec = if svgs.len() == 1 && lower(svgs[0]).ends_with(".svg") {
        let name = safe(base_name(&svgs[0].filename));
        if let Err(e) = std::fs::write(base.join(&name), &svgs[0].content) {
            return detail(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
        }
        name
    } else {
        let folder_name = safe(if up.name.is_empty() { "font" } else { &up.name });
        let folder = base.join(&folder_name);
        let _ = std::fs::create_dir_all(&folder);
        for f in svgs {
            let name = base_name(&f.filename);
            let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
            let stem = if SAFE.is_match(stem) {
                stem.chars()
                    .map(|c| if SAFE.is_match(&c.to_string()) { format!("uni{:04X}", c as u32) } else { c.to_string() })
                    .collect()
            } else {
                stem.to_string()
            };
            if let Err(e) = std::fs::write(folder.join(format!("{stem}.{ext}")), &f.content) {
                return detail(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
            }
        }
        folder_name
    };
    font_reply(&spec)
}

fn list_fonts() -> Value {
    let fp = font_paths();
    let ext_ok = |p: &Path| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| ["svg", "ttf", "otf"].contains(&e.to_lowercase().as_str()))
    };
    let mut out = Vec::new();
    let mut builtin: Vec<_> =
        std::fs::read_dir(&fp.builtin_dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    builtin.sort();
    for p in builtin.iter().filter(|p| ext_ok(p)) {
        let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        out.push(json!({"spec": format!("{BUILTIN_PREFIX}{name}"), "label": format!("Встроенный: {stem}"),
                        "mode": mode_str(mode_for_path(p))}));
    }
    let mut user: Vec<_> =
        std::fs::read_dir(&fp.user_fonts_dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    user.sort();
    for p in user.iter().filter(|p| ext_ok(p) || p.is_dir()) {
        let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let label = if p.is_dir() { format!("{name}/") } else { name.clone() };
        out.push(json!({"spec": name, "label": label, "mode": mode_str(mode_for_path(p))}));
    }
    json!(out)
}

fn text_preview(s: Settings) -> Response {
    match provider(&s.font, s.mode, &s.outline) {
        Ok(p) => {
            let c = compose(&s, p.as_ref());
            ok(preview_payload(&c, Some(p.as_ref())))
        }
        Err(e) => {
            let c = compose_without_font(&s, &e);
            ok(preview_payload(&c, None))
        }
    }
}

fn opt(v: Option<i64>) -> String {
    v.map_or_else(|| "None".into(), |x| x.to_string())
}

fn text_gcode(s: Settings) -> Response {
    let c = match provider(&s.font, s.mode, &s.outline) {
        Ok(p) => compose(&s, p.as_ref()),
        Err(e) => compose_without_font(&s, &e),
    };
    match make_gcode(&c) {
        Err(e) => refused(e),
        Ok(code) => {
            let lay = c.layout.as_ref();
            let mut name =
                format!("handwriter_w{}-{}", opt(lay.and_then(|l| l.first_word)), opt(lay.and_then(|l| l.last_word)));
            if let Some(r) = &c.resume {
                name += &format!("_from_w{}_l{}", r.word, r.letter);
            }
            name += ".gcode";
            ok(json!({"gcode": code, "filename": name, "warnings": c.warnings}))
        }
    }
}

fn drawing_gcode(s: Settings, q: HashMap<String, String>) -> Response {
    let part: i64 = match q.get("part").map(|v| v.trim().parse::<i64>()) {
        None => 0,
        Some(Ok(v)) => v,
        Some(Err(_)) => return invalid("query", vec!["part: нужно число".into()]),
    };
    let (test, all_tests) = match (qbool(&q, "test", false), qbool(&q, "all_tests", false)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(r), _) | (_, Err(r)) => return r,
    };
    let c = dp::compose_drawing(&s, &loader);
    let files: Result<Vec<Value>, Vec<String>> = if part != 0 {
        if part < 1 || part as usize > c.parts.len() {
            return detail(StatusCode::BAD_REQUEST, &format!("Нет прохода {part}"));
        }
        let p = &c.parts[part as usize - 1];
        if !c.errors.is_empty() {
            Err(c.errors.clone())
        } else {
            let code = if test { dp::make_part_test_gcode(&c, p) } else { dp::make_part_gcode(&c, p) };
            code.map(|g| {
                vec![json!({"filename": dp::part_filename(&c, p, test), "gcode": g, "pass": p.index,
                            "rotation": p.rotation, "test": test})]
            })
        }
    } else if test {
        if !c.errors.is_empty() {
            Err(c.errors.clone())
        } else {
            c.parts
                .iter()
                .map(|p| {
                    dp::make_part_test_gcode(&c, p).map(|g| {
                        json!({"filename": dp::part_filename(&c, p, true), "gcode": g, "pass": p.index,
                               "rotation": p.rotation, "test": true})
                    })
                })
                .collect()
        }
    } else {
        dp::make_all_files(&c, all_tests).map(|fs| fs.iter().map(file_json).collect())
    };
    match files {
        Err(e) => refused(e),
        Ok(files) => {
            let first = files[0].clone();
            ok(json!({"files": files, "gcode": first["gcode"], "filename": first["filename"], "warnings": c.warnings}))
        }
    }
}

fn build_zip(s: &Settings, tests: bool) -> Result<(String, Vec<u8>), Response> {
    let c = dp::compose_drawing(s, &loader);
    let files = match dp::make_all_files(&c, tests) {
        Ok(f) => f,
        Err(e) => return Err(refused(e)),
    };
    let mut lines = vec![format!("Рабочих областей: {}", c.parts.len()), String::new()];
    for p in &c.parts {
        let cn = corner(p.rotation);
        lines.push(format!("{}. {}", p.index, dp::part_filename(&c, p, false)));
        lines.push(format!(
            "   Поворот листа {}°, в упоры угол {cn} ({}), карандаш в этот угол до касания — ноль.",
            p.rotation,
            corner_name(cn)
        ));
    }
    lines.push(String::new());
    lines.push("Запускай файлы по порядку номеров. Файлы *_test.gcode — тестовые проходы без линий чертежа.".into());
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for f in &files {
            if z.start_file(f.filename.as_str(), opts).is_err() || z.write_all(f.gcode.as_bytes()).is_err() {
                return Err(detail(StatusCode::INTERNAL_SERVER_ERROR, "zip"));
            }
        }
        let readme = tr(&lines.join("\n")) + "\n";
        if z.start_file(tr("порядок.txt"), opts).is_err() || z.write_all(readme.as_bytes()).is_err() {
            return Err(detail(StatusCode::INTERNAL_SERVER_ERROR, "zip"));
        }
        if z.finish().is_err() {
            return Err(detail(StatusCode::INTERNAL_SERVER_ERROR, "zip"));
        }
    }
    Ok((dp::file_stem(&c) + ".zip", buf.into_inner()))
}

fn drawing_zip_save(s: Settings, tests: bool) -> Response {
    let (name, data) = match build_zip(&s, tests) {
        Ok(z) => z,
        Err(r) => return r,
    };
    let path = crate::paths::unique_path(&crate::paths::downloads_dir(), &name);
    match std::fs::write(&path, data) {
        Ok(()) => {
            logs::info("handwriter.server", &format!("Архив сохранён: {}", path.display()));
            ok(json!({"path": path.display().to_string(), "filename": name}))
        }
        Err(e) => detail(StatusCode::INTERNAL_SERVER_ERROR, &format!("Не удалось сохранить архив: {e}")),
    }
}

fn drawing_zip(s: Settings, tests: bool) -> Response {
    let (name, data) = match build_zip(&s, tests) {
        Ok(z) => z,
        Err(r) => return r,
    };
    let ascii: String = name.chars().map(|ch| if ch.is_ascii() && ch != '?' { ch } else { '_' }).collect();
    let quoted: String = name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let disp = format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{quoted}");
    let mut r = (StatusCode::OK, data).into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    if let Ok(v) = HeaderValue::from_str(&disp) {
        r.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    r
}

fn debug_glyphs(req: DebugRequest) -> Response {
    let s = req.settings;
    let p = match provider(&s.font, s.mode, &s.outline) {
        Ok(p) => p,
        Err(e) => return detail(StatusCode::BAD_REQUEST, &format!("Шрифт не загрузился: {e}")),
    };
    let mut items: Vec<Value> = Vec::new();
    let put = |d: Value, x: f64, y: f64, ch: String, cl: i64| {
        let mut d = d;
        if let Value::Object(m) = &mut d {
            m.insert("x".into(), json!(x));
            m.insert("y".into(), json!(y));
            m.insert("char".into(), json!(ch));
            m.insert("cluster".into(), json!(cl));
        }
        d
    };
    if !req.glyph.is_empty() {
        match p.debug_glyph(&req.glyph) {
            Some(d) => items.push(put(d, 0.0, 0.0, String::new(), -1)),
            None => return detail(StatusCode::BAD_REQUEST, &format!("В шрифте нет глифа «{}»", req.glyph)),
        }
    } else {
        let text: Vec<char> = normalize(&req.text).chars().collect();
        let joined: String = text.iter().collect();
        let mut x = 0.0;
        for sg in p.shape(&joined) {
            if let Some(d) = p.debug_glyph(&sg.name) {
                let ch = text.get(sg.cluster).map(|c| c.to_string()).unwrap_or_default();
                items.push(put(d, x + sg.x_offset, sg.y_offset, ch, sg.cluster as i64));
            }
            x += sg.advance;
        }
    }
    let m = p.metrics();
    ok(json!({"font": p.name(), "mode": p.mode(),
              "metrics": {"x_height": m.x_height, "cap_height": m.cap_height, "ascent": m.ascent, "descent": m.descent},
              "glyphs": items}))
}

pub fn router(state: St) -> Router {
    Router::new()
        .route("/api/ping", get(ping).post(ping))
        .route("/api/instance", get(instance))
        .route("/", get(|| async { ui_page() }))
        .route("/ui/{name}", get(ui_handler))
        .route("/classic", get(|| async { page("index.html") }))
        .route("/drawing", get(|| async { page("drawing.html") }))
        .route("/debug", get(|| async { page("debug.html") }))
        .route("/static/{name}", get(static_handler))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/defaults", get(get_defaults))
        .route("/api/app", get(get_app).put(put_app))
        .route("/api/save", post(save_file))
        .route("/api/preview", post(|b: Bytes| async move { with_settings("/api/preview", b, text_preview).await }))
        .route("/api/gcode", post(|b: Bytes| async move { with_settings("/api/gcode", b, text_gcode).await }))
        .route("/api/testfile", post(|b: Bytes| async move { with_settings("/api/testfile", b, testfile).await }))
        .route(
            "/api/testfile/preview",
            post(|b: Bytes| async move { with_settings("/api/testfile/preview", b, testfile_preview).await }),
        )
        .route("/api/fonts", get(|| async { blocking("/api/fonts".into(), || ok(list_fonts())).await }))
        .route("/api/fonts/upload", post(fonts_upload))
        .route(
            "/api/drawing/files",
            get(|| async {
                let files = sources::list_files(&user_drawings_dir());
                ok(json!(
                    files.iter().map(|f| json!({"spec": f.spec, "label": f.label, "kind": f.kind})).collect::<Vec<_>>()
                ))
            }),
        )
        .route("/api/drawing/upload", post(drawing_upload))
        .route(
            "/api/drawing/preview",
            post(|b: Bytes| async move {
                with_settings("/api/drawing/preview", b, |s| ok(dp::preview_payload(&dp::compose_drawing(&s, &loader))))
                    .await
            }),
        )
        .route(
            "/api/drawing/gcode",
            post(|Query(q): Query<HashMap<String, String>>, b: Bytes| async move {
                with_settings("/api/drawing/gcode", b, move |s| drawing_gcode(s, q)).await
            }),
        )
        .route(
            "/api/drawing/zip/save",
            post(|Query(q): Query<HashMap<String, String>>, b: Bytes| async move {
                let tests = match qbool(&q, "tests", true) {
                    Ok(t) => t,
                    Err(r) => return r,
                };
                with_settings("/api/drawing/zip/save", b, move |s| drawing_zip_save(s, tests)).await
            }),
        )
        .route(
            "/api/drawing/zip",
            post(|Query(q): Query<HashMap<String, String>>, b: Bytes| async move {
                let tests = match qbool(&q, "tests", true) {
                    Ok(t) => t,
                    Err(r) => return r,
                };
                with_settings("/api/drawing/zip", b, move |s| drawing_zip(s, tests)).await
            }),
        )
        .route(
            "/api/calibration/info",
            post(|b: Bytes| async move {
                with_settings("/api/calibration/info", b, |s| {
                    let (errors, warnings) = calibration::check_errors(&s);
                    let corners: Vec<Value> = if s.printer.travel.is_some() && errors.is_empty() {
                        calibration::reach_corners(&s)
                            .iter()
                            .map(|p| json!([round_to(p.0, 2), round_to(p.1, 2)]))
                            .collect()
                    } else {
                        Vec::new()
                    };
                    ok(json!({"errors": errors, "warnings": warnings, "corners": corners}))
                })
                .await
            }),
        )
        .route(
            "/api/calibration/check_gcode",
            post(|b: Bytes| async move {
                with_settings("/api/calibration/check_gcode", b, |s| match calibration::make_reach_check_gcode(&s) {
                    Ok(g) => ok(json!({"gcode": g, "filename": "handwriter_reach_check.gcode"})),
                    Err(e) => refused(e),
                })
                .await
            }),
        )
        .route(
            "/api/calibration/zero_gcode",
            post(|b: Bytes| async move {
                with_settings("/api/calibration/zero_gcode", b, |s| match calibration::make_zero_gcode(&s) {
                    Ok(g) => ok(json!({"gcode": g, "filename": "handwriter_zero.gcode"})),
                    Err(e) => refused(e),
                })
                .await
            }),
        )
        .route(
            "/api/debug/glyphs",
            post(|b: Bytes| async move {
                let req: DebugRequest = match parse_body("/api/debug/glyphs", &b) {
                    Ok(r) => r,
                    Err(r) => return r,
                };
                blocking("/api/debug/glyphs".into(), move || debug_glyphs(req)).await
            }),
        )
        .route(
            "/api/debug/names",
            post(|b: Bytes| async move {
                with_settings("/api/debug/names", b, |s| match provider(&s.font, s.mode, &OutlineOptions::default()) {
                    Ok(p) => ok(json!({"names": p.glyph_names()})),
                    Err(e) => detail(StatusCode::BAD_REQUEST, &format!("Шрифт не загрузился: {e}")),
                })
                .await
            }),
        )
        .route(
            "/api/printer/ports",
            get(|| async { blocking("/api/printer/ports".into(), || ok(json!(printer::list_ports()))).await }),
        )
        .route("/api/printer/status", get(printer_status))
        .route("/api/printer/connect", post(printer_connect))
        .route("/api/printer/disconnect", post(|| async { printer_result(PRINTER.disconnect()) }))
        .route("/api/printer/command", post(printer_command))
        .route("/api/printer/pause", post(|| async { printer_result(PRINTER.pause()) }))
        .route("/api/printer/resume", post(|| async { printer_result(PRINTER.resume()) }))
        .route("/api/printer/stop", post(|| async { printer_result(PRINTER.stop()) }))
        .route("/api/printer/emergency", post(|| async { printer_result(PRINTER.emergency()) }))
        .route(
            "/api/printer/print",
            post(|Query(q): Query<HashMap<String, String>>, b: Bytes| async move {
                with_settings("/api/printer/print", b, move |s| printer_print(s, q)).await
            }),
        )
        .fallback(|uri: Uri| async move { detail(StatusCode::NOT_FOUND, &format!("Не найдено: {}", uri.path())) })
        .with_state(state)
}

async fn with_settings<F: FnOnce(Settings) -> Response + Send + 'static>(path: &str, b: Bytes, f: F) -> Response {
    let s = body_settings!(path, b);
    blocking(path.to_string(), move || f(s)).await
}

async fn ping(State(st): State<St>) -> Response {
    st.ping();
    ok(json!({"ok": true}))
}

async fn instance(State(st): State<St>) -> Response {
    ok(json!({"app": "HandWriter", "token": st.token, "version": env!("CARGO_PKG_VERSION")}))
}

async fn get_settings() -> Response {
    blocking("/api/settings".into(), || match load_settings(&settings_path()) {
        Ok(s) => ok(serde_json::to_value(&s).expect("settings")),
        Err(e) => detail(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    })
    .await
}

async fn put_settings(b: Bytes) -> Response {
    let s = body_settings!("/api/settings", b);
    blocking("/api/settings".into(), move || match save_settings(&s, &settings_path()) {
        Ok(()) => ok(json!({"ok": true})),
        Err(e) => detail(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    })
    .await
}

async fn get_defaults() -> Response {
    let profiles: serde_json::Map<String, Value> =
        default_profiles().into_iter().map(|(k, v)| (k, serde_json::to_value(v).expect("profile"))).collect();
    ok(json!({"settings": Settings::default(), "profiles": profiles}))
}

fn testfile(s: Settings) -> Response {
    match make_test_gcode(&s) {
        Ok((g, _)) => ok(json!({"gcode": g, "filename": "handwriter_test.gcode"})),
        Err(e) => refused(e),
    }
}

fn testfile_preview(s: Settings) -> Response {
    let strokes: Vec<Vec<[f64; 2]>> =
        test_pattern(&s).iter().map(|st| st.iter().map(|p| [round_to(p.0, 3), round_to(p.1, 3)]).collect()).collect();
    ok(json!({"strokes": strokes}))
}

async fn fonts_upload(b: Bytes) -> Response {
    let up: FontUpload = match parse_body("/api/fonts/upload", &b) {
        Ok(u) => u,
        Err(r) => return r,
    };
    blocking("/api/fonts/upload".into(), move || upload_font(up)).await
}

async fn drawing_upload(b: Bytes) -> Response {
    let up: DrawingUpload = match parse_body("/api/drawing/upload", &b) {
        Ok(u) => u,
        Err(r) => return r,
    };
    blocking("/api/drawing/upload".into(), move || {
        let name = safe(base_name(&up.filename));
        let Some(kind) = sources::file_kind(&name) else {
            return detail(StatusCode::BAD_REQUEST, "Нужен файл .dxf, .jpeg, .jpg, .pdf, .png, .svg");
        };
        let Ok(data) = B64.decode(up.content_b64.trim()) else {
            return detail(StatusCode::BAD_REQUEST, "Файл повреждён при передаче");
        };
        match std::fs::write(user_drawings_dir().join(&name), data) {
            Ok(()) => ok(json!({"spec": name, "kind": kind})),
            Err(e) => detail(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        }
    })
    .await
}

async fn get_app() -> Response {
    ok(json!({"lang": lang(), "langs": [["ru", "Русский"], ["en", "English"]], "version": env!("CARGO_PKG_VERSION")}))
}

#[derive(Deserialize)]
struct AppPrefs {
    lang: String,
}

async fn put_app(b: Bytes) -> Response {
    let prefs: AppPrefs = match parse_body("/api/app", &b) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if !handwriter_core::i18n::LANGS.contains(&prefs.lang.as_str()) {
        return detail(StatusCode::BAD_REQUEST, "Неизвестный язык");
    }
    if let Err(e) = std::fs::write(crate::paths::lang_path(), &prefs.lang) {
        return detail(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    handwriter_core::i18n::set_lang(&prefs.lang);
    ok(json!({"ok": true, "lang": prefs.lang}))
}

#[derive(Deserialize)]
struct SaveRequest {
    filename: String,
    content: String,
}

async fn save_file(b: Bytes) -> Response {
    let req: SaveRequest = match parse_body("/api/save", &b) {
        Ok(r) => r,
        Err(r) => return r,
    };
    blocking("/api/save".into(), move || {
        let name = safe(base_name(&req.filename));
        if name.is_empty() || name.starts_with('.') {
            return detail(StatusCode::BAD_REQUEST, "Неверное имя файла");
        }
        let path = crate::paths::unique_path(&crate::paths::downloads_dir(), &name);
        match std::fs::write(&path, req.content) {
            Ok(()) => {
                logs::info("handwriter.server", &format!("Файл сохранён: {}", path.display()));
                ok(json!({"path": path.display().to_string(), "filename": name}))
            }
            Err(e) => detail(StatusCode::INTERNAL_SERVER_ERROR, &format!("Не удалось сохранить файл: {e}")),
        }
    })
    .await
}

fn printer_result(r: Result<(), String>) -> Response {
    match r {
        Ok(()) => ok(json!({"ok": true})),
        Err(e) => detail(StatusCode::CONFLICT, &e),
    }
}

async fn printer_status(Query(q): Query<HashMap<String, String>>) -> Response {
    let num = |k: &str| q.get(k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    ok(PRINTER.status(num("gen"), num("from") as usize, num("log")))
}

#[derive(Deserialize)]
struct ConnectRequest {
    port: String,
    #[serde(default = "default_baud")]
    baud: u32,
}

fn default_baud() -> u32 {
    115200
}

async fn printer_connect(b: Bytes) -> Response {
    let req: ConnectRequest = match parse_body("/api/printer/connect", &b) {
        Ok(r) => r,
        Err(r) => return r,
    };
    blocking("/api/printer/connect".into(), move || printer_result(PRINTER.connect(&req.port, req.baud))).await
}

#[derive(Deserialize)]
struct CommandRequest {
    cmd: String,
}

async fn printer_command(b: Bytes) -> Response {
    let req: CommandRequest = match parse_body("/api/printer/command", &b) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let lines: Vec<String> = req.cmd.lines().filter_map(printer::protocol::clean_line).collect();
    if lines.is_empty() {
        return detail(StatusCode::BAD_REQUEST, "Пустая команда");
    }
    if lines.iter().any(|l| l.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case("M112"))) {
        return printer_result(PRINTER.emergency());
    }
    printer_result(PRINTER.command(lines))
}

fn printer_print(s: Settings, q: HashMap<String, String>) -> Response {
    let test = match qbool(&q, "test", false) {
        Ok(t) => t,
        Err(r) => return r,
    };
    let pen = printer::Pen {
        up: s.printer.pen_up_z,
        down: s.printer.pen_down_z,
        end: s.printer.pen_up_z + s.printer.end_lift,
        feed_z: s.printer.feed_z,
        feed_travel: s.printer.feed_travel,
    };
    let kind = q.get("kind").map_or("text", String::as_str);
    let (code, map, label, part) = if kind == "drawing" {
        let part: usize = match q.get("part").map(|v| v.trim().parse::<usize>()) {
            None => 1,
            Some(Ok(v)) => v,
            Some(Err(_)) => return invalid("query", vec!["part: нужно число".into()]),
        };
        let c = dp::compose_drawing(&s, &loader);
        if part < 1 || part > c.parts.len() {
            return detail(StatusCode::BAD_REQUEST, &format!("Нет прохода {part}"));
        }
        if !c.errors.is_empty() {
            return refused(c.errors.clone());
        }
        let p = &c.parts[part - 1];
        let code = if test { dp::make_part_test_gcode(&c, p) } else { dp::make_part_gcode(&c, p) };
        let label = if c.parts.len() > 1 {
            format!("{}проход {part} из {}", if test { "тест, " } else { "" }, c.parts.len())
        } else if test {
            "тест рамки".into()
        } else {
            "чертёж".into()
        };
        (code, c.machine_map(p), label, part)
    } else {
        let code = if test {
            make_test_gcode(&s).map(|(g, _)| g)
        } else {
            let c = match provider(&s.font, s.mode, &s.outline) {
                Ok(p) => compose(&s, p.as_ref()),
                Err(e) => compose_without_font(&s, &e),
            };
            make_gcode(&c)
        };
        (code, dp::MachineMap::for_text(&s), if test { "тест рамки".into() } else { "лист".into() }, 0)
    };
    let code = match code {
        Ok(c) => c,
        Err(e) => return refused(e),
    };
    let lines = printer::protocol::gcode_lines(&code);
    let job = printer::Job { label, kind: kind.to_string(), part, test, lines, map, pen };
    printer_result(PRINTER.start(job))
}
