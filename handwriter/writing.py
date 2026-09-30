from __future__ import annotations

import math
from dataclasses import dataclass

from .geometry import make_transform, rdp_indices
from .glyphs import GlyphProvider
from .model import DrawnStroke, PlacedGlyph, Point
from .rand import vnoise
from .settings import Settings

DRIFT_WAVELENGTH_MM = 40.0
JITTER_WAVELENGTH_MM = 2.5
RESAMPLE_MM = 0.4
BRIDGE_STEP_MM = 0.25
GLUE_EM_FRACTION = 0.02
DETAIL_EM_FRACTION = 0.45


@dataclass
class _Elem:
    gi: int
    si: int
    rev: bool
    join: str


def _oriented(st, rev):
    return st[::-1] if rev else st


def _diag(st) -> float:
    xs = [p[0] for p in st]
    ys = [p[1] for p in st]
    return math.hypot(max(xs) - min(xs), max(ys) - min(ys))


def build_paths(s: Settings, prov: GlyphProvider, glyphs: list[PlacedGlyph], scale: float) -> list[DrawnStroke]:
    xh = prov.metrics.x_height
    conn = s.connections
    glue_d = GLUE_EM_FRACTION * xh
    detail_d = DETAIL_EM_FRACTION * xh

    local = {}
    for gi, g in enumerate(glyphs):
        if g.glyph is not None:
            st = [list(x) for x in prov.glyph(g.glyph).strokes if x]
            if st:
                local[gi] = st

    segments: dict[int, list[int]] = {}
    for gi, g in enumerate(glyphs):
        segments.setdefault(g.segment, []).append(gi)

    elems: list[_Elem] = []
    for seg_id in sorted(segments):
        elems += _order_segment(segments[seg_id], glyphs, local,
                                conn.distance * xh if conn.enabled else -1.0, glue_d, detail_d)

    polylines = _assemble(elems, glyphs, local, scale, glue_d * scale, xh * scale)
    return _finish(s, glyphs, polylines)


def _order_segment(seg: list[int], glyphs, local, conn_d: float, glue_d: float, detail_d: float) -> list[_Elem]:
    origin = {}
    ox = 0.0
    for gi in seg:
        g = glyphs[gi]
        origin[gi] = (ox + g.dx_em, g.dy_em)
        ox += g.adv_em

    entry: dict[int, int] = {}
    exit_: dict[int, int] = {}
    orient: dict[tuple[int, int], bool] = {}
    connected: set[tuple[int, int]] = set()

    if conn_d >= 0:
        for ga, gb in zip(seg, seg[1:]):
            if ga not in local or gb not in local or glyphs[ga].hyphen or glyphs[gb].hyphen:
                continue
            best = None
            ax, ay = origin[ga]
            bx, by = origin[gb]
            for sa, sta in enumerate(local[ga]):
                if len(sta) < 2:
                    continue
                for ra in (False, True):
                    if (ga, sa) in orient and orient[(ga, sa)] != ra:
                        continue
                    pa = _oriented(sta, ra)[-1]
                    for sb, stb in enumerate(local[gb]):
                        if len(stb) < 2:
                            continue
                        for rb in (False, True):
                            pb = _oriented(stb, rb)[0]
                            d = math.hypot(pa[0] + ax - pb[0] - bx, pa[1] + ay - pb[1] - by)
                            cand = (d, ra, rb, sa, sb)
                            if best is None or cand < best:
                                best = cand
            if best is not None and best[0] <= conn_d:
                d, ra, rb, sa, sb = best
                exit_[ga], entry[gb] = sa, sb
                orient[(ga, sa)] = ra
                orient[(gb, sb)] = rb
                connected.add((ga, gb))

    main: list[_Elem] = []
    details: list[tuple[int, int]] = []
    prev_gi = None
    for gi in seg:
        if gi not in local:
            prev_gi = None
            continue
        strokes = local[gi]
        E, X = entry.get(gi), exit_.get(gi)
        others = [si for si in range(len(strokes)) if si not in (E, X)]
        if glyphs[gi].hyphen:
            mains = list(others)
        else:
            mains = [si for si in others if _diag(strokes[si]) >= detail_d]
        small = [si for si in others if si not in mains]
        details += [(gi, si) for si in small]
        seq: list[tuple[int, bool]] = []
        if E is not None:
            seq.append((E, orient[(gi, E)]))
        if E is not None and E == X:
            details += [(gi, si) for si in mains]
        else:
            cur = _oriented(strokes[E], orient[(gi, E)])[-1] if E is not None else None
            seq += _chain(strokes, mains, cur, glue_d)
            if X is not None:
                seq.append((X, orient[(gi, X)]))
        for k, (si, rev) in enumerate(seq):
            if k == 0:
                join = "bridge" if (prev_gi, gi) in connected and si == E else "lift"
            else:
                prev_end = _oriented(strokes[seq[k - 1][0]], seq[k - 1][1])[-1]
                start = _oriented(strokes[si], rev)[0]
                join = "glue" if math.dist(prev_end, start) <= glue_d else "lift"
            main.append(_Elem(gi, si, rev, join))
        prev_gi = gi

    def left(item):
        gi, si = item
        return (min(p[0] for p in local[gi][si]) + origin[gi][0], gi, si)

    out = list(main)
    last = None
    for gi, si in sorted(details, key=left):
        st = local[gi][si]
        rev = False
        join = "lift"
        if last is not None and last[0] == gi:
            end = _oriented(local[gi][last[1]], last[2])[-1]
            if math.dist(end, st[0]) <= glue_d:
                join = "glue"
            elif math.dist(end, st[-1]) <= glue_d:
                join, rev = "glue", True
        out.append(_Elem(gi, si, rev, join))
        last = (gi, si, rev)
    return out


def _chain(strokes, pool: list[int], cur: Point | None, glue_d: float) -> list[tuple[int, bool]]:
    rest = list(pool)
    out = []
    while rest:
        pick = None
        if cur is not None:
            for si in rest:
                st = strokes[si]
                if math.dist(cur, st[0]) <= glue_d:
                    pick = (si, False)
                    break
                if math.dist(cur, st[-1]) <= glue_d:
                    pick = (si, True)
                    break
        if pick is None:
            pick = (rest[0], False)
        rest.remove(pick[0])
        out.append(pick)
        cur = _oriented(strokes[pick[0]], pick[1])[-1]
    return out


def _glyph_transform(g: PlacedGlyph, scale: float):
    k = scale * g.size
    sh = math.tan(math.radians(g.slant))
    ox, oy = g.x, g.y + g.voff

    def t(p: Point) -> Point:
        x, y = p
        return (ox + (g.dx_em + x + y * sh) * k, oy + (g.dy_em + y) * k)
    return t


def _direction(pts: list[Point], at_end: bool, look: float = 0.3) -> tuple[float, float] | None:
    seq = pts[::-1] if at_end else pts
    p0 = seq[0]
    for q in seq[1:]:
        if math.dist(p0, q) >= look:
            v = (p0[0] - q[0], p0[1] - q[1]) if at_end else (q[0] - p0[0], q[1] - p0[1])
            n = math.hypot(*v)
            return (v[0] / n, v[1] / n)
    if len(seq) > 1 and seq[-1] != p0:
        q = seq[-1]
        v = (p0[0] - q[0], p0[1] - q[1]) if at_end else (q[0] - p0[0], q[1] - p0[1])
        n = math.hypot(*v)
        return (v[0] / n, v[1] / n)
    return None


def _bridge(p0: Point, t0, p1: Point, t1) -> list[Point]:
    d = math.dist(p0, p1)
    if d < 0.02:
        return []
    chord = ((p1[0] - p0[0]) / d, (p1[1] - p0[1]) / d)
    if t0 is None or t0[0] * chord[0] + t0[1] * chord[1] <= 0:
        t0 = chord
    if t1 is None or t1[0] * chord[0] + t1[1] * chord[1] <= 0:
        t1 = chord
    h = 0.4 * d
    c1 = (p0[0] + t0[0] * h, p0[1] + t0[1] * h)
    c2 = (p1[0] - t1[0] * h, p1[1] - t1[1] * h)
    n = max(2, math.ceil(d / BRIDGE_STEP_MM))
    out = []
    for i in range(1, n):
        u = i / n
        a, b, c, e = (1 - u) ** 3, 3 * u * (1 - u) ** 2, 3 * u * u * (1 - u), u ** 3
        out.append((a * p0[0] + b * c1[0] + c * c2[0] + e * p1[0], a * p0[1] + b * c1[1] + c * c2[1] + e * p1[1]))
    return out


def _cut_tail(pts: list[Point], tags: list[int], length: float, floor: int) -> None:
    while length > 1e-9 and len(pts) - 1 > floor:
        a, b = pts[-2], pts[-1]
        seg = math.dist(a, b)
        if seg <= length:
            pts.pop()
            tags.pop()
            length -= seg
        else:
            u = 1 - length / seg
            pts[-1] = (a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u)
            break


def _cut_head(pts: list[Point], length: float) -> list[Point]:
    i = 0
    while length > 1e-9 and i < len(pts) - 1:
        a, b = pts[i], pts[i + 1]
        seg = math.dist(a, b)
        if seg <= length:
            length -= seg
            i += 1
        else:
            u = length / seg
            return [(a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u)] + pts[i + 1:]
    return pts[i:]


def _length(pts: list[Point]) -> float:
    return sum(math.dist(pts[i], pts[i + 1]) for i in range(len(pts) - 1))


def _assemble(elems: list[_Elem], glyphs, local, scale: float, glue_mm: float, xh_mm: float):
    tf = {}
    out = []
    pts: list[Point] = []
    tags: list[int] = []
    keep: set[int] = set()

    def flush():
        nonlocal pts, tags, keep
        if pts:
            out.append((pts, tags, keep))
        pts, tags, keep = [], [], set()

    for e in elems:
        if e.gi not in tf:
            tf[e.gi] = _glyph_transform(glyphs[e.gi], scale)
        new = [tf[e.gi](p) for p in _oriented(local[e.gi][e.si], e.rev)]
        if e.join == "lift" or not pts:
            flush()
        elif e.join == "bridge":
            gap = math.dist(pts[-1], new[0])
            floor = max([i for i in keep if i < len(pts)] + [0])
            want = min(max(1.5 * gap, 0.12 * xh_mm), 0.35 * xh_mm)
            cut_a = min(want, 0.4 * _length(pts[floor:]))
            cut_b = min(want, 0.4 * _length(new))
            _cut_tail(pts, tags, cut_a, floor)
            new = _cut_head(new, cut_b)
            mid = _bridge(pts[-1], _direction(pts, True), new[0], _direction(new, False))
            keep.add(len(pts) - 1)
            new = mid + new
        elif e.join == "glue" and math.dist(pts[-1], new[0]) <= max(glue_mm, 1e-9):
            new = new[1:]
        if pts and tags[-1] != e.gi and new:
            keep.add(len(pts) - 1)
            keep.add(len(pts))
        pts += new
        tags += [e.gi] * len(new)
    flush()
    return out


def _resample(pts, tags, keep, step: float):
    if len(pts) < 2:
        return pts, tags, keep
    npts, ntags, nkeep = [pts[0]], [tags[0]], set()
    if 0 in keep:
        nkeep.add(0)
    for i in range(1, len(pts)):
        a, b = pts[i - 1], pts[i]
        n = int(math.dist(a, b) / step)
        for k in range(1, n + 1):
            u = k / (n + 1)
            npts.append((a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u))
            ntags.append(tags[i])
        if i in keep:
            nkeep.add(len(npts))
        npts.append(b)
        ntags.append(tags[i])
    return npts, ntags, nkeep


def _finish(s: Settings, glyphs: list[PlacedGlyph], polylines) -> list[DrawnStroke]:
    R = s.randomness
    ty = s.typography
    noisy = R.enabled and (R.drift > 0 or R.jitter > 0)
    t = make_transform(ty.rotation_deg, ty.dx, ty.dy)
    seen: dict[tuple[int, int], int] = {}
    out = []
    for pts, tags, keep in polylines:
        g0 = glyphs[tags[0]]
        k = seen.get((g0.word, g0.letter), 0)
        seen[(g0.word, g0.letter)] = k + 1
        if noisy:
            if R.jitter > 0:
                pts, tags, keep = _resample(pts, tags, keep, RESAMPLE_MM)
            moved = []
            s_len = 0.0
            for i, (x, y) in enumerate(pts):
                if i:
                    s_len += math.dist(pts[i - 1], pts[i])
                g = glyphs[tags[i]]
                if R.drift > 0:
                    y += R.drift * vnoise(R.seed, "drift", x / DRIFT_WAVELENGTH_MM, g.line)
                if R.jitter > 0:
                    u = s_len / JITTER_WAVELENGTH_MM
                    x += R.jitter * vnoise(R.seed, "jx", u, g0.word, g0.letter, k)
                    y += R.jitter * vnoise(R.seed, "jy", u, g0.word, g0.letter, k)
                moved.append((x, y))
            pts = moved
        pts = [t(p) for p in pts]
        idx = rdp_indices(pts, s.printer.simplify_tol, sorted(keep))
        pts = [pts[i] for i in idx]
        tags = [tags[i] for i in idx]
        out.append(DrawnStroke(points=pts, tags=tags, word=g0.word, letter=g0.letter, line=g0.line,
                               hyphen=all(glyphs[i].hyphen for i in tags)))
    return out
