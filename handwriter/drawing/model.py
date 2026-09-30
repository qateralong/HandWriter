from __future__ import annotations

from dataclasses import dataclass, field

Point = tuple[float, float]

UNIT_MM = {
    "mm": 1.0, "cm": 10.0, "m": 1000.0, "in": 25.4, "ft": 304.8,
    "pt": 25.4 / 72, "px": 25.4 / 96, "pc": 25.4 / 6, "q": 0.25,
}
UNIT_NAMES = {"mm": "мм", "cm": "см", "m": "м", "in": "дюймы", "ft": "футы", "pt": "пункты (1/72″)",
              "px": "пиксели (96 на дюйм)"}


@dataclass
class DPath:
    points: list[Point]
    closed: bool = False
    width: float | None = None
    layer: str = ""
    dash: tuple[float, ...] | None = None
    dash_offset: float = 0.0


@dataclass
class TextMark:
    x: float
    y: float
    text: str
    kind: str = "text"


@dataclass
class ImportResult:
    kind: str
    name: str
    paths: list[DPath] = field(default_factory=list)
    texts: list[TextMark] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)
    units: str = "mm"
    units_note: str = ""
    pages: int = 1
    page: int = 1
    layers: dict[str, dict] = field(default_factory=dict)
    info: dict = field(default_factory=dict)
    fill_mask: object = None
    fill_px: float = 0.0

    def fill_bbox(self) -> tuple[float, float, float, float] | None:
        m = self.fill_mask
        if m is None or not m.any():
            return None
        import numpy as np
        rows = np.nonzero(m.any(axis=1))[0]
        cols = np.nonzero(m.any(axis=0))[0]
        px = self.fill_px
        return cols[0] * px, -(rows[-1] + 1) * px, (cols[-1] + 1) * px, -rows[0] * px

    def bbox(self) -> tuple[float, float, float, float] | None:
        xs = [p[0] for pa in self.paths for p in pa.points]
        ys = [p[1] for pa in self.paths for p in pa.points]
        fb = self.fill_bbox()
        if fb:
            xs += [fb[0], fb[2]]
            ys += [fb[1], fb[3]]
        if not xs:
            return None
        return min(xs), min(ys), max(xs), max(ys)
