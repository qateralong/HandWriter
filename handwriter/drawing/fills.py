from __future__ import annotations

import math

import numpy as np
from scipy import ndimage
from skimage.morphology import skeletonize

from ..glyphs.skeleton import SkeletonParams, mask_centerlines, rasterize

Point = tuple[float, float]
PARAMS = SkeletonParams()
REF_HALF_WIDTHS = 25.0


def reference_px(mask: np.ndarray) -> float:
    dt = ndimage.distance_transform_edt(mask)
    sk = skeletonize(mask)
    r = float(np.median(dt[sk])) if sk.any() else 1.0
    return max(8.0, REF_HALF_WIDTHS * max(r, 0.5))


def mask_to_paths(mask: np.ndarray, px_mm: float, x0: float = 0.0, y_top: float = 0.0) -> list[list[Point]]:
    if not mask.any():
        return []
    paths_rc, _, _ = mask_centerlines(mask, reference_px(mask), PARAMS)
    return [[(x0 + c * px_mm, y_top - r * px_mm) for r, c in p] for p in paths_rc if p]


def fill_centerlines(contours: list[list[Point]], max_px: int = 1200, px_per_mm: float = 40.0) -> list[list[Point]]:
    pts = [p for c in contours for p in c]
    if not pts:
        return []
    xmin, xmax = min(p[0] for p in pts), max(p[0] for p in pts)
    ymin, ymax = min(p[1] for p in pts), max(p[1] for p in pts)
    size = max(xmax - xmin, ymax - ymin, 1e-6)
    ppm = min(px_per_mm, max_px / size)
    pad = 3
    w = int(math.ceil((xmax - xmin) * ppm)) + 2 * pad
    h = int(math.ceil((ymax - ymin) * ppm)) + 2 * pad
    to_px = lambda p: ((p[0] - xmin) * ppm + pad, (ymax - p[1]) * ppm + pad)
    mask = rasterize([[to_px(p) for p in c] for c in contours], w, h)
    return mask_to_paths(mask, 1.0 / ppm, xmin + (0.5 - pad) / ppm, ymax - (0.5 - pad) / ppm)
