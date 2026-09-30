from __future__ import annotations

from pathlib import Path

from ..paths import resolve_font_path
from .base import FontInfo, FontMetrics, Glyph, GlyphProvider, ShapedGlyph
from .outline_provider import OutlineGlyphProvider
from .stroke_provider import StrokeGlyphProvider

__all__ = ["FontInfo", "FontMetrics", "Glyph", "GlyphProvider", "ShapedGlyph", "OutlineGlyphProvider",
           "StrokeGlyphProvider", "load_provider", "mode_for_path", "PROVIDERS"]

PROVIDERS = {
    "strokes": StrokeGlyphProvider.from_path,
    "outlines": OutlineGlyphProvider.from_path,
}


def mode_for_path(p: Path) -> str:
    return "outlines" if p.suffix.lower() in (".ttf", ".otf") else "strokes"

_cache: dict[tuple, GlyphProvider] = {}


def _stamp(p: Path) -> float:
    if p.is_dir():
        return max([p.stat().st_mtime] + [f.stat().st_mtime for f in p.iterdir()])
    return p.stat().st_mtime


def load_provider(font_spec: str, mode: str = "strokes") -> GlyphProvider:
    if mode not in PROVIDERS:
        raise ValueError(f"Режим «{mode}» пока не поддерживается")
    path = resolve_font_path(font_spec)
    if not path.exists():
        raise FileNotFoundError(f"Шрифт не найден: {path}")
    if mode_for_path(path) != mode:
        need = "«Контуры»" if mode_for_path(path) == "outlines" else "«Штрихи»"
        raise ValueError(f"Для шрифта {path.name} нужен режим {need}")
    key = (mode, str(path), _stamp(path))
    prov = _cache.pop(key, None)
    if prov is None:
        prov = PROVIDERS[mode](path)
    _cache[key] = prov
    while len(_cache) > 4:
        _cache.pop(next(iter(_cache)))
    return prov
