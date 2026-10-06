use std::collections::HashMap;

use crate::geometry::Point;
use crate::numeric::{self, dist, hypot};

pub fn dedupe(pts: &[Point], eps: f64) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::new();
    for &p in pts {
        match out.last() {
            Some(&l) if !((p.0 - l.0).abs() > eps || (p.1 - l.1).abs() > eps) => {
                if out.len() > 1 {
                    let n = out.len() - 1;
                    out[n] = p;
                }
            }
            _ => out.push(p),
        }
    }
    out
}

pub fn path_length(pts: &[Point]) -> f64 {
    numeric::sum(pts.windows(2).map(|w| dist(w[0], w[1])))
}

pub fn py_mod(x: f64, y: f64) -> f64 {
    let m = x % y;
    if m != 0.0 { if (y < 0.0) != (m < 0.0) { m + y } else { m } } else { 0.0_f64.copysign(y) }
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

pub fn dash_polyline(pts: &[Point], pattern: &[f64], offset: f64) -> Vec<Vec<Point>> {
    let mut pat: Vec<f64> = pattern.iter().map(|&v| numeric::max(0.0, v)).collect();
    if pat.len() % 2 == 1 {
        pat.extend(pat.clone());
    }
    let total = numeric::sum(pat.iter().copied());
    let pts = dedupe(pts, 1e-9);
    if total <= 1e-9 || pts.len() < 2 || pat.is_empty() {
        return vec![pts];
    }
    let n = pat.len();
    let mut i = 0;
    let mut off = py_mod(offset, total);
    for _ in 0..4 * n {
        if off < pat[i] || pat[i] == 0.0 && off <= 0.0 {
            break;
        }
        off -= pat[i];
        i = (i + 1) % n;
    }
    let mut left = pat[i] - off;
    let mut on = i % 2 == 0;
    let mut out = Vec::new();
    let mut cur: Option<Vec<Point>> = if on { Some(vec![pts[0]]) } else { None };
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let seg = dist(a, b);
        let mut t = 0.0;
        while seg - t > left {
            t += left;
            let p = lerp(a, b, t / seg);
            if on {
                let mut c = cur.take().expect("current dash");
                c.push(p);
                out.push(dedupe(&c, 1e-9));
            } else {
                cur = Some(vec![p]);
            }
            on = !on;
            i = (i + 1) % n;
            left = pat[i];
        }
        left -= seg - t;
        if on {
            cur.as_mut().expect("current dash").push(b);
        }
    }
    if on && let Some(c) = cur.filter(|c| !c.is_empty()) {
        out.push(dedupe(&c, 1e-9));
    }
    out
}

pub fn offset_polyline(pts: &[Point], d: f64, closed: bool) -> Vec<Point> {
    let pts = dedupe(pts, 1e-9);
    let (body, closed) = if closed && pts.len() > 2 && pts[0] == pts[pts.len() - 1] {
        (&pts[..pts.len() - 1], true)
    } else {
        (&pts[..], false)
    };
    let n = body.len();
    if n < 2 || d == 0.0 {
        return pts.clone();
    }
    let normal = |a: Point, b: Point| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l = hypot(dx, dy);
        let l = if l == 0.0 { 1.0 } else { l };
        (-dy / l, dx / l)
    };
    let mut segs: Vec<Point> = (0..n - 1).map(|i| normal(body[i], body[i + 1])).collect();
    if closed {
        segs.push(normal(body[n - 1], body[0]));
    }
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..n {
        let (n1, n2) = if closed {
            (segs[(i + segs.len() - 1) % segs.len()], segs[i])
        } else if i == 0 {
            (segs[0], segs[0])
        } else if i == n - 1 {
            (segs[segs.len() - 1], segs[segs.len() - 1])
        } else {
            (segs[i - 1], segs[i])
        };
        let (mut bx, mut by) = (n1.0 + n2.0, n1.1 + n2.1);
        let l = hypot(bx, by);
        let k;
        if l < 1e-9 {
            (bx, by, k) = (n2.0, n2.1, d);
        } else {
            bx /= l;
            by /= l;
            let cos_half = bx * n2.0 + by * n2.1;
            k = d / numeric::max(cos_half, 0.5);
        }
        out.push((body[i].0 + bx * k, body[i].1 + by * k));
    }
    if closed {
        out.push(out[0]);
    }
    out
}

pub fn pass_offsets(passes: i64, step: f64) -> Vec<f64> {
    (0..passes).map(|k| (k as f64 - (passes - 1) as f64 / 2.0) * step).collect()
}

pub fn expand_passes(pts: &[Point], closed: bool, passes: i64, step: f64) -> Vec<Vec<Point>> {
    if passes <= 1 || pts.len() < 2 {
        return vec![pts.to_vec()];
    }
    pass_offsets(passes, step)
        .into_iter()
        .enumerate()
        .map(|(k, d)| {
            let mut q = offset_polyline(pts, d, closed);
            if !closed && k % 2 == 1 {
                q.reverse();
            }
            q
        })
        .collect()
}

fn direction(a: Point, b: Point) -> Point {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l = hypot(dx, dy);
    if l != 0.0 { (dx / l, dy / l) } else { (0.0, 0.0) }
}

pub fn join_paths(paths: &[Vec<Point>], tol: f64) -> Vec<Vec<Point>> {
    if tol <= 0.0 || paths.len() < 2 {
        return paths.to_vec();
    }
    let cell = numeric::max(tol, 1e-6);
    let key = |p: Point| ((p.0 / cell).floor() as i64, (p.1 / cell).floor() as i64);
    let is_open: Vec<bool> = paths.iter().map(|p| p.len() >= 2 && dist(p[0], p[p.len() - 1]) > tol).collect();
    let mut grid: HashMap<(i64, i64), Vec<(usize, u8)>> = HashMap::new();
    for (i, p) in paths.iter().enumerate() {
        if is_open[i] {
            grid.entry(key(p[0])).or_default().push((i, 0));
            grid.entry(key(p[p.len() - 1])).or_default().push((i, 1));
        }
    }
    let mut used: Vec<bool> = is_open.iter().map(|o| !o).collect();

    let best_next = |end: Point, heading: Point, used: &[bool]| -> Option<(f64, usize, Vec<Point>)> {
        let (kx, ky) = key(end);
        let mut best: Option<(f64, usize, Vec<Point>)> = None;
        for gx in [kx - 1, kx, kx + 1] {
            for gy in [ky - 1, ky, ky + 1] {
                let Some(list) = grid.get(&(gx, gy)) else { continue };
                for &(j, which) in list {
                    if used[j] {
                        continue;
                    }
                    let q = &paths[j];
                    let p0 = if which == 0 { q[0] } else { q[q.len() - 1] };
                    if dist(p0, end) > tol {
                        continue;
                    }
                    let seq: Vec<Point> = if which == 0 { q.clone() } else { q.iter().rev().copied().collect() };
                    let d = direction(seq[0], seq[1]);
                    let turn = -(heading.0 * d.0 + heading.1 * d.1);
                    let better = match &best {
                        None => true,
                        Some((bt, bj, _)) => {
                            if turn != *bt {
                                turn < *bt
                            } else {
                                j < *bj
                            }
                        }
                    };
                    if better {
                        best = Some((turn, j, seq));
                    }
                }
            }
        }
        best
    };

    let mut out = Vec::new();
    for (i, p) in paths.iter().enumerate() {
        if used[i] {
            if !is_open[i] {
                out.push(p.clone());
            }
            continue;
        }
        used[i] = true;
        let mut chain = p.clone();
        for _ in 0..2 {
            loop {
                let n = chain.len();
                let Some((_, j, seq)) = best_next(chain[n - 1], direction(chain[n - 2], chain[n - 1]), &used) else {
                    break;
                };
                used[j] = true;
                chain.extend_from_slice(&seq[1..]);
            }
            chain.reverse();
        }
        let n = chain.len();
        if n > 2 && dist(chain[0], chain[n - 1]) <= tol {
            chain[n - 1] = chain[0];
        }
        out.push(chain);
    }
    out
}

fn kd_distance(a: Point, b: Point) -> f64 {
    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
    (0.0 + dx * dx + dy * dy).sqrt()
}

pub fn order_paths(paths: &[Vec<Point>], long_path: f64, start: Point) -> Vec<(usize, Vec<Point>)> {
    if paths.is_empty() {
        return Vec::new();
    }
    let lengths: Vec<f64> = paths.iter().map(|p| path_length(p)).collect();
    let groups = [
        (0..paths.len()).filter(|&i| lengths[i] >= long_path).collect::<Vec<_>>(),
        (0..paths.len()).filter(|&i| lengths[i] < long_path).collect::<Vec<_>>(),
    ];
    let mut out = Vec::new();
    let mut pos = start;
    for g in groups {
        if g.is_empty() {
            continue;
        }
        let mut cand: Vec<(Point, usize, isize)> = Vec::new();
        let mut range: HashMap<usize, (usize, usize)> = HashMap::new();
        for &i in &g {
            let p = &paths[i];
            let from = cand.len();
            let closed = p.len() > 2 && p[0] == p[p.len() - 1];
            if closed {
                for (k, &q) in p.iter().enumerate().take(p.len() - 1) {
                    cand.push((q, i, k as isize));
                }
            } else {
                cand.push((p[0], i, 0));
                if p.len() > 1 {
                    cand.push((p[p.len() - 1], i, -1));
                }
            }
            range.insert(i, (from, cand.len()));
        }
        let mut alive = vec![true; cand.len()];
        for _ in 0..g.len() {
            let mut best: Option<(f64, usize)> = None;
            for (c, &(xy, _, _)) in cand.iter().enumerate() {
                if !alive[c] {
                    continue;
                }
                let d = kd_distance(pos, xy);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, c));
                }
            }
            let (_, c) = best.expect("alive candidate");
            let (_, i, v) = cand[c];
            let (a, b) = range[&i];
            alive[a..b].iter_mut().for_each(|x| *x = false);
            let p = &paths[i];
            let seq: Vec<Point> = match v {
                -1 => p.iter().rev().copied().collect(),
                0 => p.clone(),
                v => {
                    let body = &p[..p.len() - 1];
                    let v = v as usize;
                    let mut s = body[v..].to_vec();
                    s.extend_from_slice(&body[..v]);
                    s.push(body[v]);
                    s
                }
            };
            pos = seq[seq.len() - 1];
            out.push((i, seq));
        }
    }
    out
}

fn clip(a: Point, b: Point, bx: (f64, f64, f64, f64)) -> Option<(f64, f64)> {
    let (x0, y0, x1, y1) = bx;
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0, 1.0);
    for (p, q) in [(-dx, a.0 - x0), (dx, x1 - a.0), (-dy, a.1 - y0), (dy, y1 - a.1)] {
        if p.abs() < 1e-15 {
            if q < -1e-9 {
                return None;
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            t0 = numeric::max(t0, r);
        } else {
            t1 = numeric::min(t1, r);
        }
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, t1))
}

pub fn outside_parts(pts: &[Point], bx: (f64, f64, f64, f64), eps: f64) -> Vec<Vec<Point>> {
    let (x0, y0, x1, y1) = bx;
    let inside = |p: Point| x0 - eps <= p.0 && p.0 <= x1 + eps && y0 - eps <= p.1 && p.1 <= y1 + eps;
    if pts.len() == 1 {
        return if inside(pts[0]) { Vec::new() } else { vec![pts.to_vec()] };
    }
    let mut out = Vec::new();
    let mut cur: Option<Vec<Point>> = None;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let Some((t0, t1)) = clip(a, b, bx) else {
            cur.get_or_insert_with(|| vec![a]).push(b);
            continue;
        };
        if t0 > 1e-9 {
            cur.get_or_insert_with(|| vec![a]).push(lerp(a, b, t0));
        }
        if let Some(c) = cur.take() {
            out.push(c);
        }
        if t1 < 1.0 - 1e-9 {
            cur = Some(vec![lerp(a, b, t1), b]);
        }
    }
    if let Some(c) = cur {
        out.push(c);
    }
    out
}
