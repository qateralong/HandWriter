from __future__ import annotations

import xml.etree.ElementTree as ET
from pathlib import Path

from HersheyFonts import HersheyFonts

OUT = Path(__file__).resolve().parent.parent / "handwriter" / "fonts" / "hershey_cyrillic.svg"

UPM = 1000
HERSHEY_EM = 32.0
BASE_Y = 9.0
S = UPM / HERSHEY_EM

UPPER = "АБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩ"
LOWER = UPPER.lower()


def load(name):
    h = HersheyFonts()
    h.load_default_font(name)
    return h.all_glyphs


def build_map():
    c1, cy, tr = load("cyrilc_1"), load("cyrillic"), load("timesr")
    m: dict[str, object] = {}
    for i, ch in enumerate(UPPER):
        m[ch] = c1[chr(ord("A") + i)]
    for i, ch in enumerate(LOWER):
        m[ch] = c1[chr(ord("a") + i)]
    for slot, ch in {"$": "Ы", "]": "Ъ", "^": "Я", "_": "Ь", "C": "Э", "U": "Ю",
                     "&": "ы", "|": "ъ", "}": "я", "~": "ь", "u": "ю",
                     '"': '"'}.items():
        m[ch] = cy[slot]
    for ch in " !#$%&'()*+,-./0123456789:;<=>?@[\\]_{|}~":
        m.setdefault(ch, c1[ch])
    m["°"] = c1["\x7f"]
    for i in range(26):
        m[chr(ord("A") + i)] = tr[chr(ord("A") + i)]
        m[chr(ord("a") + i)] = tr[chr(ord("a") + i)]
    return m


def to_font(g):
    left = g.left_offset
    strokes = [[((x - left) * S, (BASE_Y - y) * S) for x, y in st] for st in g.strokes]
    return strokes, g.char_width * S


def dots_for(strokes, dot_strokes, gap=2.6 * S, lift=3.2 * S, scale=0.8):
    xs = [p[0] for s in strokes for p in s]
    ys = [p[1] for s in strokes for p in s]
    cx, top = (min(xs) + max(xs)) / 2, max(ys)
    dxs = [p[0] for s in dot_strokes for p in s]
    dys = [p[1] for s in dot_strokes for p in s]
    dcx, dcy = (min(dxs) + max(dxs)) / 2, (min(dys) + max(dys)) / 2
    out = []
    for sign in (-1, 1):
        ox, oy = cx + sign * gap - dcx * scale, top + lift - dcy * scale
        out += [[(ox + x * scale, oy + y * scale) for x, y in s] for s in dot_strokes]
    return out


def small_e_rev(strokes, adv):
    mirrored = [[(adv - x, y) for x, y in reversed(s)] for s in strokes]
    xs = [p[0] for s in mirrored for p in s]
    ys = [p[1] for s in mirrored for p in s]
    x0, x1 = min(xs), max(xs)
    w, ymid = x1 - x0, (min(ys) + max(ys)) / 2
    bar = [(x0 + 0.35 * w, ymid), (x1 - 0.12 * w, ymid)]
    return mirrored + [bar], adv


def d_attr(strokes):
    parts = []
    for s in strokes:
        if not s:
            continue
        pts = [f"{x:.2f} {y:.2f}".replace(".00", "") for x, y in s]
        parts.append("M" + pts[0] + ("L" + "L".join(pts[1:]) if len(pts) > 1 else ""))
    return "".join(parts)


def main():
    m = build_map()
    font_glyphs = {ch: to_font(g) for ch, g in m.items()}
    dot, _ = font_glyphs["."]
    for base, new in (("Е", "Ё"), ("е", "ё")):
        st, adv = font_glyphs[base]
        font_glyphs[new] = (st + dots_for(st, dot), adv)
    font_glyphs["э"] = small_e_rev(*font_glyphs["с"])

    x_top = max(p[1] for s in font_glyphs["х"][0] for p in s)
    cap_top = max(p[1] for s in font_glyphs["Н"][0] for p in s)
    all_y = [p[1] for st, _ in font_glyphs.values() for s in st for p in s]

    ET.register_namespace("", "http://www.w3.org/2000/svg")
    svg = ET.Element("{http://www.w3.org/2000/svg}svg")
    meta = ET.SubElement(svg, "metadata")
    meta.text = ("Hershey Complex Cyrillic + Complex Roman (A. V. Hershey, NBS, 1967), public domain. "
                 "Converted from the Hershey-Fonts Python package by tools/build_hershey_font.py. "
                 "Single-line (stroke) font: paths are centre lines, not outlines.")
    defs = ET.SubElement(svg, "defs")
    font = ET.SubElement(defs, "font", {"id": "HersheyComplexCyrillic", "horiz-adv-x": "500"})
    ET.SubElement(font, "font-face", {
        "font-family": "Hershey Complex Cyrillic", "units-per-em": str(UPM),
        "ascent": f"{max(all_y):.0f}", "descent": f"{min(all_y):.0f}",
        "x-height": f"{x_top:.0f}", "cap-height": f"{cap_top:.0f}",
    })
    for ch in sorted(font_glyphs):
        st, adv = font_glyphs[ch]
        attrs = {"unicode": ch, "glyph-name": f"uni{ord(ch):04X}", "horiz-adv-x": f"{adv:.0f}"}
        d = d_attr(st)
        if d:
            attrs["d"] = d
        ET.SubElement(font, "glyph", attrs)
    ET.indent(svg)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(svg).write(OUT, encoding="utf-8", xml_declaration=True)
    print(f"{OUT}: {len(font_glyphs)} глифов, x-height {x_top:.0f}/{UPM}, cap {cap_top:.0f}/{UPM}")


if __name__ == "__main__":
    main()
