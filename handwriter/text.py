from __future__ import annotations

import re
import unicodedata
from dataclasses import dataclass, field
from functools import lru_cache

import pyphen

SOFT_HYPHEN = "­"
_WORD_SPLIT = re.compile(r"[ \t]+")
_LETTER_RUN = re.compile(r"[^\W\d_]+")


@dataclass
class Word:
    index: int
    text: str
    paragraph: int
    text_line: int
    soft_breaks: frozenset[int] = frozenset()


@dataclass
class Paragraph:
    index: int
    indent: bool
    blank_before: int
    words: list[Word]


@dataclass
class MissingChar:
    char: str
    count: int
    positions: list[dict]

    @property
    def code(self) -> str:
        return f"U+{ord(self.char):04X}"

    @property
    def name(self) -> str:
        return unicodedata.name(self.char, "?")


@dataclass
class ProcessedText:
    paragraphs: list[Paragraph]
    words: list[Word]
    missing: list[MissingChar] = field(default_factory=list)
    skipped: set[str] = field(default_factory=set)
    unresolved: set[str] = field(default_factory=set)
    replaced: dict[str, str] = field(default_factory=dict)


def normalize(text: str) -> str:
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    return unicodedata.normalize("NFC", text)


def apply_replacements(text: str, has_char, replacements, missing_choices) -> tuple[str, dict[str, str]]:
    table: dict[str, str] = {}
    for src, dst in replacements:
        if len(src) == 1 and not has_char(src):
            table[src] = dst
    for ch, choice in (missing_choices or {}).items():
        if len(ch) == 1 and choice.action == "replace" and not has_char(ch):
            table[ch] = unicodedata.normalize("NFC", choice.replacement)
    used: dict[str, str] = {}
    out = []
    for ch in text:
        if ch in table:
            used[ch] = table[ch]
            out.append(table[ch])
        else:
            out.append(ch)
    return "".join(out), used


def process_text(text: str, has_char, replacements=(), missing_choices=None) -> ProcessedText:
    text = normalize(text)
    text, used = apply_replacements(text, has_char, replacements, missing_choices)
    missing_choices = missing_choices or {}

    paragraphs: list[Paragraph] = []
    words: list[Word] = []
    blank = 0
    for line_no, line in enumerate(text.split("\n"), start=1):
        body = line.lstrip(" \t")
        if not body.strip(" \t"):
            blank += 1
            continue
        lead = line[: len(line) - len(body)]
        p = Paragraph(index=len(paragraphs) + 1, indent="\t" in lead, blank_before=blank, words=[])
        blank = 0
        for token in _WORD_SPLIT.split(body.strip(" \t")):
            visible, soft = strip_soft_hyphens(token)
            if not visible:
                continue
            w = Word(index=len(words) + 1, text=visible, paragraph=p.index, text_line=line_no,
                     soft_breaks=soft)
            p.words.append(w)
            words.append(w)
        if p.words:
            paragraphs.append(p)

    missing: dict[str, MissingChar] = {}
    for w in words:
        for i, ch in enumerate(w.text):
            if has_char(ch):
                continue
            mc = missing.setdefault(ch, MissingChar(ch, 0, []))
            mc.count += 1
            if len(mc.positions) < 20:
                mc.positions.append({"line": w.text_line, "word": w.index, "letter": i + 1})
    skipped = {ch for ch in missing if ch in missing_choices and missing_choices[ch].action == "skip"}
    unresolved = set(missing) - skipped
    return ProcessedText(paragraphs=paragraphs, words=words, missing=list(missing.values()),
                         skipped=skipped, unresolved=unresolved, replaced=used)


def strip_soft_hyphens(token: str) -> tuple[str, frozenset[int]]:
    out, breaks = [], set()
    for ch in token:
        if ch == SOFT_HYPHEN:
            if out:
                breaks.add(len(out))
        else:
            out.append(ch)
    visible = "".join(out)
    return visible, frozenset(b for b in breaks if 0 < b < len(visible))


@lru_cache(maxsize=4)
def _dictionary(lang: str = "ru_RU") -> pyphen.Pyphen:
    return pyphen.Pyphen(lang=lang, left=2, right=2)


def break_positions(word: Word, auto: bool) -> dict[int, bool]:
    text = word.text
    res: dict[int, bool] = {b: True for b in word.soft_breaks}
    for i, ch in enumerate(text):
        if ch == "-" and 0 < i < len(text) - 1:
            res[i + 1] = False
    if auto:
        for m in _LETTER_RUN.finditer(text):
            run = m.group(0)
            if len(run) < 4:
                continue
            dic = _dictionary("ru_RU" if re.search(r"[Ѐ-ӿ]", run) else "en_US")
            for pos in dic.positions(run):
                res.setdefault(m.start() + pos, True)
    return dict(sorted(res.items()))
