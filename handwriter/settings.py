from __future__ import annotations

import json
import os
import threading
import time
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, PrivateAttr

from .i18n import tr
from .paths import DEFAULT_FONT, settings_path


class _M(BaseModel):
    model_config = ConfigDict(extra="ignore")


class Sheet(_M):
    width: float = 165.0
    height: float = 205.0
    margin_left: float = 20.0
    margin_right: float = 8.0
    first_line_top: float = 15.0
    bottom_limit: float = 10.0
    line_pitch: float = 10.0
    indent: float = 10.0
    ruling: Literal["grid", "lines", "none"] = "grid"
    grid_step: float = 5.0


class Typography(_M):
    size_mm: float = 3.0
    baseline_shift: float = 0.0
    dx: float = 0.0
    dy: float = 0.0
    rotation_deg: float = 0.0
    hyphenate: bool = True


class MissingChoice(_M):
    action: Literal["skip", "replace"] = "skip"
    replacement: str = ""


class TextOptions(_M):
    replacements: list[tuple[str, str]] = Field(default_factory=lambda: [
        ("«", '"'), ("»", '"'), ("„", '"'), ("“", '"'), ("”", '"'),
        ("—", "-"), ("–", "-"), ("…", "..."), (" ", " "),
    ])
    missing: dict[str, MissingChoice] = Field(default_factory=dict)
    start_word: int = 1
    resume_word: int = Field(0, ge=0)
    resume_letter: int = Field(1, ge=1)


class Randomness(_M):
    enabled: bool = True
    seed: int = 1
    size: float = Field(3.0, ge=0, le=30)
    slant: float = Field(2.0, ge=0, le=20)
    offset: float = Field(0.3, ge=0, le=3)
    letter_spacing: float = Field(5.0, ge=0, le=50)
    word_spacing: float = Field(10.0, ge=0, le=100)
    drift: float = Field(0.5, ge=0, le=5)
    line_start: float = Field(1.0, ge=0, le=10)
    right_edge: float = Field(1.5, ge=0, le=20)
    jitter: float = Field(0.1, ge=0, le=1)
    variants: bool = True


class Connections(_M):
    enabled: bool = True
    distance: float = Field(0.15, ge=0, le=1)


class Travel(_M):
    x_min: float
    x_max: float
    y_min: float
    y_max: float


class TableSetup(_M):
    overhang_x: bool = True
    overhang_y: bool = False
    table_x: float | None = None
    table_y: float | None = None
    touch_s: float = Field(1.0, ge=0, le=30)
    pause_s: float = Field(2.0, ge=0, le=60)
    readings: dict[str, float] = Field(default_factory=dict)


class Printer(_M):
    pen_up_z: float = 4.0
    pen_down_z: float = -1.0
    feed_draw: float = 1200.0
    feed_travel: float = 3000.0
    feed_z: float = 600.0
    simplify_tol: float = 0.05
    travel: Travel | None = None
    work_w: float = Field(220.0, gt=0, le=2000)
    work_h: float = Field(220.0, gt=0, le=2000)
    _use_work_area: bool = PrivateAttr(False)
    safety_margin: float = 2.0
    flip_x: bool = False
    flip_y: bool = False
    test_mark_offset: float = 5.0
    table: TableSetup = Field(default_factory=TableSetup)


class OutlineOptions(_M):
    px_per_em: float = Field(1500.0, ge=200, le=4000)
    prune: float = Field(0.08, ge=0, le=1)
    extend: float = Field(1.0, ge=0, le=3)
    smooth: float = Field(0.03, ge=0, le=0.5)
    simplify: float = Field(0.004, ge=0, le=0.1)
    junction_merge: float = Field(2.5, ge=0, le=10)


class Preview(_M):
    show_travel: bool = True
    show_ruling: bool = True


class DrawingImport(_M):
    units: Literal["auto", "mm", "cm", "m", "in", "ft", "pt", "px"] = "auto"
    pdf_page: int = Field(1, ge=1)
    threshold_auto: bool = True
    threshold: int = Field(128, ge=1, le=254)
    invert: bool = False
    raster_dpi: float = Field(0.0, ge=0, le=4800)
    fill_centerlines: bool = False
    fill_centerline_max: float = Field(5.0, gt=0, le=100)
    raster_mode: Literal["centerlines", "fill"] = "centerlines"
    fill_step: float = Field(0.4, ge=0.05, le=5)
    fill_dir: Literal["auto", "horizontal", "vertical"] = "auto"


class DrawingSheet(_M):
    format: Literal["A4", "A3", "custom"] = "A4"
    width: float = Field(210.0, gt=0, le=2000)
    height: float = Field(297.0, gt=0, le=2000)
    orientation: Literal["auto", "portrait", "landscape"] = "auto"


class DrawingPlacement(_M):
    scale_mode: Literal["fit", "fit_reach", "fit_passes", "one_to_one", "percent"] = "fit"
    percent: float = Field(100.0, gt=0, le=10000)
    margin: float = Field(10.0, ge=0, le=200)
    dx: float = 0.0
    dy: float = 0.0
    anchor: Literal["center", "zero"] = "center"


class GostFrame(_M):
    enabled: bool = False
    left: float = Field(20.0, ge=0, le=100)
    right: float = Field(5.0, ge=0, le=100)
    top: float = Field(5.0, ge=0, le=100)
    bottom: float = Field(5.0, ge=0, le=100)
    title_block: bool = True
    tb_width: float = Field(185.0, gt=0, le=400)
    tb_height: float = Field(55.0, gt=0, le=200)


class LineWeights(_M):
    enabled: bool = False
    threshold: float = Field(0.4, ge=0, le=10)
    passes: int = Field(3, ge=1, le=9)
    step: float = Field(0.15, ge=0.01, le=2)
    layers: dict[str, Literal["auto", "thin", "thick"]] = Field(default_factory=dict)


class DrawingPaths(_M):
    curve_tol: float = Field(0.05, ge=0.005, le=1)
    join_tol: float = Field(0.05, ge=0, le=2)
    long_path: float = Field(30.0, ge=0, le=10000)


class DrawingSplit(_M):
    overlap: float = Field(0.5, ge=0, le=5)
    slack: float = Field(1.0, ge=0, le=10)
    marks: bool = False
    mark_size: float = Field(3.0, gt=0.5, le=20)
    mark_count: int = Field(3, ge=1, le=10)
    offsets: dict[str, tuple[float, float]] = Field(default_factory=dict)
    areas: int = Field(0, ge=0, le=4)


class DrawingSettings(_M):
    file: str = "builtin:test"
    imp: DrawingImport = Field(default_factory=DrawingImport)
    sheet: DrawingSheet = Field(default_factory=DrawingSheet)
    placement: DrawingPlacement = Field(default_factory=DrawingPlacement)
    frame: GostFrame = Field(default_factory=GostFrame)
    weights: LineWeights = Field(default_factory=LineWeights)
    paths: DrawingPaths = Field(default_factory=DrawingPaths)
    split: DrawingSplit = Field(default_factory=DrawingSplit)
    show_travel: bool = True


class Profile(_M):
    sheet: Sheet
    size_mm: float = 3.0
    baseline_shift: float = 0.0


def default_profiles() -> dict[str, Profile]:
    return {
        tr("Тетрадь в клетку"): Profile(
            sheet=Sheet(width=165, height=205, margin_left=20, margin_right=8, first_line_top=15,
                        bottom_limit=10, line_pitch=10, indent=10, ruling="grid", grid_step=5),
            size_mm=3.0, baseline_shift=0.0),
        tr("Тетрадь в линейку"): Profile(
            sheet=Sheet(width=165, height=205, margin_left=20, margin_right=8, first_line_top=16,
                        bottom_limit=10, line_pitch=8, indent=10, ruling="lines", grid_step=8),
            size_mm=2.5, baseline_shift=0.3),
        "A4": Profile(
            sheet=Sheet(width=210, height=297, margin_left=25, margin_right=15, first_line_top=25,
                        bottom_limit=20, line_pitch=9, indent=12.5, ruling="none", grid_step=5),
            size_mm=3.0, baseline_shift=0.0),
    }


SAMPLE_TEXT = (
    "\tСъешь же ещё этих мягких французских булок, да выпей чаю. "
    "Широкая электрификация южных губерний даст мощный толчок подъёму сельского хозяйства.\n"
    "\n"
    "\tВторой абзац после пропущенной строки."
)


class Settings(_M):
    version: int = 1
    font: str = DEFAULT_FONT
    mode: Literal["strokes", "outlines"] = "strokes"
    text: str = Field(default_factory=lambda: tr(SAMPLE_TEXT))
    active_profile: str = Field(default_factory=lambda: tr("Тетрадь в клетку"))
    sheet: Sheet = Field(default_factory=Sheet)
    typography: Typography = Field(default_factory=Typography)
    text_options: TextOptions = Field(default_factory=TextOptions)
    printer: Printer = Field(default_factory=Printer)
    outline: OutlineOptions = Field(default_factory=OutlineOptions)
    randomness: Randomness = Field(default_factory=Randomness)
    connections: Connections = Field(default_factory=Connections)
    preview: Preview = Field(default_factory=Preview)
    profiles: dict[str, Profile] = Field(default_factory=default_profiles)
    drawing: DrawingSettings = Field(default_factory=DrawingSettings)


_io_lock = threading.Lock()


def _retry(fn, attempts: int = 20):
    for i in range(attempts):
        try:
            return fn()
        except PermissionError:
            if i == attempts - 1:
                raise
            time.sleep(0.02)


def load_settings(path: Path | None = None) -> Settings:
    path = path or settings_path()
    with _io_lock:
        if not path.exists():
            return Settings()
        raw = _retry(lambda: path.read_text(encoding="utf-8"))
        try:
            return Settings.model_validate(json.loads(raw))
        except ValueError:
            _retry(lambda: path.replace(path.with_suffix(".broken.json")))
            return Settings()


def save_settings(s: Settings, path: Path | None = None) -> None:
    path = path or settings_path()
    data = json.dumps(s.model_dump(mode="json"), ensure_ascii=False, indent=2)
    with _io_lock:
        tmp = path.with_suffix(".tmp")
        tmp.write_text(data, encoding="utf-8")
        _retry(lambda: os.replace(tmp, path))
