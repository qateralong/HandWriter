from __future__ import annotations

import re
from types import SimpleNamespace

from ..glyphs.svgparse import FlattenPen
from .model import UNIT_MM, DPath, ImportResult, TextMark
from .svg_import import add_filled

PT = UNIT_MM["pt"]


def _white(c) -> bool:
    return c is not None and len(c) >= 3 and min(c[:3]) > 0.95


def _dashes(s: str | None, k: float) -> tuple[tuple[float, ...] | None, float]:
    if not s:
        return None, 0.0
    m = re.match(r"\s*\[([^\]]*)\]\s*([-+\d.eE]*)", s)
    if not m:
        return None, 0.0
    vals = [abs(float(v)) * k for v in m.group(1).split()]
    off = float(m.group(2) or 0) * k
    return (tuple(vals) if vals and sum(vals) > 0 else None), off


def import_pdf(data: bytes, name: str, opts, tol_mm: float) -> ImportResult:
    import pymupdf

    res = ImportResult(kind="pdf", name=name, units="pt", units_note="PDF: пункты (1/72 дюйма), размер как на бумаге")
    try:
        doc = pymupdf.open(stream=data, filetype="pdf")
    except Exception as e:
        res.errors.append(f"PDF не читается: {e}")
        return res
    res.pages = doc.page_count
    if not doc.page_count:
        res.errors.append("В PDF нет страниц")
        return res
    page_no = min(max(opts.pdf_page, 1), doc.page_count)
    if page_no != opts.pdf_page:
        res.warnings.append(f"В PDF {doc.page_count} стр., взята страница {page_no}")
    res.page = page_no
    page = doc[page_no - 1]
    rot = page.rotation_matrix

    def tr(p):
        q = pymupdf.Point(p) * rot
        return (q.x * PT, -q.y * PT)

    white = 0
    counter = SimpleNamespace(fills_small=0)
    for d in page.get_drawings():
        subpaths: list[list[tuple[float, float]]] = []
        pen = None

        def start(p):
            nonlocal pen
            if pen is not None:
                subpaths.extend(pen.finish())
            pen = FlattenPen(tol=tol_mm)
            pen.moveTo(tr(p))

        last = None
        for it in d["items"]:
            op = it[0]
            if op == "l":
                a, b = it[1], it[2]
                if last is None or abs(a.x - last.x) > 1e-6 or abs(a.y - last.y) > 1e-6:
                    start(a)
                pen.lineTo(tr(b))
                last = b
            elif op == "c":
                a, c1, c2, b = it[1], it[2], it[3], it[4]
                if last is None or abs(a.x - last.x) > 1e-6 or abs(a.y - last.y) > 1e-6:
                    start(a)
                pen.curveTo(tr(c1), tr(c2), tr(b))
                last = b
            elif op in ("re", "qu"):
                q = it[1].quad if op == "re" else it[1]
                start(q.ul)
                for p in (q.ur, q.lr, q.ll, q.ul):
                    pen.lineTo(tr(p))
                pen.closePath()
                last = None
        if pen is not None:
            if d.get("closePath"):
                pen.closePath()
            subpaths.extend(pen.finish())
        subpaths = [p for p in subpaths if len(p) >= 1]
        if not subpaths:
            continue
        typ = d.get("type") or ""
        layer = d.get("layer") or ""
        if "s" in typ:
            if _white(d.get("color")):
                white += 1
                continue
            width = (d.get("width") or 0.0) * PT
            dash, off = _dashes(d.get("dashes"), PT)
            for p in subpaths:
                closed = len(p) > 2 and p[0] == p[-1]
                res.paths.append(DPath(p, closed, width, layer, dash, off))
        elif "f" in typ:
            if _white(d.get("fill")):
                white += 1
                continue
            add_filled(subpaths, res, opts, counter=counter, layer=layer)
    fills_small = counter.fills_small

    td = page.get_text("dict")
    for block in td.get("blocks", []):
        if block.get("type") == 1:
            x0, y0, x1, y1 = block["bbox"]
            x, y = tr((x0, y1))
            res.texts.append(TextMark(x, y, "встроенная картинка", "image"))
            continue
        for line in block.get("lines", []):
            text = " ".join("".join(s.get("text", "") for s in line.get("spans", [])).split())
            if text:
                x0, y0, x1, y1 = line["bbox"]
                x, y = tr((x0, y1))
                res.texts.append(TextMark(x, y, text))
    if white:
        res.warnings.append(f"Белые заливки и обводки (фон) пропущены: {white}")
    if fills_small:
        res.warnings.append(f"Мелкие закрашенные фигуры превращены в центральные линии: {fills_small}")
    layers: dict[str, dict] = {}
    for p in res.paths:
        if p.layer:
            layers.setdefault(p.layer, {"count": 0, "width": None, "linetype": ""})["count"] += 1
    res.layers = layers
    return res

