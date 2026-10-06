use std::fs;
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

pub fn load_drawing(spec: &str, imp: &DrawingImport, tol_mm: f64, drawings_dir: &Path) -> ImportResult {
    if spec == BUILTIN_TEST {
        let mut res = import_svg(TEST_SVG.as_bytes(), "Тестовый чертёж", imp, tol_mm);
        res.kind = "test".into();
        return res;
    }
    let file_name = Path::new(spec).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let path = drawings_dir.join(&file_name);
    let Some(kind) = file_kind(&file_name) else {
        let mut res = ImportResult::new("?", &file_name);
        res.errors.push(format!("Неизвестный тип файла: {file_name}"));
        return res;
    };
    let data = match fs::read(&path) {
        Ok(d) => d,
        Err(_) => {
            let mut res = ImportResult::new(kind, &file_name);
            res.errors.push(format!("Файл чертежа не найден: {file_name}"));
            return res;
        }
    };
    match kind {
        "svg" => import_svg(&data, &file_name, imp, tol_mm),
        "pdf" => crate::drawing::pdf_import::import_pdf(&data, &file_name, imp, tol_mm),
        "dxf" => crate::dxf::import_dxf(&data, &file_name, imp, tol_mm),
        "raster" => crate::drawing::raster_import::import_raster(&data, &file_name, imp),
        _ => {
            let mut res = ImportResult::new(kind, &file_name);
            res.errors.push(format!("Импорт {kind} ещё не перенесён в версию 2.0"));
            res
        }
    }
}
