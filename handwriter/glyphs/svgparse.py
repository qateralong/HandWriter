from __future__ import annotations

import math
import re
import xml.etree.ElementTree as ET

from fontTools.svgLib.path.parser import parse_path

Matrix = tuple[float, float, float, float, float, float]
IDENTITY: Matrix = (1.0, 0.0, 0.0, 1.0, 0.0, 0.0)

_NUM_RE = re.compile(r"[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?")
_TRANSFORM_RE = re.compile(r"(matrix|translate|scale|rotate|skewX|skewY)\s*\(([^)]*)\)")
_SKIP_TAGS = {"defs", "clipPath", "mask", "symbol", "metadata", "title", "desc", "style",
              "font", "font-face", "missing-glyph", "glyph", "marker", "pattern"}


def local_tag(el: ET.Element) -> str:
    tag = el.tag if isinstance(el.tag, str) else ""
    return tag.split("}", 1)[1] if "}" in tag else tag


def num(value: str | None, default: float = 0.0) -> float:
    if value is None:
        return default
    m = _NUM_RE.search(value)
    return float(m.group(0)) if m else default


def mat_mul(m1: Matrix, m2: Matrix) -> Matrix:
    a1, b1, c1, d1, e1, f1 = m1
    a2, b2, c2, d2, e2, f2 = m2
    return (
        a1 * a2 + c1 * b2,
        b1 * a2 + d1 * b2,
        a1 * c2 + c1 * d2,
        b1 * c2 + d1 * d2,
        a1 * e2 + c1 * f2 + e1,
        b1 * e2 + d1 * f2 + f1,
    )


def apply(m: Matrix, p: tuple[float, float]) -> tuple[float, float]:
    a, b, c, d, e, f = m
    x, y = p
    return (a * x + c * y + e, b * x + d * y + f)


def parse_transform(s: str | None) -> Matrix:
    m = IDENTITY
    if not s:
        return m
    for kind, args in _TRANSFORM_RE.findall(s):
        v = [float(x) for x in _NUM_RE.findall(args)]
        if kind == "matrix" and len(v) == 6:
            t = tuple(v)
        elif kind == "translate":
            t = (1, 0, 0, 1, v[0] if v else 0, v[1] if len(v) > 1 else 0)
        elif kind == "scale":
            sx = v[0] if v else 1
            sy = v[1] if len(v) > 1 else sx
            t = (sx, 0, 0, sy, 0, 0)
        elif kind == "rotate":
            a = math.radians(v[0] if v else 0)
            cos, sin = math.cos(a), math.sin(a)
            t = (cos, sin, -sin, cos, 0, 0)
            if len(v) >= 3:
                cx, cy = v[1], v[2]
                t = mat_mul(mat_mul((1, 0, 0, 1, cx, cy), t), (1, 0, 0, 1, -cx, -cy))
        elif kind == "skewX":
            t = (1, 0, math.tan(math.radians(v[0] if v else 0)), 1, 0, 0)
        elif kind == "skewY":
            t = (1, math.tan(math.radians(v[0] if v else 0)), 0, 1, 0, 0)
        else:
            continue
        m = mat_mul(m, t)
    return m


class FlattenPen:
    def __init__(self, matrix: Matrix = IDENTITY, tol: float = 0.001):
        self.m = matrix
        self.tol = tol
        self.strokes: list[list[tuple[float, float]]] = []
        self._cur: list[tuple[float, float]] | None = None

    def _t(self, p):
        return apply(self.m, (float(p[0]), float(p[1])))

    def _flush(self):
        if self._cur:
            self.strokes.append(self._cur)
        self._cur = None

    def moveTo(self, p):
        self._flush()
        self._cur = [self._t(p)]

    def lineTo(self, p):
        if self._cur is None:
            self._cur = [self._t(p)]
        else:
            self._cur.append(self._t(p))

    def curveTo(self, *pts):
        if self._cur is None:
            return
        tp = [self._t(p) for p in pts]
        p0 = self._cur[-1]
        while len(tp) >= 3:
            c1, c2, p3 = tp[0], tp[1], tp[2]
            self._cubic(p0, c1, c2, p3, 0)
            p0 = p3
            tp = tp[3:]

    def qCurveTo(self, *pts):
        if self._cur is None:
            return
        tp = [self._t(p) for p in pts]
        p0 = self._cur[-1]
        c, p2 = tp[0], tp[-1]
        c1 = (p0[0] + 2 / 3 * (c[0] - p0[0]), p0[1] + 2 / 3 * (c[1] - p0[1]))
        c2 = (p2[0] + 2 / 3 * (c[0] - p2[0]), p2[1] + 2 / 3 * (c[1] - p2[1]))
        self._cubic(p0, c1, c2, p2, 0)

    def _cubic(self, p0, p1, p2, p3, depth):
        if depth > 16 or _cubic_flat(p0, p1, p2, p3, self.tol):
            self._cur.append(p3)
            return
        p01 = _mid(p0, p1)
        p12 = _mid(p1, p2)
        p23 = _mid(p2, p3)
        p012 = _mid(p01, p12)
        p123 = _mid(p12, p23)
        p0123 = _mid(p012, p123)
        self._cubic(p0, p01, p012, p0123, depth + 1)
        self._cubic(p0123, p123, p23, p3, depth + 1)

    def closePath(self):
        if self._cur and self._cur[0] != self._cur[-1]:
            self._cur.append(self._cur[0])
        self._flush()

    def endPath(self):
        self._flush()

    def finish(self) -> list[list[tuple[float, float]]]:
        self._flush()
        return self.strokes


def _mid(a, b):
    return ((a[0] + b[0]) / 2, (a[1] + b[1]) / 2)


def _cubic_flat(p0, p1, p2, p3, tol) -> bool:
    return max(_dist_to_segment(p1, p0, p3), _dist_to_segment(p2, p0, p3)) <= tol


def _dist_to_segment(p, a, b) -> float:
    dx, dy = b[0] - a[0], b[1] - a[1]
    L2 = dx * dx + dy * dy
    if L2 == 0:
        return math.hypot(p[0] - a[0], p[1] - a[1])
    t = max(0.0, min(1.0, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / L2))
    return math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy)


def path_d_to_strokes(d: str, matrix: Matrix = IDENTITY, tol: float = 0.001):
    pen = FlattenPen(matrix, tol)
    parse_path(d, pen)
    return pen.finish()


def _points_attr(s: str) -> list[tuple[float, float]]:
    v = [float(x) for x in _NUM_RE.findall(s or "")]
    return list(zip(v[0::2], v[1::2]))


def element_strokes(el: ET.Element, matrix: Matrix, tol: float) -> list[list[tuple[float, float]]]:
    tag = local_tag(el)
    a = el.attrib
    if tag == "path":
        d = a.get("d")
        return path_d_to_strokes(d, matrix, tol) if d else []
    if tag == "line":
        p = [(num(a.get("x1")), num(a.get("y1"))), (num(a.get("x2")), num(a.get("y2")))]
        return [[apply(matrix, q) for q in p]]
    if tag in ("polyline", "polygon"):
        pts = _points_attr(a.get("points", ""))
        if tag == "polygon" and pts and pts[0] != pts[-1]:
            pts.append(pts[0])
        return [[apply(matrix, q) for q in pts]] if pts else []
    if tag == "rect":
        x, y, w, h = num(a.get("x")), num(a.get("y")), num(a.get("width")), num(a.get("height"))
        pts = [(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)]
        return [[apply(matrix, q) for q in pts]]
    if tag in ("circle", "ellipse"):
        cx, cy = num(a.get("cx")), num(a.get("cy"))
        if tag == "circle":
            rx = ry = num(a.get("r"))
        else:
            rx, ry = num(a.get("rx")), num(a.get("ry"))
        d = (f"M{cx - rx},{cy} A{rx},{ry} 0 1 1 {cx + rx},{cy} "
             f"A{rx},{ry} 0 1 1 {cx - rx},{cy} Z")
        return path_d_to_strokes(d, matrix, tol)
    return []


def walk_strokes(el: ET.Element, matrix: Matrix = IDENTITY, tol: float = 0.001,
                 skip_tags: set[str] = _SKIP_TAGS) -> list[list[tuple[float, float]]]:
    out: list[list[tuple[float, float]]] = []
    tag = local_tag(el)
    if tag in skip_tags:
        return out
    if el.attrib.get("display") == "none" or "display:none" in el.attrib.get("style", "").replace(" ", ""):
        return out
    m = mat_mul(matrix, parse_transform(el.attrib.get("transform")))
    out.extend(element_strokes(el, m, tol))
    for child in el:
        out.extend(walk_strokes(child, m, tol, skip_tags))
    return out


def parse_viewbox(root: ET.Element) -> tuple[float, float, float, float] | None:
    vb = root.attrib.get("viewBox")
    if vb:
        v = [float(x) for x in _NUM_RE.findall(vb)]
        if len(v) == 4 and v[2] > 0 and v[3] > 0:
            return v[0], v[1], v[2], v[3]
    w, h = root.attrib.get("width"), root.attrib.get("height")
    if w and h and num(w) > 0 and num(h) > 0:
        return 0.0, 0.0, num(w), num(h)
    return None
