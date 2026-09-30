import math
import re
from pathlib import Path

import numpy as np
import pytest

from handwriter.glyphs import OutlineGlyphProvider, load_provider
from handwriter.glyphs.skeleton import SkeletonParams, rasterize, skeleton_strokes
from handwriter.pipeline import compose, make_gcode
from handwriter.settings import Travel

FONTS = Path(__file__).parent / "fonts"
BAD = FONTS / "BadScript-Regular.ttf"
MARCK = FONTS / "MarckScript-Regular.ttf"
P = SkeletonParams()


def rect(x0, y0, x1, y1):
    return [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]


def circle(cx, cy, r, n=120, ccw=True):
    pts = [(cx + r * math.cos(2 * math.pi * i / n), cy + r * math.sin(2 * math.pi * i / n)) for i in range(n + 1)]
    return pts if ccw else pts[::-1]


def length(st):
    return sum(math.dist(st[i], st[i + 1]) for i in range(len(st) - 1))


def test_rasterize_square_area():
    img = rasterize([[(10, 10), (60, 10), (60, 40), (10, 40), (10, 10)]], 80, 50)
    assert img.sum() == 50 * 30


def test_rasterize_nonzero_union_and_hole():
    outer = [(0, 0), (40, 0), (40, 40), (0, 40), (0, 0)]
    same = [(10, 10), (30, 10), (30, 30), (10, 30), (10, 10)]
    assert rasterize([outer, same], 50, 50).sum() == 1600
    assert rasterize([outer, same[::-1]], 50, 50).sum() == 1600 - 400


def test_bar_gives_one_stroke_left_to_right_with_full_length():
    bar = rect(0.1, 0.0, 0.9, 0.08)
    res = skeleton_strokes([bar], 0.5, P)
    assert len(res.strokes) == 1
    st = res.strokes[0]
    assert st[0][0] < st[-1][0]
    assert abs(st[0][1] - 0.04) < 0.01
    assert st[0][0] < 0.1 + 0.015 and st[-1][0] > 0.9 - 0.015
    short = skeleton_strokes([bar], 0.5, SkeletonParams(extend=0)).strokes[0]
    assert length(st) > length(short) + 0.05


def test_ring_is_closed_loop_from_top_right_counterclockwise():
    ring = [circle(0.5, 0.5, 0.3), circle(0.5, 0.5, 0.24, ccw=False)]
    res = skeleton_strokes(ring, 0.5, P)
    assert len(res.strokes) == 1 and res.closed == [True]
    st = res.strokes[0]
    assert math.dist(st[0], st[-1]) < 1e-9
    x0, y0 = st[0]
    assert x0 > 0.55 and y0 > 0.55
    area = 0.5 * sum(st[i][0] * st[i + 1][1] - st[i + 1][0] * st[i][1] for i in range(len(st) - 1))
    assert area > 0


def test_T_shape_has_two_strokes_and_no_spurs():
    bar = rect(0.1, 0.62, 0.9, 0.7)
    stem = rect(0.46, 0.0, 0.54, 0.66)
    res = skeleton_strokes([bar, stem], 0.5, P)
    assert len(res.strokes) == 2
    assert all(length(s) > 0.3 for s in res.strokes)


def test_crossing_is_passed_straight():
    w = 0.04
    d1 = [(0.1, 0.1 - w), (0.9 + w, 0.9), (0.9, 0.9 + w), (0.1 - w, 0.1), (0.1, 0.1 - w)]
    d2 = [(0.1, 0.9 + w), (0.1 - w, 0.9), (0.9, 0.1 - w), (0.9 + w, 0.1), (0.1, 0.9 + w)]
    res = skeleton_strokes([d1, d2], 0.5, P)
    assert len(res.strokes) == 2
    for st in res.strokes:
        assert math.dist(st[0], st[-1]) > 0.9


def test_short_bump_is_pruned_but_long_tail_kept():
    bar = rect(0.1, 0.0, 0.9, 0.08)
    bump = rect(0.5, 0.06, 0.54, 0.11)
    tail = rect(0.3, 0.06, 0.34, 0.4)
    assert len(skeleton_strokes([bar, bump], 0.5, P).strokes) == 1
    assert len(skeleton_strokes([bar, tail], 0.5, P).strokes) == 2


def test_dot_survives():
    res = skeleton_strokes([circle(0.5, 0.5, 0.03)], 0.5, P)
    assert len(res.strokes) == 1


@pytest.fixture(scope="module", params=[BAD, MARCK], ids=["BadScript", "MarckScript"])
def prov(request):
    return OutlineGlyphProvider.from_path(request.param)


def test_full_cyrillic_and_measured_x_height(prov):
    abc = "абвгдеёжзийклмнопрстуфхцчшщъыьэюя"
    assert all(prov.has_char(c) for c in abc + abc.upper())
    assert 0.3 < prov.metrics.x_height < 0.6
    assert "по глифу" in prov.metrics.x_height_source


def test_privet_strokes_stay_inside_and_reach_ends(prov):
    for sg in prov.shape("Привет"):
        g = prov.glyph(sg.name)
        cont = [p for c in prov.data.contours(sg.name) for p in c]
        xs, ys = [p[0] for p in cont], [p[1] for p in cont]
        pts = [p for s in g.strokes for p in s]
        assert g.strokes, sg.name
        assert min(p[0] for p in pts) >= min(xs) - 0.01 and max(p[0] for p in pts) <= max(xs) + 0.01
        assert min(p[1] for p in pts) >= min(ys) - 0.01 and max(p[1] for p in pts) <= max(ys) + 0.01
        tol = 0.25 * prov.metrics.x_height
        assert max(p[1] for p in pts) > max(ys) - tol, sg.name
        assert min(p[1] for p in pts) < min(ys) + tol, sg.name


def count_holes(img: np.ndarray, min_px: int = 20) -> int:
    from scipy import ndimage
    lab, n = ndimage.label(~img)
    border = set(np.unique(np.concatenate([lab[0], lab[-1], lab[:, 0], lab[:, -1]])))
    sizes = ndimage.sum(~img, lab, range(1, n + 1))
    return sum(1 for i in range(1, n + 1) if i not in border and sizes[i - 1] >= min_px)


def raster(strokes, contours, ppem=300, pencil_em=0.035):
    from scipy import ndimage
    from skimage.draw import line
    pts = [p for c in contours for p in c]
    x0, y1 = min(p[0] for p in pts) - 0.05, max(p[1] for p in pts) + 0.05
    w = int((max(p[0] for p in pts) - x0 + 0.05) * ppem) + 1
    h = int((y1 - min(p[1] for p in pts) + 0.05) * ppem) + 1
    to = lambda p: ((p[0] - x0) * ppem, (y1 - p[1]) * ppem)
    mask = rasterize([[to(p) for p in c] for c in contours], w, h)
    ink = np.zeros((h, w), dtype=bool)
    for s in strokes:
        for a, b in zip(s, s[1:]):
            (ax, ay), (bx, by) = to(a), to(b)
            for dx in (0, 1):
                rr, cc = line(int(ay), int(ax) + dx, int(by), int(bx) + dx)
                ok = (rr >= 0) & (rr < h) & (cc >= 0) & (cc < w)
                ink[rr[ok], cc[ok]] = True
    ink = ndimage.binary_dilation(ink, iterations=max(1, int(pencil_em * ppem / 2)))
    return mask, ink


def test_hole_check_detects_broken_loop(prov):
    name = next(n for n in (prov.data.cmap[ord(c)] for c in "оОвбд")
                if count_holes(raster([], prov.data.contours(n))[0]) > 0)
    strokes = prov.glyph(name).strokes
    top = max((p[1], i, k) for i, s in enumerate(strokes) for k, p in enumerate(s))
    _, i, k = top
    broken = [s for j, s in enumerate(strokes) if j != i]
    s = strokes[i]
    gap = 0.15
    before = [p for p in s[:k] if math.dist(p, s[k]) > gap]
    after = [p for p in s[k:] if math.dist(p, s[k]) > gap]
    broken += [before, after]
    mask, ink = raster([b for b in broken if len(b) > 1], prov.data.contours(name))
    assert count_holes(ink) < count_holes(mask)


@pytest.mark.parametrize("ch", ["о", "в", "б", "О", "а", "д", "е", "я", "ф"])
def test_loops_not_lost(prov, ch):
    name = prov.data.cmap[ord(ch)]
    mask, ink = raster(prov.glyph(name).strokes, prov.data.contours(name))
    assert count_holes(ink) >= count_holes(mask), ch


def test_glyph_by_name_and_id(prov):
    name = prov.data.cmap[ord("в")]
    gid = prov.data.order.index(name)
    assert prov.glyph(f"#{gid}").strokes == prov.glyph(name).strokes
    with pytest.raises(KeyError):
        prov.glyph("no_such_glyph")


def test_missing_char_is_not_shaped(prov):
    assert not prov.has_char("∑")
    assert [g.cluster for g in prov.shape("а∑б")] == [0, 2]


def test_bad_script_variants_and_ligatures():
    p = OutlineGlyphProvider.from_path(BAD)
    info = p.info()
    assert "ss01" in info.features and "liga" in info.features
    assert "uni0412.ss01" in p.glyph_names_for_char("В")
    assert "ss01" in info.variant_sources["В"]["uni0412.ss01"]
    assert {"chars": "ffi", "glyph": "f_f_i", "feature": "liga"} in info.ligatures
    assert "1" not in p.variants() and "a" not in p.variants()
    assert p.glyph("uni0412.ss01").strokes
    sh = p.shape("ffi")
    assert [(g.name, g.cluster) for g in sh] == [("f_f_i", 0)]


def test_marck_has_no_variants():
    p = OutlineGlyphProvider.from_path(MARCK)
    assert p.variants() == {} and p.info().ligatures == []


def test_ligature_letters_numbered_by_cluster(settings):
    s = settings
    s.font, s.mode, s.text = str(BAD), "outlines", "office"
    c = compose(s)
    assert c.errors == []
    lig = [g for g in c.layout.glyphs if g.glyph == "f_f_i"]
    assert len(lig) == 1 and lig[0].letter == 2
    assert {st.letter for st in c.layout.strokes} <= {1, 2, 5, 6}


def test_disk_cache_roundtrip():
    p1 = OutlineGlyphProvider.from_path(MARCK)
    g1 = p1.glyph(p1.data.cmap[ord("ж")])
    p2 = OutlineGlyphProvider.from_path(MARCK)
    key = p2.params.key()
    p2.data.load_disk(key)
    assert (g1.name, key) in p2.data.glyph_cache
    cached = p2.glyph(g1.name)
    assert cached.strokes == g1.strokes


def test_disk_cache_keeps_only_recent_param_sets():
    p = OutlineGlyphProvider.from_path(MARCK)
    name = p.data.cmap[ord("л")]
    for i in range(9):
        p.with_options({"smooth": 0.02 + i * 0.001}).glyph(name)
    files = list(p.data._cache_file("x").parent.glob(f"{p.data.hash}_*.jsonl"))
    assert len(files) <= 6


def test_params_change_result():
    p = OutlineGlyphProvider.from_path(MARCK)
    name = p.data.cmap[ord("т")]
    a = p.glyph(name)
    b = p.with_options({"extend": 0.0}).glyph(name)
    assert sum(map(length, a.strokes)) > sum(map(length, b.strokes))


@pytest.mark.parametrize("font", [BAD, MARCK], ids=["BadScript", "MarckScript"])
def test_privet_gcode_passes_checks(settings, font):
    s = settings
    s.font, s.mode, s.text = str(font), "outlines", "\tПривет"
    s.printer.travel = Travel(x_min=-2, x_max=200, y_min=-2, y_max=230)
    c = compose(s)
    assert c.errors == []
    code = make_gcode(c)
    t, m = s.printer.travel, s.printer.safety_margin
    for x, y in ((float(a), float(b)) for a, b in re.findall(r"X(-?\d+\.\d+) Y(-?\d+\.\d+)", code)):
        if (x, y) != (0.0, 0.0):
            assert t.x_min + m <= x <= t.x_max - m and t.y_min + m <= y <= t.y_max - m
    assert "G28" not in code and "M104 S0" in code
    i_strokes = [st.points for st in c.layout.strokes if st.letter == 3]
    ys = [p[1] for st in i_strokes for p in st]
    assert max(ys) - c.layout.baselines[0] == pytest.approx(s.typography.size_mm, rel=0.2)


def test_mode_mismatch_is_reported():
    with pytest.raises(ValueError, match="Контуры"):
        load_provider(str(BAD), "strokes")
