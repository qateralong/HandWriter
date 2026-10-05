use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use roxmltree::Node;

use crate::drawing::model::{DPath, ImportResult, TextMark, unit_mm, unit_name};
use crate::geometry::Point;
use crate::numeric::{self, format_g, repr};
use crate::settings::{DrawingImport, Units};
use crate::svgparse::{Matrix, apply, local_tag, mat_mul, num, numbers, parse_transform, path_d_to_strokes};
use crate::xml;

const STYLE_PROPS: &[&str] =
    &["fill", "stroke", "stroke-width", "stroke-dasharray", "stroke-dashoffset", "display", "visibility"];
const INHERITED: &[&str] = &["fill", "stroke", "stroke-width", "stroke-dasharray", "stroke-dashoffset", "visibility"];
const CONTAINERS: &[&str] = &["svg", "g", "a", "switch", "symbol"];
const SKIP: &[&str] = &[
    "defs",
    "clipPath",
    "mask",
    "marker",
    "pattern",
    "metadata",
    "title",
    "desc",
    "style",
    "script",
    "linearGradient",
    "radialGradient",
    "filter",
    "font",
    "font-face",
    "foreignObject",
    "namedview",
];
const TEXT: &[&str] = &["text", "flowRoot"];
const SHAPES: &[&str] = &["path", "line", "polyline", "polygon", "rect", "circle", "ellipse"];
const WHITE: &[&str] = &["white", "#fff", "#ffffff", "rgb(255,255,255)", "rgb(100%,100%,100%)"];
const XLINK: &str = "http://www.w3.org/1999/xlink";

static UNIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*([-+]?(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:[eE][-+]?[0-9]+)?)\s*([a-zA-Z%]*)\s*$").expect("valid regex")
});
static IMPORTANT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*!important\s*$").expect("valid regex"));
static CSS_COMMENT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)/\*.*?\*/").expect("valid regex"));
static CSS_RULE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([^{}]+)\{([^{}]*)\}").expect("valid regex"));
static CSS_SELECTOR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\*|[a-zA-Z][\w-]*)?(?:\.([\w-]+))?(?:#([\w-]+))?$").expect("valid regex"));

pub fn length_mm(value: Option<&str>) -> Option<(f64, String)> {
    let c = UNIT_RE.captures(value.filter(|v| !v.is_empty())?)?;
    let unit = match c.get(2).map(|m| m.as_str()).filter(|u| !u.is_empty()) {
        Some(u) => u.to_lowercase(),
        None => "px".into(),
    };
    unit_mm(&unit)?;
    Some((c[1].parse().expect("regex matches a float"), unit))
}

type Decls = IndexMap<String, String>;

fn parse_decls(text: &str) -> Decls {
    let mut out = Decls::new();
    for part in text.split(';') {
        if let Some((k, v)) = part.split_once(':') {
            let k = k.trim().to_lowercase();
            let v = IMPORTANT_RE.replace(v.trim(), "").into_owned();
            if STYLE_PROPS.contains(&k.as_str()) {
                out.insert(k, v);
            }
        }
    }
    out
}

struct Rule {
    spec: (u8, u8, u8),
    order: usize,
    tag: Option<String>,
    class: Option<String>,
    id: Option<String>,
    decls: Decls,
}

struct Css {
    rules: Vec<Rule>,
}

fn leading_text(el: Node) -> Option<String> {
    let mut s = String::new();
    let mut any = false;
    for c in el.children() {
        if c.is_element() {
            break;
        }
        if let Some(t) = c.text().filter(|_| c.is_text()) {
            s.push_str(t);
            any = true;
        }
    }
    any.then_some(s)
}

impl Css {
    fn new(root: Node) -> Self {
        let mut rules = Vec::new();
        let mut order = 0;
        for el in root.descendants().filter(|n| n.is_element()) {
            if local_tag(el) != "style" {
                continue;
            }
            let Some(text) = leading_text(el).filter(|t| !t.is_empty()) else { continue };
            let text = CSS_COMMENT_RE.replace_all(&text, "");
            for cap in CSS_RULE_RE.captures_iter(&text) {
                let decls = parse_decls(&cap[2]);
                if decls.is_empty() {
                    continue;
                }
                for sel in cap[1].split(',') {
                    let sel = sel.trim();
                    if sel.is_empty() {
                        continue;
                    }
                    let Some(m) = CSS_SELECTOR_RE.captures(sel) else { continue };
                    let tag = m.get(1).map(|x| x.as_str().to_string());
                    let class = m.get(2).map(|x| x.as_str().to_string());
                    let id = m.get(3).map(|x| x.as_str().to_string());
                    let spec = (
                        u8::from(id.is_some()),
                        u8::from(class.is_some()),
                        u8::from(tag.as_deref().is_some_and(|t| t != "*")),
                    );
                    rules.push(Rule { spec, order, tag: tag.filter(|t| t != "*"), class, id, decls: decls.clone() });
                    order += 1;
                }
            }
        }
        rules.sort_by_key(|r| (r.spec, r.order));
        Self { rules }
    }

    fn matches(&self, el: Node) -> Decls {
        let mut out = Decls::new();
        if self.rules.is_empty() {
            return out;
        }
        let tag = local_tag(el);
        let classes: HashSet<&str> = el.attribute("class").unwrap_or("").split_whitespace().collect();
        let ident = el.attribute("id");
        for r in &self.rules {
            if r.tag.as_deref().is_none_or(|t| t == tag)
                && r.class.as_deref().is_none_or(|c| classes.contains(c))
                && r.id.as_deref().is_none_or(|i| Some(i) == ident)
            {
                for (k, v) in &r.decls {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
        out
    }
}

fn is_none_value(v: Option<&str>) -> bool {
    v.is_none_or(|v| matches!(v.trim().to_lowercase().as_str(), "none" | "transparent" | ""))
}

fn is_white(v: Option<&str>) -> bool {
    v.is_some_and(|v| WHITE.contains(&v.trim().to_lowercase().replace(' ', "").as_str()))
}

type Style = HashMap<String, String>;

struct Ctx<'a, 'input> {
    res: ImportResult,
    tol: f64,
    opts: &'a DrawingImport,
    ids: HashMap<&'a str, Node<'a, 'input>>,
    css: Css,
    skipped_white: usize,
    skipped_invisible: usize,
    use_depth: usize,
    fills_small: usize,
}

pub type CenterlineFn = dyn Fn(&[Vec<Point>]) -> Vec<Vec<Point>>;

fn units_key(u: Units) -> &'static str {
    match u {
        Units::Auto => "auto",
        Units::Mm => "mm",
        Units::Cm => "cm",
        Units::M => "m",
        Units::In => "in",
        Units::Ft => "ft",
        Units::Pt => "pt",
        Units::Px => "px",
    }
}

pub fn import_svg(data: &[u8], name: &str, opts: &DrawingImport, tol: f64) -> ImportResult {
    let mut res = ImportResult::new("svg", name);
    let text = match xml::decode(data) {
        Ok(t) => t,
        Err(e) => {
            res.errors.push(format!("SVG не читается: {e}"));
            return res;
        }
    };
    let doc = match xml::parse(&text) {
        Ok(d) => d,
        Err(e) => {
            res.errors.push(format!("SVG не читается: {e}"));
            return res;
        }
    };
    let root = doc.root_element();
    if local_tag(root) != "svg" {
        res.errors.push("Это не SVG: корневой элемент не <svg>".into());
        return res;
    }

    let vb =
        Some(numbers(root.attribute("viewBox").unwrap_or(""))).filter(|v| v.len() == 4 && v[2] > 0.0 && v[3] > 0.0);
    let (w, h) = (length_mm(root.attribute("width")), length_mm(root.attribute("height")));
    let k;
    if opts.units != Units::Auto {
        let u = units_key(opts.units);
        k = unit_mm(u).expect("known unit");
        res.units = u.into();
        res.units_note = format!("указано вручную: 1 единица файла = 1 {}", unit_name(u).expect("known unit"));
    } else if let (Some(vb), Some(w), Some(h)) = (&vb, &w, &h) {
        let kx = w.0 * unit_mm(&w.1).expect("known unit") / vb[2];
        let ky = h.0 * unit_mm(&h.1).expect("known unit") / vb[3];
        k = numeric::min(kx, ky);
        res.units = if (k - 1.0).abs() < 1e-9 { "mm".into() } else { w.1.clone() };
        res.units_note = format!(
            "по width/height ({} × {}) и viewBox: 1 единица = {} мм",
            root.attribute("width").unwrap_or(""),
            root.attribute("height").unwrap_or(""),
            format_g(k, 4)
        );
    } else if let (Some(w), Some(_), None) = (&w, &h, &vb) {
        k = unit_mm(&w.1).expect("known unit");
        res.units = w.1.clone();
        res.units_note = format!("по width/height: единица {}", unit_name(&w.1).unwrap_or(&w.1));
    } else {
        k = unit_mm("px").expect("known unit");
        res.units = "px".into();
        res.units_note = "в файле нет размеров с единицами: считаю 96 px на дюйм".into();
    }
    let mut base: Matrix = [k, 0.0, 0.0, -k, 0.0, 0.0];
    if let Some(vb) = &vb {
        base = mat_mul(base, [1.0, 0.0, 0.0, 1.0, -vb[0], -vb[1]]);
    }

    let mut ids = HashMap::new();
    for el in root.descendants().filter(|n| n.is_element()) {
        if let Some(id) = el.attribute("id") {
            ids.insert(id, el);
        }
    }
    let mut ctx = Ctx {
        res,
        tol,
        opts,
        ids,
        css: Css::new(root),
        skipped_white: 0,
        skipped_invisible: 0,
        use_depth: 0,
        fills_small: 0,
    };
    let style: Style = [
        ("fill", "black"),
        ("stroke", "none"),
        ("stroke-width", "1"),
        ("stroke-dasharray", "none"),
        ("stroke-dashoffset", "0"),
        ("visibility", "visible"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    walk(root, base, &style, &mut ctx, true);

    let mut res = ctx.res;
    if ctx.skipped_white > 0 {
        res.warnings.push(format!("Белые заливки и обводки (фон) пропущены: {}", ctx.skipped_white));
    }
    if ctx.fills_small > 0 {
        res.warnings.push(format!("Мелкие закрашенные фигуры превращены в центральные линии: {}", ctx.fills_small));
    }
    res
}

fn computed(el: Node, parent: &Style, ctx: &Ctx) -> Style {
    let mut own = Decls::new();
    for &k in STYLE_PROPS {
        if let Some(v) = el.attribute(k) {
            own.insert(k.to_string(), v.to_string());
        }
    }
    for (k, v) in ctx.css.matches(el) {
        own.insert(k, v);
    }
    if let Some(s) = el.attribute("style") {
        for (k, v) in parse_decls(s) {
            own.insert(k, v);
        }
    }
    let mut st: Style = INHERITED.iter().map(|&k| (k.to_string(), parent[k].clone())).collect();
    st.insert("display".into(), "inline".into());
    for (k, v) in own {
        if v.trim() == "inherit" {
            continue;
        }
        st.insert(k, v.trim().to_string());
    }
    st
}

fn translate(x: f64, y: f64) -> Matrix {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

fn walk<'a, 'input>(el: Node<'a, 'input>, matrix: Matrix, parent: &Style, ctx: &mut Ctx<'a, 'input>, is_root: bool) {
    let tag = local_tag(el);
    if SKIP.contains(&tag) || (tag == "symbol" && ctx.use_depth == 0) {
        return;
    }
    let st = computed(el, parent, ctx);
    if st["display"] == "none" {
        return;
    }
    let mut m = mat_mul(matrix, parse_transform(el.attribute("transform")));
    if tag == "svg" && !is_root {
        m = mat_mul(m, translate(num(el.attribute("x"), 0.0), num(el.attribute("y"), 0.0)));
    }
    if TEXT.contains(&tag) {
        text_mark(el, m, ctx);
        return;
    }
    if tag == "image" {
        let (x, y) = apply(m, (num(el.attribute("x"), 0.0), num(el.attribute("y"), 0.0)));
        ctx.res
            .texts
            .push(TextMark { x, y, text: "встроенная картинка".into(), kind: "image".into() });
        return;
    }
    if tag == "use" {
        let href = [el.attribute("href"), el.attribute((XLINK, "href"))]
            .into_iter()
            .flatten()
            .find(|h| !h.is_empty())
            .unwrap_or("");
        let Some(&target) = ctx.ids.get(href.trim_start_matches('#')) else { return };
        if ctx.use_depth > 8 {
            return;
        }
        let m2 = mat_mul(m, translate(num(el.attribute("x"), 0.0), num(el.attribute("y"), 0.0)));
        ctx.use_depth += 1;
        walk(target, m2, &st, ctx, false);
        ctx.use_depth -= 1;
        return;
    }
    if SHAPES.contains(&tag) {
        shape(el, tag, m, &st, ctx);
        return;
    }
    if CONTAINERS.contains(&tag) || is_root {
        for child in el.children().filter(|c| c.is_element()) {
            walk(child, m, &st, ctx, false);
        }
    }
}

fn text_mark(el: Node, m: Matrix, ctx: &mut Ctx) {
    let raw: String = el.descendants().filter(|n| n.is_text()).filter_map(|n| n.text()).collect();
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return;
    }
    let mut xy: Option<(f64, f64)> = None;
    for sub in el.descendants().filter(|n| n.is_element()) {
        if xy.is_none() && sub.attribute("x").is_some_and(|x| !x.is_empty()) {
            xy = Some((num(sub.attribute("x"), 0.0), num(sub.attribute("y"), 0.0)));
        }
    }
    let (x, y) = apply(m, xy.unwrap_or((0.0, 0.0)));
    ctx.res.texts.push(TextMark { x, y, text, kind: "text".into() });
}

fn shape_d(el: Node, tag: &str) -> Option<String> {
    let n = |name: &str| num(el.attribute(name), 0.0);
    let r = repr;
    match tag {
        "path" => el.attribute("d").map(String::from),
        "line" => Some(format!("M{},{} L{},{}", r(n("x1")), r(n("y1")), r(n("x2")), r(n("y2")))),
        "polyline" | "polygon" => {
            let v = numbers(el.attribute("points").unwrap_or(""));
            if v.len() < 2 {
                return None;
            }
            let pts: Vec<String> = v.as_chunks::<2>().0.iter().map(|&[x, y]| format!("{},{}", r(x), r(y))).collect();
            Some(format!("M{}{}", pts.join(" L"), if tag == "polygon" { " Z" } else { "" }))
        }
        "rect" => {
            let (x, y, w, h) = (n("x"), n("y"), n("width"), n("height"));
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            let (rxa, rya) = (el.attribute("rx"), el.attribute("ry"));
            let rx = match (rxa, rya) {
                (Some(v), _) => num(Some(v), 0.0),
                (None, Some(v)) => num(Some(v), 0.0),
                (None, None) => 0.0,
            };
            let ry = rya.map_or(rx, |v| num(Some(v), 0.0));
            let (rx, ry) = (numeric::min(rx, w / 2.0), numeric::min(ry, h / 2.0));
            if rx <= 0.0 || ry <= 0.0 {
                return Some(format!("M{},{} H{} V{} H{} Z", r(x), r(y), r(x + w), r(y + h), r(x)));
            }
            Some(format!(
                "M{},{} H{} A{},{} 0 0 1 {},{} V{} A{},{} 0 0 1 {},{} H{} A{},{} 0 0 1 {},{} V{} A{},{} 0 0 1 {},{} Z",
                r(x + rx),
                r(y),
                r(x + w - rx),
                r(rx),
                r(ry),
                r(x + w),
                r(y + ry),
                r(y + h - ry),
                r(rx),
                r(ry),
                r(x + w - rx),
                r(y + h),
                r(x + rx),
                r(rx),
                r(ry),
                r(x),
                r(y + h - ry),
                r(y + ry),
                r(rx),
                r(ry),
                r(x + rx),
                r(y)
            ))
        }
        "circle" | "ellipse" => {
            let (cx, cy) = (n("cx"), n("cy"));
            let (rx, ry) = if tag == "circle" { (n("r"), n("r")) } else { (n("rx"), n("ry")) };
            if rx <= 0.0 || ry <= 0.0 {
                return None;
            }
            Some(format!(
                "M{},{} A{},{} 0 1 1 {},{} A{},{} 0 1 1 {},{} Z",
                r(cx + rx),
                r(cy),
                r(rx),
                r(ry),
                r(cx - rx),
                r(cy),
                r(rx),
                r(ry),
                r(cx + rx),
                r(cy)
            ))
        }
        _ => None,
    }
}

fn shape(el: Node, tag: &str, m: Matrix, st: &Style, ctx: &mut Ctx) {
    if matches!(st.get("visibility").map(String::as_str), Some("hidden" | "collapse")) {
        ctx.skipped_invisible += 1;
        return;
    }
    let Some(d) = shape_d(el, tag).filter(|d| !d.is_empty()) else { return };
    let subpaths = match path_d_to_strokes(&d, m, ctx.tol) {
        Ok(s) => s,
        Err(_) => {
            ctx.res.warnings.push(format!("Не разобран путь {}", el.attribute("id").unwrap_or(tag)));
            return;
        }
    };
    let subpaths: Vec<Vec<Point>> = subpaths.into_iter().filter(|p| !p.is_empty()).collect();
    if subpaths.is_empty() {
        return;
    }
    let scale = (m[0] * m[3] - m[1] * m[2]).abs().sqrt();
    let (stroke, fill) = (st.get("stroke").map(String::as_str), st.get("fill").map(String::as_str));
    if !is_none_value(stroke) {
        if is_white(stroke) {
            ctx.skipped_white += 1;
            return;
        }
        let width = num(st.get("stroke-width").map(String::as_str), 1.0) * scale;
        let da = st.get("stroke-dasharray").map_or("none", String::as_str);
        let mut dash = None;
        if !is_none_value(Some(da)) {
            let vals: Vec<f64> = numbers(da).into_iter().map(|v| v.abs() * scale).collect();
            if !vals.is_empty() && numeric::sum(vals.iter().copied()) > 0.0 {
                dash = Some(vals);
            }
        }
        let off = num(st.get("stroke-dashoffset").map(String::as_str), 0.0) * scale;
        for p in subpaths {
            let closed = p.len() > 2 && p[0] == p[p.len() - 1];
            ctx.res.paths.push(DPath {
                points: p,
                closed,
                width: Some(width),
                layer: String::new(),
                dash: dash.clone(),
                dash_offset: off,
            });
        }
        return;
    }
    if is_none_value(fill) || tag == "line" {
        ctx.skipped_invisible += 1;
        return;
    }
    if is_white(fill) {
        ctx.skipped_white += 1;
        return;
    }
    if add_filled(&subpaths, &mut ctx.res, ctx.opts, "", true, None) {
        ctx.fills_small += 1;
    }
}

fn distinct_points(p: &[Point]) -> usize {
    let norm = |v: f64| if v == 0.0 { 0.0_f64.to_bits() } else { v.to_bits() };
    p.iter().map(|q| (norm(q.0), norm(q.1))).collect::<HashSet<_>>().len()
}

pub fn add_filled(
    subpaths: &[Vec<Point>],
    res: &mut ImportResult,
    opts: &DrawingImport,
    layer: &str,
    outline_big: bool,
    centerlines: Option<&CenterlineFn>,
) -> bool {
    let closed_sub: Vec<Vec<Point>> = subpaths
        .iter()
        .filter(|p| distinct_points(p) >= 3)
        .map(|p| {
            let mut q = p.clone();
            if q[0] != q[q.len() - 1] {
                q.push(q[0]);
            }
            q
        })
        .collect();
    if closed_sub.is_empty() {
        return false;
    }
    if opts.fill_centerlines
        && let Some(f) = centerlines
    {
        let xs = closed_sub.iter().flatten().map(|q| q.0);
        let ys = closed_sub.iter().flatten().map(|q| q.1);
        let (x0, x1) = (xs.clone().reduce(numeric::min).unwrap(), xs.reduce(numeric::max).unwrap());
        let (y0, y1) = (ys.clone().reduce(numeric::min).unwrap(), ys.reduce(numeric::max).unwrap());
        if numeric::min(x1 - x0, y1 - y0) <= opts.fill_centerline_max {
            let lines = f(&closed_sub);
            if !lines.is_empty() {
                for p in lines {
                    let closed = p.len() > 2 && p[0] == p[p.len() - 1];
                    res.paths.push(DPath::new(p, closed, None, layer));
                }
                return true;
            }
        }
    }
    if outline_big {
        for p in closed_sub {
            res.paths.push(DPath::new(p, true, None, layer));
        }
    }
    false
}
