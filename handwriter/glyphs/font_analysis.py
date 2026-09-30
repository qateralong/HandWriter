from __future__ import annotations

import re
import unicodedata

VARIANT_FEATURES = {"calt", "salt", "rand", "swsh", "cswh", "init", "medi", "fina", "isol", "hist"} | {
    f"ss{i:02d}" for i in range(1, 21)} | {f"cv{i:02d}" for i in range(1, 100)}
TECH_SUFFIX = re.compile(r"^(lf|tf|osf|onum|lnum|pnum|tnum|sups|subs|sinf|numr|dnom|sc|c2sc|smcp|case|cy|"
                         r"locl.*|ordn|frac|dflt|null|notdef|superior|inferior)$", re.IGNORECASE)
LIGATURE_FEATURES = {"liga", "clig", "rlig", "dlig", "calt"}
FREE_FEATURES = {"salt", "rand"} | {f"ss{i:02d}" for i in range(1, 21)} | {f"cv{i:02d}" for i in range(1, 100)}
POSITIONAL = {"init", "medi", "fina", "isol", "calt", "swsh", "cswh", "hist"}


def is_free_variant(source: str) -> bool:
    if source in FREE_FEATURES:
        return True
    if source.startswith("суффикс ."):
        suffix = source[len("суффикс ."):].split(".")[0].lower()
        return suffix not in POSITIONAL and not TECH_SUFFIX.match(suffix)
    return False


SHAPING_FEATURES = {"calt": True, "liga": True, "clig": True, "rlig": True, "locl": True, "kern": True}


def _nested_lookups(obj, out: set[int], depth=0) -> None:
    if depth > 6 or obj is None:
        return
    if isinstance(obj, list):
        for x in obj:
            _nested_lookups(x, out, depth + 1)
        return
    if type(obj).__name__ in ("SubstLookupRecord", "LookupRecord") and hasattr(obj, "LookupListIndex"):
        out.add(obj.LookupListIndex)
        return
    d = getattr(obj, "__dict__", None)
    if not d:
        return
    for k, v in d.items():
        if k in ("Coverage", "BacktrackCoverage", "LookAheadCoverage", "InputCoverage", "ClassDef",
                 "BacktrackClassDef", "InputClassDef", "LookAheadClassDef"):
            continue
        if not isinstance(v, (str, int, float, bytes, dict)):
            _nested_lookups(v, out, depth + 1)


def _subtables(lookup):
    for st in lookup.SubTable:
        if lookup.LookupType == 7:
            yield st.ExtensionLookupType, st.ExtSubTable
        else:
            yield lookup.LookupType, st


def analyze(ttfont) -> dict:
    cmap = ttfont.getBestCmap() or {}
    glyph_to_char: dict[str, str] = {}
    for code, name in sorted(cmap.items()):
        glyph_to_char.setdefault(name, chr(code))
    res = {"gsub_features": [], "gpos_features": [], "variants": {}, "ligatures": []}
    variants: dict[str, dict[str, set[str]]] = {}

    def add_variant(src_glyph, dst_glyph, source):
        ch = glyph_to_char.get(src_glyph)
        if ch is None:
            for c, gl in variants.items():
                if src_glyph in gl:
                    ch = c
                    break
        if ch is None or dst_glyph == cmap.get(ord(ch)):
            return
        if unicodedata.category(ch)[0] not in "LNP":
            return
        if any(TECH_SUFFIX.match(part) for part in dst_glyph.split(".")[1:]):
            return
        variants.setdefault(ch, {}).setdefault(dst_glyph, set()).add(source)

    if "GPOS" in ttfont:
        fl = ttfont["GPOS"].table.FeatureList
        res["gpos_features"] = sorted({fr.FeatureTag for fr in fl.FeatureRecord}) if fl else []
    if "GSUB" in ttfont:
        t = ttfont["GSUB"].table
        lookups = t.LookupList.Lookup if t.LookupList else []
        feats: dict[str, set[int]] = {}
        for fr in (t.FeatureList.FeatureRecord if t.FeatureList else []):
            feats.setdefault(fr.FeatureTag, set()).update(fr.Feature.LookupListIndex)
        res["gsub_features"] = sorted(feats)
        ligs = set()
        for tag, idxs in sorted(feats.items()):
            if tag not in VARIANT_FEATURES and tag not in LIGATURE_FEATURES:
                continue
            todo, seen = list(idxs), set()
            while todo:
                li = todo.pop()
                if li in seen or li >= len(lookups):
                    continue
                seen.add(li)
                for ltype, st in _subtables(lookups[li]):
                    if ltype == 1 and tag in VARIANT_FEATURES:
                        for a, b in st.mapping.items():
                            add_variant(a, b, tag)
                    elif ltype == 3 and tag in VARIANT_FEATURES:
                        for a, alts in st.alternates.items():
                            for b in alts:
                                add_variant(a, b, tag)
                    elif ltype == 4 and tag in LIGATURE_FEATURES:
                        for first, lst in st.ligatures.items():
                            for lig in lst:
                                comps = [first] + list(lig.Component)
                                chars = "".join(glyph_to_char.get(c, "?") for c in comps)
                                ligs.add((chars, lig.LigGlyph, tag))
                    elif ltype in (5, 6):
                        nested: set[int] = set()
                        _nested_lookups(st, nested)
                        todo.extend(nested)
                    elif ltype == 8 and tag in VARIANT_FEATURES:
                        for a, b in zip(st.Coverage.glyphs, st.Substitute):
                            add_variant(a, b, tag)
        res["ligatures"] = [{"chars": c, "glyph": g, "feature": f} for c, g, f in sorted(ligs)]

    names = set(ttfont.getGlyphOrder())
    for name in ttfont.getGlyphOrder():
        m = re.match(r"^(.+?)\.(.+)$", name)
        if not m or name.startswith("."):
            continue
        base = m.group(1)
        if base in glyph_to_char and name in names:
            add_variant(base, name, "суффикс ." + m.group(2))
    res["variants"] = {ch: {g: sorted(s) for g, s in gl.items()} for ch, gl in sorted(variants.items())}
    return res
