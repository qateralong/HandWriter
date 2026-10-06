use std::f64::consts::{PI, TAU};

use super::geom::*;
use crate::numeric;

#[derive(Debug, Clone)]
pub struct Bezier4 {
    offset: V3,
    rel: [V3; 4],
}

impl Bezier4 {
    pub fn new(p: [V3; 4]) -> Bezier4 {
        let o = p[0];
        Bezier4 { offset: o, rel: [p[0] - o, p[1] - o, p[2] - o, p[3] - o] }
    }

    pub fn control_points(&self) -> [V3; 4] {
        let o = self.offset;
        [o, self.rel[1] + o, self.rel[2] + o, self.rel[3] + o]
    }

    pub fn reverse(&self) -> Bezier4 {
        let [a, b, c, d] = self.control_points();
        Bezier4::new([d, c, b, a])
    }

    fn point(&self, t: f64) -> V3 {
        let [_, p1, p2, p3] = self.rel;
        let t2 = t * t;
        let omt = 1.0 - t;
        let b = 3.0 * omt * omt * t;
        let c = 3.0 * omt * t2;
        let d = t2 * t;
        p1 * b + p2 * c + p3 * d + self.offset
    }

    pub fn flattening(&self, distance: f64, segments: usize, out: &mut Vec<V3>) {
        let mut stack: Vec<(f64, V3)> = Vec::new();
        let dt = 1.0 / segments as f64;
        let mut t0 = 0.0;
        let cp = self.control_points();
        let mut start_point = cp[0];
        out.push(start_point);
        while t0 < 1.0 {
            let mut t1 = t0 + dt;
            let mut end_point;
            if isclose_def(t1, 1.0) {
                end_point = cp[3];
                t1 = 1.0;
            } else {
                end_point = self.point(t1);
            }
            loop {
                let mid_t = (t0 + t1) * 0.5;
                let mid_point = self.point(mid_t);
                let chk = start_point.lerp(end_point);
                if chk.distance(mid_point) < distance {
                    out.push(end_point);
                    t0 = t1;
                    start_point = end_point;
                    match stack.pop() {
                        Some((t, e)) => {
                            t1 = t;
                            end_point = e;
                        }
                        None => break,
                    }
                } else {
                    stack.push((t1, end_point));
                    t1 = mid_t;
                    end_point = mid_point;
                }
            }
        }
    }
}

pub fn reverse_bezier_curves(curves: &[Bezier4]) -> Vec<Bezier4> {
    curves.iter().rev().map(|c| c.reverse()).collect()
}

fn cubic_bezier_arc_parameters(start_angle: f64, end_angle: f64, segments: usize) -> R<Vec<[V3; 4]>> {
    let delta = end_angle - start_angle;
    if delta <= 0.0 || delta.is_nan() {
        return Err(Fail::Error);
    }
    let arc_count = ((delta / PI * 2.0).ceil() as usize).max(segments);
    let seg = delta / arc_count as f64;
    let tl = 4.0 / 3.0 * (seg / 4.0).tan();
    let mut angle = start_angle;
    let mut end_point = V3::from_angle(angle);
    let mut out = Vec::with_capacity(arc_count);
    for _ in 0..arc_count {
        let sp = end_point;
        angle += seg;
        end_point = V3::from_angle(angle);
        let c1 = sp + v3(-sp.y * tl, sp.x * tl, 0.0);
        let c2 = end_point + v3(end_point.y * tl, -end_point.x * tl, 0.0);
        out.push([sp, c1, c2, end_point]);
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct Ellipse {
    pub center: V3,
    pub major_axis: V3,
    pub minor_axis: V3,
    pub extrusion: V3,
    pub ratio: f64,
    pub start_param: f64,
    pub end_param: f64,
}

pub fn minor_axis(major: V3, extrusion: V3, ratio: f64) -> R<V3> {
    extrusion.cross(major).normalize_to(major.magnitude() * ratio)
}

pub fn vertex(param: f64, major: V3, minor: V3, center: V3, ratio: f64) -> R<V3> {
    let xa = major.normalize()?;
    let ya = minor.normalize()?;
    let rx = major.magnitude();
    let ry = rx * ratio;
    let x = xa * (param.cos() * rx);
    let y = ya * (param.sin() * ry);
    Ok(center + x + y)
}

fn mid_param(start: f64, end: f64) -> f64 {
    let end = if end < start { end + TAU } else { end };
    (start + end) / 2.0
}

fn rytz(d1: V3, d2: V3) -> R<(V3, V3, f64)> {
    let q = d1;
    let p1 = if isclose(d1.z, 0.0, 1e-9, 1e-9) && isclose(d2.z, 0.0, 1e-9, 1e-9) {
        d2.orthogonal(false)
    } else {
        d1.cross(d2).cross(d2).normalize_to(d2.magnitude())?
    };
    let d = p1.lerp(q);
    let radius = d.magnitude();
    let rv = (q - p1).normalize_to(radius)?;
    let a = d - rv;
    let b = d + rv;
    if a.isclose(NULLVEC) || b.isclose(NULLVEC) {
        return Err(Fail::Error);
    }
    let major_len = (a - q).magnitude();
    let minor_len = (b - q).magnitude();
    if isclose_def(major_len, 0.0) || isclose_def(minor_len, 0.0) {
        return Err(Fail::Error);
    }
    Ok((b.normalize_to(major_len)?, a.normalize_to(minor_len)?, minor_len / major_len))
}

impl Ellipse {
    pub fn new(center: V3, major: V3, extrusion: V3, ratio: f64, start: f64, end: f64, ccw: bool) -> R<Ellipse> {
        if major.isclose(NULLVEC) {
            return Err(Fail::Error);
        }
        let (s, e) = if ccw { (start, end) } else { (end, start) };
        Ok(Ellipse {
            center,
            major_axis: major,
            minor_axis: minor_axis(major, extrusion, ratio)?,
            extrusion,
            ratio,
            start_param: s,
            end_param: e,
        })
    }

    pub fn from_arc(center: V3, radius: f64, extrusion: V3, start_angle: f64, end_angle: f64, ccw: bool) -> R<Ellipse> {
        let radius = radius.abs();
        if NULLVEC.isclose(extrusion) {
            return Err(Fail::Error);
        }
        let ocs = Ocs::new(extrusion)?;
        let c = ocs.to_wcs(center);
        let major = ocs.to_wcs(v3(radius, 0.0, 0.0));
        Ellipse::new(c, major, extrusion, 1.0, radians(start_angle), radians(end_angle), ccw)
    }

    pub fn start_point(&self) -> R<V3> {
        vertex(self.start_param, self.major_axis, self.minor_axis, self.center, self.ratio)
    }

    pub fn param_span(&self) -> f64 {
        arc_angle_span_rad(self.start_param, self.end_param)
    }

    pub fn swap_axis(&mut self) -> R<()> {
        self.major_axis = self.minor_axis;
        let ratio = div(1.0, self.ratio)?;
        self.ratio = numeric::max(ratio, 1e-6);
        self.minor_axis = minor_axis(self.major_axis, self.extrusion, self.ratio)?;
        if isclose_def(self.start_param, 0.0) && isclose_def(self.end_param, TAU) {
            return Ok(());
        }
        self.start_param = py_mod(self.start_param - PI / 2.0, TAU);
        self.end_param = py_mod(self.end_param - PI / 2.0, TAU);
        Ok(())
    }

    pub fn dxfattribs(&self) -> R<Ellipse> {
        let mut e = self.clone();
        if self.ratio > 1.0 {
            e = Ellipse::new(
                self.center,
                self.major_axis,
                self.extrusion,
                self.ratio,
                self.start_param,
                self.end_param,
                true,
            )?;
            e.swap_axis()?;
        }
        e.ratio = numeric::max(e.ratio, 1e-6);
        Ok(e)
    }

    pub fn transform(&mut self, m: &M44) -> R<()> {
        let new_center = m.transform(self.center);
        let (old_start, old_end) = (self.start_param, self.end_param);
        let (mut start, mut end) = (old_start, old_end);
        let old_minor = minor_axis(self.major_axis, self.extrusion, self.ratio)?;
        let mut new_major = m.transform_direction(self.major_axis);
        let mut new_minor = m.transform_direction(old_minor);
        let dot = new_major.normalize()?.dot(new_minor.normalize()?);
        let mut new_ratio;
        let new_extrusion;
        let adjust;
        if dot.abs() > 1e-6 {
            let (a, b, r) = rytz(new_major, new_minor)?;
            new_major = a;
            new_minor = b;
            new_ratio = r;
            new_extrusion = new_major.cross(new_minor).normalize()?;
            adjust = true;
        } else {
            new_ratio = div(new_minor.magnitude(), new_major.magnitude())?;
            new_extrusion = new_major.cross(new_minor).normalize()?;
            new_minor = minor_axis(new_major, new_extrusion, new_ratio)?;
            adjust = false;
        }
        if adjust && !isclose(start, end, 1e-9, 1e-9) {
            let xa = new_major.normalize()?;
            let ya = new_minor.normalize()?;
            let old_span = py_mod(end - start, TAU);
            let param = |v: V3| -> R<f64> {
                let dy = div(ya.dot(v), new_ratio)?;
                let dx = xa.dot(v);
                Ok(py_mod(dy.atan2(dx), TAU))
            };
            let sp = m.transform(vertex(start, self.major_axis, old_minor, self.center, self.ratio)?);
            let ep = m.transform(vertex(end, self.major_axis, old_minor, self.center, self.ratio)?);
            start = param(sp - new_center)?;
            end = param(ep - new_center)?;
            if !isclose(old_span, PI, 1e-9, 1e-9) {
                let new_span = py_mod(end - start, TAU);
                if !isclose(old_span, new_span, 1e-9, 1e-9) {
                    std::mem::swap(&mut start, &mut end);
                }
            } else {
                let old_chk = m.transform(vertex(
                    mid_param(old_start, old_end),
                    self.major_axis,
                    old_minor,
                    self.center,
                    self.ratio,
                )?);
                let new_chk = vertex(mid_param(start, end), new_major, new_minor, new_center, new_ratio)?;
                if !old_chk.isclose_tol(new_chk, 1e-9, 1e-9) {
                    std::mem::swap(&mut start, &mut end);
                }
            }
        }
        if new_ratio > 1.0 {
            new_major = minor_axis(new_major, new_extrusion, new_ratio)?;
            new_ratio = 1.0 / new_ratio;
            new_minor = minor_axis(new_major, new_extrusion, new_ratio)?;
            if !(isclose_def(start, 0.0) && isclose_def(end, TAU)) {
                start -= PI / 2.0;
                end -= PI / 2.0;
            }
        }
        start = py_mod(start, TAU);
        end = py_mod(end, TAU);
        if isclose_def(start, end) {
            start = 0.0;
            end = TAU;
        }
        self.center = new_center;
        self.major_axis = new_major;
        self.minor_axis = new_minor;
        self.extrusion = new_extrusion;
        self.ratio = new_ratio;
        self.start_param = start;
        self.end_param = end;
        Ok(())
    }

    pub fn to_beziers(&self, segments: usize) -> R<Vec<Bezier4>> {
        let span = self.param_span();
        if span.abs() < 1e-9 {
            return Ok(Vec::new());
        }
        let start = py_mod(self.start_param, TAU);
        let mut end = start + span;
        while start > end {
            end += TAU;
        }
        let (c, xa, ya) = (self.center, self.major_axis, self.minor_axis);
        Ok(cubic_bezier_arc_parameters(start, end, segments)?
            .into_iter()
            .map(|d| Bezier4::new(d.map(|p| c + xa * p.x + ya * p.y)))
            .collect())
    }
}

pub fn angle_to_param(ratio: f64, angle: f64) -> R<f64> {
    Ok(py_mod(div(angle.sin(), ratio)?.atan2(angle.cos()), TAU))
}

pub fn param_to_angle(ratio: f64, param: f64) -> f64 {
    (param.sin() * ratio).atan2(param.cos())
}

pub fn bulge_to_arc(p1: V3, p2: V3, bulge: f64) -> R<(V2, f64, f64, f64)> {
    let (s, e) = (V2::of(p1), V2::of(p2));
    let r = div(s.distance(e) * (1.0 + bulge * bulge) / 4.0, bulge)?;
    let ang = |a: V2, b: V2| V2 { x: b.x - a.x, y: b.y - a.y }.angle();
    let a = ang(s, e) + (PI / 2.0 - bulge.atan() * 2.0);
    let c = V2 { x: s.x + a.cos() * r, y: s.y + a.sin() * r };
    if bulge < 0.0 { Ok((c, ang(c, e), ang(c, s), r.abs())) } else { Ok((c, ang(c, s), ang(c, e), r.abs())) }
}

#[derive(Debug, Clone)]
pub struct BSpline {
    pub cps: Vec<V3>,
    pub knots: Vec<f64>,
    pub weights: Vec<f64>,
    pub order: usize,
    pub clamped: bool,
}

fn bisect_right(a: &[f64], x: f64, mut lo: usize, mut hi: usize) -> usize {
    while lo < hi {
        let mid = (lo + hi) / 2;
        if x < a[mid] { hi = mid } else { lo = mid + 1 }
    }
    lo
}

pub fn round_knots(knots: &[f64], tolerance: f64) -> Vec<f64> {
    if tolerance <= 0.0 || tolerance.is_nan() {
        return knots.to_vec();
    }
    let l = tolerance.log10();
    if !l.is_finite() {
        return knots.to_vec();
    }
    let nd = -(l.trunc() as i64);
    if nd <= 0 {
        return knots.to_vec();
    }
    knots.iter().map(|&k| numeric::round_to(k, nd as usize)).collect()
}

impl BSpline {
    pub fn new(cps: Vec<V3>, order: usize, knots: Option<Vec<f64>>, weights: Option<Vec<f64>>) -> R<BSpline> {
        let count = cps.len();
        if order > count {
            return Err(Fail::Error);
        }
        let knots = match knots {
            None => {
                let k = count - order;
                let max_value = (count - order + 1) as f64;
                let mut v = vec![0.0; order];
                v.extend((0..k).map(|i| (1.0 + i as f64) / max_value));
                v.extend(std::iter::repeat_n(1.0, order));
                v
            }
            Some(k) => {
                if k.len() != count + order {
                    return Err(Fail::Error);
                }
                if k[0] != 0.0 {
                    let min = k[0];
                    let max = k[k.len() - 1] - min;
                    k.iter().map(|&v| div(v - min, max)).collect::<R<Vec<f64>>>()?
                } else {
                    k
                }
            }
        };
        let weights = weights.unwrap_or_default();
        if !weights.is_empty() && weights.len() != count {
            return Err(Fail::Error);
        }
        let all_eq = |s: &[f64]| s.iter().all(|&v| v == s[0]);
        let clamped = all_eq(&knots[..order]) && all_eq(&knots[knots.len() - order..]);
        Ok(BSpline { cps, knots, weights, order, clamped })
    }

    pub fn degree(&self) -> usize {
        self.order - 1
    }

    pub fn is_rational(&self) -> bool {
        !self.weights.is_empty()
    }

    fn count(&self) -> usize {
        self.cps.len()
    }

    fn max_t(&self) -> f64 {
        self.knots[self.knots.len() - 1]
    }

    fn find_span(&self, u: f64) -> i64 {
        let (knots, count) = (&self.knots, self.count());
        if u >= knots[count] {
            return count as i64 - 1;
        }
        let p = self.order - 1;
        if knots[p] == 0.0 {
            bisect_right(knots, u, p, count) as i64 - 1
        } else {
            let mut span = 0;
            while knots[span] <= u && span < count {
                span += 1;
            }
            span as i64 - 1
        }
    }

    fn knot(&self, i: i64) -> R<f64> {
        let n = self.knots.len() as i64;
        let j = if i < 0 { i + n } else { i };
        if j < 0 || j >= n { Err(Fail::Error) } else { Ok(self.knots[j as usize]) }
    }

    fn basis_funcs(&self, span: i64, u: f64) -> R<Vec<f64>> {
        let order = self.order;
        let mut n = vec![0.0; order];
        let mut left = vec![0.0; order];
        let mut right = vec![0.0; order];
        n[0] = 1.0;
        for j in 1..order {
            left[j] = u - self.knot((span + 1 - j as i64).max(0))?;
            right[j] = self.knot(span + j as i64)? - u;
            let mut saved = 0.0;
            for r in 0..j {
                let temp = div(n[r], right[r + 1] + left[j - r])?;
                n[r] = saved + right[r + 1] * temp;
                saved = left[j - r] * temp;
            }
            n[j] = saved;
        }
        if self.is_rational() {
            let len = self.weights.len() as i64;
            let norm = |i: i64| if i < 0 { (i + len).max(0) } else { i.min(len) };
            let (lo, hi) = (norm(span - order as i64 + 1), norm(span + 1));
            let w: &[f64] = if lo < hi { &self.weights[lo as usize..hi as usize] } else { &[] };
            if w.len() != n.len() {
                return Ok(n);
            }
            let products: Vec<f64> = n.iter().zip(w).map(|(a, b)| a * b).collect();
            let s = numeric::sum(products.iter().copied());
            return Ok(if s == 0.0 { n } else { products.iter().map(|p| p / s).collect() });
        }
        Ok(n)
    }

    pub fn basis_vector(&self, t: f64) -> R<Vec<f64>> {
        let span = self.find_span(t);
        let p = self.order as i64 - 1;
        let front = (span - p).max(0) as usize;
        let back = (self.count() as i64 - span - 1).max(0) as usize;
        let mut v = vec![0.0; front];
        v.extend(self.basis_funcs(span, t)?);
        v.extend(std::iter::repeat_n(0.0, back));
        Ok(v)
    }

    pub fn point(&self, u: f64) -> R<V3> {
        let mut u = u;
        let max_t = self.max_t();
        if isclose_def(u, max_t) {
            u = max_t;
        }
        let p = self.degree() as i64;
        let span = self.find_span(u);
        let n = self.basis_funcs(span, u)?;
        let len = self.cps.len() as i64;
        let mut s = NULLVEC;
        for (i, w) in n.iter().enumerate().take(p as usize + 1) {
            let mut k = span - p + i as i64;
            if k < 0 {
                k += len;
            }
            if k < 0 || k >= len {
                return Err(Fail::Error);
            }
            s = s + self.cps[k as usize] * *w;
        }
        Ok(s)
    }

    pub fn bezier_decomposition(&self) -> R<Vec<[V3; 4]>> {
        let n = self.count() - 1;
        let p = self.degree();
        let knots = &self.knots;
        let cps = &self.cps;
        let mut alphas = vec![0.0; knots.len()];
        let m = n + p + 1;
        let mut a = p;
        let mut b = p + 1;
        let mut bez: Vec<V3> = cps[0..p + 1].to_vec();
        let mut out = Vec::new();
        while b < m {
            let mut next = vec![NULLVEC; p + 1];
            let i = b;
            while b < m && isclose_def(knots[b + 1], knots[b]) {
                b += 1;
            }
            let mult = b - i + 1;
            if mult < p {
                let numer = knots[b] - knots[a];
                for j in (mult + 1..=p).rev() {
                    alphas[j - mult - 1] = div(numer, knots[a + j] - knots[a])?;
                }
                let r = p - mult;
                for j in 1..=r {
                    let save = r - j;
                    let s = mult + j;
                    for k in (s..=p).rev() {
                        let alpha = alphas[k - s];
                        bez[k] = bez[k] * alpha + bez[k - 1] * (1.0 - alpha);
                    }
                    if b < m {
                        next[save] = bez[p];
                    }
                }
            }
            out.push([bez[0], bez[1], bez[2], bez[3]]);
            if b < m {
                for i in p - mult.min(p)..=p {
                    next[i] = cps[b - p + i];
                }
                a = b;
                b += 1;
                bez = next;
            }
        }
        Ok(out)
    }

    pub fn cubic_bezier_approximation(&self, level: usize) -> R<Vec<Bezier4>> {
        let mut params = distance_t_vector(&self.cps);
        if !params.is_empty() {
            let max_t = self.max_t();
            if max_t != 1.0 {
                params = params.iter().map(|p| p * max_t).collect();
            }
            for _ in 0..level.saturating_sub(1) {
                let mut q = Vec::with_capacity(params.len() * 2);
                for i in 0..params.len() - 1 {
                    q.push(params[i]);
                    q.push((params[i] + params[i + 1]) / 2.0);
                }
                q.push(params[params.len() - 1]);
                params = q;
            }
        }
        let pts = params.iter().map(|&t| self.point(t)).collect::<R<Vec<V3>>>()?;
        Ok(cubic_bezier_interpolation(&pts))
    }
}

pub fn distance_t_vector(points: &[V3]) -> Vec<f64> {
    let d: Vec<f64> = points.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total = numeric::sum(d.iter().copied());
    if total.abs() <= 1e-12 {
        return Vec::new();
    }
    let mut params = vec![0.0];
    let mut s = 0.0;
    for v in &d[..d.len() - 1] {
        s += v;
        params.push(s / total);
    }
    params.push(1.0);
    params
}

fn solve_tridiagonal(a: &[f64], b: &[f64], c: &[f64], r: &[f64]) -> Vec<f64> {
    let n = a.len();
    let mut u = vec![0.0; n];
    let mut gam = vec![0.0; n];
    let mut bet = b[0];
    u[0] = r[0] / bet;
    for j in 1..n {
        gam[j] = c[j - 1] / bet;
        bet = b[j] - a[j] * gam[j];
        u[j] = (r[j] - a[j] * u[j - 1]) / bet;
    }
    for j in (0..n.saturating_sub(1)).rev() {
        u[j] -= gam[j + 1] * u[j + 1];
    }
    u
}

fn by_columns(points: &[V3], solve: impl Fn(&[f64]) -> R<Vec<f64>>) -> R<Vec<V3>> {
    let xs = solve(&points.iter().map(|p| p.x).collect::<Vec<_>>())?;
    let ys = solve(&points.iter().map(|p| p.y).collect::<Vec<_>>())?;
    let zs = solve(&points.iter().map(|p| p.z).collect::<Vec<_>>())?;
    Ok((0..xs.len()).map(|i| v3(xs[i], ys[i], zs[i])).collect())
}

pub fn cubic_bezier_interpolation(pnts: &[V3]) -> Vec<Bezier4> {
    if pnts.len() < 3 {
        return Vec::new();
    }
    let num = pnts.len() - 1;
    let mut b = vec![4.0; num];
    let mut a = vec![1.0; num];
    let c = vec![1.0; num];
    b[0] = 2.0;
    b[num - 1] = 7.0;
    a[num - 1] = 2.0;
    let mut pv = vec![pnts[0] + pnts[1] * 2.0];
    pv.extend((1..num - 1).map(|i| (pnts[i] * 2.0 + pnts[i + 1]) * 2.0));
    pv.push(pnts[num - 1] * 8.0 + pnts[num]);
    let cp1 = by_columns(&pv, |col| Ok(solve_tridiagonal(&a, &b, &c, col))).expect("tridiagonal");
    let mut cp2: Vec<V3> = pnts[1..].iter().zip(&cp1[1..]).map(|(p, cp)| *p * 2.0 - *cp).collect();
    cp2.push(v3(
        (cp1[num - 1].x + pnts[num].x) / 2.0,
        (cp1[num - 1].y + pnts[num].y) / 2.0,
        (cp1[num - 1].z + pnts[num].z) / 2.0,
    ));
    (0..num).map(|i| Bezier4::new([pnts[i], cp1[i], cp2[i], pnts[i + 1]])).collect()
}

#[allow(clippy::needless_range_loop)]
fn lu_solve_dense(a: &[Vec<f64>], b: &[f64]) -> R<Vec<f64>> {
    let n = a.len();
    let mut m: Vec<Vec<f64>> = a.to_vec();
    let mut x = b.to_vec();
    for k in 0..n {
        let mut piv = k;
        for i in k + 1..n {
            if m[i][k].abs() > m[piv][k].abs() {
                piv = i;
            }
        }
        if m[piv][k] == 0.0 {
            return Err(Fail::Error);
        }
        m.swap(k, piv);
        x.swap(k, piv);
        for i in k + 1..n {
            let f = m[i][k] / m[k][k];
            for j in k..n {
                m[i][j] -= f * m[k][j];
            }
            x[i] -= f * x[k];
        }
    }
    for i in (0..n).rev() {
        let mut s = x[i];
        for j in i + 1..n {
            s -= m[i][j] * x[j];
        }
        x[i] = s / m[i][i];
    }
    Ok(x)
}

#[allow(clippy::needless_range_loop)]
fn banded_solve(a: &[Vec<f64>], b: &[f64]) -> R<Vec<f64>> {
    let n = a.len();
    let diag_any = |d: i64| {
        (0..n).any(|i| {
            let j = i as i64 + d;
            j >= 0 && (j as usize) < n && a[i][j as usize] != 0.0
        })
    };
    let mut m1 = 0;
    for d in 1..n {
        if diag_any(-(d as i64)) { m1 = d } else { break }
    }
    let mut m2 = 0;
    for d in 1..n {
        if diag_any(d as i64) { m2 = d } else { break }
    }
    let mm = m1 + m2 + 1;
    let mut upper: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            (0..mm)
                .map(|c| {
                    let j = i as i64 + c as i64 - m1 as i64;
                    if j >= 0 && (j as usize) < n { a[i][j as usize] } else { 0.0 }
                })
                .collect()
        })
        .collect();
    let mut lower = vec![vec![0.0; m1]; n];
    let mut index = vec![0usize; n];
    let mut l = m1;
    for i in 0..m1.min(n) {
        for j in m1 - i..mm {
            upper[i][j - l] = upper[i][j];
        }
        l -= 1;
        for j in mm - l - 1..mm {
            upper[i][j] = 0.0;
        }
    }
    l = m1;
    for k in 0..n {
        let mut dum = upper[k][0];
        let mut i = k;
        if l < n {
            l += 1;
        }
        for j in k + 1..l {
            if upper[j][0].abs() > dum.abs() {
                dum = upper[j][0];
                i = j;
            }
        }
        index[k] = i + 1;
        if i != k {
            upper.swap(k, i);
        }
        for i in k + 1..l {
            let dum = div(upper[i][0], upper[k][0])?;
            lower[k][i - k - 1] = dum;
            for j in 1..mm {
                upper[i][j - 1] = upper[i][j] - dum * upper[k][j];
            }
            upper[i][mm - 1] = 0.0;
        }
    }
    let mut x = b.to_vec();
    l = m1;
    for k in 0..n {
        let j = index[k] - 1;
        if j != k {
            x.swap(k, j);
        }
        if l < n {
            l += 1;
        }
        for j in k + 1..l {
            x[j] -= lower[k][j - k - 1] * x[k];
        }
    }
    l = 1;
    for i in (0..n).rev() {
        let mut dum = x[i];
        for k in 1..l {
            dum -= upper[i][k] * x[k + i];
        }
        x[i] = div(dum, upper[i][0])?;
        if l < mm {
            l += 1;
        }
    }
    Ok(x)
}

fn solve_rows(rows: &[Vec<f64>], b: &[V3]) -> R<Vec<V3>> {
    if rows.len() != b.len() || rows.iter().any(|r| r.len() != rows.len()) {
        return Err(Fail::Error);
    }
    if rows.len() < 20 {
        by_columns(b, |col| lu_solve_dense(rows, col))
    } else {
        by_columns(b, |col| banded_solve(rows, col))
    }
}

fn natural_knots_constrained(n: usize, p: usize, t: &[f64]) -> R<Vec<f64>> {
    if p + 1 > n + 1 || t.is_empty() || t[0] != 0.0 || !isclose_def(t[t.len() - 1], 1.0) {
        return Err(Fail::Error);
    }
    let mut k = vec![0.0; p + 1];
    let hi = (n - p + 1).min(t.len());
    if hi > 1 {
        k.extend_from_slice(&t[1..hi]);
    }
    k.extend(std::iter::repeat_n(1.0, p + 1));
    Ok(k)
}

fn cad_fit_point_interpolation(fit: &[V3]) -> R<BSpline> {
    let t = distance_t_vector(fit);
    let n = fit.len() - 1;
    let p = 3;
    let knots = natural_knots_constrained(n + 2, p, &t)?;
    let basis =
        BSpline { cps: vec![NULLVEC; n + 3], knots: knots.clone(), weights: vec![], order: p + 1, clamped: true };
    let mut rows = t.iter().map(|&u| basis.basis_vector(u)).collect::<R<Vec<_>>>()?;
    let pf = p as f64;
    let up1 = knots[p + 1];
    let up2 = knots[p + 2];
    let f = pf * (pf - 1.0) / up1;
    let c1 = vec![f / up1, -f * (up1 + up2) / (up1 * up2), f / up2];
    let mk = knots.len() - 1;
    let ump1 = knots[mk - p - 1];
    let ump2 = knots[mk - p - 2];
    let f2 = pf * (pf - 1.0) / (1.0 - ump1);
    let c2 = vec![f2 / (1.0 - ump2), -f2 * (2.0 - ump1 - ump2) / (1.0 - ump1) / (1.0 - ump2), f2 / (1.0 - ump1)];
    let spacing = vec![0.0; n];
    let mut r1 = c1;
    r1.extend(&spacing);
    let mut r2 = spacing;
    r2.extend(c2);
    rows.insert(1, r1);
    let li = rows.len() - 1;
    rows.insert(li, r2);
    let mut pts = fit.to_vec();
    pts.insert(1, NULLVEC);
    let li = pts.len() - 1;
    pts.insert(li, NULLVEC);
    let cps = solve_rows(&rows, &pts)?;
    BSpline::new(cps, 4, Some(knots), None)
}

pub fn fit_points_to_cad_cv(fit: &[V3], tangents: Option<(V3, V3)>) -> R<BSpline> {
    if fit.len() < 2 {
        return Err(Fail::Error);
    }
    let Some((ts, te)) = tangents else {
        return cad_fit_point_interpolation(fit);
    };
    let total = numeric::sum(fit.windows(2).map(|w| w[0].distance(w[1])));
    let st = ts.normalize_to(total)?;
    let et = te.normalize_to(total)?;
    let t = distance_t_vector(fit);
    let n = fit.len() - 1;
    let p = 3;
    let knots = natural_knots_constrained(n + 2, p, &t)?;
    let basis =
        BSpline { cps: vec![NULLVEC; n + 3], knots: knots.clone(), weights: vec![], order: p + 1, clamped: true };
    let mut rows = t.iter().map(|&u| basis.basis_vector(u)).collect::<R<Vec<_>>>()?;
    let spacing = vec![0.0; n + 1];
    let mut r1 = vec![-1.0, 1.0];
    r1.extend(&spacing);
    let mut r2 = spacing;
    r2.extend([-1.0, 1.0]);
    rows.insert(1, r1);
    let li = rows.len() - 1;
    rows.insert(li, r2);
    let mut pts = fit.to_vec();
    pts.insert(1, st * (knots[p + 1] / p as f64));
    let li = pts.len() - 1;
    pts.insert(li, et * ((1.0 - knots[knots.len() - (p + 2)]) / p as f64));
    let cps = solve_rows(&rows, &pts)?;
    BSpline::new(cps, 4, Some(knots), None)
}
