from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field

Point = tuple[float, float]
Stroke = tuple[Point, ...]


@dataclass(frozen=True)
class FontMetrics:
    x_height: float
    cap_height: float
    ascent: float
    descent: float
    x_height_source: str = ""


@dataclass(frozen=True)
class Glyph:
    name: str
    strokes: tuple[Stroke, ...]
    advance: float

    def bbox(self) -> tuple[float, float, float, float] | None:
        pts = [p for s in self.strokes for p in s]
        if not pts:
            return None
        xs = [p[0] for p in pts]
        ys = [p[1] for p in pts]
        return min(xs), min(ys), max(xs), max(ys)


@dataclass(frozen=True)
class ShapedGlyph:
    name: str
    cluster: int
    advance: float
    x_offset: float = 0.0
    y_offset: float = 0.0


@dataclass
class FontInfo:
    name: str
    mode: str
    source: str
    glyph_count: int
    chars: str
    variants: dict[str, list[str]] = field(default_factory=dict)
    metrics: FontMetrics | None = None
    notes: list[str] = field(default_factory=list)
    features: list[str] = field(default_factory=list)
    gpos_features: list[str] = field(default_factory=list)
    variant_sources: dict[str, dict[str, list[str]]] = field(default_factory=dict)
    ligatures: list[dict] = field(default_factory=list)


class GlyphProvider(ABC):
    mode: str = ""

    @property
    @abstractmethod
    def name(self) -> str: ...

    @property
    @abstractmethod
    def metrics(self) -> FontMetrics: ...

    @abstractmethod
    def glyph(self, name: str) -> Glyph:
        pass

    @abstractmethod
    def glyph_names_for_char(self, ch: str) -> list[str]:
        pass

    def has_char(self, ch: str) -> bool:
        return bool(self.glyph_names_for_char(ch))

    def prepare(self, names) -> None:
        pass

    def with_options(self, options) -> "GlyphProvider":
        return self

    def glyph_names(self) -> list[str]:
        return sorted({n for ch in self.info().chars for n in self.glyph_names_for_char(ch)})

    def debug_glyph(self, name: str) -> dict:
        g = self.glyph(name)
        return {"name": g.name, "outline": [], "strokes": [list(s) for s in g.strokes], "raw": [],
                "closed": [len(s) > 2 and s[0] == s[-1] for s in g.strokes], "advance": g.advance, "ms": 0}

    def variants(self) -> dict[str, list[str]]:
        return {}

    def advance(self, name: str) -> float:
        return self.glyph(name).advance

    def variant_pool(self, ch: str) -> list[str]:
        return self.glyph_names_for_char(ch)

    def space_advance(self) -> float:
        names = self.glyph_names_for_char(" ")
        if names:
            adv = self.glyph(names[0]).advance
            if adv > 0:
                return adv
        return 0.3

    def shape(self, text: str) -> list[ShapedGlyph]:
        out: list[ShapedGlyph] = []
        for i, ch in enumerate(text):
            names = self.glyph_names_for_char(ch)
            if not names:
                continue
            g = self.glyph(names[0])
            out.append(ShapedGlyph(name=g.name, cluster=i, advance=g.advance))
        return out

    def info(self) -> FontInfo:
        return FontInfo(name=self.name, mode=self.mode, source="", glyph_count=0, chars="")
