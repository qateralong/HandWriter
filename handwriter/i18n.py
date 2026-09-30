from __future__ import annotations

import json
import os
import re
from functools import lru_cache

from .paths import PACKAGE_DIR

LANGS = ("ru", "en")
LOCALE_DIR = PACKAGE_DIR / "locale"
LANG_FILE = PACKAGE_DIR / "lang.txt"
_CYR = re.compile(r"[Ѐ-ӿ]")
_lang: str | None = None

TR_KEYS = frozenset({"errors", "warnings", "detail", "notes", "message", "describe", "rotation_text", "label",
                     "units_note", "corner_name", "x_height_source"})


def lang() -> str:
    global _lang
    if _lang is None:
        v = os.environ.get("HANDWRITER_LANG", "").strip().lower()
        if not v and LANG_FILE.exists():
            v = LANG_FILE.read_text(encoding="utf-8").strip().lower()
        _lang = v if v in LANGS else "ru"
    return _lang


def set_lang(value: str) -> None:
    global _lang
    _lang = value if value in LANGS else "ru"


@lru_cache(maxsize=4)
def _table(code: str):
    path = LOCALE_DIR / f"{code}.json"
    if not path.exists():
        return None
    words = {k: v for k, v in json.loads(path.read_text(encoding="utf-8")).items() if v}
    keys = sorted(words, key=lambda k: (-len(k), k))
    return [(k, re.compile(_alternative(k)), words[k]) for k in keys]


def _alternative(key: str) -> str:
    alt = re.escape(key)
    if _CYR.match(key[0]):
        alt = r"(?<![Ѐ-ӿ])" + alt
    if _CYR.match(key[-1]):
        alt += r"(?![Ѐ-ӿ])"
    return alt


def tr(text: str) -> str:
    code = lang()
    if code == "ru" or not text or not _CYR.search(text):
        return text
    table = _table(code)
    if table is None:
        return text
    for key, rx, value in table:
        if key in text:
            text = rx.sub(lambda _m, v=value: v, text)
            if not _CYR.search(text):
                break
    return text


def tr_static(text: str) -> str:
    if lang() == "ru":
        return text
    return tr(text).replace('<html lang="ru">', f'<html lang="{lang()}">')


def tr_json(obj, key: str | None = None):
    if isinstance(obj, dict):
        return {k: tr_json(v, k) for k, v in obj.items()}
    if isinstance(obj, list):
        return [tr_json(v, key) for v in obj]
    if isinstance(obj, str) and key in TR_KEYS:
        return tr(obj)
    return obj
