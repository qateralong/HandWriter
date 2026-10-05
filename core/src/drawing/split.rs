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
