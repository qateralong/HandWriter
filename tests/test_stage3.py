import math
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest

from handwriter.glyphs import StrokeGlyphProvider
from handwriter.pipeline import GenerationRefused, compose, make_gcode
from handwriter.rand import rnd, vnoise
from handwriter.settings import Settings, Travel

ROOT = Path(__file__).resolve().parent.parent
FONTS = Path(__file__).parent / "fonts"
BAD = FONTS / "BadScript-Regular.ttf"
PARAGRAPH = ("\tСъешь же ещё этих мягких французских булок, да выпей чаю. Широкая электрификация "
             "южных губерний даст мощный толчок подъёму сельского хозяйства.\n\n\tМама мыла раму.")


def xy(code: str) -> list[tuple[str, str]]:
    return re.findall(r"X(-?\d+\.\d\d) Y(-?\d+\.\d\d)", code)


def outline_settings(text=PARAGRAPH, **conn) -> Settings:
    s = Settings()
    s.font, s.mode, s.text = str(BAD), "outlines", text
    s.printer.travel = Travel(x_min=-2, x_max=200, y_min=-2, y_max=230)
    for k, v in conn.items():
        setattr(s.connections, k, v)
    return s


def test_rnd_is_stable_and_uniform():
    assert rnd(1, "a", 3, 4) == rnd(1, "a", 3, 4)
    assert rnd(1, "a", 3, 4) != rnd(2, "a", 3, 4)
    vals = [rnd(7, "u", i) for i in range(4000)]
    assert 0 <= min(vals) and max(vals) < 1
    assert abs(sum(vals) / len(vals) - 0.5) < 0.02


def test_vnoise_is_smooth_not_independent():
    ts = [i * 0.01 for i in range(1000)]
    v = [vnoise(3, "j", t) for t in ts]
    assert max(abs(a) for a in v) <= 1.0
    assert max(abs(v[i + 1] - v[i]) for i in range(len(v) - 1)) < 0.05
    assert max(v) - min(v) > 0.5


def test_same_seed_same_bytes(settings):
    s = outline_settings()
    a = make_gcode(compose(s))
    from handwriter import glyphs
    glyphs._cache.clear()
    b = make_gcode(compose(outline_settings()))
    assert a == b
    s2 = outline_settings()
    s2.randomness.seed = 2
    assert make_gcode(compose(s2)) != a


def test_same_bytes_across_processes(tmp_path):
    script = (
        "import sys; sys.path.insert(0, r'%s'); sys.path.insert(0, r'%s')\n"
        "from test_stage3 import outline_settings\n"
        "from handwriter.pipeline import compose, make_gcode\n"
        "sys.stdout.buffer.write(make_gcode(compose(outline_settings())).encode())\n" % (ROOT, ROOT / "tests"))
    outs = []
    for hs in ("1", "12345"):
        env = dict(os.environ, PYTHONHASHSEED=hs, HANDWRITER_HOME=str(tmp_path / f"h{hs}"))
        r = subprocess.run([sys.executable, "-c", script], env=env, capture_output=True, timeout=300)
        assert r.returncode == 0, r.stderr.decode(errors="replace")
        outs.append(r.stdout)
    assert outs[0] == outs[1] and len(outs[0]) > 1000


def test_mean_letter_size_equals_setting():
    s = outline_settings("х" * 12 + " " + "х" * 12 + " " + "х" * 12)
    s.randomness.size = 10.0
    s.randomness.offset = s.randomness.jitter = s.randomness.drift = s.randomness.slant = 0.0
    s.connections.enabled = False
    c = compose(s)
    factors = [g.size for g in c.layout.glyphs]
    assert len(set(round(f, 6) for f in factors)) > 10
    assert sum(factors) / len(factors) == pytest.approx(1.0, abs=1e-12)
    heights = {}
    for st in c.layout.strokes:
        for (x, y), t in zip(st.points, st.tags):
            lo, hi = heights.get(t, (y, y))
            heights[t] = (min(lo, y), max(hi, y))
    h = [hi - lo for lo, hi in heights.values()]
    assert max(h) / min(h) > 1.08
    assert sum(h) / len(h) == pytest.approx(s.typography.size_mm, rel=0.03)


def variant_font(tmp_path) -> Path:
    d = tmp_path / "var"
    d.mkdir()
    svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M10 80 L{x} 40"/></svg>'
    for name, x in (("а.svg", 20), ("а.2.svg", 40), ("а.3.svg", 60), ("б.svg", 30)):
        (d / name).write_text(svg.format(x=x), encoding="utf-8")
    (d / "space.svg").write_text('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 50 100"/>', encoding="utf-8")
    return d


def chosen(c, word):
    return [g.glyph for g in c.layout.glyphs if g.word == word and g.char == "а"]


def test_variants_random_without_repeats(tmp_path):
    s = Settings()
    s.font, s.mode = str(variant_font(tmp_path)), "strokes"
    s.printer.travel = Travel(x_min=-2, x_max=200, y_min=-2, y_max=230)
    s.text = " ".join(["аааааа"] * 6)
    c = compose(s)
    assert c.errors == []
    all_names = set()
    for w in range(1, 7):
        v = chosen(c, w)
        all_names |= set(v)
        assert all(a != b for a, b in zip(v, v[1:])), v
    assert all_names == {"uni0430", "uni0430.2", "uni0430.3"}
    assert len({tuple(chosen(c, w)) for w in range(1, 7)}) > 1
    s2 = s.model_copy(deep=True)
    s2.text = "аааааа аааааа ббб"
    assert chosen(compose(s2), 2) == chosen(c, 2)
    s3 = s.model_copy(deep=True)
    s3.randomness.seed = 99
    assert any(chosen(compose(s3), w) != chosen(c, w) for w in range(1, 7))
    s4 = s.model_copy(deep=True)
    s4.randomness.variants = False
    assert set(chosen(compose(s4), 1)) == {"uni0430"}


def test_bad_script_uses_only_free_variants():
    s = outline_settings("ВВВВ кккк ВВВ ккк")
    c = compose(s)
    names = {g.glyph for g in c.layout.glyphs}
    assert "uni0412.ss01" in names or "uni043A.ss02" in names
    assert not any(n and (".init" in n or ".fina" in n) for n in names)


def connect_font(tmp_path) -> Path:
    d = tmp_path / "conn"
    d.mkdir(exist_ok=True)
    head = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">'
    (d / "а.svg").write_text(head + '<path d="M10 80 L50 40 L70 80 L95 80"/></svg>', encoding="utf-8")
    (d / "б.svg").write_text(head + '<path d="M0 80 L20 80 L40 40 L80 80"/><circle cx="40" cy="20" r="2"/></svg>',
                             encoding="utf-8")
    (d / "в.svg").write_text(head + '<path d="M10 80 L10 40"/><path d="M70 40 L10 40"/></svg>', encoding="utf-8")
    (d / "space.svg").write_text('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 50 100"/>', encoding="utf-8")
    (d / "font.json").write_text('{"x_height": 40}', encoding="utf-8")
    return d


def conn_settings(tmp_path, text, connect=True) -> Settings:
    s = Settings()
    s.font, s.mode, s.text = str(connect_font(tmp_path)), "strokes", text
    s.printer.travel = Travel(x_min=-2, x_max=200, y_min=-2, y_max=230)
    s.randomness.enabled = True
    s.connections.enabled = connect
    return s


def test_connection_joins_letters_without_lift(tmp_path):
    c = compose(conn_settings(tmp_path, "аб"))
    first = c.layout.strokes[0]
    letters = [c.layout.glyphs[t].letter for t in first.tags]
    assert letters[0] == 1 and letters[-1] == 2
    assert letters == sorted(letters)
    assert len(c.layout.strokes) == 2
    dot = c.layout.strokes[1]
    assert {c.layout.glyphs[t].letter for t in dot.tags} == {2}
    c2 = compose(conn_settings(tmp_path, "аб", connect=False))
    assert len(c2.layout.strokes) == 3


def test_bridge_is_smooth_curve(tmp_path):
    s = conn_settings(tmp_path, "аб")
    s.randomness.jitter = 0.0
    s.randomness.drift = 0.0
    s.randomness.letter_spacing = 20.0
    s.printer.simplify_tol = 0.0
    c = compose(s)
    pts = c.layout.strokes[0].points
    tags = c.layout.strokes[0].tags
    k = next(i for i in range(1, len(tags)) if tags[i] != tags[i - 1])
    seg = pts[k - 1:k + 6]
    for a, b, cc in zip(seg, seg[1:], seg[2:]):
        v1 = (b[0] - a[0], b[1] - a[1])
        v2 = (cc[0] - b[0], cc[1] - b[1])
        n1, n2 = math.hypot(*v1), math.hypot(*v2)
        if n1 > 1e-6 and n2 > 1e-6:
            cosang = (v1[0] * v2[0] + v1[1] * v2[1]) / n1 / n2
            assert cosang > math.cos(math.radians(25))


def test_touching_strokes_are_glued(tmp_path):
    c = compose(conn_settings(tmp_path, "в"))
    assert len(c.layout.strokes) == 1


def test_order_left_to_right_main_line_then_details():
    c = compose(outline_settings("ёж йод мой", distance=0.35))
    words = [st.word for st in c.layout.strokes]
    assert words == sorted(words)
    for w in (1, 2, 3):
        paths = [st for st in c.layout.strokes if st.word == w]
        sizes = []
        for st in paths:
            xs = [p[0] for p in st.points]
            ys = [p[1] for p in st.points]
            sizes.append(math.hypot(max(xs) - min(xs), max(ys) - min(ys)))
        small = [i for i, v in enumerate(sizes) if v < 0.45 * 3.0]
        big = [i for i, v in enumerate(sizes) if v >= 0.45 * 3.0 * 1.5]
        if small and big:
            assert min(small) > max(big), (w, sizes)


def resume_case(s: Settings, word: int, letter: int):
    full = make_gcode(compose(s))
    s2 = s.model_copy(deep=True)
    s2.text_options.resume_word, s2.text_options.resume_letter = word, letter
    c2 = compose(s2)
    part = make_gcode(c2)
    return full, part, c2


def assert_tail(full: str, part: str):
    f, p = xy(full)[:-1], xy(part)[:-1]
    assert 0 < len(p) < len(f)
    assert f[-len(p):] == p


def test_resume_matches_tail_of_full_gcode():
    s = outline_settings()
    full, part, c2 = resume_case(s, 9, 3)
    assert_tail(full, part)
    assert c2.resume and c2.resume.word == 9
    assert "RESUME from word 9, letter 3" in part


def test_resume_from_connected_letter_starts_at_connection_point():
    s = outline_settings(distance=0.35)
    c = compose(s)
    target = None
    for st in c.layout.strokes:
        for k in range(1, len(st.tags)):
            if st.tags[k] != st.tags[k - 1]:
                g = c.layout.glyphs[st.tags[k]]
                target = (g.word, g.letter, st.points[k - 1])
                break
        if target:
            break
    assert target, "в тексте должна быть хотя бы одна связка"
    full, part, c2 = resume_case(s, target[0], target[1])
    assert c2.resume.connected
    assert_tail(full, part)
    first = xy(part)[0]
    t = s.printer
    assert first == (f"{target[2][0]:.2f}", f"{target[2][1]:.2f}")


def test_resume_with_offset_and_rotation_still_tail():
    s = outline_settings()
    s.typography.dx, s.typography.dy, s.typography.rotation_deg = 1.5, -0.7, 0.4
    full, part, _ = resume_case(s, 12, 1)
    assert_tail(full, part)


def test_resume_errors():
    s = outline_settings("Один два три")
    s.text_options.resume_word = 9
    assert any("нет на этом листе" in e for e in compose(s).errors)
    s.text_options.resume_word, s.text_options.resume_letter = 2, 7
    assert any("нет буквы 7" in e for e in compose(s).errors)


def test_resume_preview_marks_done_part():
    from handwriter.pipeline import preview_payload
    s = outline_settings()
    s.text_options.resume_word, s.text_options.resume_letter = 9, 3
    p = preview_payload(compose(s))
    done = [st for st in p["strokes"] if st["d"]]
    todo = [st for st in p["strokes"] if not st["d"]]
    assert done and todo
    assert p["resume"] == {"word": 9, "letter": 3, "connected": p["resume"]["connected"]}
    assert all(g["b"] for g in p["glyphs"] if not g["missing"] and g["c"] not in " ")


def test_out_of_travel_refuses_with_randomness():
    s = outline_settings()
    s.randomness.right_edge = 8.0
    s.printer.travel = Travel(x_min=-2, x_max=150, y_min=-2, y_max=230)
    c = compose(s)
    assert any("вне хода" in e for e in c.errors)
    with pytest.raises(GenerationRefused):
        make_gcode(c)


def test_randomness_changes_geometry_but_keeps_text():
    s = outline_settings()
    s.randomness.enabled = False
    a = compose(s)
    b = compose(outline_settings())
    assert [g.char for g in a.layout.glyphs if not g.hyphen][:30] == [g.char for g in b.layout.glyphs if not g.hyphen][:30]
    assert xy(make_gcode(a))[:50] != xy(make_gcode(b))[:50]
