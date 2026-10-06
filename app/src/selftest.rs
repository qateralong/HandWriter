use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

use handwriter_core::calibration::make_reach_check_gcode;
use handwriter_core::drawing::passes::{ROTATIONS, from_pass, to_pass};
use handwriter_core::drawing::pipeline::{compose_drawing, make_all_files};
use handwriter_core::drawing::sources;
use handwriter_core::glyphs::load_provider;
use handwriter_core::i18n::{lang, tr};
use handwriter_core::pipeline::{Composition, compose_with_font, make_gcode, make_test_gcode};
use handwriter_core::settings::{
    MissingAction, MissingChoice, Mode, OutlineOptions, Settings, Travel, load_settings, save_settings,
};
use handwriter_core::text::{Word, break_positions, normalize, process_text};
use regex::Regex;

use crate::assets::{FONTS, STATIC, UI, install_fonts};
use crate::paths::{font_paths, log_path, settings_path, user_drawings_dir};

type Check = Result<String, String>;
type CheckFn = (&'static str, fn() -> Check);

fn ensure(cond: bool, msg: impl Into<String>) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn base(font: &str, mode: Mode, text: &str) -> Settings {
    let mut s = Settings { font: font.into(), mode, text: text.into(), ..Default::default() };
    s.printer.travel = Some(Travel { x_min: -2.0, x_max: 200.0, y_min: -2.0, y_max: 230.0 });
    s
}

fn plain() -> Settings {
    base("builtin:hershey_cyrillic.svg", Mode::Strokes, "\tПроба пера: электрификация.\n\nВторой абзац.")
}

fn outline(text: &str) -> Settings {
    base("builtin:BadScript-Regular.ttf", Mode::Outlines, text)
}

fn outline_default() -> Settings {
    outline("\tСъешь же ещё этих мягких французских булок, да выпей чаю.\n\n\tМама мыла раму.")
}

fn comp(s: &Settings) -> Composition {
    compose_with_font(s, &font_paths())
}

fn word(text: &str) -> Word {
    Word {
        index: 1,
        text: text.into(),
        chars: text.chars().collect(),
        paragraph: 1,
        text_line: 1,
        soft_breaks: BTreeSet::new(),
    }
}

fn data_files() -> Check {
    install_fonts().map_err(|e| e.to_string())?;
    let dir = font_paths().builtin_dir;
    let missing: Vec<&str> = FONTS.iter().filter(|a| !dir.join(a.name).exists()).map(|a| a.name).collect();
    ensure(missing.is_empty(), format!("нет файлов: {}", missing.join(", ")))?;
    Ok(format!("{} файлов на месте", STATIC.len() + UI.len() + FONTS.len()))
}

fn hyphen_ok() -> Check {
    let got: BTreeSet<usize> = break_positions(&word("переносы"), true).into_keys().collect();
    ensure(got == BTreeSet::from([2, 4, 6]), format!("{got:?}"))?;
    Ok("ru_RU: пе-ре-но-сы".into())
}

fn shaping_ok() -> Check {
    let p = load_provider("builtin:BadScript-Regular.ttf", Mode::Outlines, &OutlineOptions::default(), &font_paths())?;
    let sh: Vec<(String, usize)> = p.shape("ffi").into_iter().map(|g| (g.name, g.cluster)).collect();
    ensure(sh == vec![("f_f_i".to_string(), 0)], format!("{sh:?}"))?;
    Ok("лигатура ffi → f_f_i".into())
}

fn skeleton_ok() -> Check {
    let p = load_provider("builtin:BadScript-Regular.ttf", Mode::Outlines, &OutlineOptions::default(), &font_paths())?;
    let name = p.glyph_names_for_char("в").into_iter().next().ok_or("нет «в»")?;
    let g = p.glyph(&name).ok_or("нет глифа")?;
    ensure(!g.strokes.is_empty() && g.strokes.iter().map(Vec::len).sum::<usize>() > 10, "мало точек")?;
    Ok(format!("«в»: {} штрих(а)", g.strokes.len()))
}

fn bounds_refuse() -> Check {
    let mut s = plain();
    s.printer.travel = Some(Travel { x_min: -2.0, x_max: 60.0, y_min: -2.0, y_max: 230.0 });
    let c = comp(&s);
    ensure(c.errors.iter().any(|e| e.contains("вне хода")), format!("{:?}", c.errors))?;
    ensure(make_gcode(&c).is_err(), "gcode создан, хотя точки вне хода")?;
    Ok("генерация отказывается".into())
}

fn z_checks() -> Check {
    let mut s = plain();
    s.printer.pen_down_z = -3.5;
    s.printer.pen_up_z = 1.5;
    let c = comp(&s);
    ensure(
        c.warnings.iter().any(|w| w.contains("pen_down_z")) && c.warnings.iter().any(|w| w.contains("pen_up_z")),
        format!("{:?}", c.warnings),
    )?;
    s.printer.pen_up_z = -4.0;
    ensure(comp(&s).errors.iter().any(|e| e.contains("pen_up_z должен быть выше")), "нет ошибки pen_up_z")?;
    Ok("предупреждения и ошибка на месте".into())
}

fn glyph_checks() -> Check {
    let mut s = plain();
    s.text = "Номер №5".into();
    ensure(comp(&s).errors.iter().any(|e| e.contains("Нет в шрифте")), "нет ошибки про глиф")?;
    s.text_options
        .missing
        .insert("№".into(), MissingChoice { action: MissingAction::Skip, replacement: String::new() });
    let errs = comp(&s).errors;
    ensure(errs.is_empty(), format!("{errs:?}"))?;
    Ok("без решения по «№» gcode не создаётся".into())
}

fn sheet_checks() -> Check {
    let mut s = plain();
    s.sheet.margin_left = s.sheet.width;
    s.sheet.margin_right = s.sheet.width;
    ensure(comp(&s).errors.iter().any(|e| e.contains("поля")), "нет ошибки про поля")?;
    Ok("несогласованные поля → ошибка".into())
}

fn gcode_format() -> Check {
    let code = make_gcode(&comp(&outline_default())).map_err(|e| e.join("; "))?;
    let body: Vec<&str> = code.lines().filter(|l| !l.is_empty() && !l.starts_with(';')).collect();
    let re = Regex::new(r"^(G0|G1|G21|G90|G92 X0 Y0 Z0|M104 S0|M140 S0|M420 S0|M211 S0|M400)\b").expect("regex");
    let bad: Vec<&&str> = body.iter().filter(|l| !re.is_match(l)).take(3).collect();
    ensure(bad.is_empty(), format!("{bad:?}"))?;
    ensure(!code.contains("G28") && !code.contains("M109") && !code.contains("M190"), "нагрев или G28")?;
    let n = body.len();
    ensure(
        n >= 2 && body[n - 1] == "M400" && body[n - 2].starts_with("G0 Z14.00") && !code.contains("G0 X0.00 Y0.00"),
        "конец файла: ожидается подъём на Z14.00 без возврата в ноль",
    )?;
    let (test, _) = make_test_gcode(&plain()).map_err(|e| e.join("; "))?;
    ensure(test.contains("G92 X0 Y0 Z0"), "тестовый файл без G92")?;
    Ok(format!("{n} команд, только G0/G1, без нагрева и G28, в конце подъём без возврата в ноль"))
}

fn determinism() -> Check {
    let a = make_gcode(&comp(&outline_default())).map_err(|e| e.join("; "))?;
    let b = make_gcode(&comp(&outline_default())).map_err(|e| e.join("; "))?;
    ensure(a == b, "gcode отличается")?;
    let mut s = outline_default();
    s.randomness.seed = 2;
    ensure(make_gcode(&comp(&s)).map_err(|e| e.join("; "))? != a, "другой seed дал тот же файл")?;
    Ok(format!("{} байт, совпадает побайтно; другой seed даёт другой файл", a.len()))
}

fn resume_tail() -> Check {
    let re = Regex::new(r"X(-?\d+\.\d\d) Y(-?\d+\.\d\d)").expect("regex");
    let xy = |code: &str| {
        let mut v: Vec<(String, String)> =
            re.captures_iter(code).map(|c| (c[1].to_string(), c[2].to_string())).collect();
        v.pop();
        v
    };
    let mut s = outline_default();
    s.connections.distance = 0.35;
    let full = make_gcode(&comp(&s)).map_err(|e| e.join("; "))?;
    let c = comp(&s);
    let lay = c.layout.as_ref().ok_or("нет раскладки")?;
    let mut target = None;
    'outer: for st in &lay.strokes {
        for k in 1..st.tags.len() {
            if st.tags[k] != st.tags[k - 1] {
                let g = &lay.glyphs[st.tags[k]];
                target = Some((g.word, g.letter));
                break 'outer;
            }
        }
    }
    let mut results = Vec::new();
    for (w, l) in target.into_iter().chain([(6, 3)]) {
        let mut s2 = s.clone();
        s2.text_options.resume_word = w;
        s2.text_options.resume_letter = l;
        let part = make_gcode(&comp(&s2)).map_err(|e| e.join("; "))?;
        let (f, p) = (xy(&full), xy(&part));
        ensure(!p.is_empty() && p.len() < f.len() && f[f.len() - p.len()..] == p[..], format!("слово {w}, буква {l}"))?;
        results.push(format!("слово {w} буква {l}"));
    }
    Ok(format!(
        "совпадает с хвостом: {}{}",
        results.join(", "),
        if target.is_some() { " (первое — от точки связки)" } else { "" }
    ))
}

fn nfc() -> Check {
    let p = load_provider("builtin:hershey_cyrillic.svg", Mode::Strokes, &OutlineOptions::default(), &font_paths())?;
    let has = |c: &str| p.has_char(c);
    let pt = process_text("йод ёж", &has, &[], &Default::default());
    let words: Vec<&str> = pt.words.iter().map(|w| w.text.as_str()).collect();
    ensure(words == ["йод", "ёж"] && pt.missing.is_empty(), format!("{words:?}"))?;
    ensure(normalize("й") == "й", "NFC")?;
    Ok("й и ё из двух кодов находятся в шрифте".into())
}

fn hyph() -> Check {
    for short in ["кот", "да", "ёж", "мир!"] {
        ensure(break_positions(&word(short), true).is_empty(), short)?;
    }
    for long in ["электрификация", "подъёму", "переносы"] {
        let n = long.chars().count();
        for b in break_positions(&word(long), true).into_keys() {
            ensure(b >= 2 && n - b >= 2, long)?;
        }
    }
    Ok("в коротких словах переносов нет, минимум 2 буквы с каждой стороны".into())
}

fn mean_size() -> Check {
    let mut s = outline(&format!("{} {}", "х".repeat(12), "х".repeat(12)));
    s.randomness.size = 10.0;
    let c = comp(&s);
    let f: Vec<f64> = c.layout.as_ref().ok_or("нет раскладки")?.glyphs.iter().map(|g| g.size).collect();
    let mean = f.iter().sum::<f64>() / f.len() as f64;
    let spread = f.iter().cloned().fold(f64::MIN, f64::max) - f.iter().cloned().fold(f64::MAX, f64::min);
    ensure((mean - 1.0).abs() < 1e-12 && spread > 0.05, format!("{mean} {spread}"))?;
    Ok(format!("средний множитель {mean:.12} при разбросе ±10%"))
}

const ST_DXF: &[u8] = include_bytes!("../../tests/golden/dxf/input/simple.dxf");
const ST_PDF: &[u8] = include_bytes!("../../tests/golden/pdf/input/shape_paths.pdf");
const ST_PNG: &[u8] = include_bytes!("../../tests/golden/raster/input/gray.png");

fn drawing_formats() -> Check {
    let d = user_drawings_dir();
    for (name, data) in [("st.dxf", ST_DXF), ("st.pdf", ST_PDF), ("st.png", ST_PNG)] {
        std::fs::write(d.join(name), data).map_err(|e| e.to_string())?;
    }
    let loader = |spec: &str, imp: &_, tol: f64| sources::load_drawing(spec, imp, tol, &d);
    let mut out = Vec::new();
    for name in ["builtin:test", "st.dxf", "st.pdf", "st.png"] {
        sources::clear_cache();
        let mut s = Settings::default();
        s.drawing.file = name.into();
        let c = compose_drawing(&s, &loader);
        let imp = c.imp.as_ref().ok_or("нет импорта")?;
        ensure(c.errors.is_empty() && !imp.paths.is_empty(), format!("{name}: {:?}", c.errors))?;
        out.push(format!("{} {}", imp.kind, imp.paths.len()));
    }
    Ok(format!("SVG, DXF, PDF (MuPDF), PNG (скелет): {}", out.join(", ")))
}

fn drawing_passes() -> Check {
    for r in ROTATIONS {
        for p in [(0.0, 0.0), (12.5, 200.0), (297.0, 210.0)] {
            ensure(from_pass(to_pass(p, r, 297.0, 210.0), r, 297.0, 210.0) == p, format!("поворот {r}"))?;
        }
    }
    let mut s = Settings::default();
    s.printer.travel = Some(Travel { x_min: -3.0, x_max: 200.0, y_min: -3.0, y_max: 215.0 });
    s.drawing.weights.enabled = true;
    s.drawing.split.marks = true;
    let d = user_drawings_dir();
    let loader = |spec: &str, imp: &_, tol: f64| sources::load_drawing(spec, imp, tol, &d);
    let c = compose_drawing(&s, &loader);
    let rots: Vec<i64> = c.parts.iter().map(|p| p.rotation).collect();
    ensure(c.errors.is_empty() && rots == [0, 180], format!("{:?} {rots:?}", c.errors))?;
    let st = c.cut_stats.as_ref().ok_or("нет статистики разрезов")?;
    ensure((st.extension - 2.0 * s.drawing.split.overlap * st.cuts as f64).abs() < 1e-6, "удлинение разрезов")?;
    let a = make_all_files(&c, true).map_err(|e| e.join("; "))?;
    let b = make_all_files(&compose_drawing(&s, &loader), true).map_err(|e| e.join("; "))?;
    ensure(a.iter().map(|f| &f.gcode).eq(b.iter().map(|f| &f.gcode)), "файлы не повторяются")?;
    for f in &a {
        ensure(f.gcode.contains("G92 X0 Y0 Z0") && !f.gcode.contains("G28") && !f.gcode.contains("M109"), "формат")?;
    }
    let seam = c.split.as_ref().and_then(|n| n.seams().first().map(|s| s.s)).unwrap_or(f64::NAN);
    Ok(format!(
        "A4 альбом: проходы 0° и 180°, шов x = {seam:.1}, разрезов {}, {} файла, побайтно повторяются",
        st.cuts,
        a.len()
    ))
}

fn calibration() -> Check {
    let mut s = Settings::default();
    s.printer.travel = Some(Travel { x_min: -3.0, x_max: 220.0, y_min: -3.0, y_max: 219.0 });
    let g = make_reach_check_gcode(&s).map_err(|e| e.join("; "))?;
    ensure(g.matches("G4 P1000").count() == 4 && g.contains("G92 X0 Y0 Z0") && !g.contains("G28"), "формат")?;
    Ok("проверочный файл: 4 угла окна, касание и паузы".into())
}

fn settings_io() -> Check {
    let s = Settings { text: "проверка".into(), ..Default::default() };
    save_settings(&s, &settings_path()).map_err(|e| e.to_string())?;
    let back = load_settings(&settings_path()).map_err(|e| e.to_string())?;
    ensure(back.text == "проверка" && settings_path().exists(), "настройки не прочитались")?;
    Ok(settings_path().parent().map(|p| p.display().to_string()).unwrap_or_default())
}

pub fn run(report: Option<PathBuf>) -> i32 {
    let report = report.unwrap_or_else(|| log_path().with_file_name("selftest.txt"));
    let tmp = std::env::temp_dir().join(format!("handwriter-selftest-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    unsafe { std::env::set_var("HANDWRITER_HOME", &tmp) };
    let mut lines = vec![format!(
        "HandWriter {} самопроверка, Rust, язык интерфейса: {}, {}",
        env!("CARGO_PKG_VERSION"),
        lang(),
        std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
    )];
    let mut ok_all = true;
    let checks: [CheckFn; 18] = [
        ("Интерфейс и встроенные шрифты в сборке", data_files),
        ("Словарь переносов ru_RU", hyphen_ok),
        ("Шейпинг с лигатурами", shaping_ok),
        ("Скелет глифа", skeleton_ok),
        ("Проверки: точка вне хода карандаша → отказ", bounds_refuse),
        ("Проверки: значения Z", z_checks),
        ("Проверки: глифы для всех символов", glyph_checks),
        ("Проверки: поля и размеры листа", sheet_checks),
        ("Gcode: формат и тестовый файл", gcode_format),
        ("Почерк: тот же seed → тот же gcode побайтно", determinism),
        ("Почерк: «продолжить с буквы» = хвост полного gcode", resume_tail),
        ("Почерк: NFC-нормализация", nfc),
        ("Почерк: переносы", hyph),
        ("Почерк: средний размер букв равен заданному", mean_size),
        ("Чертёж: форматы файлов", drawing_formats),
        ("Чертёж: разбиение на проходы и файлы", drawing_passes),
        ("Чертёж: калибровка окна достижимости", calibration),
        ("Настройки пишутся в папку пользователя", settings_io),
    ];
    for (name, f) in checks {
        let t = Instant::now();
        let r = std::panic::catch_unwind(f).unwrap_or_else(|p| {
            Err(p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default())
        });
        match r {
            Ok(detail) => lines.push(format!(
                "[OK]   {name}{} ({:.1} с)",
                if detail.is_empty() { String::new() } else { format!(" — {detail}") },
                t.elapsed().as_secs_f64()
            )),
            Err(e) => {
                ok_all = false;
                lines.push(format!("[FAIL] {name} — {e}"));
            }
        }
    }
    lines.push(format!("ИТОГ: {}", if ok_all { "всё в порядке" } else { "ЕСТЬ ОШИБКИ" }));
    let text = lines.iter().map(|l| tr(l)).collect::<Vec<_>>().join("\n") + "\n";
    if let Some(dir) = report.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&report, &text);
    print!("{text}");
    let _ = std::fs::remove_dir_all(&tmp);
    if ok_all { 0 } else { 1 }
}
