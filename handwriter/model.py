from __future__ import annotations

from dataclasses import dataclass, field

Point = tuple[float, float]


@dataclass
class PlacedGlyph:
    word: int
    letter: int
    line: int
    char: str
    glyph: str | None
    x: float
    y: float
    advance: float
    missing: bool = False
    hyphen: bool = False
    segment: int = 0
    adv_em: float = 0.0
    dx_em: float = 0.0
    dy_em: float = 0.0
    size: float = 1.0
    slant: float = 0.0
    voff: float = 0.0


@dataclass
class DrawnStroke:
    points: list[Point]
    word: int
    letter: int
    line: int
    hyphen: bool = False
    tags: list[int] = field(default_factory=list)
