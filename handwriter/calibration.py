from __future__ import annotations

from .checks import check_printer
from .gcode import _ascii, fmt
from .pipeline import GenerationRefused
from .settings import Settings

PREAMBLE = ["G21", "G90", "M104 S0", "M140 S0", "M420 S0", "M211 S0", "G92 X0 Y0 Z0"]


def reach_corners(s: Settings) -> list[tuple[float, float]]:
    t, m = s.printer.travel, s.printer.safety_margin
    return [(t.x_min + m, t.y_min + m), (t.x_max - m, t.y_min + m),
            (t.x_max - m, t.y_max - m), (t.x_min + m, t.y_max - m)]


def check_errors(s: Settings) -> tuple[list[str], list[str]]:
    pr = s.printer
    errors, warnings = check_printer(s)
    warnings = [w for w in warnings if "не измерен" not in w]
    if pr.travel is None:
        return errors + ["Окно достижимости не введено: сначала запиши замеры"], warnings
    t, m = pr.travel, pr.safety_margin
    if t.x_max - t.x_min <= 2 * m or t.y_max - t.y_min <= 2 * m:
        errors.append(f"Окно достижимости меньше двух запасов ({2 * m:g} мм) по одной из осей")
    for i, (x, y) in enumerate(reach_corners(s) if not errors else [], 1):
        if x < 0 or y < 0:
            warnings.append(f"Угол {i} (X{x:.1f} Y{y:.1f}) за краем листа со стороны упоров: карандаш коснётся "
                            "стола или упора — подложи туда бумагу или проверь, что там можно касаться")
    return errors, warnings


def make_reach_check_gcode(s: Settings) -> str:
    errors, _ = check_errors(s)
    if errors:
        raise GenerationRefused(errors)
    pr = s.printer
    tb = pr.table
    up, down = fmt(pr.pen_up_z), fmt(pr.pen_down_z)
    fz, ft = str(int(round(pr.feed_z))), str(int(round(pr.feed_travel)))
    t = pr.travel
    touch_ms, pause_ms = int(round(tb.touch_s * 1000)), int(round(tb.pause_s * 1000))
    lines = [
        "; HandWriter gcode",
        "; REACH CHECK: pencil visits the 4 corners of the reach window (inset by the safety margin),",
        f"; touches the paper for {tb.touch_s:g} s in each corner, pause {tb.pause_s:g} s between corners",
        f"; reach window X{fmt(t.x_min)}..{fmt(t.x_max)} Y{fmt(t.y_min)}..{fmt(t.y_max)}, "
        f"margin {fmt(pr.safety_margin)} mm",
        "; " + _ascii(f"sheet may overhang +X {int(tb.overhang_x)} +Y {int(tb.overhang_y)}"),
        f"; pen up Z{up} down Z{down}, feed travel {ft} z {fz}",
        *PREAMBLE,
        f"G0 Z{up} F{fz}",
    ]
    corners = reach_corners(s)
    for i, (x, y) in enumerate(corners, 1):
        lines += [
            f"; corner {i}",
            f"G0 X{fmt(x)} Y{fmt(y)} F{ft}",
            f"G1 Z{down} F{fz}",
            f"G4 P{touch_ms}",
            f"G0 Z{up} F{fz}",
        ]
        if i < len(corners):
            lines.append(f"G4 P{pause_ms}")
    lines += [f"G0 Z{up} F{fz}", f"G0 X0.00 Y0.00 F{ft}", "M400"]
    return "\n".join(lines) + "\n"


def make_zero_gcode(s: Settings) -> str:
    errors, _ = check_printer(s)
    errors = [e for e in errors if "Ход карандаша" not in e]
    if errors:
        raise GenerationRefused(errors)
    pr = s.printer
    up, fz = fmt(pr.pen_up_z), str(int(round(pr.feed_z)))
    lines = [
        "; HandWriter gcode",
        "; ZERO: run with the pencil touching the paper in the sheet corner at the stops.",
        "; Sets X0 Y0 Z0 here (G92), lifts the pencil, software endstops off (M211 S0).",
        *PREAMBLE,
        f"G0 Z{up} F{fz}",
        "M117 Zero set, pen up",
        "M400",
    ]
    return "\n".join(lines) + "\n"
