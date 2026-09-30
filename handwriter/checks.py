from __future__ import annotations

from dataclasses import dataclass

from .settings import Settings

Z_DOWN_MIN = -3.0
Z_UP_MIN = 2.0


@dataclass
class TravelBox:
    x_min: float
    x_max: float
    y_min: float
    y_max: float
    measured: bool


def to_machine(p: tuple[float, float], s: Settings) -> tuple[float, float]:
    x, y = p
    return (-x if s.printer.flip_x else x, -y if s.printer.flip_y else y)


def travel_box(s: Settings) -> TravelBox:
    t = s.printer.travel
    if t is not None:
        return TravelBox(t.x_min, t.x_max, t.y_min, t.y_max, True)
    x0, y0 = to_machine((0.0, 0.0), s)
    x1, y1 = to_machine((s.sheet.width, s.sheet.height), s)
    return TravelBox(min(x0, x1), max(x0, x1), min(y0, y1), max(y0, y1), False)


def check_settings(s: Settings) -> tuple[list[str], list[str]]:
    e1, w1 = check_sheet(s)
    e2, w2 = check_printer(s)
    return e1 + e2, w1 + w2


def check_sheet(s: Settings) -> tuple[list[str], list[str]]:
    errors: list[str] = []
    warnings: list[str] = []
    sh, ty = s.sheet, s.typography

    if sh.width <= 0 or sh.height <= 0:
        errors.append("Размер листа должен быть больше нуля")
    if min(sh.margin_left, sh.margin_right, sh.first_line_top, sh.bottom_limit, sh.indent) < 0:
        errors.append("Поля, первая строка, нижний предел и красная строка не могут быть отрицательными")
    if sh.margin_left + sh.margin_right >= sh.width:
        errors.append("Левое и правое поля вместе не меньше ширины листа")
    elif sh.margin_left + sh.indent >= sh.width - sh.margin_right:
        errors.append("Красная строка не помещается между полями")
    if sh.first_line_top >= sh.height - sh.bottom_limit + 1e-9:
        errors.append("Первая строка ниже нижнего предела")
    if sh.line_pitch <= 0:
        errors.append("Шаг строк должен быть больше нуля")
    if ty.size_mm <= 0:
        errors.append("Размер букв должен быть больше нуля")
    elif sh.line_pitch > 0 and ty.size_mm * 2.5 > sh.line_pitch:
        warnings.append("Буквы крупные для этого шага строк: хвосты и заглавные могут налезать на соседние строки")
    return errors, warnings


def check_printer(s: Settings) -> tuple[list[str], list[str]]:
    errors: list[str] = []
    warnings: list[str] = []
    pr = s.printer
    if pr.pen_up_z <= pr.pen_down_z:
        errors.append("pen_up_z должен быть выше pen_down_z")
    if pr.pen_down_z < Z_DOWN_MIN:
        warnings.append(f"pen_down_z = {pr.pen_down_z} ниже {Z_DOWN_MIN}: карандаш сильно давит на стол")
    if pr.pen_up_z < Z_UP_MIN:
        warnings.append(f"pen_up_z = {pr.pen_up_z} меньше {Z_UP_MIN}: карандаш может чертить на переездах")
    if pr.pen_down_z > 0:
        warnings.append("pen_down_z выше нуля: карандаш может не доставать до бумаги")
    for name in ("feed_draw", "feed_travel", "feed_z"):
        if getattr(pr, name) <= 0:
            errors.append(f"{name} должен быть больше нуля")
    if pr.simplify_tol < 0:
        errors.append("Допуск упрощения не может быть отрицательным")

    if pr.travel is None:
        warnings.append("Ход карандаша не измерен: границы взяты по размеру листа")
    else:
        t = pr.travel
        if t.x_min >= t.x_max or t.y_min >= t.y_max:
            errors.append("Ход карандаша: минимум должен быть меньше максимума")
        elif not (t.x_min <= 0 <= t.x_max and t.y_min <= 0 <= t.y_max):
            errors.append("Ход карандаша должен включать ноль (угол листа, откуда стартует карандаш)")
    if pr.safety_margin < 0:
        errors.append("Запас до границ хода не может быть отрицательным")
    return errors, warnings


def check_bounds(strokes_mm, s: Settings, limit: int = 5) -> list[str]:
    box = travel_box(s)
    m = s.printer.safety_margin
    lo_x, hi_x = box.x_min + m, box.x_max - m
    lo_y, hi_y = box.y_min + m, box.y_max - m
    bad = []
    count = 0
    for st in strokes_mm:
        for p in st:
            mx, my = to_machine(p, s)
            if not (lo_x - 1e-9 <= mx <= hi_x + 1e-9 and lo_y - 1e-9 <= my <= hi_y + 1e-9):
                count += 1
                if len(bad) < limit:
                    bad.append(f"X{mx:.2f} Y{my:.2f}")
    if not count:
        return []
    return [f"{count} точек вне хода карандаша (с запасом {m} мм: X {lo_x:.1f}..{hi_x:.1f}, "
            f"Y {lo_y:.1f}..{hi_y:.1f} в координатах принтера), например: " + ", ".join(bad)]
