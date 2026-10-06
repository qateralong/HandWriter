use std::env;
use std::fs;
use std::path::PathBuf;

use handwriter_core::glyphs::FontPaths;

pub const APP_NAME: &str = "HandWriter";

fn home() -> PathBuf {
    env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|| ".".into())
}

fn app_dir(win_var: &str, win_fallback: &str, xdg_var: &str, xdg_fallback: &str) -> PathBuf {
    if let Some(v) = env::var_os(win_var).filter(|v| !v.is_empty()) {
        return PathBuf::from(v).join(APP_NAME);
    }
    if cfg!(windows) {
        return home().join("AppData").join(win_fallback).join(APP_NAME);
    }
    match env::var_os(xdg_var).filter(|v| !v.is_empty()) {
        Some(x) => PathBuf::from(x).join(APP_NAME),
        None => home().join(xdg_fallback).join(APP_NAME),
    }
}

fn ensure(d: PathBuf) -> PathBuf {
    let _ = fs::create_dir_all(&d);
    d
}

pub fn user_dir() -> PathBuf {
    match env::var_os("HANDWRITER_HOME").filter(|v| !v.is_empty()) {
        Some(h) => ensure(PathBuf::from(h)),
        None => ensure(app_dir("APPDATA", "Roaming", "XDG_DATA_HOME", ".local/share")),
    }
}

pub fn local_dir() -> PathBuf {
    match env::var_os("HANDWRITER_HOME").filter(|v| !v.is_empty()) {
        Some(h) => ensure(PathBuf::from(h).join("local")),
        None => ensure(app_dir("LOCALAPPDATA", "Local", "XDG_CACHE_HOME", ".cache")),
    }
}

pub fn settings_path() -> PathBuf {
    user_dir().join("settings.json")
}

pub fn logs_dir() -> PathBuf {
    ensure(user_dir().join("logs"))
}

pub fn log_path() -> PathBuf {
    logs_dir().join("handwriter.log")
}

pub fn user_fonts_dir() -> PathBuf {
    ensure(user_dir().join("fonts"))
}

pub fn user_drawings_dir() -> PathBuf {
    ensure(user_dir().join("drawings"))
}

pub fn builtin_fonts_dir() -> PathBuf {
    local_dir().join("builtin-fonts").join(env!("CARGO_PKG_VERSION"))
}

pub fn font_paths() -> FontPaths {
    FontPaths { builtin_dir: builtin_fonts_dir(), user_fonts_dir: user_fonts_dir() }
}
