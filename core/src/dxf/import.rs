use std::collections::{BTreeMap, HashMap};

use super::doc::*;
use super::explode::*;
use super::geom::*;
use super::path::Path;
use super::text::fast_plain_mtext;
use crate::drawing::model::{DPath, ImportResult, LayerInfo, TextMark, unit_mm, unit_name};
use crate::drawing::svg_import::{add_filled, units_key};
use crate::geometry::Point;
use crate::numeric;
use crate::settings::{DrawingImport, Units};

const DEFAULT_LW: f64 = 0.25;

fn insunits(u: i64) -> Option<(f64, &'static str)> {
    Some(match u {
        1 => (25.4, "дюймы"),
        2 => (304.8, "футы"),
        3 => (1609344.0, "мили"),
        4 => (1.0, "мм"),
        5 => (10.0, "см"),
        6 => (1000.0, "м"),
        7 => (1e6, "км"),
        8 => (25.4e-6, "микродюймы"),
        9 => (0.0254, "милы"),
        10 => (914.4, "ярды"),
        11 => (1e-7, "ангстремы"),
        12 => (1e-6, "нм"),
        13 => (1e-3, "мкм"),
        14 => (100.0, "дм"),
        15 => (1e4, "дам"),
        16 => (1e5, "гм"),
        _ => return None,
    })
}

#[derive(Clone)]
struct BlockCtx {
    layer: String,
    width: Option<f64>,
    linetype: String,
}

struct Ctx<'a> {
    doc: &'a Doc,
    res: ImportResult,
    k: f64,
    tol: f64,
    opts: &'a DrawingImport,
    ltscale: f64,
    lw_default: f64,
    skipped: BTreeMap<String, usize>,
    fills_small: usize,
    patterns: HashMap<String, Option<Vec<f64>>>,
}

impl Ctx<'_> {
    fn skip(&mut self, kind: &str) {
        *self.skipped.entry(kind.to_string()).or_insert(0) += 1;
    }
}

fn lw_mm(lw: Option<i64>, lw_default: f64) -> Option<f64> {
    let lw = lw?;
    if lw >= 0 {
        Some(lw as f64 / 100.0)
    } else if lw == -3 {
        Some(lw_default)
    } else {
        None
    }
}

pub fn import_dxf(data: &[u8], name: &str, opts: &DrawingImport, tol_mm: f64) -> ImportResult {
    let mut res = ImportResult::new("dxf", name);
    let doc = match load(data) {
        Ok(d) => d,
        Err(e) => {
            res.errors.push(format!("DXF не читается: {e}"));
            return res;
        }
    };
    if doc.errors > 0 {
        res.warnings.push(format!("DXF с ошибками структуры ({}), прочитано что удалось", doc.errors));
    }
    let ins = match doc.header.get("$INSUNITS") {
        Some(Val::I(v)) => *v,
        Some(Val::F(v)) => *v as i64,
        _ => 0,
    };
    let k;
    if opts.units != Units::Auto {
        let u = units_key(opts.units);
        k = unit_mm(u).expect("known unit");
        res.units = u.into();
        res.units_note = format!("указано вручную: {}", unit_name(u).unwrap_or(u));
    } else if let Some((kk, nm)) = insunits(ins) {
        k = kk;
        res.units = match ins {
            1 => "in",
            2 => "ft",
            5 => "cm",
            6 => "m",
            _ => "mm",
        }
        .into();
        res.units_note = format!("по $INSUNITS = {ins} ({nm})");
    } else {
        k = 1.0;
        res.units = "mm".into();
        res.units_note = format!("$INSUNITS = {ins} (единицы не заданы): считаю мм, можно указать вручную");
        res.warnings.push(
            "В DXF не заданы единицы ($INSUNITS = 0): считаю, что чертёж в мм. Если размер не тот, выбери единицы вручную"
                .into(),
        );
    }
    let ltscale = match doc.header_f("$LTSCALE") {
        Some(v) if v != 0.0 => v,
        _ => 1.0,
    };
    let lw_default = match doc.header.get("$LWDEFAULT") {
        Some(Val::I(v)) if *v > 0 => *v as f64 / 100.0,
        Some(Val::F(v)) if *v > 0.0 => *v / 100.0,
        None => 0.25,
        _ => DEFAULT_LW,
    };
    let mut ctx = Ctx {
        doc: &doc,
        res,
        k,
        tol: tol_mm / k,
        opts,
        ltscale,
        lw_default,
        skipped: BTreeMap::new(),
        fills_small: 0,
        patterns: HashMap::new(),
    };
    for layer in doc.layers.values() {
        ctx.res.layers.insert(
            layer.name.clone(),
            LayerInfo {
                count: 0,
                width: lw_mm(Some(layer.lineweight.unwrap_or(-3)), lw_default),
                linetype: layer.linetype.clone().unwrap_or_else(|| "Continuous".into()),
            },
        );
    }
    let mut entities: &[Entity] = &doc.msp;
    if entities.is_empty() {
        for (lname, lents) in &doc.layouts {
            if lents.iter().any(|e| e.dxftype != "VIEWPORT") {
                entities = lents;
                ctx.res.warnings.push(format!("Пространство модели пустое, взят лист «{lname}»"));
                break;
            }
        }
    }
    for e in entities {
        entity(e.clone(), &mut ctx, None);
    }
    ctx.res.layers.retain(|_, v| v.count > 0);
    let skipped = std::mem::take(&mut ctx.skipped);
    for (kind, n) in skipped {
        let what = match kind.as_str() {
            "HATCH" => "штриховки и заливки (HATCH) не рисуются".to_string(),
            "SOLID" => "залитые фигуры (SOLID/TRACE) не рисуются (галочка «мелкие заливки → линии»)".to_string(),
            "IMAGE" => "встроенные картинки не рисуются".to_string(),
            "error" => "объекты, которые не удалось разобрать".to_string(),
            _ => format!("{kind} не поддерживается"),
        };
        ctx.res.warnings.push(format!("{what}: {n}"));
    }
    if ctx.fills_small > 0 {
        ctx.res.warnings.push(format!("Мелкие залитые фигуры превращены в центральные линии: {}", ctx.fills_small));
    }
    ctx.res
}

fn layer_name(e: &Entity, block: Option<&BlockCtx>) -> String {
    let name = e.c.layer.clone().unwrap_or_else(|| "0".into());
    match block {
        Some(b) if name == "0" => b.layer.clone(),
        _ => name,
    }
}

fn entity_linetype(e: &Entity, doc: &Doc) -> String {
    let lt = e.c.linetype.clone().unwrap_or_else(|| "BYLAYER".into());
    let low = lt.to_lowercase();
    if low != "bylayer" && low != "byblock" && !doc.linetypes.contains_key(&low) {
        return "BYLAYER".into();
    }
    lt
}

fn resolved(e: &Entity, ctx: &Ctx, block: Option<&BlockCtx>) -> (String, Option<f64>, String) {
    let layer = layer_name(e, block);
    let ld = ctx.res.layers.get(&layer);
    let lw = e.c.lineweight.unwrap_or(-1);
    let width = match lw {
        -1 => ld.map_or(Some(ctx.lw_default), |l| l.width),
        -2 => block.map_or(Some(ctx.lw_default), |b| b.width),
        _ => lw_mm(Some(lw), ctx.lw_default),
    };
    let mut lt = entity_linetype(e, ctx.doc);
    if lt.to_uppercase() == "BYLAYER" {
        lt = ld.map_or("Continuous".into(), |l| l.linetype.clone());
    } else if lt.to_uppercase() == "BYBLOCK" {
        lt = block.map_or("Continuous".into(), |b| b.linetype.clone());
    }
    (layer, width, lt)
}

fn merge_dashes(el: &[f64]) -> Vec<f64> {
    let sign = |v: f64| {
        if v < 0.0 {
            -1
        } else if v > 0.0 {
            1
        } else {
            0
        }
    };
    let mut out = Vec::new();
    let mut buf = el[0];
    let mut prev = sign(buf);
    for &e in &el[1..] {
        if sign(e) == prev {
            buf += e;
        } else {
            out.push(buf);
            buf = e;
            prev = sign(e);
        }
    }
    out.push(buf);
    out
}

fn simplified_pattern(lt: &Linetype) -> Vec<f64> {
    if lt.complex || lt.elements.len() < 2 {
        return Vec::new();
    }
    let mut el = merge_dashes(&lt.elements);
    if el.len() < 2 || lt.total <= 0.0 {
        return Vec::new();
    }
    let s = numeric::sum(el.iter().map(|e| e.abs()));
    if lt.total != 0.0 && lt.total > s {
        el.push(s - lt.total);
    }
    if el[0] < 0.0 {
        let e = el.remove(0);
        let n = el.len();
        if el[n - 1] < 0.0 {
            el[n - 1] += e;
        } else {
            el.push(e);
        }
    }
    el.iter().map(|e| e.abs()).collect()
}

fn pattern(name: &str, ctx: &mut Ctx) -> Option<Vec<f64>> {
    let key_u = name.to_uppercase();
    if let Some(p) = ctx.patterns.get(&key_u) {
        return p.clone();
    }
    let mut pat = None;
    if !matches!(key_u.as_str(), "CONTINUOUS" | "BYLAYER" | "BYBLOCK" | "")
        && let Some(lt) = ctx.doc.linetypes.get(&key(name))
    {
        let p: Vec<f64> = simplified_pattern(lt).iter().map(|v| v.abs()).collect();
        if p.len() >= 2 && numeric::sum(p.iter().copied()) > 0.0 {
            pat = Some(p);
        }
    }
    ctx.patterns.insert(key_u, pat.clone());
    pat
}

fn entity(e: Entity, ctx: &mut Ctx, block: Option<&BlockCtx>) {
    let t = e.dxftype.clone();
    if e.c.invisible.unwrap_or(0) != 0 {
        return;
    }
    let layer = layer_name(&e, block);
    if let Some(l) = ctx.doc.layer(&layer)
        && (l.is_off() || l.is_frozen())
    {
        return;
    }
    match t.as_str() {
        "INSERT" | "DIMENSION" | "ARC_DIMENSION" | "LARGE_RADIAL_DIMENSION" | "LEADER" | "MLEADER" | "MULTILEADER" => {
            let (_, width, lt) = resolved(&e, ctx, block);
            let inner = BlockCtx { layer, width, linetype: lt };
            let doc = ctx.doc;
            let mut sink = |ve: Entity| entity(ve, ctx, Some(&inner));
            let r = match t.as_str() {
                "INSERT" => insert_entities(&e, doc, &mut sink),
                "LEADER" => leader_entities(&e, doc, &mut sink),
                "MLEADER" | "MULTILEADER" => Err(Fail::Error),
                _ => dimension_entities(&e, doc, &mut sink),
            };
            if r.is_err() {
                ctx.skip("error");
            }
        }
        "TEXT" | "MTEXT" | "ATTRIB" => text(&e, ctx),
        "LINE" | "LWPOLYLINE" | "POLYLINE" | "ARC" | "CIRCLE" | "ELLIPSE" | "SPLINE" => curve(&e, ctx, block),
        "SOLID" | "TRACE" | "HATCH" => fill(&e, ctx, block),
        "IMAGE" => {
            if let Body::Image { insert: Some(p) } = &e.body {
                ctx.res.texts.push(TextMark {
                    x: p.x * ctx.k,
                    y: p.y * ctx.k,
                    text: "встроенная картинка".into(),
                    kind: "image".into(),
                });
            }
            ctx.skip("IMAGE");
        }
        "VIEWPORT" | "ATTDEF" | "POINT" => {}
        _ => ctx.skip(&t),
    }
}

fn flatten(path: &Path, ctx: &Ctx) -> R<Vec<Vec<Point>>> {
    let subs = if path.has_sub_paths { path.sub_paths() } else { vec![path.clone()] };
    let mut out = Vec::new();
    for sp in subs {
        let mut dd: Vec<Point> = Vec::new();
        for v in sp.flattening(ctx.tol)? {
            let p = (v.x * ctx.k, v.y * ctx.k);
            if dd.is_empty() || numeric::dist(p, dd[dd.len() - 1]) > 1e-9 {
                dd.push(p);
            }
        }
        if !dd.is_empty() {
            out.push(dd);
        }
    }
    Ok(out)
}

fn curve(e: &Entity, ctx: &mut Ctx, block: Option<&BlockCtx>) {
    if let Body::Polyline { flags, .. } = &e.body
        && flags & (16 | 64) != 0
    {
        ctx.skip("POLYLINE (сетка)");
        return;
    }
    let pieces = match make_path(e).and_then(|p| flatten(&p, ctx)) {
        Ok(p) => p,
        Err(_) => {
            ctx.skip("error");
            return;
        }
    };
    let (layer, mut width, lt) = resolved(e, ctx, block);
    if let Body::LwPolyline { pts, const_width, .. } = &e.body {
        let cw = match const_width {
            Some(v) if *v != 0.0 => *v,
            _ => 0.0,
        };
        let vw = pts.iter().map(|p| numeric::max(p[2], p[3])).reduce(numeric::max).unwrap_or(0.0);
        let w_poly = numeric::max(cw, vw) * ctx.k;
        if w_poly > 0.0 {
            width = Some(numeric::max(width.unwrap_or(0.0), w_poly));
        }
    }
    let dash = pattern(&lt, ctx).map(|pat| {
        let els = match e.c.ltscale {
            Some(v) if v != 0.0 => v,
            _ => 1.0,
        };
        let s = ctx.ltscale * els * ctx.k;
        pat.iter().map(|v| v * s).collect::<Vec<f64>>()
    });
    let ld = ctx.res.layers.entry(layer.clone()).or_insert(LayerInfo {
        count: 0,
        width: None,
        linetype: "Continuous".into(),
    });
    ld.count += 1;
    for mut p in pieces {
        let closed = p.len() > 2 && numeric::dist(p[0], p[p.len() - 1]) < 1e-9;
        if closed {
            let n = p.len();
            p[n - 1] = p[0];
        }
        ctx.res.paths.push(DPath {
            points: p,
            closed,
            width,
            layer: layer.clone(),
            dash: dash.clone(),
            dash_offset: 0.0,
        });
    }
}

fn fill(e: &Entity, ctx: &mut Ctx, block: Option<&BlockCtx>) {
    let is_hatch = e.dxftype == "HATCH";
    let solid = matches!(e.body, Body::Hatch { solid_fill, .. } if solid_fill != 0);
    if !ctx.opts.fill_centerlines || (is_hatch && !solid) {
        ctx.skip(if is_hatch { "HATCH" } else { "SOLID" });
        return;
    }
    let contours: R<Vec<Vec<Point>>> = (|| {
        if is_hatch {
            let mut out = Vec::new();
            for p in hatch_paths(e)? {
                out.extend(flatten(&p, ctx)?);
            }
            Ok(out)
        } else {
            let Body::Solid { vtx } = &e.body else { return Err(Fail::Error) };
            let mut v: Vec<V3> = vtx.iter().flatten().copied().collect();
            if v.len() == 4 {
                v = vec![v[0], v[1], v[3], v[2]];
            }
            let ocs = Ocs::new(e.c.extrusion())?;
            Ok(vec![v.iter().map(|x| ocs.to_wcs(*x)).map(|w| (w.x * ctx.k, w.y * ctx.k)).collect()])
        }
    })();
    let Ok(contours) = contours else {
        ctx.skip("error");
        return;
    };
    let layer = layer_name(e, block);
    let before = ctx.res.paths.len();
    if add_filled(&contours, &mut ctx.res, ctx.opts, &layer, false) {
        ctx.fills_small += 1;
    }
    if ctx.res.paths.len() > before {
        let ld =
            ctx.res.layers.entry(layer).or_insert(LayerInfo { count: 0, width: None, linetype: "Continuous".into() });
        ld.count += 1;
    }
}

fn py_split_join(s: &str) -> String {
    s.split(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn text(e: &Entity, ctx: &mut Ctx) {
    let (raw, insert, is_mtext) = match &e.body {
        Body::MText { text, insert, .. } => (fast_plain_mtext(text), *insert, true),
        Body::Text { text, insert, .. } => (text.clone(), *insert, false),
        _ => return,
    };
    let t = py_split_join(&raw);
    if t.is_empty() {
        return;
    }
    let Some(mut ins) = insert else { return };
    if !is_mtext && let Ok(o) = Ocs::new(e.c.extrusion()) {
        ins = o.to_wcs(ins);
    }
    ctx.res.texts.push(TextMark { x: ins.x * ctx.k, y: ins.y * ctx.k, text: t, kind: "text".into() });
}
