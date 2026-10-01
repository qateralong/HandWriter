from __future__ import annotations

from dataclasses import dataclass, field

SHEETS = {"A4": (210.0, 297.0), "A3": (297.0, 420.0)}

Rect = tuple[float, float, float, float]

TITLE_BLOCK_W, TITLE_BLOCK_H = 185.0, 55.0
TITLE_BLOCK_LINES: list[tuple[float, float, float, float, bool]] = [
    (0, 0, 0, 55, True),
    (0, 55, 185, 55, True),
    (7, 30, 7, 55, True),
    (17, 0, 17, 55, True),
    (40, 0, 40, 55, True),
    (55, 0, 55, 55, True),
    (65, 0, 65, 55, True),
    (0, 5, 65, 5, False), (0, 10, 65, 10, False), (0, 15, 65, 15, False),
    (0, 20, 65, 20, False), (0, 25, 65, 25, False),
    (0, 30, 65, 30, True), (0, 35, 65, 35, True),
    (0, 40, 65, 40, False), (0, 45, 65, 45, False), (0, 50, 65, 50, False),
    (65, 40, 185, 40, True),
    (65, 15, 185, 15, True),
    (135, 0, 135, 40, True),
    (135, 35, 185, 35, True),
    (135, 20, 185, 20, True),
    (150, 20, 150, 40, True),
    (167, 20, 167, 40, True),
    (140, 20, 140, 35, False), (145, 20, 145, 35, False),
    (155, 15, 155, 20, True),
]


@dataclass
class FrameLine:
    points: list[tuple[float, float]]
    thick: bool


@dataclass
class SheetLayout:
    width: float
    height: float
    orientation: str
    frame: list[FrameLine] = field(default_factory=list)
    inner: Rect | None = None
    title_block: Rect | None = None
    areas: list[Rect] = field(default_factory=list)


def sheet_size(ds, bbox: Rect | None) -> tuple[float, float, str]:
    sh = ds.sheet
    w, h = SHEETS.get(sh.format, (sh.width, sh.height))
    short, long_ = min(w, h), max(w, h)
    if sh.orientation == "portrait":
        o = "portrait"
    elif sh.orientation == "landscape":
        o = "landscape"
    elif sh.format == "custom":
        o = "portrait" if sh.width <= sh.height else "landscape"
        return sh.width, sh.height, o
    else:
        o = "landscape" if bbox and (bbox[2] - bbox[0]) > (bbox[3] - bbox[1]) else "portrait"
    return (short, long_, o) if o == "portrait" else (long_, short, o)


def sheet_layout(ds, bbox: Rect | None) -> SheetLayout:
    W, H, o = sheet_size(ds, bbox)
    lay = SheetLayout(W, H, o)
    f = ds.frame
    if not f.enabled:
        m = ds.placement.margin
        lay.areas = [(f.left, f.bottom, W - f.right, H - f.top) if ds.marked.enabled or ds.a3.enabled else (m, m, W - m, H - m)]
        return lay
    x0, y0, x1, y1 = f.left, f.bottom, W - f.right, H - f.top
    lay.inner = (x0, y0, x1, y1)
    lay.frame.append(FrameLine([(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)], True))
    if f.title_block:
        tw, th = min(f.tb_width, x1 - x0), min(f.tb_height, y1 - y0)
        tx0, ty0 = x1 - tw, y0
        lay.title_block = (tx0, ty0, x1, ty0 + th)
        kx, ky = tw / TITLE_BLOCK_W, th / TITLE_BLOCK_H
        for a, b, c, d, thick in TITLE_BLOCK_LINES:
            lay.frame.append(FrameLine([(tx0 + a * kx, ty0 + b * ky), (tx0 + c * kx, ty0 + d * ky)], thick))
        lay.areas = [(x0, ty0 + th, x1, y1), (x0, y0, tx0, y1)]
    else:
        lay.areas = [lay.inner]
    return lay


def _fit_scale(bbox: Rect, area: Rect) -> float:
    bw, bh = bbox[2] - bbox[0], bbox[3] - bbox[1]
    aw, ah = area[2] - area[0], area[3] - area[1]
    if aw <= 0 or ah <= 0:
        return 0.0
    sx = aw / bw if bw > 1e-9 else float("inf")
    sy = ah / bh if bh > 1e-9 else float("inf")
    s = min(sx, sy)
    return 0.0 if s == float("inf") else s


def shrink(a: Rect, d: float) -> Rect:
    return (a[0] + d, a[1] + d, a[2] - d, a[3] - d)


def intersect(a: Rect, b: Rect) -> Rect | None:
    r = (max(a[0], b[0]), max(a[1], b[1]), min(a[2], b[2]), min(a[3], b[3]))
    return r if r[2] > r[0] and r[3] > r[1] else None


@dataclass
class Placement:
    scale: float
    tx: float
    ty: float
    area: Rect | None
    errors: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)

    def apply(self, p):
        return (p[0] * self.scale + self.tx, p[1] * self.scale + self.ty)


def best_fit(bbox: Rect, areas: list[Rect]) -> tuple[float, Rect | None]:
    best, best_area = 0.0, None
    for a in areas:
        s = _fit_scale(bbox, a)
        if s > best + 1e-12:
            best, best_area = s, a
    return best, best_area


def reach_areas(lay: SheetLayout, reach: list[Rect], pad: float = 0.0) -> list[Rect]:
    return [r for w in reach for r in (intersect(shrink(a, pad), shrink(w, pad)) for a in lay.areas) if r]


def fixed_mode(ds) -> bool:
    return ds.marked.enabled or ds.a3.enabled


def anchor_of(ds) -> str:
    return "left" if fixed_mode(ds) and not ds.frame.enabled else ds.placement.anchor


def place(ds, bbox: Rect, lay: SheetLayout, reach: list[Rect], pad: float = 0.0) -> Placement:
    pl = ds.placement
    mode = pl.scale_mode
    areas = [shrink(a, pad) for a in lay.areas]
    if mode == "fit_reach":
        areas = reach_areas(lay, reach, pad)
        if not areas:
            return Placement(0, 0, 0, None, ["Окно достижимости не пересекается с полем листа ни в одном "
                                             "допустимом повороте: вписать некуда (проверь окно, стол и лист)"])
    s_fit, area = best_fit(bbox, areas)
    if area is None:
        return Placement(0, 0, 0, None, ["Чертёж пустой или поле листа слишком маленькое"])
    errors: list[str] = []
    warnings: list[str] = []
    if mode in ("fit", "fit_reach", "fit_passes"):
        s = s_fit
    elif mode == "one_to_one":
        s = 1.0
    else:
        s = pl.percent / 100.0
    if mode in ("one_to_one", "percent"):
        bw, bh = (bbox[2] - bbox[0]) * s, (bbox[3] - bbox[1]) * s
        aw, ah = area[2] - area[0], area[3] - area[1]
        if bw > aw + 1e-6 or bh > ah + 1e-6:
            what = "1:1" if mode == "one_to_one" else f"{pl.percent:g}%"
            warnings.append(f"При масштабе {what} чертёж {bw:.1f}×{bh:.1f} мм больше поля листа "
                            f"{aw:.1f}×{ah:.1f} мм; «вписать» дало бы {s_fit * 100:.1f}%")
    if anchor_of(ds) == "zero":
        return Placement(s, -bbox[0] * s + pad + pl.dx, -bbox[1] * s + pad + pl.dy, area, errors, warnings)
    cx = (area[0] + area[2]) / 2 - (bbox[0] + bbox[2]) / 2 * s
    if anchor_of(ds) == "left":
        cx = area[0] - bbox[0] * s
    cy = (area[1] + area[3]) / 2 - (bbox[1] + bbox[3]) / 2 * s
    return Placement(s, cx + pl.dx, cy + pl.dy, area, errors, warnings)


def scale_label(s: float) -> str:
    if s <= 0:
        return "—"
    if abs(s - 1) < 1e-9:
        return "1:1"
    if s < 1:
        return f"1:{_num(1 / s)}"
    return f"{_num(s)}:1"


def _num(v: float) -> str:
    return f"{v:.0f}" if abs(v - round(v)) < 0.005 else f"{v:.2f}".rstrip("0").rstrip(".")
