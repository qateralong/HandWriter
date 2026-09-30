import re

import pytest

from handwriter.checks import check_bounds, travel_box
from handwriter.pipeline import GenerationRefused, compose, make_gcode, make_test_gcode
from handwriter.settings import MissingChoice, Travel

SAMPLE = ("\tКороткий абзац с красной строкой, чтобы проверить электрификацию и переносы.\n"
          "\n"
          "\tПосле пропущенной строки ещё немного текста.")
ALLOWED = re.compile(r"^(G21|G90|M104 S0|M140 S0|M420 S0|M211 S0|G92 X0 Y0 Z0|M400|"
                     r"G0 Z-?\d+\.\d\d F\d+|G1 Z-?\d+\.\d\d F\d+|"
                     r"G0 X-?\d+\.\d\d Y-?\d+\.\d\d F\d+|G1 X-?\d+\.\d\d Y-?\d+\.\d\d( F\d+)?)$")


def xy(code):
    return [(float(a), float(b)) for a, b in re.findall(r"X(-?\d+\.\d+) Y(-?\d+\.\d+)", code)]


def narrow(s):
    s.sheet.width, s.sheet.margin_left, s.sheet.margin_right = 95, 15, 8
    s.text = SAMPLE
    return s


def test_sample_paragraph_gcode_inside_travel(settings):
    s = narrow(settings)
    c = compose(s)
    assert c.errors == []
    assert any(g.hyphen for g in c.layout.glyphs), "в образце должен быть перенос"
    para = {w.index: w.paragraph for w in c.text.words}
    p1_last = max(g.line for g in c.layout.glyphs if para[g.word] == 1)
    p2_first = min(g.line for g in c.layout.glyphs if para[g.word] == 2)
    assert p2_first == p1_last + 2
    first = min(c.layout.glyphs, key=lambda g: (g.line, g.x))
    assert first.x == pytest.approx(s.sheet.margin_left + s.sheet.indent)
    code = make_gcode(c)
    t = s.printer.travel
    m = s.printer.safety_margin
    pts = xy(code)
    assert pts
    for x, y in pts:
        if (x, y) == (0.0, 0.0):
            continue
        assert t.x_min + m <= x <= t.x_max - m and t.y_min + m <= y <= t.y_max - m


def test_gcode_uses_only_allowed_commands(settings):
    code = make_gcode(compose(narrow(settings)))
    body = [ln for ln in code.splitlines() if ln and not ln.startswith(";")]
    for ln in body:
        assert ALLOWED.match(ln), ln
    assert "G28" not in code and "M109" not in code and "M190" not in code
    assert body[:9] == ["G21", "G90", "M104 S0", "M140 S0", "M420 S0", "M211 S0", "G92 X0 Y0 Z0",
                        "G0 Z4.00 F600", body[8]]
    assert body[-3:] == ["G0 Z4.00 F600", "G0 X0.00 Y0.00 F3000", "M400"]
    assert code.isascii()


def test_each_stroke_pen_down_then_up(settings):
    code = make_gcode(compose(narrow(settings)))
    body = [ln for ln in code.splitlines() if ln and not ln.startswith(";")]
    down = False
    for ln in body[8:]:
        if ln.startswith("G1 Z"):
            assert not down; down = True
        elif ln.startswith("G0 Z"):
            down = False
        elif ln.startswith("G0 X"):
            assert not down, "переезд с опущенным карандашом"
        elif ln.startswith("G1 X"):
            assert down, "рисование с поднятым карандашом"


def test_same_settings_same_bytes(settings):
    a = make_gcode(compose(narrow(settings)))
    b = make_gcode(compose(narrow(settings)))
    assert a == b


def test_point_outside_travel_refuses(settings):
    s = narrow(settings)
    s.printer.travel = Travel(x_min=-2, x_max=60, y_min=-2, y_max=230)
    c = compose(s)
    assert any("вне хода" in e for e in c.errors)
    with pytest.raises(GenerationRefused):
        make_gcode(c)


def test_safety_margin_is_enforced(settings):
    s = settings
    s.printer.travel = Travel(x_min=0, x_max=100, y_min=0, y_max=100)
    assert check_bounds([[(0.0, 0.0), (1.0, 50.0)]], s) == []
    assert check_bounds([[(-0.5, 50.0)]], s)
    assert check_bounds([[(98.5, 50.0)]], s)
    assert check_bounds([[(98.0, 50.0)]], s) == []


def test_flip_x_checks_machine_coordinates(settings):
    s = settings
    s.printer.flip_x = True
    s.printer.travel = Travel(x_min=-200, x_max=2, y_min=-2, y_max=230)
    assert check_bounds([[(50.0, 50.0)]], s) == []
    s.printer.flip_x = False
    assert check_bounds([[(50.0, 50.0)]], s)


def test_flip_changes_sign_in_gcode(settings):
    s = narrow(settings)
    s.printer.flip_x = True
    s.printer.travel = Travel(x_min=-200, x_max=2, y_min=-2, y_max=230)
    pts = [p for p in xy(make_gcode(compose(s))) if p != (0.0, 0.0)]
    assert all(x < 0 for x, _ in pts) and all(y > 0 for _, y in pts)


def test_unmeasured_travel_uses_sheet_and_warns(settings):
    s = narrow(settings)
    s.printer.travel = None
    box = travel_box(s)
    assert (box.x_min, box.x_max, box.y_min, box.y_max, box.measured) == (0, s.sheet.width, 0, s.sheet.height, False)
    c = compose(s)
    assert any("не измерен" in w for w in c.warnings)
    assert "NOT MEASURED" in make_gcode(c)


def test_z_warnings(settings):
    s = narrow(settings)
    s.printer.pen_down_z, s.printer.pen_up_z = -3.5, 1.5
    c = compose(s)
    assert any("pen_down_z" in w for w in c.warnings)
    assert any("pen_up_z" in w for w in c.warnings)
    s.printer.pen_up_z = -4
    assert any("pen_up_z должен быть выше" in e for e in compose(s).errors)


def test_missing_glyph_blocks_until_resolved(settings):
    s = narrow(settings)
    s.text = "Номер №5"
    c = compose(s)
    assert any("Нет в шрифте" in e for e in c.errors)
    with pytest.raises(GenerationRefused):
        make_gcode(c)
    s.text_options.missing["№"] = MissingChoice(action="skip")
    assert compose(s).errors == []


def test_inconsistent_sheet_refuses(settings):
    s = narrow(settings)
    s.sheet.margin_left, s.sheet.margin_right = 60, 60
    c = compose(s)
    assert any("поля" in e for e in c.errors)
    s = narrow(settings)
    s.sheet.first_line_top = s.sheet.height
    assert compose(s).errors


def test_test_file(settings):
    code, strokes = make_test_gcode(settings)
    assert "G92 X0 Y0 Z0" in code and code.rstrip().endswith("M400")
    pts = [p for p in xy(code) if p != (0.0, 0.0)]
    sh = settings.sheet
    xs, ys = [p[0] for p in pts], [p[1] for p in pts]
    assert min(x for x in xs if x > 10) == pytest.approx(sh.margin_left)
    assert max(xs) == pytest.approx(sh.width - sh.margin_right)
    assert max(ys) == pytest.approx(sh.height - sh.first_line_top)
    o = settings.printer.test_mark_offset
    assert [(o, o), (o + 40, o)] in strokes and [(o, o), (o, o + 20)] in strokes


def test_test_file_refused_outside_travel(settings):
    settings.printer.travel = Travel(x_min=-2, x_max=50, y_min=-2, y_max=50)
    with pytest.raises(GenerationRefused):
        make_test_gcode(settings)
