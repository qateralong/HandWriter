from __future__ import annotations

import os
import sys
from pathlib import Path

PACKAGE_DIR = Path(__file__).resolve().parent
STATIC_DIR = PACKAGE_DIR / "static"
BUILTIN_FONTS_DIR = PACKAGE_DIR / "fonts"
BUILTIN_PREFIX = "builtin:"
DEFAULT_FONT = BUILTIN_PREFIX + "hershey_cyrillic.svg"
APP_NAME = "HandWriter"
FROZEN = bool(getattr(sys, "frozen", False))


def _app_dir(win_var: str, win_fallback: str, xdg_var: str, xdg_fallback: str) -> Path:
    """%APPDATA%-style dir on Windows (or when the variable is set), XDG base dir elsewhere."""
    v = os.environ.get(win_var)
    if v:
        return Path(v) / APP_NAME
    if os.name == "nt":
        return Path.home() / "AppData" / win_fallback / APP_NAME
    x = os.environ.get(xdg_var)
    return (Path(x) if x else Path.home() / xdg_fallback) / APP_NAME


def user_dir() -> Path:
    env = os.environ.get("HANDWRITER_HOME")
    d = Path(env) if env else _app_dir("APPDATA", "Roaming", "XDG_DATA_HOME", ".local/share")
    d.mkdir(parents=True, exist_ok=True)
    return d


def local_dir() -> Path:
    env = os.environ.get("HANDWRITER_HOME")
    d = Path(env) / "local" if env else _app_dir("LOCALAPPDATA", "Local", "XDG_CACHE_HOME", ".cache")
    d.mkdir(parents=True, exist_ok=True)
    return d


def settings_path() -> Path:
    return user_dir() / "settings.json"


def logs_dir() -> Path:
    d = user_dir() / "logs"
    d.mkdir(parents=True, exist_ok=True)
    return d


def log_path() -> Path:
    return logs_dir() / "handwriter.log"


def instance_path() -> Path:
    return user_dir() / "instance.json"


def user_fonts_dir() -> Path:
    d = user_dir() / "fonts"
    d.mkdir(parents=True, exist_ok=True)
    return d


def user_drawings_dir() -> Path:
    d = user_dir() / "drawings"
    d.mkdir(parents=True, exist_ok=True)
    return d


def resolve_font_path(spec: str) -> Path:
    if spec.startswith(BUILTIN_PREFIX):
        return BUILTIN_FONTS_DIR / spec[len(BUILTIN_PREFIX):]
    p = Path(spec).expanduser()
    if not p.is_absolute():
        p = user_fonts_dir() / p
    return p
