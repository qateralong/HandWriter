import pytest

from handwriter.layout import layout, line_baselines
from handwriter.settings import TextOptions
from handwriter.text import process_text


def run(s, font, text):
    s.text = text
    pt = process_text(text, font.has_char, TextOptions().replacements, s.text_options.missing)
    return layout(s, font, pt), pt


def line_text(res):
    lines = {}
    for g in res.glyphs:
        lines.setdefault(g.line, []).append(g)
    return lines


def test_baselines_from_top_with_pitch(settings):
    s = settings
    s.sheet.height, s.sheet.first_line_top, s.sheet.line_pitch, s.sheet.bottom_limit = 100, 10, 8, 20
    assert line_baselines(s) == [90, 82, 74, 66, 58, 50, 42, 34, 26]


def test_scale_is_size_over_x_height(settings, font):
    settings.typography.size_mm = 4.0
    res, _ = run(settings, font, "х")
    assert res.scale == pytest.approx(4.0 / font.metrics.x_height)
    ys = [p[1] for st in res.strokes for p in st.points]
    base = res.baselines[0]
    assert max(ys) - base == pytest.approx(4.0, abs=0.06)


def test_every_stroke_knows_word_letter_line(settings, font):
    res, pt = run(settings, font, "\tМама мыла раму.\n\nВторая строка")
    assert res.strokes
    for st in res.strokes:
        word = pt.words[st.word - 1]
        assert 1 <= st.letter <= len(word.text)
        assert st.line >= 1
    assert any(st.word == 3 and st.letter == 5 for st in res.strokes)
    assert {st.line for st in res.strokes if st.word == 4} == {3}


def test_indent_only_with_tab(settings, font):
    res, _ = run(settings, font, "\tС отступом\n   без отступа")
    first = {g.line: g.x for g in reversed(res.glyphs)}
    assert first[1] == pytest.approx(settings.sheet.margin_left + settings.sheet.indent)
    assert first[2] == pytest.approx(settings.sheet.margin_left)


def test_text_stays_inside_margins(settings, font):
    s = settings
    s.sheet.width, s.sheet.margin_left, s.sheet.margin_right = 90, 15, 10
    res, _ = run(s, font, "Широкая электрификация южных губерний даст мощный толчок подъёму сельского хозяйства. " * 3)
    right = s.sheet.width - s.sheet.margin_right
    for g in res.glyphs:
        assert g.x >= s.sheet.margin_left - 1e-6
        assert g.x + g.advance <= right + 1e-6


def test_hyphenation_puts_hyphen_at_end_of_upper_line(settings, font):
    s = settings
    s.sheet.width, s.sheet.margin_left, s.sheet.margin_right = 60, 10, 5
    res, pt = run(s, font, "Электрификация")
    hy = [g for g in res.glyphs if g.hyphen]
    assert hy, "ожидался перенос"
    h = hy[0]
    line_glyphs = [g for g in res.glyphs if g.line == h.line]
    assert line_glyphs[-1] is h
    assert h.letter == max(g.letter for g in line_glyphs if not g.hyphen)
    nxt = [g for g in res.glyphs if g.line == h.line + 1]
    assert nxt[0].letter == h.letter + 1
    assert any(st.hyphen and st.word == 1 for st in res.strokes)
    assert sorted(g.letter for g in res.glyphs if not g.hyphen) == list(range(1, len("Электрификация") + 1))


def test_no_auto_hyphenation_when_disabled(settings, font):
    s = settings
    s.sheet.width, s.sheet.margin_left, s.sheet.margin_right = 80, 10, 5
    s.typography.hyphenate = False
    res, _ = run(s, font, "Мы электрификация")
    assert not any(g.hyphen for g in res.glyphs)
    assert {g.line for g in res.glyphs if g.word == 2} == {2}


def test_soft_hyphen_used_even_when_auto_off(settings, font):
    s = settings
    s.sheet.width, s.sheet.margin_left, s.sheet.margin_right = 80, 10, 5
    s.typography.hyphenate = False
    res, _ = run(s, font, "Мы электри­фикация")
    hy = [g for g in res.glyphs if g.hyphen]
    assert len(hy) == 1 and hy[0].letter == 7


def test_word_longer_than_line_is_forced_apart(settings, font):
    s = settings
    s.sheet.width, s.sheet.margin_left, s.sheet.margin_right = 40, 5, 5
    s.typography.hyphenate = False
    res, _ = run(s, font, "Ааааааааааааааааааааааааааааааааа")
    assert any(g.hyphen for g in res.glyphs)
    assert any("принудительно" in w for w in res.warnings)
    assert res.next_word is None


def test_overflow_reports_next_word_and_no_split_on_last_line(settings, font):
    s = settings
    s.sheet.height, s.sheet.first_line_top, s.sheet.bottom_limit, s.sheet.line_pitch = 40, 10, 10, 10
    words = "электрификация " * 40
    res, pt = run(s, font, words)
    assert res.used_lines == 3
    assert res.next_word is not None and res.next_letter == 1
    last_line = [g for g in res.glyphs if g.line == 3]
    assert not any(g.hyphen for g in last_line)
    assert res.last_word == res.next_word - 1
    assert res.last_letter == len(pt.words[res.last_word - 1].text)


def test_start_word(settings, font):
    settings.text_options.start_word = 3
    res, _ = run(settings, font, "\tодин два\n\n\tтри четыре")
    assert res.first_word == 3
    assert min(st.word for st in res.strokes) == 3
    first = [g for g in res.glyphs if g.line == 1]
    assert first[0].word == 3
    assert first[0].x == pytest.approx(settings.sheet.margin_left + settings.sheet.indent)


def test_rotation_and_offset_applied(settings, font):
    res0, _ = run(settings, font, "а")
    settings.typography.dx, settings.typography.dy = 1.5, -2.0
    res1, _ = run(settings, font, "а")
    p0, p1 = res0.strokes[0].points[0], res1.strokes[0].points[0]
    assert p1[0] - p0[0] == pytest.approx(1.5) and p1[1] - p0[1] == pytest.approx(-2.0)
