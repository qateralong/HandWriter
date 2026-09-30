import pytest

from handwriter.text import Word, break_positions, strip_soft_hyphens


def W(text):
    visible, soft = strip_soft_hyphens(text)
    return Word(index=1, text=visible, paragraph=1, text_line=1, soft_breaks=soft)


@pytest.mark.parametrize("word", ["кот", "да", "и", "ёж", "мир!", "Он,", "123"])
def test_no_hyphenation_in_short_words(word):
    assert break_positions(W(word), auto=True) == {}


@pytest.mark.parametrize("word", ["переносы", "электрификация", "подъёму", "французских",
                                  "сельского", "Широкая", "хозяйства", "автоматические"])
def test_at_least_two_letters_each_side(word):
    bps = break_positions(W(word), auto=True)
    assert bps, word
    for b, need_hyphen in bps.items():
        assert b >= 2 and len(word) - b >= 2, (word, b)
        assert need_hyphen


def test_known_breaks():
    assert set(break_positions(W("переносы"), auto=True)) == {2, 4, 6}


def test_punctuation_not_counted_as_letters():
    bps = break_positions(W("переносы,"), auto=True)
    assert max(bps) <= len("переносы") - 2


def test_auto_off_keeps_soft_and_hard_hyphens():
    assert break_positions(W("электрификация"), auto=False) == {}
    assert break_positions(W("элек­три­фикация"), auto=False) == {4: True, 7: True}
    assert break_positions(W("кто-нибудь"), auto=False) == {4: False}


def test_soft_hyphen_always_counts_with_auto_on():
    bps = break_positions(W("ab­cdefgh"), auto=True)
    assert bps.get(2) is True
