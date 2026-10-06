use std::f64::consts::{PI, TAU};
use std::ops::{Add, Mul, Neg, Sub};

pub use crate::drawing::ops::py_mod;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fail {
    Error,
    NonUniform,
    InsertTransform,
}

pub type R<T> = Result<T, Fail>;

pub const ABS_TOL: f64 = 1e-12;

pub fn isclose(a: f64, b: f64, rel_tol: f64, abs_tol: f64) -> bool {
    if a == b {
        return true;
    }
    if a.is_infinite() || b.is_infinite() {
        return false;
    }
    let diff = (b - a).abs();
    diff <= (rel_tol * b).abs() || diff <= (rel_tol * a).abs() || diff <= abs_tol
}

pub fn isclose_def(a: f64, b: f64) -> bool {
    isclose(a, b, 1e-9, 0.0)
}

pub fn radians(d: f64) -> f64 {
    d * (PI / 180.0)
}

pub fn degrees(r: f64) -> f64 {
    r * (180.0 / PI)
}

pub fn div(a: f64, b: f64) -> R<f64> {
    if b == 0.0 { Err(Fail::Error) } else { Ok(a / b) }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct V3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

pub const fn v3(x: f64, y: f64, z: f64) -> V3 {
    V3 { x, y, z }
}

pub const NULLVEC: V3 = v3(0.0, 0.0, 0.0);
pub const X_AXIS: V3 = v3(1.0, 0.0, 0.0);
pub const Y_AXIS: V3 = v3(0.0, 1.0, 0.0);
pub const Z_AXIS: V3 = v3(0.0, 0.0, 1.0);

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f64> for V3 {
    type Output = V3;
    fn mul(self, s: f64) -> V3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        v3(-self.x, -self.y, -self.z)
    }
}

impl V3 {
    pub fn xy(x: f64, y: f64) -> V3 {
        v3(x, y, 0.0)
    }

    pub fn checked_div(self, s: f64) -> R<V3> {
        if s == 0.0 {
            return Err(Fail::Error);
        }
        Ok(v3(self.x / s, self.y / s, self.z / s))
    }

    pub fn magnitude_square(self) -> f64 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    pub fn magnitude(self) -> f64 {
        self.magnitude_square().powf(std::hint::black_box(0.5))
    }

    pub fn is_null(self) -> bool {
        self.x.abs() <= ABS_TOL && self.y.abs() <= ABS_TOL && self.z.abs() <= ABS_TOL
    }

    pub fn normalize_to(self, length: f64) -> R<V3> {
        Ok(self * div(length, self.magnitude())?)
    }

    pub fn normalize(self) -> R<V3> {
        self.normalize_to(1.0)
    }

    pub fn dot(self, o: V3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: V3) -> V3 {
        v3(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }

    pub fn distance(self, o: V3) -> f64 {
        (o - self).magnitude()
    }

    pub fn isclose_tol(self, o: V3, rel_tol: f64, abs_tol: f64) -> bool {
        isclose(self.x, o.x, rel_tol, abs_tol)
            && isclose(self.y, o.y, rel_tol, abs_tol)
            && isclose(self.z, o.z, rel_tol, abs_tol)
    }

    pub fn isclose(self, o: V3) -> bool {
        self.isclose_tol(o, 1e-9, 1e-12)
    }

    pub fn lerp(self, o: V3) -> V3 {
        self + (o - self) * 0.5
    }

    pub fn angle(self) -> f64 {
        self.y.atan2(self.x)
    }

    pub fn angle_deg(self) -> f64 {
        degrees(self.angle())
    }

    pub fn from_angle(angle: f64) -> V3 {
        v3(angle.cos(), angle.sin(), 0.0)
    }

    pub fn from_angle_len(angle: f64, length: f64) -> V3 {
        v3(angle.cos() * length, angle.sin() * length, 0.0)
    }

    pub fn from_deg_angle(angle: f64) -> V3 {
        V3::from_angle(radians(angle))
    }

    pub fn with_z(self, z: f64) -> V3 {
        v3(self.x, self.y, z)
    }

    pub fn orthogonal(self, ccw: bool) -> V3 {
        if ccw { v3(-self.y, self.x, self.z) } else { v3(self.y, -self.x, self.z) }
    }

    pub fn rotate_deg(self, angle: f64) -> V3 {
        let v = v3(self.x, self.y, 0.0);
        let r = V3::from_angle_len(v.angle() + radians(angle), v.magnitude());
        v3(r.x, r.y, self.z)
    }

    pub fn angle_between(self, o: V3) -> R<f64> {
        let c = self.normalize()?.dot(o.normalize()?).clamp(-1.0, 1.0);
        Ok(c.acos())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct V2 {
    pub x: f64,
    pub y: f64,
}

impl V2 {
    pub fn of(v: V3) -> V2 {
        V2 { x: v.x, y: v.y }
    }

    pub fn v3(self) -> V3 {
        v3(self.x, self.y, 0.0)
    }

    pub fn magnitude(self) -> f64 {
        crate::numeric::hypot(self.x, self.y)
    }

    pub fn angle(self) -> f64 {
        self.y.atan2(self.x)
    }

    pub fn distance(self, o: V2) -> f64 {
        crate::numeric::hypot(self.x - o.x, self.y - o.y)
    }

    pub fn is_null(self) -> bool {
        self.x.abs() <= ABS_TOL && self.y.abs() <= ABS_TOL
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct M44(pub [f64; 16]);

impl Default for M44 {
    fn default() -> Self {
        M44::identity()
    }
}

impl M44 {
    pub fn identity() -> M44 {
        let mut m = [0.0; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[15] = 1.0;
        M44(m)
    }

    pub fn ucs(ux: V3, uy: V3, uz: V3) -> M44 {
        M44([ux.x, ux.y, ux.z, 0.0, uy.x, uy.y, uy.z, 0.0, uz.x, uz.y, uz.z, 0.0, 0.0, 0.0, 0.0, 1.0])
    }

    pub fn translate(dx: f64, dy: f64, dz: f64) -> M44 {
        M44([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, dx, dy, dz, 1.0])
    }

    pub fn axis_rotate(axis: V3, angle: f64) -> R<M44> {
        let c = angle.cos();
        let s = angle.sin();
        let omc = 1.0 - c;
        let V3 { x, y, z } = axis.normalize()?;
        Ok(M44([
            x * x * omc + c,
            y * x * omc + z * s,
            x * z * omc - y * s,
            0.0,
            x * y * omc - z * s,
            y * y * omc + c,
            y * z * omc + x * s,
            0.0,
            x * z * omc + y * s,
            y * z * omc - x * s,
            z * z * omc + c,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ]))
    }

    pub fn matmul(&self, o: &M44) -> M44 {
        let mut r = [0.0; 16];
        for i in 0..4 {
            for j in 0..4 {
                let mut s = 0.0;
                for k in 0..4 {
                    s = self.0[i * 4 + k].mul_add(o.0[k * 4 + j], s);
                }
                r[i * 4 + j] = s;
            }
        }
        M44(r)
    }

    pub fn transform(&self, v: V3) -> V3 {
        let m = &self.0;
        v3(
            v.x * m[0] + v.y * m[4] + v.z * m[8] + m[12],
            v.x * m[1] + v.y * m[5] + v.z * m[9] + m[13],
            v.x * m[2] + v.y * m[6] + v.z * m[10] + m[14],
        )
    }

    pub fn transform_direction(&self, v: V3) -> V3 {
        let m = &self.0;
        v3(
            v.x * m[0] + v.y * m[4] + v.z * m[8],
            v.x * m[1] + v.y * m[5] + v.z * m[9],
            v.x * m[2] + v.y * m[6] + v.z * m[10],
        )
    }

    pub fn transform_array(&self, pts: &mut [V3]) {
        let m = &self.0;
        let single = pts.len() == 1;
        for p in pts.iter_mut() {
            let x = [p.x, p.y, p.z, 1.0];
            let mut r = [0.0; 3];
            for (j, rj) in r.iter_mut().enumerate() {
                let b = [m[j], m[4 + j], m[8 + j], m[12 + j]];
                *rj = if single {
                    x[2].mul_add(b[2], x[0] * b[0]) + x[3].mul_add(b[3], x[1] * b[1])
                } else {
                    let mut s = 0.0;
                    for k in 0..4 {
                        s = x[k].mul_add(b[k], s);
                    }
                    s
                };
            }
            *p = v3(r[0], r[1], r[2]);
        }
    }

    pub fn ucs_direction_from_wcs(&self, v: V3) -> V3 {
        let m = &self.0;
        v3(
            v.x * m[0] + v.y * m[1] + v.z * m[2],
            v.x * m[4] + v.y * m[5] + v.z * m[6],
            v.x * m[8] + v.y * m[9] + v.z * m[10],
        )
    }

    pub fn ux(&self) -> V3 {
        v3(self.0[0], self.0[1], self.0[2])
    }

    pub fn uy(&self) -> V3 {
        v3(self.0[4], self.0[5], self.0[6])
    }

    pub fn uz(&self) -> V3 {
        v3(self.0[8], self.0[9], self.0[10])
    }

    pub fn set_row3(&mut self, v: V3) {
        self.0[12] = v.x;
        self.0[13] = v.y;
        self.0[14] = v.z;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ocs {
    pub matrix: Option<M44>,
}

impl Ocs {
    pub fn wcs() -> Ocs {
        Ocs { matrix: None }
    }

    pub fn new(extrusion: V3) -> R<Ocs> {
        let az = extrusion.normalize()?;
        if az.isclose(Z_AXIS) {
            return Ok(Ocs { matrix: None });
        }
        let ax = if az.x.abs() < 1.0 / 64.0 && az.y.abs() < 1.0 / 64.0 { Y_AXIS.cross(az) } else { Z_AXIS.cross(az) };
        let ax = ax.normalize()?;
        let ay = az.cross(ax).normalize()?;
        Ok(Ocs { matrix: Some(M44::ucs(ax, ay, az)) })
    }

    pub fn transform(&self) -> bool {
        self.matrix.is_some()
    }

    pub fn ux(&self) -> V3 {
        self.matrix.map_or(X_AXIS, |m| m.ux())
    }

    pub fn uy(&self) -> V3 {
        self.matrix.map_or(Y_AXIS, |m| m.uy())
    }

    pub fn uz(&self) -> V3 {
        self.matrix.map_or(Z_AXIS, |m| m.uz())
    }

    pub fn to_wcs(&self, p: V3) -> V3 {
        match &self.matrix {
            Some(m) => m.transform_direction(p),
            None => p,
        }
    }

    pub fn from_wcs(&self, p: V3) -> V3 {
        match &self.matrix {
            Some(m) => m.ucs_direction_from_wcs(p),
            None => p,
        }
    }
}

pub fn arc_angle_span_deg(start: f64, end: f64) -> f64 {
    let tol = 1e-13;
    if isclose(start, end, 1e-9, tol) {
        return 0.0;
    }
    let start = py_mod(start, 360.0);
    if isclose(start, py_mod(end, 360.0), 1e-9, tol) {
        return 360.0;
    }
    let mut end = end;
    if !isclose(end, 360.0, 1e-9, tol) {
        end = py_mod(end, 360.0);
    }
    if end < start {
        end += 360.0;
    }
    end - start
}

pub fn arc_angle_span_rad(start: f64, end: f64) -> f64 {
    let tol = 1e-15;
    if isclose(start, end, 1e-9, tol) {
        return 0.0;
    }
    let start = py_mod(start, TAU);
    if isclose(start, py_mod(end, TAU), 1e-9, tol) {
        return TAU;
    }
    let mut end = end;
    if !isclose(end, TAU, 1e-9, tol) {
        end = py_mod(end, TAU);
    }
    if end < start {
        end += TAU;
    }
    end - start
}

pub fn transform_extrusion(extrusion: V3, m: &M44) -> R<(V3, bool)> {
    let ocs = Ocs::new(extrusion)?;
    let x = m.transform_direction(ocs.to_wcs(X_AXIS));
    let y = m.transform_direction(ocs.to_wcs(Y_AXIS));
    let uniform = isclose(x.magnitude_square(), y.magnitude_square(), 1e-9, 1e-9);
    Ok((x.cross(y).normalize()?, uniform))
}

pub struct OcsTransform {
    pub m: M44,
    pub old_ocs: Ocs,
    pub new_ocs: Ocs,
    pub scale_uniform: bool,
}

impl OcsTransform {
    pub fn new(extrusion: V3, m: &M44) -> R<OcsTransform> {
        let (new_extrusion, uniform) = transform_extrusion(extrusion, m)?;
        Ok(OcsTransform {
            m: *m,
            old_ocs: Ocs::new(extrusion)?,
            new_ocs: Ocs::new(new_extrusion)?,
            scale_uniform: uniform,
        })
    }

    pub fn from_ocs(old: Ocs, new: Ocs, m: &M44) -> OcsTransform {
        OcsTransform { m: *m, old_ocs: old, new_ocs: new, scale_uniform: true }
    }

    pub fn new_extrusion(&self) -> V3 {
        self.new_ocs.uz()
    }

    pub fn old_extrusion(&self) -> V3 {
        self.old_ocs.uz()
    }

    pub fn transform_length(&self, v: V3) -> f64 {
        self.m.transform_direction(self.old_ocs.to_wcs(v)).magnitude()
    }

    pub fn transform_width(&self, width: f64) -> f64 {
        let w = width.abs();
        if w > 1e-12 {
            crate::numeric::max(self.transform_length(v3(w, 0.0, 0.0)), self.transform_length(v3(0.0, w, 0.0)))
        } else {
            0.0
        }
    }

    pub fn transform_vertex(&self, v: V3) -> V3 {
        self.new_ocs.from_wcs(self.m.transform(self.old_ocs.to_wcs(v)))
    }

    pub fn transform_2d_vertex(&self, v: V2, elevation: f64) -> V2 {
        V2::of(self.transform_vertex(v3(v.x, v.y, elevation)))
    }

    pub fn transform_direction(&self, d: V3) -> V3 {
        self.new_ocs.from_wcs(self.m.transform_direction(self.old_ocs.to_wcs(d)))
    }

    pub fn transform_thickness(&self, t: f64) -> f64 {
        self.transform_direction(v3(0.0, 0.0, t)).z
    }

    pub fn transform_angle(&self, angle: f64) -> f64 {
        self.transform_direction(V3::from_angle(angle)).angle()
    }

    pub fn transform_deg_angle(&self, angle: f64) -> f64 {
        self.transform_angle(angle * (PI / 180.0)) * (180.0 / PI)
    }

    pub fn transform_ccw_arc_angles(&self, start: f64, end: f64) -> (f64, f64) {
        let mut old_span = arc_angle_span_rad(start, end);
        let new_start = self.transform_angle(start);
        let new_end = self.transform_angle(end);
        let new_span = if isclose_def(old_span, PI) {
            old_span = 1.0;
            let check = self.transform_angle(start + old_span);
            arc_angle_span_rad(new_start, check)
        } else if isclose_def(old_span, TAU) {
            return (new_start, new_start + TAU);
        } else {
            arc_angle_span_rad(new_start, new_end)
        };
        if isclose(old_span, new_span, 1e-8, 0.0) { (new_start, new_end) } else { (new_end, new_start) }
    }

    pub fn transform_ccw_arc_angles_deg(&self, start: f64, end: f64) -> (f64, f64) {
        let (s, e) = self.transform_ccw_arc_angles(start * (PI / 180.0), end * (PI / 180.0));
        (s * (180.0 / PI), e * (180.0 / PI))
    }
}
