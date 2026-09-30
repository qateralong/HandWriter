import math
import re

import numpy as np
import pytest

from handwriter.drawing.ops import path_length
from handwriter.drawing.passes import ROTATIONS, from_pass, pass_dims, to_pass
from handwriter.drawing.pipeline import (compose_drawing, make_all_files, make_part_gcode, make_part_test_gcode,
                                         part_test_strokes, preview_payload)
from handwriter.drawing.sources import clear_cache
from handwriter.drawing.split import Geometry, _crossings, _cum
from handwriter.paths import user_drawings_dir
from handwriter.settings import Settings, Travel

OV = 0.5


@pytest.fixture(autouse=True)
def _fresh_cache():
    clear_cache()
    yield
    clear_cache()


def a4_two_passes(**changes) -> Settings:
    s = Settings()
    s.printer.travel = Travel(x_min=-3, x_max=200, y_min=-3, y_max=215)
    for k, v in changes.items():
        obj = s
        *path, last = k.split("__")
        for p in path:
            obj = getattr(obj, p)
        setattr(obj, last, v)
    return s


def _dist_to_strokes(P: np.ndarray, strokes) -> np.ndarray:
    return Geometry(strokes).distance_to(P) if strokes else np.full(len(P), np.inf)


def _samples(strokes, step=1.0):
    pts = []
    for st in strokes:
        if len(st) == 1:
            pts.append(st[0])
            continue
        cum = _cum(st)
        for t in np.arange(0, cum[-1], step):
            i = min(int(np.searchsorted(cum, t, side="right")) - 1, len(st) - 2)
            seg = cum[i + 1] - cum[i]
            k = 0 if seg == 0 else (t - cum[i]) / seg
            pts.append((st[i][0] + (st[i + 1][0] - st[i][0]) * k, st[i][1] + (st[i + 1][1] - st[i][1]) * k))
    return np.array(pts)


def test_a4_test_drawing_gives_two_passes_and_files():
    c = compose_drawing(a4_two_passes())
    assert c.errors == [], c.errors
    assert [(p.index, p.rotation, p.corner) for p in c.parts] == [(1, 0, "A"), (2, 180, "C")]
    names = [f["filename"] for f in make_all_files(c, tests=True)]
    assert names == ["drawing_test_A4L_185pct_pass1_rot0.gcode", "drawing_test_A4L_185pct_pass1_rot0_test.gcode",
                     "drawing_test_A4L_185pct_pass2_rot180.gcode", "drawing_test_A4L_185pct_pass2_rot180_test.gcode"]
    seams = c.split.seams()
    assert len(seams) == 1 and seams[0].axis == 0
    assert abs(seams[0].s - 148.5) > 2 * OV


def test_each_line_belongs_to_exactly_one_pass():
    s = a4_two_passes()
    s.drawing.weights.enabled = True
    c = compose_drawing(s)
    st = c.cut_stats
    assert st.cuts > 0
    assert st.extension == pytest.approx(2 * OV * st.cuts, abs=1e-6)
    total_parts = sum(path_length(q) for p in c.parts for q in p.strokes[p.marks:])
    assert total_parts == pytest.approx(sum(path_length(q) for q in c.source_strokes) + 2 * OV * st.cuts, abs=1e-6)
    seam = c.split.seams()[0].s
    P = _samples(c.source_strokes, 0.7)
    hit = np.stack([_dist_to_strokes(P, p.strokes[p.marks:]) < 1e-6 for p in c.parts], axis=1)
    far = np.abs(P[:, 0] - seam) > OV + 1e-6
    assert (hit[far].sum(axis=1) == 1).all()
    assert (hit[~far].sum(axis=1) >= 1).all()
    for p in c.parts:
        x0, y0, x1, y1 = p.region
        for q in p.strokes[p.marks:]:
            for x, y in q:
                assert x0 - OV - 1e-6 <= x <= x1 + OV + 1e-6 and y0 - OV - 1e-6 <= y <= y1 + OV + 1e-6


@pytest.mark.parametrize("r", ROTATIONS)
def test_rotation_and_inverse_on_gcode_points(r):
    W, H = 297.0, 210.0
    rng = np.random.default_rng(r)
    for u, v in rng.uniform([0, 0], [W, H], size=(50, 2)):
        x, y = to_pass((u, v), r, W, H)
        Wp, Hp = pass_dims(r, W, H)
        assert 0 <= x <= Wp and 0 <= y <= Hp
        assert from_pass((x, y), r, W, H) == pytest.approx((u, v), abs=1e-9)


def test_gcode_points_map_back_to_the_sheet():
    c = compose_drawing(a4_two_passes())
    W, H = c.layout.width, c.layout.height
    p = c.parts[1]
    code = make_part_gcode(c, p)
    pts = [(float(x), float(y)) for x, y in re.findall(r"^G1 X(-?\d+\.\d+) Y(-?\d+\.\d+)", code, re.M)]
    back = np.array([from_pass(q, 180, W, H) for q in pts])
    assert (_dist_to_strokes(back, p.strokes) < 0.011).all()


def test_gcode_is_deterministic():
    s = a4_two_passes()
    s.drawing.weights.enabled = True
    s.drawing.split.marks = True
    a = make_all_files(compose_drawing(s), tests=True)
    clear_cache()
    b = make_all_files(compose_drawing(Settings.model_validate(s.model_dump(mode="json"))), tests=True)
    assert [f["gcode"] for f in a] == [f["gcode"] for f in b]
    for f in a:
        g = f["gcode"]
        body = [ln for ln in g.splitlines() if not ln.startswith(";")]
        assert body[:7] == ["G21", "G90", "M104 S0", "M140 S0", "M420 S0", "M211 S0", "G92 X0 Y0 Z0"]
        assert body[-2:] == ["G0 X0.00 Y0.00 F3000", "M400"]
        assert "G28" not in g and "M109" not in g and "M190" not in g and all(ord(ch) < 128 for ch in g)


def test_order_inside_pass_long_paths_first():
    s = a4_two_passes()
    c = compose_drawing(s)
    for p in c.parts:
        lens = [path_length(q) for q in p.strokes[p.marks:]]
        long_ = [i for i, v in enumerate(lens) if v >= s.drawing.paths.long_path]
        short = [i for i, v in enumerate(lens) if v < s.drawing.paths.long_path]
        assert not long_ or not short or max(long_) < min(short)


SVG_CIRCLE = """<svg xmlns="http://www.w3.org/2000/svg" width="280mm" height="180mm" viewBox="0 0 280 180">
  <g fill="none" stroke="black" stroke-width="0.25">
    <circle cx="100" cy="90" r="30"/>
    <line x1="10" y1="10" x2="270" y2="10"/>
    <line x1="10" y1="170" x2="270" y2="170"/>
    <line x1="10" y1="90" x2="60" y2="90"/>
  </g>
</svg>"""


def test_seam_avoids_circles():
    (user_drawings_dir() / "circle.svg").write_text(SVG_CIRCLE, encoding="utf-8")
    s = a4_two_passes()
    s.drawing.file = "circle.svg"
    s.drawing.placement.scale_mode = "one_to_one"
    c = compose_drawing(s)
    assert c.errors == [] and len(c.parts) == 2
    seam = c.split.seams()[0].s
    cx = 100 + c.placement.tx
    assert not (cx - 30 - OV <= seam <= cx + 30 + OV), seam
    circle = [q for p in c.parts for q in p.strokes if len(q) > 20]
    assert len(circle) == 1 and circle[0][0] == circle[0][-1]


def test_seam_crosses_least_line_length():
    geo = Geometry([[(0, 0), (100, 0)], [(0, 10), (100, 10)], [(40, 20), (60, 20)]])
    cands = np.linspace(30, 70, 81)
    cost = geo.seam_cost((0, -5, 100, 30), 0, cands, OV)
    best = cands[np.argmin(cost)]
    assert not (40 < best < 60)


def test_a3_four_passes_every_line_once():
    s = Settings()
    s.printer.travel = Travel(x_min=-3, x_max=230, y_min=-3, y_max=225)
    s.printer.table.overhang_y = True
    s.drawing.sheet.format = "A3"
    s.drawing.sheet.orientation = "portrait"
    c = compose_drawing(s)
    assert c.errors == [], c.errors
    assert sorted(p.rotation for p in c.parts) == [0, 90, 180, 270]
    st = c.cut_stats
    assert st.extension == pytest.approx(2 * OV * st.cuts, abs=1e-6)
    P = _samples(c.source_strokes, 1.0)
    hit = np.stack([_dist_to_strokes(P, p.strokes) < 1e-6 for p in c.parts], axis=1)
    near = np.zeros(len(P), dtype=bool)
    for n in c.split.seams():
        near |= np.abs(P[:, n.axis] - n.s) <= OV + 1e-6
    assert (hit[~near].sum(axis=1) == 1).all() and (hit.sum(axis=1) >= 1).all()
    for p in c.parts:
        rect = c.pass_rects[p.rotation]
        for q in p.strokes:
            for x, y in q:
                assert rect[0] - 1e-6 <= x <= rect[2] + 1e-6 and rect[1] - 1e-6 <= y <= rect[3] + 1e-6


def test_offsets_shift_pass_coordinates():
    s = a4_two_passes()
    base = make_part_gcode(c0 := compose_drawing(s), c0.parts[1])
    s.drawing.split.offsets = {"180": (0.7, -0.4)}
    c = compose_drawing(s)
    shifted = make_part_gcode(c, c.parts[1])
    xy = lambda g: np.array(re.findall(r"^G[01] X(-?\d+\.\d+) Y(-?\d+\.\d+)", g, re.M), dtype=float)[:-1]
    assert np.allclose(xy(shifted) - xy(base), [0.7, -0.4], atol=0.011)
    assert "dx 0.70 dy -0.40" in shifted
    assert make_part_gcode(c, c.parts[0]) == make_part_gcode(c0, c0.parts[0])


def test_control_marks_same_sheet_points_in_both_passes():
    s = a4_two_passes()
    s.drawing.split.marks = True
    c = compose_drawing(s)
    assert c.marks and all(m.passes == (0, 180) for m in c.marks)
    seam = c.split.seams()[0].s
    a, b = c.parts
    assert a.marks == b.marks == 2 * len(c.marks)
    assert a.strokes[: a.marks] == b.strokes[: b.marks]
    for m in c.marks:
        assert m.x == pytest.approx(seam)
        for r in (0, 180):
            q = c.pass_rects[r]
            assert q[0] <= m.x - 1.5 and m.x + 1.5 <= q[2] and q[1] <= m.y - 1.5 and m.y + 1.5 <= q[3]
    W, H = c.layout.width, c.layout.height
    pa = c.part_strokes(a)[0][0]
    pb = c.part_strokes(b)[0][0]
    assert from_pass(pa, 0, W, H) == pytest.approx(from_pass(pb, 180, W, H))
    assert not compose_drawing(a4_two_passes()).marks


def test_pass_test_file_corner_mark_and_available_frame():
    s = a4_two_passes()
    c = compose_drawing(s)
    for p in c.parts:
        st = part_test_strokes(c, p)
        frame = st[0]
        assert frame[0] == frame[-1] and len(frame) == 5
        W, H = c.layout.width, c.layout.height
        q = c.pass_rects[p.rotation]
        corners = {tuple(np.round(to_pass(pt, p.rotation, W, H), 6)) for pt in [(q[0], q[1]), (q[2], q[3])]}
        assert corners <= {tuple(np.round(v, 6)) for v in frame}
        ticks = [t for t in st if len(t) == 2 and t[0][0] == t[1][0] and abs(t[1][1] - t[0][1] - 3) < 1e-9]
        assert len(ticks) == p.index
        g = make_part_test_gcode(c, p)
        assert "TEST FILE for PASS" in g and "G92 X0 Y0 Z0" in g


def test_preview_parts_cards_and_api():
    from fastapi.testclient import TestClient
    from handwriter.server import app
    s = a4_two_passes()
    p = preview_payload(compose_drawing(s))
    assert [q["rotation"] for q in p["parts"]] == [0, 180] and p["parts"][1]["corner"] == "C"
    assert {st["k"] for st in p["strokes"]} == {1, 2}
    assert p["parts"][0]["stats"]["time_s"] > 0 and p["seams"]
    client = TestClient(app)
    body = s.model_dump(mode="json")
    r = client.post("/api/drawing/gcode", json=body).json()
    assert [f["filename"].split("_")[-1] for f in r["files"]] == ["rot0.gcode", "rot180.gcode"]
    r = client.post("/api/drawing/gcode?all_tests=true", json=body).json()
    assert len(r["files"]) == 4
    r = client.post("/api/drawing/gcode?part=2&test=true", json=body).json()
    assert r["files"][0]["filename"].endswith("_pass2_rot180_test.gcode")


def test_closed_path_cut_wraps_through_start():
    from handwriter.drawing.split import CutStats, Node, cut_stroke
    root = Node((0, 0, 100, 100), (0, 0, 100, 100), axis=0, s=50,
                low=Node((0, 0, 50, 100), (0, 0, 50, 100), rotation=0),
                high=Node((50, 0, 100, 100), (50, 0, 100, 100), rotation=180))
    square = [(20, 20), (80, 20), (80, 80), (20, 80), (20, 20)]
    st = CutStats()
    parts = cut_stroke(square, root, OV, st)
    assert sorted(r for r, _ in parts) == [0, 180] and st.cuts == 2
    left = [q for r, q in parts if r == 0][0]
    assert math.isclose(path_length(left), 30 + 60 + 30 + 2 * OV)
    assert min(x for x, _ in left) == 20 and max(x for x, _ in left) == pytest.approx(50 + OV)
    assert _crossings(square, _cum(square), 0, 50) == [30.0, 150.0]
