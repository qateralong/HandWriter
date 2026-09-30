from __future__ import annotations

import math
from dataclasses import dataclass, field

import numpy as np
from scipy import ndimage
from skimage.morphology import skeletonize

from ..geometry import rdp

_N8 = ((-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1))
_K8 = np.array([[1, 1, 1], [1, 0, 1], [1, 1, 1]], dtype=np.uint8)


@dataclass(frozen=True)
class SkeletonParams:
    px_per_em: float = 1500.0
    prune: float = 0.08
    extend: float = 1.0
    smooth: float = 0.03
    simplify: float = 0.004
    junction_merge: float = 2.5

    def key(self) -> str:
        return (f"p{self.px_per_em:g}_r{self.prune:g}_e{self.extend:g}_s{self.smooth:g}"
                f"_t{self.simplify:g}_j{self.junction_merge:g}")


@dataclass
class SkeletonResult:
    strokes: list[list[tuple[float, float]]]
    raw: list[list[tuple[float, float]]] = field(default_factory=list)
    closed: list[bool] = field(default_factory=list)


def rasterize(contours: list[list[tuple[float, float]]], width: int, height: int) -> np.ndarray:
    edges = []
    for c in contours:
        if len(c) < 2:
            continue
        pts = np.asarray(c, dtype=float)
        if not np.allclose(pts[0], pts[-1]):
            pts = np.vstack([pts, pts[:1]])
        edges.append(np.hstack([pts[:-1], pts[1:]]))
    img = np.zeros((height, width), dtype=bool)
    if not edges:
        return img
    e = np.vstack(edges)
    x0, y0, x1, y1 = e.T
    keep = y0 != y1
    x0, y0, x1, y1 = x0[keep], y0[keep], x1[keep], y1[keep]
    wind = np.where(y1 > y0, 1, -1)
    ylo, yhi = np.minimum(y0, y1), np.maximum(y0, y1)
    r0 = np.clip(np.ceil(ylo - 0.5).astype(int), 0, height)
    r1 = np.clip(np.ceil(yhi - 0.5).astype(int), 0, height)
    counts = np.maximum(r1 - r0, 0)
    if counts.sum() == 0:
        return img
    idx = np.repeat(np.arange(len(x0)), counts)
    starts = np.repeat(r0, counts)
    offs = np.arange(counts.sum()) - np.repeat(np.cumsum(counts) - counts, counts)
    rows = starts + offs
    yc = rows + 0.5
    t = (yc - y0[idx]) / (y1[idx] - y0[idx])
    xs = x0[idx] + t * (x1[idx] - x0[idx])
    w = wind[idx]
    order = np.lexsort((xs, rows))
    rows, xs, w = rows[order], xs[order], w[order]
    csum = np.cumsum(w)
    row_start = np.r_[True, rows[1:] != rows[:-1]]
    base = np.maximum.accumulate(np.where(row_start, np.arange(len(rows)), 0))
    prev = np.where(base > 0, csum[base - 1], 0)
    winding = csum - prev
    same_next = np.r_[rows[1:] == rows[:-1], False]
    fill = (winding != 0) & same_next
    k = np.nonzero(fill)[0]
    c0 = np.clip(np.ceil(xs[k] - 0.5).astype(int), 0, width)
    c1 = np.clip(np.ceil(xs[k + 1] - 0.5).astype(int), 0, width)
    diff = np.zeros((height, width + 1), dtype=np.int32)
    np.add.at(diff, (rows[k], c0), 1)
    np.add.at(diff, (rows[k], c1), -1)
    return np.cumsum(diff, axis=1)[:, :width] > 0


class _Graph:
    def __init__(self):
        self.nodes: dict[int, tuple[float, float]] = {}
        self.edges: dict[int, dict] = {}
        self._eid = 0

    def add_edge(self, u, v, pts, closed=False):
        self.edges[self._eid] = {"u": u, "v": v, "pts": pts, "closed": closed}
        self._eid += 1

    def degree(self) -> dict[int, int]:
        deg = {n: 0 for n in self.nodes}
        for e in self.edges.values():
            if e["closed"]:
                continue
            deg[e["u"]] += 1
            deg[e["v"]] += 1
        return deg

    def incident(self, n):
        return [eid for eid, e in self.edges.items() if not e["closed"] and n in (e["u"], e["v"])]


def _length(pts) -> float:
    a = np.asarray(pts, dtype=float)
    return float(np.hypot(*np.diff(a, axis=0).T).sum()) if len(a) > 1 else 0.0


def build_graph(skel: np.ndarray) -> _Graph:
    g = _Graph()
    nb = ndimage.convolve(skel.astype(np.uint8), _K8, mode="constant") * skel
    pix = set(zip(*np.nonzero(skel)))
    pix = {(int(r), int(c)) for r, c in pix}
    node_mask = skel & (nb != 2)
    lab, n = ndimage.label(node_mask, structure=np.ones((3, 3)))
    nr, nc = np.nonzero(node_mask)
    ids = lab[nr, nc]
    if n:
        cnt = np.bincount(ids, minlength=n + 1)
        cr = np.bincount(ids, weights=nr, minlength=n + 1)
        cc = np.bincount(ids, weights=nc, minlength=n + 1)
        for i in range(1, n + 1):
            g.nodes[i] = (float(cr[i] / cnt[i]), float(cc[i] / cnt[i]))
    node_of = {(int(r), int(c)): int(i) for r, c, i in zip(nr, nc, ids)}

    visited: set = set()

    def neigh(p):
        return [(p[0] + dr, p[1] + dc) for dr, dc in _N8 if (p[0] + dr, p[1] + dc) in pix]

    for p, nid in node_of.items():
        for q in neigh(p):
            if q in node_of or q in visited:
                continue
            visited.add(q)
            path = [g.nodes[nid], q]
            prev, cur = p, q
            end = None
            while True:
                nxt = [r for r in neigh(cur) if r != prev and (r in node_of or r not in visited)]
                if not nxt:
                    break
                r = nxt[0]
                if r in node_of:
                    end = node_of[r]
                    break
                visited.add(r)
                path.append(r)
                prev, cur = cur, r
            if end is None:
                end = max(g.nodes) + 1
                g.nodes[end] = (float(path[-1][0]), float(path[-1][1]))
                path = path[:-1]
            path.append(g.nodes[end])
            g.add_edge(nid, end, [(float(a), float(b)) for a, b in path])

    for p in pix:
        if p in visited or p in node_of:
            continue
        loop = [p]
        visited.add(p)
        prev, cur = None, p
        while True:
            nxt = [r for r in neigh(cur) if r != prev and r not in visited]
            if not nxt:
                break
            prev, cur = cur, nxt[0]
            visited.add(cur)
            loop.append(cur)
        loop.append(loop[0])
        g.add_edge(None, None, [(float(a), float(b)) for a, b in loop], closed=True)
    return g


def _merge_degree2(g: _Graph) -> None:
    changed = True
    while changed:
        changed = False
        deg = g.degree()
        for n, d in deg.items():
            if d != 2:
                continue
            inc = g.incident(n)
            if len(inc) == 1:
                e = g.edges[inc[0]]
                e["closed"] = True
                e["u"] = e["v"] = None
                del g.nodes[n]
                changed = True
                break
            a, b = (g.edges[i] for i in inc)
            pa = a["pts"] if a["v"] == n else a["pts"][::-1]
            ua = a["u"] if a["v"] == n else a["v"]
            pb = b["pts"] if b["u"] == n else b["pts"][::-1]
            vb = b["v"] if b["u"] == n else b["u"]
            for i in inc:
                del g.edges[i]
            del g.nodes[n]
            g.add_edge(ua, vb, pa + pb[1:])
            changed = True
            break


def _radius_at(dt: np.ndarray | None, p) -> float:
    if dt is None:
        return 0.0
    r = min(max(int(round(p[0])), 0), dt.shape[0] - 1)
    c = min(max(int(round(p[1])), 0), dt.shape[1] - 1)
    return float(dt[r, c])


def prune(g: _Graph, min_len: float, dt: np.ndarray | None = None) -> None:
    _merge_degree2(g)
    for _ in range(20):
        deg = g.degree()
        drop: dict[int, list[int]] = {}
        for eid, e in g.edges.items():
            if e["closed"]:
                continue
            u, v = e["u"], e["v"]
            L = _length(e["pts"])
            if u == v and L < min_len + 2 * _radius_at(dt, g.nodes[u]):
                drop.setdefault(u, []).append(eid)
            elif (deg[u] == 1) != (deg[v] == 1):
                j = v if deg[u] == 1 else u
                if L - _radius_at(dt, g.nodes[j]) < min_len:
                    drop.setdefault(j, []).append(eid)
        if not drop:
            break
        for j, eids in drop.items():
            if len(eids) >= deg[j]:
                eids = sorted(eids, key=lambda i: _length(g.edges[i]["pts"]))[:-1]
            for eid in eids:
                e = g.edges.pop(eid)
                for n in (e["u"], e["v"]):
                    if n != j and n in g.nodes and not g.incident(n):
                        del g.nodes[n]
        _merge_degree2(g)
    for eid in [i for i, e in g.edges.items() if e["closed"] and _length(e["pts"]) < min_len]:
        del g.edges[eid]


def contract_junctions(g: _Graph, dt: np.ndarray, factor: float) -> None:
    if factor <= 0:
        return

    def radius(p):
        return _radius_at(dt, p)

    changed = True
    while changed:
        changed = False
        deg = g.degree()
        for eid, e in list(g.edges.items()):
            if e["closed"] or e["u"] == e["v"]:
                continue
            u, v = e["u"], e["v"]
            if deg[u] < 3 or deg[v] < 3:
                continue
            if _length(e["pts"]) > factor * max(radius(g.nodes[u]), radius(g.nodes[v])):
                continue
            pts = e["pts"]
            c = pts[len(pts) // 2]
            del g.edges[eid]
            del g.nodes[v]
            g.nodes[u] = c
            for e2 in g.edges.values():
                if e2["closed"]:
                    continue
                if e2["u"] == v:
                    e2["u"] = u
                if e2["v"] == v:
                    e2["v"] = u
                if e2["u"] == u:
                    e2["pts"][0] = c
                if e2["v"] == u:
                    e2["pts"][-1] = c
            changed = True
            break
    _merge_degree2(g)


def _dir(pts, from_start: bool, look: float) -> np.ndarray:
    a = np.asarray(pts if from_start else pts[::-1], dtype=float)
    acc = 0.0
    j = 1
    for j in range(1, len(a)):
        acc += math.hypot(*(a[j] - a[j - 1]))
        if acc >= look:
            break
    v = a[j] - a[0]
    if not from_start:
        v = -v
    n = math.hypot(*v)
    return v / n if n else np.zeros(2)


def _ll_score(p) -> float:
    return p[1] - p[0]


def traverse(g: _Graph, look: float) -> list[tuple[list, bool, bool, bool]]:
    deg = g.degree()
    unused = {eid for eid, e in g.edges.items() if not e["closed"]}
    out = []
    while unused:
        cnt = {n: 0 for n in g.nodes}
        for eid in unused:
            e = g.edges[eid]
            cnt[e["u"]] += 1
            cnt[e["v"]] += 1
        cands = [n for n, c in cnt.items() if c % 2 == 1] or [n for n, c in cnt.items() if c > 0]
        start = min(cands, key=lambda n: _ll_score(g.nodes[n]))
        cur, incoming, pts = start, None, []
        while True:
            opts = []
            for eid in unused:
                e = g.edges[eid]
                if e["u"] == cur:
                    opts.append((eid, e["pts"], e["v"]))
                if e["v"] == cur and e["u"] != e["v"]:
                    opts.append((eid, e["pts"][::-1], e["u"]))
            if not opts:
                break
            live = [o for o in opts if deg.get(o[2]) != 1] or opts
            if incoming is None:
                eid, p, nxt = max(live, key=lambda o: _dir(o[1], True, look)[1])
            else:
                eid, p, nxt = max(live, key=lambda o: float(np.dot(incoming, _dir(o[1], True, look))))
            unused.discard(eid)
            pts += p if not pts else p[1:]
            incoming = _dir(p, False, look)
            cur = nxt
        closed = cur == start and len(pts) > 2
        out.append((pts, closed, deg.get(start) == 1, deg.get(cur) == 1))
    for e in g.edges.values():
        if e["closed"]:
            out.append((_orient_loop(e["pts"]), True, False, False))
    for n, d in deg.items():
        if d == 0:
            out.append(([g.nodes[n]], False, False, False))
    return out


def _orient_loop(pts):
    a = pts[:-1] if pts[0] == pts[-1] else list(pts)
    x = np.array([p[1] for p in a])
    y = -np.array([p[0] for p in a])
    area = 0.5 * float(np.sum(x * np.roll(y, -1) - np.roll(x, -1) * y))
    if area < 0:
        a = a[::-1]
    i = max(range(len(a)), key=lambda k: a[k][1] - a[k][0])
    a = a[i:] + a[:i]
    return a + [a[0]]


def _smooth(pts, sigma: float, closed: bool) -> np.ndarray:
    a = np.asarray(pts, dtype=float)
    if sigma <= 0 or len(a) < 3:
        return a
    if closed:
        body = a[:-1]
        out = ndimage.gaussian_filter1d(body, sigma, axis=0, mode="wrap")
        return np.vstack([out, out[:1]])
    pad = int(min(len(a) - 1, math.ceil(3 * sigma)))
    padded = np.pad(a, ((pad, pad), (0, 0)), mode="reflect", reflect_type="odd")
    out = ndimage.gaussian_filter1d(padded, sigma, axis=0, mode="nearest")[pad:pad + len(a)]
    out[0], out[-1] = a[0], a[-1]
    return out


def _extend(a: np.ndarray, at_start: bool, dt: np.ndarray, mask: np.ndarray, factor: float, look: float) -> np.ndarray:
    if factor <= 0 or len(a) < 2:
        return a
    p = a[0] if at_start else a[-1]
    r, c = int(round(p[0])), int(round(p[1]))
    if not (0 <= r < dt.shape[0] and 0 <= c < dt.shape[1]):
        return a
    reach = float(dt[r, c]) * factor
    d = -_dir(a.tolist(), True, look) if at_start else _dir(a.tolist(), False, look)
    if not d.any() or reach <= 0:
        return a
    step, dist, last = 0.5, 0.0, p
    while dist + step <= reach:
        q = p + d * (dist + step)
        rr, cc = int(round(q[0])), int(round(q[1]))
        if not (0 <= rr < mask.shape[0] and 0 <= cc < mask.shape[1]) or not mask[rr, cc]:
            break
        dist += step
        last = q
    if dist <= 0:
        return a
    return np.vstack([last[None], a]) if at_start else np.vstack([a, last[None]])


def skeleton_strokes(contours_em: list[list[tuple[float, float]]], x_height_em: float,
                     params: SkeletonParams, want_raw: bool = False) -> SkeletonResult:
    pts = [p for c in contours_em for p in c]
    if not pts:
        return SkeletonResult(strokes=[])
    ppem = params.px_per_em
    xh_px = max(x_height_em * ppem, 1.0)
    xmin = min(p[0] for p in pts)
    ymax = max(p[1] for p in pts)
    xmax = max(p[0] for p in pts)
    ymin = min(p[1] for p in pts)
    pad = 4
    w = int(math.ceil((xmax - xmin) * ppem)) + 2 * pad
    h = int(math.ceil((ymax - ymin) * ppem)) + 2 * pad

    def to_px(p):
        return ((p[0] - xmin) * ppem + pad, (ymax - p[1]) * ppem + pad)

    def to_em(rc):
        return (float(rc[1]) / ppem - pad / ppem + xmin, ymax - (float(rc[0]) / ppem - pad / ppem))

    mask = rasterize([[to_px(p) for p in c] for c in contours_em], w, h)
    if not mask.any():
        return SkeletonResult(strokes=[])
    paths_rc, closed_flags, raw_rc = mask_centerlines(mask, xh_px, params, want_raw)
    raw = [[to_em(p) for p in e] for e in raw_rc]
    strokes = [[to_em(p) for p in pts] for pts in paths_rc]
    order = sorted(range(len(strokes)),
                   key=lambda i: (i != _longest(strokes), min(p[0] for p in strokes[i])))
    return SkeletonResult(strokes=[strokes[i] for i in order], raw=raw,
                          closed=[closed_flags[i] for i in order])


def mask_centerlines(mask: np.ndarray, xh_px: float, params: SkeletonParams, want_raw: bool = False):
    dt = ndimage.distance_transform_edt(mask)
    skel = skeletonize(mask)
    g = build_graph(skel)
    raw = [list(e["pts"]) for e in g.edges.values()] if want_raw else []
    prune(g, params.prune * xh_px, dt)
    contract_junctions(g, dt, params.junction_merge)
    look = max(3.0, 0.08 * xh_px)
    paths = traverse(g, look)

    sigma = params.smooth * xh_px / 2.0
    tol = params.simplify * xh_px
    out, closed_flags = [], []
    for pts_rc, closed, start_end, end_end in paths:
        a = _smooth(pts_rc, sigma, closed)
        if not closed:
            if start_end:
                a = _extend(a, True, dt, mask, params.extend, look)
            if end_end:
                a = _extend(a, False, dt, mask, params.extend, look)
        simp = rdp([(float(r), float(c)) for r, c in a], tol) if len(a) > 2 else [tuple(x) for x in a]
        out.append([(float(r), float(c)) for r, c in simp])
        closed_flags.append(closed)
    return out, closed_flags, raw


def _longest(strokes) -> int:
    return max(range(len(strokes)), key=lambda i: _length([(p[1], p[0]) for p in strokes[i]]))
