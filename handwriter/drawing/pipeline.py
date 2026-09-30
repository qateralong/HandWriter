from __future__ import annotations

import itertools
import math
import re
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np

from ..checks import check_bounds, check_printer, to_machine
from ..gcode import compute_stats, fmt, generate_gcode
from ..geometry import rdp
from ..pipeline import GenerationRefused
from ..settings import Settings
from .model import ImportResult
from .ops import dash_polyline, dedupe, expand_passes, join_paths, order_paths, outside_parts
from .passes import (CORNER, CORNER_NAME, Plan, corner_point, covered_mask, intersect, make_table, pass_dims,
                     plan_sheet, rotation_rects, segments, to_pass, uncovered)
from .place import (Placement, SheetLayout, best_fit, place, reach_areas, scale_label, sheet_layout, shrink)
from .sources import BUILTIN_TEST, load_drawing
from .split import CutStats, Geometry, Mark, Node, control_marks, cut_stroke, mark_strokes, solve

Point = tuple[float, float]
Rect = tuple[float, float, float, float]
MIN_DASH_PERIOD = 0.5
TEXT_LIST_LIMIT = 12
_CORNER_EN = {"A": "bottom left", "B": "bottom right", "C": "top right", "D": "top left"}


@dataclass
class PassPart:
    index: int
    rotation: int
    region: Rect
    strokes: list[list[Point]]
    thick: list[bool]
    marks: int = 0
    dx: float = 0.0
    dy: float = 0.0
    errors: list[str] = field(default_factory=list)

    @property
    def corner(self) -> str:
        return CORNER[self.rotation]


@dataclass
class DrawingComposition:
    settings: Settings
    sheet_settings: Settings | None = None
    imp: ImportResult | None = None
    layout: SheetLayout | None = None
    placement: Placement | None = None
    reach_measured: bool = False
    allowed: list[int] = field(default_factory=list)
    pass_rects: dict[int, Rect] = field(default_factory=dict)
    rotation_notes: list[str] = field(default_factory=list)
    sheet_plan: Plan | None = None
    drawing_passes: list[int] | None = None
    parts: list[PassPart] = field(default_factory=list)
    split: Node | None = None
    marks: list[Mark] = field(default_factory=list)
    cut_stats: CutStats | None = None
    source_strokes: list[list[Point]] = field(default_factory=list)
    strokes: list[list[Point]] = field(default_factory=list)
    thick: list[bool] = field(default_factory=list)
    stroke_part: list[int] = field(default_factory=list)
    unreachable: list[list[Point]] = field(default_factory=list)
    texts: list[dict] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    reach_percent: float | None = None
    passes_percent: float | None = None
    dashed_solid: int = 0

    @property
    def rotation(self) -> int | None:
        return self.parts[0].rotation if len(self.parts) == 1 else None

    @property
    def reach(self) -> Rect | None:
        r = self.rotation if self.rotation is not None else 0
        return self.pass_rects.get(r)

    def part_strokes(self, part: PassPart, strokes: list[list[Point]] | None = None) -> list[list[Point]]:
        W, H = self.layout.width, self.layout.height
        out = []
        for st in (part.strokes if strokes is None else strokes):
            out.append([(q[0] + part.dx, q[1] + part.dy) for q in (to_pass(p, part.rotation, W, H) for p in st)])
        return out

    def pass_strokes(self) -> list[list[Point]]:
        return self.part_strokes(self.parts[0]) if len(self.parts) == 1 else self.strokes

    def part_settings(self, rotation: int) -> Settings:
        s = self.sheet_settings.model_copy(deep=True)
        s.sheet.width, s.sheet.height = pass_dims(rotation, self.layout.width, self.layout.height)
        s.printer._use_work_area = True
        return s

    def pass_settings(self) -> Settings:
        return self.part_settings(self.rotation) if self.rotation is not None else self.sheet_settings


def is_thick(p, ds) -> bool:
    w = ds.weights
    if not w.enabled:
        return False
    mode = w.layers.get(p.layer, "auto") if p.layer else "auto"
    if mode != "auto":
        return mode == "thick"
    return p.width is not None and p.width > w.threshold + 1e-9


def field_rect(lay: SheetLayout, ds) -> Rect:
    if lay.inner is not None:
        return lay.inner
    m = ds.placement.margin
    return (m, m, lay.width - m, lay.height - m)


def _file_segments(imp: ImportResult):
    lines = [p.points for p in imp.paths]
    fb = imp.fill_bbox()
    if fb:
        x0, y0, x1, y1 = fb
        lines.append([(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)])
    return segments(lines)


def fit_to_passes(imp: ImportResult, bbox, area: Rect, s_max: float, rects: list[Rect], pad: float,
                  dx: float, dy: float, anchor: str = "center", areas: int = 0) -> float:
    rects = [r for r in (shrink(q, pad) for q in rects) if r[2] > r[0] and r[3] > r[1]]
    if not rects or s_max <= 0:
        return 0.0
    groups = [list(g) for g in itertools.combinations(rects, areas)] if 0 < areas < len(rects) else [rects]
    A, B = _file_segments(imp)
    cxa, cya = (area[0] + area[2]) / 2, (area[1] + area[3]) / 2
    cxb, cyb = (bbox[0] + bbox[2]) / 2, (bbox[1] + bbox[3]) / 2

    def ok(sc):
        if anchor == "zero":
            t = np.array([-bbox[0] * sc + pad + dx, -bbox[1] * sc + pad + dy])
        else:
            t = np.array([cxa - cxb * sc + dx, cya - cyb * sc + dy])
        return any(bool(covered_mask(A * sc + t, B * sc + t, g).all()) for g in groups)

    if ok(s_max):
        return s_max
    lo, hi = 0.0, s_max
    for _ in range(40):
        mid = (lo + hi) / 2
        if ok(mid):
            lo = mid
        else:
            hi = mid
        if hi - lo < s_max * 1e-4:
            break
    return lo if lo > 0 and ok(lo) else 0.0


def hatch_mask(mask, px: float, pl: Placement, step: float, direction: str) -> list[list[Point]]:
    if mask is None or not mask.any() or pl.scale <= 0:
        return []
    if direction == "auto":
        starts_h = int(mask[:, 0].sum() + (mask[:, 1:] & ~mask[:, :-1]).sum()) / mask.shape[0]
        starts_v = int(mask[0, :].sum() + (mask[1:, :] & ~mask[:-1, :]).sum()) / mask.shape[1]
        cost_h = starts_h * mask.shape[0]
        cost_v = starts_v * mask.shape[1]
        direction = "horizontal" if cost_h <= cost_v else "vertical"
    m = mask if direction == "horizontal" else mask.T
    n_lines = m.shape[0]
    step_px = max(step / pl.scale / px, 1e-6)
    out: list[list[Point]] = []
    k = 0
    pos = step_px / 2
    while pos < n_lines:
        row = m[int(pos)]
        if row.any():
            d = np.diff(np.concatenate(([0], row.view(np.int8), [0])))
            starts, ends = np.nonzero(d == 1)[0], np.nonzero(d == -1)[0]
            segs = []
            for a, b in zip(starts, ends):
                if direction == "horizontal":
                    y = -pos * px
                    p0, p1 = pl.apply((a * px, y)), pl.apply((b * px, y))
                else:
                    x = pos * px
                    p0, p1 = pl.apply((x, -a * px)), pl.apply((x, -b * px))
                segs.append([p0, p1])
            if k % 2:
                segs = [s[::-1] for s in segs[::-1]]
            out += segs
            k += 1
        pos += step_px
    return out


def compose_drawing(s: Settings) -> DrawingComposition:
    ds = s.drawing
    c = DrawingComposition(settings=s)
    tol = ds.paths.curve_tol
    imp = load_drawing(ds.file, ds.imp, tol)
    c.imp = imp
    c.warnings += imp.warnings
    if imp.errors:
        c.errors += imp.errors
        return c
    bbox = imp.bbox()
    if bbox is None:
        n_img = sum(1 for t in imp.texts if t.kind == "image")
        n_txt = len(imp.texts) - n_img
        extra = []
        if n_txt:
            extra.append(f"текст не в кривых: {n_txt}")
        if n_img:
            extra.append(f"встроенные картинки: {n_img} (скан? загрузи его как PNG/JPG)")
        c.errors.append("В файле нет линий, которые можно нарисовать" + (f" ({'; '.join(extra)})" if extra else ""))
        return c

    pad = (ds.weights.passes - 1) / 2 * ds.weights.step * 2 if ds.weights.enabled else 0.0

    def setup(imp_, bbox_):
        lay_ = sheet_layout(ds, bbox_)
        allowed_, rects_, notes_ = rotation_rects(lay_.width, lay_.height, s.printer)
        windows = [rects_[r] for r in allowed_]
        if ds.placement.anchor == "zero" and 0 in rects_:
            windows = [rects_[0]]
        pl_ = place(ds, bbox_, lay_, windows, pad)
        if ds.placement.scale_mode == "fit_passes" and not pl_.errors:
            sc = fit_to_passes(imp_, bbox_, pl_.area, pl_.scale, windows, pad, ds.placement.dx, ds.placement.dy,
                               ds.placement.anchor, ds.split.areas)
            if sc <= 0:
                pl_.errors.append("Подобрать масштаб под проходы не получается: середина поля листа не достаётся "
                                  "ни в одном допустимом проходе. Сдвинь чертёж (dx, dy), разреши другую сторону "
                                  "или проверь окно достижимости")
            elif sc < pl_.scale and ds.placement.anchor == "zero":
                pl_ = Placement(sc, -bbox_[0] * sc + pad + ds.placement.dx, -bbox_[1] * sc + pad + ds.placement.dy, pl_.area,
                                pl_.errors, pl_.warnings)
            elif sc < pl_.scale:
                k = sc / pl_.scale
                cx, cy = (pl_.area[0] + pl_.area[2]) / 2, (pl_.area[1] + pl_.area[3]) / 2
                pl_ = Placement(sc, cx + (pl_.tx - cx) * k + ds.placement.dx * (1 - k),
                                cy + (pl_.ty - cy) * k + ds.placement.dy * (1 - k), pl_.area, pl_.errors, pl_.warnings)
        return lay_, allowed_, rects_, notes_, pl_

    lay, allowed, rects, notes, pl = setup(imp, bbox)
    if pl.scale > 1.0001 and imp.kind != "raster":
        k = 2 ** math.ceil(math.log2(pl.scale))
        imp2 = load_drawing(ds.file, ds.imp, tol / k)
        if not imp2.errors and imp2.bbox() is not None:
            imp, bbox = imp2, imp2.bbox()
            c.imp = imp
            lay, allowed, rects, notes, pl = setup(imp, bbox)
    W, H = lay.width, lay.height
    s2 = s.model_copy(deep=True)
    s2.sheet.width, s2.sheet.height = W, H
    c.layout, c.sheet_settings, c.placement = lay, s2, pl
    c.allowed, c.pass_rects, c.rotation_notes = allowed, rects, notes
    c.reach_measured = s.printer.travel is not None
    e, w = check_printer(s2)
    c.errors += e
    c.warnings += w
    c.errors += pl.errors
    c.warnings += pl.warnings
    c.sheet_plan = plan_sheet(W, H, field_rect(lay, ds), s.printer, "рабочее поле листа")
    if pl.errors:
        return c

    s_reach, _ = best_fit(bbox, reach_areas(lay, [rects[r] for r in allowed], pad))
    c.reach_percent = s_reach * 100 if s_reach > 0 else None

    groups: dict[bool, list[list[Point]]] = {False: [], True: []}
    for p in imp.paths:
        pts = dedupe([pl.apply(q) for q in p.points])
        thick = is_thick(p, ds)
        pieces = [pts]
        if p.dash:
            pat = [v * pl.scale for v in p.dash]
            if sum(pat) < MIN_DASH_PERIOD:
                c.dashed_solid += 1
            else:
                pieces = dash_polyline(pts, pat, p.dash_offset * pl.scale)
        groups[thick] += [q for q in pieces if q]
    if c.dashed_solid:
        c.warnings.append(f"Штриховые линии с узором мельче {MIN_DASH_PERIOD} мм на бумаге нарисованы сплошными: "
                          f"{c.dashed_solid}")
    for fl in lay.frame:
        groups[ds.weights.enabled and fl.thick].append(list(fl.points))
    if imp.fill_mask is not None:
        groups[False] += hatch_mask(imp.fill_mask, imp.fill_px, pl, ds.imp.fill_step, ds.imp.fill_dir)

    tol_rdp = s.printer.simplify_tol
    final: list[list[Point]] = []
    flags: list[bool] = []
    for thick in (True, False):
        for pts in join_paths(groups[thick], ds.paths.join_tol):
            pts = rdp(pts, tol_rdp) if len(pts) > 2 else pts
            closed = len(pts) > 2 and pts[0] == pts[-1]
            passes = expand_passes(pts, closed, ds.weights.passes, ds.weights.step) if thick else [pts]
            for q in passes:
                final.append(q)
                flags.append(thick)

    for t in imp.texts:
        x, y = pl.apply((t.x, t.y))
        c.texts.append({"x": x, "y": y, "text": t.text, "kind": t.kind})
    txt = [t for t in c.texts if t["kind"] == "text"]
    img = [t for t in c.texts if t["kind"] == "image"]
    if txt:
        listed = "; ".join(f"«{_short(t['text'])}» (X {t['x']:.0f}, Y {t['y']:.0f})" for t in txt[:TEXT_LIST_LIMIT])
        more = f" и ещё {len(txt) - TEXT_LIST_LIMIT}" if len(txt) > TEXT_LIST_LIMIT else ""
        c.warnings.append(f"Текст не в кривых не рисуется ({len(txt)} мест, координаты на листе, мм): {listed}{more}. "
                          "Преврати текст в кривые в редакторе, если он нужен на бумаге")
    if img:
        c.warnings.append(f"Встроенные картинки не рисуются: {len(img)}")

    off_sheet = sum(1 for st in final for x, y in st if not (-1e-6 <= x <= W + 1e-6 and -1e-6 <= y <= H + 1e-6))
    if off_sheet:
        c.errors.append(f"Чертёж выходит за край листа ({off_sheet} точек): уменьши масштаб или сдвиг")

    c.source_strokes = final
    A, B = segments(final)
    windows = [rects[r] for r in allowed]
    if covered_mask(A, B, windows).all() and final:
        _split(c, final, flags, W, H)
    else:
        ordered = order_paths(final, ds.paths.long_path, (0.0, 0.0))
        c.strokes = [seq for _, seq in ordered]
        c.thick = [flags[i] for i, _ in ordered]
        c.stroke_part = [0] * len(c.strokes)
        if final:
            c.unreachable = _outside_all(c.strokes, windows)
            c.errors.append(_unreachable_message(c, s, bbox, pad, W, H, lay))

    if not c.errors:
        for part in c.parts:
            be = check_bounds(c.part_strokes(part), c.part_settings(part.rotation))
            if be:
                part.errors = [f"Проход {part.index} ({part.rotation}°): {m}" for m in be]
                if part.dx or part.dy:
                    part.errors.append(f"Проход {part.index}: поправка dx {part.dx:g}, dy {part.dy:g} выводит линии "
                                       "за окно достижимости — уменьши поправку или масштаб")
                c.errors += part.errors
    if not final:
        c.errors.append("Нечего рисовать")
    return c


def _split(c: DrawingComposition, final: list[list[Point]], flags: list[bool], W: float, H: float) -> None:
    ds = c.settings.drawing
    sp = ds.split
    rects = c.pass_rects
    A, B = segments(final)
    geo = Geometry(final)
    sheet = (0.0, 0.0, W, H)
    root = None
    used_slack = True
    counts = [sp.areas] if sp.areas else range(1, len(c.allowed) + 1)
    for k in counts:
        for combo in itertools.combinations(c.allowed, k):
            if not covered_mask(A, B, [rects[r] for r in combo]).all():
                continue
            for margin in (sp.overlap + sp.slack, sp.overlap):
                root = solve(geo, rects, list(combo), sheet, sheet, margin, sp.overlap)
                if root is not None:
                    used_slack = margin > sp.overlap or sp.slack == 0
                    break
            if root is not None:
                break
        if root is not None:
            break
    if root is None:
        if sp.areas and not any(covered_mask(A, B, [rects[r] for r in combo]).all()
                                for combo in itertools.combinations(c.allowed, min(sp.areas, len(c.allowed)))):
            c.errors.append(f"Выбрано рабочих областей: {sp.areas} — их не хватает, чтобы достать весь чертёж. "
                            "Поставь «авто» или больше областей, либо уменьши масштаб")
        else:
            c.errors.append("Проходы вместе достают весь чертёж, но провести между ними прямые швы не получилось. "
                            "Уменьши масштаб («Подобрать масштаб под проходы» с запасом) или сдвинь чертёж")
        ordered = order_paths(final, ds.paths.long_path, (0.0, 0.0))
        c.strokes = [q for _, q in ordered]
        c.thick = [flags[i] for i, _ in ordered]
        c.stroke_part = [0] * len(c.strokes)
        return
    if not used_slack:
        c.warnings.append(f"Запас у шва под сдвиг нуля ({sp.slack:g} мм) не поместился: шов проведён только с "
                          f"нахлёстом {sp.overlap:g} мм, ставь ноль особенно точно")
    c.split = root
    leaves = {lf.rotation: lf for lf in root.leaves()}
    stats = CutStats()
    pieces: dict[int, list[tuple[list[Point], bool]]] = {r: [] for r in leaves}
    for st, th in zip(final, flags):
        for rot, piece in cut_stroke(st, root, sp.overlap, stats):
            pieces[rot].append((piece, th))
    c.cut_stats = stats
    c.marks = control_marks(root, geo, rects, sp.mark_size, sp.mark_count) if sp.marks and len(leaves) > 1 else []
    if sp.marks and len(leaves) > 1 and not c.marks:
        c.warnings.append("Контрольные крестики не поместились: у швов нет места, которое достают оба прохода "
                          "вдали от линий")
    idx = 0
    for rot in sorted(pieces):
        mk = [q for m in c.marks if rot in m.passes for q in mark_strokes(m, sp.mark_size)]
        if not pieces[rot] and not mk:
            continue
        strokes, th = [], []
        for thick in (True, False):
            for pts in join_paths([p for p, t in pieces[rot] if t == thick], ds.paths.join_tol):
                strokes.append(pts)
                th.append(thick)
        ordered = order_paths(strokes, ds.paths.long_path, corner_point(rot, W, H))
        idx += 1
        dx, dy = sp.offsets.get(str(rot), (0.0, 0.0))
        c.parts.append(PassPart(idx, rot, leaves[rot].core, mk + [q for _, q in ordered],
                                [False] * len(mk) + [th[i] for i, _ in ordered], len(mk), float(dx), float(dy)))
    empty = [r for r in sorted(leaves) if r not in {p.rotation for p in c.parts}]
    if empty:
        c.warnings.append(f"Областей по раскладке: {len(leaves)}, но в {len(empty)} из них нет линий "
                          f"(поворот {', '.join(f'{r}°' for r in empty)}) — файлов будет {len(c.parts)}")
    c.drawing_passes = [p.rotation for p in c.parts]
    for p in c.parts:
        c.strokes += p.strokes
        c.thick += p.thick
        c.stroke_part += [p.index] * len(p.strokes)


def _outside_all(strokes: list[list[Point]], rects: list[Rect]) -> list[list[Point]]:
    out = []
    for st in strokes:
        pieces = [st]
        for r in rects:
            pieces = [q for p in pieces for q in outside_parts(p, r)]
            if not pieces:
                break
        out += pieces
    return out


def _unreachable_message(c: DrawingComposition, s: Settings, bbox, pad: float, W: float, H: float,
                         lay: SheetLayout) -> str:
    ds = s.drawing
    msg = ("Часть чертежа не достаётся ни в одном допустимом проходе (красным"
           + (f"; допустимые повороты: {', '.join(f'{r}°' for r in c.allowed)}" if c.allowed else
              "; допустимых поворотов нет: " + "; ".join(c.rotation_notes)) + "). ")
    if lay.frame and c.allowed:
        A, B = segments([fl.points for fl in lay.frame])
        if not covered_mask(A, B, [c.pass_rects[r] for r in c.allowed]).all():
            msg += "Рамка ГОСТ не покрывается проходами: масштаб чертежа тут не поможет, выключи рамку или смени лист. "
    if c.allowed and ds.placement.scale_mode != "fit_passes":
        area = c.placement.area
        s_fit, fit_area = best_fit(bbox, [shrink(a, pad) for a in lay.areas])
        zero = ds.placement.anchor == "zero" and 0 in c.pass_rects
        sc = fit_to_passes(c.imp, bbox, fit_area or area, s_fit,
                           [c.pass_rects[0]] if zero else [c.pass_rects[r] for r in c.allowed], pad,
                           ds.placement.dx, ds.placement.dy, ds.placement.anchor, ds.split.areas)
        if sc > 0:
            c.passes_percent = sc * 100
            msg += f"Уменьши масштаб до {math.floor(sc * 1000) / 10:g}% — кнопка «Подобрать масштаб под проходы». "
    for flag, name in (("overhang_x", "+X"), ("overhang_y", "+Y")):
        if not getattr(s.printer.table, flag):
            p2 = s.printer.model_copy(deep=True)
            setattr(p2.table, flag, True)
            allowed2, rects2, _ = rotation_rects(W, H, p2)
            A, B = segments(c.strokes)
            if allowed2 != c.allowed and covered_mask(A, B, [rects2[r] for r in allowed2]).all():
                msg += f"Или включи «лист может выступать в сторону {name}». "
    return msg.strip()


def _short(t: str, n: int = 40) -> str:
    return t if len(t) <= n else t[: n - 1] + "…"


_SAFE = re.compile(r'[<>:"/\\|?*\x00-\x1f\s]+')


def file_stem(c: DrawingComposition) -> str:
    ds = c.settings.drawing
    stem = "test" if ds.file == BUILTIN_TEST else _SAFE.sub("_", Path(c.imp.name).stem) or "drawing"
    fmt_name = ds.sheet.format if ds.sheet.format != "custom" else f"{c.layout.width:g}x{c.layout.height:g}"
    o = "P" if c.layout.orientation == "portrait" else "L"
    return f"drawing_{stem}_{fmt_name}{o}_{c.placement.scale * 100:.0f}pct"


def part_filename(c: DrawingComposition, part: PassPart, test: bool = False) -> str:
    return f"{file_stem(c)}_pass{part.index}_rot{part.rotation}" + ("_test" if test else "") + ".gcode"


def gcode_filename(c: DrawingComposition) -> str:
    return part_filename(c, c.parts[0]) if c.parts else file_stem(c) + ".gcode"


def _seams_text(c: DrawingComposition) -> list[str]:
    if c.split is None:
        return []
    return [f"{'x' if n.axis == 0 else 'y'} = {n.s:.2f}" for n in c.split.seams()]


def part_header(c: DrawingComposition, part: PassPart, test: bool = False) -> tuple[list[str], list[str]]:
    ds, pl, lay = c.settings.drawing, c.placement, c.layout
    w, fr, sp = ds.weights, ds.frame, ds.split
    r = part.rotation
    n = len(c.parts)
    header = [
        f"DRAWING: {ds.file if ds.file == BUILTIN_TEST else c.imp.name} ({c.imp.kind}"
        + (f", page {c.imp.page}" if c.imp.kind == "pdf" else "") + ")",
        ("TEST FILE for " if test else "") + f"PASS {part.index}/{n}: rotate sheet {r} deg CCW, corner {part.corner} "
        f"({_CORNER_EN[part.corner]} in layout) at the stops = zero",
        "run order: " + ", ".join(f"{p.index}) {part_filename(c, p, test)}" for p in c.parts),
        f"zero correction dx {fmt(part.dx)} dy {fmt(part.dy)} mm (pass coordinates)",
        f"scale {scale_label(pl.scale)} ({pl.scale * 100:.2f}%), mode {ds.placement.scale_mode}",
    ]
    if n > 1:
        header.append(f"seams (layout mm): {'; '.join(_seams_text(c))}; overlap {fmt(sp.overlap)} mm along lines")
    if part.marks:
        header.append(f"control crosses: {part.marks // 2}, size {fmt(sp.mark_size)} mm, drawn first")
    Wp, Hp = pass_dims(r, lay.width, lay.height)
    tb = make_table(c.settings.printer, lay.width, lay.height, r)
    info = [
        f"sheet {ds.sheet.format} {lay.orientation} {fmt(lay.width)}x{fmt(lay.height)} mm (in this pass "
        f"{fmt(Wp)}x{fmt(Hp)} along X x Y), drawing origin offset X{fmt(pl.tx)} Y{fmt(pl.ty)}",
        f"table: sheet may overhang +X {int(tb.allow_x)} +Y {int(tb.allow_y)}; table edge X{fmt(tb.table_x)} "
        f"Y{fmt(tb.table_y)}" + ("" if tb.table_given else " (= reach window edge)"),
        ("frame GOST 2.104 form 1: L{} R{} T{} B{}".format(fmt(fr.left), fmt(fr.right), fmt(fr.top), fmt(fr.bottom))
         + (f", title block {fmt(fr.tb_width)}x{fmt(fr.tb_height)}" if fr.title_block else ", no title block"))
        if fr.enabled else "frame: off",
        (f"line weights: thick > {fmt(w.threshold)} mm drawn in {w.passes} passes, step {fmt(w.step)} mm"
         if w.enabled else "line weights: off (single pass for all lines)"),
        f"curves {fmt(ds.paths.curve_tol)} mm, join {fmt(ds.paths.join_tol)} mm, long paths first >= "
        f"{fmt(ds.paths.long_path)} mm",
    ]
    return header, info


def make_part_gcode(c: DrawingComposition, part: PassPart) -> str:
    if c.errors:
        raise GenerationRefused(c.errors)
    header, info = part_header(c, part)
    return generate_gcode(c.part_strokes(part), c.part_settings(part.rotation), header, info)


def make_drawing_gcode(c: DrawingComposition, index: int = 1) -> str:
    if c.errors:
        raise GenerationRefused(c.errors)
    if not c.parts:
        raise GenerationRefused(["Нет проходов"])
    return make_part_gcode(c, c.parts[index - 1])


def part_test_strokes(c: DrawingComposition, part: PassPart) -> list[list[Point]]:
    lay = c.layout
    W, H = lay.width, lay.height
    r = part.rotation

    def rect_pts(q: Rect) -> list[Point]:
        a = to_pass((q[0], q[1]), r, W, H)
        b = to_pass((q[2], q[3]), r, W, H)
        x0, x1 = sorted((a[0], b[0]))
        y0, y1 = sorted((a[1], b[1]))
        return [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]

    out: list[list[Point]] = []
    fixed: list[list[Point]] = []
    reach = c.pass_rects.get(r)
    if reach:
        fixed.append(rect_pts(reach))
    if len(c.parts) > 1:
        cell = (max(part.region[0], 0), max(part.region[1], 0), min(part.region[2], W), min(part.region[3], H))
        cell = intersect(cell, reach) if reach else cell
        if cell:
            out += dash_polyline(rect_pts(cell), [4.0, 3.0])
    o = c.settings.printer.test_mark_offset
    head = 2.0
    out.append([(o, o), (o + 20, o)])
    out.append([(o + 20 - head, o + head * 0.6), (o + 20, o), (o + 20 - head, o - head * 0.6)])
    out.append([(o, o), (o, o + 10)])
    out.append([(o - head * 0.6, o + 10 - head), (o, o + 10), (o + head * 0.6, o + 10 - head)])
    for i in range(part.index):
        x = o + 25 + 2.5 * i
        out.append([(x, o - 1.5), (x, o + 1.5)])
    marks = [to_pass_pts(st, r, W, H) for st in part.strokes[: part.marks]]
    shifted = [[(p[0] + part.dx, p[1] + part.dy) for p in st] for st in out + marks]
    return fixed + shifted


def to_pass_pts(st, r, W, H):
    return [to_pass(p, r, W, H) for p in st]


def make_part_test_gcode(c: DrawingComposition, part: PassPart) -> str:
    strokes = part_test_strokes(c, part)
    errors = check_bounds(strokes, c.part_settings(part.rotation))
    if errors:
        raise GenerationRefused([f"Тестовый файл прохода {part.index}: {m}" for m in errors])
    header, info = part_header(c, part, test=True)
    header.insert(2, "TEST: available part of the sheet (solid), this pass cell (dashed), "
                     "corner mark X long / Y short with pass number ticks, control crosses")
    return generate_gcode(strokes, c.part_settings(part.rotation), header, info)


def make_all_files(c: DrawingComposition, tests: bool = False) -> list[dict]:
    if c.errors:
        raise GenerationRefused(c.errors)
    out = []
    for p in c.parts:
        out.append({"filename": part_filename(c, p), "gcode": make_part_gcode(c, p), "pass": p.index,
                    "rotation": p.rotation, "test": False})
        if tests:
            out.append({"filename": part_filename(c, p, True), "gcode": make_part_test_gcode(c, p), "pass": p.index,
                        "rotation": p.rotation, "test": True})
    return out


def _travel_on_sheet(strokes: list[list[Point]], start: Point) -> list[tuple[Point, Point]]:
    moves = []
    cur = start
    for st in strokes:
        if st and st[0] != cur:
            moves.append((cur, st[0]))
        if st:
            cur = st[-1]
    if cur != start:
        moves.append((cur, start))
    return moves


def preview_payload(c: DrawingComposition) -> dict:
    r2 = lambda p: [round(p[0], 3), round(p[1], 3)]
    rr = lambda q: [round(v, 2) for v in q]
    s2 = c.sheet_settings or c.settings
    out: dict = {"errors": c.errors, "warnings": c.warnings, "strokes": [], "travel": [], "unreachable": [],
                 "texts": [], "frame": [], "stats": compute_stats([], s2).as_dict(), "parts": [], "seams": [],
                 "marks": []}
    imp = c.imp
    if imp is not None:
        bb = imp.bbox()
        out["import"] = {
            "kind": imp.kind, "name": imp.name, "units": imp.units, "units_note": imp.units_note,
            "pages": imp.pages, "page": imp.page, "info": imp.info,
            "size": [round(bb[2] - bb[0], 2), round(bb[3] - bb[1], 2)] if bb else None,
            "paths": len(imp.paths),
            "layers": [{"name": n, "count": v["count"], "width": v["width"]} for n, v in sorted(imp.layers.items())],
            "has_widths": any(p.width is not None for p in imp.paths),
        }
    if c.layout is None:
        return out
    lay, pl = c.layout, c.placement
    W, H = lay.width, lay.height
    out["sheet"] = {"width": W, "height": H, "orientation": lay.orientation}
    out["frame"] = [{"p": [r2(p) for p in fl.points], "t": fl.thick} for fl in lay.frame]
    out["title_block"] = lay.title_block
    out["areas"] = lay.areas
    out["reach"] = {"rect": c.reach, "measured": c.reach_measured, "margin": s2.printer.safety_margin}
    sp = c.sheet_plan
    show = sp.rotations if sp and sp.ok else c.allowed
    tables = {str(r): make_table(c.settings.printer, W, H, r) for r in (0, 90, 180, 270)}
    out["passes"] = {
        "allowed": c.allowed, "notes": c.rotation_notes,
        "rects": {str(r): rr(q) for r, q in c.pass_rects.items()},
        "corners": {"A": [0, 0], "B": [W, 0], "C": [W, H], "D": [0, H]},
        "corner_of": {str(r): CORNER[r] for r in (0, 90, 180, 270)},
        "map": show,
        "map_uncovered": [rr(q) for q in uncovered((0, 0, W, H), [c.pass_rects[r] for r in show])],
        "field": rr(field_rect(lay, c.settings.drawing)),
        "sheet_plan": {"rotations": sp.rotations if sp else None, "describe": sp.describe() if sp and sp.ok else "",
                       "message": sp.message if sp else ""},
        "drawing": c.drawing_passes, "rotation": c.rotation,
        "rotation_text": (f"{c.rotation}°, в упоре угол {CORNER[c.rotation]} ({CORNER_NAME[CORNER[c.rotation]]})"
                          if c.rotation is not None else ""),
        "table": {"allow_x": tables["0"].allow_x, "allow_y": tables["0"].allow_y,
                  "table_x": tables["0"].table_x, "table_y": tables["0"].table_y,
                  "given": tables["0"].table_given, "raw": rr(tables["0"].raw), "safe": rr(tables["0"].safe)},
        "windows": {r: {"raw": rr(t.raw), "safe": rr(t.safe)} for r, t in tables.items()},
    }
    if pl is not None and not pl.errors:
        out["scale"] = {"value": pl.scale, "label": scale_label(pl.scale), "percent": round(pl.scale * 100, 2),
                        "area": pl.area, "reach_percent": round(c.reach_percent, 2) if c.reach_percent else None,
                        "passes_percent": round(c.passes_percent, 2) if c.passes_percent else None}
        bb = imp.bbox()
        if bb:
            out["scale"]["paper_size"] = [round((bb[2] - bb[0]) * pl.scale, 1), round((bb[3] - bb[1]) * pl.scale, 1)]
    totals = {"draw_mm": 0.0, "travel_mm": 0.0, "strokes": 0, "lifts": 0, "time_s": 0}
    travel = []
    for p in c.parts:
        st = compute_stats(c.part_strokes(p), c.part_settings(p.rotation)).as_dict()
        for k in totals:
            totals[k] += st[k]
        corner = corner_point(p.rotation, W, H)
        travel += [[r2(a), r2(b), p.index] for a, b in _travel_on_sheet(p.strokes, corner)]
        pw, ph = pass_dims(p.rotation, W, H)
        out["parts"].append({
            "index": p.index, "rotation": p.rotation, "corner": p.corner, "corner_name": CORNER_NAME[p.corner],
            "region": rr(p.region), "stats": st, "marks": p.marks // 2, "dx": p.dx, "dy": p.dy,
            "filename": part_filename(c, p), "test_filename": part_filename(c, p, True),
            "errors": p.errors, "pass_size": [pw, ph],
        })
    if c.parts:
        totals["draw_mm"] = round(totals["draw_mm"], 1)
        totals["travel_mm"] = round(totals["travel_mm"], 1)
        out["stats"] = totals
    else:
        out["stats"] = compute_stats(c.strokes, s2).as_dict()
        travel = [[r2(a), r2(b), 0] for a, b in _travel_on_sheet(c.strokes, (0.0, 0.0))]
    out["strokes"] = [{"p": [r2(p) for p in st], "t": t, "k": k}
                      for st, t, k in zip(c.strokes, c.thick, c.stroke_part)]
    out["travel"] = travel
    out["seams"] = [{"axis": n.axis, "s": round(n.s, 3), "span": rr((n.core[1 - n.axis], n.core[3 - n.axis]))}
                    for n in (c.split.seams() if c.split else [])]
    out["marks"] = [{"x": round(m.x, 3), "y": round(m.y, 3), "passes": list(m.passes)} for m in c.marks]
    out["overlap"] = c.settings.drawing.split.overlap
    out["unreachable"] = [[r2(p) for p in st] for st in c.unreachable]
    out["texts"] = [{"x": round(t["x"], 2), "y": round(t["y"], 2), "text": t["text"], "kind": t["kind"]}
                    for t in c.texts]
    out["flip"] = [s2.printer.flip_x, s2.printer.flip_y]
    out["machine_origin"] = list(to_machine((0.0, 0.0), s2))
    return out
