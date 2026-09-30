import json

import pytest

from handwriter.glyphs import StrokeGlyphProvider, load_provider
from handwriter.glyphs.svgparse import parse_transform, apply

SVG = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">{}</svg>'


def write(p, name, body, **root_attrs):
    attrs = " ".join(f'{k.replace("_", "-")}="{v}"' for k, v in root_attrs.items())
    svg = SVG.format(body).replace("<svg ", f"<svg {attrs} ", 1) if attrs else SVG.format(body)
    (p / name).write_text(svg, encoding="utf-8")


def test_builtin_font_has_full_russian_alphabet(font):
    abc = "абвгдеёжзийклмнопрстуфхцчшщъыьэюя"
    for ch in abc + abc.upper() + "0123456789.,:;!?-()\"'":
        assert font.has_char(ch), ch
    assert 0.3 < font.metrics.x_height < 0.6
    assert font.mode == "strokes"


def test_builtin_font_glyph_is_y_up_on_baseline(font):
    g = font.glyph(font.glyph_names_for_char("р")[0])
    x0, y0, x1, y1 = g.bbox()
    assert y0 < 0 < y1
    assert x0 >= 0 and g.advance > x1 - 1e-9 - 0.2


def test_folder_font_variants_and_coordinates(tmp_path):
    d = tmp_path / "hand"
    d.mkdir()
    write(d, "а.svg", '<path d="M10 80 L10 30"/>')
    write(d, "а.2.svg", '<path d="M20 80 L20 30"/>')
    write(d, "а.3.svg", '<polyline points="30,80 30,30"/>')
    write(d, "uni0410.svg", '<path d="M0 80 L40 10 L80 80"/>', data_advance="85")
    write(d, "period.svg", '<circle cx="5" cy="78" r="2"/>')
    (d / "font.json").write_text(json.dumps({"x_height": 50}), encoding="utf-8")
    p = StrokeGlyphProvider.from_path(d)
    assert p.glyph_names_for_char("а") == ["uni0430", "uni0430.2", "uni0430.3"]
    assert p.variants() == {"а": ["uni0430", "uni0430.2", "uni0430.3"]}
    g = p.glyph("uni0430")
    (a, b), = g.strokes
    assert a == pytest.approx((0.1, 0.0)) and b == pytest.approx((0.1, 0.5))
    assert g.advance == pytest.approx(1.0)
    assert p.glyph(p.glyph_names_for_char("А")[0]).advance == pytest.approx(0.85)
    assert p.metrics.x_height == pytest.approx(0.5)
    dot = p.glyph(p.glyph_names_for_char(".")[0]).strokes[0]
    assert dot[0] == pytest.approx(dot[-1])
    assert len(dot) > 8


def test_open_paths_stay_open_and_order_kept(tmp_path):
    d = tmp_path / "f"
    d.mkdir()
    write(d, "т.svg", '<g transform="translate(10 0)"><path d="M0 30 L60 30 M30 30 L30 80"/></g>')
    p = StrokeGlyphProvider.from_path(d)
    st = p.glyph("uni0442").strokes
    assert len(st) == 2
    assert st[0][0] == pytest.approx((0.1, 0.5)) and st[0][-1] == pytest.approx((0.7, 0.5))
    assert st[1][0] == pytest.approx((0.4, 0.5)) and st[1][-1] == pytest.approx((0.4, 0.0))


def test_curves_are_flattened(tmp_path):
    d = tmp_path / "c"
    d.mkdir()
    write(d, "с.svg", '<path d="M70 40 C50 20 10 30 10 60 C10 90 60 90 70 70"/>')
    p = StrokeGlyphProvider.from_path(d)
    st = p.glyph("uni0441").strokes[0]
    assert len(st) > 10
    assert st[0] == pytest.approx((0.7, 0.4)) and st[-1] == pytest.approx((0.7, 0.1))


def test_svg_font_with_variant_glyph_names(tmp_path):
    f = tmp_path / "mini.svg"
    f.write_text("""<svg xmlns="http://www.w3.org/2000/svg"><defs>
      <font id="Mini" horiz-adv-x="500">
        <font-face font-family="Mini" units-per-em="1000" x-height="400"/>
        <glyph unicode="б" glyph-name="be" d="M0 0 L0 700"/>
        <glyph glyph-name="be.2" d="M10 0 L10 700"/>
        <glyph unicode="б" glyph-name="be.3" d="M20 0 L20 700"/>
        <glyph unicode="ff" glyph-name="f_f" d="M0 0 L1 1"/>
        <glyph unicode=" " horiz-adv-x="300"/>
      </font></defs></svg>""", encoding="utf-8")
    p = load_provider(str(f))
    assert p.name == "Mini"
    assert p.glyph_names_for_char("б") == ["be", "be.2", "be.3"]
    (a, b), = p.glyph("be").strokes
    assert a == pytest.approx((0.0, 0.0)) and b == pytest.approx((0.0, 0.7))
    assert p.glyph("be").advance == pytest.approx(0.5)
    assert p.space_advance() == pytest.approx(0.3)
    assert p.metrics.x_height == pytest.approx(0.4)
    assert any("лигатур" in n for n in p.info().notes)


def test_transform_parsing():
    m = parse_transform("translate(10,5) scale(2) rotate(90)")
    assert apply(m, (1, 0)) == pytest.approx((10, 7))


def test_unknown_mode_rejected():
    with pytest.raises(ValueError):
        load_provider("builtin:hershey_cyrillic.svg", mode="outlines")
