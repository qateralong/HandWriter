from __future__ import annotations

import json
import re
import unicodedata
import xml.etree.ElementTree as ET
from pathlib import Path

from .base import FontInfo, FontMetrics, Glyph, GlyphProvider
from .svgparse import (local_tag, mat_mul, num, parse_transform, parse_viewbox,
                       path_d_to_strokes, walk_strokes)

ALIASES: dict[str, str] = {
    "space": " ", "exclam": "!", "quotedbl": '"', "numbersign": "#", "dollar": "$",
    "percent": "%", "ampersand": "&", "quotesingle": "'", "parenleft": "(",
    "parenright": ")", "asterisk": "*", "plus": "+", "comma": ",", "hyphen": "-",
    "period": ".", "slash": "/", "colon": ":", "semicolon": ";", "less": "<",
    "equal": "=", "greater": ">", "question": "?", "at": "@", "bracketleft": "[",
    "backslash": "\\", "bracketright": "]", "underscore": "_", "braceleft": "{",
    "bar": "|", "braceright": "}", "asciitilde": "~", "numero": "№",
    "guillemotleft": "«", "guillemotright": "»", "emdash": "—", "endash": "–",
    "ellipsis": "…", "degree": "°",
}

FLATTEN_TOL_EM = 0.0005

_UNI_RE = re.compile(r"^(?:uni|u\+|u)([0-9a-f]{4,6})$", re.IGNORECASE)
_VARIANT_RE = re.compile(r"^(.*)\.(\d+|alt\d*)$", re.IGNORECASE)


def char_from_stem(stem: str) -> str | None:
    stem = unicodedata.normalize("NFC", stem)
    if len(stem) == 1:
        return stem
    m = _UNI_RE.match(stem)
    if m:
        return chr(int(m.group(1), 16))
    return ALIASES.get(stem)


def _default_glyph_name(ch: str) -> str:
    return f"uni{ord(ch):04X}"


class StrokeGlyphProvider(GlyphProvider):
    mode = "strokes"

    def __init__(self, name: str, glyphs: dict[str, Glyph], cmap: dict[str, list[str]],
                 metrics: FontMetrics, source: str = "", notes: list[str] | None = None):
        self._name = name
        self._glyphs = glyphs
        self._cmap = cmap
        self._metrics = metrics
        self.source = source
        self.notes = notes or []

    @property
    def name(self) -> str:
        return self._name

    @property
    def metrics(self) -> FontMetrics:
        return self._metrics

    def glyph(self, name: str) -> Glyph:
        return self._glyphs[name]

    def glyph_names_for_char(self, ch: str) -> list[str]:
        return list(self._cmap.get(ch, ()))

    def variants(self) -> dict[str, list[str]]:
        return {ch: list(v) for ch, v in self._cmap.items() if len(v) > 1}

    def info(self) -> FontInfo:
        return FontInfo(
            name=self._name, mode=self.mode, source=self.source,
            glyph_count=len(self._glyphs),
            chars="".join(sorted(self._cmap)),
            variants=self.variants(), metrics=self._metrics, notes=list(self.notes),
        )

    @classmethod
    def from_path(cls, path: str | Path) -> "StrokeGlyphProvider":
        p = Path(path)
        if p.is_dir():
            return cls.from_svg_folder(p)
        if p.suffix.lower() == ".svg":
            return cls.from_svg_font(p)
        raise ValueError(f"Режим «Штрихи» принимает SVG-шрифт или папку с SVG: {p}")

    @classmethod
    def from_svg_font(cls, path: str | Path) -> "StrokeGlyphProvider":
        path = Path(path)
        root = ET.parse(path).getroot()
        font = next((el for el in root.iter() if local_tag(el) == "font"), None)
        if font is None:
            raise ValueError(f"В файле нет элемента <font>, это не SVG-шрифт: {path.name}")
        face = next((el for el in font.iter() if local_tag(el) == "font-face"), None)
        fa = face.attrib if face is not None else {}
        upm = num(fa.get("units-per-em"), 1000.0) or 1000.0
        default_adv = num(font.attrib.get("horiz-adv-x"), upm / 2)
        name = fa.get("font-family") or font.attrib.get("id") or path.stem

        s = 1.0 / upm
        scale = (s, 0.0, 0.0, s, 0.0, 0.0)
        glyphs: dict[str, Glyph] = {}
        cmap: dict[str, list[str]] = {}
        unnamed_variants: list[tuple[str, Glyph]] = []
        notes: list[str] = []
        for el in font:
            if local_tag(el) != "glyph":
                continue
            uni = el.attrib.get("unicode")
            gname = el.attrib.get("glyph-name")
            adv = num(el.attrib.get("horiz-adv-x"), default_adv) * s
            m = mat_mul(scale, parse_transform(el.attrib.get("transform")))
            strokes = []
            d = el.attrib.get("d")
            if d:
                strokes += path_d_to_strokes(d, m, FLATTEN_TOL_EM)
            for child in el:
                strokes += walk_strokes(child, m, FLATTEN_TOL_EM)
            uni = unicodedata.normalize("NFC", uni) if uni else uni
            if uni is not None and len(uni) != 1:
                notes.append(f"Глиф для «{uni}» (лигатура) пропущен: в режиме «Штрихи» лигатур нет")
                continue
            if not gname:
                if uni is None:
                    continue
                base = _default_glyph_name(uni)
                gname = base if base not in glyphs else f"{base}.{len(cmap.get(uni, [])) + 1}"
            while gname in glyphs:
                gname += "_"
            g = Glyph(name=gname, strokes=_freeze(strokes), advance=adv)
            glyphs[gname] = g
            if uni is not None:
                cmap.setdefault(uni, []).append(gname)
            else:
                unnamed_variants.append((gname, g))
        by_name = {n: ch for ch, names in cmap.items() for n in names}
        for gname, _g in unnamed_variants:
            vm = _VARIANT_RE.match(gname)
            if vm and vm.group(1) in by_name:
                cmap[by_name[vm.group(1)]].append(gname)
        for ch in cmap:
            cmap[ch] = _primary_first(cmap[ch])

        metrics = _metrics(glyphs, cmap,
                           x_height=num(fa.get("x-height"), 0) / upm if fa.get("x-height") else None,
                           cap_height=num(fa.get("cap-height"), 0) / upm if fa.get("cap-height") else None,
                           ascent=num(fa.get("ascent"), 0) / upm if fa.get("ascent") else None,
                           descent=num(fa.get("descent"), 0) / upm if fa.get("descent") else None)
        return cls(name=name, glyphs=glyphs, cmap=cmap, metrics=metrics, source=str(path), notes=notes)

    @classmethod
    def from_svg_folder(cls, folder: str | Path) -> "StrokeGlyphProvider":
        folder = Path(folder)
        cfg: dict = {}
        cfg_path = folder / "font.json"
        if cfg_path.exists():
            cfg = json.loads(cfg_path.read_text(encoding="utf-8"))
        files = sorted(folder.glob("*.svg"), key=lambda p: p.name)
        if not files:
            raise ValueError(f"В папке нет SVG-файлов: {folder}")
        glyphs: dict[str, Glyph] = {}
        cmap: dict[str, list[str]] = {}
        notes: list[str] = []
        upm_cfg = cfg.get("units_per_em")
        base_cfg = cfg.get("baseline")
        upm_seen = None
        for f in files:
            stem = f.name[:-4]
            variant = 1
            vm = _VARIANT_RE.match(stem)
            if vm and char_from_stem(vm.group(1)) is not None and char_from_stem(stem) is None:
                stem, v = vm.group(1), vm.group(2)
                variant = int(v) if v.isdigit() else 2
            ch = char_from_stem(stem)
            if ch is None:
                notes.append(f"Файл {f.name}: не понял, какой это символ, пропущен")
                continue
            root = ET.parse(f).getroot()
            vb = parse_viewbox(root)
            if vb is None:
                notes.append(f"Файл {f.name}: нет viewBox/width/height, пропущен")
                continue
            vx, vy, vw, vh = vb
            upm = float(upm_cfg) if upm_cfg else vh
            upm_seen = upm_seen or upm
            baseline = float(base_cfg) if base_cfg is not None else vy + vh * 0.8
            s = 1.0 / upm
            m = (s, 0.0, 0.0, -s, -vx * s, baseline * s)
            strokes = walk_strokes(root, m, FLATTEN_TOL_EM)
            adv_attr = root.attrib.get("data-advance") or root.attrib.get("horiz-adv-x")
            adv = (num(adv_attr) if adv_attr else vw) * s
            gname = _default_glyph_name(ch) + (f".{variant}" if variant > 1 else "")
            while gname in glyphs:
                gname += "_"
            glyphs[gname] = Glyph(name=gname, strokes=_freeze(strokes), advance=adv)
            cmap.setdefault(ch, []).append(gname)
        for ch in cmap:
            cmap[ch] = _primary_first(cmap[ch])
        u = float(upm_cfg or upm_seen or 1.0)

        def cfg_em(key):
            v = cfg.get(key)
            return None if v is None else float(v) / u

        metrics = _metrics(glyphs, cmap, x_height=cfg_em("x_height"), cap_height=cfg_em("cap_height"),
                           ascent=cfg_em("ascent"), descent=cfg_em("descent"))
        if base_cfg is None:
            notes.append("Нет font.json: базовая линия принята на 80% высоты viewBox сверху")
        return cls(name=cfg.get("name") or folder.name, glyphs=glyphs, cmap=cmap,
                   metrics=metrics, source=str(folder), notes=notes)


def _freeze(strokes) -> tuple:
    out = []
    for s in strokes:
        pts = tuple((float(x), float(y)) for x, y in s)
        if pts:
            out.append(pts)
    return tuple(out)


def _primary_first(names: list[str]) -> list[str]:
    def key(n: str):
        m = _VARIANT_RE.match(n)
        if not m:
            return (0, 0, n)
        v = m.group(2)
        return (1, int(v) if v.isdigit() else 999, n)
    return sorted(dict.fromkeys(names), key=key)


def _top_of(glyphs, cmap, chars: str) -> tuple[float, str] | None:
    for ch in chars:
        names = cmap.get(ch)
        if names:
            bb = glyphs[names[0]].bbox()
            if bb and bb[3] > 0:
                return bb[3], ch
    return None


def _metrics(glyphs, cmap, x_height=None, cap_height=None, ascent=None, descent=None) -> FontMetrics:
    src = "из файла шрифта"
    if not x_height:
        t = _top_of(glyphs, cmap, "хxzvwuо")
        if t:
            x_height, src = t[0], f"по глифу «{t[1]}»"
        else:
            x_height, src = 0.5, "не найдена, принято 0.5 em"
    if not cap_height:
        t = _top_of(glyphs, cmap, "НHXТIЕE")
        cap_height = t[0] if t else x_height * 1.4
    ys = [p[1] for g in glyphs.values() for s in g.strokes for p in s]
    if ascent is None:
        ascent = max(ys) if ys else cap_height
    if descent is None:
        descent = min(min(ys), 0.0) if ys else -0.25
    if descent > 0:
        descent = -descent
    return FontMetrics(x_height=x_height, cap_height=cap_height, ascent=ascent, descent=descent,
                       x_height_source=src)
