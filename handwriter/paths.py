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


def _base(env_var: str, fallback: Path) -> Path:
    v = os.environ.get(env_var)
    return Path(v) if v else fallback


def user_dir() -> Path:
    env = os.environ.get("HANDWRITER_HOME")
    d = Path(env) if env else _base("APPDATA", Path.home() / "AppData" / "Roaming") / APP_NAME
    d.mkdir(parents=True, exist_ok=True)
    return d


def local_dir() -> Path:
    env = os.environ.get("HANDWRITER_HOME")
    d = Path(env) / "local" if env else _base("LOCALAPPDATA", Path.home() / "AppData" / "Local") / APP_NAME
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
