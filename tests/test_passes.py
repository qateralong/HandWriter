import re

import pytest

from handwriter.calibration import make_reach_check_gcode, make_zero_gcode, reach_corners
from handwriter.drawing.passes import (CORNER, ROTATIONS, area, from_pass, lines_covered, pass_dims, plan_sheet,
                                       rotation_rects, to_pass, uncovered)
from handwriter.drawing.pipeline import compose_drawing, make_drawing_gcode
from handwriter.drawing.sources import clear_cache
from handwriter.pipeline import GenerationRefused
from handwriter.settings import Printer, Settings, Travel

A4L = (297.0, 210.0)
A3P = (297.0, 420.0)


@pytest.fixture(autouse=True)
def _fresh_cache():
    clear_cache()
    yield
    clear_cache()


def printer(x0=-3, x1=200, y0=-3, y1=215, ox=True, oy=False, table=None) -> Printer:
    p = Printer(travel=Travel(x_min=x0, x_max=x1, y_min=y0, y_max=y1))
    p.table.overhang_x, p.table.overhang_y = ox, oy
    if table:
        p.table.table_x, p.table.table_y = table
    return p


def field(W, H, m=10.0):
    return (m, m, W - m, H - m)


@pytest.mark.parametrize("r", ROTATIONS)
def test_rotation_and_inverse_give_same_points(r):
    W, H = A4L
    pts = [(0, 0), (W, 0), (W, H), (0, H), (12.3, 45.6), (296.9, 0.1), (148.5, 105)]
    for p in pts:
        q = to_pass(p, r, W, H)
        Wp, Hp = pass_dims(r, W, H)
        assert -1e-9 <= q[0] <= Wp + 1e-9 and -1e-9 <= q[1] <= Hp + 1e-9
        back = from_pass(q, r, W, H)
        assert back == pytest.approx(p, abs=1e-12)
    a, b, c = (to_pass(p, r, W, H) for p in [(0, 0), (10, 0), (0, 10)])
    cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    assert cross == pytest.approx(100)
    assert from_pass((0, 0), r, W, H) == {"A": (0, 0), "B": (W, 0), "C": (W, H), "D": (0, H)}[CORNER[r]]


def test_rotation_180_formula():
    W, H = A4L
    assert to_pass((10, 20), 180, W, H) == (W - 10, H - 20)
    assert to_pass(to_pass((10, 20), 90, W, H), 90, *pass_dims(90, W, H)) == to_pass((10, 20), 180, W, H)


def test_a4_landscape_only_plus_x_gives_0_and_180():
    p = printer()
    plan = plan_sheet(*A4L, field(*A4L), p)
    assert plan.allowed == [0, 180]
    assert plan.rotations == [0, 180]
    assert any("90°" in n and "+Y" in n for n in plan.notes)
    assert plan.describe().startswith("1) 0°, в упоре угол A")


def test_a3_one_side_error_two_sides_four_passes():
    p = printer(x1=230, y1=225)
    plan = plan_sheet(*A3P, field(*A3P), p)
    assert not plan.ok and plan.allowed == []
    assert "не встаёт на стол" in plan.message and "+Y" in plan.message
    assert "Включи «лист может выступать в сторону +Y»" in plan.message
    p.table.overhang_y = True
    plan = plan_sheet(*A3P, field(*A3P), p)
    assert plan.ok and plan.rotations == [0, 90, 180, 270]
    assert not uncovered(field(*A3P), [plan.rects[r] for r in plan.rotations])


def test_not_enough_reach_says_how_much():
    p = printer(x1=140)
    plan = plan_sheet(*A4L, field(*A4L), p)
    assert not plan.ok and plan.allowed == [0, 180]
    m = re.search(r"по оси X не хватает ([\d.]+) мм", plan.message)
    assert m, plan.message
    assert float(m.group(1)) == pytest.approx(10.5, abs=0.15)
    p.travel.x_max = 151
    assert plan_sheet(*A4L, field(*A4L), p).rotations == [0, 180]


def test_table_size_decides_overhang():
    p = printer(y1=205)
    assert plan_sheet(*A4L, field(*A4L), p).allowed == []
    p.table.table_y = 215
    assert plan_sheet(*A4L, field(*A4L), p).rotations == [0, 180]


def test_uncovered_area_is_exact():
    assert uncovered((0, 0, 100, 100), [(0, 0, 60, 100), (50, 0, 100, 40)]) == [(60, 40, 100, 100)]
    assert uncovered((0, 0, 10, 10), [(-5, -5, 20, 20)]) == []
    p = printer(x1=150, y1=120, table=(400, 300))
    allowed, rects, _ = rotation_rects(*A4L, p)
    assert rects[0] == (0.0, 0.0, 148.0, 118.0)
    un = uncovered((0, 0, *A4L), [rects[0]])
    assert area(un) == pytest.approx(297 * 210 - 148 * 118)
    inside = lambda x, y: any(r[0] <= x <= r[2] and r[1] <= y <= r[3] for r in un)
    assert inside(200, 50) and inside(50, 150) and not inside(100, 100)
    assert rects[180] == (297 - 148, 210 - 118, 297, 210)


def test_lines_covered_by_union_only():
    rects = [(0, 0, 60, 100), (50, 0, 100, 100)]
    assert lines_covered([[(10, 50), (90, 50)]], rects)
    assert not lines_covered([[(10, 50), (110, 50)]], rects)
    assert lines_covered([[(55, 5)]], rects)


def drawing_settings(**travel) -> Settings:
    s = Settings()
    s.printer.travel = Travel(**travel)
    s.printer.table.table_x, s.printer.table.table_y = 250, 220
    return s


def test_single_pass_with_rotation_180():
    s = drawing_settings(x_min=-3, x_max=160, y_min=-3, y_max=120)
    s.drawing.placement.scale_mode = "percent"
    s.drawing.placement.percent = 40
    s.drawing.placement.dx, s.drawing.placement.dy = 100, 60
    c = compose_drawing(s)
    assert c.errors == [] and c.rotation == 180 and c.drawing_passes == [180]
    g = make_drawing_gcode(c)
    assert "rotate sheet 180 deg" in g and "corner C" in g
    W, H = c.layout.width, c.layout.height
    first = next(ln for ln in g.splitlines() if ln.startswith("G0 X") and ln != "G0 X0.00 Y0.00 F3000")
    x, y = (float(v) for v in re.findall(r"[XY](-?[\d.]+)", first))
    u, v = c.strokes[0][0]
    assert (x, y) == pytest.approx((W - u, H - v), abs=0.006)
    for st in c.pass_strokes():
        for px, py in st:
            assert -1 - 1e-6 <= px <= 158 + 1e-6 and -1 - 1e-6 <= py <= 118 + 1e-6


def test_drawing_needing_two_passes_is_split():
    s = drawing_settings(x_min=-3, x_max=200, y_min=-3, y_max=215)
    c = compose_drawing(s)
    assert c.drawing_passes == [0, 180] and c.rotation is None and len(c.parts) == 2
    assert c.errors == []
    s.drawing.placement.scale_mode = "fit_reach"
    c = compose_drawing(s)
    assert c.errors == [] and c.rotation in (0, 180)
    assert make_drawing_gcode(c)


def test_fit_passes_makes_coverage_possible():
    s = drawing_settings(x_min=-3, x_max=160, y_min=-3, y_max=120)
    c = compose_drawing(s)
    assert c.unreachable and c.passes_percent
    s.drawing.placement.scale_mode = "fit_passes"
    c2 = compose_drawing(s)
    assert not c2.unreachable
    assert c2.placement.scale * 100 == pytest.approx(c.passes_percent, rel=0.01)
    assert lines_covered(c2.strokes, [c2.pass_rects[r] for r in c2.allowed])


def test_sheet_plan_in_preview():
    from handwriter.drawing.pipeline import preview_payload
    s = drawing_settings(x_min=-3, x_max=200, y_min=-3, y_max=215)
    p = preview_payload(compose_drawing(s))
    ps = p["passes"]
    assert ps["sheet_plan"]["rotations"] == [0, 180] and ps["map"] == [0, 180]
    assert set(ps["rects"]) == {"0", "180"} and ps["corner_of"]["180"] == "C"
    assert ps["map_uncovered"] == []
    s.printer.travel = Travel(x_min=0, x_max=200, y_min=0, y_max=205)
    s.printer.table.table_y = 215
    ps = preview_payload(compose_drawing(s))["passes"]
    un = [tuple(r) for r in ps["map_uncovered"]]
    union = 196 * 201 * 2 - 99 * 196
    assert area(un) == pytest.approx(297 * 210 - union)


def test_reach_check_gcode():
    s = Settings()
    s.printer.travel = Travel(x_min=-3, x_max=220, y_min=-3, y_max=219)
    assert reach_corners(s) == [(-1, -1), (218, -1), (218, 217), (-1, 217)]
    g = make_reach_check_gcode(s)
    body = [ln for ln in g.splitlines() if not ln.startswith(";")]
    assert body[:7] == ["G21", "G90", "M104 S0", "M140 S0", "M420 S0", "M211 S0", "G92 X0 Y0 Z0"]
    assert [ln for ln in body if ln.startswith("G0 X")][:4] == [
        "G0 X-1.00 Y-1.00 F3000", "G0 X218.00 Y-1.00 F3000", "G0 X218.00 Y217.00 F3000", "G0 X-1.00 Y217.00 F3000"]
    assert body.count("G4 P1000") == 4 and body.count("G4 P2000") == 3
    assert body.count("G1 Z-1.00 F600") == 4
    assert body[-2:] == ["G0 X0.00 Y0.00 F3000", "M400"]
    assert "G28" not in g and "M109" not in g and "M190" not in g
    assert all(ord(ch) < 128 for ch in g)
    for i, ln in enumerate(body):
        if ln == "G4 P1000":
            assert body[i + 1] == "G0 Z4.00 F600"


def test_reach_check_refused_without_window_and_zero_file():
    s = Settings()
    with pytest.raises(GenerationRefused):
        make_reach_check_gcode(s)
    z = make_zero_gcode(s)
    assert "G92 X0 Y0 Z0" in z and "G0 Z4.00" in z and "G28" not in z and "M211 S0" in z


def test_calibration_api():
    from fastapi.testclient import TestClient
    from handwriter.server import app
    client = TestClient(app)
    s = Settings()
    s.printer.travel = Travel(x_min=-3, x_max=220, y_min=-3, y_max=219)
    body = s.model_dump(mode="json")
    info = client.post("/api/calibration/info", json=body).json()
    assert info["errors"] == [] and len(info["corners"]) == 4
    assert any("упор" in w for w in info["warnings"])
    r = client.post("/api/calibration/check_gcode", json=body)
    assert r.status_code == 200 and r.json()["filename"].endswith(".gcode")
    body["printer"]["travel"] = None
    assert client.post("/api/calibration/check_gcode", json=body).status_code == 422
    assert client.post("/api/calibration/zero_gcode", json=body).status_code == 200
