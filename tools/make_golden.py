from __future__ import annotations

import io
import json
import os
import shutil
import sys
import tempfile
from pathlib import Path

import ezdxf

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from handwriter import gcode as gcode_mod
from handwriter.drawing.pipeline import compose_drawing, make_all_files
from handwriter.drawing.sources import clear_cache
from handwriter.paths import user_drawings_dir, user_fonts_dir
from handwriter.pipeline import compose, make_gcode, make_test_gcode
from handwriter.settings import Settings, Travel, default_profiles

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "tests" / "golden"

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
    print(f"{name}: {len(files)} files")


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


def constraint_cases() -> list:
    import annotated_types as at
    from pydantic import BaseModel

    out = []

    def walk(model, path):
        for name, f in model.model_fields.items():
            ann = f.annotation
            if isinstance(ann, type) and issubclass(ann, BaseModel):
                walk(ann, path + [name])
                continue
            lo = hi = None
            for m in f.metadata:
                if isinstance(m, at.Ge):
                    lo = ("ge", m.ge)
                elif isinstance(m, at.Gt):
                    lo = ("gt", m.gt)
                elif isinstance(m, at.Le):
                    hi = m.le
            if lo is None and hi is None:
                continue
            step = 1 if ann is int else 1e-9
            values = []
            if lo is not None:
                values += [lo[1] - step, lo[1], lo[1] + step]
            if hi is not None:
                values += [hi - step, hi, hi + step]
            for v in values:
                data: dict = {}
                node = data
                for key in path:
                    node = node.setdefault(key, {})
                node[name] = v
                try:
                    Settings.model_validate(json.loads(json.dumps(data)))
                    ok = True
                except ValueError:
                    ok = False
                out.append([".".join(path + [name]), data, ok])

    walk(Settings, [])
    return out


def check_cases() -> list:
    import random

    from handwriter.checks import check_bounds, check_settings

    rng = random.Random(7)
    variants = []
    base = Settings()
    variants.append(base)
    for mutate in (
        lambda s: setattr(s.sheet, "width", 0),
        lambda s: setattr(s.sheet, "margin_left", -1),
        lambda s: setattr(s.sheet, "margin_right", 150),
        lambda s: setattr(s.sheet, "indent", 140),
        lambda s: setattr(s.sheet, "first_line_top", 196),
        lambda s: setattr(s.sheet, "line_pitch", 0),
        lambda s: setattr(s.typography, "size_mm", 0),
        lambda s: setattr(s.typography, "size_mm", 4.5),
        lambda s: (setattr(s.printer, "pen_up_z", -1.5), setattr(s.printer, "pen_down_z", -3.25)),
        lambda s: setattr(s.printer, "pen_up_z", 1.5),
        lambda s: setattr(s.printer, "pen_down_z", 0.5),
        lambda s: (setattr(s.printer, "feed_draw", 0), setattr(s.printer, "feed_z", -5)),
        lambda s: setattr(s.printer, "simplify_tol", -0.1),
        lambda s: setattr(s.printer, "travel", Travel(x_min=5, x_max=1, y_min=0, y_max=10)),
        lambda s: setattr(s.printer, "travel", Travel(x_min=5, x_max=100, y_min=0, y_max=10)),
        lambda s: setattr(s.printer, "travel", Travel(x_min=-2, x_max=200, y_min=-2, y_max=230)),
        lambda s: setattr(s.printer, "safety_margin", -1),
        lambda s: (setattr(s.printer, "flip_x", True), setattr(s.printer, "safety_margin", 0.0)),
        lambda s: (setattr(s.printer, "flip_y", True), s.printer.__setattr__("_use_work_area", True)),
    ):
        s = Settings()
        mutate(s)
        flag = s.printer._use_work_area
        s = Settings.model_validate(s.model_dump())
        s.printer._use_work_area = flag
        variants.append(s)
    out = []
    for s in variants:
        strokes = [[(rng.uniform(-20, 240), rng.uniform(-20, 240)) for _ in range(rng.randint(1, 6))]
                   for _ in range(rng.randint(0, 5))]
        settings = s.model_dump(mode="json")
        settings["printer"]["_use_work_area"] = s.printer._use_work_area
        errors, warnings = check_settings(s)
        out.append({"settings": settings, "strokes": strokes, "errors": errors, "warnings": warnings,
                    "bounds": check_bounds(strokes, s), "bounds2": check_bounds(strokes, s, limit=2)})
    return out


def geometry_cases() -> dict:
    import random

    from handwriter.geometry import make_transform, polyline_length, rdp, rdp_indices

    rng = random.Random(11)
    transforms = []
    for _ in range(300):
        rot, dx, dy = rng.uniform(-360, 360), rng.uniform(-50, 50), rng.uniform(-50, 50)
        p = (rng.uniform(-300, 300), rng.uniform(-300, 300))
        transforms.append([rot, dx, dy, p, make_transform(rot, dx, dy)(p)])
    for rot in (0, 90, 180, 270, -90, 45, 1.5):
        p = (12.5, -7.25)
        transforms.append([rot, 0.0, 0.0, p, make_transform(rot, 0.0, 0.0)(p)])
    polylines = []
    for _ in range(200):
        n = rng.randint(0, 40)
        x = y = 0.0
        pts = []
        for _ in range(n):
            x += rng.uniform(-3, 3)
            y += rng.uniform(-3, 3)
            pts.append((x, y))
        if n > 3 and rng.random() < 0.2:
            pts[2] = pts[0]
        tol = rng.choice([0.0, 0.01, 0.05, 0.3, 1.0, 5.0])
        keep = sorted(rng.sample(range(n), min(n, 2))) if rng.random() < 0.3 else []
        polylines.append({"points": pts, "tol": tol, "must_keep": keep, "rdp": rdp(pts, tol),
                          "indices": rdp_indices(pts, tol, keep), "length": polyline_length(pts)})
    return {"transforms": transforms, "polylines": polylines}


def conformance_case() -> None:
    data = {"constraints": constraint_cases(), "checks": check_cases(), "geometry": geometry_cases()}
    (OUT / "conformance.json").write_text(json.dumps(data, ensure_ascii=False), encoding="utf-8")
    print(f"conformance.json: {len(data['constraints'])} constraint cases, {len(data['checks'])} check cases")


def main() -> None:
    os.environ["HANDWRITER_HOME"] = tempfile.mkdtemp(prefix="hw-golden-")
    OUT.mkdir(parents=True, exist_ok=True)
    numeric_case()
    conformance_case()
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
