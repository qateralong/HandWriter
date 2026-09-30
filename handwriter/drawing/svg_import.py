from __future__ import annotations

import math
import re
import xml.etree.ElementTree as ET

from ..glyphs.svgparse import (IDENTITY, Matrix, apply, local_tag, mat_mul, num, parse_transform,
                               path_d_to_strokes)
from .model import UNIT_MM, UNIT_NAMES, DPath, ImportResult, TextMark

_STYLE_PROPS = ("fill", "stroke", "stroke-width", "stroke-dasharray", "stroke-dashoffset",
                "display", "visibility")
_INHERITED = ("fill", "stroke", "stroke-width", "stroke-dasharray", "stroke-dashoffset", "visibility")
_CONTAINERS = {"svg", "g", "a", "switch", "symbol"}
_SKIP = {"defs", "clipPath", "mask", "marker", "pattern", "metadata", "title", "desc", "style", "script",
         "linearGradient", "radialGradient", "filter", "font", "font-face", "foreignObject", "namedview"}
_TEXT = {"text", "flowRoot"}
_SHAPES = {"path", "line", "polyline", "polygon", "rect", "circle", "ellipse"}
_UNIT_RE = re.compile(r"^\s*([-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?)\s*([a-zA-Z%]*)\s*$")


def length_mm(value: str | None) -> tuple[float, str] | None:
    if not value:
        return None
    m = _UNIT_RE.match(value)
    if not m:
        return None
    unit = (m.group(2) or "px").lower()
    if unit not in UNIT_MM:
        return None
    return float(m.group(1)), unit


def _parse_decls(text: str) -> dict[str, str]:
    out = {}
    for part in text.split(";"):
        if ":" in part:
            k, v = part.split(":", 1)
            k = k.strip().lower()
            v = re.sub(r"\s*!important\s*$", "", v.strip())
            if k in _STYLE_PROPS:
                out[k] = v
    return out


class _Css:
    def __init__(self, root: ET.Element):
        self.rules: list[tuple[tuple[int, int, int], int, str, str | None, str | None, dict]] = []
        order = 0
        for el in root.iter():
            if local_tag(el) != "style" or not el.text:
                continue
            text = re.sub(r"/\*.*?\*/", "", el.text, flags=re.S)
            for sel_text, body in re.findall(r"([^{}]+)\{([^{}]*)\}", text):
                decls = _parse_decls(body)
                if not decls:
                    continue
                for sel in sel_text.split(","):
                    sel = sel.strip()
                    m = re.fullmatch(r"(\*|[a-zA-Z][\w-]*)?(?:\.([\w-]+))?(?:#([\w-]+))?", sel)
                    if not sel or not m:
                        continue
                    tag, cls, ident = m.group(1), m.group(2), m.group(3)
                    spec = (1 if ident else 0, 1 if cls else 0, 1 if tag and tag != "*" else 0)
                    self.rules.append((spec, order, tag if tag != "*" else None, cls, ident, decls))
                    order += 1
        self.rules.sort(key=lambda r: (r[0], r[1]))

    def match(self, el: ET.Element) -> dict[str, str]:
        if not self.rules:
            return {}
        tag = local_tag(el)
        classes = set(el.attrib.get("class", "").split())
        ident = el.attrib.get("id")
        out: dict[str, str] = {}
        for _, _, t, c, i, decls in self.rules:
            if (t is None or t == tag) and (c is None or c in classes) and (i is None or i == ident):
                out.update(decls)
        return out


_WHITE = {"white", "#fff", "#ffffff", "rgb(255,255,255)", "rgb(100%,100%,100%)"}


def _is_none(v: str | None) -> bool:
    return v is None or v.strip().lower() in ("none", "transparent", "")


def _is_white(v: str | None) -> bool:
    return v is not None and v.strip().lower().replace(" ", "") in _WHITE


class _Ctx:
    def __init__(self, res: ImportResult, tol: float, opts, ids: dict[str, ET.Element], css: _Css):
        self.res = res
        self.tol = tol
        self.opts = opts
        self.ids = ids
        self.css = css
        self.skipped_white = 0
        self.skipped_invisible = 0
        self.use_depth = 0
        self.fills_small = 0


def import_svg(data: bytes | str, name: str, opts, tol: float) -> ImportResult:
    res = ImportResult(kind="svg", name=name)
    try:
        root = ET.fromstring(data)
    except ET.ParseError as e:
        res.errors.append(f"SVG не читается: {e}")
        return res
    if local_tag(root) != "svg":
        res.errors.append("Это не SVG: корневой элемент не <svg>")
        return res

    vb = [float(v) for v in re.findall(r"[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?", root.attrib.get("viewBox", ""))]
    vb = vb if len(vb) == 4 and vb[2] > 0 and vb[3] > 0 else None
    w, h = length_mm(root.attrib.get("width")), length_mm(root.attrib.get("height"))
    if opts.units != "auto":
        k = UNIT_MM[opts.units]
        res.units, res.units_note = opts.units, "указано вручную: 1 единица файла = 1 " + UNIT_NAMES[opts.units]
    elif vb and w and h:
        kx = w[0] * UNIT_MM[w[1]] / vb[2]
        ky = h[0] * UNIT_MM[h[1]] / vb[3]
        k = min(kx, ky)
        res.units = "mm" if abs(k - 1) < 1e-9 else w[1]
        res.units_note = (f"по width/height ({root.attrib.get('width')} × {root.attrib.get('height')}) и viewBox: "
                          f"1 единица = {k:.4g} мм")
    elif w and h and not vb:
        k = UNIT_MM[w[1]]
        res.units = w[1]
        res.units_note = f"по width/height: единица {UNIT_NAMES.get(w[1], w[1])}"
    else:
        k = UNIT_MM["px"]
        res.units = "px"
        res.units_note = "в файле нет размеров с единицами: считаю 96 px на дюйм"
    base: Matrix = (k, 0.0, 0.0, -k, 0.0, 0.0)
    if vb:
        base = mat_mul(base, (1.0, 0.0, 0.0, 1.0, -vb[0], -vb[1]))

    ids = {el.attrib["id"]: el for el in root.iter() if "id" in el.attrib}
    ctx = _Ctx(res, tol, opts, ids, _Css(root))
    style = {"fill": "black", "stroke": "none", "stroke-width": "1", "stroke-dasharray": "none",
             "stroke-dashoffset": "0", "visibility": "visible"}
    _walk(root, base, style, ctx, is_root=True)

    if ctx.skipped_white:
        res.warnings.append(f"Белые заливки и обводки (фон) пропущены: {ctx.skipped_white}")
    if ctx.fills_small:
        res.warnings.append(f"Мелкие закрашенные фигуры превращены в центральные линии: {ctx.fills_small}")
    return res


def _computed(el: ET.Element, parent: dict, ctx: _Ctx) -> dict:
    own = {k: el.attrib[k] for k in _STYLE_PROPS if k in el.attrib}
    own.update(ctx.css.match(el))
    if "style" in el.attrib:
        own.update(_parse_decls(el.attrib["style"]))
    st = {k: parent[k] for k in _INHERITED}
    st["display"] = "inline"
    for k, v in own.items():
        if v.strip() == "inherit":
            continue
        st[k] = v.strip()
    return st


def _walk(el: ET.Element, matrix: Matrix, parent_style: dict, ctx: _Ctx, is_root: bool = False) -> None:
    tag = local_tag(el)
    if tag in _SKIP or (tag == "symbol" and ctx.use_depth == 0):
        return
    st = _computed(el, parent_style, ctx)
    if st["display"] == "none":
        return
    m = mat_mul(matrix, parse_transform(el.attrib.get("transform")))
    if tag == "svg" and not is_root:
        m = mat_mul(m, (1.0, 0.0, 0.0, 1.0, num(el.attrib.get("x")), num(el.attrib.get("y"))))
    if tag in _TEXT:
        _text(el, m, ctx)
        return
    if tag == "image":
        x, y = apply(m, (num(el.attrib.get("x")), num(el.attrib.get("y"))))
        ctx.res.texts.append(TextMark(x, y, "встроенная картинка", "image"))
        return
    if tag == "use":
        href = el.attrib.get("href") or el.attrib.get("{http://www.w3.org/1999/xlink}href") or ""
        target = ctx.ids.get(href.lstrip("#"))
        if target is None or ctx.use_depth > 8:
            return
        m2 = mat_mul(m, (1.0, 0.0, 0.0, 1.0, num(el.attrib.get("x")), num(el.attrib.get("y"))))
        ctx.use_depth += 1
        _walk(target, m2, st, ctx)
        ctx.use_depth -= 1
        return
    if tag in _SHAPES:
        _shape(el, tag, m, st, ctx)
        return
    if tag in _CONTAINERS or is_root:
        for child in el:
            _walk(child, m, st, ctx)


def _text(el: ET.Element, m: Matrix, ctx: _Ctx) -> None:
    text = " ".join("".join(el.itertext()).split())
    if not text:
        return
    x = y = None
    for sub in el.iter():
        if x is None and sub.attrib.get("x"):
            x, y = num(sub.attrib.get("x")), num(sub.attrib.get("y"))
    px, py = apply(m, (x or 0.0, y or 0.0))
    ctx.res.texts.append(TextMark(px, py, text))


def _shape_d(el: ET.Element, tag: str) -> str | None:
    a = el.attrib
    if tag == "path":
        return a.get("d")
    if tag == "line":
        return f"M{num(a.get('x1'))},{num(a.get('y1'))} L{num(a.get('x2'))},{num(a.get('y2'))}"
    if tag in ("polyline", "polygon"):
        v = [float(x) for x in re.findall(r"[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?", a.get("points", ""))]
        if len(v) < 2:
            return None
        pts = list(zip(v[0::2], v[1::2]))
        return "M" + " L".join(f"{x},{y}" for x, y in pts) + (" Z" if tag == "polygon" else "")
    if tag == "rect":
        x, y, w, h = num(a.get("x")), num(a.get("y")), num(a.get("width")), num(a.get("height"))
        if w <= 0 or h <= 0:
            return None
        rx, ry = a.get("rx"), a.get("ry")
        rx = num(rx) if rx is not None else (num(ry) if ry is not None else 0.0)
        ry = num(ry) if ry is not None else rx
        rx, ry = min(rx, w / 2), min(ry, h / 2)
        if rx <= 0 or ry <= 0:
            return f"M{x},{y} H{x + w} V{y + h} H{x} Z"
        return (f"M{x + rx},{y} H{x + w - rx} A{rx},{ry} 0 0 1 {x + w},{y + ry} V{y + h - ry} "
                f"A{rx},{ry} 0 0 1 {x + w - rx},{y + h} H{x + rx} A{rx},{ry} 0 0 1 {x},{y + h - ry} "
                f"V{y + ry} A{rx},{ry} 0 0 1 {x + rx},{y} Z")
    if tag in ("circle", "ellipse"):
        cx, cy = num(a.get("cx")), num(a.get("cy"))
        if tag == "circle":
            rx = ry = num(a.get("r"))
        else:
            rx, ry = num(a.get("rx")), num(a.get("ry"))
        if rx <= 0 or ry <= 0:
            return None
        return (f"M{cx + rx},{cy} A{rx},{ry} 0 1 1 {cx - rx},{cy} "
                f"A{rx},{ry} 0 1 1 {cx + rx},{cy} Z")
    return None


def _shape(el: ET.Element, tag: str, m: Matrix, st: dict, ctx: _Ctx) -> None:
    if st.get("visibility") in ("hidden", "collapse"):
        ctx.skipped_invisible += 1
        return
    d = _shape_d(el, tag)
    if not d:
        return
    try:
        subpaths = path_d_to_strokes(d, m, ctx.tol)
    except Exception:
        ctx.res.warnings.append(f"Не разобран путь {el.attrib.get('id', tag)}")
        return
    subpaths = [p for p in subpaths if p]
    if not subpaths:
        return
    scale = math.sqrt(abs(m[0] * m[3] - m[1] * m[2]))
    stroke, fill = st.get("stroke"), st.get("fill")
    if not _is_none(stroke):
        if _is_white(stroke):
            ctx.skipped_white += 1
            return
        width = num(st.get("stroke-width"), 1.0) * scale
        dash = None
        da = st.get("stroke-dasharray", "none")
        if not _is_none(da):
            vals = [abs(float(v)) * scale for v in re.findall(r"[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?", da)]
            if vals and sum(vals) > 0:
                dash = tuple(vals)
        off = num(st.get("stroke-dashoffset"), 0.0) * scale
        for p in subpaths:
            closed = len(p) > 2 and p[0] == p[-1]
            ctx.res.paths.append(DPath(p, closed, width, "", dash, off))
        return
    if _is_none(fill) or tag == "line":
        ctx.skipped_invisible += 1
        return
    if _is_white(fill):
        ctx.skipped_white += 1
        return
    add_filled(subpaths, ctx.res, ctx.opts, counter=ctx)


def add_filled(subpaths, res: ImportResult, opts, counter=None, layer: str = "", outline_big: bool = True) -> None:
    closed_sub = [p if p[0] == p[-1] else p + [p[0]] for p in subpaths if len(set(p)) >= 3]
    if not closed_sub:
        return
    if opts.fill_centerlines:
        xs = [q[0] for p in closed_sub for q in p]
        ys = [q[1] for p in closed_sub for q in p]
        if min(max(xs) - min(xs), max(ys) - min(ys)) <= opts.fill_centerline_max:
            from .fills import fill_centerlines
            lines = fill_centerlines(closed_sub)
            if lines:
                for p in lines:
                    res.paths.append(DPath(p, len(p) > 2 and p[0] == p[-1], None, layer))
                if counter is not None:
                    counter.fills_small += 1
                return
    if not outline_big:
        return
    for p in closed_sub:
        res.paths.append(DPath(p, True, None, layer))
