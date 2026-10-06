use std::f64::consts::TAU;

use super::curves::*;
use super::geom::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    Line,
    Move,
    Curve4,
}

impl Cmd {
    fn size(self) -> usize {
        if self == Cmd::Curve4 { 3 } else { 1 }
    }
}

#[derive(Debug, Clone)]
pub struct Path {
    pub vertices: Vec<V3>,
    pub commands: Vec<Cmd>,
    start_index: Vec<usize>,
    pub has_sub_paths: bool,
}

pub enum Elem {
    Line(V3),
    Move(V3),
    Curve4(V3, V3, V3),
}

fn vertex_index(cmds: &[Cmd]) -> Vec<usize> {
    let mut idx = Vec::with_capacity(cmds.len());
    let mut i = 1;
    for c in cmds {
        idx.push(i);
        i += c.size();
    }
    idx
}

const IS_CLOSE_TOL: f64 = 1e-10;

impl Path {
    pub fn new(start: V3) -> Path {
        Path { vertices: vec![start], commands: Vec::new(), start_index: Vec::new(), has_sub_paths: false }
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    pub fn start(&self) -> V3 {
        self.vertices[0]
    }

    pub fn set_start(&mut self, p: V3) {
        if self.commands.is_empty() {
            self.vertices[0] = p;
        }
    }

    pub fn end(&self) -> V3 {
        self.vertices[self.vertices.len() - 1]
    }

    pub fn is_closed(&self) -> bool {
        self.vertices.len() > 1 && self.vertices[0].isclose(self.end())
    }

    pub fn line_to(&mut self, p: V3) {
        self.commands.push(Cmd::Line);
        self.start_index.push(self.vertices.len());
        self.vertices.push(p);
    }

    pub fn move_to(&mut self, p: V3) {
        if self.commands.is_empty() {
            self.vertices[0] = p;
            return;
        }
        self.has_sub_paths = true;
        if self.commands[self.commands.len() - 1] == Cmd::Move {
            self.commands.pop();
            self.vertices.pop();
            self.start_index.pop();
        }
        self.commands.push(Cmd::Move);
        self.start_index.push(self.vertices.len());
        self.vertices.push(p);
    }

    pub fn curve4_to(&mut self, end: V3, c1: V3, c2: V3) {
        self.commands.push(Cmd::Curve4);
        self.start_index.push(self.vertices.len());
        self.vertices.extend([c1, c2, end]);
    }

    pub fn close(&mut self) {
        if !self.is_closed() {
            self.line_to(self.start());
        }
    }

    pub fn elems(&self) -> Vec<Elem> {
        self.commands
            .iter()
            .zip(&self.start_index)
            .map(|(c, &i)| match c {
                Cmd::Line => Elem::Line(self.vertices[i]),
                Cmd::Move => Elem::Move(self.vertices[i]),
                Cmd::Curve4 => Elem::Curve4(self.vertices[i + 2], self.vertices[i], self.vertices[i + 1]),
            })
            .collect()
    }

    pub fn append_elem(&mut self, e: &Elem) {
        match *e {
            Elem::Line(p) => self.line_to(p),
            Elem::Move(p) => self.move_to(p),
            Elem::Curve4(end, c1, c2) => self.curve4_to(end, c1, c2),
        }
    }

    pub fn reversed(&self) -> Path {
        let mut p = self.clone();
        if p.commands.is_empty() {
            return p;
        }
        if p.commands[p.commands.len() - 1] == Cmd::Move {
            p.commands.pop();
            p.vertices.pop();
            p.start_index.pop();
            p.has_sub_paths = p.commands.contains(&Cmd::Move);
        }
        p.commands.reverse();
        p.vertices.reverse();
        p.start_index = vertex_index(&p.commands);
        p
    }

    pub fn append_path(&mut self, other: &Path) {
        if other.is_empty() {
            return;
        }
        if !self.commands.is_empty() {
            if !self.end().isclose(other.start()) {
                self.line_to(other.start());
            }
        } else {
            self.set_start(other.start());
        }
        for e in other.elems() {
            self.append_elem(&e);
        }
    }

    pub fn extend_multi_path(&mut self, other: &Path) {
        if !other.is_empty() {
            self.move_to(other.start());
            for e in other.elems() {
                self.append_elem(&e);
            }
        }
    }

    pub fn to_wcs(&mut self, ocs: &Ocs, elevation: f64) {
        for v in &mut self.vertices {
            *v = ocs.to_wcs(v.with_z(elevation));
        }
    }

    pub fn transform(&self, m: &M44) -> Path {
        let mut p = self.clone();
        for v in &mut p.vertices {
            *v = m.transform(*v);
        }
        p
    }

    pub fn sub_paths(&self) -> Vec<Path> {
        let mut out = Vec::new();
        let mut path = Path::new(self.start());
        for e in self.elems() {
            if let Elem::Move(p) = e {
                out.push(std::mem::replace(&mut path, Path::new(p)));
            } else {
                path.append_elem(&e);
            }
        }
        out.push(path);
        out
    }

    pub fn flattening(&self, distance: f64) -> R<Vec<V3>> {
        let mut out = Vec::new();
        if self.commands.is_empty() {
            return Ok(out);
        }
        let mut start = self.vertices[0];
        out.push(start);
        for (&si, cmd) in self.start_index.iter().zip(&self.commands) {
            match cmd {
                Cmd::Line | Cmd::Move => {
                    start = self.vertices[si];
                    out.push(start);
                }
                Cmd::Curve4 => {
                    if distance == 0.0 {
                        return Err(Fail::Error);
                    }
                    let (c1, c2, e) = (self.vertices[si], self.vertices[si + 1], self.vertices[si + 2]);
                    let mut pts = Vec::new();
                    Bezier4::new([start, c1, c2, e]).flattening(distance, 4, &mut pts);
                    out.extend_from_slice(&pts[1..]);
                    start = e;
                }
            }
        }
        Ok(out)
    }
}

pub fn to_multi_path(paths: &[Path]) -> Path {
    let mut m = Path::new(NULLVEC);
    for p in paths {
        m.extend_multi_path(p);
    }
    m
}

pub fn from_vertices(vertices: &[V3], close: bool) -> Path {
    if vertices.len() < 2 {
        return Path::new(NULLVEC);
    }
    let mut p = Path::new(vertices[0]);
    for &v in &vertices[1..] {
        if !p.end().isclose(v) {
            p.line_to(v);
        }
    }
    if close {
        p.close();
    }
    p
}

pub fn add_bezier4p(path: &mut Path, curves: Vec<Bezier4>) {
    if curves.is_empty() {
        return;
    }
    let end = curves[curves.len() - 1].control_points()[3];
    let curves = if path.end().isclose(end) { reverse_bezier_curves(&curves) } else { curves };
    for c in &curves {
        let [s, c1, c2, e] = c.control_points();
        if !s.isclose(path.end()) {
            path.line_to(s);
        }
        if s.isclose_tol(c1, 1e-15, 0.0) && e.isclose_tol(c2, 1e-15, 0.0) {
            path.line_to(e);
        } else {
            path.curve4_to(e, c1, c2);
        }
    }
}

pub fn add_ellipse(path: &mut Path, e: &Ellipse, segments: usize) -> R<()> {
    if e.param_span().abs() < 1e-9 {
        return Ok(());
    }
    if path.is_empty() {
        path.set_start(e.start_point()?);
    }
    add_bezier4p(path, e.to_beziers(segments)?);
    Ok(())
}

pub fn add_spline(path: &mut Path, s: &BSpline) -> R<()> {
    if path.is_empty() {
        path.set_start(s.point(0.0)?);
    }
    let curves = if s.degree() == 3 && !s.is_rational() && s.clamped {
        s.bezier_decomposition()?.into_iter().map(Bezier4::new).collect()
    } else {
        s.cubic_bezier_approximation(4)?
    };
    add_bezier4p(path, curves);
    Ok(())
}

fn bulge_to(path: &mut Path, p1: V3, p2: V3, bulge: f64) -> R<()> {
    if p1.isclose_tol(p2, IS_CLOSE_TOL, 0.0) {
        return Ok(());
    }
    let (center, sa, ea, radius) = bulge_to_arc(p1, p2, bulge)?;
    let sa = py_mod(sa, TAU);
    let mut ea = py_mod(ea, TAU);
    if sa > ea {
        ea += TAU;
    }
    let e = Ellipse::from_arc(center.v3(), radius, Z_AXIS, degrees(sa), degrees(ea), true)?;
    let mut curves = e.to_beziers(1)?;
    if curves.is_empty() {
        return Err(Fail::Error);
    }
    if curves[0].control_points()[0].isclose_tol(p2, IS_CLOSE_TOL, 0.0) {
        curves = reverse_bezier_curves(&curves);
    }
    add_bezier4p(path, curves);
    Ok(())
}

pub fn add_2d_polyline(path: &mut Path, points: &[(f64, f64, f64)], close: bool, ocs: &Ocs, elevation: f64) -> R<()> {
    let mut prev: Option<V3> = None;
    let mut prev_bulge = 0.0;
    for &(x, y, b) in points {
        let b = if b.abs() < 1e-6 { 0.0 } else { b };
        let point = V3::xy(x, y);
        let Some(pp) = prev else {
            path.set_start(point);
            prev = Some(point);
            prev_bulge = b;
            continue;
        };
        if prev_bulge != 0.0 {
            bulge_to(path, pp, point, prev_bulge)?;
        } else {
            path.line_to(point);
        }
        prev = Some(point);
        prev_bulge = b;
    }
    if close && !path.start().isclose_tol(path.end(), IS_CLOSE_TOL, 0.0) {
        if prev_bulge != 0.0 {
            bulge_to(path, path.end(), path.start(), prev_bulge)?;
        } else {
            path.line_to(path.start());
        }
    }
    if ocs.transform() || elevation != 0.0 {
        path.to_wcs(ocs, elevation);
    }
    Ok(())
}
