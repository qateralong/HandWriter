use std::fs;
use std::io;
use std::path::Path;
use std::thread;
use std::time::Duration;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Ruling {
    #[default]
    Grid,
    Lines,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sheet {
    pub width: f64,
    pub height: f64,
    pub margin_left: f64,
    pub margin_right: f64,
    pub first_line_top: f64,
    pub bottom_limit: f64,
    pub line_pitch: f64,
    pub indent: f64,
    pub ruling: Ruling,
    pub grid_step: f64,
}

impl Default for Sheet {
    fn default() -> Self {
        Self {
            width: 165.0,
            height: 205.0,
            margin_left: 20.0,
            margin_right: 8.0,
            first_line_top: 15.0,
            bottom_limit: 10.0,
            line_pitch: 10.0,
            indent: 10.0,
            ruling: Ruling::Grid,
            grid_step: 5.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Typography {
    pub size_mm: f64,
    pub baseline_shift: f64,
    pub dx: f64,
    pub dy: f64,
    pub rotation_deg: f64,
    pub hyphenate: bool,
}

impl Default for Typography {
    fn default() -> Self {
        Self { size_mm: 3.0, baseline_shift: 0.0, dx: 0.0, dy: 0.0, rotation_deg: 0.0, hyphenate: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MissingAction {
    #[default]
    Skip,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MissingChoice {
    pub action: MissingAction,
    pub replacement: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextOptions {
    pub replacements: Vec<(String, String)>,
    pub missing: IndexMap<String, MissingChoice>,
    pub start_word: i64,
    pub resume_word: i64,
    pub resume_letter: i64,
}

impl Default for TextOptions {
    fn default() -> Self {
        let pairs = [
            ("«", "\""),
            ("»", "\""),
            ("„", "\""),
            ("“", "\""),
            ("”", "\""),
            ("—", "-"),
            ("–", "-"),
            ("…", "..."),
            ("\u{a0}", " "),
        ];
        Self {
            replacements: pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            missing: IndexMap::new(),
            start_word: 1,
            resume_word: 0,
            resume_letter: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Randomness {
    pub enabled: bool,
    pub seed: i64,
    pub size: f64,
    pub slant: f64,
    pub offset: f64,
    pub letter_spacing: f64,
    pub word_spacing: f64,
    pub drift: f64,
    pub line_start: f64,
    pub right_edge: f64,
    pub jitter: f64,
    pub variants: bool,
}

impl Default for Randomness {
    fn default() -> Self {
        Self {
            enabled: true,
            seed: 1,
            size: 3.0,
            slant: 2.0,
            offset: 0.3,
            letter_spacing: 5.0,
            word_spacing: 10.0,
            drift: 0.5,
            line_start: 1.0,
            right_edge: 1.5,
            jitter: 0.1,
            variants: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Connections {
    pub enabled: bool,
    pub distance: f64,
}

impl Default for Connections {
    fn default() -> Self {
        Self { enabled: true, distance: 0.15 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Travel {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TableSetup {
    pub overhang_x: bool,
    pub overhang_y: bool,
    pub table_x: Option<f64>,
    pub table_y: Option<f64>,
    pub touch_s: f64,
    pub pause_s: f64,
    pub readings: IndexMap<String, f64>,
}

impl Default for TableSetup {
    fn default() -> Self {
        Self {
            overhang_x: true,
            overhang_y: false,
            table_x: None,
            table_y: None,
            touch_s: 1.0,
            pause_s: 2.0,
            readings: IndexMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Printer {
    pub pen_up_z: f64,
    pub pen_down_z: f64,
    pub feed_draw: f64,
    pub feed_travel: f64,
    pub feed_z: f64,
    pub simplify_tol: f64,
    pub travel: Option<Travel>,
    pub work_w: f64,
    pub work_h: f64,
    #[serde(skip)]
    pub use_work_area: bool,
    pub safety_margin: f64,
    pub flip_x: bool,
    pub flip_y: bool,
    pub test_mark_offset: f64,
    pub table: TableSetup,
}

impl Default for Printer {
    fn default() -> Self {
        Self {
            pen_up_z: 4.0,
            pen_down_z: -1.0,
            feed_draw: 1200.0,
            feed_travel: 3000.0,
            feed_z: 600.0,
            simplify_tol: 0.05,
            travel: None,
            work_w: 220.0,
            work_h: 220.0,
            use_work_area: false,
            safety_margin: 2.0,
            flip_x: false,
            flip_y: false,
            test_mark_offset: 5.0,
            table: TableSetup::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutlineOptions {
    pub px_per_em: f64,
    pub prune: f64,
    pub extend: f64,
    pub smooth: f64,
    pub simplify: f64,
    pub junction_merge: f64,
}

impl Default for OutlineOptions {
    fn default() -> Self {
        Self { px_per_em: 1500.0, prune: 0.08, extend: 1.0, smooth: 0.03, simplify: 0.004, junction_merge: 2.5 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preview {
    pub show_travel: bool,
    pub show_ruling: bool,
}

impl Default for Preview {
    fn default() -> Self {
        Self { show_travel: true, show_ruling: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    #[default]
    Auto,
    Mm,
    Cm,
    M,
    In,
    Ft,
    Pt,
    Px,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RasterMode {
    #[default]
    Centerlines,
    Fill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FillDir {
    #[default]
    Auto,
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawingImport {
    pub units: Units,
    pub pdf_page: i64,
    pub threshold_auto: bool,
    pub threshold: i64,
    pub invert: bool,
    pub raster_dpi: f64,
    pub fill_centerlines: bool,
    pub fill_centerline_max: f64,
    pub raster_mode: RasterMode,
    pub fill_step: f64,
    pub fill_dir: FillDir,
}

impl Default for DrawingImport {
    fn default() -> Self {
        Self {
            units: Units::Auto,
            pdf_page: 1,
            threshold_auto: true,
            threshold: 128,
            invert: false,
            raster_dpi: 0.0,
            fill_centerlines: false,
            fill_centerline_max: 5.0,
            raster_mode: RasterMode::Centerlines,
            fill_step: 0.4,
            fill_dir: FillDir::Auto,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SheetFormat {
    #[default]
    A4,
    A3,
    #[serde(rename = "custom")]
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    #[default]
    Auto,
    Portrait,
    Landscape,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawingSheet {
    pub format: SheetFormat,
    pub width: f64,
    pub height: f64,
    pub orientation: Orientation,
}

impl Default for DrawingSheet {
    fn default() -> Self {
        Self { format: SheetFormat::A4, width: 210.0, height: 297.0, orientation: Orientation::Auto }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScaleMode {
    #[default]
    Fit,
    FitReach,
    FitPasses,
    OneToOne,
    Percent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Anchor {
    #[default]
    Center,
    Zero,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawingPlacement {
    pub scale_mode: ScaleMode,
    pub percent: f64,
    pub margin: f64,
    pub dx: f64,
    pub dy: f64,
    pub anchor: Anchor,
}

impl Default for DrawingPlacement {
    fn default() -> Self {
        Self { scale_mode: ScaleMode::Fit, percent: 100.0, margin: 10.0, dx: 0.0, dy: 0.0, anchor: Anchor::Center }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Frame {
    pub enabled: bool,
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
    pub title_block: bool,
    pub tb_width: f64,
    pub tb_height: f64,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            enabled: false,
            left: 20.0,
            right: 5.0,
            top: 5.0,
            bottom: 5.0,
            title_block: true,
            tb_width: 185.0,
            tb_height: 55.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LayerWeight {
    #[default]
    Auto,
    Thin,
    Thick,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LineWeights {
    pub enabled: bool,
    pub threshold: f64,
    pub passes: i64,
    pub step: f64,
    pub layers: IndexMap<String, LayerWeight>,
}

impl Default for LineWeights {
    fn default() -> Self {
        Self { enabled: false, threshold: 0.4, passes: 3, step: 0.15, layers: IndexMap::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawingPaths {
    pub curve_tol: f64,
    pub join_tol: f64,
    pub long_path: f64,
}

impl Default for DrawingPaths {
    fn default() -> Self {
        Self { curve_tol: 0.05, join_tol: 0.05, long_path: 30.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawingSplit {
    pub overlap: f64,
    pub slack: f64,
    pub marks: bool,
    pub mark_size: f64,
    pub mark_count: i64,
    pub offsets: IndexMap<String, (f64, f64)>,
    pub areas: i64,
}

impl Default for DrawingSplit {
    fn default() -> Self {
        Self {
            overlap: 0.5,
            slack: 1.0,
            marks: false,
            mark_size: 3.0,
            mark_count: 3,
            offsets: IndexMap::new(),
            areas: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MarkedSheet {
    pub enabled: bool,
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub length: f64,
}

impl Default for MarkedSheet {
    fn default() -> Self {
        Self { enabled: false, x_min: 12.0, x_max: 209.0, y_min: 0.0, y_max: 189.0, length: 297.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct A3Sheet {
    pub enabled: bool,
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
}

impl Default for A3Sheet {
    fn default() -> Self {
        Self { enabled: false, x_min: 0.0, x_max: 239.0, y_min: 0.0, y_max: 190.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawingSettings {
    pub file: String,
    pub imp: DrawingImport,
    pub sheet: DrawingSheet,
    pub placement: DrawingPlacement,
    pub frame: Frame,
    pub weights: LineWeights,
    pub paths: DrawingPaths,
    pub split: DrawingSplit,
    pub marked: MarkedSheet,
    pub a3: A3Sheet,
    pub show_travel: bool,
}

impl Default for DrawingSettings {
    fn default() -> Self {
        Self {
            file: "builtin:test".into(),
            imp: DrawingImport::default(),
            sheet: DrawingSheet::default(),
            placement: DrawingPlacement::default(),
            frame: Frame::default(),
            weights: LineWeights::default(),
            paths: DrawingPaths::default(),
            split: DrawingSplit::default(),
            marked: MarkedSheet::default(),
            a3: A3Sheet::default(),
            show_travel: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Strokes,
    Outlines,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub sheet: Sheet,
    #[serde(default = "default_size_mm")]
    pub size_mm: f64,
    #[serde(default)]
    pub baseline_shift: f64,
}

fn default_size_mm() -> f64 {
    3.0
}

pub const PROFILE_GRID: &str = "Тетрадь в клетку";
pub const PROFILE_LINES: &str = "Тетрадь в линейку";

pub fn default_profiles() -> IndexMap<String, Profile> {
    let mut m = IndexMap::new();
    m.insert(PROFILE_GRID.into(), Profile { sheet: Sheet::default(), size_mm: 3.0, baseline_shift: 0.0 });
    m.insert(
        PROFILE_LINES.into(),
        Profile {
            sheet: Sheet {
                first_line_top: 16.0,
                line_pitch: 8.0,
                ruling: Ruling::Lines,
                grid_step: 8.0,
                ..Sheet::default()
            },
            size_mm: 2.5,
            baseline_shift: 0.3,
        },
    );
    m.insert(
        "A4".into(),
        Profile {
            sheet: Sheet {
                width: 210.0,
                height: 297.0,
                margin_left: 25.0,
                margin_right: 15.0,
                first_line_top: 25.0,
                bottom_limit: 20.0,
                line_pitch: 9.0,
                indent: 12.5,
                ruling: Ruling::None,
                grid_step: 5.0,
            },
            size_mm: 3.0,
            baseline_shift: 0.0,
        },
    );
    m
}

pub const SAMPLE_TEXT: &str = "\tСъешь же ещё этих мягких французских булок, да выпей чаю. \
Широкая электрификация южных губерний даст мощный толчок подъёму сельского хозяйства.\n\
\n\
\tВторой абзац после пропущенной строки.";

pub const DEFAULT_FONT: &str = "builtin:hershey_cyrillic.svg";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: i64,
    pub font: String,
    pub mode: Mode,
    pub text: String,
    pub active_profile: String,
    pub sheet: Sheet,
    pub typography: Typography,
    pub text_options: TextOptions,
    pub printer: Printer,
    pub outline: OutlineOptions,
    pub randomness: Randomness,
    pub connections: Connections,
    pub preview: Preview,
    pub profiles: IndexMap<String, Profile>,
    pub drawing: DrawingSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            font: DEFAULT_FONT.into(),
            mode: Mode::Strokes,
            text: SAMPLE_TEXT.into(),
            active_profile: PROFILE_GRID.into(),
            sheet: Sheet::default(),
            typography: Typography::default(),
            text_options: TextOptions::default(),
            printer: Printer::default(),
            outline: OutlineOptions::default(),
            randomness: Randomness::default(),
            connections: Connections::default(),
            preview: Preview::default(),
            profiles: default_profiles(),
            drawing: DrawingSettings::default(),
        }
    }
}

#[derive(Clone, Copy)]
enum Lo {
    Ge(f64),
    Gt(f64),
}

fn check_range(errors: &mut Vec<String>, name: &str, v: f64, lo: Lo, hi: Option<f64>) {
    let ok_lo = match lo {
        Lo::Ge(a) => v >= a,
        Lo::Gt(a) => v > a,
    };
    let ok_hi = hi.is_none_or(|b| v <= b);
    if !(ok_lo && ok_hi) {
        errors.push(name.to_string());
    }
}

impl Settings {
    pub fn invalid_fields(&self) -> Vec<String> {
        use Lo::{Ge, Gt};
        let mut e = Vec::new();
        let mut r = |name: &str, v: f64, lo: Lo, hi: Option<f64>| check_range(&mut e, name, v, lo, hi);

        let rn = &self.randomness;
        r("randomness.size", rn.size, Ge(0.0), Some(30.0));
        r("randomness.slant", rn.slant, Ge(0.0), Some(20.0));
        r("randomness.offset", rn.offset, Ge(0.0), Some(3.0));
        r("randomness.letter_spacing", rn.letter_spacing, Ge(0.0), Some(50.0));
        r("randomness.word_spacing", rn.word_spacing, Ge(0.0), Some(100.0));
        r("randomness.drift", rn.drift, Ge(0.0), Some(5.0));
        r("randomness.line_start", rn.line_start, Ge(0.0), Some(10.0));
        r("randomness.right_edge", rn.right_edge, Ge(0.0), Some(20.0));
        r("randomness.jitter", rn.jitter, Ge(0.0), Some(1.0));
        r("connections.distance", self.connections.distance, Ge(0.0), Some(1.0));
        r("text_options.resume_word", self.text_options.resume_word as f64, Ge(0.0), None);
        r("text_options.resume_letter", self.text_options.resume_letter as f64, Ge(1.0), None);

        let pr = &self.printer;
        r("printer.table.touch_s", pr.table.touch_s, Ge(0.0), Some(30.0));
        r("printer.table.pause_s", pr.table.pause_s, Ge(0.0), Some(60.0));
        r("printer.work_w", pr.work_w, Gt(0.0), Some(2000.0));
        r("printer.work_h", pr.work_h, Gt(0.0), Some(2000.0));

        let o = &self.outline;
        r("outline.px_per_em", o.px_per_em, Ge(200.0), Some(4000.0));
        r("outline.prune", o.prune, Ge(0.0), Some(1.0));
        r("outline.extend", o.extend, Ge(0.0), Some(3.0));
        r("outline.smooth", o.smooth, Ge(0.0), Some(0.5));
        r("outline.simplify", o.simplify, Ge(0.0), Some(0.1));
        r("outline.junction_merge", o.junction_merge, Ge(0.0), Some(10.0));

        let d = &self.drawing;
        r("drawing.imp.pdf_page", d.imp.pdf_page as f64, Ge(1.0), None);
        r("drawing.imp.threshold", d.imp.threshold as f64, Ge(1.0), Some(254.0));
        r("drawing.imp.raster_dpi", d.imp.raster_dpi, Ge(0.0), Some(4800.0));
        r("drawing.imp.fill_centerline_max", d.imp.fill_centerline_max, Gt(0.0), Some(100.0));
        r("drawing.imp.fill_step", d.imp.fill_step, Ge(0.05), Some(5.0));
        r("drawing.sheet.width", d.sheet.width, Gt(0.0), Some(2000.0));
        r("drawing.sheet.height", d.sheet.height, Gt(0.0), Some(2000.0));
        r("drawing.placement.percent", d.placement.percent, Gt(0.0), Some(10000.0));
        r("drawing.placement.margin", d.placement.margin, Ge(0.0), Some(200.0));
        r("drawing.frame.left", d.frame.left, Ge(0.0), Some(100.0));
        r("drawing.frame.right", d.frame.right, Ge(0.0), Some(100.0));
        r("drawing.frame.top", d.frame.top, Ge(0.0), Some(100.0));
        r("drawing.frame.bottom", d.frame.bottom, Ge(0.0), Some(100.0));
        r("drawing.frame.tb_width", d.frame.tb_width, Gt(0.0), Some(400.0));
        r("drawing.frame.tb_height", d.frame.tb_height, Gt(0.0), Some(200.0));
        r("drawing.weights.threshold", d.weights.threshold, Ge(0.0), Some(10.0));
        r("drawing.weights.passes", d.weights.passes as f64, Ge(1.0), Some(9.0));
        r("drawing.weights.step", d.weights.step, Ge(0.01), Some(2.0));
        r("drawing.paths.curve_tol", d.paths.curve_tol, Ge(0.005), Some(1.0));
        r("drawing.paths.join_tol", d.paths.join_tol, Ge(0.0), Some(2.0));
        r("drawing.paths.long_path", d.paths.long_path, Ge(0.0), Some(10000.0));
        r("drawing.split.overlap", d.split.overlap, Ge(0.0), Some(5.0));
        r("drawing.split.slack", d.split.slack, Ge(0.0), Some(10.0));
        r("drawing.split.mark_size", d.split.mark_size, Gt(0.5), Some(20.0));
        r("drawing.split.mark_count", d.split.mark_count as f64, Ge(1.0), Some(10.0));
        r("drawing.split.areas", d.split.areas as f64, Ge(0.0), Some(4.0));
        r("drawing.marked.length", d.marked.length, Gt(0.0), Some(2000.0));
        e
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let s: Settings = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let bad = s.invalid_fields();
        if bad.is_empty() { Ok(s) } else { Err(format!("out of range: {}", bad.join(", "))) }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("settings serialize")
    }
}

fn retry<T>(mut f: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let attempts = 20;
    for _ in 1..attempts {
        match f() {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => thread::sleep(Duration::from_millis(20)),
            other => return other,
        }
    }
    f()
}

pub fn load_settings(path: &Path) -> io::Result<Settings> {
    let text = match retry(|| fs::read_to_string(path)) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => return Err(e),
    };
    match Settings::from_json(&text) {
        Ok(s) => Ok(s),
        Err(_) => {
            retry(|| fs::rename(path, path.with_extension("broken.json")))?;
            Ok(Settings::default())
        }
    }
}

pub fn save_settings(s: &Settings, path: &Path) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, s.to_json())?;
    retry(|| fs::rename(&tmp, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_missing_broken_and_saved_files() {
        let dir = std::env::temp_dir().join(format!("hw-settings-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let _ = fs::remove_file(&path);
        assert_eq!(load_settings(&path).unwrap(), Settings::default());

        let mut s = Settings::default();
        s.printer.pen_up_z = 5.5;
        s.text = "x".into();
        save_settings(&s, &path).unwrap();
        assert!(!dir.join("settings.tmp").exists());
        assert_eq!(load_settings(&path).unwrap(), s);

        fs::write(&path, r#"{"randomness": {"size": 31}}"#).unwrap();
        assert_eq!(load_settings(&path).unwrap(), Settings::default());
        assert!(!path.exists());
        assert!(dir.join("settings.broken.json").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
