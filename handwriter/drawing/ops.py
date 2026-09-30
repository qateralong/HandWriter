from __future__ import annotations

import math

import numpy as np

Point = tuple[float, float]


def dedupe(pts: list[Point], eps: float = 1e-9) -> list[Point]:
    out: list[Point] = []
    for p in pts:
        if not out or abs(p[0] - out[-1][0]) > eps or abs(p[1] - out[-1][1]) > eps:
            out.append(p)
        elif len(out) > 1:
            out[-1] = p
    return out


def path_length(pts: list[Point]) -> float:
    return sum(math.dist(pts[i], pts[i + 1]) for i in range(len(pts) - 1))


def dash_polyline(pts: list[Point], pattern, offset: float = 0.0) -> list[list[Point]]:
    pat = [max(0.0, float(v)) for v in pattern]
    if len(pat) % 2:
        pat = pat * 2
    total = sum(pat)
    pts = dedupe(pts)
    if total <= 1e-9 or len(pts) < 2 or not pat:
        return [pts]
    n = len(pat)
    i = 0
    off = offset % total
    for _ in range(4 * n):
        if off < pat[i] or pat[i] == 0 and off <= 0:
            break
        off -= pat[i]
        i = (i + 1) % n
    left = pat[i] - off
    on = i % 2 == 0
    out: list[list[Point]] = []
    cur: list[Point] | None = [pts[0]] if on else None
    for a, b in zip(pts, pts[1:]):
        seg = math.dist(a, b)
        t = 0.0
        while seg - t > left:
            t += left
            k = t / seg
            p = (a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k)
            if on:
                cur.append(p)
                out.append(dedupe(cur))
                cur = None
            else:
                cur = [p]
            on = not on
            i = (i + 1) % n
            left = pat[i]
        left -= seg - t
        if on:
            cur.append(b)
    if on and cur:
        out.append(dedupe(cur))
    return out


def offset_polyline(pts: list[Point], d: float, closed: bool) -> list[Point]:
    pts = dedupe(pts)
    if closed and len(pts) > 2 and pts[0] == pts[-1]:
        body = pts[:-1]
    else:
        body = pts
        closed = False
    n = len(body)
    if n < 2 or d == 0:
        return list(pts)

    def normal(a, b):
        dx, dy = b[0] - a[0], b[1] - a[1]
        L = math.hypot(dx, dy) or 1.0
        return (-dy / L, dx / L)

    segs = [normal(body[i], body[i + 1]) for i in range(n - 1)]
    if closed:
        segs.append(normal(body[-1], body[0]))
    out = []
    for i in range(n):
        if closed:
            n1, n2 = segs[i - 1], segs[i]
        elif i == 0:
            n1 = n2 = segs[0]
        elif i == n - 1:
            n1 = n2 = segs[-1]
        else:
            n1, n2 = segs[i - 1], segs[i]
        bx, by = n1[0] + n2[0], n1[1] + n2[1]
        L = math.hypot(bx, by)
        if L < 1e-9:
            bx, by, k = n2[0], n2[1], d
        else:
            bx, by = bx / L, by / L
            cos_half = bx * n2[0] + by * n2[1]
            k = d / max(cos_half, 0.5)
        out.append((body[i][0] + bx * k, body[i][1] + by * k))
    if closed:
        out.append(out[0])
    return out


def pass_offsets(passes: int, step: float) -> list[float]:
    return [(k - (passes - 1) / 2) * step for k in range(passes)]


def expand_passes(pts: list[Point], closed: bool, passes: int, step: float) -> list[list[Point]]:
    if passes <= 1 or len(pts) < 2:
        return [pts]
    out = []
    for k, d in enumerate(pass_offsets(passes, step)):
        q = offset_polyline(pts, d, closed)
        if not closed and k % 2 == 1:
            q = q[::-1]
        out.append(q)
    return out


def join_paths(paths: list[list[Point]], tol: float) -> list[list[Point]]:
    if tol <= 0 or len(paths) < 2:
        return [list(p) for p in paths]
    cell = max(tol, 1e-6)
    grid: dict[tuple[int, int], list[tuple[int, int]]] = {}
    is_open = [len(p) >= 2 and math.dist(p[0], p[-1]) > tol for p in paths]

    def key(p):
        return (math.floor(p[0] / cell), math.floor(p[1] / cell))

    for i, p in enumerate(paths):
        if is_open[i]:
            grid.setdefault(key(p[0]), []).append((i, 0))
            grid.setdefault(key(p[-1]), []).append((i, 1))
    used = [not o for o in is_open]

    def direction(a, b):
        dx, dy = b[0] - a[0], b[1] - a[1]
        L = math.hypot(dx, dy)
        return (dx / L, dy / L) if L else (0.0, 0.0)

    def best_next(end: Point, heading):
        kx, ky = key(end)
        best = None
        for gx in (kx - 1, kx, kx + 1):
            for gy in (ky - 1, ky, ky + 1):
                for j, which in grid.get((gx, gy), ()):
                    if used[j]:
                        continue
                    q = paths[j]
                    p0 = q[0] if which == 0 else q[-1]
                    if math.dist(p0, end) > tol:
                        continue
                    seq = q if which == 0 else q[::-1]
                    d = direction(seq[0], seq[1])
                    turn = -(heading[0] * d[0] + heading[1] * d[1])
                    cand = (turn, j, seq)
                    if best is None or cand[:2] < best[:2]:
                        best = cand
        return best

    out = []
    for i, p in enumerate(paths):
        if used[i]:
            if not is_open[i]:
                out.append(list(p))
            continue
        used[i] = True
        chain = list(p)
        for _ in range(2):
            while True:
                nxt = best_next(chain[-1], direction(chain[-2], chain[-1]))
                if nxt is None:
                    break
                used[nxt[1]] = True
                chain += nxt[2][1:]
            chain.reverse()
        if len(chain) > 2 and math.dist(chain[0], chain[-1]) <= tol:
            chain[-1] = chain[0]
        out.append(chain)
    return out


def order_paths(paths: list[list[Point]], long_path: float,
                start: Point = (0.0, 0.0)) -> list[tuple[int, list[Point]]]:
    if not paths:
        return []
    lengths = [path_length(p) for p in paths]
    groups = [[i for i in range(len(paths)) if lengths[i] >= long_path],
              [i for i in range(len(paths)) if lengths[i] < long_path]]
    out: list[tuple[int, list[Point]]] = []
    pos = np.array(start, dtype=float)
    for g in groups:
        if not g:
            continue
        cand_xy, cand_owner, cand_vertex = [], [], []
        for i in g:
            p = paths[i]
            closed = len(p) > 2 and p[0] == p[-1]
            if closed:
                for k in range(len(p) - 1):
                    cand_xy.append(p[k]); cand_owner.append(i); cand_vertex.append(k)
            else:
                cand_xy.append(p[0]); cand_owner.append(i); cand_vertex.append(0)
                if len(p) > 1:
                    cand_xy.append(p[-1]); cand_owner.append(i); cand_vertex.append(-1)
        xy = np.asarray(cand_xy, dtype=float)
        owner = np.asarray(cand_owner)
        first = {}
        for k, i in enumerate(cand_owner):
            first.setdefault(i, [k, k])[1] = k + 1
        near = _Nearest(xy)
        for _ in range(len(g)):
            c = near.pop_nearest(pos)
            i, v = int(owner[c]), cand_vertex[c]
            near.kill(*first[i])
            p = paths[i]
            if v == -1:
                seq = p[::-1]
            elif v == 0:
                seq = list(p)
            else:
                body = p[:-1]
                seq = body[v:] + body[:v] + [body[v]]
            out.append((i, seq))
            pos = np.array(seq[-1], dtype=float)
    return out


class _Nearest:
    def __init__(self, xy: np.ndarray):
        self.xy = xy
        self.alive = np.ones(len(xy), dtype=bool)
        self._build()

    def _build(self):
        from scipy.spatial import cKDTree
        self.ids = np.nonzero(self.alive)[0]
        self.tree = cKDTree(self.xy[self.ids]) if len(self.ids) else None
        self.dead_in_tree = 0

    def kill(self, a: int, b: int) -> None:
        n = int(self.alive[a:b].sum())
        self.alive[a:b] = False
        self.dead_in_tree += n
        if self.dead_in_tree > max(64, len(self.ids) // 2) and self.alive.any():
            self._build()

    def pop_nearest(self, pos) -> int:
        n = len(self.ids)
        k = min(8, n)
        while True:
            d, ii = self.tree.query(pos, k=k)
            d, ii = np.atleast_1d(d), np.atleast_1d(ii)
            ids = self.ids[ii]
            ok = self.alive[ids]
            if ok.any():
                dmin = d[ok].min()
                if k >= n or d[-1] > dmin:
                    return int(ids[ok & (d == dmin)].min())
            elif k >= n:
                raise RuntimeError("нет живых точек")
            k = min(n, k * 4)


def outside_parts(pts: list[Point], box: tuple[float, float, float, float], eps: float = 1e-9) -> list[list[Point]]:
    x0, y0, x1, y1 = box
    out: list[list[Point]] = []
    cur: list[Point] | None = None

    def inside(p):
        return x0 - eps <= p[0] <= x1 + eps and y0 - eps <= p[1] <= y1 + eps

    if len(pts) == 1:
        return [] if inside(pts[0]) else [list(pts)]
    for a, b in zip(pts, pts[1:]):
        t0, t1 = _clip(a, b, box)
        lerp = lambda t: (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
        if t0 is None:
            if cur is None:
                cur = [a]
            cur.append(b)
            continue
        if t0 > 1e-9:
            if cur is None:
                cur = [a]
            cur.append(lerp(t0))
        if cur is not None:
            out.append(cur)
            cur = None
        if t1 < 1 - 1e-9:
            cur = [lerp(t1), b]
    if cur is not None:
        out.append(cur)
    return out


def _clip(a: Point, b: Point, box):
    x0, y0, x1, y1 = box
    dx, dy = b[0] - a[0], b[1] - a[1]
    t0, t1 = 0.0, 1.0
    for p, q in ((-dx, a[0] - x0), (dx, x1 - a[0]), (-dy, a[1] - y0), (dy, y1 - a[1])):
        if abs(p) < 1e-15:
            if q < -1e-9:
                return None, None
            continue
        r = q / p
        if p < 0:
            t0 = max(t0, r)
        else:
            t1 = min(t1, r)
        if t0 > t1:
            return None, None
    return t0, t1
