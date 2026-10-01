from __future__ import annotations

import itertools
import math
from dataclasses import dataclass, field

import numpy as np

Point = tuple[float, float]
Rect = tuple[float, float, float, float]

ROTATIONS = (0, 90, 180, 270)
CORNER = {0: "A", 90: "D", 180: "C", 270: "B"}
CORNER_NAME = {"A": "левый нижний", "B": "правый нижний", "C": "правый верхний", "D": "левый верхний"}
OVERHANG_TOL = 0.5
EPS = 1e-6


def pass_dims(r: int, W: float, H: float) -> tuple[float, float]:
    return (W, H) if r in (0, 180) else (H, W)


def to_pass(p: Point, r: int, W: float, H: float) -> Point:
    u, v = p
    if r == 0:
        return (u, v)
    if r == 90:
        return (H - v, u)
    if r == 180:
        return (W - u, H - v)
    if r == 270:
        return (v, W - u)
    raise ValueError(f"поворот {r}")


def from_pass(q: Point, r: int, W: float, H: float) -> Point:
    x, y = q
    if r == 0:
        return (x, y)
    if r == 90:
        return (y, H - x)
    if r == 180:
        return (W - x, H - y)
    if r == 270:
        return (W - y, x)
    raise ValueError(f"поворот {r}")


def corner_point(r: int, W: float, H: float) -> Point:
    return from_pass((0.0, 0.0), r, W, H)


def rect_from_pass(R: Rect, r: int, W: float, H: float) -> Rect:
    a = from_pass((R[0], R[1]), r, W, H)
    b = from_pass((R[2], R[3]), r, W, H)
    return (min(a[0], b[0]), min(a[1], b[1]), max(a[0], b[0]), max(a[1], b[1]))


def intersect(a: Rect, b: Rect) -> Rect | None:
    r = (max(a[0], b[0]), max(a[1], b[1]), min(a[2], b[2]), min(a[3], b[3]))
    return r if r[2] > r[0] + EPS and r[3] > r[1] + EPS else None


@dataclass
class Table:
    raw: Rect
    safe: Rect
    measured: bool
    allow_x: bool
    allow_y: bool
    table_x: float
    table_y: float
    table_given: bool


def make_table(printer, W: float, H: float, r: int = 0) -> Table:
    t = printer.travel
    m = printer.safety_margin
    if t is None:
        raw = (0.0, 0.0, float(printer.work_w), float(printer.work_h))
        measured = False
    else:
        fx = -1.0 if printer.flip_x else 1.0
        fy = -1.0 if printer.flip_y else 1.0
        xs = sorted((t.x_min * fx, t.x_max * fx))
        ys = sorted((t.y_min * fy, t.y_max * fy))
        raw = (xs[0], ys[0], xs[1], ys[1])
        measured = True
    tb = printer.table
    given = tb.table_x is not None and tb.table_y is not None
    safe = (min(raw[0] + m, 0.0), min(raw[1] + m, 0.0), max(raw[2] - m, 0.0), max(raw[3] - m, 0.0))
    return Table(raw, safe, measured,
                 tb.overhang_x, tb.overhang_y,
                 tb.table_x if tb.table_x is not None else raw[2],
                 tb.table_y if tb.table_y is not None else raw[3], given)


def overhang(r: int, W: float, H: float, table: Table) -> tuple[float, float]:
    Wp, Hp = pass_dims(r, W, H)
    return max(0.0, Wp - table.table_x), max(0.0, Hp - table.table_y)


def rotation_allowed(r: int, W: float, H: float, table: Table) -> tuple[bool, str]:
    if not table.table_given:
        return True, ""
    ox, oy = overhang(r, W, H, table)
    bad = []
    if ox > OVERHANG_TOL and not table.allow_x:
        bad.append(f"выступает за стол на {ox:.0f} мм в +X")
    if oy > OVERHANG_TOL and not table.allow_y:
        bad.append(f"выступает за стол на {oy:.0f} мм в +Y")
    return (not bad), " и ".join(bad)


def pass_rect(r: int, W: float, H: float, safe: Rect) -> Rect | None:
    Wp, Hp = pass_dims(r, W, H)
    R = intersect(safe, (0.0, 0.0, Wp, Hp))
    return rect_from_pass(R, r, W, H) if R else None


def uncovered(target: Rect, rects: list[Rect]) -> list[Rect]:
    rects = [r for r in (intersect(target, q) for q in rects) if r]
    xs = sorted({target[0], target[2], *[v for r in rects for v in (r[0], r[2])]})
    ys = sorted({target[1], target[3], *[v for r in rects for v in (r[1], r[3])]})
    out: list[Rect] = []
    for y0, y1 in zip(ys, ys[1:]):
        if y1 - y0 <= EPS:
            continue
        cy = (y0 + y1) / 2
        run = None
        for x0, x1 in zip(xs, xs[1:]):
            if x1 - x0 <= EPS:
                continue
            cx = (x0 + x1) / 2
            hit = any(r[0] <= cx <= r[2] and r[1] <= cy <= r[3] for r in rects)
            if hit:
                if run:
                    out.append(run)
                    run = None
            else:
                run = (run[0], y0, x1, y1) if run else (x0, y0, x1, y1)
        if run:
            out.append(run)
    return out


def area(rects: list[Rect]) -> float:
    return sum((r[2] - r[0]) * (r[3] - r[1]) for r in rects)


def segments(strokes: list[list[Point]]) -> tuple[np.ndarray, np.ndarray]:
    A, B = [], []
    for st in strokes:
        if len(st) == 1:
            A.append(st[0]); B.append(st[0])
        for a, b in zip(st, st[1:]):
            A.append(a); B.append(b)
    if not A:
        return np.zeros((0, 2)), np.zeros((0, 2))
    return np.asarray(A, dtype=float), np.asarray(B, dtype=float)


def covered_mask(A: np.ndarray, B: np.ndarray, rects: list[Rect], eps: float = 1e-7) -> np.ndarray:
    n = len(A)
    if n == 0:
        return np.zeros(0, dtype=bool)
    if not rects:
        return np.zeros(n, dtype=bool)
    D = B - A
    T0, T1 = [], []
    with np.errstate(divide="ignore", invalid="ignore"):
        for x0, y0, x1, y1 in rects:
            t0 = np.zeros(n)
            t1 = np.ones(n)
            valid = np.ones(n, dtype=bool)
            for p, q in ((-D[:, 0], A[:, 0] - x0), (D[:, 0], x1 - A[:, 0]),
                         (-D[:, 1], A[:, 1] - y0), (D[:, 1], y1 - A[:, 1])):
                q = q + eps
                zero = np.abs(p) < 1e-15
                valid &= ~(zero & (q < 0))
                r = np.where(zero, 0.0, q / np.where(zero, 1.0, p))
                t0 = np.where((p < 0) & ~zero, np.maximum(t0, r), t0)
                t1 = np.where((p > 0) & ~zero, np.minimum(t1, r), t1)
            valid &= t0 <= t1 + 1e-12
            T0.append(np.where(valid, t0, np.inf))
            T1.append(np.where(valid, t1, -np.inf))
    T0, T1 = np.stack(T0, axis=1), np.stack(T1, axis=1)
    order = np.argsort(T0, axis=1, kind="stable")
    T0 = np.take_along_axis(T0, order, axis=1)
    T1 = np.take_along_axis(T1, order, axis=1)
    reach = np.zeros(n)
    for k in range(T0.shape[1]):
        ok = T0[:, k] <= reach + 1e-9
        reach = np.where(ok, np.maximum(reach, T1[:, k]), reach)
    return reach >= 1 - 1e-9


def lines_covered(strokes: list[list[Point]], rects: list[Rect]) -> bool:
    A, B = segments(strokes)
    return bool(covered_mask(A, B, rects).all())


@dataclass
class Plan:
    W: float
    H: float
    allowed: list[int]
    rects: dict[int, Rect]
    rotations: list[int] | None = None
    uncovered: list[Rect] = field(default_factory=list)
    message: str = ""
    notes: list[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return self.rotations is not None

    def describe(self, rotations: list[int] | None = None) -> str:
        rs = self.rotations if rotations is None else rotations
        return "; ".join(f"{i + 1}) {r}°, в упоре угол {CORNER[r]} ({CORNER_NAME[CORNER[r]]})"
                         for i, r in enumerate(rs or []))


def rotation_rects(W: float, H: float, printer) -> tuple[list[int], dict[int, Rect], list[str]]:
    allowed, rects, notes = [], {}, []
    for r in ROTATIONS:
        tb = make_table(printer, W, H, r)
        ok, why = rotation_allowed(r, W, H, tb)
        if not ok:
            notes.append(f"{r}°: {why}")
            continue
        R = pass_rect(r, W, H, tb.safe)
        if R is None:
            notes.append(f"{r}°: окно достижимости не заходит на лист")
            continue
        allowed.append(r)
        rects[r] = R
    return allowed, rects, notes


def min_cover(allowed: list[int], covers) -> list[int] | None:
    for k in range(1, len(allowed) + 1):
        for combo in itertools.combinations(allowed, k):
            if covers(list(combo)):
                return list(combo)
    return None


def plan_sheet(W: float, H: float, target: Rect, printer, what: str = "лист") -> Plan:
    allowed, rects, notes = rotation_rects(W, H, printer)
    plan = Plan(W, H, allowed, rects, notes=notes)
    combo = min_cover(allowed, lambda rs: not uncovered(target, [rects[r] for r in rs]))
    if combo is not None:
        plan.rotations = combo
        plan.uncovered = uncovered((0, 0, W, H), [rects[r] for r in combo])
        return plan
    plan.uncovered = uncovered(target, [rects[r] for r in allowed])
    plan.message = explain(W, H, target, printer, allowed, notes, what)
    return plan


def _with(printer, **table_changes):
    p = printer.model_copy(deep=True)
    for k, v in table_changes.items():
        setattr(p.table, k, v)
    return p


def _grow(printer, side: str, d: float):
    p = printer.model_copy(deep=True)
    t = p.travel
    tb = make_table(printer, 1, 1)
    if p.table.table_x is None:
        p.table.table_x = tb.table_x
    if p.table.table_y is None:
        p.table.table_y = tb.table_y
    axis = side[0]
    flip = p.flip_x if axis == "x" else p.flip_y
    grow_max = (side[1] == "+") != flip
    name = f"{axis}_{'max' if grow_max else 'min'}"
    setattr(t, name, getattr(t, name) + (d if grow_max else -d))
    return p


def _covers_with(printer, W, H, target, allowed_fixed: list[int] | None) -> list[int] | None:
    allowed, rects, _ = rotation_rects(W, H, printer)
    if allowed_fixed is not None:
        allowed = [r for r in allowed if r in allowed_fixed]
    return min_cover(allowed, lambda rs: not uncovered(target, [rects[r] for r in rs]))


def deficit(printer, W, H, target, allowed: list[int], side: str, limit: float) -> float | None:
    if printer.travel is None or not allowed:
        return None
    if _covers_with(_grow(printer, side, limit), W, H, target, allowed) is None:
        return None
    lo, hi = 0.0, limit
    for _ in range(40):
        mid = (lo + hi) / 2
        if _covers_with(_grow(printer, side, mid), W, H, target, allowed) is None:
            lo = mid
        else:
            hi = mid
        if hi - lo < 0.05:
            break
    return hi


def explain(W, H, target, printer, allowed, notes, what: str) -> str:
    parts = []
    size = f"{W:g}×{H:g} мм"
    if not allowed:
        parts.append(f"{what.capitalize()} {size} не встаёт на стол ни в одном повороте: " + "; ".join(notes) + ".")
        if printer.table.table_x is None or printer.table.table_y is None:
            parts.append("Размер стола не задан, край стола считается по краю окна достижимости; "
                         "если стол больше, введи его размер от упора.")
    else:
        parts.append(f"{what.capitalize()} {size} не покрывается проходами "
                     f"(допустимые повороты: {', '.join(f'{r}°' for r in allowed)}).")
        lim = W + H
        best = {}
        for side in ("x+", "x-", "y+", "y-"):
            d = deficit(printer, W, H, target, allowed, side, lim)
            if d is not None:
                ax = side[0]
                if ax not in best or d < best[ax][0]:
                    best[ax] = (d, side)
        if best:
            texts = []
            for ax in ("x", "y"):
                if ax in best:
                    d, side = best[ax]
                    texts.append(f"по оси {ax.upper()} не хватает {d:.1f} мм окна "
                                 f"(в сторону {'+' if side[1] == '+' else '−'}{ax.upper()})")
            parts.append("Не хватает: " + " или ".join(texts) + ".")
        elif printer.travel is not None:
            parts.append("Одним расширением окна по одной оси не исправить.")
    tb = printer.table
    for flag, name in (("overhang_x", "+X"), ("overhang_y", "+Y")):
        if not getattr(tb, flag):
            combo = _covers_with(_with(printer, **{flag: True}), W, H, target, None)
            if combo is not None:
                parts.append(f"Включи «лист может выступать в сторону {name}» — будет проходов: {len(combo)} "
                             f"({', '.join(f'{r}°' for r in combo)}).")
    if not tb.overhang_x and not tb.overhang_y:
        combo = _covers_with(_with(printer, overhang_x=True, overhang_y=True), W, H, target, None)
        if combo is not None and not any("Включи" in p for p in parts):
            parts.append(f"Включи обе стороны +X и +Y — будет проходов: {len(combo)}.")
    parts.append("Или уменьши: формат листа, рабочее поле (поля, рамку) или чертёж "
                 "(кнопка «Подобрать масштаб под проходы»).")
    return " ".join(parts)


Affine = tuple[float, float, float, float, float, float]


def marked_affine(mk, W: float, H: float, r: int) -> Affine:
    if W > H:
        a, b, c, d, e, f = _marked_base(mk, H, W, r)
        return (-b, a, -d, c, b * W + e, d * W + f)
    return _marked_base(mk, W, H, r)


def _marked_base(mk, W: float, H: float, r: int) -> Affine:
    ex, ey = mk.tr_x - mk.tl_x, mk.tr_y - mk.tl_y
    n = math.hypot(ex, ey) or 1.0
    ex, ey = ex / n, ey / n
    nx, ny = ey, -ex
    if r == 180:
        return (-ex, nx, -ey, ny, mk.tl_x + W * ex, mk.tl_y + W * ey)
    return (ex, -nx, ey, -ny, mk.tl_x + H * nx, mk.tl_y + H * ny)


def apply_affine(m: Affine, p: Point) -> Point:
    return (m[0] * p[0] + m[1] * p[1] + m[4], m[2] * p[0] + m[3] * p[1] + m[5])


def invert_affine(m: Affine, q: Point) -> Point:
    det = m[0] * m[3] - m[1] * m[2]
    x, y = q[0] - m[4], q[1] - m[5]
    return ((m[3] * x - m[1] * y) / det, (-m[2] * x + m[0] * y) / det)


def marked_rects(mk, W: float, H: float) -> tuple[list[int], dict[int, Rect], list[str]]:
    allowed, rects, notes = [], {}, []
    for r in (0, 180):
        m = marked_affine(mk, W, H, r)
        A, B, C = m[2], m[3], m[5]
        if abs(B) >= abs(A):
            bounds = [(-C - A * u) / B for u in (0.0, W)]
            R = (0.0, max(0.0, max(bounds)), W, H) if B > 0 else (0.0, 0.0, W, min(H, min(bounds)))
        else:
            bounds = [(-C - B * v) / A for v in (0.0, H)]
            R = (max(0.0, max(bounds)), 0.0, W, H) if A > 0 else (0.0, 0.0, min(W, min(bounds)), H)
        if R[3] - R[1] <= EPS or R[2] - R[0] <= EPS:
            notes.append(f"{r}°: лист не заходит выше линии нуля")
            continue
        allowed.append(r)
        rects[r] = R
    return allowed, rects, notes
