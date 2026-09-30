from __future__ import annotations

from dataclasses import dataclass, field

from .checks import check_bounds, check_printer, check_settings, check_sheet, travel_box
from .gcode import compute_stats, generate_gcode, test_pattern, travel_moves
from .glyphs import GlyphProvider, load_provider
from .layout import LayoutResult, layout
from .model import DrawnStroke
from .settings import Settings
from .text import ProcessedText, process_text


class GenerationRefused(Exception):
    def __init__(self, errors: list[str]):
        super().__init__("; ".join(errors))
        self.errors = errors


@dataclass
class Resume:
    word: int
    letter: int
    path: int
    point: int
    connected: bool


@dataclass
class Composition:
    settings: Settings
    provider: GlyphProvider | None
    text: ProcessedText | None
    layout: LayoutResult | None
    errors: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    resume: Resume | None = None

    @property
    def all_strokes(self) -> list[list[tuple[float, float]]]:
        return [st.points for st in self.layout.strokes] if self.layout else []

    @property
    def strokes(self) -> list[list[tuple[float, float]]]:
        if not self.layout:
            return []
        paths = [st.points for st in self.layout.strokes]
        r = self.resume
        if r is None:
            return paths
        return [paths[r.path][r.point:]] + paths[r.path + 1:]


def compose(s: Settings, provider: GlyphProvider | None = None) -> Composition:
    sheet_errors, warnings = check_sheet(s)
    printer_errors, printer_warnings = check_printer(s)
    errors = sheet_errors + printer_errors
    warnings += printer_warnings
    try:
        prov = (provider or load_provider(s.font, s.mode)).with_options(s.outline)
    except Exception as e:
        return Composition(s, None, None, None, errors + [f"Шрифт не загрузился: {e}"], warnings)
    pt = process_text(s.text, prov.has_char, s.text_options.replacements, s.text_options.missing)
    if pt.unresolved:
        chars = " ".join(f"«{c}»" for c in sorted(pt.unresolved))
        errors.append(f"Нет в шрифте: {chars}. Выбери для них «пропустить» или «заменить»")
    lay = None
    resume = None
    if not sheet_errors:
        start = s.text_options.start_word
        names = {g.name for w in pt.words if w.index >= start for g in prov.shape(w.text)}
        names |= {g.name for g in prov.shape("-")}
        if s.randomness.enabled and s.randomness.variants:
            names |= {n for w in pt.words if w.index >= start for ch in w.text for n in prov.variant_pool(ch)}
        prov.prepare(names)
        lay = layout(s, prov, pt)
        errors += lay.errors
        warnings += lay.warnings
        if lay.next_word is not None:
            warnings.append(f"Текст не поместился: следующий лист начинать со слова {lay.next_word}"
                            + (f", буквы {lay.next_letter}" if lay.next_letter and lay.next_letter > 1 else ""))
        if s.text_options.resume_word:
            resume, err = find_resume(lay, s.text_options.resume_word, s.text_options.resume_letter)
            if err:
                errors.append(err)
        c = Composition(s, prov, pt, lay, errors, warnings, resume)
        errors += check_bounds(c.strokes, s)
        return c
    return Composition(s, prov, pt, lay, errors, warnings, resume)


def find_resume(lay: LayoutResult, word: int, letter: int) -> tuple[Resume | None, str | None]:
    targets = {i for i, g in enumerate(lay.glyphs) if g.word == word and g.letter == letter and not g.hyphen}
    if not targets:
        on_sheet = {g.word for g in lay.glyphs}
        if word not in on_sheet:
            return None, f"Продолжить с: слова {word} нет на этом листе (здесь слова {lay.first_word}–{lay.last_word})"
        return None, f"Продолжить с: в слове {word} нет буквы {letter}"
    for pi, st in enumerate(lay.strokes):
        for k, t in enumerate(st.tags):
            if t in targets:
                connected = k > 0
                return Resume(word, letter, pi, k - 1 if connected else 0, connected), None
    return None, f"Продолжить с: у буквы {letter} слова {word} нет штрихов (пропущенный символ?)"


def make_gcode(c: Composition) -> str:
    if c.errors:
        raise GenerationRefused(c.errors)
    if not c.strokes:
        raise GenerationRefused(["Нечего писать: текст пустой"])
    s, lay = c.settings, c.layout
    R = s.randomness
    rand = (f"seed: {R.seed}" + (f" (size {R.size}% slant {R.slant} offset {R.offset} letters {R.letter_spacing}% "
                                 f"words {R.word_spacing}% drift {R.drift} line start {R.line_start} "
                                 f"right edge {R.right_edge} jitter {R.jitter} variants {int(R.variants)})"
                                 if R.enabled else " (randomness off)"))
    header = [
        f"font: {c.provider.name} (mode {s.mode})",
        rand,
        f"connections: {'on, distance ' + str(s.connections.distance) if s.connections.enabled else 'off'}",
        f"sheet start: word {lay.first_word}, letter 1",
        f"words: {lay.first_word}..{lay.last_word} (last written: word {lay.last_word}, letter {lay.last_letter})",
        "next sheet: " + (f"word {lay.next_word}" + (f", letter {lay.next_letter}" if (lay.next_letter or 1) > 1 else "")
                          if lay.next_word else "text finished"),
    ]
    if c.resume:
        r = c.resume
        header.append(f"RESUME from word {r.word}, letter {r.letter}"
                      + (" (starts at the connection point)" if r.connected else ""))
    return generate_gcode(c.strokes, s, header)


def make_test_gcode(s: Settings) -> tuple[str, list[list[tuple[float, float]]]]:
    errors, _ = check_settings(s)
    strokes = test_pattern(s)
    errors += check_bounds(strokes, s)
    if errors:
        raise GenerationRefused(errors)
    return generate_gcode(strokes, s, ["TEST: writing area rectangle + axis arrows (X long, Y short)"]), strokes


def _glyph_boxes(lay: LayoutResult) -> dict[int, list[float]]:
    boxes: dict[int, list[float]] = {}
    for st in lay.strokes:
        for (x, y), t in zip(st.points, st.tags):
            b = boxes.get(t)
            if b is None:
                boxes[t] = [x, y, x, y]
            else:
                b[0], b[1], b[2], b[3] = min(b[0], x), min(b[1], y), max(b[2], x), max(b[3], y)
    return boxes


def preview_payload(c: Composition) -> dict:
    s = c.settings
    box = travel_box(s)
    lay = c.layout
    strokes = c.strokes
    out = {
        "errors": c.errors,
        "warnings": c.warnings,
        "sheet": s.sheet.model_dump(),
        "travel_box": {"x_min": box.x_min, "x_max": box.x_max, "y_min": box.y_min, "y_max": box.y_max,
                       "measured": box.measured},
        "flip_x": s.printer.flip_x, "flip_y": s.printer.flip_y,
        "safety_margin": s.printer.safety_margin,
        "strokes": [], "travel": [], "glyphs": [], "baselines": [],
        "stats": compute_stats(strokes, s).as_dict(),
        "missing": [], "font": None, "end": None, "resume": None,
    }
    if c.provider is not None:
        info = c.provider.info()
        m = info.metrics
        out["font"] = {"name": info.name, "mode": info.mode, "glyph_count": info.glyph_count,
                       "chars": info.chars, "variants": info.variants, "notes": info.notes,
                       "x_height": m.x_height if m else None,
                       "x_height_source": m.x_height_source if m else "",
                       "features": info.features, "gpos_features": info.gpos_features,
                       "variant_sources": info.variant_sources, "ligatures": info.ligatures}
    if c.text is not None:
        out["missing"] = [{"char": mc.char, "code": mc.code, "name": mc.name, "count": mc.count,
                           "positions": mc.positions,
                           "resolved": mc.char not in c.text.unresolved} for mc in c.text.missing]
        out["replaced"] = c.text.replaced
        out["word_count"] = len(c.text.words)
    if lay is not None:
        r2 = lambda p: [round(p[0], 3), round(p[1], 3)]
        r = c.resume

        def item(st: DrawnStroke, pts, done: bool):
            return {"p": [r2(p) for p in pts], "w": st.word, "l": st.letter, "n": st.line, "h": st.hyphen,
                    "d": done}

        items = []
        for pi, st in enumerate(lay.strokes):
            if r is None or pi > r.path:
                items.append(item(st, st.points, False))
            elif pi < r.path:
                items.append(item(st, st.points, True))
            else:
                if r.point > 0:
                    items.append(item(st, st.points[: r.point + 1], True))
                items.append(item(st, st.points[r.point:], False))
        out["strokes"] = items
        out["travel"] = [[r2(a), r2(b)] for a, b in travel_moves(strokes)]
        boxes = _glyph_boxes(lay)
        out["glyphs"] = [{"w": g.word, "l": g.letter, "n": g.line, "c": g.char, "x": round(g.x, 3),
                          "y": round(g.y, 3), "adv": round(g.advance, 3), "missing": g.missing,
                          "h": g.hyphen, "b": [round(v, 3) for v in boxes[i]] if i in boxes else None}
                         for i, g in enumerate(lay.glyphs)]
        out["baselines"] = lay.baselines
        out["scale"] = lay.scale
        out["end"] = {"first_word": lay.first_word, "last_word": lay.last_word, "last_letter": lay.last_letter,
                      "next_word": lay.next_word, "next_letter": lay.next_letter}
        if r:
            out["resume"] = {"word": r.word, "letter": r.letter, "connected": r.connected}
    return out
