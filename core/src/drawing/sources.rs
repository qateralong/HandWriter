use std::fs;
use std::sync::Mutex;

use blake2::Blake2bVar;
use blake2::digest::{Update, VariableOutput};
use std::path::Path;

use crate::drawing::model::ImportResult;
use crate::drawing::svg_import::import_svg;
use crate::settings::DrawingImport;

pub const BUILTIN_TEST: &str = "builtin:test";

pub const TEST_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="160mm" height="110mm" viewBox="0 0 160 110">
  <g fill="none" stroke="#000">
    <rect x="5" y="5" width="150" height="100" stroke-width="0.7"/>
    <line x1="5" y1="105" x2="155" y2="5" stroke-width="0.25"/>
    <line x1="5" y1="5" x2="155" y2="105" stroke-width="0.25"/>
    <circle cx="80" cy="55" r="35" stroke-width="0.7"/>
    <g stroke-width="0.25" stroke-dasharray="12 3 1 3">
      <line x1="35" y1="55" x2="125" y2="55"/>
      <line x1="80" y1="12" x2="80" y2="98"/>
    </g>
  </g>
</svg>
"##;

pub fn file_kind(name: &str) -> Option<&'static str> {
    let ext = Path::new(name).extension()?.to_string_lossy().to_lowercase();
    Some(match ext.as_str() {
        "svg" => "svg",
        "dxf" => "dxf",
        "pdf" => "pdf",
        "png" | "jpg" | "jpeg" => "raster",
        _ => return None,
    })
}

pub struct DrawingFile {
    pub spec: String,
    pub label: String,
    pub kind: String,
}

pub fn list_files(drawings_dir: &Path) -> Vec<DrawingFile> {
    let mut out =
        vec![DrawingFile {
            spec: BUILTIN_TEST.into(), label: "Встроенный тестовый чертёж".into(), kind: "svg".into()
        }];
    let mut names: Vec<String> = fs::read_dir(drawings_dir)
        .map(|rd| {
            rd.flatten().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()
        })
        .unwrap_or_default();
    names.sort();
    for n in names {
        if let Some(k) = file_kind(&n) {
            out.push(DrawingFile { spec: n.clone(), label: n, kind: k.into() });
        }
    }
    out
}

fn import_key(kind: &str, imp: &DrawingImport) -> String {
    if kind == "raster" {
        return format!(
            "{:?}",
            (
                imp.threshold_auto,
                (!imp.threshold_auto).then_some(imp.threshold),
                imp.invert,
                imp.raster_dpi.to_bits(),
                imp.raster_mode
            )
        );
    }
    let common = (imp.fill_centerlines, imp.fill_centerlines.then_some(imp.fill_centerline_max.to_bits()));
    if kind == "pdf" { format!("{:?}", (imp.pdf_page, common)) } else { format!("{:?}", (imp.units, common)) }
}

const CACHE_SIZE: usize = 8;

static CACHE: Mutex<Vec<(String, ImportResult)>> = Mutex::new(Vec::new());

pub fn clear_cache() {
    CACHE.lock().expect("cache lock").clear();
}

fn import(kind: &str, data: &[u8], name: &str, imp: &DrawingImport, tol_mm: f64) -> ImportResult {
    match kind {
        "svg" => import_svg(data, name, imp, tol_mm),
        "pdf" => crate::drawing::pdf_import::import_pdf(data, name, imp, tol_mm),
        "dxf" => crate::dxf::import_dxf(data, name, imp, tol_mm),
        _ => crate::drawing::raster_import::import_raster(data, name, imp),
    }
}

pub fn load_drawing(spec: &str, imp: &DrawingImport, tol_mm: f64, drawings_dir: &Path) -> ImportResult {
    let (data, name, kind): (Vec<u8>, String, &str) = if spec == BUILTIN_TEST {
        (TEST_SVG.as_bytes().to_vec(), crate::i18n::tr("Тестовый чертёж"), "svg")
    } else {
        let file_name = Path::new(spec).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let path = drawings_dir.join(&file_name);
        let Some(kind) = file_kind(&file_name) else {
            let mut res = ImportResult::new("?", &file_name);
            res.errors.push(format!("Неизвестный тип файла: {file_name}"));
            return res;
        };
        match fs::read(&path) {
            Ok(d) => (d, file_name, kind),
            Err(_) => {
                let mut res = ImportResult::new(kind, &file_name);
                res.errors.push(format!("Файл чертежа не найден: {file_name}"));
                return res;
            }
        }
    };
    let mut h = Blake2bVar::new(16).expect("digest size");
    h.update(&data);
    let mut digest = [0u8; 16];
    h.finalize_variable(&mut digest).expect("digest");
    let tol_key =
        if kind == "raster" { "None".to_string() } else { crate::numeric::repr(crate::numeric::round_to(tol_mm, 6)) };
    let key = format!("{digest:?}|{kind}|{}|{tol_key}", import_key(kind, imp));
    {
        let mut cache = CACHE.lock().expect("cache lock");
        if let Some(i) = cache.iter().position(|(k, _)| *k == key) {
            let item = cache.remove(i);
            let res = item.1.clone();
            cache.push(item);
            return res;
        }
    }
    let mut res = import(kind, &data, &name, imp, tol_mm);
    if spec == BUILTIN_TEST {
        res.kind = "test".into();
    }
    let mut cache = CACHE.lock().expect("cache lock");
    cache.push((key, res.clone()));
    while cache.len() > CACHE_SIZE {
        cache.remove(0);
    }
    res
}
