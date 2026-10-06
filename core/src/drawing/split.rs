use indexmap::IndexMap;

use crate::drawing::passes::covered_mask;
use crate::drawing::place::Rect;
use crate::geometry::{Point, polyline_length};
use crate::numeric::{dist, hypot};

pub const CURVE_WEIGHT: f64 = 4.0;
pub const ALONG_WEIGHT: f64 = 50.0;
pub const CENTER_WEIGHT: f64 = 0.05;
pub const CURVE_TURN: (f64, f64) = (0.3, 35.0);
pub const CURVE_SEG_MAX: f64 = 15.0;
pub const GRID_STEP: f64 = 0.25;
pub const TRIES: usize = 3;

fn np_max(a: f64, b: f64) -> f64 {
    if a >= b || a.is_nan() { a } else { b }
}

fn np_min(a: f64, b: f64) -> f64 {
    if a <= b || a.is_nan() { a } else { b }
}

pub type Clipped = (Vec<Point>, Vec<Point>, Vec<usize>);

pub fn clip_segments(a: &[Point], b: &[Point], q: Rect, eps: f64) -> Clipped {
    let (x0, y0, x1, y1) = q;
    let (mut ca, mut cb, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    for (i, (&pa, &pb)) in a.iter().zip(b).enumerate() {
        let d = (pb.0 - pa.0, pb.1 - pa.1);
        let (mut t0, mut t1) = (0.0, 1.0);
        let mut valid = true;
        for (p, qq) in [(-d.0, pa.0 - x0), (d.0, x1 - pa.0), (-d.1, pa.1 - y0), (d.1, y1 - pa.1)] {
            let qq = qq + eps;
            let zero = p.abs() < 1e-15;
            if zero && qq < 0.0 {
                valid = false;
            }
            let r = if zero { 0.0 } else { qq / p };
            if p < 0.0 && !zero {
                t0 = np_max(t0, r);
            }
            if p > 0.0 && !zero {
                t1 = np_min(t1, r);
            }
        }
        if valid && t0 <= t1 + 1e-12 {
            ca.push((pa.0 + d.0 * t0, pa.1 + d.1 * t0));
            cb.push((pa.0 + d.0 * t1, pa.1 + d.1 * t1));
            idx.push(i);
        }
    }
    (ca, cb, idx)
}

fn turn(a: Point, b: Point, c: Point) -> Option<f64> {
    let v1 = (b.0 - a.0, b.1 - a.1);
    let v2 = (c.0 - b.0, c.1 - b.1);
    if hypot(v1.0, v1.1) == 0.0 || hypot(v2.0, v2.1) == 0.0 {
        return None;
    }
    let cr = v1.0 * v2.1 - v1.1 * v2.0;
    let dt = v1.0 * v2.0 + v1.1 * v2.1;
    Some(cr.atan2(dt).to_degrees().abs())
}

pub fn curved_segments(st: &[Point]) -> Vec<bool> {
    if st.len() < 2 {
        return Vec::new();
    }
    let n = st.len() - 1;
    let closed = n >= 3 && st[0] == st[n];
    let mut ang: Vec<Option<f64>> = vec![None; n + 1];
    for i in 1..n {
        ang[i] = turn(st[i - 1], st[i], st[i + 1]);
    }
    if closed {
        let a = turn(st[n - 1], st[0], st[1]);
        ang[0] = a;
        ang[n] = a;
    }
    let ok = |a: f64| CURVE_TURN.0 <= a && a <= CURVE_TURN.1;
    (0..n)
        .map(|i| {
            let ends: Vec<f64> = [ang[i], ang[i + 1]].into_iter().flatten().collect();
            dist(st[i], st[i + 1]) <= CURVE_SEG_MAX && !ends.is_empty() && ends.iter().all(|&a| ok(a))
        })
        .collect()
}

pub struct Geometry {
    pub a: Vec<Point>,
    pub b: Vec<Point>,
    pub seg_path: Vec<usize>,
    pub curved: Vec<bool>,
    pub seg_len: Vec<f64>,
    pub path_len: Vec<f64>,
}

fn search_right(cands: &[f64], v: f64) -> usize {
    cands.partition_point(|&c| c <= v)
}

fn search_left(cands: &[f64], v: f64) -> usize {
    cands.partition_point(|&c| c < v)
}

impl Geometry {
    pub fn new(strokes: &[Vec<Point>]) -> Self {
        let mut g = Geometry {
            a: Vec::new(),
            b: Vec::new(),
            seg_path: Vec::new(),
            curved: Vec::new(),
            seg_len: Vec::new(),
            path_len: strokes.iter().map(|s| polyline_length(s)).collect(),
        };
        for (k, st) in strokes.iter().enumerate() {
            if st.len() == 1 {
                g.a.push(st[0]);
                g.b.push(st[0]);
                g.seg_path.push(k);
                g.curved.push(false);
                continue;
            }
            let flags = curved_segments(st);
            for i in 0..st.len().saturating_sub(1) {
                g.a.push(st[i]);
                g.b.push(st[i + 1]);
                g.seg_path.push(k);
                g.curved.push(flags[i]);
            }
        }
        g.seg_len = g.a.iter().zip(&g.b).map(|(p, q)| (q.0 - p.0).hypot(q.1 - p.1)).collect();
        g
    }

    pub fn covered(&self, q: Rect, rects: &[Rect]) -> bool {
        let (a, b, _) = clip_segments(&self.a, &self.b, q, 1e-9);
        if a.is_empty() {
            return true;
        }
        let segs: Vec<(Point, Point)> = a.into_iter().zip(b).collect();
        covered_mask(&segs, rects).into_iter().all(|c| c)
    }

    pub fn seam_cost(&self, q: Rect, axis: usize, cands: &[f64], ov: f64) -> Vec<f64> {
        let mut cost = vec![0.0; cands.len() + 1];
        let (a, b, idx) = clip_segments(&self.a, &self.b, q, 1e-9);
        if !a.is_empty() {
            let coord = |p: Point| if axis == 0 { p.0 } else { p.1 };
            let lo: Vec<f64> = a.iter().zip(&b).map(|(&p, &q)| np_min(coord(p), coord(q))).collect();
            let hi: Vec<f64> = a.iter().zip(&b).map(|(&p, &q)| np_max(coord(p), coord(q))).collect();
            let w: Vec<f64> = idx
                .iter()
                .map(|&i| self.path_len[self.seg_path[i]] * if self.curved[i] { CURVE_WEIGHT } else { 1.0 })
                .collect();
            let i0: Vec<usize> = lo.iter().map(|&v| search_right(cands, v)).collect();
            let i1: Vec<usize> =
                (0..lo.len()).map(|k| if hi[k] - lo[k] < 1e-12 { i0[k] } else { search_right(cands, hi[k]) }).collect();
            for k in 0..w.len() {
                cost[i0[k]] += w[k];
            }
            for k in 0..w.len() {
                cost[i1[k]] += -w[k];
            }
            let seglen: Vec<f64> = a.iter().zip(&b).map(|(p, q)| (q.0 - p.0).hypot(q.1 - p.1)).collect();
            let along: Vec<usize> = (0..lo.len()).filter(|&k| hi[k] - lo[k] < 2.0 * ov + 1e-9).collect();
            for &k in &along {
                cost[search_left(cands, lo[k] - 2.0 * ov)] += seglen[k] * ALONG_WEIGHT;
            }
            for &k in &along {
                cost[search_right(cands, hi[k] + 2.0 * ov)] += -(seglen[k] * ALONG_WEIGHT);
            }
        }
        let mut acc = 0.0;
        let mut out = Vec::with_capacity(cands.len());
        for &c in &cost[..cands.len()] {
            acc += c;
            out.push(acc);
        }
        let mid = (cands[0] + cands[cands.len() - 1]) / 2.0;
        out.iter().zip(cands).map(|(&v, &c)| v + CENTER_WEIGHT * (c - mid).abs()).collect()
    }

    pub fn distance_to(&self, pts: &[Point]) -> Vec<f64> {
        if self.a.is_empty() {
            return vec![f64::INFINITY; pts.len()];
        }
        pts.iter()
            .map(|&p| {
                let mut best: Option<f64> = None;
                for (&a, &b) in self.a.iter().zip(&self.b) {
                    let d = (b.0 - a.0, b.1 - a.1);
                    let l2 = d.0 * d.0 + d.1 * d.1;
                    let mut t = if l2 > 0.0 { ((p.0 - a.0) * d.0 + (p.1 - a.1) * d.1) / l2 } else { 0.0 };
                    t = np_min(np_max(t, 0.0), 1.0);
                    let e = ((a.0 + d.0 * t) - p.0, (a.1 + d.1 * t) - p.1);
                    let v = (e.0 * e.0 + e.1 * e.1).sqrt();
                    best = Some(match best {
                        None => v,
                        Some(m) if m.is_nan() => m,
                        Some(m) => {
                            if v < m || v.is_nan() {
                                v
                            } else {
                                m
                            }
                        }
                    });
                }
                best.expect("non-empty geometry")
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub core: Rect,
    pub ext: Rect,
    pub rotation: Option<i64>,
    pub axis: usize,
    pub s: f64,
    pub low: Option<Box<Node>>,
    pub high: Option<Box<Node>>,
    pub cost: f64,
}

fn coord(p: Point, axis: usize) -> f64 {
    if axis == 0 { p.0 } else { p.1 }
}

fn rect_get(q: Rect, i: usize) -> f64 {
    [q.0, q.1, q.2, q.3][i]
}

impl Node {
    fn leaf(core: Rect, ext: Rect, rotation: i64) -> Self {
        Node { core, ext, rotation: Some(rotation), axis: 0, s: 0.0, low: None, high: None, cost: 0.0 }
    }

    fn children(&self) -> (&Node, &Node) {
        (self.low.as_deref().expect("inner node"), self.high.as_deref().expect("inner node"))
    }

    pub fn leaf_at(&self, p: Point) -> &Node {
        let mut n = self;
        while n.rotation.is_none() {
            let (lo, hi) = n.children();
            n = if coord(p, n.axis) <= n.s { lo } else { hi };
        }
        n
    }

    pub fn leaves(&self) -> Vec<&Node> {
        if self.rotation.is_some() {
            return vec![self];
        }
        let (lo, hi) = self.children();
        let mut v = lo.leaves();
        v.extend(hi.leaves());
        v
    }

    pub fn seams(&self) -> Vec<&Node> {
        if self.rotation.is_some() {
            return Vec::new();
        }
        let (lo, hi) = self.children();
        let mut v = vec![self];
        v.extend(lo.seams());
        v.extend(hi.seams());
        v
    }
}

fn with_bounds(q: Rect, axis: usize, lo: Option<f64>, hi: Option<f64>) -> Rect {
    let mut v = [q.0, q.1, q.2, q.3];
    if let Some(lo) = lo {
        v[axis] = crate::numeric::max(v[axis], lo);
    }
    if let Some(hi) = hi {
        v[axis + 2] = crate::numeric::min(v[axis + 2], hi);
    }
    (v[0], v[1], v[2], v[3])
}

pub fn bisect(ok: impl Fn(f64) -> bool, lo: f64, hi: f64, want_max: bool) -> Option<f64> {
    if want_max {
        if ok(hi) {
            return Some(hi);
        }
        if !ok(lo) {
            return None;
        }
    } else {
        if ok(lo) {
            return Some(lo);
        }
        if !ok(hi) {
            return None;
        }
    }
    let (mut a, mut b) = (lo, hi);
    for _ in 0..60 {
        let m = (a + b) / 2.0;
        if ok(m) == want_max {
            a = m;
        } else {
            b = m;
        }
        if b - a < 0.005 {
            break;
        }
    }
    Some(if want_max { a } else { b })
}

pub fn linspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    let div = (num - 1) as f64;
    let delta = stop - start;
    let step = delta / div;
    let mut y: Vec<f64> = (0..num)
        .map(|i| if step == 0.0 { (i as f64 / div) * delta + start } else { i as f64 * step + start })
        .collect();
    if num > 1 {
        y[num - 1] = stop;
    }
    y
}

pub fn arange(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let len = ((stop - start) / step).ceil();
    if len.is_nan() || len <= 0.0 {
        return Vec::new();
    }
    let len = len as usize;
    let mut out = vec![start];
    if len > 1 {
        out.push(start + step);
        let delta = out[1] - out[0];
        for i in 2..len {
            out.push(start + i as f64 * delta);
        }
    }
    out
}

fn total_cmp_nan_last(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or_else(|| a.is_nan().cmp(&b.is_nan()))
}

pub fn solve(
    geo: &Geometry,
    rects: &IndexMap<i64, Rect>,
    passes: &[i64],
    core: Rect,
    ext: Rect,
    margin: f64,
    ov: f64,
) -> Option<Node> {
    for &r in passes {
        if geo.covered(ext, &[rects[&r]]) {
            return Some(Node::leaf(core, ext, r));
        }
    }
    if passes.len() < 2 {
        return None;
    }
    let mut best: Option<Node> = None;
    for axis in [0usize, 1] {
        let centre = |r: i64| (rect_get(rects[&r], axis) + rect_get(rects[&r], axis + 2)) / 2.0;
        let mut order = passes.to_vec();
        order.sort_by(|&a, &b| total_cmp_nan_last(centre(a), centre(b)).then(a.cmp(&b)));
        for k in 1..order.len() {
            let (l, r) = (&order[..k], &order[k..]);
            let lrects: Vec<Rect> = l.iter().map(|x| rects[x]).collect();
            let rrects: Vec<Rect> = r.iter().map(|x| rects[x]).collect();
            let (lo, hi) = (rect_get(core, axis), rect_get(core, axis + 2));
            let left_ok = |s: f64| geo.covered(with_bounds(ext, axis, None, Some(s + margin)), &lrects);
            let right_ok = |s: f64| geo.covered(with_bounds(ext, axis, Some(s - margin), None), &rrects);
            let (Some(s_max), Some(s_min)) = (bisect(left_ok, lo, hi, true), bisect(right_ok, lo, hi, false)) else {
                continue;
            };
            if s_min > s_max + 1e-9 {
                continue;
            }
            let n = 1_i64.max(((s_max - s_min) / GRID_STEP).round_ties_even() as i64) as usize;
            let cands = linspace(s_min, s_max, n + 1);
            let costs = geo.seam_cost(core, axis, &cands, ov);
            let mut idx: Vec<usize> = (0..cands.len()).collect();
            idx.sort_by(|&i, &j| total_cmp_nan_last(costs[i], costs[j]).then(total_cmp_nan_last(cands[i], cands[j])));
            let mut tried: Vec<f64> = Vec::new();
            for i in idx {
                let s = cands[i];
                if tried.iter().any(|t| (s - t).abs() < 5.0) {
                    continue;
                }
                if tried.len() >= TRIES {
                    break;
                }
                tried.push(s);
                let Some(low) = solve(
                    geo,
                    rects,
                    l,
                    with_bounds(core, axis, None, Some(s)),
                    with_bounds(ext, axis, None, Some(s + margin)),
                    margin,
                    ov,
                ) else {
                    continue;
                };
                let Some(high) = solve(
                    geo,
                    rects,
                    r,
                    with_bounds(core, axis, Some(s), None),
                    with_bounds(ext, axis, Some(s - margin), None),
                    margin,
                    ov,
                ) else {
                    continue;
                };
                let total = costs[i] + low.cost + high.cost;
                if best.as_ref().is_none_or(|b| total < b.cost - 1e-9) {
                    best = Some(Node {
                        core,
                        ext,
                        rotation: None,
                        axis,
                        s,
                        low: Some(Box::new(low)),
                        high: Some(Box::new(high)),
                        cost: total,
                    });
                }
                break;
            }
        }
    }
    best
}

pub fn cum_lengths(pts: &[Point]) -> Vec<f64> {
    let mut c = vec![0.0];
    for w in pts.windows(2) {
        let last = c[c.len() - 1];
        c.push(last + dist(w[0], w[1]));
    }
    c
}

pub fn point_at(pts: &[Point], cum: &[f64], t: f64) -> Point {
    if t <= 0.0 {
        return pts[0];
    }
    if t >= cum[cum.len() - 1] {
        return pts[pts.len() - 1];
    }
    let i = cum.partition_point(|&c| c <= t) as i64 - 1;
    let i = i.max(0).min(pts.len() as i64 - 2) as usize;
    let seg = cum[i + 1] - cum[i];
    let k = if seg == 0.0 { 0.0 } else { (t - cum[i]) / seg };
    let (a, b) = (pts[i], pts[i + 1]);
    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
}

pub fn crossings(pts: &[Point], cum: &[f64], axis: usize, s: f64) -> Vec<f64> {
    let mut out = Vec::new();
    for i in 0..pts.len().saturating_sub(1) {
        let (d0, d1) = (coord(pts[i], axis) - s, coord(pts[i + 1], axis) - s);
        if d0 == 0.0 {
            out.push(cum[i]);
        } else if d0 * d1 < 0.0 {
            out.push(cum[i] + (cum[i + 1] - cum[i]) * d0 / (d0 - d1));
        }
    }
    if coord(pts[pts.len() - 1], axis) == s {
        out.push(cum[cum.len() - 1]);
    }
    out
}

pub fn extract(pts: &[Point], cum: &[f64], a: f64, b: f64, closed: bool) -> Vec<Point> {
    let l = cum[cum.len() - 1];
    let (mut a, mut b) = (a, b);
    let (mut p, mut c) = (pts.to_vec(), cum.to_vec());
    if closed && l > 0.0 {
        while a < 0.0 {
            a += l;
            b += l;
        }
        let laps = (b / l).ceil() as i64 + 1;
        for k in 1..laps {
            p.extend_from_slice(&pts[1..]);
            c.extend(cum[1..].iter().map(|x| x + k as f64 * l));
        }
    } else {
        a = crate::numeric::max(0.0, a);
        b = crate::numeric::min(l, b);
    }
    let mut out = vec![point_at(&p, &c, a)];
    let i0 = c.partition_point(|&x| x <= a);
    let i1 = c.partition_point(|&x| x < b);
    if i0 < i1 {
        out.extend_from_slice(&p[i0..i1]);
    }
    let end = point_at(&p, &c, b);
    if end != out[out.len() - 1] {
        out.push(end);
    }
    out
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CutStats {
    pub length_in: f64,
    pub length_out: f64,
    pub cuts: usize,
    pub extension: f64,
}

fn distinct_count(pts: &[Point]) -> usize {
    let norm = |v: f64| if v == 0.0 { 0.0_f64.to_bits() } else { v.to_bits() };
    pts.iter().map(|q| (norm(q.0), norm(q.1))).collect::<std::collections::HashSet<_>>().len()
}

pub fn cut_stroke(pts: &[Point], root: &Node, ov: f64, stats: Option<&mut CutStats>) -> Vec<(i64, Vec<Point>)> {
    if pts.len() == 1 || (pts[0] == pts[pts.len() - 1] && distinct_count(pts) == 1) {
        return vec![(root.leaf_at(pts[0]).rotation.expect("leaf"), vec![pts[0]])];
    }
    let cum = cum_lengths(pts);
    let l = cum[cum.len() - 1];
    let closed = pts.len() > 2 && pts[0] == pts[pts.len() - 1];
    let mut parts: Vec<(i64, f64, f64)> = Vec::new();
    let mut cache: std::collections::HashMap<usize, Vec<f64>> = std::collections::HashMap::new();

    fn rec(
        n: &Node,
        a: f64,
        b: f64,
        pts: &[Point],
        cum: &[f64],
        parts: &mut Vec<(i64, f64, f64)>,
        cache: &mut std::collections::HashMap<usize, Vec<f64>>,
    ) {
        if let Some(rot) = n.rotation {
            if let Some(last) = parts.last_mut()
                && last.0 == rot
                && (last.2 - a).abs() < 1e-9
            {
                last.2 = b;
            } else {
                parts.push((rot, a, b));
            }
            return;
        }
        let key = n as *const Node as usize;
        let ts: Vec<f64> = cache
            .entry(key)
            .or_insert_with(|| crossings(pts, cum, n.axis, n.s))
            .iter()
            .copied()
            .filter(|&t| a + 1e-9 < t && t < b - 1e-9)
            .collect();
        let mut bounds = vec![a];
        bounds.extend(ts);
        bounds.push(b);
        let (lo, hi) = n.children();
        for w in bounds.windows(2) {
            let (x, y) = (w[0], w[1]);
            if y - x <= 1e-9 {
                continue;
            }
            let m = point_at(pts, cum, (x + y) / 2.0);
            rec(if coord(m, n.axis) <= n.s { lo } else { hi }, x, y, pts, cum, parts, cache);
        }
    }

    rec(root, 0.0, l, pts, &cum, &mut parts, &mut cache);
    if closed && parts.len() > 1 && parts[0].0 == parts[parts.len() - 1].0 {
        let last = parts.pop().expect("several parts");
        parts[0] = (last.0, last.1 - l, parts[0].2);
    }
    let mut out = Vec::new();
    let mut stats = stats;
    if let Some(st) = stats.as_deref_mut() {
        st.length_in += l;
    }
    let n_parts = parts.len();
    for &(rot, a, b) in &parts {
        let piece = if n_parts == 1 {
            pts.to_vec()
        } else {
            let cut_a = closed || a > 1e-9;
            let cut_b = closed || b < l - 1e-9;
            let mut a2 = if cut_a { a - ov } else { a };
            let mut b2 = if cut_b { b + ov } else { b };
            if !closed {
                a2 = crate::numeric::max(0.0, a2);
                b2 = crate::numeric::min(l, b2);
            } else if b2 - a2 > l {
                b2 = a2 + l;
            }
            let piece = extract(pts, &cum, a2, b2, closed);
            if let Some(st) = stats.as_deref_mut() {
                st.extension += (b2 - a2) - (b - a);
            }
            piece
        };
        if let Some(st) = stats.as_deref_mut() {
            st.length_out += polyline_length(&piece);
        }
        out.push((rot, piece));
    }
    if let Some(st) = stats {
        st.cuts += if n_parts > 1 { if closed { n_parts } else { n_parts - 1 } } else { 0 };
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub x: f64,
    pub y: f64,
    pub passes: (i64, i64),
}

pub fn control_marks(
    root: &Node,
    geo: &Geometry,
    rects: &IndexMap<i64, Rect>,
    size: f64,
    count: usize,
    clearance: f64,
) -> Vec<Mark> {
    let h = size / 2.0;
    let mut out = Vec::new();
    for n in root.seams() {
        let (ax, other) = (n.axis, 1 - n.axis);
        let (a0, a1) = (rect_get(n.core, other) + h + 1.0, rect_get(n.core, other + 2) - h - 1.0);
        if a1 < a0 {
            continue;
        }
        let mut good: Vec<(f64, Point, (i64, i64))> = Vec::new();
        for t in arange(a0, a1 + 1e-9, 1.0) {
            let mut p = [0.0, 0.0];
            p[ax] = n.s;
            p[other] = t;
            let (mut lo_p, mut hi_p) = (p, p);
            lo_p[ax] -= 1e-6;
            hi_p[ax] += 1e-6;
            let ra = n.leaf_at((lo_p[0], lo_p[1])).rotation.expect("leaf");
            let rb = n.leaf_at((hi_p[0], hi_p[1])).rotation.expect("leaf");
            if ra == rb {
                continue;
            }
            let bx = (p[0] - h, p[1] - h, p[0] + h, p[1] + h);
            if [rects[&ra], rects[&rb]].iter().all(|r| r.0 <= bx.0 && r.1 <= bx.1 && bx.2 <= r.2 && bx.3 <= r.3) {
                good.push((t, (p[0], p[1]), (ra, rb)));
            }
        }
        if good.is_empty() {
            continue;
        }
        let d = geo.distance_to(&good.iter().map(|g| g.1).collect::<Vec<_>>());
        let good: Vec<_> = good.into_iter().zip(d).filter(|(_, d)| *d > h + clearance).map(|(g, _)| g).collect();
        if good.is_empty() {
            continue;
        }
        let (lo_t, hi_t) = (good[0].0, good[good.len() - 1].0);
        let fracs: Vec<f64> = if count == 1 {
            vec![0.5]
        } else {
            (0..count).map(|i| 0.12 + 0.76 * i as f64 / (count - 1) as f64).collect()
        };
        let mut chosen: Vec<(f64, Point, (i64, i64))> = Vec::new();
        for f in fracs {
            let target = lo_t + (hi_t - lo_t) * f;
            let mut best = good[0];
            for &g in &good[1..] {
                let (kg, kb) = ((g.0 - target).abs(), (best.0 - target).abs());
                if kg < kb || (kg == kb && g.0 < best.0) {
                    best = g;
                }
            }
            if chosen.iter().all(|c| (best.0 - c.0).abs() >= 4.0 * size) {
                chosen.push(best);
            }
        }
        chosen.sort_by(|a, b| total_cmp_nan_last(a.0, b.0));
        out.extend(chosen.into_iter().map(|g| Mark { x: g.1.0, y: g.1.1, passes: g.2 }));
    }
    out
}

pub fn mark_strokes(m: &Mark, size: f64) -> Vec<Vec<Point>> {
    let h = size / 2.0;
    vec![vec![(m.x - h, m.y), (m.x + h, m.y)], vec![(m.x, m.y - h), (m.x, m.y + h)]]
}
