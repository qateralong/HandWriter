use std::sync::LazyLock;

use regex::Regex;
use roxmltree::Node;

use crate::geometry::Point;
use crate::numeric::{self, repr};
use crate::svgpath::{PathError, Pen, parse_path};

pub type Matrix = [f64; 6];
pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

static NUM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[-+]?(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:[eE][-+]?[0-9]+)?").expect("valid regex"));
static TRANSFORM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(matrix|translate|scale|rotate|skewX|skewY)\s*\(([^)]*)\)").expect("valid regex"));

pub const SKIP_TAGS: &[&str] = &[
    "defs",
    "clipPath",
    "mask",
    "symbol",
    "metadata",
    "title",
    "desc",
    "style",
    "font",
    "font-face",
    "missing-glyph",
    "glyph",
    "marker",
    "pattern",
];

pub fn local_tag<'a>(el: Node<'a, '_>) -> &'a str {
    el.tag_name().name()
}

fn parse_f64(s: &str) -> f64 {
    s.parse().expect("regex matches a float literal")
}

pub fn num(value: Option<&str>, default: f64) -> f64 {
    match value.and_then(|v| NUM_RE.find(v)) {
        Some(m) => parse_f64(m.as_str()),
        None => default,
    }
}

pub fn numbers(s: &str) -> Vec<f64> {
    NUM_RE.find_iter(s).map(|m| parse_f64(m.as_str())).collect()
}

pub fn mat_mul(m1: Matrix, m2: Matrix) -> Matrix {
    let [a1, b1, c1, d1, e1, f1] = m1;
    let [a2, b2, c2, d2, e2, f2] = m2;
    [
        a1 * a2 + c1 * b2,
        b1 * a2 + d1 * b2,
        a1 * c2 + c1 * d2,
        b1 * c2 + d1 * d2,
        a1 * e2 + c1 * f2 + e1,
        b1 * e2 + d1 * f2 + f1,
    ]
}

pub fn apply(m: Matrix, (x, y): Point) -> Point {
    let [a, b, c, d, e, f] = m;
    (a * x + c * y + e, b * x + d * y + f)
}

pub fn parse_transform(s: Option<&str>) -> Matrix {
    let mut m = IDENTITY;
    let Some(s) = s.filter(|s| !s.is_empty()) else { return m };
    for cap in TRANSFORM_RE.captures_iter(s) {
        let v = numbers(&cap[2]);
        let arg = |i: usize, default: f64| v.get(i).copied().unwrap_or(default);
        let t: Matrix = match &cap[1] {
            "matrix" if v.len() == 6 => [v[0], v[1], v[2], v[3], v[4], v[5]],
            "matrix" => continue,
            "translate" => [1.0, 0.0, 0.0, 1.0, arg(0, 0.0), arg(1, 0.0)],
            "scale" => {
                let sx = arg(0, 1.0);
                [sx, 0.0, 0.0, arg(1, sx), 0.0, 0.0]
            }
            "rotate" => {
                let a = arg(0, 0.0).to_radians();
                let (cos, sin) = (a.cos(), a.sin());
                let mut t = [cos, sin, -sin, cos, 0.0, 0.0];
                if v.len() >= 3 {
                    let (cx, cy) = (v[1], v[2]);
                    t = mat_mul(mat_mul([1.0, 0.0, 0.0, 1.0, cx, cy], t), [1.0, 0.0, 0.0, 1.0, -cx, -cy]);
                }
                t
            }
            "skewX" => [1.0, 0.0, arg(0, 0.0).to_radians().tan(), 1.0, 0.0, 0.0],
            "skewY" => [1.0, arg(0, 0.0).to_radians().tan(), 0.0, 1.0, 0.0, 0.0],
            _ => continue,
        };
        m = mat_mul(m, t);
    }
    m
}

pub struct FlattenPen {
    m: Matrix,
    tol: f64,
    strokes: Vec<Vec<Point>>,
    cur: Option<Vec<Point>>,
}

impl FlattenPen {
    pub fn new(matrix: Matrix, tol: f64) -> Self {
        Self { m: matrix, tol, strokes: Vec::new(), cur: None }
    }

    fn t(&self, p: Point) -> Point {
        apply(self.m, p)
    }

    fn flush(&mut self) {
        if let Some(cur) = self.cur.take()
            && !cur.is_empty()
        {
            self.strokes.push(cur);
        }
    }

    fn cubic(&mut self, p0: Point, p1: Point, p2: Point, p3: Point, depth: u32) {
        if depth > 16 || cubic_flat(p0, p1, p2, p3, self.tol) {
            self.cur.as_mut().expect("curve inside a subpath").push(p3);
            return;
        }
        let p01 = mid(p0, p1);
        let p12 = mid(p1, p2);
        let p23 = mid(p2, p3);
        let p012 = mid(p01, p12);
        let p123 = mid(p12, p23);
        let p0123 = mid(p012, p123);
        self.cubic(p0, p01, p012, p0123, depth + 1);
        self.cubic(p0123, p123, p23, p3, depth + 1);
    }

    pub fn finish(mut self) -> Vec<Vec<Point>> {
        self.flush();
        self.strokes
    }
}

impl Pen for FlattenPen {
    fn move_to(&mut self, p: Point) {
        self.flush();
        self.cur = Some(vec![self.t(p)]);
    }

    fn line_to(&mut self, p: Point) {
        let q = self.t(p);
        self.cur.get_or_insert_with(Vec::new).push(q);
    }

    fn curve_to(&mut self, c1: Point, c2: Point, p: Point) {
        let Some(&p0) = self.cur.as_ref().and_then(|c| c.last()) else { return };
        let (c1, c2, p3) = (self.t(c1), self.t(c2), self.t(p));
        self.cubic(p0, c1, c2, p3, 0);
    }

    fn qcurve_to(&mut self, c: Point, p: Point) {
        let Some(&p0) = self.cur.as_ref().and_then(|c| c.last()) else { return };
        let (c, p2) = (self.t(c), self.t(p));
        let c1 = (p0.0 + 2.0 / 3.0 * (c.0 - p0.0), p0.1 + 2.0 / 3.0 * (c.1 - p0.1));
        let c2 = (p2.0 + 2.0 / 3.0 * (c.0 - p2.0), p2.1 + 2.0 / 3.0 * (c.1 - p2.1));
        self.cubic(p0, c1, c2, p2, 0);
    }

    fn close_path(&mut self) {
        if let Some(cur) = self.cur.as_mut()
            && let (Some(&first), Some(&last)) = (cur.first(), cur.last())
            && first != last
        {
            cur.push(first);
        }
        self.flush();
    }

    fn end_path(&mut self) {
        self.flush();
    }
}

pub fn mid(a: Point, b: Point) -> Point {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

pub fn cubic_flat(p0: Point, p1: Point, p2: Point, p3: Point, tol: f64) -> bool {
    numeric::max(dist_to_segment(p1, p0, p3), dist_to_segment(p2, p0, p3)) <= tol
}

fn dist_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    if l2 == 0.0 {
        return numeric::hypot(p.0 - a.0, p.1 - a.1);
    }
    let t = numeric::max(0.0, numeric::min(1.0, ((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2));
    numeric::hypot(p.0 - a.0 - t * dx, p.1 - a.1 - t * dy)
}

pub fn path_d_to_strokes(d: &str, matrix: Matrix, tol: f64) -> Result<Vec<Vec<Point>>, PathError> {
    let mut pen = FlattenPen::new(matrix, tol);
    parse_path(d, &mut pen)?;
    Ok(pen.finish())
}

fn points_attr(s: &str) -> Vec<Point> {
    let v = numbers(s);
    v.as_chunks::<2>().0.iter().map(|&[x, y]| (x, y)).collect()
}

pub fn element_strokes(el: Node, matrix: Matrix, tol: f64) -> Result<Vec<Vec<Point>>, PathError> {
    let tag = local_tag(el);
    let a = |name: &str| el.attribute(name);
    let num_attr = |name: &str| num(el.attribute(name), 0.0);
    let map = |pts: &[Point]| pts.iter().map(|&q| apply(matrix, q)).collect::<Vec<_>>();
    Ok(match tag {
        "path" => match a("d") {
            Some(d) if !d.is_empty() => path_d_to_strokes(d, matrix, tol)?,
            _ => Vec::new(),
        },
        "line" => vec![map(&[(num_attr("x1"), num_attr("y1")), (num_attr("x2"), num_attr("y2"))])],
        "polyline" | "polygon" => {
            let mut pts = points_attr(a("points").unwrap_or(""));
            if tag == "polygon"
                && let (Some(&first), Some(&last)) = (pts.first(), pts.last())
                && first != last
            {
                pts.push(first);
            }
            if pts.is_empty() { Vec::new() } else { vec![map(&pts)] }
        }
        "rect" => {
            let (x, y, w, h) = (num_attr("x"), num_attr("y"), num_attr("width"), num_attr("height"));
            vec![map(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)])]
        }
        "circle" | "ellipse" => {
            let (cx, cy) = (num_attr("cx"), num_attr("cy"));
            let (rx, ry) = if tag == "circle" {
                let r = num_attr("r");
                (r, r)
            } else {
                (num_attr("rx"), num_attr("ry"))
            };
            let d = format!(
                "M{},{} A{},{} 0 1 1 {},{} A{},{} 0 1 1 {},{} Z",
                repr(cx - rx),
                repr(cy),
                repr(rx),
                repr(ry),
                repr(cx + rx),
                repr(cy),
                repr(rx),
                repr(ry),
                repr(cx - rx),
                repr(cy)
            );
            path_d_to_strokes(&d, matrix, tol)?
        }
        _ => Vec::new(),
    })
}

pub fn walk_strokes(el: Node, matrix: Matrix, tol: f64, skip_tags: &[&str]) -> Result<Vec<Vec<Point>>, PathError> {
    let mut out = Vec::new();
    if skip_tags.contains(&local_tag(el)) {
        return Ok(out);
    }
    let style: String = el.attribute("style").unwrap_or("").chars().filter(|&c| c != ' ').collect();
    if el.attribute("display") == Some("none") || style.contains("display:none") {
        return Ok(out);
    }
    let m = mat_mul(matrix, parse_transform(el.attribute("transform")));
    out.extend(element_strokes(el, m, tol)?);
    for child in el.children().filter(|c| c.is_element()) {
        out.extend(walk_strokes(child, m, tol, skip_tags)?);
    }
    Ok(out)
}

pub fn parse_viewbox(root: Node) -> Option<(f64, f64, f64, f64)> {
    if let Some(vb) = root.attribute("viewBox").filter(|s| !s.is_empty()) {
        let v = numbers(vb);
        if v.len() == 4 && v[2] > 0.0 && v[3] > 0.0 {
            return Some((v[0], v[1], v[2], v[3]));
        }
    }
    let (w, h) = (root.attribute("width"), root.attribute("height"));
    if let (Some(w), Some(h)) = (w, h)
        && !w.is_empty()
        && !h.is_empty()
        && num(Some(w), 0.0) > 0.0
        && num(Some(h), 0.0) > 0.0
    {
        return Some((0.0, 0.0, num(Some(w), 0.0), num(Some(h), 0.0)));
    }
    None
}
