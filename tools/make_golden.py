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
from handwriter.settings import MissingChoice, Settings, Travel, default_profiles

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


FONT_SVG = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"{attrs}>{body}</svg>'

FOLDER_FONTS = {
    "hand": {
        "а.svg": ('<path d="M10 80 L10 30"/>', ""),
        "а.2.svg": ('<path d="M20 80 L20 30"/>', ""),
        "а.3.svg": ('<polyline points="30,80 30,30"/>', ""),
        "uni0410.svg": ('<path d="M0 80 L40 10 L80 80"/>', ' data-advance="85"'),
        "period.svg": ('<circle cx="5" cy="78" r="2"/>', ""),
        "font.json": '{"x_height": 50}',
    },
    "mixed": {
        "т.svg": ('<g transform="translate(10 0)"><path d="M0 30 L60 30 M30 30 L30 80"/></g>', ""),
        "с.svg": ('<path d="M70 40 C50 20 10 30 10 60 C10 90 60 90 70 70"/>', ""),
        "x.alt.svg": ('<path d="M0 0 L50 50 M50 0 L0 50" style="display: none"/><line x1="1" y1="2" x2="3" y2="4"/>', ""),
        "x.svg": ('<ellipse cx="50" cy="50" rx="20" ry="10" transform="rotate(30 50 50) skewX(10)"/>', ""),
        "U+0444.svg": ('<rect x="10" y="20" width="30" height="40"/><polygon points="1,1 5,1 5,5"/>', ' horiz-adv-x="70"'),
        "uni1F600.svg": ('<path d="M10 10 Q50 0 90 10 T90 90 S10 90 10 50 A40 30 15 1 0 60 60 z m5 5 h10 v10 Z l3 3"/>', ""),
        "space.svg": ("", ' width="30" height="100"'),
        "z.svg": ('<path d="M0 0 L1 1"/>', None),
        "?.svg": ('<path d="M0 0 L1 1"/>', ""),
        "font.json": '{"name": "Mixed", "units_per_em": "200", "baseline": 90, "cap_height": 140, "descent": 30}',
    },
}

SVG_FONTS = {
    "mini.svg": """<svg xmlns="http://www.w3.org/2000/svg"><defs>
      <font id="Mini" horiz-adv-x="500">
        <font-face font-family="Mini" units-per-em="1000" x-height="400"/>
        <glyph unicode="б" glyph-name="be" d="M0 0 L0 700"/>
        <glyph glyph-name="be.2" d="M10 0 L10 700"/>
        <glyph unicode="б" glyph-name="be.3" d="M20 0 L20 700"/>
        <glyph unicode="ff" glyph-name="f_f" d="M0 0 L1 1"/>
        <glyph unicode=" " horiz-adv-x="300"/>
      </font></defs></svg>""",
    "odd.svg": """<svg xmlns="http://www.w3.org/2000/svg"><font horiz-adv-x="0">
        <font-face units-per-em="0" ascent="800" descent="200"/>
        <glyph unicode="е" d="M100 100 C 200 300 400 -50 500 200" transform="translate(10) scale(1 -1)"/>
        <glyph unicode="е" d="M0 0 L5 5"/>
        <glyph unicode="е" glyph-name="" d="M0 0 L6 6"/>
        <glyph unicode="Н" horiz-adv-x="650"><g transform="matrix(1 0 0 1 5 5)"><circle cx="300" cy="300" r="250"/></g></glyph>
        <glyph glyph-name="uni0435.alt" d="M1 1 L2 2"/>
        <glyph glyph-name="orphan" d="M1 1 L2 2"/>
        <glyph unicode="" d="M1 1 L2 2"/>
        <glyph unicode="e&#769;" d="M1 1 L2 2"/>
        <glyph unicode="й" d="M10 10 L20 20 Z Z"/>
      </font></svg>""",
    "notfont.svg": '<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0 L1 1"/></svg>',
}

PATHS = [
    "M10 80 L10 30", "M0,0 L10,0 L10,10 Z", "m10 10 l5 5 l-5 5 z m20 0 l1 1",
    "M0 0 H10 V10 h-5 v-5 Z", "M0 0 10 10 20 0", "m5 5 10 10 20 0",
    "M70 40 C50 20 10 30 10 60 C10 90 60 90 70 70", "M0 0 c10 0 10 10 20 10 s10 -10 20 -10 S50 10 60 0",
    "M0 0 Q5 10 10 0 T20 0 t10 0 T40 5", "M0 0 S10 10 20 0", "M0 0 T10 10",
    "M0,0 A10,10 0 0,1 20,0", "M0 0 a10 10 0 1 0 20 0", "M0 0 A10 5 30 1 1 15 15",
    "M0 0 A0 5 0 0 1 10 0", "M5 5 A10 10 0 0 1 5 5", "M0 0 A1 1 0 0 1 50 0",
    "M60 40 A15 15 0 0 1 90 40", "M0 0 a1,1 0 00.5.5", "M0 0 A10 10 0 1110 10",
    "M-5.5e-1,.5L1e2-3.25", "M.5.5.5.5", "M0 0 L 10 10 Z L 5 5", "M0 0 Z Z", "M0 0 L1 1 Z M3 3 L4 4",
    "M1 1 L2 2 L1 1 Z", "M0 0 C1 1 2 2", "L1 1", "", "M0 0 A1 1 0 2 1 5 5", "M0 0 X 5 5",
    "M 10 10 q 5 -5 10 0 q 5 5 10 0 Z", "M0 0 c 5 0 5 5 0 5 z c 1 1 2 2 3 3",
    "M100 100 C 200 300 400 -50 500 200", "M0 0 L10 10 m5 5 L0 0 z",
]

TRANSFORMS = [
    "", "translate(10,5) scale(2) rotate(90)", "translate(10)", "scale(2 3)", "rotate(45 10 10)",
    "skewX(30)", "skewY(-15)", "matrix(1 2 3 4 5 6)", "matrix(1 2 3)", "rotate(-90) translate(1e1, -2.5e-1)",
    "translate( 1 , 2 )scale(.5)", "unknown(1) scale(-1,1)", "rotate(180)", "rotate(30.5 -2 7)",
]

SVG_DOCS = {
    "simple": SVG_SIMPLE,
    "nested": """<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="http://example.com/x">
      <g transform="translate(5 5)" display="none"><line x1="0" y1="0" x2="1" y2="1"/></g>
      <g transform="scale(2)" style="stroke: black; display : none ;"><line x1="0" y1="0" x2="1" y2="1"/></g>
      <g transform="rotate(10)"><x:path d="M0 0 L1 1"/><path d="M1 1 L 2 2"/>
        <defs><path d="M9 9 L8 8"/></defs><title>t</title>
        <polygon points="0,0 10,0 10,10 0,0"/><polyline points="1 2 3"/><polygon points=""/>
        <rect width="10" height="5"/><circle r="3"/><ellipse rx="4" ry="2" cx="1"/><text>x</text>
        <!-- comment --><g><g transform="translate(1 1) matrix(1 0 0 -1 0 0)"><path d="m0 0 h5"/></g></g>
      </g></svg>""",
}


def rand_case() -> dict:
    import random

    from handwriter.rand import pick, rnd, urnd, vnoise

    rng = random.Random(3)
    out = {"rnd": [], "urnd": [], "pick": [], "vnoise": []}
    channels = ["size", "slant", "voff", "ls", "ws", "lstart", "redge", "variant", "drift", "jx", "ч"]
    for _ in range(300):
        seed = rng.choice([0, 1, 2, 42, -7, 10**12, rng.randint(-10**6, 10**6)])
        ch = rng.choice(channels)
        keys = [rng.randint(-5, 500) for _ in range(rng.randint(0, 4))]
        out["rnd"].append([seed, ch, keys, rnd(seed, ch, *keys)])
        out["urnd"].append([seed, ch, keys, urnd(seed, ch, *keys)])
        n = rng.randint(1, 7)
        out["pick"].append([seed, ch, n, keys, pick(seed, ch, n, *keys)])
        t = rng.choice([rng.uniform(-50, 50), float(rng.randint(-5, 5)), -0.0, 1e-9])
        out["vnoise"].append([seed, ch, t, keys, vnoise(seed, ch, t, *keys)])
    return out


def py_error(e: Exception) -> list:
    return [type(e).__name__, str(e)]


def dump_font(p) -> dict:
    glyphs = {}
    for name in p._glyphs:
        g = p.glyph(name)
        glyphs[name] = {"strokes": [list(s) for s in g.strokes], "advance": g.advance}
    info = p.info()
    m = p.metrics
    return {
        "name": p.name, "glyph_count": info.glyph_count, "chars": info.chars, "variants": info.variants,
        "notes": info.notes, "cmap": [[ch, names] for ch, names in p._cmap.items()], "glyph_order": list(p._glyphs),
        "metrics": {"x_height": m.x_height, "cap_height": m.cap_height, "ascent": m.ascent, "descent": m.descent,
                    "x_height_source": m.x_height_source},
        "space_advance": p.space_advance(), "glyphs": glyphs,
    }


def svg_fonts_case() -> None:
    import random
    import xml.etree.ElementTree as ET

    from handwriter.glyphs import StrokeGlyphProvider
    from handwriter.glyphs.svgparse import parse_transform, parse_viewbox, path_d_to_strokes, walk_strokes
    from handwriter.paths import BUILTIN_FONTS_DIR

    d = OUT / "svg_fonts"
    shutil.rmtree(d, ignore_errors=True)
    inp = d / "input"
    inp.mkdir(parents=True)
    for folder, files in FOLDER_FONTS.items():
        (inp / folder).mkdir()
        for fname, spec in files.items():
            if isinstance(spec, str):
                text = spec
            else:
                body, attrs = spec
                text = FONT_SVG.format(attrs=attrs or "", body=body)
                if attrs is None:
                    text = text.replace(' viewBox="0 0 100 100"', "")
            (inp / folder / fname).write_text(text, encoding="utf-8")
    (inp / "empty").mkdir()
    for fname, text in SVG_FONTS.items():
        (inp / fname).write_text(text, encoding="utf-8")
    (inp / "font.txt").write_text("x", encoding="utf-8")

    fonts = []
    targets = [("builtin", BUILTIN_FONTS_DIR / "hershey_cyrillic.svg")]
    targets += [(f"input/{n}", inp / n) for n in [*FOLDER_FONTS, "empty", *SVG_FONTS, "font.txt"]]
    for label, path in targets:
        try:
            fonts.append({"font": label, "result": dump_font(StrokeGlyphProvider.from_path(path))})
        except Exception as e:
            kind, msg = py_error(e)
            fonts.append({"font": label, "error": [kind, msg.replace(str(inp), "<input>")]})

    rng = random.Random(5)
    paths = []
    matrices = [(1.0, 0.0, 0.0, 1.0, 0.0, 0.0), (0.001, 0.0, 0.0, -0.001, 0.0, 0.8), (2.0, 0.5, -0.3, 1.5, 7.0, -3.0)]
    for dd in PATHS:
        for m in matrices:
            for tol in (0.001, 0.5):
                try:
                    paths.append([dd, m, tol, path_d_to_strokes(dd, m, tol)])
                except Exception as e:
                    paths.append([dd, m, tol, py_error(e)])
    cmds = "MmLlHhVvCcSsQqTtAaZz"
    argc = {"M": 2, "L": 2, "H": 1, "V": 1, "C": 6, "S": 4, "Q": 4, "T": 2, "A": 7, "Z": 0}
    for _ in range(300):
        parts = [rng.choice(["M", "m"]) + f" {rng.uniform(-50, 50):.3f} {rng.uniform(-50, 50):.3f}"]
        for _ in range(rng.randint(1, 8)):
            c = rng.choice(cmds)
            n = argc[c.upper()]
            args = []
            for k in range(n * rng.choice([1, 1, 2])):
                if c.upper() == "A" and k % 7 in (3, 4):
                    args.append(str(rng.randint(0, 1)))
                elif c.upper() == "A" and k % 7 in (0, 1):
                    args.append(f"{rng.choice([0, rng.uniform(0.1, 60)]):g}")
                else:
                    args.append(f"{rng.uniform(-60, 60):.{rng.randint(0, 4)}f}")
            parts.append(c + rng.choice([" ", ""]) + rng.choice([" ", ","]).join(args))
        dd = rng.choice([" ", "", "\n"]).join(parts)
        m = rng.choice(matrices)
        try:
            paths.append([dd, m, 0.01, path_d_to_strokes(dd, m, 0.01)])
        except Exception as e:
            paths.append([dd, m, 0.01, py_error(e)])

    transforms = [[s, list(parse_transform(s))] for s in TRANSFORMS]
    docs = []
    for name, text in SVG_DOCS.items():
        root = ET.fromstring(text)
        docs.append({"name": name, "svg": text, "viewbox": parse_viewbox(root),
                     "strokes": walk_strokes(root, (1.0, 0.0, 0.0, -1.0, 0.0, 100.0), 0.01)})

    data = {"fonts": fonts, "paths": paths, "transforms": transforms, "documents": docs, "rand": rand_case()}
    (d / "cases.json").write_text(json.dumps(data, ensure_ascii=False), encoding="utf-8")
    print(f"svg_fonts: {len(fonts)} fonts, {len(paths)} paths")


HYPHEN_WORDS = [
    "электрификация", "губерний", "сельского", "хозяйства", "французских", "подъёму", "Съешь", "СЪЕШЬ",
    "по-русски", "кое-что", "ёлка", "йод", "ай", "пятисотпятидесятитысячный", "Hyphenation", "computer",
    "programming", "extraordinary", "mother-in-law", "a", "ab", "abcd", "rhythm", "naïve", "İstanbul",
    "достопримечательность", "противоестественный", "за́мок", "x²yz", "word_with_under", "mixedРусскийEnglish",
    "12345", "абв123где", "ΣΑΣ", "straße", "­софт", "три­ста",
]

TEXTS = {
    "sample": None,
    "long_words": "\tПятисотпятидесятитысячный достопримечательность противоестественный электрификация. "
                  "Кое-что по-русски, mother-in-law extraordinary programming.\n\n\n\tАбзац после двух пустых.",
    "soft_and_tabs": "  \t Три\u00adста\u00adшестьдесят пять дней\t\tв году.\r\nВторая строка\rтретья — «кавычки» … "
                     "\u00a0неразрывный.\n\t\n\t\tОтступ: x² ×y.",
    "missing": "Буквы ⌘ и ☃ и ⌘ и \U0001F600 — нет в шрифте; № и ° есть. За́мок й",
    "numbers": "1234567890 3,14 15:00 (скобки) [квадратные] {фигурные} @#$%^&*+=<>/\\|~`!?;'\"",
    "empty": "   \n\t\n",
    "one_long": "Пятисотпятидесятитысячныйпятисотпятидесятитысячныйпятисотпятидесятитысячный",
}


def text_settings_variants() -> list:
    out = []

    def add(name, text_key, **mut):
        s = Settings()
        s.printer.travel = Travel(x_min=-2, x_max=200, y_min=-2, y_max=230)
        if TEXTS[text_key] is not None:
            s.text = TEXTS[text_key]
        for path, v in mut.items():
            obj = s
            parts = path.split("__")
            for k in parts[:-1]:
                obj = getattr(obj, k)
            setattr(obj, parts[-1], v)
        s = Settings.model_validate(s.model_dump())
        out.append((name, s))

    add("sample_default", "sample")
    add("sample_plain", "sample", randomness__enabled=False, connections__enabled=False)
    add("sample_no_hyphen", "sample", typography__hyphenate=False, randomness__seed=7)
    add("sample_jitter_only", "sample", randomness__drift=0.0, randomness__jitter=0.6, randomness__variants=False)
    add("sample_drift_only", "sample", randomness__jitter=0.0, randomness__drift=2.0, randomness__size=0.0)
    add("long_words_narrow", "long_words", sheet__width=70.0, sheet__margin_left=5.0, sheet__margin_right=5.0,
        typography__size_mm=4.0, sheet__line_pitch=12.0)
    add("long_words_wide", "long_words", sheet__width=200.0, typography__size_mm=2.5, sheet__line_pitch=8.0,
        connections__distance=0.6)
    add("soft_and_tabs", "soft_and_tabs", typography__rotation_deg=-2.0, typography__baseline_shift=0.4,
        sheet__indent=15.0, typography__dx=1.5, typography__dy=-0.5)
    add("missing_unresolved", "missing")
    add("missing_resolved", "missing", text_options__missing={
        "⌘": MissingChoice(action="skip"), "☃": MissingChoice(action="replace", replacement="сне\u0301г"),
        "\U0001F600": MissingChoice(action="replace", replacement=":)"), "\u0301": MissingChoice(action="skip"),
        "№": MissingChoice(action="replace", replacement="N")})
    add("soft_and_tabs_resolved", "soft_and_tabs", text_options__missing={
        "²": MissingChoice(action="replace", replacement="2"), "×": MissingChoice(action="skip")})
    add("numbers_resolved", "numbers", text_options__missing={
        "^": MissingChoice(action="skip"), "`": MissingChoice(action="replace", replacement="'")})
    add("numbers", "numbers", randomness__seed=123456789)
    add("empty", "empty")
    add("one_long", "one_long", sheet__width=60.0, sheet__margin_left=5.0, sheet__margin_right=5.0)
    add("one_long_too_narrow", "one_long", sheet__width=12.0, sheet__margin_left=5.0, sheet__margin_right=5.0,
        sheet__indent=0.0, typography__size_mm=5.0, sheet__line_pitch=14.0)
    add("start_word", "sample", text_options__start_word=9)
    add("start_word_past_end", "sample", text_options__start_word=999)
    add("resume_mid", "sample", text_options__resume_word=5, text_options__resume_letter=3)
    add("resume_missing_word", "sample", text_options__resume_word=500)
    add("resume_missing_letter", "sample", text_options__resume_word=2, text_options__resume_letter=40)
    add("overflow", "long_words", sheet__height=40.0, sheet__first_line_top=10.0, sheet__bottom_limit=5.0)
    add("blank_first", "long_words", text="\n\n\tПосле пустых строк в начале. " * 3)
    add("no_lines", "sample", sheet__height=20.0, sheet__first_line_top=15.0, sheet__bottom_limit=10.0)
    add("sheet_error", "sample", sheet__margin_left=100.0, sheet__margin_right=100.0)
    add("out_of_bounds", "sample", printer__travel=Travel(x_min=-2, x_max=50, y_min=-2, y_max=230))
    add("flip_x", "sample", printer__flip_x=True,
        printer__travel=Travel(x_min=-200, x_max=2, y_min=-2, y_max=230))
    add("big_letters_warning", "sample", typography__size_mm=5.0)
    add("zero_pitch", "sample", sheet__line_pitch=0.0)
    return out


def text_case() -> None:
    from handwriter.glyphs import load_provider
    from handwriter.layout import layout
    from handwriter.pipeline import GenerationRefused, find_resume
    from handwriter.text import _dictionary, break_positions, process_text

    prov = load_provider("builtin:hershey_cyrillic.svg")
    hyph = []
    for w in HYPHEN_WORDS:
        hyph.append([w, list(_dictionary("ru_RU").positions(w)), list(_dictionary("en_US").positions(w))])

    def dump_text(pt):
        return {
            "paragraphs": [{"index": p.index, "indent": p.indent, "blank_before": p.blank_before,
                            "words": [w.index for w in p.words]} for p in pt.paragraphs],
            "words": [{"index": w.index, "text": w.text, "paragraph": w.paragraph, "text_line": w.text_line,
                       "soft_breaks": sorted(w.soft_breaks), "breaks": [[k, v] for k, v in break_positions(w, True).items()],
                       "breaks_manual": [[k, v] for k, v in break_positions(w, False).items()]} for w in pt.words],
            "missing": [{"char": m.char, "count": m.count, "positions": m.positions} for m in pt.missing],
            "skipped": sorted(pt.skipped), "unresolved": sorted(pt.unresolved), "replaced": pt.replaced,
        }

    cases = []
    for name, s in text_settings_variants():
        pt = process_text(s.text, prov.has_char, s.text_options.replacements, s.text_options.missing)
        c = compose(s, prov)
        lay = c.layout
        entry = {"name": name, "settings": s.model_dump(mode="json"), "text": dump_text(pt),
                 "errors": c.errors, "warnings": c.warnings}
        if lay is not None:
            entry["layout"] = {
                "glyphs": [g.__dict__ for g in lay.glyphs], "baselines": lay.baselines, "used_lines": lay.used_lines,
                "scale": lay.scale, "first_word": lay.first_word, "last_word": lay.last_word,
                "last_letter": lay.last_letter, "next_word": lay.next_word, "next_letter": lay.next_letter,
                "warnings": lay.warnings, "errors": lay.errors,
                "strokes": [{"points": st.points, "tags": st.tags, "word": st.word, "letter": st.letter,
                             "line": st.line, "hyphen": st.hyphen} for st in lay.strokes],
            }
        if c.resume is not None:
            entry["resume"] = c.resume.__dict__
        try:
            entry["gcode"] = make_gcode(c)
        except GenerationRefused as e:
            entry["refused"] = e.errors
        try:
            entry["test_gcode"] = make_test_gcode(s)[0]
        except GenerationRefused as e:
            entry["test_refused"] = e.errors
        cases.append(entry)
    d = OUT / "text"
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    (d / "cases.json").write_text(json.dumps({"hyphenation": hyph, "cases": cases}, ensure_ascii=False),
                                  encoding="utf-8")
    print(f"text: {len(cases)} cases, {len(hyph)} hyphenation words")


SVG_IMPORTS = {
    "simple_cm": """<svg xmlns="http://www.w3.org/2000/svg" width="10cm" height="6cm" viewBox="0 0 1000 600">
  <g transform="scale(10)" fill="none" stroke="black" style="stroke-width:0.25">
    <rect x="0" y="0" width="100" height="60"/>
    <g transform="translate(10 50)"><line x1="0" y1="0" x2="80" y2="-40"/></g>
    <ellipse cx="30" cy="30" rx="12" ry="12"/>
    <path d="M60 40 a15 15 0 0 1 30 0"/>
    <polyline points="10,10 20,20 30,10"/>
  </g>
</svg>""",
    "css_use_text": """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="210mm" height="148mm"
     viewBox="-5 -5 210 148">
  <style><![CDATA[
    /* comment { stroke: red } */
    .thick { stroke: #000; stroke-width: 0.8 !important ; fill: none }
    .thin, line { stroke: black; stroke-width: 0.2 }
    #special { stroke-dasharray: 4, 1.5 ; stroke-dashoffset: 1 }
    rect.bg { fill: WHITE; stroke: none }
    * { visibility: visible }
    .ghost { visibility: hidden }
    bad selector here { stroke: red }
  ]]></style>
  <defs>
    <symbol id="cross"><path class="thin" d="M-2 0 H2 M0 -2 V2"/></symbol>
    <g id="mark"><circle class="thick" r="1.5"/></g>
  </defs>
  <rect class="bg" width="200" height="138"/>
  <rect class="thick" x="1" y="1" width="198" height="136" rx="4"/>
  <rect class="thick" x="10" y="10" width="20" height="10" ry="30"/>
  <rect class="thick" x="40" y="10" width="0" height="10"/>
  <line id="special" x1="10" y1="30" x2="190" y2="30"/>
  <use xlink:href="#cross" x="50" y="50"/>
  <use href="##mark" x="60" y="50" style="stroke-width: inherit"/>
  <use href="#missing"/>
  <g fill="none" stroke="black" stroke-width="0.35" transform="translate(0 60)">
    <polyline points="0,0 10,5 20,0 30"/>
    <polygon points="40,0 50,10 60,0"/>
    <path d="M70 0 C 80 20 90 -20 100 0 S 120 20 130 0" stroke-dasharray="none"/>
    <path d="M140 0 Q 150 20 160 0 T 180 0" stroke-dasharray="2 1 0.5" stroke-dashoffset="-3"/>
    <path d="L 5 5 oops"/>
    <path class="ghost" d="M0 0 L5 5"/>
    <g display="none"><path d="M0 0 L9 9"/></g>
    <g style="display:none"><path d="M0 0 L9 9"/></g>
    <path d="M0 20 L10 20" stroke="#FFF"/>
    <path d="M0 25 L10 25" stroke="rgb(255, 255, 255)"/>
  </g>
  <g transform="translate(0 80)">
    <path d="M10 0 L20 0 L20 10 Z M30 0 L40 0 L40 10 L30 10"/>
    <circle cx="60" cy="5" r="4" fill="#333"/>
    <ellipse cx="80" cy="5" rx="6" ry="0" fill="black"/>
    <line x1="0" y1="0" x2="5" y2="5" fill="black"/>
    <path d="M90 0 L100 0" fill="none" stroke="none"/>
    <path d="M110 0 L110 0 L110 0 Z" fill="black"/>
    <rect x="120" y="0" width="10" height="10" fill="white"/>
  </g>
  <svg x="150" y="100" width="40" height="30" viewBox="0 0 10 10">
    <rect width="10" height="10" fill="none" stroke="black" stroke-width="0.1"/>
  </svg>
  <text x="20" y="130" font-size="5">Основная <tspan x="40" y="131">надпись</tspan>  тест</text>
  <text><tspan>   </tspan></text>
  <text transform="rotate(90)"><tspan x="3" y="4">повёрнутый</tspan></text>
  <flowRoot><flowPara>поток</flowPara></flowRoot>
  <image x="5" y="5" width="10" height="10"/>
  <switch><g><line x1="0" y1="0" x2="1" y2="1" stroke="black"/></g></switch>
  <a><line x1="1" y1="0" x2="2" y2="1" stroke="black"/></a>
  <foreignObject><line x1="1" y1="0" x2="2" y2="1" stroke="black"/></foreignObject>
  <unknown><line x1="1" y1="0" x2="2" y2="1" stroke="black"/></unknown>
</svg>""",
    "px_only": """<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0 L96 0 L96 96" stroke="black" fill="none"/></svg>""",
    "wh_no_viewbox": """<svg xmlns="http://www.w3.org/2000/svg" width="4in" height="3in"><path d="M0 0 L4 3" stroke="black" stroke-width="0.01"/></svg>""",
    "percent_units": """<svg xmlns="http://www.w3.org/2000/svg" width="100%" height="100%" viewBox="0 0 50 50"><path d="M0 0 L50 50" stroke="black"/></svg>""",
    "pt_units_viewbox": """<svg xmlns="http://www.w3.org/2000/svg" width="72pt" height="36pt" viewBox="0 0 100 40"><path d="M0 0 L100 40" stroke="black"/></svg>""",
    "not_svg": """<html><body/></html>""",
    "broken": """<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0 L1 1" </svg>""",
}
SVG_IMPORTS_BYTES = {
    "cp1251": '<?xml version="1.0" encoding="windows-1251"?>\n<svg xmlns="http://www.w3.org/2000/svg" width="20mm" '
              'height="10mm" viewBox="0 0 20 10"><text x="1" y="5">Чертёж</text><path d="M0 0 L20 10" '
              'stroke="black"/></svg>'.encode("cp1251"),
}


def dump_import(res) -> dict:
    return {
        "kind": res.kind, "name": res.name,
        "paths": [{"points": p.points, "closed": p.closed, "width": p.width, "layer": p.layer,
                   "dash": list(p.dash) if p.dash is not None else None, "dash_offset": p.dash_offset}
                  for p in res.paths],
        "texts": [{"x": t.x, "y": t.y, "text": t.text, "kind": t.kind} for t in res.texts],
        "warnings": res.warnings, "errors": res.errors, "units": res.units, "units_note": res.units_note,
        "pages": res.pages, "page": res.page, "layers": res.layers, "bbox": res.bbox(),
    }


def drawing_import_case() -> None:
    import random

    from handwriter.drawing import ops
    from handwriter.drawing.sources import TEST_SVG
    from handwriter.drawing.svg_import import import_svg
    from handwriter.settings import DrawingImport

    d = OUT / "drawing_import"
    shutil.rmtree(d, ignore_errors=True)
    (d / "input").mkdir(parents=True)
    inputs = {"builtin_test.svg": TEST_SVG.encode("utf-8"), "simple.svg": SVG_SIMPLE.encode("utf-8")}
    inputs |= {f"{k}.svg": v.encode("utf-8") for k, v in SVG_IMPORTS.items()}
    inputs |= {f"{k}.svg": v for k, v in SVG_IMPORTS_BYTES.items()}
    imports = []
    for fname, data in inputs.items():
        (d / "input" / fname).write_bytes(data)
        for units in ("auto", "mm", "in"):
            for tol in (0.05, 0.5):
                if units != "auto" and tol != 0.05:
                    continue
                imp = DrawingImport(units=units)
                imports.append({"file": fname, "units": units, "tol": tol,
                                "result": dump_import(import_svg(data, fname, imp, tol))})

    rng = random.Random(17)

    def rand_path(n, closed=False, scale=20.0):
        pts = [(rng.uniform(-scale, scale), rng.uniform(-scale, scale)) for _ in range(n)]
        if rng.random() < 0.2 and n > 2:
            pts.insert(1, pts[0])
        if rng.random() < 0.15 and n > 3:
            pts[2] = (pts[1][0] + 1e-12, pts[1][1])
        if closed and pts:
            pts.append(pts[0])
        return pts

    opsdata = {"dash": [], "offset": [], "expand": [], "join": [], "order": [], "outside": [], "dedupe": []}
    for _ in range(150):
        pts = rand_path(rng.randint(1, 8))
        pat = [rng.choice([0.0, 0.5, 1.0, 2.5, 4.0]) for _ in range(rng.randint(0, 5))]
        off = rng.choice([0.0, 1.3, -2.7, 100.0])
        opsdata["dash"].append([pts, pat, off, ops.dash_polyline(pts, pat, off)])
        opsdata["dedupe"].append([pts, ops.dedupe(pts)])
    for _ in range(120):
        closed = rng.random() < 0.5
        pts = rand_path(rng.randint(1, 7), closed)
        dd = rng.choice([0.0, 0.15, -0.3, 1.0])
        opsdata["offset"].append([pts, dd, closed, ops.offset_polyline(pts, dd, closed)])
        passes, step = rng.randint(1, 5), rng.choice([0.1, 0.15, 0.5])
        opsdata["expand"].append([pts, closed, passes, step, ops.expand_passes(pts, closed, passes, step)])
    for _ in range(60):
        paths = []
        for _ in range(rng.randint(0, 25)):
            p = rand_path(rng.randint(2, 4), rng.random() < 0.2, 10.0)
            if paths and rng.random() < 0.5:
                q = rng.choice(paths)
                end = q[-1] if rng.random() < 0.5 else q[0]
                p[0] = (end[0] + rng.uniform(-0.03, 0.03), end[1] + rng.uniform(-0.03, 0.03))
            paths.append(p)
        tol = rng.choice([0.0, 0.01, 0.05, 0.2])
        opsdata["join"].append([paths, tol, ops.join_paths(paths, tol)])
        long_path = rng.choice([0.0, 10.0, 30.0, 1e9])
        start = (rng.uniform(-5, 5), rng.uniform(-5, 5))
        opsdata["order"].append([paths, long_path, start, [[i, s] for i, s in ops.order_paths(paths, long_path, start)]])
    grid_paths = [[(float(x), float(y)), (float(x) + 1.0, float(y))] for x in range(30) for y in range(30)]
    rng.shuffle(grid_paths)
    opsdata["order"].append([grid_paths, 0.5, (0.0, 0.0),
                             [[i, s] for i, s in ops.order_paths(grid_paths, 0.5, (0.0, 0.0))]])
    for _ in range(150):
        pts = rand_path(rng.randint(1, 6))
        box = rng.choice([(-5.0, -5.0, 5.0, 5.0), (0.0, 0.0, 20.0, 20.0), (-30.0, -30.0, 30.0, 30.0), (1.0, 1.0, 1.0, 9.0)])
        opsdata["outside"].append([pts, box, ops.outside_parts(pts, box)])

    (d / "cases.json").write_text(json.dumps({"imports": imports, "ops": opsdata}, ensure_ascii=False),
                                  encoding="utf-8")
    print(f"drawing_import: {len(imports)} imports")


def placement_case() -> None:
    import random

    from handwriter.drawing import passes, place

    rng = random.Random(23)
    layouts, places, labels = [], [], []

    def rect(lo=-20.0, hi=300.0):
        x0, y0 = rng.uniform(lo, hi), rng.uniform(lo, hi)
        return (x0, y0, x0 + rng.choice([0.0, 1e-10, rng.uniform(0, 250)]), y0 + rng.choice([0.0, rng.uniform(0, 250)]))

    for _ in range(400):
        s = Settings()
        ds = s.drawing
        ds.sheet.format = rng.choice(["A4", "A3", "custom"])
        ds.sheet.width, ds.sheet.height = rng.choice([210.0, 150.0, 400.0]), rng.choice([297.0, 150.0, 100.0])
        ds.sheet.orientation = rng.choice(["auto", "portrait", "landscape"])
        ds.frame.enabled = rng.random() < 0.5
        ds.frame.title_block = rng.random() < 0.7
        ds.frame.left, ds.frame.bottom = rng.choice([20.0, 5.0, 0.0]), rng.choice([5.0, 12.5])
        ds.frame.tb_width, ds.frame.tb_height = rng.choice([185.0, 400.0, 100.0]), rng.choice([55.0, 200.0, 30.0])
        ds.placement.margin = rng.choice([10.0, 0.0, 33.3])
        ds.placement.scale_mode = rng.choice(["fit", "fit_reach", "fit_passes", "one_to_one", "percent"])
        ds.placement.percent = rng.choice([100.0, 50.0, 233.33, 12.5])
        ds.placement.anchor = rng.choice(["center", "zero"])
        ds.placement.dx, ds.placement.dy = rng.choice([0.0, 3.5]), rng.choice([0.0, -2.25])
        ds.marked.enabled = rng.random() < 0.2
        ds.a3.enabled = not ds.marked.enabled and rng.random() < 0.2
        s = Settings.model_validate(s.model_dump())
        ds = s.drawing
        bbox = rng.choice([None, rect(), (0.0, 0.0, 100.0, 60.0), (5.0, 5.0, 5.0, 5.0), (-50.0, 10.0, 350.0, 20.0)])
        lay = place.sheet_layout(ds, bbox)
        layouts.append({"settings": s.model_dump(mode="json"), "bbox": bbox,
                        "layout": {"width": lay.width, "height": lay.height, "orientation": lay.orientation,
                                   "frame": [[fl.points, fl.thick] for fl in lay.frame], "inner": lay.inner,
                                   "title_block": lay.title_block, "areas": lay.areas}})
        if bbox is not None:
            reach = [rect(-10.0, 200.0) for _ in range(rng.randint(0, 3))]
            pad = rng.choice([0.0, 0.5, 2.0])
            pl = place.place(ds, bbox, lay, reach, pad)
            places.append({"index": len(layouts) - 1, "reach": reach, "pad": pad,
                           "placement": {"scale": pl.scale, "tx": pl.tx, "ty": pl.ty, "area": pl.area,
                                         "errors": pl.errors, "warnings": pl.warnings,
                                         "applied": pl.apply((12.5, -3.0))}})
    for v in [0.0, -1.0, 1.0, 1.0 + 1e-10, 0.5, 0.25, 0.2, 1 / 3, 2.0, 2.5, 10.0, 0.004, 1.004, 3.14159, 0.9999,
              199.996, 0.333333, 7.505, 0.00001]:
        labels.append([v, place.scale_label(v)])

    pas = {"maps": [], "tables": [], "uncovered": []}
    for _ in range(200):
        W, H = rng.choice([210.0, 297.0, 420.0, 123.4]), rng.choice([297.0, 210.0, 77.7])
        r = rng.choice(passes.ROTATIONS)
        pt = (rng.uniform(-10, 450), rng.uniform(-10, 450))
        R = rect(-10.0, 300.0)
        pas["maps"].append([W, H, r, pt, passes.to_pass(pt, r, W, H), passes.from_pass(pt, r, W, H),
                            passes.corner_point(r, W, H), list(passes.pass_dims(r, W, H)), R,
                            passes.rect_from_pass(R, r, W, H)])
    for _ in range(200):
        s = Settings()
        pr = s.printer
        if rng.random() < 0.8:
            pr.travel = Travel(x_min=rng.uniform(-30, 0), x_max=rng.uniform(100, 320),
                               y_min=rng.uniform(-30, 0), y_max=rng.uniform(100, 320))
        pr.flip_x, pr.flip_y = rng.random() < 0.3, rng.random() < 0.3
        pr.safety_margin = rng.choice([0.0, 2.0, 7.5])
        pr.work_w, pr.work_h = rng.choice([220.0, 255.5]), rng.choice([220.0, 180.0])
        pr.table.overhang_x, pr.table.overhang_y = rng.random() < 0.5, rng.random() < 0.5
        if rng.random() < 0.6:
            pr.table.table_x, pr.table.table_y = rng.uniform(150, 320), rng.uniform(150, 320)
        s = Settings.model_validate(s.model_dump())
        W, H = rng.choice([(210.0, 297.0), (297.0, 210.0), (297.0, 420.0), (420.0, 297.0), (100.0, 100.0)])
        tb = passes.make_table(s.printer, W, H)
        per_r = []
        for r in passes.ROTATIONS:
            per_r.append([r, list(passes.overhang(r, W, H, tb)), list(passes.rotation_allowed(r, W, H, tb)),
                          passes.pass_rect(r, W, H, tb.safe)])
        pas["tables"].append({"printer": s.model_dump(mode="json")["printer"], "W": W, "H": H,
                              "table": tb.__dict__, "rotations": per_r})
    for _ in range(200):
        target = rect(0.0, 100.0)
        rects = [rect(-20.0, 150.0) for _ in range(rng.randint(0, 5))]
        if rng.random() < 0.3:
            rects.append(target)
        if rng.random() < 0.3 and rects:
            q = rects[0]
            rects.append((q[2], q[1], q[2] + 10.0, q[3]))
        un = passes.uncovered(target, rects)
        pas["uncovered"].append([target, rects, un, passes.area(un), passes.intersect(target, rects[0]) if rects else None])

    d = OUT / "placement"
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    (d / "cases.json").write_text(json.dumps({"layouts": layouts, "places": places, "labels": labels,
                                              "passes": pas}, ensure_ascii=False), encoding="utf-8")
    print(f"placement: {len(layouts)} layouts, {len(places)} placements")


def plan_case() -> None:
    import random

    from handwriter.drawing import passes

    rng = random.Random(29)
    out = {"covered": [], "plans": [], "marked": [], "a3": []}

    def rand_strokes():
        st = []
        for _ in range(rng.randint(0, 6)):
            n = rng.randint(1, 5)
            st.append([(rng.uniform(-20, 320), rng.uniform(-20, 440)) for _ in range(n)])
        if rng.random() < 0.2:
            st.append([(10.0, 10.0), (10.0, 10.0), (10.0, 50.0)])
        return st

    def rand_rect():
        x0, y0 = rng.uniform(-30, 250), rng.uniform(-30, 350)
        return (x0, y0, x0 + rng.uniform(0, 300), y0 + rng.uniform(0, 300))

    for _ in range(300):
        strokes = rand_strokes()
        rects = [rand_rect() for _ in range(rng.randint(0, 4))]
        if rng.random() < 0.3 and rects:
            q = rects[0]
            rects.append((q[2] - 1e-8, q[1], q[2] + 50.0, q[3]))
        A, B = passes.segments(strokes)
        out["covered"].append([strokes, rects, passes.covered_mask(A, B, rects).tolist(),
                               passes.lines_covered(strokes, rects)])

    for _ in range(120):
        s = Settings()
        pr = s.printer
        if rng.random() < 0.85:
            pr.travel = Travel(x_min=rng.uniform(-20, 0), x_max=rng.uniform(80, 300),
                               y_min=rng.uniform(-20, 0), y_max=rng.uniform(80, 300))
        pr.flip_x, pr.flip_y = rng.random() < 0.2, rng.random() < 0.2
        pr.safety_margin = rng.choice([0.0, 2.0])
        pr.table.overhang_x, pr.table.overhang_y = rng.random() < 0.5, rng.random() < 0.3
        if rng.random() < 0.6:
            pr.table.table_x, pr.table.table_y = rng.uniform(120, 320), rng.uniform(120, 320)
        s = Settings.model_validate(s.model_dump())
        W, H = rng.choice([(210.0, 297.0), (297.0, 210.0), (297.0, 420.0), (420.0, 297.0), (150.0, 100.0)])
        target = rng.choice([(0.0, 0.0, W, H), (20.0, 5.0, W - 5.0, H - 5.0), (W / 3, H / 3, W / 2, H / 2)])
        what = rng.choice(["лист", "чертёж в рабочем поле"])
        pl = passes.plan_sheet(W, H, target, s.printer, what)
        out["plans"].append({"printer": s.model_dump(mode="json")["printer"], "W": W, "H": H, "target": target,
                             "what": what,
                             "plan": {"allowed": pl.allowed, "rects": [[r, pl.rects[r]] for r in pl.allowed],
                                      "rotations": pl.rotations, "uncovered": pl.uncovered, "message": pl.message,
                                      "notes": pl.notes, "ok": pl.ok, "describe": pl.describe(),
                                      "describe_all": pl.describe(pl.allowed)}})

    for _ in range(60):
        s = Settings()
        mk = s.drawing.marked
        if rng.random() < 0.7:
            mk.x_min, mk.x_max = rng.uniform(-5, 30), rng.uniform(150, 240)
            mk.y_min, mk.y_max = rng.uniform(-5, 30), rng.uniform(150, 260)
        z = s.drawing.a3
        if rng.random() < 0.7:
            z.x_min, z.x_max = rng.uniform(-5, 30), rng.uniform(150, 260)
            z.y_min, z.y_max = rng.uniform(-5, 30), rng.uniform(150, 260)
        s = Settings.model_validate(s.model_dump())
        mk, z = s.drawing.marked, s.drawing.a3
        W, H = rng.choice([(210.0, 297.0), (297.0, 210.0), (120.0, 80.0)])
        pts = [(rng.uniform(0, W), rng.uniform(0, H)) for _ in range(3)]
        aff = {r: passes.marked_affine(mk, W, H, r) for r in (0, 180)}
        out["marked"].append({"drawing": s.model_dump(mode="json")["drawing"], "W": W, "H": H,
                              "affine": [[r, m, [passes.apply_affine(m, q) for q in pts],
                                          [passes.invert_affine(m, q) for q in pts]] for r, m in aff.items()],
                              "pts": pts, "rects": list(passes.marked_rects(mk, W, H))})
        W3, H3 = rng.choice([(420.0, 297.0), (297.0, 420.0), (300.0, 200.0)])
        aff3 = {r: passes.a3_affine(z, W3, H3, r) for r in passes.A3_RUNS}
        allowed, rects, notes = passes.a3_rects(z, W3, H3)
        out["a3"].append({"drawing": s.model_dump(mode="json")["drawing"], "W": W3, "H": H3,
                          "affine": [[r, m, [passes.apply_affine(m, q) for q in pts]] for r, m in aff3.items()],
                          "pts": pts, "rects": [allowed, [[r, rects[r]] for r in allowed], notes]})

    for m in out["marked"]:
        allowed, rects, notes = m["rects"]
        m["rects"] = [allowed, [[r, rects[r]] for r in allowed], notes]
    d = OUT / "plan"
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    (d / "cases.json").write_text(json.dumps(out, ensure_ascii=False), encoding="utf-8")
    print(f"plan: {len(out['covered'])} coverage, {len(out['plans'])} plans")


def split_geometry_case() -> None:
    import random

    import numpy as np

    from handwriter.drawing import split

    rng = random.Random(31)
    cases = []
    for _ in range(150):
        strokes = []
        for _ in range(rng.randint(0, 6)):
            kind = rng.random()
            if kind < 0.3:
                cx, cy, r = rng.uniform(0, 200), rng.uniform(0, 200), rng.uniform(1, 40)
                n = rng.randint(8, 40)
                st = [(cx + r * np.cos(2 * np.pi * k / n), cy + r * np.sin(2 * np.pi * k / n)) for k in range(n)]
                st.append(st[0])
            elif kind < 0.4:
                st = [(rng.uniform(0, 200), rng.uniform(0, 200))]
            else:
                st = [(rng.uniform(0, 200), rng.uniform(0, 200)) for _ in range(rng.randint(2, 6))]
                if rng.random() < 0.2:
                    st.insert(1, st[0])
            strokes.append([(float(x), float(y)) for x, y in st])
        geo = split.Geometry(strokes)
        x0, y0 = rng.uniform(-20, 150), rng.uniform(-20, 150)
        Q = (x0, y0, x0 + rng.uniform(0, 150), y0 + rng.uniform(0, 150))
        rects = [(a, b, a + rng.uniform(0, 200), b + rng.uniform(0, 200))
                 for a, b in ((rng.uniform(-20, 150), rng.uniform(-20, 150)) for _ in range(rng.randint(0, 3)))]
        axis = rng.randint(0, 1)
        lo, hi = Q[axis], Q[axis + 2]
        cands = np.arange(lo, hi + 1e-9, rng.choice([0.25, 1.0, 7.5])) if hi > lo else np.array([lo])
        ov = rng.choice([0.0, 0.5, 2.0])
        P = np.array([(rng.uniform(-10, 210), rng.uniform(-10, 210)) for _ in range(4)])
        A2, B2, idx = split.clip_segments(geo.A, geo.B, Q)
        cases.append({
            "strokes": strokes, "Q": Q, "rects": rects, "axis": axis, "cands": cands.tolist(), "ov": ov,
            "P": P.tolist(),
            "geo": {"A": geo.A.tolist(), "B": geo.B.tolist(), "seg_path": geo.seg_path.tolist(),
                    "curved": geo.curved.tolist(), "seg_len": geo.seg_len.tolist(), "path_len": geo.path_len.tolist()},
            "clip": [A2.tolist(), B2.tolist(), idx.tolist()],
            "covered": geo.covered(Q, rects), "seam_cost": geo.seam_cost(Q, axis, cands, ov).tolist(),
            "distance": [v if np.isfinite(v) else repr(v) for v in geo.distance_to(P).tolist()],
        })
    d = OUT / "split"
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    (d / "geometry.json").write_text(json.dumps(cases, allow_nan=False), encoding="utf-8")
    print(f"split geometry: {len(cases)} cases")


def dump_node(n):
    if n is None:
        return None
    return {"core": n.core, "ext": n.ext, "rotation": n.rotation, "axis": n.axis, "s": n.s, "cost": n.cost,
            "low": dump_node(n.low), "high": dump_node(n.high)}


def split_case() -> None:
    import random

    import numpy as np

    from handwriter.drawing import passes, split

    rng = random.Random(37)
    cases = []
    tries = 0
    while len(cases) < 40 and tries < 400:
        tries += 1
        s = Settings()
        pr = s.printer
        pr.travel = Travel(x_min=rng.uniform(-5, 0), x_max=rng.uniform(150, 260),
                           y_min=rng.uniform(-5, 0), y_max=rng.uniform(150, 260))
        pr.safety_margin = rng.choice([0.0, 2.0])
        pr.table.overhang_x, pr.table.overhang_y = True, rng.random() < 0.5
        s = Settings.model_validate(s.model_dump())
        W, H = rng.choice([(297.0, 210.0), (210.0, 297.0), (420.0, 297.0), (297.0, 420.0)])
        allowed, rects, _ = passes.rotation_rects(W, H, s.printer)
        if len(allowed) < 2:
            continue
        strokes = []
        for _ in range(rng.randint(1, 25)):
            if rng.random() < 0.25:
                cx, cy, r = rng.uniform(20, W - 20), rng.uniform(20, H - 20), rng.uniform(2, 30)
                n = rng.randint(12, 48)
                st = [(float(cx + r * np.cos(2 * np.pi * k / n)), float(cy + r * np.sin(2 * np.pi * k / n))) for k in range(n)]
                st.append(st[0])
            elif rng.random() < 0.1:
                st = [(rng.uniform(5, W - 5), rng.uniform(5, H - 5))]
            else:
                st = [(rng.uniform(5, W - 5), rng.uniform(5, H - 5)) for _ in range(rng.randint(2, 5))]
            strokes.append(st)
        A, B = passes.segments(strokes)
        if not passes.covered_mask(A, B, [rects[r] for r in allowed]).all():
            continue
        geo = split.Geometry(strokes)
        sheet = (0.0, 0.0, W, H)
        ov, slack = rng.choice([0.5, 1.0]), rng.choice([0.0, 1.0])
        root = split.solve(geo, rects, list(allowed), sheet, sheet, ov + slack, ov)
        entry = {"W": W, "H": H, "allowed": allowed, "rects": [[r, rects[r]] for r in allowed], "strokes": strokes,
                 "ov": ov, "margin": ov + slack, "root": dump_node(root)}
        if root is not None:
            stats = split.CutStats()
            entry["cuts"] = [[[rot, piece] for rot, piece in split.cut_stroke(st, root, ov, stats)] for st in strokes]
            entry["stats"] = stats.__dict__
            size, count = rng.choice([3.0, 5.0]), rng.choice([1, 3])
            marks = split.control_marks(root, geo, rects, size, count)
            entry["mark_size"], entry["mark_count"] = size, count
            entry["marks"] = [[m.x, m.y, list(m.passes), split.mark_strokes(m, size)] for m in marks]
        cases.append(entry)
    extract = []
    for _ in range(100):
        n = rng.randint(2, 8)
        pts = [(rng.uniform(0, 50), rng.uniform(0, 50)) for _ in range(n)]
        closed = rng.random() < 0.5 and n > 2
        if closed:
            pts.append(pts[0])
        cum = split._cum(pts)
        L = cum[-1]
        a = rng.uniform(-L, L)
        b = a + rng.uniform(0, 1.5 * L)
        axis, sv = rng.randint(0, 1), rng.uniform(0, 50)
        extract.append([pts, closed, a, b, cum, split._extract(pts, cum, a, b, closed),
                        split._point_at(pts, cum, a), axis, sv, split._crossings(pts, cum, axis, sv)])
    bis = []
    for lo, hi, thr, want in [(0.0, 10.0, 3.3, True), (0.0, 10.0, 3.3, False), (0.0, 10.0, -1.0, True),
                              (0.0, 10.0, 11.0, False), (2.5, 2.5, 2.5, True), (0.0, 100.0, 99.999, True)]:
        f = (lambda s, thr=thr: s <= thr) if want else (lambda s, thr=thr: s >= thr)
        bis.append([lo, hi, thr, want, split._bisect(f, lo, hi, want)])
    d = OUT / "split"
    d.mkdir(parents=True, exist_ok=True)
    (d / "split.json").write_text(json.dumps({"cases": cases, "extract": extract, "bisect": bis}, allow_nan=False),
                                  encoding="utf-8")
    print(f"split: {len(cases)} cases, {sum(1 for c in cases if c['root'])} solved")


def drawing_pipeline_case() -> None:
    import random

    from handwriter.drawing.pipeline import (compose_drawing, gcode_filename, make_all_files, preview_payload)
    from handwriter.pipeline import GenerationRefused

    d = OUT / "drawing_pipeline"
    shutil.rmtree(d, ignore_errors=True)
    (d / "input").mkdir(parents=True)
    files = {"simple.svg": SVG_SIMPLE, "css_use_text.svg": SVG_IMPORTS["css_use_text"],
             "simple_cm.svg": SVG_IMPORTS["simple_cm"], "wide.svg": SVG_IMPORTS["wh_no_viewbox"],
             "empty.svg": '<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"><text x="1" y="1">t</text>'
                          '<image x="1" y="1"/></svg>',
             "framed.svg": '<svg xmlns="http://www.w3.org/2000/svg" width="400mm" height="280mm" viewBox="0 0 400 280">'
                           '<g fill="none" stroke="black" stroke-width="0.5"><rect x="0" y="0" width="400" height="280"/>'
                           '<line x1="0" y1="200" x2="400" y2="200"/><circle cx="120" cy="100" r="40"/>'
                           '<path d="M200 40 L380 40 L380 180" stroke-dasharray="0.1 0.1"/></g></svg>'}
    for name, text in files.items():
        (d / "input" / name).write_text(text, encoding="utf-8")
        (user_drawings_dir() / name).write_text(text, encoding="utf-8")

    rng = random.Random(41)
    variants = []

    def base():
        s = Settings()
        s.printer.travel = Travel(x_min=-2, x_max=300, y_min=-2, y_max=300)
        return s

    def v(name, s):
        variants.append((name, Settings.model_validate(s.model_dump())))

    s = base(); v("builtin_fit", s)
    s = base(); s.drawing.frame.enabled = True; s.drawing.weights.enabled = True; s.drawing.placement.scale_mode = "fit_reach"; v("frame_weights_reach", s)
    s = base(); s.drawing.file = "simple.svg"; s.drawing.placement.scale_mode = "one_to_one"; s.drawing.placement.anchor = "zero"; v("simple_1to1_zero", s)
    s = base(); s.drawing.file = "simple.svg"; s.drawing.placement.scale_mode = "percent"; s.drawing.placement.percent = 450.0; v("simple_percent_big", s)
    s = base(); s.drawing.file = "css_use_text.svg"; s.drawing.sheet.format = "A3"; v("css_a3", s)
    s = base(); s.drawing.file = "css_use_text.svg"; s.printer.travel = Travel(x_min=-2, x_max=200, y_min=-2, y_max=215); v("css_small_reach", s)
    s = base(); s.drawing.file = "css_use_text.svg"; s.printer.travel = Travel(x_min=-2, x_max=200, y_min=-2, y_max=215); s.drawing.placement.scale_mode = "fit_passes"; v("css_fit_passes", s)
    s = base(); s.drawing.file = "simple_cm.svg"; s.drawing.split.marks = True; s.drawing.split.mark_count = 2; s.drawing.sheet.format = "A3"; s.printer.travel = Travel(x_min=-2, x_max=250, y_min=-2, y_max=250); v("cm_a3_marks", s)
    s = base(); s.drawing.file = "simple_cm.svg"; s.drawing.sheet.format = "A3"; s.printer.travel = Travel(x_min=-2, x_max=250, y_min=-2, y_max=250); s.drawing.split.offsets = {"180": (0.4, -0.25)}; s.drawing.split.areas = 3; v("cm_a3_areas_offsets", s)
    s = base(); s.drawing.file = "wide.svg"; s.drawing.sheet.format = "custom"; s.drawing.sheet.width = 150.0; s.drawing.sheet.height = 100.0; v("wide_custom", s)
    s = base(); s.drawing.file = "empty.svg"; v("empty", s)
    s = base(); s.drawing.file = "missing.svg"; v("missing_file", s)
    s = base(); s.drawing.file = "framed.svg"; s.drawing.marked.enabled = True; v("framed_marked_fit_frame", s)
    s = base(); s.drawing.file = "framed.svg"; s.drawing.a3.enabled = True; v("framed_a3_fit_frame", s)
    s = base(); s.drawing.file = "framed.svg"; s.drawing.a3.enabled = True; s.drawing.frame.enabled = True; s.drawing.split.marks = True; v("framed_a3_frame_marks", s)
    s = base(); s.drawing.marked.enabled = True; s.drawing.weights.enabled = True; v("marked_weights", s)
    s = base(); s.drawing.marked.enabled = True; s.drawing.marked.x_max = 5.0; v("marked_bad_zone", s)
    s = base(); s.printer.travel = None; v("unmeasured", s)
    s = base(); s.printer.travel = Travel(x_min=-2, x_max=120, y_min=-2, y_max=120); s.drawing.placement.scale_mode = "one_to_one"; v("unreachable_1to1", s)
    s = base(); s.printer.travel = Travel(x_min=-2, x_max=120, y_min=-2, y_max=120); s.drawing.frame.enabled = True; s.printer.table.overhang_x = False; v("unreachable_frame", s)
    s = base(); s.printer.table.table_x, s.printer.table.table_y = 230.0, 230.0; s.printer.table.overhang_y = False; s.drawing.sheet.format = "A3"; v("table_a3", s)
    s = base(); s.drawing.placement.anchor = "zero"; s.drawing.weights.enabled = True; s.drawing.split.areas = 2; v("zero_weights_areas", s)
    s = base(); s.drawing.sheet.orientation = "portrait"; s.drawing.split.areas = 3; s.printer.travel = Travel(x_min=-3, x_max=200, y_min=-3, y_max=215); v("portrait_three_areas", s)
    s = base(); s.drawing.file = "simple.svg"; s.drawing.placement.dx, s.drawing.placement.dy = 500.0, 0.0; v("off_sheet", s)
    s = base(); s.drawing.file = "simple.svg"; s.drawing.paths.join_tol = 0.0; s.drawing.paths.long_path = 0.0; s.printer.simplify_tol = 0.5; v("no_join", s)
    s = base(); s.printer.flip_x = True; s.printer.travel = Travel(x_min=-300, x_max=2, y_min=-2, y_max=300); v("flip_x", s)
    for i in range(30):
        s = base()
        ds = s.drawing
        ds.file = rng.choice(["builtin:test", "simple.svg", "css_use_text.svg", "simple_cm.svg", "framed.svg"])
        ds.sheet.format = rng.choice(["A4", "A3", "custom"])
        ds.sheet.width, ds.sheet.height = rng.choice([(150.0, 200.0), (300.0, 200.0)])
        ds.sheet.orientation = rng.choice(["auto", "portrait", "landscape"])
        ds.placement.scale_mode = rng.choice(["fit", "fit_reach", "fit_passes", "one_to_one", "percent"])
        ds.placement.percent = rng.choice([50.0, 100.0, 150.0])
        ds.placement.anchor = rng.choice(["center", "zero"])
        ds.placement.margin = rng.choice([5.0, 10.0])
        ds.frame.enabled = rng.random() < 0.4
        ds.frame.title_block = rng.random() < 0.7
        ds.weights.enabled = rng.random() < 0.4
        ds.weights.threshold = rng.choice([0.3, 0.5])
        ds.split.marks = rng.random() < 0.4
        ds.split.slack = rng.choice([0.0, 1.0])
        ds.marked.enabled = rng.random() < 0.15
        ds.a3.enabled = not ds.marked.enabled and rng.random() < 0.15
        s.printer.travel = Travel(x_min=-2, x_max=rng.uniform(150, 320), y_min=-2, y_max=rng.uniform(150, 320))
        s.printer.table.overhang_y = rng.random() < 0.5
        v(f"random_{i}", s)

    cases = []
    for name, s in variants:
        clear_cache()
        c = compose_drawing(s)
        entry = {"name": name, "settings": s.model_dump(mode="json"), "preview": preview_payload(c),
                 "frame_fitted": c.frame_fitted, "dashed_solid": c.dashed_solid,
                 "source_strokes": len(c.source_strokes)}
        try:
            entry["files"] = [{k: f[k] for k in ("filename", "gcode", "pass", "rotation", "test")}
                              for f in make_all_files(c, tests=True)]
            entry["gcode_filename"] = gcode_filename(c)
        except GenerationRefused as e:
            entry["refused"] = e.errors
        cases.append(entry)
    (d / "cases.json").write_text(json.dumps(cases, ensure_ascii=False, allow_nan=False), encoding="utf-8")
    print(f"drawing_pipeline: {len(cases)} cases, {sum('files' in c for c in cases)} with gcode")


def calibration_case() -> None:
    import random

    from handwriter.calibration import check_errors, make_reach_check_gcode, make_zero_gcode, reach_corners
    from handwriter.pipeline import GenerationRefused

    rng = random.Random(43)
    out = []
    for i in range(80):
        s = Settings()
        pr = s.printer
        if i % 7:
            pr.travel = Travel(x_min=rng.uniform(-10, 5), x_max=rng.uniform(0, 250),
                               y_min=rng.uniform(-10, 5), y_max=rng.uniform(0, 250))
        pr.safety_margin = rng.choice([0.0, 2.0, 7.5, 60.0])
        pr.table.touch_s, pr.table.pause_s = rng.choice([1.0, 0.25, 2.5]), rng.choice([2.0, 0.5, 0.0005])
        pr.table.overhang_x, pr.table.overhang_y = rng.random() < 0.5, rng.random() < 0.5
        pr.pen_up_z = rng.choice([4.0, 1.5, -2.0])
        pr.feed_z = rng.choice([600.0, 0.0, 450.5])
        s = Settings.model_validate(s.model_dump())
        e = {"settings": s.model_dump(mode="json")}
        errors, warnings = check_errors(s)
        e["info"] = {"errors": errors, "warnings": warnings,
                     "corners": reach_corners(s) if s.printer.travel is not None and not errors else []}
        for key, fn in (("check", make_reach_check_gcode), ("zero", make_zero_gcode)):
            try:
                e[key] = fn(s)
            except GenerationRefused as ex:
                e[key + "_refused"] = ex.errors
        out.append(e)
    (OUT / "calibration.json").write_text(json.dumps(out, ensure_ascii=False), encoding="utf-8")
    print(f"calibration: {len(out)} cases")


def skeleton_case() -> None:
    import random

    import numpy as np
    from scipy import ndimage
    from skimage.morphology import skeletonize

    from handwriter.drawing import fills
    from handwriter.glyphs import skeleton as sk
    from handwriter.glyphs.outline_provider import _FontData

    fd = _FontData(TEST_FONTS / "BadScript-Regular.ttf")
    xh = fd.metrics.x_height
    chars = "абвгдеёжзийклмнопрстуфхцчшщъыьэюяАБВГДЖЗИЙКФШЩЫЮЯabcdefghijkpqxyzRSWQ0123456789@&%?!.,;«»"
    names = []
    for ch in chars:
        n = fd.cmap.get(ord(ch))
        if n and n not in names:
            names.append(n)
    variants = [sk.SkeletonParams(), sk.SkeletonParams(px_per_em=600.0, prune=0.2, extend=0.0, smooth=0.0,
                                                       simplify=0.01, junction_merge=0.0),
                sk.SkeletonParams(px_per_em=2400.0, prune=0.02, extend=2.0, smooth=0.1, simplify=0.001,
                                  junction_merge=6.0)]
    glyphs = []
    for i, n in enumerate(names):
        contours = fd.contours(n)
        params = variants[i % 3] if i % 5 else variants[0]
        r = sk.skeleton_strokes(contours, xh, params, want_raw=i < 15)
        glyphs.append({"name": n, "want_raw": i < 15, "contours": contours, "x_height": xh, "params": params.__dict__,
                       "key": params.key(),
                       "strokes": r.strokes, "raw": r.raw, "closed": r.closed})
    rng = random.Random(47)
    prims = []
    for i, n in enumerate(names[:25]):
        contours = fd.contours(n)
        pts = [p for c in contours for p in c]
        if not pts:
            continue
        ppem = rng.choice([300.0, 700.0])
        xmin, ymax = min(p[0] for p in pts), max(p[1] for p in pts)
        w = int(np.ceil((max(p[0] for p in pts) - xmin) * ppem)) + 8
        h = int(np.ceil((ymax - min(p[1] for p in pts)) * ppem)) + 8
        px = [[((p[0] - xmin) * ppem + 4, (ymax - p[1]) * ppem + 4) for p in c] for c in contours]
        mask = sk.rasterize(px, w, h)
        skel = skeletonize(mask)
        dt = ndimage.distance_transform_edt(mask)
        prims.append({"contours_px": px, "w": w, "h": h, "mask": mask_rle(mask), "skel": mask_rle(skel),
                      "dt_sum": float(dt.sum()), "dt_on": [float(v) for v in dt.ravel()[np.nonzero(mask.ravel())[0]]][:4000],
                      "reference_px": fills.reference_px(mask)})
    fill_cases = []
    for _ in range(25):
        cx, cy, r = rng.uniform(0, 50), rng.uniform(0, 50), rng.uniform(0.3, 4.0)
        n = rng.randint(3, 30)
        outer = [(cx + r * np.cos(2 * np.pi * k / n), cy + r * np.sin(2 * np.pi * k / n) * rng.choice([1.0, 0.4]))
                 for k in range(n)]
        outer = [(float(x), float(y)) for x, y in outer]
        contours = [outer + [outer[0]]]
        if rng.random() < 0.3:
            inner = [(cx + 0.4 * r * np.cos(-2 * np.pi * k / 12), cy + 0.4 * r * np.sin(-2 * np.pi * k / 12)) for k in range(12)]
            contours.append([(float(x), float(y)) for x, y in inner] + [(float(inner[0][0]), float(inner[0][1]))])
        fill_cases.append({"contours": contours, "lines": fills.fill_centerlines(contours)})
    d = OUT / "skeleton"
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    (d / "cases.json").write_text(json.dumps({"glyphs": glyphs, "prims": prims, "fills": fill_cases}, allow_nan=False),
                                  encoding="utf-8")
    print(f"skeleton: {len(glyphs)} glyphs, {len(prims)} primitive sets, {len(fill_cases)} fills")


def raster_images() -> dict:
    import random

    from PIL import Image, ImageDraw

    rng = random.Random(53)

    def base(w, h, bg=255):
        im = Image.new("L", (w, h), bg)
        d = ImageDraw.Draw(im)
        for _ in range(6):
            x0, y0 = rng.randint(0, w - 1), rng.randint(0, h - 1)
            x1, y1 = rng.randint(0, w - 1), rng.randint(0, h - 1)
            d.line((x0, y0, x1, y1), fill=rng.randint(0, 90), width=rng.randint(2, 6))
        d.ellipse((w // 4, h // 4, w // 2, h // 2), outline=20, width=4)
        d.rectangle((3, 3, w - 4, h - 4), outline=0, width=3)
        for _ in range(5):
            x, y = rng.randint(0, w - 3), rng.randint(0, h - 3)
            d.point((x, y), fill=0)
        return im

    out = {}

    def save(name, im, **kw):
        import io
        b = io.BytesIO()
        im.save(b, **kw)
        out[name] = b.getvalue()

    g = base(240, 180)
    save("gray.png", g, format="PNG")
    save("gray_dpi.png", g, format="PNG", dpi=(150, 150))
    rgb = Image.merge("RGB", (g, g.point(lambda v: min(255, v + 30)), g.point(lambda v: v // 2 + 100)))
    save("rgb.png", rgb, format="PNG")
    rgba = rgb.convert("RGBA")
    a = Image.new("L", g.size, 255)
    ImageDraw.Draw(a).rectangle((0, 0, 120, 90), fill=0)
    ImageDraw.Draw(a).rectangle((120, 90, 239, 179), fill=128)
    rgba.putalpha(a)
    save("rgba.png", rgba, format="PNG")
    la = Image.merge("LA", (g, a))
    save("la.png", la, format="PNG")
    p_img = rgb.quantize(colors=16)
    save("palette.png", p_img, format="PNG", transparency=bytes([255, 0] + [255] * 14))
    save("palette_plain.png", rgb.quantize(colors=7), format="PNG")
    save("bilevel.png", g.point(lambda v: 255 if v > 128 else 0).convert("1"), format="PNG")
    g16 = g.point(lambda v: v).convert("I")
    g16 = g16.point(lambda v: v * 3)
    save("gray16.png", g16.convert("I;16"), format="PNG")
    save("rgb.jpg", rgb, format="JPEG", quality=88, dpi=(200, 200))
    save("gray.jpg", g, format="JPEG", quality=70)
    exif = Image.Exif()
    exif[0x0112] = 6
    exif[0x011A] = 254.0
    exif[0x0128] = 3
    save("rotated_exif.jpg", rgb, format="JPEG", quality=90, exif=exif.tobytes())
    exif2 = Image.Exif()
    exif2[0x0112] = 3
    save("rotated_noresunit.jpg", g, format="JPEG", quality=90, exif=exif2.tobytes())
    exif3 = Image.Exif()
    exif3[0x0112] = 8
    save("rotated8.png", g, format="PNG", exif=exif3.tobytes())
    big = base(3200, 900)
    save("big.png", big, format="PNG")
    noisy = Image.effect_noise((420, 300), 90).point(lambda v: 0 if v < 110 else 255)
    save("noisy.png", noisy, format="PNG")
    save("flat.png", Image.new("L", (40, 30), 77), format="PNG")
    out["broken.png"] = b"\x89PNG\r\n\x1a\nbroken"
    return out


def mask_rle(m) -> list:
    import numpy as np
    flat = np.asarray(m, dtype=bool).ravel()
    runs, cur, n = [], False, 0
    for v in flat:
        if v == cur:
            n += 1
        else:
            runs.append(n)
            cur, n = v, 1
    runs.append(n)
    return runs


def raster_case() -> None:
    from handwriter.drawing.pipeline import compose_drawing, make_all_files, preview_payload
    from handwriter.drawing.raster_import import import_raster
    from handwriter.pipeline import GenerationRefused
    from handwriter.settings import DrawingImport

    d = OUT / "raster"
    shutil.rmtree(d, ignore_errors=True)
    (d / "input").mkdir(parents=True)
    images = raster_images()
    cases = []
    for name, data in images.items():
        (d / "input" / name).write_bytes(data)
        (user_drawings_dir() / name).write_bytes(data)
        variants = [DrawingImport()]
        if name in ("gray.png", "rgb.jpg"):
            variants += [DrawingImport(threshold_auto=False, threshold=60, invert=False),
                         DrawingImport(threshold_auto=False, threshold=200, invert=True, raster_dpi=96.0),
                         DrawingImport(raster_mode="fill")]
        for opts in variants:
            r = import_raster(data, name, opts)
            e = dump_import(r)
            e["info"] = r.info
            e["fill_px"] = r.fill_px
            e["fill_mask"] = mask_rle(r.fill_mask) if r.fill_mask is not None else None
            e["fill_bbox"] = r.fill_bbox()
            cases.append({"file": name, "opts": opts.model_dump(mode="json"), "result": e})
    drawings = []
    for name, mode in (("gray.png", "centerlines"), ("rgb.jpg", "fill"), ("noisy.png", "centerlines"), ("big.png", "fill")):
        s = Settings()
        s.printer.travel = Travel(x_min=-2, x_max=300, y_min=-2, y_max=300)
        s.drawing.file = name
        s.drawing.imp.raster_mode = mode
        s.drawing.imp.fill_dir = "auto" if name != "big.png" else "vertical"
        s = Settings.model_validate(s.model_dump())
        clear_cache()
        c = compose_drawing(s)
        entry = {"file": name, "settings": s.model_dump(mode="json"), "preview": preview_payload(c)}
        try:
            entry["files"] = [{"filename": f["filename"], "gcode": f["gcode"]} for f in make_all_files(c, True)]
        except GenerationRefused as ex:
            entry["refused"] = ex.errors
        drawings.append(entry)
    from handwriter.drawing.svg_import import import_svg
    svg_fills = []
    fill_svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="60mm" height="40mm" viewBox="0 0 60 40">'
                '<rect x="2" y="2" width="1.2" height="20" fill="black"/><circle cx="20" cy="20" r="3" fill="#222"/>'
                '<path d="M30 5 L50 5 L50 7 L30 7 Z M30 10 L32 10 L32 30 L30 30 Z" fill="black"/>'
                '<ellipse cx="45" cy="30" rx="12" ry="6" fill="black"/></svg>')
    for mx in (2.5, 8.0, 100.0):
        opts = DrawingImport(fill_centerlines=True, fill_centerline_max=mx)
        svg_fills.append({"svg": fill_svg, "max": mx, "result": dump_import(import_svg(fill_svg.encode(), "f.svg", opts, 0.05))})
    (d / "cases.json").write_text(json.dumps({"imports": cases, "drawings": drawings, "svg_fills": svg_fills}, ensure_ascii=False,
                                             allow_nan=False), encoding="utf-8")
    print(f"raster: {len(cases)} imports, {len(drawings)} drawings")


def outline_font_case() -> None:
    from handwriter.glyphs.outline_provider import OutlineGlyphProvider
    from handwriter.pipeline import GenerationRefused

    fonts = {}
    texts = ["Съешь же ещё этих мягких французских булок", "fi fl ff ffi Th", "Привет, мир! 123", "ёЁйЙщ—«»…",
             "Hello World", "a", "абв́где"]
    for fname in ("BadScript-Regular.ttf", "MarckScript-Regular.ttf"):
        prov = OutlineGlyphProvider.from_path(TEST_FONTS / fname)
        d = prov.data
        info = prov.info()
        sample = sorted({ch for t in texts for ch in t} | set("абвгдеёжзийклмнопрстуфхцчшщъыьэюяABCDEFGHIJabcdefghij0123456789"))
        contour_names = [d.cmap[ord(c)] for c in "абвгдйщЖQgfx@&8" if ord(c) in d.cmap][:12]
        fonts[fname] = {
            "name": prov.name, "upem": d.upem, "order": d.order, "cmap": [[k, v] for k, v in sorted(d.cmap.items())],
            "metrics": {"x_height": d.metrics.x_height, "cap_height": d.metrics.cap_height, "ascent": d.metrics.ascent,
                        "descent": d.metrics.descent, "x_height_source": d.metrics.x_height_source},
            "analysis": d.analysis, "chars": info.chars, "variants": info.variants,
            "names_for_char": {ch: prov.glyph_names_for_char(ch) for ch in sample},
            "variant_pool": {ch: prov.variant_pool(ch) for ch in sample},
            "space_advance": prov.space_advance(),
            "advances": {n: prov.advance(n) for n in d.order[:80]},
            "shape": [[t, [[g.name, g.cluster, g.advance, g.x_offset, g.y_offset] for g in prov.shape(t)]] for t in texts],
            "contours": {n: d.contours(n) for n in contour_names},
        }
    cases = []
    for name, fname, mut in [
        ("bad_default", "BadScript-Regular.ttf", {}),
        ("bad_plain", "BadScript-Regular.ttf", {"randomness__enabled": False, "connections__enabled": False}),
        ("bad_variants_seed", "BadScript-Regular.ttf", {"randomness__seed": 99, "typography__size_mm": 4.0}),
        ("marck_default", "MarckScript-Regular.ttf", {}),
        ("marck_lines", "MarckScript-Regular.ttf", {"sheet__line_pitch": 8.0, "typography__size_mm": 2.5,
                                                    "outline__px_per_em": 900.0, "outline__smooth": 0.06}),
    ]:
        s = Settings()
        s.printer.travel = Travel(x_min=-2, x_max=200, y_min=-2, y_max=230)
        s.font, s.mode = str(TEST_FONTS / fname), "outlines"
        for path, v in mut.items():
            obj = s
            parts = path.split("__")
            for k in parts[:-1]:
                obj = getattr(obj, k)
            setattr(obj, parts[-1], v)
        s = Settings.model_validate(s.model_dump())
        c = compose(s)
        entry = {"name": name, "font": fname, "settings": s.model_dump(mode="json"), "errors": c.errors,
                 "warnings": c.warnings}
        if c.layout is not None:
            entry["glyphs"] = [[g.word, g.letter, g.char, g.glyph, g.x, g.y, g.advance] for g in c.layout.glyphs]
            entry["strokes"] = [st.points for st in c.layout.strokes]
        try:
            entry["gcode"] = make_gcode(c)
        except GenerationRefused as e:
            entry["refused"] = e.errors
        cases.append(entry)
    d = OUT / "outline_fonts"
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    (d / "cases.json").write_text(json.dumps({"fonts": fonts, "cases": cases}, ensure_ascii=False, allow_nan=False),
                                  encoding="utf-8")
    print(f"outline_fonts: {len(fonts)} fonts, {len(cases)} texts")


def _dxf_bytes(doc) -> bytes:
    fd, name = tempfile.mkstemp(suffix=".dxf")
    os.close(fd)
    doc.saveas(name)
    data = Path(name).read_bytes()
    os.unlink(name)
    return data


def dxf_inputs() -> dict:
    import math

    out = {"simple.dxf": dxf_simple()}

    doc = ezdxf.new("R2018", units=4)
    msp = doc.modelspace()
    for i, (s, e) in enumerate([(0, 90), (30, 300), (300, 30), (-45, 45), (10, 370), (0, 360), (90, 90.0000001), (720, 45)]):
        msp.add_arc((i * 30, 0), 5 + i, s, e)
    msp.add_arc((0, 40), 7, 20, 160, dxfattribs={"extrusion": (0, 0, -1)})
    msp.add_arc((30, 40), 7, 20, 160, dxfattribs={"extrusion": (0.3, 0.2, 0.9)})
    msp.add_circle((60, 40), 6)
    msp.add_circle((90, 40), 6, dxfattribs={"extrusion": (0, 0, -1)})
    msp.add_circle((120, 40), 1e-13)
    msp.add_circle((150, 40), -4)
    msp.add_ellipse((0, 80), (10, 3), 0.4)
    msp.add_ellipse((30, 80), (3, 10), 0.6, 0.5, 4.0)
    msp.add_ellipse((60, 80), (-8, 5), 0.25, 5.0, 1.0)
    msp.add_ellipse((90, 80), (8, 0, 0), 0.5, 0, math.pi, dxfattribs={"extrusion": (0, 0, -1)})
    msp.add_line((0, -20, 0), (50, -25, 3))
    msp.add_line((5, -30), (5, -30))
    out["curves.dxf"] = _dxf_bytes(doc)

    doc = ezdxf.new("R2018", units=1)
    msp = doc.modelspace()
    msp.add_lwpolyline([(0, 0, 0, 0, 0.5), (10, 0, 0, 0, -1), (20, 5, 0, 0, 0), (25, 15, 0, 0, 0.3)], format="xyseb")
    msp.add_lwpolyline([(0, 20, 0, 0, 1), (10, 20, 0, 0, 0), (10, 30, 0, 0, -0.4)], format="xyseb", close=True)
    msp.add_lwpolyline([(30, 0), (40, 0), (40, 10)], dxfattribs={"const_width": 0.5})
    msp.add_lwpolyline([(30, 20, 0.2, 0.8, 0), (40, 20, 1.5, 0.1, 0), (45, 30, 0, 0, 0)], format="xyseb")
    msp.add_lwpolyline([(50, 0, 0, 0, 0.7), (60, 0, 0, 0, 0), (60, 10, 0, 0, 0)], format="xyseb",
                       dxfattribs={"elevation": 2.0, "extrusion": (0, 0, -1)})
    msp.add_lwpolyline([(70, 0), (80, 5), (70, 0.0000000001), (70, 0)], close=True)
    msp.add_lwpolyline([(90, 0)])
    msp.add_polyline2d([(0, 50, 0, 0, 0.4), (10, 50, 0, 0, 0), (10, 60, 0, 0, -0.9)], format="xyseb", close=True)
    msp.add_polyline2d([(20, 50), (30, 55), (35, 50)], dxfattribs={"elevation": (0, 0, 1.5)})
    msp.add_polyline3d([(40, 50, 0), (50, 55, 5), (55, 50, -2)], close=True)
    msp.add_polymesh((3, 3))
    out["polylines.dxf"] = _dxf_bytes(doc)

    doc = ezdxf.new("R2018", units=4)
    msp = doc.modelspace()
    msp.add_open_spline([(0, 0), (10, 10), (20, 0), (30, 10), (40, 0)], degree=3)
    msp.add_open_spline([(0, 20), (10, 30), (20, 20), (30, 30)], degree=2)
    msp.add_rational_spline([(0, 40), (10, 50), (20, 40), (30, 50)], [1, 2, 0.5, 1], degree=3)
    sp = msp.add_open_spline([(50, 0), (60, 10), (70, 0), (60, -10), (50, 0)], degree=3)
    sp.knots = [float(k) for k in range(9)]
    sp = msp.add_open_spline([(50, 30), (55, 40), (65, 35), (70, 45), (75, 30)], degree=3)
    sp.knots = [k * 2.5 + 1.0 for k in sp.knots]
    sp = msp.add_open_spline([(80, 0), (85, 10), (95, 5), (100, 15)], degree=3)
    sp.knots = [0.0, 0.0, 0.0, 0.0, 0.33333333333, 0.66666666667, 1.0, 1.0][:len(sp.knots)]
    msp.add_spline([(x, 60 + 5 * math.sin(x / 7)) for x in range(0, 100, 5)])
    sp = msp.add_spline([(x, 80 + 5 * math.cos(x / 9)) for x in range(0, 110, 5)])
    sp.dxf.start_tangent, sp.dxf.end_tangent = (1, 1, 0), (1, -1, 0)
    out["splines.dxf"] = _dxf_bytes(doc)

    doc = ezdxf.new("R2018", units=4)
    msp = doc.modelspace()
    msp.add_spline([(0, 0), (10, 10), (20, 0), (30, 5)])
    sp = msp.add_spline([(0, 20), (10, 30), (20, 25)])
    sp.dxf.start_tangent, sp.dxf.end_tangent = (0, 1, 0), (1, 0, 0)
    out["approx_fit_splines.dxf"] = _dxf_bytes(doc)

    doc = ezdxf.new("R2018", units=4, setup=True)
    msp = doc.modelspace()
    doc.layers.add("RED", lineweight=50)
    doc.layers.add("OFF").off()
    doc.layers.add("FROZEN").freeze()
    doc.layers.add("dash", linetype="DASHED")
    blk = doc.blocks.new("PART", base_point=(5, 5))
    blk.add_line((0, 0), (10, 0))
    blk.add_line((0, 0), (0, 10), dxfattribs={"layer": "RED", "lineweight": -2, "linetype": "BYBLOCK"})
    blk.add_circle((5, 5), 3)
    blk.add_arc((5, 5), 4, 0, 120)
    blk.add_ellipse((5, 5), (3, 1), 0.5, 0, 3)
    blk.add_lwpolyline([(0, 10, 0, 0, 0.5), (10, 10, 0, 0, 0), (10, 0, 0, 0, 0)], format="xyseb")
    blk.add_lwpolyline([(0, 12), (10, 12), (10, 14)], dxfattribs={"const_width": 0.4})
    blk.add_polyline2d([(0, 15, 0, 0, -0.5), (5, 18, 0, 0, 0), (8, 15, 0, 0, 0)], format="xyseb")
    blk.add_polyline3d([(0, 0, 0), (3, 3, 3), (6, 0, 1)])
    blk.add_spline([(0, 0), (2, 3), (4, 1), (6, 4), (8, 0), (10, 2), (12, 1), (14, 3), (16, 0), (18, 2),
                    (20, 1), (22, 3), (24, 0), (26, 2), (28, 1), (30, 3), (32, 0), (34, 2)])
    blk.add_text("BLOCKTEXT", dxfattribs={"insert": (1, 1), "rotation": 30})
    blk.add_mtext("M\\PTEXT", dxfattribs={"insert": (2, 2)})
    blk.add_point((1, 1))
    blk.add_attdef("TAG", (0, 0))
    blk.add_solid([(0, 0), (1, 0), (0, 1), (1, 1)])
    inner = doc.blocks.new("INNER")
    inner.add_line((0, 0), (3, 3))
    inner.add_circle((0, 0), 1)
    inner.add_lwpolyline([(0, 0, 0, 0, 1), (2, 0, 0, 0, 0)], format="xyseb")
    blk.add_blockref("INNER", (2, 2), dxfattribs={"rotation": 45, "xscale": 2, "yscale": 0.5})
    blk.add_blockref("INNER", (4, 4), dxfattribs={"rotation": 15})
    msp.add_blockref("PART", (0, 0))
    msp.add_blockref("PART", (50, 0), dxfattribs={"xscale": 2, "yscale": 2, "rotation": 30, "lineweight": 70,
                                                   "linetype": "DASHED"})
    msp.add_blockref("PART", (100, 0), dxfattribs={"xscale": 2, "yscale": 0.5, "layer": "RED"})
    msp.add_blockref("PART", (150, 0), dxfattribs={"xscale": -1, "yscale": 1, "rotation": 10})
    msp.add_blockref("PART", (0, 60), dxfattribs={"xscale": 1.5, "yscale": 1.5, "zscale": 1.5,
                                                   "extrusion": (0, 0, -1)})
    msp.add_blockref("PART", (50, 60), dxfattribs={"layer": "OFF"})
    msp.add_blockref("PART", (100, 60), dxfattribs={"layer": "FROZEN"})
    msp.add_blockref("NOPE", (0, 0))
    m = msp.add_blockref("INNER", (0, 120))
    m.grid(size=(2, 3), spacing=(10, 10))
    ins = msp.add_blockref("INNER", (150, 120), dxfattribs={"rotation": 90, "xscale": 3, "yscale": 1})
    ins.add_attrib("TAG", "VALUE", (150, 120))
    msp.add_line((0, 200), (100, 200), dxfattribs={"layer": "dash"})
    msp.add_line((0, 210), (100, 210), dxfattribs={"linetype": "DASHDOT", "ltscale": 0.5})
    msp.add_line((0, 220), (100, 220), dxfattribs={"linetype": "UNKNOWN_LT"})
    msp.add_line((0, 230), (100, 230), dxfattribs={"layer": "OFF"})
    msp.add_line((0, 240), (100, 240), dxfattribs={"invisible": 1})
    msp.add_line((0, 250), (100, 250), dxfattribs={"lineweight": -3})
    msp.add_line((0, 260), (100, 260), dxfattribs={"lineweight": -2})
    msp.add_line((0, 270), (100, 270), dxfattribs={"layer": "NEWLAYER", "linetype": "BORDER"})
    doc.header["$LTSCALE"] = 2.0
    out["blocks.dxf"] = _dxf_bytes(doc)

    doc = ezdxf.new("R2018", units=4, setup=True)
    msp = doc.modelspace()
    msp.add_text("Hello  world", dxfattribs={"insert": (0, 0), "height": 2.5})
    msp.add_text("%%c10 %%d %%p", dxfattribs={"insert": (0, 10)})
    msp.add_text("ocs", dxfattribs={"insert": (5, 5), "extrusion": (0, 0, -1)})
    msp.add_text("   ", dxfattribs={"insert": (0, 20)})
    msp.add_text("Жук \\U+0416", dxfattribs={"insert": (0, 25)})
    msp.add_mtext("{\\fArial|b1;Bold} plain\\Pnext line \\S1/2; %%d ^I tab \\\\ back {\\H2x;big}",
                  dxfattribs={"insert": (0, 30)})
    msp.add_mtext("A" * 300 + " end", dxfattribs={"insert": (0, 40)})
    msp.add_mtext("\\Lunder\\l \\Ttrack;", dxfattribs={"insert": (0, 50), "rotation": 30})
    d = msp.add_linear_dim(base=(0, 70), p1=(0, 60), p2=(40, 60))
    d.render()
    d = msp.add_aligned_dim(p1=(50, 60), p2=(80, 80), distance=5)
    d.render()
    d = msp.add_radius_dim(center=(100, 60), radius=10, angle=45)
    d.render()
    d = msp.add_angular_dim_2l(base=(130, 70), line1=((120, 60), (140, 60)), line2=((120, 60), (135, 75)))
    d.render()
    blk = doc.blocks.new("WITHDIM")
    d = blk.add_linear_dim(base=(0, 10), p1=(0, 0), p2=(20, 0))
    d.render()
    msp.add_blockref("WITHDIM", (0, 100), dxfattribs={"xscale": 2, "yscale": 2, "rotation": 20})
    msp.add_blockref("WITHDIM", (50, 100), dxfattribs={"xscale": 2, "yscale": 1})
    msp.add_leader([(0, 130), (10, 140), (20, 140)])
    msp.add_leader([(30, 130), (40, 140)], dimstyle="EZDXF")
    msp.add_point((0, 0))
    msp.add_xline((0, 0), (1, 1))
    msp.add_3dface([(0, 0, 0), (1, 0, 0), (1, 1, 0)])
    out["annotations.dxf"] = _dxf_bytes(doc)

    doc = ezdxf.new("R2018", units=4)
    msp = doc.modelspace()
    msp.add_solid([(0, 0), (2, 0), (0, 1.5), (2, 1.5)])
    msp.add_solid([(5, 0), (7, 0), (6, 1)])
    msp.add_trace([(10, 0), (12, 0), (10, 1), (12, 1)])
    msp.add_solid([(0, 10), (20, 10), (0, 30), (20, 30)])
    h = msp.add_hatch()
    h.paths.add_polyline_path([(20, 0, 0), (22, 0, 0.5), (22, 2, 0), (20, 2, 0)], is_closed=True)
    h = msp.add_hatch()
    ep = h.paths.add_edge_path()
    ep.add_line((30, 0), (33, 0))
    ep.add_arc((33, 1), 1, -90, 90)
    ep.add_line((33, 2), (30, 2))
    ep.add_ellipse((30, 1), (0, 1), 0.5, 90, 270)
    h = msp.add_hatch()
    ep = h.paths.add_edge_path()
    ep.add_spline(control_points=[(40, 0), (41, 2), (43, 2), (44, 0)], knot_values=[0, 0, 0, 0, 1, 1, 1, 1], degree=3)
    ep.add_line((44, 0), (40, 0))
    h = msp.add_hatch()
    h.set_pattern_fill("ANSI31")
    h.paths.add_polyline_path([(50, 0), (55, 0), (55, 5)], is_closed=True)
    h = msp.add_hatch(dxfattribs={"extrusion": (0, 0, -1), "elevation": (0, 0, 3)})
    h.paths.add_polyline_path([(60, 0), (61, 0), (61, 1), (60, 1)], is_closed=True)
    blk = doc.blocks.new("FILLS")
    blk.add_solid([(0, 0), (1, 0), (0, 1)])
    hh = blk.add_hatch()
    hh.paths.add_polyline_path([(0, 0), (1, 0), (1, 1)], is_closed=True)
    msp.add_blockref("FILLS", (70, 0), dxfattribs={"rotation": 30})
    msp.add_blockref("FILLS", (80, 0), dxfattribs={"xscale": 2, "yscale": 1})
    idef = doc.add_image_def("img.png", (100, 100))
    msp.add_image(idef, (90, 0), (10, 10))
    out["fills.dxf"] = _dxf_bytes(doc)

    doc = ezdxf.new("R12")
    msp = doc.modelspace()
    msp.add_line((0, 0), (10, 0))
    msp.add_arc((5, 5), 3, 0, 270)
    msp.add_polyline2d([(0, 10, 0, 0, 0.5), (10, 10, 0, 0, 0)], format="xyseb")
    msp.add_text("R12 text", dxfattribs={"insert": (0, 20)})
    blk = doc.blocks.new("B12")
    blk.add_circle((0, 0), 2)
    msp.add_blockref("B12", (20, 20), dxfattribs={"xscale": 2, "yscale": 3})
    out["r12.dxf"] = _dxf_bytes(doc)

    for units in (0, 6, 16, 2):
        doc = ezdxf.new("R2010", units=units)
        doc.modelspace().add_line((0, 0), (1, 2))
        out[f"units_{units}.dxf"] = _dxf_bytes(doc)

    doc = ezdxf.new("R2000", units=4)
    doc.header["$DWGCODEPAGE"] = "ANSI_1251"
    doc.layers.add("Слой")
    msp = doc.modelspace()
    msp.add_line((0, 0), (5, 5), dxfattribs={"layer": "Слой"})
    msp.add_text("Привет", dxfattribs={"insert": (1, 1)})
    msp.add_mtext("Мир\\Pвторая", dxfattribs={"insert": (2, 2)})
    out["cp1251.dxf"] = _dxf_bytes(doc)

    doc = ezdxf.new("R2018", units=4)
    psp = doc.layout("Layout1")
    psp.add_line((0, 0), (100, 50))
    psp.add_circle((50, 50), 20)
    out["paperspace.dxf"] = _dxf_bytes(doc)

    out["broken.dxf"] = b"0\nSECTION\n2\nENTITIES\n0\nLINE\n10\nabc\n20\n1\n0\nENDSEC\n0\nEOF\n"
    out["empty.dxf"] = b""
    out["garbage.dxf"] = b"\x00\x01binary\xff"

    import random

    rng = random.Random(23)

    def rnd_ext():
        return rng.choice([(0, 0, 1), (0, 0, -1), (rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(0.2, 1)),
                           (0.01, 0.005, 1)])

    def rnd_entities(lay):
        for _ in range(rng.randint(4, 9)):
            kind = rng.choice(["line", "arc", "circle", "ellipse", "lw", "poly2d", "spline", "text", "solid", "hatch",
                               "mtext", "fit", "edgehatch", "dim", "leader", "poly3d"])
            c = (rng.uniform(-50, 50), rng.uniform(-50, 50))
            at = {"extrusion": rnd_ext()} if rng.random() < 0.3 else {}
            if kind == "line":
                lay.add_line(c, (rng.uniform(-50, 50), rng.uniform(-50, 50), rng.uniform(-3, 3)))
            elif kind == "arc":
                lay.add_arc(c, rng.uniform(0.5, 20), rng.uniform(-400, 400), rng.uniform(-400, 400), dxfattribs=at)
            elif kind == "circle":
                lay.add_circle(c, rng.uniform(0.5, 20), dxfattribs=at)
            elif kind == "ellipse":
                lay.add_ellipse(c, (rng.uniform(-10, 10), rng.uniform(-10, 10), 0), rng.uniform(0.05, 1),
                                rng.uniform(-7, 7), rng.uniform(-7, 7), dxfattribs=at)
            elif kind == "lw":
                pts = [(rng.uniform(-30, 30), rng.uniform(-30, 30), 0, 0, rng.choice([0, 0, rng.uniform(-2, 2)]))
                       for _ in range(rng.randint(2, 6))]
                lay.add_lwpolyline(pts, format="xyseb", close=rng.random() < 0.4, dxfattribs=at)
            elif kind == "poly2d":
                pts = [(rng.uniform(-30, 30), rng.uniform(-30, 30), 0, 0, rng.choice([0, rng.uniform(-1, 1)]))
                       for _ in range(rng.randint(2, 5))]
                lay.add_polyline2d(pts, format="xyseb", close=rng.random() < 0.4, dxfattribs=at)
            elif kind == "spline":
                pts = [(rng.uniform(-30, 30), rng.uniform(-30, 30)) for _ in range(rng.randint(4, 7))]
                sp = lay.add_open_spline(pts, degree=rng.choice([2, 3, 3]))
                if rng.random() < 0.3:
                    sp.weights = [rng.uniform(0.5, 2) for _ in pts]
            elif kind == "text":
                lay.add_text(f"T{rng.randint(0, 99)}", dxfattribs={"insert": c, "rotation": rng.uniform(0, 360), **at})
            elif kind == "solid":
                lay.add_solid([c, (c[0] + 1, c[1]), (c[0], c[1] + 0.7), (c[0] + 1.2, c[1] + 1)], dxfattribs=at)
            elif kind == "mtext":
                lay.add_mtext(f"M{rng.randint(0, 9)}\\P{{\\H2;x}}", dxfattribs={"insert": c, "rotation": rng.uniform(0, 90), **at})
            elif kind == "fit":
                pts = [(c[0] + i * 2, c[1] + rng.uniform(-3, 3)) for i in range(rng.choice([18, 19, 25]))]
                sp = lay.add_spline(pts)
                if rng.random() < 0.5:
                    sp.dxf.start_tangent, sp.dxf.end_tangent = (1, rng.uniform(-1, 1), 0), (1, rng.uniform(-1, 1), 0)
            elif kind == "edgehatch":
                h = lay.add_hatch(dxfattribs=at)
                ep = h.paths.add_edge_path()
                ep.add_line(c, (c[0] + 2, c[1]))
                ep.add_arc((c[0] + 2, c[1] + 1), 1, -90, 90, ccw=rng.random() < 0.7)
                ep.add_ellipse((c[0], c[1] + 1), (0, 1), rng.uniform(0.3, 1), 90, 270)
            elif kind == "dim":
                lay.add_linear_dim(base=(c[0], c[1] + 5), p1=c, p2=(c[0] + rng.uniform(5, 20), c[1]),
                                   angle=rng.choice([0, 0, 30])).render()
            elif kind == "leader":
                lay.add_leader([c, (c[0] + 5, c[1] + 4), (c[0] + 9, c[1] + 4)])
            elif kind == "poly3d":
                lay.add_polyline3d([(c[0], c[1], 0), (c[0] + 3, c[1] + 1, 2), (c[0] + 5, c[1] - 2, -1)],
                                   close=rng.random() < 0.5)
            else:
                h = lay.add_hatch(dxfattribs=at)
                h.paths.add_polyline_path([(c[0], c[1], rng.choice([0, 0.4])), (c[0] + 1.5, c[1], 0),
                                           (c[0] + 1.5, c[1] + 1, 0)], is_closed=True)

    for n in range(24):
        doc = ezdxf.new("R2018", units=4, setup=n % 2 == 0)
        msp = doc.modelspace()
        rnd_entities(msp)
        for b in range(3):
            blk = doc.blocks.new(f"B{b}", base_point=(rng.uniform(-5, 5), rng.uniform(-5, 5)))
            rnd_entities(blk)
            if b > 0 and rng.random() < 0.7:
                blk.add_blockref(f"B{b - 1}", (rng.uniform(-9, 9), rng.uniform(-9, 9)),
                                 dxfattribs={"xscale": rng.choice([1, 2, -1, 0.5]), "yscale": rng.choice([1, 1, 3, -2]),
                                             "rotation": rng.uniform(0, 360)})
        for _ in range(4):
            sx = rng.choice([1, 1.5, -1, 2])
            sy = rng.choice([sx, sx, 0.5, -sx])
            msp.add_blockref(f"B{rng.randint(0, 2)}", (rng.uniform(-80, 80), rng.uniform(-80, 80)),
                             dxfattribs={"xscale": sx, "yscale": sy, "rotation": rng.uniform(0, 360),
                                         "extrusion": rnd_ext() if rng.random() < 0.3 else (0, 0, 1)})
        out[f"random_{n}.dxf"] = _dxf_bytes(doc)
    return out


def dxf_case() -> None:
    from handwriter.drawing.dxf_import import import_dxf
    from handwriter.settings import DrawingImport

    d = OUT / "dxf"
    shutil.rmtree(d, ignore_errors=True)
    (d / "input").mkdir(parents=True)
    imports = []
    for fname, data in dxf_inputs().items():
        (d / "input" / fname).write_bytes(data)
        variants = (("auto", 0.05, False), ("auto", 0.5, True), ("mm", 0.05, True), ("in", 0.2, False))
        if fname.startswith("random_"):
            variants = (("auto", 0.1, False), ("in", 0.5, True))
        for units, tol, fills in variants:
            imp = DrawingImport(units=units, fill_centerlines=fills)
            imports.append({"file": fname, "units": units, "tol": tol, "fills": fills,
                            "result": dump_import(import_dxf(data, fname, imp, tol))})
    (d / "cases.json").write_text(json.dumps({"imports": imports}, ensure_ascii=False), encoding="utf-8")
    print(f"dxf: {len(imports)} imports")


def pdf_inputs() -> dict:
    import pymupdf

    out = {}

    def page_shapes(page, k=0):
        page.draw_line((50, 50), (300, 80), color=(0, 0, 0), width=0.7)
        page.draw_line((50, 90), (300, 90), color=(1, 0, 0), width=0.3, dashes="[6 2] 0")
        page.draw_line((50, 100), (300, 100), color=(0, 0, 1), width=1.5, dashes="[3 1.5 0.5 1.5] 1")
        page.draw_bezier((50, 120), (100, 200), (200, 50), (300, 150), color=(0, 0.5, 0), width=0.5)
        page.draw_rect(pymupdf.Rect(60, 200, 160, 260), color=(0, 0, 0), width=0.4)
        page.draw_rect(pymupdf.Rect(170, 200, 190, 215), color=None, fill=(0, 0, 0))
        page.draw_rect(pymupdf.Rect(200, 200, 260, 260), color=(0, 0, 0), fill=(0.8, 0.2, 0.2), width=1)
        page.draw_rect(pymupdf.Rect(0, 0, 595, 842), color=None, fill=(1, 1, 1), overlay=False)
        page.draw_quad(pymupdf.Quad((300, 300), (380, 310), (290, 380), (370, 395)), color=(0, 0, 0), width=0.6)
        page.draw_polyline([(50, 400), (90, 430), (130, 400), (170, 440)], color=(0, 0, 0), width=0.5)
        page.draw_polyline([(200, 400), (240, 430), (280, 400)], color=(0, 0, 0), fill=(0, 0, 0), closePath=True)
        page.draw_circle((400, 500), 30, color=(0, 0, 0), width=0.5)
        page.draw_circle((480, 500), 4, color=None, fill=(0, 0, 0))
        page.draw_oval(pymupdf.Rect(300, 550, 450, 620), color=(0.5, 0.5, 0.5), width=2)
        page.draw_sector((150, 650), (200, 650), 120, color=(0, 0, 0), fill=(0.1, 0.1, 0.1))
        page.draw_line((50, 700), (300, 700), color=(1, 1, 1), width=1)
        page.draw_line((50, 710 + k), (300, 712), color=(0.2, 0.2, 0.2), width=0)
        page.draw_squiggle((50, 740), (300, 760), color=(0, 0, 0))
        page.draw_zigzag((50, 780), (300, 790), color=(0, 0, 0))

    doc = pymupdf.open()
    page_shapes(doc.new_page())
    out["shapes.pdf"] = doc.tobytes()

    doc = pymupdf.open()
    page = doc.new_page(width=420, height=297)
    page.insert_text((40, 60), "Hello, Чертёж!", fontsize=12, fontname="helv")
    page.insert_text((40, 90), "Times  Roman   text", fontsize=10, fontname="tiro")
    page.insert_text((40, 120), "Courier", fontsize=14, fontname="cour")
    page.insert_text((200, 200), "Rotated", fontsize=11, fontname="helv", rotate=90)
    page.insert_text((300, 150), "Upside", fontsize=9, fontname="helv", rotate=180)
    page.insert_text((40, 160), "x2 small", fontsize=6, fontname="tiro")
    page.insert_text((40, 180), "tab\there  and  nbsp", fontsize=8, fontname="helv")
    page.insert_textbox(pymupdf.Rect(40, 200, 200, 280), "Box text with several words that wrap around", fontsize=9)
    page.draw_line((40, 62), (150, 62), color=(0, 0, 0), width=0.3)
    out["text.pdf"] = doc.tobytes()

    doc = pymupdf.open()
    for rot in (0, 90, 180, 270):
        page = doc.new_page(width=300, height=200)
        page.draw_line((20, 20), (200, 60), color=(0, 0, 0), width=0.5)
        page.draw_rect(pymupdf.Rect(30, 80, 120, 150), color=(0, 0, 0), width=0.5)
        page.insert_text((40, 180), f"rot {rot}", fontsize=10)
        page.set_rotation(rot)
    out["rotated.pdf"] = doc.tobytes()

    doc = pymupdf.open()
    page = doc.new_page()
    ocg1 = doc.add_ocg("Contour", on=True)
    ocg2 = doc.add_ocg("Hidden", on=False)
    page.draw_line((50, 50), (300, 50), color=(0, 0, 0), oc=ocg1)
    page.draw_line((50, 60), (300, 60), color=(0, 0, 0), oc=ocg2)
    page.draw_rect(pymupdf.Rect(50, 70, 90, 110), color=(0, 0, 0), oc=ocg1)
    page.draw_line((50, 120), (300, 120), color=(0, 0, 0))
    out["layers.pdf"] = doc.tobytes()

    doc = pymupdf.open()
    page = doc.new_page(width=200, height=200)
    import zlib, struct
    def png(w, h):
        raw = b"".join(b"\x00" + bytes([(x * 7 + y * 3) % 256 for x in range(w)]) for y in range(h))
        def chunk(t, d):
            return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
        return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 0, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
    page.insert_image(pymupdf.Rect(20, 20, 120, 90), stream=png(16, 12))
    page.draw_line((20, 150), (180, 150), color=(0, 0, 0))
    out["image.pdf"] = doc.tobytes()

    doc = pymupdf.open()
    page = doc.new_page()
    page.set_cropbox(pymupdf.Rect(100, 100, 400, 500))
    page.draw_line((120, 120), (380, 480), color=(0, 0, 0), width=0.5)
    page.draw_line((0, 0), (595, 842), color=(0, 0, 0), width=0.5)
    page.insert_text((150, 300), "cropped", fontsize=10)
    page.insert_text((10, 30), "outside", fontsize=10)
    out["cropbox.pdf"] = doc.tobytes()

    doc = pymupdf.open()
    page = doc.new_page()
    sh = page.new_shape()
    sh.draw_line((100, 100), (200, 100))
    sh.draw_line((200, 100), (200, 200))
    sh.draw_line((200, 200), (100, 200))
    sh.draw_line((100, 200), (100, 100))
    sh.finish(color=(0, 0, 0), width=0.8, closePath=False)
    sh.draw_line((250, 100), (350, 120))
    sh.draw_line((350, 120), (330, 200))
    sh.draw_line((330, 200), (240, 190))
    sh.draw_line((240, 190), (250, 100))
    sh.finish(color=(0, 0, 0), fill=(0.3, 0.3, 0.3))
    sh.draw_curve((100, 300), (150, 250), (200, 300))
    sh.draw_curve((200, 300), (250, 350), (300, 300))
    sh.finish(color=(0, 0, 0), closePath=True)
    sh.draw_rect(pymupdf.Rect(400, 400, 402, 401))
    sh.finish(color=None, fill=(0, 0, 0))
    sh.draw_line((50, 500), (51, 500.0000001))
    sh.finish(color=(0, 0, 0), width=3, lineCap=1)
    sh.commit()
    page2 = doc.new_page(width=100, height=100)
    page2.draw_line((10, 10), (90, 90), color=(0, 0, 0))
    out["shape_paths.pdf"] = doc.tobytes()

    doc = pymupdf.open()
    doc.new_page()
    p = doc.new_page()
    page_shapes(p, 3)
    p.insert_text((100, 600), "second page", fontsize=12)
    out["two_pages.pdf"] = doc.tobytes()

    out["empty.pdf"] = b""
    out["garbage.pdf"] = b"garbage"
    out["noobj.pdf"] = b"%PDF-1.4\n%%EOF"
    out["nopages.pdf"] = b"%PDF-1.4\n1 0 obj <<>> endobj\ntrailer <</Root 1 0 R>>\n%%EOF"
    return out


PDF_RESOURCES = ["test_5054.pdf", "test2238.pdf", "test_4564.pdf", "test_4043.pdf", "bug1971.pdf",
                 "test_4415.pdf", "test-3591.pdf", "test_4928.pdf", "symbol-list.pdf", "test_4936.pdf",
                 "test_4712_a.pdf", "test-2812.pdf", "test_2969.pdf", "small-table.pdf", "test_2730.pdf",
                 "test_4004.pdf", "widgettest.pdf", "type3font.pdf", "test-3150.pdf", "test_2742.pdf",
                 "test_3569.pdf", "text-find-ligatures.pdf", "test-linebreaks.pdf", "github_sample.pdf",
                 "img-regular.pdf", "img-transparent.pdf", "has-bad-fonts.pdf", "quad-calc-0.pdf",
                 "test-2462.pdf", "test_3448.pdf", "test-3143.pdf"]


def dump_pdf_case(d: Path, inputs: dict) -> list:
    from handwriter.drawing.pdf_import import import_pdf
    from handwriter.settings import DrawingImport

    imports = []
    for fname, data in inputs.items():
        (d / "input" / fname).write_bytes(data)
        for page, tol, fills in ((1, 0.05, False), (2, 0.5, True)):
            imp = DrawingImport(pdf_page=page, fill_centerlines=fills)
            imports.append({"file": fname, "page": page, "tol": tol, "fills": fills,
                            "result": dump_import(import_pdf(data, fname, imp, tol))})
    return imports


def pdf_case() -> None:
    d = OUT / "pdf"
    res_dir = Path(os.environ.get("PYMUPDF_RESOURCES") or d / "input")
    resources = {name: (res_dir / name).read_bytes() for name in PDF_RESOURCES if (res_dir / name).exists()}
    shutil.rmtree(d, ignore_errors=True)
    (d / "input").mkdir(parents=True)
    inputs = pdf_inputs() | resources
    imports = dump_pdf_case(d, inputs)
    (d / "cases.json").write_text(json.dumps({"imports": imports}, ensure_ascii=False), encoding="utf-8")
    print(f"pdf: {len(imports)} imports")


def main() -> None:
    os.environ["HANDWRITER_HOME"] = tempfile.mkdtemp(prefix="hw-golden-")
    OUT.mkdir(parents=True, exist_ok=True)
    numeric_case()
    conformance_case()
    svg_fonts_case()
    text_case()
    drawing_import_case()
    placement_case()
    plan_case()
    split_geometry_case()
    split_case()
    drawing_pipeline_case()
    calibration_case()
    skeleton_case()
    raster_case()
    outline_font_case()
    dxf_case()
    pdf_case()
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
