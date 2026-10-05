"""Эталоны для переноса на Rust: gcode текущей Python-версии и данные, из которых он собран.

Каждый случай — папка tests/golden/<имя>/:
  settings.json  — настройки на входе;
  input/         — файлы чертежей на входе (если есть);
  files.json     — для каждого gcode-файла: штрихи, настройки прохода, заголовок — то, что получает generate_gcode;
  *.gcode        — результат.

Запуск: python tools/make_golden.py
"""
from __future__ import annotations

import io
import json
import os
import shutil
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "tests" / "golden"
sys.path.insert(0, str(ROOT))
os.environ["HANDWRITER_HOME"] = tempfile.mkdtemp(prefix="hw-golden-")

import ezdxf  # noqa: E402

from handwriter import gcode as gcode_mod  # noqa: E402
from handwriter.drawing.pipeline import compose_drawing, make_all_files  # noqa: E402
from handwriter.drawing.sources import clear_cache  # noqa: E402
from handwriter.paths import user_drawings_dir, user_fonts_dir  # noqa: E402
from handwriter.pipeline import compose, make_gcode, make_test_gcode  # noqa: E402
from handwriter.settings import Settings, Travel, default_profiles  # noqa: E402

TEST_FONTS = ROOT / "tests" / "fonts"

SVG_SIMPLE = """<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="60mm" viewBox="0 0 100 60">
  <g fill="none" stroke="black" stroke-width="0.25">
    <rect x="0" y="0" width="100" height="60"/>
    <line x1="10" y1="50" x2="90" y2="10"/>
    <circle cx="30" cy="30" r="12"/>
    <path d="M60 40 A15 15 0 0 1 90 40"/>
    <polyline points="10,10 20,20 30,10"/>
  </g>
</svg>"""


def dxf_simple() -> bytes:
    doc = ezdxf.new("R2010", units=4)
    msp = doc.modelspace()

    def P(x, y):
        return (x, 60 - y)
    for a, b in [((0, 0), (100, 0)), ((100, 0), (100, 60)), ((100, 60), (0, 60)), ((0, 60), (0, 0))]:
        msp.add_line(P(*a), P(*b))
    msp.add_line(P(10, 50), P(90, 10))
    msp.add_circle(P(30, 30), 12)
    msp.add_arc(P(75, 40), 15, 0, 180)
    msp.add_lwpolyline([P(10, 10), P(20, 20), P(30, 10)])
    buf = io.StringIO()
    doc.write(buf)
    return buf.getvalue().encode("utf-8")


class Recorder:
    """Перехватывает вызовы generate_gcode и запоминает входные данные."""

    def __init__(self):
        self.calls: list[dict] = []
        self._orig = gcode_mod.generate_gcode

    def __enter__(self):
        def spy(strokes, s, header=None, info=None):
            out = self._orig(strokes, s, header, info)
            settings = s.model_dump(mode="json")
            settings["printer"]["_use_work_area"] = s.printer._use_work_area
            st = gcode_mod.compute_stats(strokes, s)
            self.calls.append({"strokes": strokes, "settings": settings, "header": header, "info": info,
                               "stats": {"draw_mm": st.draw_mm, "travel_mm": st.travel_mm, "time_s": st.time_s},
                               "gcode": out})
            return out
        for mod in self._modules():
            mod.generate_gcode = spy
        return self

    def __exit__(self, *exc):
        for mod in self._modules():
            mod.generate_gcode = self._orig

    @staticmethod
    def _modules():
        import handwriter.drawing.pipeline as dp
        import handwriter.pipeline as tp
        return (gcode_mod, dp, tp)


def save_case(name: str, s: Settings, run, inputs: dict[str, bytes] | None = None) -> None:
    d = OUT / name
    # готовые входные файлы не пересоздаются: в DXF попадают дата и случайные GUID
    inputs = {f: ((d / "input" / f).read_bytes() if (d / "input" / f).exists() else data)
              for f, data in (inputs or {}).items()}
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    (d / "settings.json").write_text(json.dumps(s.model_dump(mode="json"), ensure_ascii=False, indent=1),
                                     encoding="utf-8")
    for fname, data in inputs.items():
        (d / "input").mkdir(exist_ok=True)
        (d / "input" / fname).write_bytes(data)
        (user_drawings_dir() / fname).write_bytes(data)
    clear_cache()
    with Recorder() as rec:
        names = run(s)
    files = []
    for fname, call in zip(names, rec.calls, strict=True):
        (d / fname).write_text(call.pop("gcode"), encoding="utf-8", newline="\n")
        files.append({"filename": fname, **call})
    (d / "files.json").write_text(json.dumps(files, ensure_ascii=False), encoding="utf-8")
    print(f"{name}: {len(files)} файл(ов)")


def text_run(s: Settings) -> list[str]:
    make_gcode(compose(s))
    return ["text.gcode"]


def text_test_run(s: Settings) -> list[str]:
    make_test_gcode(s)
    return ["test.gcode"]


def drawing_run(tests: bool):
    def run(s: Settings) -> list[str]:
        c = compose_drawing(s)
        assert not c.errors, c.errors
        return [f["filename"] for f in make_all_files(c, tests)]
    return run


def text_settings(**kw) -> Settings:
    s = Settings()
    s.printer.travel = Travel(x_min=-2, x_max=200, y_min=-2, y_max=230)
    for k, v in kw.items():
        setattr(s, k, v)
    return s


def drawing_settings(file: str = "builtin:test") -> Settings:
    s = Settings()
    s.printer.travel = Travel(x_min=-2, x_max=300, y_min=-2, y_max=300)
    s.drawing.file = file
    return s


def numeric_case() -> None:
    """Случайные числа и результаты math.dist, sum и repr — для проверки numeric.rs."""
    import math
    import random
    rng = random.Random(20261005)
    pts = []
    for _ in range(2000):
        k = 10 ** rng.uniform(-6, 4)
        pts.append([[rng.uniform(-k, k), rng.uniform(-k, k)], [rng.uniform(-k, k), rng.uniform(-k, k)]])
    floats = [rng.uniform(-1, 1) * 10 ** rng.randint(-8, 8) for _ in range(500)]
    sums = [floats[i:i + n] for i, n in zip(range(0, 450, 9), range(1, 51))]
    data = {
        "dist": [[p, q, math.dist(p, q)] for p, q in pts],
        "sum": [[xs, sum(xs)] for xs in sums],
        "repr": [[x, repr(x)] for x in floats],
        "fmt2": [[x, f"{x:.2f}"] for x in floats] + [[x / 1000, f"{x / 1000:.2f}"] for x in range(-3000, 3000)],
    }
    (OUT / "numeric.json").write_text(json.dumps(data), encoding="utf-8")
    print("numeric.json")


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    numeric_case()
    shutil.copy(TEST_FONTS / "BadScript-Regular.ttf", user_fonts_dir())

    save_case("text_default", text_settings(), text_run)
    s = text_settings()
    s.randomness.enabled = False
    s.connections.enabled = False
    save_case("text_plain", s, text_run)
    s = text_settings()
    prof = default_profiles()["Тетрадь в линейку"]
    s.sheet, s.typography.size_mm, s.typography.baseline_shift = prof.sheet, prof.size_mm, prof.baseline_shift
    s.typography.rotation_deg, s.typography.dx, s.typography.dy = 1.5, 0.7, -0.4
    s.printer.flip_y = True
    s.printer.travel = Travel(x_min=-2, x_max=200, y_min=-230, y_max=2)
    save_case("text_lines_rotated_flip", s, text_run)
    s = text_settings(font="BadScript-Regular.ttf", mode="outlines")
    save_case("text_outline_badscript", s, text_run)
    save_case("text_test_pattern", text_settings(), text_test_run)

    save_case("drawing_builtin_a4", drawing_settings(), drawing_run(False))
    s = drawing_settings()
    s.drawing.frame.enabled = True
    s.drawing.weights.enabled = True
    s.drawing.placement.scale_mode = "fit_reach"
    save_case("drawing_builtin_frame_weights", s, drawing_run(True))
    s = Settings()
    s.drawing.marked.enabled = True
    save_case("drawing_marked_a4", s, drawing_run(True))
    s = Settings()
    s.drawing.a3.enabled = True
    s.drawing.frame.enabled = True
    save_case("drawing_a3_four_runs", s, drawing_run(True))
    s = Settings()
    s.drawing.sheet.orientation = "portrait"
    s.drawing.split.areas = 3
    s.printer.travel = Travel(x_min=-3, x_max=200, y_min=-3, y_max=215)
    save_case("drawing_split_areas", s, drawing_run(True))
    save_case("drawing_svg", drawing_settings("simple.svg"), drawing_run(False),
              {"simple.svg": SVG_SIMPLE.encode("utf-8")})
    save_case("drawing_dxf", drawing_settings("simple.dxf"), drawing_run(False), {"simple.dxf": dxf_simple()})


if __name__ == "__main__":
    main()
