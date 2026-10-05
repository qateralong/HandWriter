use crate::geometry::Point;
use crate::numeric::{max, min, repr};
use crate::settings::Settings;

pub const Z_DOWN_MIN: f64 = -3.0;
pub const Z_UP_MIN: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TravelBox {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub measured: bool,
}

pub fn to_machine((x, y): Point, s: &Settings) -> Point {
    (if s.printer.flip_x { -x } else { x }, if s.printer.flip_y { -y } else { y })
}

pub fn travel_box(s: &Settings) -> TravelBox {
    let pr = &s.printer;
    if let Some(t) = pr.travel {
        return TravelBox { x_min: t.x_min, x_max: t.x_max, y_min: t.y_min, y_max: t.y_max, measured: true };
    }
    let (w, h) = if pr.use_work_area { (pr.work_w, pr.work_h) } else { (s.sheet.width, s.sheet.height) };
    let (x0, y0) = to_machine((0.0, 0.0), s);
    let (x1, y1) = to_machine((w, h), s);
    TravelBox { x_min: min(x0, x1), x_max: max(x0, x1), y_min: min(y0, y1), y_max: max(y0, y1), measured: false }
}

pub type Issues = (Vec<String>, Vec<String>);

pub fn check_settings(s: &Settings) -> Issues {
    let (mut e, mut w) = check_sheet(s);
    let (e2, w2) = check_printer(s);
    e.extend(e2);
    w.extend(w2);
    (e, w)
}

pub fn check_sheet(s: &Settings) -> Issues {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let (sh, ty) = (&s.sheet, &s.typography);

    if sh.width <= 0.0 || sh.height <= 0.0 {
        errors.push("Размер листа должен быть больше нуля".to_string());
    }
    if [sh.margin_left, sh.margin_right, sh.first_line_top, sh.bottom_limit, sh.indent].iter().any(|&v| v < 0.0) {
        errors.push("Поля, первая строка, нижний предел и красная строка не могут быть отрицательными".to_string());
    }
    if sh.margin_left + sh.margin_right >= sh.width {
        errors.push("Левое и правое поля вместе не меньше ширины листа".to_string());
    } else if sh.margin_left + sh.indent >= sh.width - sh.margin_right {
        errors.push("Красная строка не помещается между полями".to_string());
    }
    if sh.first_line_top >= sh.height - sh.bottom_limit + 1e-9 {
        errors.push("Первая строка ниже нижнего предела".to_string());
    }
    if sh.line_pitch <= 0.0 {
        errors.push("Шаг строк должен быть больше нуля".to_string());
    }
    if ty.size_mm <= 0.0 {
        errors.push("Размер букв должен быть больше нуля".to_string());
    } else if sh.line_pitch > 0.0 && ty.size_mm * 2.5 > sh.line_pitch {
        warnings.push(
            "Буквы крупные для этого шага строк: хвосты и заглавные могут налезать на соседние строки".to_string(),
        );
    }
    (errors, warnings)
}

pub fn check_printer(s: &Settings) -> Issues {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let pr = &s.printer;
    if pr.pen_up_z <= pr.pen_down_z {
        errors.push("pen_up_z должен быть выше pen_down_z".to_string());
    }
    if pr.pen_down_z < Z_DOWN_MIN {
        warnings.push(format!(
            "pen_down_z = {} ниже {}: карандаш сильно давит на стол",
            repr(pr.pen_down_z),
            repr(Z_DOWN_MIN)
        ));
    }
    if pr.pen_up_z < Z_UP_MIN {
        warnings.push(format!(
            "pen_up_z = {} меньше {}: карандаш может чертить на переездах",
            repr(pr.pen_up_z),
            repr(Z_UP_MIN)
        ));
    }
    if pr.pen_down_z > 0.0 {
        warnings.push("pen_down_z выше нуля: карандаш может не доставать до бумаги".to_string());
    }
    for (name, v) in [("feed_draw", pr.feed_draw), ("feed_travel", pr.feed_travel), ("feed_z", pr.feed_z)] {
        if v <= 0.0 {
            errors.push(format!("{name} должен быть больше нуля"));
        }
    }
    if pr.simplify_tol < 0.0 {
        errors.push("Допуск упрощения не может быть отрицательным".to_string());
    }
    match pr.travel {
        None => warnings.push(
            "Ход карандаша не измерен: границы взяты по рабочей зоне принтера (в «Почерке» — по размеру листа)"
                .to_string(),
        ),
        Some(t) => {
            if t.x_min >= t.x_max || t.y_min >= t.y_max {
                errors.push("Ход карандаша: минимум должен быть меньше максимума".to_string());
            } else if !(t.x_min <= 0.0 && 0.0 <= t.x_max && t.y_min <= 0.0 && 0.0 <= t.y_max) {
                errors.push("Ход карандаша должен включать ноль (угол листа, откуда стартует карандаш)".to_string());
            }
        }
    }
    if pr.safety_margin < 0.0 {
        errors.push("Запас до границ хода не может быть отрицательным".to_string());
    }
    (errors, warnings)
}

pub fn check_bounds(strokes_mm: &[Vec<Point>], s: &Settings, limit: usize) -> Vec<String> {
    let bx = travel_box(s);
    let m = s.printer.safety_margin;
    let (lo_x, hi_x) = (min(bx.x_min + m, 0.0), max(bx.x_max - m, 0.0));
    let (lo_y, hi_y) = (min(bx.y_min + m, 0.0), max(bx.y_max - m, 0.0));
    let mut bad = Vec::new();
    let mut count = 0usize;
    for p in strokes_mm.iter().flatten() {
        let (mx, my) = to_machine(*p, s);
        if !(lo_x - 1e-9 <= mx && mx <= hi_x + 1e-9 && lo_y - 1e-9 <= my && my <= hi_y + 1e-9) {
            count += 1;
            if bad.len() < limit {
                bad.push(format!("X{mx:.2} Y{my:.2}"));
            }
        }
    }
    if count == 0 {
        return Vec::new();
    }
    vec![format!(
        "{count} точек вне хода карандаша (с запасом {} мм: X {lo_x:.1}..{hi_x:.1}, Y {lo_y:.1}..{hi_y:.1} в координатах принтера), например: {}",
        repr(m),
        bad.join(", ")
    )]
}
