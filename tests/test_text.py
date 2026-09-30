import unicodedata

from handwriter.settings import MissingChoice, TextOptions
from handwriter.text import normalize, process_text

DEFAULT_REPL = TextOptions().replacements


def proc(text, font, missing=None):
    return process_text(text, font.has_char, DEFAULT_REPL, missing)


def test_nfc_composes_short_i_and_yo(font):
    decomposed = "йод ёж"
    assert len(decomposed) == 8
    out = normalize(decomposed)
    assert out == "йод ёж"
    pt = proc(decomposed, font)
    assert [w.text for w in pt.words] == ["йод", "ёж"]
    assert pt.missing == []


def test_nfc_does_not_touch_composed_text():
    assert normalize("уже составлено: йё") == unicodedata.normalize("NFC", "уже составлено: йё")


def test_newlines_normalized():
    assert normalize("а\r\nб\rв") == "а\nб\nв"


def test_default_replacements_for_chars_missing_in_font(font):
    pt = proc("«Да» — сказал он… ну", font)
    assert [w.text for w in pt.words] == ['"Да"', "-", "сказал", "он...", "ну"]
    assert pt.missing == []
    assert pt.replaced["«"] == '"' and pt.replaced["…"] == "..."


def test_replacement_not_applied_if_font_has_char(font):
    pt = process_text("а-б", font.has_char, [("-", "x")], None)
    assert pt.words[0].text == "а-б"


def test_missing_chars_listed_with_positions(font):
    pt = proc("Слово №5 и ещё №7\nвторая ∑", font)
    chars = {m.char: m for m in pt.missing}
    assert set(chars) == {"№", "∑"}
    assert chars["№"].count == 2
    assert chars["№"].positions[0] == {"line": 1, "word": 2, "letter": 1}
    assert chars["∑"].positions[0] == {"line": 2, "word": 7, "letter": 1}
    assert pt.unresolved == {"№", "∑"}


def test_missing_choice_skip_and_replace(font):
    missing = {"№": MissingChoice(action="replace", replacement="N"), "∑": MissingChoice(action="skip")}
    pt = proc("№5 ∑", font, missing)
    assert pt.words[0].text == "N5"
    assert pt.unresolved == set()
    assert pt.skipped == {"∑"}


def test_words_numbered_across_text_with_punctuation(font):
    pt = proc("\tПривет, мир!\n\nВторой  абзац.", font)
    assert [(w.index, w.text, w.paragraph) for w in pt.words] == [
        (1, "Привет,", 1), (2, "мир!", 1), (3, "Второй", 2), (4, "абзац.", 2)]
    assert pt.words[0].text[6] == ","


def test_paragraphs_indent_blank_lines_and_leading_spaces(font):
    pt = proc("\tПервый\n   второй без отступа\n\n\n\tтретий", font)
    ps = pt.paragraphs
    assert [p.indent for p in ps] == [True, False, True]
    assert [p.blank_before for p in ps] == [0, 0, 2]
    assert ps[1].words[0].text == "второй"


def test_soft_hyphen_removed_and_recorded(font):
    pt = proc("пере­нос", font)
    w = pt.words[0]
    assert w.text == "перенос"
    assert w.soft_breaks == frozenset({4})
