import base64
import io
import math

import ezdxf
import pytest

from handwriter.drawing import ops
from handwriter.drawing.pipeline import compose_drawing, make_drawing_gcode, preview_payload
from handwriter.drawing.place import TITLE_BLOCK_LINES, scale_label
from handwriter.drawing.sources import clear_cache
from handwriter.paths import user_drawings_dir
from handwriter.pipeline import GenerationRefused
from handwriter.settings import Settings, Travel

SVG_SIMPLE = """<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="60mm" viewBox="0 0 100 60">
  <g fill="none" stroke="black" stroke-width="0.25">
    <rect x="0" y="0" width="100" height="60"/>
    <line x1="10" y1="50" x2="90" y2="10"/>
    <circle cx="30" cy="30" r="12"/>
    <path d="M60 40 A15 15 0 0 1 90 40"/>
    <polyline points="10,10 20,20 30,10"/>
  </g>
</svg>"""

SVG_SIMPLE_CM = """<svg xmlns="http://www.w3.org/2000/svg" width="10cm" height="6cm" viewBox="0 0 1000 600">
  <g transform="scale(10)" fill="none" stroke="black" style="stroke-width:0.25">
    <rect x="0" y="0" width="100" height="60"/>
    <g transform="translate(10 50)"><line x1="0" y1="0" x2="80" y2="-40"/></g>
    <ellipse cx="30" cy="30" rx="12" ry="12"/>
    <path d="M60 40 a15 15 0 0 1 30 0"/>
    <polyline points="10,10 20,20 30,10"/>
  </g>
</svg>"""


def dxf_simple(units: int = 4, k: float = 1.0) -> bytes:
    doc = ezdxf.new("R2010", units=units)
    msp = doc.modelspace()

    def P(x, y):
        return (x * k, (60 - y) * k)
    for a, b in [((0, 0), (100, 0)), ((100, 0), (100, 60)), ((100, 60), (0, 60)), ((0, 60), (0, 0))]:
        msp.add_line(P(*a), P(*b))
    msp.add_line(P(10, 50), P(90, 10))
    msp.add_circle(P(30, 30), 12 * k)
    msp.add_arc(P(75, 40), 15 * k, 0, 180)
    msp.add_lwpolyline([P(10, 10), P(20, 20), P(30, 10)])
    buf = io.StringIO()
    doc.write(buf)
    return buf.getvalue().encode("utf-8")


def put_file(name: str, data: bytes | str) -> str:
    p = user_drawings_dir() / name
    if isinstance(data, str):
        data = data.encode("utf-8")
    p.write_bytes(data)
    return name


def base_settings(file: str = "builtin:test") -> Settings:
    s = Settings()
    s.printer.travel = Travel(x_min=-2, x_max=300, y_min=-2, y_max=300)
    s.drawing.file = file
    return s


@pytest.fixture(autouse=True)
def _fresh_cache():
    clear_cache()
    yield
    clear_cache()


def _dist_to_polyline(p, pl):
    best = math.inf
    if len(pl) == 1:
        return math.dist(p, pl[0])
    for a, b in zip(pl, pl[1:]):
        dx, dy = b[0] - a[0], b[1] - a[1]
        L2 = dx * dx + dy * dy
        t = 0.0 if L2 == 0 else max(0.0, min(1.0, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / L2))
        best = min(best, math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy))
    return best


def _hausdorff(A, B):
    def one(X, Y):
        return max(min(_dist_to_polyline(p, q) for q in Y) for st in X for p in st)
    return max(one(A, B), one(B, A))


def _one_to_one(file):
    s = base_settings(file)
    s.drawing.placement.scale_mode = "one_to_one"
    return compose_drawing(s)


def test_svg_and_dxf_import_to_same_coordinates():
    svg = _one_to_one(put_file("simple.svg", SVG_SIMPLE))
    dxf = _one_to_one(put_file("simple.dxf", dxf_simple()))
    for c in (svg, dxf):
        assert c.errors == [], c.errors
        assert c.placement.scale == 1.0
    bs, bd = svg.imp.bbox(), dxf.imp.bbox()
    assert abs((bs[2] - bs[0]) - 100) < 1e-6 and abs((bs[3] - bs[1]) - 60) < 1e-6
    assert all(abs(a - b) < 1e-6 for a, b in zip((bs[2] - bs[0], bs[3] - bs[1]), (bd[2] - bd[0], bd[3] - bd[1])))
    assert _hausdorff(svg.strokes, dxf.strokes) < 0.08
    assert len(dxf.strokes) == len(svg.strokes) == 5


def test_svg_units_and_transforms():
    a = _one_to_one(put_file("mm.svg", SVG_SIMPLE))
    b = _one_to_one(put_file("cm.svg", SVG_SIMPLE_CM))
    assert b.errors == [] and "viewBox" in b.imp.units_note
    assert _hausdorff(a.strokes, b.strokes) < 0.08
    assert abs(b.imp.paths[0].width - 0.25) < 1e-9


def test_dxf_units_insunits_and_manual():
    cm = _one_to_one(put_file("cm.dxf", dxf_simple(units=5, k=0.1)))
    mm = _one_to_one(put_file("mm.dxf", dxf_simple()))
    assert _hausdorff(cm.strokes, mm.strokes) < 0.08
    none = put_file("nounits.dxf", dxf_simple(units=0, k=0.1))
    c = _one_to_one(none)
    assert any("INSUNITS" in w for w in c.warnings)
    assert abs((c.imp.bbox()[2] - c.imp.bbox()[0]) - 10) < 1e-6
    s = base_settings(none)
    s.drawing.placement.scale_mode = "one_to_one"
    s.drawing.imp.units = "cm"
    c = compose_drawing(s)
    assert abs((c.imp.bbox()[2] - c.imp.bbox()[0]) - 100) < 1e-6


def test_dash_polyline_pattern():
    pieces = ops.dash_polyline([(0, 0), (20, 0)], [5, 5])
    assert [(p[0][0], p[-1][0]) for p in pieces] == [(0, 5), (10, 15)]
    pieces = ops.dash_polyline([(0, 0), (10, 0)], [4, 2, 0, 2])
    assert [len(p) for p in pieces] == [2, 1, 2]
    assert pieces[1] == [(6.0, 0.0)] and pieces[2][0] == (8.0, 0.0)
    shifted = ops.dash_polyline([(0, 0), (20, 0)], [5, 5], offset=2)
    assert shifted[0] == [(0, 0), (3, 0)] and shifted[1][0] == (8.0, 0.0)
    corner = ops.dash_polyline([(0, 0), (3, 0), (3, 3)], [5, 10])
    assert corner == [[(0, 0), (3, 0), (3, 2)]]


def test_dxf_linetypes_become_real_dashes():
    doc = ezdxf.new("R2010", setup=True, units=4)
    doc.header["$LTSCALE"] = 10
    msp = doc.modelspace()
    msp.add_line((0, 0), (100, 0), dxfattribs={"linetype": "DASHED"})
    msp.add_line((0, 20), (100, 20), dxfattribs={"linetype": "DASHDOT"})
    msp.add_line((0, 40), (100, 40))
    buf = io.StringIO()
    doc.write(buf)
    s = base_settings(put_file("lt.dxf", buf.getvalue()))
    s.drawing.placement.scale_mode = "one_to_one"
    s.drawing.paths.join_tol = 0
    c = compose_drawing(s)
    assert c.errors == []
    ys = {}
    for st in c.strokes:
        ys.setdefault(round(st[0][1] - c.placement.ty), []).append(st)
    assert len(ys[40]) == 1
    dashed = ys[0]
    assert len(dashed) > 5
    assert all(abs(ops.path_length(p) - 12.7) < 1e-6 for p in dashed[:-1])
    dashdot = ys[20]
    assert any(len(p) == 1 for p in dashdot)


def test_svg_dasharray_scaled_with_drawing():
    c = compose_drawing(base_settings())
    s = c.placement.scale
    axis = [st for st in c.strokes if len(st) <= 2 and ops.path_length(st) < 13 * s]
    lengths = sorted({round(ops.path_length(p) / s, 3) for p in axis})
    assert 12.0 in lengths and 1.0 in lengths


def test_sheet_orientation_and_scale_modes():
    s = base_settings()
    c = compose_drawing(s)
    assert c.layout.orientation == "landscape" and (c.layout.width, c.layout.height) == (297, 210)
    assert abs(c.placement.scale - min(277 / 150, 190 / 100)) < 1e-9
    s.drawing.sheet.orientation = "portrait"
    assert compose_drawing(s).layout.width == 210
    s.drawing.placement.scale_mode = "percent"
    s.drawing.placement.percent = 50
    c = compose_drawing(s)
    assert c.placement.scale == 0.5
    bb = [min(p[0] for st in c.strokes for p in st), max(p[0] for st in c.strokes for p in st)]
    assert abs((bb[0] + bb[1]) / 2 - 105) < 1e-6
    s.drawing.placement.scale_mode = "one_to_one"
    s.drawing.sheet.format = "custom"
    s.drawing.sheet.width, s.drawing.sheet.height = 100, 100
    c = compose_drawing(s)
    assert any("1:1" in w for w in c.warnings)
    assert any("за край листа" in e for e in c.errors)
    assert scale_label(0.5) == "1:2" and scale_label(2) == "2:1" and scale_label(1 / 2.5) == "1:2.5"


def test_gost_frame_and_title_block():
    s = base_settings()
    s.drawing.frame.enabled = True
    s.drawing.sheet.format = "A3"
    s.printer.travel = Travel(x_min=-3, x_max=430, y_min=-3, y_max=310)
    c = compose_drawing(s)
    assert c.errors == []
    lay = c.layout
    assert lay.inner == (20, 5, 420 - 5, 297 - 5)
    assert lay.title_block == (415 - 185, 5, 415, 60)
    assert len(lay.frame) == 1 + len(TITLE_BLOCK_LINES)
    xs = sorted({a for a, b, c2, d, t in TITLE_BLOCK_LINES if a == c2})
    assert xs == [0, 7, 17, 40, 55, 65, 135, 140, 145, 150, 155, 167]
    rows = sorted({b for a, b, c2, d, t in TITLE_BLOCK_LINES if b == d and a == 0})
    assert rows == [5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55]
    tb = lay.title_block
    area = c.placement.area
    assert area[1] >= tb[3] - 1e-9 or area[2] <= tb[0] + 1e-9
    s.drawing.frame.tb_width, s.drawing.frame.tb_height = 150, 40
    c2 = compose_drawing(s)
    assert c2.layout.title_block == (415 - 150, 5, 415, 45)


def test_thick_lines_are_parallel_passes():
    s = base_settings()
    s.drawing.weights.enabled = True
    c = compose_drawing(s)
    thin = compose_drawing(base_settings())
    assert sum(c.thick) == 3 * 2
    assert len(c.strokes) == len(thin.strokes) + 4
    frame = sorted((st for st, t in zip(c.strokes, c.thick) if t and len(st) == 5),
                   key=lambda st: min(p[0] for p in st))
    lefts = [min(p[0] for p in st) for st in frame]
    assert [round(b - a, 6) for a, b in zip(lefts, lefts[1:])] == [0.15, 0.15]
    s2 = base_settings(put_file("w.dxf", dxf_simple()))
    s2.drawing.weights.enabled = True
    s2.drawing.weights.layers = {"0": "thick"}
    assert all(compose_drawing(s2).thick)


def test_offset_polyline_keeps_distance():
    sq = [(0, 0), (10, 0), (10, 10), (0, 10), (0, 0)]
    inner = ops.offset_polyline(sq, 0.15, True)
    assert inner[0] == inner[-1]
    assert all(abs(abs(v) - 0.15) < 1e-9 or abs(abs(v) - 9.85) < 1e-9 for p in inner for v in p)


def test_join_and_order():
    segs = [[(10, 0), (20, 0)], [(0, 0), (10, 0)], [(20, 0), (20, 5)], [(50, 50), (51, 50)]]
    joined = ops.join_paths(segs, 0.05)
    assert sorted(len(p) for p in joined) == [2, 4]
    long_first = ops.order_paths([[(100, 0), (101, 0)], [(0, 0), (0, 50)], [(5, 0), (5, 1)]], long_path=30)
    assert [i for i, _ in long_first] == [1, 2, 0]
    assert long_first[1][1][0] == (5, 1)


def test_out_of_reach_gives_no_gcode_and_fit_reach_fixes_it():
    s = base_settings()
    s.printer.travel = Travel(x_min=0, x_max=160, y_min=0, y_max=120)
    s.printer.table.table_x, s.printer.table.table_y = 250, 220
    c = compose_drawing(s)
    assert c.allowed == [0, 180]
    assert c.unreachable and any("не достаётся" in e for e in c.errors)
    assert c.passes_percent and any("Подобрать масштаб под проходы" in e for e in c.errors)
    with pytest.raises(GenerationRefused):
        make_drawing_gcode(c)
    s.drawing.placement.scale_mode = "fit_reach"
    s.drawing.weights.enabled = True
    c = compose_drawing(s)
    assert c.errors == [] and not c.unreachable
    xs = [p[0] for st in c.strokes for p in st]
    ys = [p[1] for st in c.strokes for p in st]
    assert min(xs) >= 2 and max(xs) <= 158 and min(ys) >= 2 and max(ys) <= 118
    assert c.rotation == 0 and make_drawing_gcode(c)


def test_unmeasured_travel_warns_and_uses_work_area():
    s = base_settings()
    s.printer.travel = None
    c = compose_drawing(s)
    assert any("не измерен" in w for w in c.warnings)
    assert c.reach[:3] == (0, 0, s.printer.work_w - s.printer.safety_margin)


def test_gcode_template_and_determinism():
    s = base_settings()
    s.drawing.frame.enabled = True
    s.drawing.weights.enabled = True
    s.drawing.placement.scale_mode = "fit_reach"
    g1 = make_drawing_gcode(compose_drawing(s))
    clear_cache()
    g2 = make_drawing_gcode(compose_drawing(Settings.model_validate(s.model_dump(mode="json"))))
    assert g1 == g2
    lines = g1.splitlines()
    body = [ln for ln in lines if not ln.startswith(";")]
    assert body[:9] == ["G21", "G90", "M104 S0", "M140 S0", "M420 S0", "M211 S0", "G92 X0 Y0 Z0",
                        "G0 Z4.00 F600", body[8]]
    assert body[-3:] == ["G0 Z4.00 F600", "G0 X0.00 Y0.00 F3000", "M400"]
    assert all(ln.split()[0] in ("G0", "G1", "G21", "G90", "M104", "M140", "M420", "M211", "G92", "M400")
               for ln in body)
    assert "G28" not in g1 and "M109" not in g1 and "M190" not in g1
    assert all(ord(ch) < 128 for ch in g1)


def test_rdp_simplifies_on_sheet():
    s = base_settings()
    c = compose_drawing(s)
    circle = max(c.strokes, key=len)
    s.printer.simplify_tol = 0.5
    rough = max(compose_drawing(s).strokes, key=len)
    assert len(rough) < len(circle)


def test_text_warning_lists_places():
    svg = SVG_SIMPLE.replace("</svg>", '<text x="50" y="30" font-size="5">Масштаб 1:2</text></svg>')
    c = _one_to_one(put_file("t.svg", svg))
    w = [x for x in c.warnings if "Текст не в кривых" in x]
    assert w and "Масштаб 1:2" in w[0]
    assert c.texts[0]["text"] == "Масштаб 1:2"
    doc = ezdxf.new(units=4)
    doc.modelspace().add_line((0, 0), (50, 0))
    doc.modelspace().add_text("Деталь", dxfattribs={"insert": (10, 10), "height": 3.5})
    buf = io.StringIO()
    doc.write(buf)
    c = _one_to_one(put_file("t.dxf", buf.getvalue()))
    assert any("Деталь" in x for x in c.warnings)


def test_fills_contour_or_centerline():
    svg = """<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="20mm" viewBox="0 0 40 20">
      <rect x="5" y="5" width="20" height="0.8" fill="black"/>
      <rect x="0" y="0" width="40" height="20" fill="white"/>
    </svg>"""
    name = put_file("fill.svg", svg)
    c = _one_to_one(name)
    assert len(c.imp.paths) == 1 and c.imp.paths[0].closed
    assert any("Белые" in w for w in c.warnings)
    s = base_settings(name)
    s.drawing.placement.scale_mode = "one_to_one"
    s.drawing.imp.fill_centerlines = True
    c = compose_drawing(s)
    assert len(c.imp.paths) == 1 and not c.imp.paths[0].closed
    L = ops.path_length(c.imp.paths[0].points)
    assert 18.5 < L < 20.5
    ys = [p[1] for p in c.imp.paths[0].points]
    assert max(ys) - min(ys) < 0.2


def test_pdf_vector_import_and_page_choice():
    pymupdf = pytest.importorskip("pymupdf")
    doc = pymupdf.open()
    for n in range(2):
        page = doc.new_page(width=595.28, height=841.89)
        sh = page.new_shape()
        sh.draw_rect(pymupdf.Rect(72, 72, 72 + 72 * (n + 1), 144))
        sh.finish(width=0.5, color=(0, 0, 0))
        sh.draw_line((72, 200), (216, 200))
        sh.finish(width=1, color=(0, 0, 0), dashes="[6 6] 0", closePath=False)
        sh.commit()
        page.insert_text((72, 300), "Текст на странице", fontname="helv", fontsize=10)
    name = put_file("two.pdf", doc.tobytes())
    s = base_settings(name)
    s.drawing.placement.scale_mode = "one_to_one"
    c = compose_drawing(s)
    assert c.errors == [] and c.imp.pages == 2
    rect = [p for p in c.imp.paths if p.closed][0]
    xs = [q[0] for q in rect.points]
    assert abs(max(xs) - min(xs) - 25.4) < 1e-6
    dashed = [p for p in c.imp.paths if p.dash]
    assert dashed and abs(dashed[0].dash[0] - 6 * 25.4 / 72) < 1e-9
    assert c.texts
    s.drawing.imp.pdf_page = 2
    c2 = compose_drawing(s)
    rect2 = [p for p in c2.imp.paths if p.closed][0]
    assert abs(max(q[0] for q in rect2.points) - min(q[0] for q in rect2.points) - 50.8) < 1e-6


def test_raster_import_uses_skeleton():
    from PIL import Image, ImageDraw
    img = Image.new("L", (400, 200), 255)
    d = ImageDraw.Draw(img)
    d.line((50, 100, 350, 100), fill=0, width=9)
    d.ellipse((150, 20, 250, 180), outline=0, width=7)
    buf = io.BytesIO()
    img.save(buf, format="PNG", dpi=(254, 254))
    c = _one_to_one(put_file("r.png", buf.getvalue()))
    assert c.errors == []
    assert any("качество ниже" in w for w in c.warnings)
    bb = c.imp.bbox()
    assert abs((bb[2] - bb[0]) - 30) < 1.2
    assert len(c.imp.paths) <= 8


def test_drawing_api():
    from fastapi.testclient import TestClient
    from handwriter.server import app
    client = TestClient(app)
    assert "Чертёж" in client.get("/drawing").text
    assert 'href="/drawing"' in client.get("/").text
    r = client.post("/api/drawing/upload", json={"filename": "simple.dxf",
                                                 "content_b64": base64.b64encode(dxf_simple()).decode()})
    assert r.status_code == 200 and r.json()["kind"] == "dxf"
    assert any(f["spec"] == "simple.dxf" for f in client.get("/api/drawing/files").json())
    assert client.post("/api/drawing/upload", json={"filename": "x.doc", "content_b64": ""}).status_code == 400
    s = base_settings("simple.dxf").model_dump(mode="json")
    p = client.post("/api/drawing/preview", json=s).json()
    assert p["errors"] == [] and p["strokes"] and p["scale"]["label"] and p["import"]["kind"] == "dxf"
    g = client.post("/api/drawing/gcode", json=s)
    assert g.status_code == 200 and g.json()["filename"].endswith(".gcode")
    s["printer"]["travel"] = {"x_min": 0, "x_max": 50, "y_min": 0, "y_max": 50}
    s["drawing"]["placement"]["scale_mode"] = "one_to_one"
    bad = client.post("/api/drawing/gcode", json=s)
    assert bad.status_code == 422 and bad.json()["errors"]
    assert preview_payload(compose_drawing(Settings.model_validate(s)))["unreachable"]


def test_striped_image_hatch_fill_at_zero():
    from PIL import Image, ImageDraw
    img = Image.new("L", (400, 300), 255)
    d = ImageDraw.Draw(img)
    for i, h in enumerate((6, 12, 20)):
        d.rectangle((20, 40 + i * 80, 380, 40 + i * 80 + h), fill=0)
    buf = io.BytesIO()
    img.save(buf, format="PNG", dpi=(254, 254))
    s = base_settings(put_file("stripes.png", buf.getvalue()))
    s.drawing.imp.raster_mode = "fill"
    s.drawing.placement.scale_mode = "one_to_one"
    s.drawing.placement.anchor = "zero"
    c = compose_drawing(s)
    assert c.errors == []
    pts = [p for st in c.strokes for p in st]
    assert min(p[0] for p in pts) == pytest.approx(0, abs=0.05) and min(p[1] for p in pts) == pytest.approx(0, abs=0.3)
    assert all(abs(st[0][1] - st[-1][1]) < 1e-9 for st in c.strokes)
    assert len(c.strokes) == pytest.approx((0.7 + 1.3 + 2.1) / 0.4, abs=3)
    assert make_drawing_gcode(c)


def test_work_areas_bounds_count_and_zip_name(monkeypatch):
    from urllib.parse import unquote

    from fastapi.testclient import TestClient

    from handwriter.drawing import pipeline
    from handwriter.server import app
    from handwriter.settings import Settings
    s = Settings()
    s.drawing.sheet.format, s.drawing.sheet.width, s.drawing.sheet.height = "A3", 297, 420
    s.drawing.split.offsets = {"0": (40.0, 0.0)}
    assert any("вне хода карандаша" in e for e in compose_drawing(s).errors)
    s.drawing.split.offsets = {}
    s.drawing.split.areas = 1
    assert any("Выбрано рабочих областей: 1" in e for e in compose_drawing(s).errors)
    s.drawing.split.areas = 0
    monkeypatch.setattr(pipeline, "file_stem", lambda c: "чертёж")
    r = TestClient(app).post("/api/drawing/zip", json=s.model_dump(mode="json"))
    assert r.status_code == 200 and "чертёж.zip" in unquote(r.headers["content-disposition"])


def test_zero_anchor_with_weights_and_forced_areas_test_files():
    from handwriter.drawing.pipeline import make_all_files
    from handwriter.settings import Settings, Travel
    s = Settings()
    s.drawing.placement.anchor = "zero"
    s.drawing.weights.enabled = True
    assert compose_drawing(s).errors == []
    s = Settings()
    s.drawing.sheet.orientation = "portrait"
    s.drawing.split.areas = 3
    s.printer.travel = Travel(x_min=-3, x_max=200, y_min=-3, y_max=215)
    c = compose_drawing(s)
    assert c.errors == [] and len(make_all_files(c, True)) == 2 * len(c.parts)
