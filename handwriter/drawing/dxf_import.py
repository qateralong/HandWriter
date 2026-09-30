from __future__ import annotations

import io
import math

from .model import UNIT_MM, UNIT_NAMES, DPath, ImportResult, TextMark
from .svg_import import add_filled

INSUNITS_MM = {1: 25.4, 2: 304.8, 3: 1609344.0, 4: 1.0, 5: 10.0, 6: 1000.0, 7: 1e6, 8: 25.4e-6, 9: 0.0254,
               10: 914.4, 11: 1e-7, 12: 1e-6, 13: 1e-3, 14: 100.0, 15: 1e4, 16: 1e5}
INSUNITS_NAME = {1: "дюймы", 2: "футы", 3: "мили", 4: "мм", 5: "см", 6: "м", 7: "км", 8: "микродюймы",
                 9: "милы", 10: "ярды", 11: "ангстремы", 12: "нм", 13: "мкм", 14: "дм", 15: "дам", 16: "гм"}
CURVES = {"LINE", "LWPOLYLINE", "POLYLINE", "ARC", "CIRCLE", "ELLIPSE", "SPLINE"}
EXPLODE = {"INSERT", "DIMENSION", "ARC_DIMENSION", "LARGE_RADIAL_DIMENSION", "LEADER", "MLEADER", "MULTILEADER"}
TEXTS = {"TEXT", "MTEXT", "ATTRIB"}
FILLS = {"SOLID", "TRACE"}
QUIET = {"VIEWPORT", "ATTDEF", "POINT"}
DEFAULT_LW = 0.25


class _Ctx:
    def __init__(self, doc, res: ImportResult, k: float, tol_units: float, opts):
        self.doc = doc
        self.res = res
        self.k = k
        self.tol = tol_units
        self.opts = opts
        self.ltscale = float(doc.header.get("$LTSCALE", 1.0) or 1.0)
        lwd = doc.header.get("$LWDEFAULT", 25)
        self.lw_default = (lwd / 100.0) if isinstance(lwd, (int, float)) and lwd > 0 else DEFAULT_LW
        self.skipped: dict[str, int] = {}
        self.fills_small = 0
        self.patterns: dict[str, tuple[float, ...] | None] = {}

    def skip(self, kind: str) -> None:
        self.skipped[kind] = self.skipped.get(kind, 0) + 1


def import_dxf(data: bytes, name: str, opts, tol_mm: float) -> ImportResult:
    from ezdxf import recover

    res = ImportResult(kind="dxf", name=name)
    try:
        doc, auditor = recover.read(io.BytesIO(data))
    except Exception as e:
        res.errors.append(f"DXF не читается: {e}")
        return res
    if auditor.has_errors:
        res.warnings.append(f"DXF с ошибками структуры ({len(auditor.errors)}), прочитано что удалось")

    insunits = int(doc.header.get("$INSUNITS", 0) or 0)
    if opts.units != "auto":
        k = UNIT_MM[opts.units]
        res.units, res.units_note = opts.units, "указано вручную: " + UNIT_NAMES[opts.units]
    elif insunits in INSUNITS_MM:
        k = INSUNITS_MM[insunits]
        res.units = {1: "in", 2: "ft", 4: "mm", 5: "cm", 6: "m"}.get(insunits, "mm")
        res.units_note = f"по $INSUNITS = {insunits} ({INSUNITS_NAME[insunits]})"
    else:
        k = 1.0
        res.units = "mm"
        res.units_note = f"$INSUNITS = {insunits} (единицы не заданы): считаю мм, можно указать вручную"
        res.warnings.append("В DXF не заданы единицы ($INSUNITS = 0): считаю, что чертёж в мм. "
                            "Если размер не тот, выбери единицы вручную")
    ctx = _Ctx(doc, res, k, tol_mm / k, opts)

    for layer in doc.layers:
        res.layers[layer.dxf.name] = {"count": 0, "width": _lw_mm(layer.dxf.get("lineweight", -3), ctx),
                                      "linetype": layer.dxf.get("linetype", "Continuous")}
    msp = doc.modelspace()
    entities = list(msp)
    if not entities:
        for lay in doc.layouts:
            if not lay.is_modelspace:
                entities = [e for e in lay if e.dxftype() != "VIEWPORT"]
                if entities:
                    res.warnings.append(f"Пространство модели пустое, взят лист «{lay.name}»")
                    break
    for e in entities:
        _entity(e, ctx, None)

    res.layers = {n: v for n, v in res.layers.items() if v["count"]}
    for kind, n in sorted(ctx.skipped.items()):
        what = {"HATCH": "штриховки и заливки (HATCH) не рисуются",
                "SOLID": "залитые фигуры (SOLID/TRACE) не рисуются (галочка «мелкие заливки → линии»)",
                "IMAGE": "встроенные картинки не рисуются",
                "error": "объекты, которые не удалось разобрать"}.get(kind, f"{kind} не поддерживается")
        res.warnings.append(f"{what}: {n}")
    if ctx.fills_small:
        res.warnings.append(f"Мелкие залитые фигуры превращены в центральные линии: {ctx.fills_small}")
    return res


def _lw_mm(lw, ctx: _Ctx) -> float | None:
    if lw is None:
        return None
    lw = int(lw)
    if lw >= 0:
        return lw / 100.0
    if lw == -3:
        return ctx.lw_default
    return None


def _layer_name(e, block) -> str:
    name = e.dxf.get("layer", "0")
    if name == "0" and block is not None:
        return block["layer"]
    return name


def _resolved(e, ctx: _Ctx, block) -> tuple[str, float | None, str]:
    layer = _layer_name(e, block)
    ld = ctx.res.layers.get(layer)
    lw = int(e.dxf.get("lineweight", -1))
    if lw == -1:
        width = ld["width"] if ld else ctx.lw_default
    elif lw == -2:
        width = block["width"] if block else ctx.lw_default
    else:
        width = _lw_mm(lw, ctx)
    lt = e.dxf.get("linetype", "BYLAYER")
    if lt.upper() == "BYLAYER":
        lt = ld["linetype"] if ld else "Continuous"
    elif lt.upper() == "BYBLOCK":
        lt = block["linetype"] if block else "Continuous"
    return layer, width, lt


def _pattern(name: str, ctx: _Ctx) -> tuple[float, ...] | None:
    key = name.upper()
    if key in ctx.patterns:
        return ctx.patterns[key]
    pat = None
    if key not in ("CONTINUOUS", "BYLAYER", "BYBLOCK", "") and name in ctx.doc.linetypes:
        try:
            p = tuple(abs(float(v)) for v in ctx.doc.linetypes.get(name).simplified_line_pattern())
            if len(p) >= 2 and sum(p) > 0:
                pat = p
        except Exception:
            pat = None
    ctx.patterns[key] = pat
    return pat


def _entity(e, ctx: _Ctx, block) -> None:
    t = e.dxftype()
    if e.dxf.get("invisible", 0):
        return
    layer = _layer_name(e, block)
    lay = ctx.doc.layers.get(layer) if layer in ctx.doc.layers else None
    if lay is not None and (lay.is_off() or lay.is_frozen()):
        return
    if t in EXPLODE:
        _, width, lt = _resolved(e, ctx, block)
        inner = {"layer": layer, "width": width, "linetype": lt}
        try:
            for ve in e.virtual_entities():
                _entity(ve, ctx, inner)
        except Exception:
            ctx.skip("error")
        return
    if t in TEXTS:
        _text(e, ctx)
        return
    if t in CURVES:
        _curve(e, t, ctx, block)
        return
    if t in FILLS or t == "HATCH":
        _fill(e, t, ctx, block)
        return
    if t == "IMAGE":
        ins = e.dxf.get("insert")
        if ins is not None:
            ctx.res.texts.append(TextMark(ins[0] * ctx.k, ins[1] * ctx.k, "встроенная картинка", "image"))
        ctx.skip("IMAGE")
        return
    if t not in QUIET:
        ctx.skip(t)


def _flatten(path, ctx: _Ctx) -> list[list[tuple[float, float]]]:
    out = []
    subs = path.sub_paths() if path.has_sub_paths else [path]
    for sp in subs:
        pts = [(v.x * ctx.k, v.y * ctx.k) for v in sp.flattening(ctx.tol)]
        dd = []
        for p in pts:
            if not dd or math.dist(p, dd[-1]) > 1e-9:
                dd.append(p)
        if dd:
            out.append(dd)
    return out


def _curve(e, t: str, ctx: _Ctx, block) -> None:
    from ezdxf import path as ezpath
    if t == "POLYLINE" and (e.is_polygon_mesh or e.is_poly_face_mesh):
        ctx.skip("POLYLINE (сетка)")
        return
    try:
        pieces = _flatten(ezpath.make_path(e), ctx)
    except Exception:
        ctx.skip("error")
        return
    layer, width, lt = _resolved(e, ctx, block)
    if t == "LWPOLYLINE":
        cw = e.dxf.get("const_width", 0.0) or 0.0
        try:
            vw = max((max(a, b) for _, _, a, b, _ in e.get_points("xyseb")), default=0.0)
        except Exception:
            vw = 0.0
        w_poly = max(cw, vw) * ctx.k
        if w_poly > 0:
            width = max(width or 0.0, w_poly)
    pat = _pattern(lt, ctx)
    dash = None
    if pat:
        s = ctx.ltscale * float(e.dxf.get("ltscale", 1.0) or 1.0) * ctx.k
        dash = tuple(v * s for v in pat)
    ld = ctx.res.layers.setdefault(layer, {"count": 0, "width": None, "linetype": "Continuous"})
    ld["count"] += 1
    for p in pieces:
        closed = len(p) > 2 and math.dist(p[0], p[-1]) < 1e-9
        if closed:
            p[-1] = p[0]
        ctx.res.paths.append(DPath(p, closed, width, layer, dash, 0.0))


def _fill(e, t: str, ctx: _Ctx, block) -> None:
    from ezdxf import path as ezpath
    if not ctx.opts.fill_centerlines or (t == "HATCH" and not e.dxf.get("solid_fill", 0)):
        ctx.skip("HATCH" if t == "HATCH" else "SOLID")
        return
    try:
        if t == "HATCH":
            contours = [c for p in ezpath.from_hatch(e) for c in _flatten(p, ctx)]
        else:
            v = [e.dxf.get(f"vtx{i}") for i in range(4)]
            v = [x for x in v if x is not None]
            if len(v) == 4:
                v = [v[0], v[1], v[3], v[2]]
            ocs = e.ocs()
            contours = [[(w.x * ctx.k, w.y * ctx.k) for w in (ocs.to_wcs(x) for x in v)]]
    except Exception:
        ctx.skip("error")
        return
    layer = _layer_name(e, block)
    before = len(ctx.res.paths)
    add_filled(contours, ctx.res, ctx.opts, counter=ctx, layer=layer, outline_big=False)
    if len(ctx.res.paths) > before:
        ld = ctx.res.layers.setdefault(layer, {"count": 0, "width": None, "linetype": "Continuous"})
        ld["count"] += 1


def _text(e, ctx: _Ctx) -> None:
    t = e.dxftype()
    try:
        text = e.plain_text() if t == "MTEXT" else e.dxf.get("text", "")
    except Exception:
        text = e.dxf.get("text", "")
    text = " ".join(str(text).split())
    if not text:
        return
    ins = e.dxf.get("insert")
    if ins is None:
        return
    try:
        if t != "MTEXT":
            ins = e.ocs().to_wcs(ins)
    except Exception:
        pass
    ctx.res.texts.append(TextMark(ins[0] * ctx.k, ins[1] * ctx.k, text))
