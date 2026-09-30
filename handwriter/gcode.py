from __future__ import annotations

import math
from dataclasses import dataclass

from .checks import to_machine, travel_box
from .geometry import make_transform, polyline_length
from .settings import Settings

Point = tuple[float, float]


def fmt(v: float) -> str:
    s = f"{v:.2f}"
    return "0.00" if s == "-0.00" else s


def _feed(v: float) -> str:
    return str(int(round(v)))


def _round_stroke(points: list[Point], s: Settings) -> list[tuple[str, str]]:
    out: list[tuple[str, str]] = []
    for p in points:
        mx, my = to_machine(p, s)
        q = (fmt(mx), fmt(my))
        if not out or out[-1] != q:
            out.append(q)
    return out


@dataclass
class Stats:
    draw_mm: float
    travel_mm: float
    strokes: int
    lifts: int
    time_s: float

    def as_dict(self) -> dict:
        return {"draw_mm": round(self.draw_mm, 1), "travel_mm": round(self.travel_mm, 1),
                "strokes": self.strokes, "lifts": self.lifts, "time_s": round(self.time_s)}


def travel_moves(strokes: list[list[Point]]) -> list[tuple[Point, Point]]:
    moves = []
    cur = (0.0, 0.0)
    for st in strokes:
        if not st:
            continue
        if st[0] != cur:
            moves.append((cur, st[0]))
        cur = st[-1]
    if cur != (0.0, 0.0):
        moves.append((cur, (0.0, 0.0)))
    return moves


def compute_stats(strokes: list[list[Point]], s: Settings) -> Stats:
    pr = s.printer
    strokes = [st for st in strokes if st]
    draw = sum(polyline_length(st) for st in strokes)
    travel = sum(math.dist(a, b) for a, b in travel_moves(strokes))
    dz = abs(pr.pen_up_z - pr.pen_down_z)
    t = 0.0
    if pr.feed_draw > 0 and pr.feed_travel > 0 and pr.feed_z > 0:
        t = (draw / pr.feed_draw + travel / pr.feed_travel + 2 * dz * len(strokes) / pr.feed_z
             + 2 * abs(pr.pen_up_z) / pr.feed_z) * 60
    return Stats(draw_mm=draw, travel_mm=travel, strokes=len(strokes), lifts=len(strokes), time_s=t)


def generate_gcode(strokes: list[list[Point]], s: Settings, header: list[str] | None = None,
                   info: list[str] | None = None) -> str:
    pr = s.printer
    up, down = fmt(pr.pen_up_z), fmt(pr.pen_down_z)
    fz, fd, ft = _feed(pr.feed_z), _feed(pr.feed_draw), _feed(pr.feed_travel)
    box = travel_box(s)
    st = compute_stats(strokes, s)
    lines = ["; HandWriter gcode"]
    for h in header or []:
        lines.append("; " + _ascii(h))
    sh, ty = s.sheet, s.typography
    if info is None:
        info = [
            f"sheet {fmt(sh.width)}x{fmt(sh.height)} mm, margins L{fmt(sh.margin_left)} R{fmt(sh.margin_right)}, "
            f"first line {fmt(sh.first_line_top)}, bottom {fmt(sh.bottom_limit)}, pitch {fmt(sh.line_pitch)}",
            f"size {fmt(ty.size_mm)} mm, baseline shift {fmt(ty.baseline_shift)}, dx {fmt(ty.dx)} dy {fmt(ty.dy)}, "
            f"rotation {fmt(ty.rotation_deg)} deg",
        ]
    lines += ["; " + _ascii(i) for i in info]
    lines += [
        f"; pen up Z{up} down Z{down}, feed draw {fd} travel {ft} z {fz}, simplify {fmt(pr.simplify_tol)}",
        f"; travel X{fmt(box.x_min)}..{fmt(box.x_max)} Y{fmt(box.y_min)}..{fmt(box.y_max)}"
        + ("" if box.measured else " (NOT MEASURED, " + ("printer work area" if pr._use_work_area else "sheet size")
                                     + " used)")
        + f", flip_x {int(pr.flip_x)} flip_y {int(pr.flip_y)}",
        f"; strokes {st.strokes}, draw {st.draw_mm:.0f} mm, travel {st.travel_mm:.0f} mm, est {st.time_s / 60:.1f} min",
        "G21",
        "G90",
        "M104 S0",
        "M140 S0",
        "M420 S0",
        "M211 S0",
        "G92 X0 Y0 Z0",
        f"G0 Z{up} F{fz}",
    ]
    for stroke in strokes:
        pts = _round_stroke(stroke, s)
        if not pts:
            continue
        x, y = pts[0]
        lines.append(f"G0 X{x} Y{y} F{ft}")
        lines.append(f"G1 Z{down} F{fz}")
        for i, (x, y) in enumerate(pts[1:]):
            lines.append(f"G1 X{x} Y{y}" + (f" F{fd}" if i == 0 else ""))
        lines.append(f"G0 Z{up} F{fz}")
    lines += [
        f"G0 Z{up} F{fz}",
        f"G0 X0.00 Y0.00 F{ft}",
        "M400",
    ]
    return "\n".join(lines) + "\n"


def _ascii(text: str) -> str:
    return text.encode("ascii", "backslashreplace").decode("ascii")


def test_pattern(s: Settings) -> list[list[Point]]:
    from .layout import line_baselines

    sh, ty = s.sheet, s.typography
    t = make_transform(ty.rotation_deg, ty.dx, ty.dy)
    x0, x1 = sh.margin_left, sh.width - sh.margin_right
    bases = line_baselines(s)
    y_top = sh.height - sh.first_line_top
    y_bot = bases[-1] if bases else sh.bottom_limit
    rect = [(x0, y_bot), (x1, y_bot), (x1, y_top), (x0, y_top), (x0, y_bot)]
    out = [[t(p) for p in rect]]
    for b in bases[1:-1]:
        y = b + ty.baseline_shift
        out.append([t((x0, y)), t((x0 + 3.0, y))])

    o = s.printer.test_mark_offset
    ax, ay = 40.0, 20.0
    head = 3.0
    out.append([(o, o), (o + ax, o)])
    out.append([(o + ax - head, o + head * 0.6), (o + ax, o), (o + ax - head, o - head * 0.6)])
    out.append([(o, o), (o, o + ay)])
    out.append([(o - head * 0.6, o + ay - head), (o, o + ay), (o + head * 0.6, o + ay - head)])
    return out
