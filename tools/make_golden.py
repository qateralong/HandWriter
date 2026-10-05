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


def main() -> None:
    os.environ["HANDWRITER_HOME"] = tempfile.mkdtemp(prefix="hw-golden-")
    OUT.mkdir(parents=True, exist_ok=True)
    numeric_case()
    conformance_case()
    svg_fonts_case()
    text_case()
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
