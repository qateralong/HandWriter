from __future__ import annotations

import math
from dataclasses import dataclass, field

import numpy as np

from .passes import covered_mask

Point = tuple[float, float]
Rect = tuple[float, float, float, float]

CURVE_WEIGHT = 4.0
ALONG_WEIGHT = 50.0
CENTER_WEIGHT = 0.05
CURVE_TURN = (0.3, 35.0)
CURVE_SEG_MAX = 15.0
GRID_STEP = 0.25
TRIES = 3


def clip_segments(A: np.ndarray, B: np.ndarray, Q: Rect, eps: float = 1e-9):
    if len(A) == 0:
        return A, B, np.zeros(0, dtype=int)
    x0, y0, x1, y1 = Q
    D = B - A
    n = len(A)
    t0, t1 = np.zeros(n), np.ones(n)
    valid = np.ones(n, dtype=bool)
    with np.errstate(divide="ignore", invalid="ignore"):
        for p, q in ((-D[:, 0], A[:, 0] - x0), (D[:, 0], x1 - A[:, 0]),
                     (-D[:, 1], A[:, 1] - y0), (D[:, 1], y1 - A[:, 1])):
            q = q + eps
            zero = np.abs(p) < 1e-15
            valid &= ~(zero & (q < 0))
            r = np.where(zero, 0.0, q / np.where(zero, 1.0, p))
            t0 = np.where((p < 0) & ~zero, np.maximum(t0, r), t0)
            t1 = np.where((p > 0) & ~zero, np.minimum(t1, r), t1)
    valid &= t0 <= t1 + 1e-12
    idx = np.nonzero(valid)[0]
    return A[idx] + D[idx] * t0[idx, None], A[idx] + D[idx] * t1[idx, None], idx


class Geometry:
    def __init__(self, strokes: list[list[Point]]):
        A, B, sp, curved = [], [], [], []
        self.path_len = np.array([sum(math.dist(s[i], s[i + 1]) for i in range(len(s) - 1)) for s in strokes])
        for k, st in enumerate(strokes):
            if len(st) == 1:
                A.append(st[0]); B.append(st[0]); sp.append(k); curved.append(False)
                continue
            flags = _curved_segments(st)
            for i in range(len(st) - 1):
                A.append(st[i]); B.append(st[i + 1]); sp.append(k); curved.append(flags[i])
        self.A = np.asarray(A, dtype=float).reshape(-1, 2)
        self.B = np.asarray(B, dtype=float).reshape(-1, 2)
        self.seg_path = np.asarray(sp, dtype=int)
        self.curved = np.asarray(curved, dtype=bool)
        self.seg_len = np.hypot(*(self.B - self.A).T) if len(self.A) else np.zeros(0)

    def covered(self, Q: Rect, rects: list[Rect]) -> bool:
        A, B, _ = clip_segments(self.A, self.B, Q)
        return bool(covered_mask(A, B, rects).all()) if len(A) else True

    def seam_cost(self, Q: Rect, axis: int, cands: np.ndarray, ov: float) -> np.ndarray:
        cost = np.zeros(len(cands) + 1)
        A, B, idx = clip_segments(self.A, self.B, Q)
        if len(A):
            lo = np.minimum(A[:, axis], B[:, axis])
            hi = np.maximum(A[:, axis], B[:, axis])
            w = self.path_len[self.seg_path[idx]] * np.where(self.curved[idx], CURVE_WEIGHT, 1.0)
            i0 = np.searchsorted(cands, lo, side="right")
            i1 = np.searchsorted(cands, hi, side="right")
            flat = hi - lo < 1e-12
            i1 = np.where(flat, i0, i1)
            np.add.at(cost, i0, w)
            np.add.at(cost, i1, -w)
            seglen = np.hypot(*(B - A).T)
            along = (hi - lo) < 2 * ov + 1e-9
            if along.any():
                j0 = np.searchsorted(cands, lo[along] - 2 * ov, side="left")
                j1 = np.searchsorted(cands, hi[along] + 2 * ov, side="right")
                wa = seglen[along] * ALONG_WEIGHT
                np.add.at(cost, j0, wa)
                np.add.at(cost, j1, -wa)
        cost = np.cumsum(cost)[:-1]
        mid = (cands[0] + cands[-1]) / 2
        return cost + CENTER_WEIGHT * np.abs(cands - mid)

    def distance_to(self, P: np.ndarray) -> np.ndarray:
        if len(self.A) == 0:
            return np.full(len(P), np.inf)
        out = np.empty(len(P))
        D = self.B - self.A
        L2 = (D ** 2).sum(axis=1)
        for i, p in enumerate(P):
            t = np.where(L2 > 0, ((p - self.A) * D).sum(axis=1) / np.where(L2 > 0, L2, 1), 0.0)
            t = np.clip(t, 0, 1)
            out[i] = np.sqrt((((self.A + D * t[:, None]) - p) ** 2).sum(axis=1)).min()
        return out


def _curved_segments(st: list[Point]) -> list[bool]:
    n = len(st) - 1
    closed = n >= 3 and st[0] == st[-1]
    ang = [None] * (n + 1)

    def turn(a, b, c):
        v1 = (b[0] - a[0], b[1] - a[1])
        v2 = (c[0] - b[0], c[1] - b[1])
        l1, l2 = math.hypot(*v1), math.hypot(*v2)
        if l1 == 0 or l2 == 0:
            return None
        cr = v1[0] * v2[1] - v1[1] * v2[0]
        dt = v1[0] * v2[0] + v1[1] * v2[1]
        return abs(math.degrees(math.atan2(cr, dt)))

    for i in range(1, n):
        ang[i] = turn(st[i - 1], st[i], st[i + 1])
    if closed:
        ang[0] = ang[n] = turn(st[-2], st[0], st[1])
    lo, hi = CURVE_TURN
    ok = lambda a: a is not None and lo <= a <= hi
    out = []
    for i in range(n):
        seg = math.dist(st[i], st[i + 1])
        ends = [a for a in (ang[i], ang[i + 1]) if a is not None]
        out.append(seg <= CURVE_SEG_MAX and bool(ends) and all(ok(a) for a in ends))
    return out


@dataclass
class Node:
    core: Rect
    ext: Rect
    rotation: int | None = None
    axis: int | None = None
    s: float = 0.0
    low: "Node | None" = None
    high: "Node | None" = None
    cost: float = 0.0

    def leaf_at(self, p: Point) -> "Node":
        n = self
        while n.rotation is None:
            n = n.low if p[n.axis] <= n.s else n.high
        return n

    def leaves(self) -> list["Node"]:
        if self.rotation is not None:
            return [self]
        return self.low.leaves() + self.high.leaves()

    def seams(self) -> list["Node"]:
        if self.rotation is not None:
            return []
        return [self] + self.low.seams() + self.high.seams()


def _with(Q: Rect, axis: int, lo: float | None = None, hi: float | None = None) -> Rect:
    q = list(Q)
    if lo is not None:
        q[axis] = max(q[axis], lo)
    if hi is not None:
        q[axis + 2] = min(q[axis + 2], hi)
    return tuple(q)


def _bisect(ok, lo: float, hi: float, want_max: bool) -> float | None:
    if want_max:
        if ok(hi):
            return hi
        if not ok(lo):
            return None
    else:
        if ok(lo):
            return lo
        if not ok(hi):
            return None
    a, b = lo, hi
    for _ in range(60):
        m = (a + b) / 2
        if ok(m) == want_max:
            a = m
        else:
            b = m
        if b - a < 0.005:
            break
    return a if want_max else b


def solve(geo: Geometry, rects: dict[int, Rect], passes: list[int], core: Rect, ext: Rect,
          margin: float, ov: float) -> Node | None:
    for r in passes:
        if geo.covered(ext, [rects[r]]):
            return Node(core, ext, rotation=r)
    if len(passes) < 2:
        return None
    best: Node | None = None
    for axis in (0, 1):
        centre = lambda r: (rects[r][axis] + rects[r][axis + 2]) / 2
        order = sorted(passes, key=lambda r: (centre(r), r))
        for k in range(1, len(order)):
            L, R = order[:k], order[k:]
            lo, hi = core[axis], core[axis + 2]
            left_ok = lambda s: geo.covered(_with(ext, axis, hi=s + margin), [rects[r] for r in L])
            right_ok = lambda s: geo.covered(_with(ext, axis, lo=s - margin), [rects[r] for r in R])
            s_max = _bisect(left_ok, lo, hi, True)
            s_min = _bisect(right_ok, lo, hi, False)
            if s_max is None or s_min is None or s_min > s_max + 1e-9:
                continue
            n = max(1, int(round((s_max - s_min) / GRID_STEP)))
            cands = np.linspace(s_min, s_max, n + 1)
            costs = geo.seam_cost(core, axis, cands, ov)
            tried: list[float] = []
            for i in np.lexsort((cands, costs)):
                s = float(cands[i])
                if any(abs(s - t) < 5.0 for t in tried):
                    continue
                if len(tried) >= TRIES:
                    break
                tried.append(s)
                low = solve(geo, rects, L, _with(core, axis, hi=s), _with(ext, axis, hi=s + margin), margin, ov)
                if low is None:
                    continue
                high = solve(geo, rects, R, _with(core, axis, lo=s), _with(ext, axis, lo=s - margin), margin, ov)
                if high is None:
                    continue
                total = float(costs[i]) + low.cost + high.cost
                if best is None or total < best.cost - 1e-9:
                    best = Node(core, ext, axis=axis, s=s, low=low, high=high, cost=total)
                break
    return best


def _cum(pts: list[Point]) -> list[float]:
    c = [0.0]
    for a, b in zip(pts, pts[1:]):
        c.append(c[-1] + math.dist(a, b))
    return c


def _point_at(pts, cum, t) -> Point:
    if t <= 0:
        return pts[0]
    if t >= cum[-1]:
        return pts[-1]
    i = int(np.searchsorted(cum, t, side="right")) - 1
    i = min(max(i, 0), len(pts) - 2)
    seg = cum[i + 1] - cum[i]
    k = 0.0 if seg == 0 else (t - cum[i]) / seg
    a, b = pts[i], pts[i + 1]
    return (a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k)


def _crossings(pts, cum, axis: int, s: float) -> list[float]:
    out = []
    for i in range(len(pts) - 1):
        d0, d1 = pts[i][axis] - s, pts[i + 1][axis] - s
        if d0 == 0:
            out.append(cum[i])
        elif d0 * d1 < 0:
            out.append(cum[i] + (cum[i + 1] - cum[i]) * d0 / (d0 - d1))
    if pts[-1][axis] == s:
        out.append(cum[-1])
    return out


def _extract(pts, cum, a: float, b: float, closed: bool) -> list[Point]:
    L = cum[-1]
    if closed and L > 0:
        while a < 0:
            a += L
            b += L
        laps = int(math.ceil(b / L)) + 1
        P = list(pts)
        C = list(cum)
        for k in range(1, laps):
            P += pts[1:]
            C += [c + k * L for c in cum[1:]]
        pts, cum = P, C
    else:
        a, b = max(0.0, a), min(L, b)
    out = [_point_at(pts, cum, a)]
    i0 = int(np.searchsorted(cum, a, side="right"))
    i1 = int(np.searchsorted(cum, b, side="left"))
    out += [tuple(pts[i]) for i in range(i0, i1)]
    end = _point_at(pts, cum, b)
    if end != out[-1]:
        out.append(end)
    return out


@dataclass
class CutStats:
    length_in: float = 0.0
    length_out: float = 0.0
    cuts: int = 0
    extension: float = 0.0


def cut_stroke(pts: list[Point], root: Node, ov: float, stats: CutStats | None = None) -> list[tuple[int, list[Point]]]:
    if len(pts) == 1 or pts[0] == pts[-1] and len(set(pts)) == 1:
        return [(root.leaf_at(pts[0]).rotation, [pts[0]])]
    cum = _cum(pts)
    L = cum[-1]
    closed = len(pts) > 2 and pts[0] == pts[-1]
    parts: list[list] = []
    cache: dict[int, list[float]] = {}

    def rec(n: Node, a: float, b: float):
        if n.rotation is not None:
            if parts and parts[-1][0] == n.rotation and abs(parts[-1][2] - a) < 1e-9:
                parts[-1][2] = b
            else:
                parts.append([n.rotation, a, b])
            return
        key = id(n)
        if key not in cache:
            cache[key] = _crossings(pts, cum, n.axis, n.s)
        ts = [t for t in cache[key] if a + 1e-9 < t < b - 1e-9]
        bounds = [a] + ts + [b]
        for x, y in zip(bounds, bounds[1:]):
            if y - x <= 1e-9:
                continue
            m = _point_at(pts, cum, (x + y) / 2)
            rec(n.low if m[n.axis] <= n.s else n.high, x, y)

    rec(root, 0.0, L)
    if closed and len(parts) > 1 and parts[0][0] == parts[-1][0]:
        last = parts.pop()
        parts[0] = [last[0], last[1] - L, parts[0][2]]
    out = []
    if stats is not None:
        stats.length_in += L
    for rot, a, b in parts:
        if len(parts) == 1:
            piece = list(pts)
        else:
            cut_a = closed or a > 1e-9
            cut_b = closed or b < L - 1e-9
            a2 = a - ov if cut_a else a
            b2 = b + ov if cut_b else b
            if not closed:
                a2, b2 = max(0.0, a2), min(L, b2)
            elif b2 - a2 > L:
                b2 = a2 + L
            piece = _extract(pts, cum, a2, b2, closed)
            if stats is not None:
                stats.extension += (b2 - a2) - (b - a)
        if stats is not None:
            stats.length_out += sum(math.dist(piece[i], piece[i + 1]) for i in range(len(piece) - 1))
        out.append((rot, piece))
    if stats is not None:
        stats.cuts += (len(parts) if closed else len(parts) - 1) if len(parts) > 1 else 0
    return out


@dataclass
class Mark:
    x: float
    y: float
    passes: tuple[int, int]


def control_marks(root: Node, geo: Geometry, rects: dict[int, Rect], size: float, count: int,
                  clearance: float = 1.0) -> list[Mark]:
    h = size / 2
    out: list[Mark] = []
    for n in root.seams():
        ax, other = n.axis, 1 - n.axis
        a0, a1 = n.core[other] + h + 1, n.core[other + 2] - h - 1
        if a1 < a0:
            continue
        ts = np.arange(a0, a1 + 1e-9, 1.0)
        good = []
        for t in ts:
            p = [0.0, 0.0]
            p[ax], p[other] = n.s, float(t)
            lo_p, hi_p = list(p), list(p)
            lo_p[ax] -= 1e-6
            hi_p[ax] += 1e-6
            ra, rb = n.leaf_at(tuple(lo_p)).rotation, n.leaf_at(tuple(hi_p)).rotation
            if ra == rb:
                continue
            box = (p[0] - h, p[1] - h, p[0] + h, p[1] + h)
            if all(r[0] <= box[0] and r[1] <= box[1] and box[2] <= r[2] and box[3] <= r[3]
                   for r in (rects[ra], rects[rb])):
                good.append((float(t), tuple(p), (ra, rb)))
        if not good:
            continue
        dist = geo.distance_to(np.array([g[1] for g in good]))
        good = [g for g, d in zip(good, dist) if d > h + clearance]
        if not good:
            continue
        lo_t, hi_t = good[0][0], good[-1][0]
        fracs = [0.5] if count == 1 else [0.12 + 0.76 * i / (count - 1) for i in range(count)]
        chosen: list = []
        for f in fracs:
            target = lo_t + (hi_t - lo_t) * f
            g = min(good, key=lambda g: (abs(g[0] - target), g[0]))
            if all(abs(g[0] - c[0]) >= 4 * size for c in chosen):
                chosen.append(g)
        out += [Mark(g[1][0], g[1][1], g[2]) for g in sorted(chosen)]
    return out


def mark_strokes(m: Mark, size: float) -> list[list[Point]]:
    h = size / 2
    return [[(m.x - h, m.y), (m.x + h, m.y)], [(m.x, m.y - h), (m.x, m.y + h)]]
