from __future__ import annotations

import hashlib
import threading
from collections import OrderedDict
from pathlib import Path

from ..i18n import tr
from ..paths import user_drawings_dir
from .model import ImportResult

BUILTIN_TEST = "builtin:test"
EXTENSIONS = {".svg": "svg", ".dxf": "dxf", ".pdf": "pdf", ".png": "raster", ".jpg": "raster", ".jpeg": "raster"}

TEST_SVG = """<svg xmlns="http://www.w3.org/2000/svg" width="160mm" height="110mm" viewBox="0 0 160 110">
  <g fill="none" stroke="#000">
    <rect x="5" y="5" width="150" height="100" stroke-width="0.7"/>
    <line x1="5" y1="105" x2="155" y2="5" stroke-width="0.25"/>
    <line x1="5" y1="5" x2="155" y2="105" stroke-width="0.25"/>
    <circle cx="80" cy="55" r="35" stroke-width="0.7"/>
    <g stroke-width="0.25" stroke-dasharray="12 3 1 3">
      <line x1="35" y1="55" x2="125" y2="55"/>
      <line x1="80" y1="12" x2="80" y2="98"/>
    </g>
  </g>
</svg>
"""

_cache: OrderedDict = OrderedDict()
_lock = threading.Lock()
CACHE_SIZE = 8


def file_kind(name: str) -> str | None:
    return EXTENSIONS.get(Path(name).suffix.lower())


def resolve(spec: str) -> Path:
    return user_drawings_dir() / Path(spec).name


def list_files() -> list[dict]:
    out = [{"spec": BUILTIN_TEST, "label": "Встроенный тестовый чертёж", "kind": "svg"}]
    for p in sorted(user_drawings_dir().iterdir()):
        k = file_kind(p.name)
        if p.is_file() and k:
            out.append({"spec": p.name, "label": p.name, "kind": k})
    return out


def _import_key(kind: str, imp) -> tuple:
    if kind == "raster":
        return (imp.threshold_auto, imp.threshold if not imp.threshold_auto else None, imp.invert, imp.raster_dpi)
    common = (imp.fill_centerlines, imp.fill_centerline_max if imp.fill_centerlines else None)
    if kind == "pdf":
        return (imp.pdf_page,) + common
    return (imp.units,) + common


def load_drawing(spec: str, imp, tol_mm: float) -> ImportResult:
    if spec == BUILTIN_TEST:
        data, name, kind = TEST_SVG.encode("utf-8"), tr("Тестовый чертёж"), "svg"
    else:
        path = resolve(spec)
        name = path.name
        kind = file_kind(name)
        if kind is None:
            return ImportResult(kind="?", name=name, errors=[f"Неизвестный тип файла: {name}"])
        if not path.exists():
            return ImportResult(kind=kind, name=name, errors=[f"Файл чертежа не найден: {name}"])
        data = path.read_bytes()
    digest = hashlib.blake2b(data, digest_size=16).hexdigest()
    key = (digest, kind, _import_key(kind, imp), round(tol_mm, 6) if kind != "raster" else None)
    with _lock:
        if key in _cache:
            _cache.move_to_end(key)
            return _cache[key]
    res = _import(kind, data, name, imp, tol_mm)
    if spec == BUILTIN_TEST:
        res.kind = "test"
    with _lock:
        _cache[key] = res
        while len(_cache) > CACHE_SIZE:
            _cache.popitem(last=False)
    return res


def _import(kind: str, data: bytes, name: str, imp, tol_mm: float) -> ImportResult:
    if kind == "svg":
        from .svg_import import import_svg
        return import_svg(data, name, imp, tol_mm)
    if kind == "dxf":
        from .dxf_import import import_dxf
        return import_dxf(data, name, imp, tol_mm)
    if kind == "pdf":
        from .pdf_import import import_pdf
        return import_pdf(data, name, imp, tol_mm)
    from .raster_import import import_raster
    return import_raster(data, name, imp)


def clear_cache() -> None:
    with _lock:
        _cache.clear()
