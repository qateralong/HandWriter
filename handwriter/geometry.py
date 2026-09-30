from __future__ import annotations

import math

Point = tuple[float, float]


def rdp(points: list[Point], tol: float) -> list[Point]:
    return [points[i] for i in rdp_indices(points, tol)]


def rdp_indices(points: list[Point], tol: float, must_keep=()) -> list[int]:
    n = len(points)
    if n < 3 or tol <= 0:
        return list(range(n))
    keep = [False] * n
    keep[0] = keep[-1] = True
    for i in must_keep:
        if 0 <= i < n:
            keep[i] = True
    stack = []
    fixed = [i for i in range(n) if keep[i]]
    for a, b in zip(fixed, fixed[1:]):
        if b - a > 1:
            stack.append((a, b))
    while stack:
        i, j = stack.pop()
        ax, ay = points[i]
        bx, by = points[j]
        dx, dy = bx - ax, by - ay
        L = math.hypot(dx, dy)
        best, idx = -1.0, -1
        for k in range(i + 1, j):
            px, py = points[k]
            if L == 0:
                d = math.hypot(px - ax, py - ay)
            else:
                d = abs(dy * (px - ax) - dx * (py - ay)) / L
            if d > best:
                best, idx = d, k
        if best > tol:
            keep[idx] = True
            stack.append((i, idx))
            stack.append((idx, j))
    return [i for i in range(n) if keep[i]]


def make_transform(rotation_deg: float, dx: float, dy: float):
    a = math.radians(rotation_deg)
    c, s = math.cos(a), math.sin(a)

    def t(p: Point) -> Point:
        x, y = p
        return (c * x - s * y + dx, s * x + c * y + dy)
    return t


def polyline_length(points: list[Point]) -> float:
    return sum(math.dist(points[i], points[i + 1]) for i in range(len(points) - 1))
