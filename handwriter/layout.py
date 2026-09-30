from __future__ import annotations

from dataclasses import dataclass, field

from .glyphs import GlyphProvider
from .model import DrawnStroke, PlacedGlyph
from .rand import pick, urnd
from .settings import Settings
from .text import ProcessedText, Word, break_positions
from .writing import build_paths

__all__ = ["DrawnStroke", "PlacedGlyph", "LayoutResult", "layout", "line_baselines", "RandomSource"]

EPS = 1e-6
PLACEHOLDER_EM = 0.5


@dataclass
class LayoutResult:
    strokes: list[DrawnStroke] = field(default_factory=list)
    glyphs: list[PlacedGlyph] = field(default_factory=list)
    baselines: list[float] = field(default_factory=list)
    used_lines: int = 0
    scale: float = 0.0
    first_word: int | None = None
    last_word: int | None = None
    last_letter: int | None = None
    next_word: int | None = None
    next_letter: int | None = None
    warnings: list[str] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)


def line_baselines(s: Settings) -> list[float]:
    sh = s.sheet
    out = []
    y = sh.height - sh.first_line_top
    if sh.line_pitch <= 0:
        return [y] if y >= sh.bottom_limit - EPS else []
    while y >= sh.bottom_limit - EPS:
        out.append(y)
        y -= sh.line_pitch
    return out


class RandomSource:
    def __init__(self, s: Settings, pt: ProcessedText, prov: GlyphProvider):
        R = s.randomness
        self.R = R
        self.on = R.enabled
        self.seed = R.seed
        self.prov = prov
        self._variants: dict[int, dict[int, str]] = {}
        self.size_mean = 0.0
        if self.on and R.size > 0:
            us = [urnd(R.seed, "size", w.index, i + 1) for w in pt.words for i in range(len(w.text))]
            self.size_mean = sum(us) / len(us) if us else 0.0

    def size(self, w: int, l: int) -> float:
        if not self.on or self.R.size <= 0:
            return 1.0
        return 1.0 + self.R.size / 100.0 * (urnd(self.seed, "size", w, l) - self.size_mean)

    def slant(self, w: int, l: int) -> float:
        return self.R.slant * urnd(self.seed, "slant", w, l) if self.on else 0.0

    def voff(self, w: int, l: int) -> float:
        return self.R.offset * urnd(self.seed, "voff", w, l) if self.on else 0.0

    def letter_spacing(self, w: int, l: int) -> float:
        return 1.0 + self.R.letter_spacing / 100.0 * urnd(self.seed, "ls", w, l) if self.on else 1.0

    def word_spacing(self, w: int) -> float:
        return 1.0 + self.R.word_spacing / 100.0 * urnd(self.seed, "ws", w) if self.on else 1.0

    def line_start(self, line: int) -> float:
        return self.R.line_start * urnd(self.seed, "lstart", line) if self.on else 0.0

    def right_edge(self, line: int) -> float:
        return self.R.right_edge * urnd(self.seed, "redge", line) if self.on else 0.0

    def variants(self, word: Word) -> dict[int, str]:
        if not (self.on and self.R.variants):
            return {}
        got = self._variants.get(word.index)
        if got is None:
            got = {}
            last: dict[str, int] = {}
            for i, ch in enumerate(word.text):
                pool = self.prov.variant_pool(ch)
                n = len(pool)
                if n < 2:
                    continue
                k = pick(self.seed, "variant", n, word.index, i + 1)
                if last.get(ch) == k:
                    k = (k + 1 + pick(self.seed, "variant2", n - 1, word.index, i + 1)) % n
                last[ch] = k
                got[i + 1] = pool[k]
            self._variants[word.index] = got
        return got


@dataclass
class _Run:
    i: int
    name: str | None
    adv_em: float
    dx_em: float
    dy_em: float
    width: float


class _Measure:
    def __init__(self, prov: GlyphProvider, scale: float, skipped: set[str], rs: RandomSource):
        self.prov = prov
        self.scale = scale
        self.skipped = skipped
        self.rs = rs
        self._cache: dict[tuple[int, int, int], list[_Run]] = {}
        hy = prov.shape("-")
        self.hyphen = hy[0] if hy else None
        self.hyphen_w = hy[0].advance * scale if hy else 0.0

    def runs(self, word: Word, a: int, b: int) -> list[_Run]:
        key = (word.index, a, b)
        got = self._cache.get(key)
        if got is not None:
            return got
        text = word.text[a:b]
        chosen = self.rs.variants(word)
        by_cluster: dict[int, list] = {}
        for g in self.prov.shape(text):
            by_cluster.setdefault(g.cluster, []).append(g)
        covered = set()
        clusters = sorted(by_cluster)
        for k, c in enumerate(clusters):
            end = clusters[k + 1] if k + 1 < len(clusters) else len(text)
            covered.update(range(c, end))
        out = []
        for i, ch in enumerate(text):
            letter = a + i + 1
            if i in by_cluster:
                f = self.rs.size(word.index, letter) * self.rs.letter_spacing(word.index, letter)
                for g in by_cluster[i]:
                    name, adv = g.name, g.advance
                    alt = chosen.get(letter)
                    if alt and len(by_cluster[i]) == 1 and name == self.prov.variant_pool(ch)[0] and alt != name:
                        name, adv = alt, self.prov.advance(alt)
                    out.append(_Run(a + i, name, adv, g.x_offset, g.y_offset, adv * self.scale * f))
            elif i not in covered or not self.prov.has_char(ch):
                w = 0.0 if ch in self.skipped else PLACEHOLDER_EM * self.scale
                out.append(_Run(a + i, None, 0.0, 0.0, 0.0, w))
        self._cache[key] = out
        return out

    def width(self, word: Word, a: int, b: int) -> float:
        return sum(r.width for r in self.runs(word, a, b))


def layout(s: Settings, prov: GlyphProvider, pt: ProcessedText) -> LayoutResult:
    res = LayoutResult()
    sh, ty = s.sheet, s.typography
    m = prov.metrics
    if m.x_height <= 0:
        res.errors.append("У шрифта нулевая высота строчной буквы")
        return res
    scale = ty.size_mm / m.x_height
    res.scale = scale
    bases = line_baselines(s)
    res.baselines = [b + ty.baseline_shift for b in bases]
    n_lines = len(bases)
    if n_lines == 0:
        res.errors.append("На листе не помещается ни одной строки: проверь первую строку и нижний предел")
        return res
    if not pt.words:
        return res

    rs = RandomSource(s, pt, prov)
    meas = _Measure(prov, scale, pt.skipped, rs)
    if meas.hyphen is None:
        res.warnings.append("В шрифте нет дефиса «-»: переносы будут без дефиса")
    space_w = prov.space_advance() * scale
    x_left = sh.margin_left
    x_right = sh.width - sh.margin_right

    start = max(1, min(s.text_options.start_word, len(pt.words)))
    if s.text_options.start_word > len(pt.words):
        res.warnings.append(f"В тексте только {len(pt.words)} слов, начинаю со слова {start}")
    res.first_word = start

    state = {"line": 1, "x": x_left + rs.line_start(1), "empty": True, "segment": 0}
    placed_any = False

    def right(line: int) -> float:
        return x_right + rs.right_edge(line)

    def gap(word: Word) -> float:
        return 0.0 if state["empty"] else space_w * rs.word_spacing(word.index)

    def place_segment(word: Word, a: int, b: int, with_hyphen: bool):
        line = state["line"]
        base = bases[line - 1] + ty.baseline_shift
        x = state["x"] + gap(word)
        seg = state["segment"]
        state["segment"] += 1
        for r in meas.runs(word, a, b):
            ch = word.text[r.i]
            letter = r.i + 1
            g = PlacedGlyph(word=word.index, letter=letter, line=line, char=ch, glyph=r.name, x=x, y=base,
                            advance=r.width, missing=r.name is None and ch not in pt.skipped, segment=seg,
                            adv_em=r.adv_em, dx_em=r.dx_em, dy_em=r.dy_em,
                            size=rs.size(word.index, letter), slant=rs.slant(word.index, letter),
                            voff=rs.voff(word.index, letter))
            res.glyphs.append(g)
            x += r.width
        res.last_word, res.last_letter = word.index, b
        if with_hyphen and meas.hyphen is not None:
            hg = meas.hyphen
            res.glyphs.append(PlacedGlyph(word=word.index, letter=b, line=line, char="-", glyph=hg.name,
                                          x=x, y=base, advance=meas.hyphen_w, hyphen=True, segment=seg,
                                          adv_em=hg.advance))
            x += meas.hyphen_w
        state["x"] = x
        state["empty"] = False
        res.used_lines = max(res.used_lines, line)

    def new_line(skip: int = 0) -> bool:
        state["line"] += 1 + skip
        state["x"] = x_left + rs.line_start(state["line"])
        state["empty"] = True
        return state["line"] <= n_lines

    def overflow(word: Word, a: int):
        res.next_word, res.next_letter = word.index, a + 1

    for p in pt.paragraphs:
        words = [w for w in p.words if w.index >= start]
        if not words:
            continue
        paragraph_start = words[0] is p.words[0]
        if placed_any:
            if not new_line(p.blank_before if paragraph_start else 0):
                overflow(words[0], 0)
                break
        elif paragraph_start and start == 1 and p.blank_before:
            new_line(p.blank_before - 1)
            if state["line"] > n_lines:
                overflow(words[0], 0)
                break
        if paragraph_start and p.indent:
            state["x"] += sh.indent
        placed_any = True
        stop = False
        for word in words:
            a = 0
            breaks = break_positions(word, ty.hyphenate)
            while True:
                avail = right(state["line"]) - state["x"] - gap(word)
                if meas.width(word, a, len(word.text)) <= avail + EPS:
                    place_segment(word, a, len(word.text), False)
                    break
                last_line = state["line"] >= n_lines
                b = None
                if not last_line:
                    b = _best_break(word, a, breaks, avail, meas)
                if b is not None:
                    place_segment(word, a, b, breaks[b])
                    a = b
                    if not new_line():
                        overflow(word, a)
                        stop = True
                        break
                    continue
                if not state["empty"]:
                    if not new_line():
                        overflow(word, a)
                        stop = True
                        break
                    continue
                b = _forced_break(word, a, avail, meas)
                if b is None:
                    res.errors.append(f"Строка слишком узкая: не помещается даже одна буква слова {word.index}")
                    overflow(word, a)
                    stop = True
                    break
                place_segment(word, a, b, True)
                res.warnings.append(f"Слово {word.index} длиннее строки, разбито принудительно")
                a = b
                if not new_line():
                    overflow(word, a)
                    stop = True
                    break
            if stop:
                break
        if stop:
            break

    res.strokes = build_paths(s, prov, res.glyphs, scale)
    return res


def _best_break(word: Word, a: int, breaks: dict[int, bool], avail: float, meas: _Measure) -> int | None:
    best = None
    for b, need_h in breaks.items():
        if b <= a or b >= len(word.text):
            continue
        w = meas.width(word, a, b) + (meas.hyphen_w if need_h else 0.0)
        if w <= avail + EPS:
            best = b
    return best


def _forced_break(word: Word, a: int, avail: float, meas: _Measure) -> int | None:
    best = None
    for b in range(a + 1, len(word.text)):
        if meas.width(word, a, b) + meas.hyphen_w <= avail + EPS:
            best = b
        else:
            break
    return best
