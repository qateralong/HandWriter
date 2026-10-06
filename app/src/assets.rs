use std::fs;
use std::io;

use crate::paths::builtin_fonts_dir;

pub struct Asset {
    pub name: &'static str,
    pub data: &'static [u8],
}

macro_rules! asset {
    ($dir:literal, $name:literal) => {
        Asset { name: $name, data: include_bytes!(concat!("../../handwriter/", $dir, "/", $name)) }
    };
}

pub const STATIC: [Asset; 9] = [
    asset!("static", "app.js"),
    asset!("static", "debug.html"),
    asset!("static", "drawing.html"),
    asset!("static", "drawing.js"),
    asset!("static", "icon.png"),
    asset!("static", "index.html"),
    asset!("static", "keepalive.js"),
    asset!("static", "outline_params.js"),
    asset!("static", "theme.js"),
];

macro_rules! ui_asset {
    ($name:literal) => {
        Asset { name: $name, data: include_bytes!(concat!("../../ui/", $name)) }
    };
}

pub const UI: [Asset; 6] = [
    ui_asset!("index.html"),
    ui_asset!("app.css"),
    ui_asset!("app.js"),
    ui_asset!("fields.js"),
    ui_asset!("printer.js"),
    ui_asset!("theme.js"),
];

pub const FONTS: [Asset; 3] = [
    asset!("fonts", "BadScript-OFL.txt"),
    asset!("fonts", "BadScript-Regular.ttf"),
    asset!("fonts", "hershey_cyrillic.svg"),
];

pub fn static_file(name: &str) -> Option<&'static [u8]> {
    STATIC.iter().find(|a| a.name == name).map(|a| a.data)
}

pub fn ui_file(name: &str) -> Option<&'static [u8]> {
    UI.iter().find(|a| a.name == name).map(|a| a.data)
}

pub fn install_fonts() -> io::Result<()> {
    let dir = builtin_fonts_dir();
    fs::create_dir_all(&dir)?;
    for a in &FONTS {
        let path = dir.join(a.name);
        if fs::read(&path).ok().as_deref() != Some(a.data) {
            let tmp = path.with_extension("tmp");
            fs::write(&tmp, a.data)?;
            fs::rename(&tmp, &path)?;
        }
    }
    Ok(())
}
