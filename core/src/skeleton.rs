use std::collections::HashSet;

use indexmap::IndexMap;

use crate::drawing::model::Mask;
use crate::geometry::{Point, rdp};
use crate::numeric;
use crate::pyset::{PySet, hash_int, hash_pair};

const N8: [(i64, i64); 8] = [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkeletonParams {
    pub px_per_em: f64,
    pub prune: f64,
    pub extend: f64,
    pub smooth: f64,
    pub simplify: f64,
    pub junction_merge: f64,
}

impl Default for SkeletonParams {
    fn default() -> Self {
        Self { px_per_em: 1500.0, prune: 0.08, extend: 1.0, smooth: 0.03, simplify: 0.004, junction_merge: 2.5 }
    }
}

impl From<&crate::settings::OutlineOptions> for SkeletonParams {
    fn from(o: &crate::settings::OutlineOptions) -> Self {
        Self {
            px_per_em: o.px_per_em,
            prune: o.prune,
            extend: o.extend,
            smooth: o.smooth,
            simplify: o.simplify,
            junction_merge: o.junction_merge,
        }
    }
}

impl SkeletonParams {
    pub fn key(&self) -> String {
        use crate::numeric::format_g as g;
        format!(
            "p{}_r{}_e{}_s{}_t{}_j{}",
            g(self.px_per_em, 6),
            g(self.prune, 6),
            g(self.extend, 6),
            g(self.smooth, 6),
            g(self.simplify, 6),
            g(self.junction_merge, 6)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SkeletonResult {
    pub strokes: Vec<Vec<Point>>,
    pub raw: Vec<Vec<Point>>,
    pub closed: Vec<bool>,
}

pub fn np_sum(values: &[f64]) -> f64 {
    fn pairwise(a: &[f64]) -> f64 {
        let n = a.len();
        if n < 8 {
            let mut res = -0.0;
            for &x in a {
                res += x;
            }
            res
        } else if n <= 128 {
            let mut r = [a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]];
            let mut i = 8;
            while i < n - (n % 8) {
                for j in 0..8 {
                    r[j] += a[i + j];
                }
                i += 8;
            }
            let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
            while i < n {
                res += a[i];
                i += 1;
            }
            res
        } else {
            let mut n2 = n / 2;
            n2 -= n2 % 8;
            pairwise(&a[..n2]) + pairwise(&a[n2..])
        }
    }
    0.0 + pairwise(values)
}

fn allclose(a: Point, b: Point) -> bool {
    let close = |x: f64, y: f64| (x - y).abs() <= 1e-8 + 1e-5 * y.abs();
    close(a.0, b.0) && close(a.1, b.1)
}

pub fn rasterize(contours: &[Vec<Point>], width: usize, height: usize) -> Mask {
    struct Edge {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    }
    let mut edges = Vec::new();
    for c in contours {
        if c.len() < 2 {
            continue;
        }
        let mut pts = c.clone();
        if !allclose(pts[0], pts[pts.len() - 1]) {
            pts.push(pts[0]);
        }
        for w in pts.windows(2) {
            edges.push(Edge { x0: w[0].0, y0: w[0].1, x1: w[1].0, y1: w[1].1 });
        }
    }
    let mut img = Mask::new(height, width);
    let edges: Vec<Edge> = edges.into_iter().filter(|e| e.y0 != e.y1).collect();
    if edges.is_empty() {
        return img;
    }
    let clip = |v: f64, hi: usize| -> i64 { (v.ceil() as i64).clamp(0, hi as i64) };
    let mut items: Vec<(i64, f64, i64)> = Vec::new();
    for e in &edges {
        let wind = if e.y1 > e.y0 { 1 } else { -1 };
        let (ylo, yhi) = (numeric_min_np(e.y0, e.y1), numeric_max_np(e.y0, e.y1));
        let r0 = clip(ylo - 0.5, height);
        let r1 = clip(yhi - 0.5, height);
        for row in r0..r1.max(r0) {
            let yc = row as f64 + 0.5;
            let t = (yc - e.y0) / (e.y1 - e.y0);
            let xs = e.x0 + t * (e.x1 - e.x0);
            items.push((row, xs, wind));
        }
    }
    if items.is_empty() {
        return img;
    }
    items.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.partial_cmp(&b.1).unwrap_or_else(|| a.1.is_nan().cmp(&b.1.is_nan()))));
    let mut diff = vec![0i32; height * (width + 1)];
    let mut winding = 0i64;
    for k in 0..items.len() {
        if k == 0 || items[k].0 != items[k - 1].0 {
            winding = 0;
        }
        winding += items[k].2;
        let same_next = k + 1 < items.len() && items[k + 1].0 == items[k].0;
        if winding != 0 && same_next {
            let row = items[k].0 as usize;
            let c0 = clip(items[k].1 - 0.5, width) as usize;
            let c1 = clip(items[k + 1].1 - 0.5, width) as usize;
            diff[row * (width + 1) + c0] += 1;
            diff[row * (width + 1) + c1] -= 1;
        }
    }
    for r in 0..height {
        let mut acc = 0i32;
        for c in 0..width {
            acc += diff[r * (width + 1) + c];
            img.set(r, c, acc > 0);
        }
    }
    img
}

fn numeric_min_np(a: f64, b: f64) -> f64 {
    if a <= b || a.is_nan() { a } else { b }
}

fn numeric_max_np(a: f64, b: f64) -> f64 {
    if a >= b || a.is_nan() { a } else { b }
}

const ZHANG_LUT: [u8; 256] = [
    0, 0, 0, 1, 0, 0, 1, 3, 0, 0, 3, 1, 1, 0, 1, 3, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 3, 0, 3, 3, 0, 0, 0, 0, 0, 0,
    0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 3, 0, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 3, 0,
    0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 3, 0, 2, 0, 0, 0, 3, 1, 0, 0, 1, 3, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 1, 3, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 2, 3, 1, 3, 0, 0, 1, 3, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 0, 1,
    0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 3, 3, 0, 1, 0, 0, 0, 0, 2, 2, 0, 0, 2, 0, 0, 0,
];

pub fn skeletonize(mask: &Mask) -> Mask {
    let (nrows, ncols) = (mask.rows + 2, mask.cols + 2);
    let mut sk = vec![0u8; nrows * ncols];
    for r in 0..mask.rows {
        for c in 0..mask.cols {
            sk[(r + 1) * ncols + c + 1] = u8::from(mask.get(r, c));
        }
    }
    let offs: [isize; 8] = [
        -(ncols as isize) - 1,
        -(ncols as isize),
        -(ncols as isize) + 1,
        1,
        ncols as isize + 1,
        ncols as isize,
        ncols as isize - 1,
        -1,
    ];
    let around = [offs[0], offs[1], offs[2], offs[7], offs[3], offs[6], offs[5], offs[4]];
    let mut stamp = vec![0u32; nrows * ncols];
    let mut epoch = 1u32;
    let mut cand: Vec<usize> = Vec::new();
    for i in 0..nrows * ncols {
        if sk[i] != 0 && around.iter().any(|&o| sk[(i as isize + o) as usize] == 0) {
            cand.push(i);
        }
    }
    let mut removed_any = true;
    while removed_any {
        removed_any = false;
        for pass in 0..2 {
            let first = pass == 0;
            let mut removed: Vec<usize> = Vec::new();
            for &i in &cand {
                if sk[i] == 0 {
                    continue;
                }
                let mut idx = 0usize;
                for (bit, &o) in offs.iter().enumerate() {
                    idx |= (sk[(i as isize + o) as usize] as usize) << bit;
                }
                let n = ZHANG_LUT[idx];
                if n == 3 || (n == 1 && first) || (n == 2 && !first) {
                    removed.push(i);
                }
            }
            if removed.is_empty() {
                continue;
            }
            removed_any = true;
            for &i in &removed {
                sk[i] = 0;
            }
            epoch += 1;
            let mut next: Vec<usize> = Vec::with_capacity(cand.len());
            for &i in cand.iter().chain(
                removed
                    .iter()
                    .flat_map(|&r| around.iter().map(move |&o| (r as isize + o) as usize))
                    .collect::<Vec<_>>()
                    .iter(),
            ) {
                if sk[i] != 0 && stamp[i] != epoch {
                    stamp[i] = epoch;
                    next.push(i);
                }
            }
            cand = next;
        }
    }
    let mut out = Mask::new(mask.rows, mask.cols);
    for r in 0..mask.rows {
        for c in 0..mask.cols {
            out.set(r, c, sk[(r + 1) * ncols + c + 1] != 0);
        }
    }
    out
}

const EDT_INF: i64 = i64::MAX / 4;

struct EdtScratch {
    v: Vec<usize>,
    z: Vec<f64>,
}

fn edt_1d(f: &[i64], out: &mut [i64], sc: &mut EdtScratch) {
    let n = f.len();
    let Some(first) = (0..n).find(|&q| f[q] < EDT_INF) else {
        out.iter_mut().for_each(|o| *o = EDT_INF);
        return;
    };
    let (v, z) = (&mut sc.v, &mut sc.z);
    let mut k = 0usize;
    v[0] = first;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in first + 1..n {
        if f[q] >= EDT_INF {
            continue;
        }
        let fq = f[q] + (q * q) as i64;
        let mut s;
        loop {
            let p = v[k];
            s = (fq - (f[p] + (p * p) as i64)) as f64 / (2 * (q as i64 - p as i64)) as f64;
            if s <= z[k] {
                k -= 1;
            } else {
                break;
            }
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    let mut j = 0usize;
    for (q, o) in out.iter_mut().enumerate() {
        while z[j + 1] < q as f64 {
            j += 1;
        }
        let d = q as i64 - v[j] as i64;
        *o = d * d + f[v[j]];
    }
}

pub fn distance_transform(mask: &Mask) -> Vec<f64> {
    let (rows, cols) = (mask.rows, mask.cols);
    if mask.data.iter().all(|&m| m) {
        return (0..rows * cols)
            .map(|i| {
                let (dr, dc) = ((i / cols + 1) as f64, (i % cols) as f64);
                (dr * dr + dc * dc).sqrt()
            })
            .collect();
    }
    let mut g = vec![EDT_INF; rows * cols];
    for (i, &m) in mask.data.iter().enumerate() {
        if !m {
            g[i] = 0;
        }
    }
    let n = rows.max(cols);
    let mut sc = EdtScratch { v: vec![0; n], z: vec![0.0; n + 1] };
    let mut col_in = vec![0i64; rows];
    let mut col_out = vec![0i64; rows];
    for c in 0..cols {
        for r in 0..rows {
            col_in[r] = g[r * cols + c];
        }
        edt_1d(&col_in, &mut col_out, &mut sc);
        for r in 0..rows {
            g[r * cols + c] = col_out[r];
        }
    }
    let mut row_out = vec![0i64; cols];
    let mut out = vec![0.0; rows * cols];
    for r in 0..rows {
        edt_1d(&g[r * cols..(r + 1) * cols], &mut row_out, &mut sc);
        for c in 0..cols {
            out[r * cols + c] = (row_out[c] as f64).sqrt();
        }
    }
    out
}

pub struct Dt {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<f64>,
}

impl Dt {
    fn at(&self, r: usize, c: usize) -> f64 {
        self.data[r * self.cols + c]
    }
}

#[derive(Debug, Clone)]
struct Edge {
    u: Option<i64>,
    v: Option<i64>,
    pts: Vec<Point>,
    closed: bool,
}

#[derive(Default)]
struct Graph {
    nodes: IndexMap<i64, Point>,
    edges: IndexMap<i64, Edge>,
    next_eid: i64,
}

impl Graph {
    fn add_edge(&mut self, u: Option<i64>, v: Option<i64>, pts: Vec<Point>, closed: bool) {
        self.edges.insert(self.next_eid, Edge { u, v, pts, closed });
        self.next_eid += 1;
    }

    fn degree(&self) -> IndexMap<i64, i64> {
        let mut deg: IndexMap<i64, i64> = self.nodes.keys().map(|&n| (n, 0)).collect();
        for e in self.edges.values() {
            if e.closed {
                continue;
            }
            *deg.get_mut(&e.u.expect("open edge")).expect("node") += 1;
            *deg.get_mut(&e.v.expect("open edge")).expect("node") += 1;
        }
        deg
    }

    fn incident(&self, n: i64) -> Vec<i64> {
        self.edges.iter().filter(|(_, e)| !e.closed && (e.u == Some(n) || e.v == Some(n))).map(|(&id, _)| id).collect()
    }
}

fn np_length(pts: &[Point]) -> f64 {
    if pts.len() <= 1 {
        return 0.0;
    }
    let segs: Vec<f64> = pts.windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).collect();
    np_sum(&segs)
}

fn label8(mask: &[bool], rows: usize, cols: usize) -> (Vec<usize>, usize) {
    let mut lab = vec![0usize; rows * cols];
    let mut n = 0;
    let mut stack = Vec::new();
    for start in 0..rows * cols {
        if !mask[start] || lab[start] != 0 {
            continue;
        }
        n += 1;
        lab[start] = n;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (r, c) = ((i / cols) as i64, (i % cols) as i64);
            for (dr, dc) in N8 {
                let (rr, cc) = (r + dr, c + dc);
                if rr < 0 || cc < 0 || rr >= rows as i64 || cc >= cols as i64 {
                    continue;
                }
                let j = rr as usize * cols + cc as usize;
                if mask[j] && lab[j] == 0 {
                    lab[j] = n;
                    stack.push(j);
                }
            }
        }
    }
    (lab, n)
}

fn build_graph(skel: &Mask) -> Graph {
    let (rows, cols) = (skel.rows, skel.cols);
    let mut g = Graph::default();
    let on =
        |r: i64, c: i64| r >= 0 && c >= 0 && r < rows as i64 && c < cols as i64 && skel.get(r as usize, c as usize);
    let mut node_mask = vec![false; rows * cols];
    for r in 0..rows {
        for c in 0..cols {
            if !skel.get(r, c) {
                continue;
            }
            let nb = N8.iter().filter(|(dr, dc)| on(r as i64 + dr, c as i64 + dc)).count();
            node_mask[r * cols + c] = nb != 2;
        }
    }
    let (lab, n) = label8(&node_mask, rows, cols);
    let mut node_px: Vec<(i64, i64, usize)> = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            if node_mask[r * cols + c] {
                node_px.push((r as i64, c as i64, lab[r * cols + c]));
            }
        }
    }
    if n > 0 {
        let mut cnt = vec![0i64; n + 1];
        let mut cr = vec![0f64; n + 1];
        let mut cc = vec![0f64; n + 1];
        for &(r, c, i) in &node_px {
            cnt[i] += 1;
            cr[i] += r as f64;
            cc[i] += c as f64;
        }
        for i in 1..=n {
            g.nodes.insert(i as i64, (cr[i] / cnt[i] as f64, cc[i] / cnt[i] as f64));
        }
    }
    let node_of: IndexMap<(i64, i64), i64> = node_px.iter().map(|&(r, c, i)| ((r, c), i as i64)).collect();

    let mut pix_first = PySet::new();
    for r in 0..rows {
        for c in 0..cols {
            if skel.get(r, c) {
                pix_first.add((r as i64, c as i64), hash_pair(r as i64, c as i64));
            }
        }
    }
    let mut pix = PySet::new();
    for p in pix_first.iter() {
        pix.add(p, hash_pair(p.0, p.1));
    }
    let neigh = |p: (i64, i64)| -> Vec<(i64, i64)> {
        N8.iter().map(|(dr, dc)| (p.0 + dr, p.1 + dc)).filter(|&(r, c)| on(r, c)).collect()
    };
    let mut visited: HashSet<(i64, i64)> = HashSet::new();
    for (&p, &nid) in &node_of {
        for q in neigh(p) {
            if node_of.contains_key(&q) || visited.contains(&q) {
                continue;
            }
            visited.insert(q);
            let mut path: Vec<Point> = vec![g.nodes[&nid], (q.0 as f64, q.1 as f64)];
            let (mut prev, mut cur) = (p, q);
            let mut end = None;
            loop {
                let nxt: Vec<(i64, i64)> = neigh(cur)
                    .into_iter()
                    .filter(|r| *r != prev && (node_of.contains_key(r) || !visited.contains(r)))
                    .collect();
                let Some(&r) = nxt.first() else { break };
                if let Some(&e) = node_of.get(&r) {
                    end = Some(e);
                    break;
                }
                visited.insert(r);
                path.push((r.0 as f64, r.1 as f64));
                (prev, cur) = (cur, r);
            }
            let end = match end {
                Some(e) => e,
                None => {
                    let e = g.nodes.keys().copied().max().expect("nodes exist") + 1;
                    let last = path[path.len() - 1];
                    g.nodes.insert(e, last);
                    path.pop();
                    e
                }
            };
            path.push(g.nodes[&end]);
            g.add_edge(Some(nid), Some(end), path, false);
        }
    }
    for p in pix.iter() {
        if visited.contains(&p) || node_of.contains_key(&p) {
            continue;
        }
        let mut lp = vec![p];
        visited.insert(p);
        let (mut prev, mut cur): (Option<(i64, i64)>, (i64, i64)) = (None, p);
        loop {
            let nxt: Vec<(i64, i64)> =
                neigh(cur).into_iter().filter(|r| Some(*r) != prev && !visited.contains(r)).collect();
            let Some(&r) = nxt.first() else { break };
            (prev, cur) = (Some(cur), r);
            visited.insert(cur);
            lp.push(cur);
        }
        lp.push(lp[0]);
        g.add_edge(None, None, lp.into_iter().map(|(a, b)| (a as f64, b as f64)).collect(), true);
    }
    g
}

fn merge_degree2(g: &mut Graph) {
    let mut changed = true;
    while changed {
        changed = false;
        let deg = g.degree();
        for (&n, &d) in &deg {
            if d != 2 {
                continue;
            }
            let inc = g.incident(n);
            if inc.len() == 1 {
                let e = g.edges.get_mut(&inc[0]).expect("edge");
                e.closed = true;
                e.u = None;
                e.v = None;
                g.nodes.shift_remove(&n);
                changed = true;
                break;
            }
            let (a, b) = (g.edges[&inc[0]].clone(), g.edges[&inc[1]].clone());
            let (pa, ua) =
                if a.v == Some(n) { (a.pts.clone(), a.u) } else { (a.pts.iter().rev().copied().collect(), a.v) };
            let (pb, vb) = if b.u == Some(n) {
                (b.pts.clone(), b.v)
            } else {
                (b.pts.iter().rev().copied().collect::<Vec<_>>(), b.u)
            };
            for i in &inc {
                g.edges.shift_remove(i);
            }
            g.nodes.shift_remove(&n);
            let mut pts = pa;
            pts.extend_from_slice(&pb[1..]);
            g.add_edge(ua, vb, pts, false);
            changed = true;
            break;
        }
    }
}

fn py_round_index(v: f64) -> i64 {
    v.round_ties_even() as i64
}

fn radius_at(dt: Option<&Dt>, p: Point) -> f64 {
    let Some(dt) = dt else { return 0.0 };
    let r = py_round_index(p.0).max(0).min(dt.rows as i64 - 1) as usize;
    let c = py_round_index(p.1).max(0).min(dt.cols as i64 - 1) as usize;
    dt.at(r, c)
}

fn prune(g: &mut Graph, min_len: f64, dt: Option<&Dt>) {
    merge_degree2(g);
    for _ in 0..20 {
        let deg = g.degree();
        let mut drop: IndexMap<i64, Vec<i64>> = IndexMap::new();
        for (&eid, e) in &g.edges {
            if e.closed {
                continue;
            }
            let (u, v) = (e.u.expect("open"), e.v.expect("open"));
            let l = np_length(&e.pts);
            if u == v && l < min_len + 2.0 * radius_at(dt, g.nodes[&u]) {
                drop.entry(u).or_default().push(eid);
            } else if (deg[&u] == 1) != (deg[&v] == 1) {
                let j = if deg[&u] == 1 { v } else { u };
                if l - radius_at(dt, g.nodes[&j]) < min_len {
                    drop.entry(j).or_default().push(eid);
                }
            }
        }
        if drop.is_empty() {
            break;
        }
        for (j, mut eids) in drop {
            if eids.len() as i64 >= deg[&j] {
                eids.sort_by(|a, b| {
                    np_length(&g.edges[a].pts)
                        .partial_cmp(&np_length(&g.edges[b].pts))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                eids.pop();
            }
            for eid in eids {
                let e = g.edges.shift_remove(&eid).expect("edge");
                for n in [e.u, e.v].into_iter().flatten() {
                    if n != j && g.nodes.contains_key(&n) && g.incident(n).is_empty() {
                        g.nodes.shift_remove(&n);
                    }
                }
            }
        }
        merge_degree2(g);
    }
    let small: Vec<i64> =
        g.edges.iter().filter(|(_, e)| e.closed && np_length(&e.pts) < min_len).map(|(&i, _)| i).collect();
    for eid in small {
        g.edges.shift_remove(&eid);
    }
}

fn contract_junctions(g: &mut Graph, dt: &Dt, factor: f64) {
    if factor <= 0.0 {
        return;
    }
    let mut changed = true;
    while changed {
        changed = false;
        let deg = g.degree();
        let ids: Vec<i64> = g.edges.keys().copied().collect();
        for eid in ids {
            let e = g.edges[&eid].clone();
            if e.closed || e.u == e.v {
                continue;
            }
            let (u, v) = (e.u.expect("open"), e.v.expect("open"));
            if deg[&u] < 3 || deg[&v] < 3 {
                continue;
            }
            let lim = factor * numeric::max(radius_at(Some(dt), g.nodes[&u]), radius_at(Some(dt), g.nodes[&v]));
            if np_length(&e.pts) > lim {
                continue;
            }
            let c = e.pts[e.pts.len() / 2];
            g.edges.shift_remove(&eid);
            g.nodes.shift_remove(&v);
            *g.nodes.get_mut(&u).expect("node") = c;
            for e2 in g.edges.values_mut() {
                if e2.closed {
                    continue;
                }
                if e2.u == Some(v) {
                    e2.u = Some(u);
                }
                if e2.v == Some(v) {
                    e2.v = Some(u);
                }
                if e2.u == Some(u) {
                    e2.pts[0] = c;
                }
                if e2.v == Some(u) {
                    let n = e2.pts.len();
                    e2.pts[n - 1] = c;
                }
            }
            changed = true;
            break;
        }
    }
    merge_degree2(g);
}

fn dir(pts: &[Point], from_start: bool, look: f64) -> Point {
    let a: Vec<Point> = if from_start { pts.to_vec() } else { pts.iter().rev().copied().collect() };
    let mut acc = 0.0;
    let mut j = 1;
    for k in 1..a.len() {
        j = k;
        acc += numeric::hypot(a[k].0 - a[k - 1].0, a[k].1 - a[k - 1].1);
        if acc >= look {
            break;
        }
    }
    let mut v = (a[j].0 - a[0].0, a[j].1 - a[0].1);
    if !from_start {
        v = (-v.0, -v.1);
    }
    let n = numeric::hypot(v.0, v.1);
    if n != 0.0 { (v.0 / n, v.1 / n) } else { (0.0, 0.0) }
}

type Path = (Vec<Point>, bool, bool, bool);

fn traverse(g: &Graph, look: f64) -> Vec<Path> {
    let deg = g.degree();
    let mut unused = PySet::new();
    for (&eid, e) in &g.edges {
        if !e.closed {
            unused.add((eid, 0), hash_int(eid));
        }
    }
    let mut out = Vec::new();
    while !unused.is_empty() {
        let mut cnt: IndexMap<i64, i64> = g.nodes.keys().map(|&n| (n, 0)).collect();
        for (eid, _) in unused.iter() {
            let e = &g.edges[&eid];
            *cnt.get_mut(&e.u.expect("open")).expect("node") += 1;
            *cnt.get_mut(&e.v.expect("open")).expect("node") += 1;
        }
        let mut cands: Vec<i64> = cnt.iter().filter(|(_, c)| *c % 2 == 1).map(|(&n, _)| n).collect();
        if cands.is_empty() {
            cands = cnt.iter().filter(|(_, c)| **c > 0).map(|(&n, _)| n).collect();
        }
        let score = |n: i64| {
            let p = g.nodes[&n];
            p.1 - p.0
        };
        let mut start = cands[0];
        for &n in &cands[1..] {
            if score(n) < score(start) {
                start = n;
            }
        }
        let mut cur = start;
        let mut incoming: Option<Point> = None;
        let mut pts: Vec<Point> = Vec::new();
        loop {
            let mut opts: Vec<(i64, Vec<Point>, i64)> = Vec::new();
            for (eid, _) in unused.iter() {
                let e = &g.edges[&eid];
                let (u, v) = (e.u.expect("open"), e.v.expect("open"));
                if u == cur {
                    opts.push((eid, e.pts.clone(), v));
                }
                if v == cur && u != v {
                    opts.push((eid, e.pts.iter().rev().copied().collect(), u));
                }
            }
            if opts.is_empty() {
                break;
            }
            let live_idx: Vec<usize> = (0..opts.len()).filter(|&i| deg.get(&opts[i].2) != Some(&1)).collect();
            let live: Vec<usize> = if live_idx.is_empty() { (0..opts.len()).collect() } else { live_idx };
            let key = |i: usize| -> f64 {
                let d = dir(&opts[i].1, true, look);
                match incoming {
                    None => d.1,
                    Some(inc) => inc.0 * d.0 + inc.1 * d.1,
                }
            };
            let mut best = live[0];
            let mut best_key = key(best);
            for &i in &live[1..] {
                let k = key(i);
                if k > best_key {
                    best = i;
                    best_key = k;
                }
            }
            let (eid, p, nxt) = opts.swap_remove(best);
            unused.discard((eid, 0), hash_int(eid));
            if pts.is_empty() {
                pts.extend_from_slice(&p);
            } else {
                pts.extend_from_slice(&p[1..]);
            }
            incoming = Some(dir(&p, false, look));
            cur = nxt;
        }
        let closed = cur == start && pts.len() > 2;
        out.push((pts, closed, deg.get(&start) == Some(&1), deg.get(&cur) == Some(&1)));
    }
    for e in g.edges.values() {
        if e.closed {
            out.push((orient_loop(&e.pts), true, false, false));
        }
    }
    for (&n, &d) in &deg {
        if d == 0 {
            out.push((vec![g.nodes[&n]], false, false, false));
        }
    }
    out
}

fn orient_loop(pts: &[Point]) -> Vec<Point> {
    let mut a: Vec<Point> = if pts[0] == pts[pts.len() - 1] { pts[..pts.len() - 1].to_vec() } else { pts.to_vec() };
    let n = a.len();
    let x: Vec<f64> = a.iter().map(|p| p.1).collect();
    let y: Vec<f64> = a.iter().map(|p| -p.0).collect();
    let terms: Vec<f64> = (0..n).map(|i| x[i] * y[(i + 1) % n] - x[(i + 1) % n] * y[i]).collect();
    let area = 0.5 * np_sum(&terms);
    if area < 0.0 {
        a.reverse();
    }
    let mut best = 0;
    for k in 1..n {
        if a[k].1 - a[k].0 > a[best].1 - a[best].0 {
            best = k;
        }
    }
    a.rotate_left(best);
    a.push(a[0]);
    a
}

fn gaussian_weights(sigma: f64) -> Vec<f64> {
    let lw = (4.0 * sigma + 0.5) as i64;
    let sigma2 = sigma * sigma;
    let phi: Vec<f64> = (-lw..=lw).map(|x| (-0.5 / sigma2 * (x * x) as f64).exp()).collect();
    let s = np_sum(&phi);
    phi.iter().map(|v| v / s).collect()
}

fn correlate_symmetric(line: &[f64], w: &[f64], wrap: bool) -> Vec<f64> {
    let n = line.len() as i64;
    let size1 = (w.len() / 2) as i64;
    let get = |i: i64| -> f64 { if wrap { line[i.rem_euclid(n) as usize] } else { line[i.clamp(0, n - 1) as usize] } };
    let fw = |j: i64| w[(size1 + j) as usize];
    (0..n)
        .map(|ll| {
            let mut o = get(ll) * fw(0);
            for jj in -size1..0 {
                o += (get(ll + jj) + get(ll - jj)) * fw(jj);
            }
            o
        })
        .collect()
}

fn smooth(pts: &[Point], sigma: f64, closed: bool) -> Vec<Point> {
    if sigma <= 0.0 || pts.len() < 3 {
        return pts.to_vec();
    }
    let w = gaussian_weights(sigma);
    let filt = |vals: &[f64], wrap: bool| correlate_symmetric(vals, &w, wrap);
    if closed {
        let body = &pts[..pts.len() - 1];
        let xs = filt(&body.iter().map(|p| p.0).collect::<Vec<_>>(), true);
        let ys = filt(&body.iter().map(|p| p.1).collect::<Vec<_>>(), true);
        let mut out: Vec<Point> = xs.into_iter().zip(ys).collect();
        out.push(out[0]);
        return out;
    }
    let n = pts.len();
    let pad = ((n - 1) as f64).min((3.0 * sigma).ceil()) as usize;
    let mut padded: Vec<Point> = Vec::with_capacity(n + 2 * pad);
    for i in 0..pad {
        let q = pts[pad - i];
        padded.push((2.0 * pts[0].0 - q.0, 2.0 * pts[0].1 - q.1));
    }
    padded.extend_from_slice(pts);
    for j in 0..pad {
        let q = pts[n - 2 - j];
        padded.push((2.0 * pts[n - 1].0 - q.0, 2.0 * pts[n - 1].1 - q.1));
    }
    let xs = filt(&padded.iter().map(|p| p.0).collect::<Vec<_>>(), false);
    let ys = filt(&padded.iter().map(|p| p.1).collect::<Vec<_>>(), false);
    let mut out: Vec<Point> = (pad..pad + n).map(|i| (xs[i], ys[i])).collect();
    out[0] = pts[0];
    out[n - 1] = pts[n - 1];
    out
}

fn extend(a: Vec<Point>, at_start: bool, dt: &Dt, mask: &Mask, factor: f64, look: f64) -> Vec<Point> {
    if factor <= 0.0 || a.len() < 2 {
        return a;
    }
    let p = if at_start { a[0] } else { a[a.len() - 1] };
    let (r, c) = (py_round_index(p.0), py_round_index(p.1));
    if !(0 <= r && r < dt.rows as i64 && 0 <= c && c < dt.cols as i64) {
        return a;
    }
    let reach = dt.at(r as usize, c as usize) * factor;
    let d = if at_start {
        let d = dir(&a, true, look);
        (-d.0, -d.1)
    } else {
        dir(&a, false, look)
    };
    if (d.0 == 0.0 && d.1 == 0.0) || reach <= 0.0 {
        return a;
    }
    let (step, mut dist, mut last) = (0.5, 0.0, p);
    while dist + step <= reach {
        let q = (p.0 + d.0 * (dist + step), p.1 + d.1 * (dist + step));
        let (rr, cc) = (py_round_index(q.0), py_round_index(q.1));
        if !(0 <= rr && rr < mask.rows as i64 && 0 <= cc && cc < mask.cols as i64)
            || !mask.get(rr as usize, cc as usize)
        {
            break;
        }
        dist += step;
        last = q;
    }
    if dist <= 0.0 {
        return a;
    }
    let mut out = a;
    if at_start {
        out.insert(0, last);
    } else {
        out.push(last);
    }
    out
}

pub type Centerlines = (Vec<Vec<Point>>, Vec<bool>, Vec<Vec<Point>>);

pub fn mask_centerlines(mask: &Mask, xh_px: f64, params: &SkeletonParams, want_raw: bool) -> Centerlines {
    let dt = Dt { rows: mask.rows, cols: mask.cols, data: distance_transform(mask) };
    let skel = skeletonize(mask);
    let mut g = build_graph(&skel);
    let raw = if want_raw { g.edges.values().map(|e| e.pts.clone()).collect() } else { Vec::new() };
    prune(&mut g, params.prune * xh_px, Some(&dt));
    contract_junctions(&mut g, &dt, params.junction_merge);
    let look = numeric::max(3.0, 0.08 * xh_px);
    let paths = traverse(&g, look);
    let sigma = params.smooth * xh_px / 2.0;
    let tol = params.simplify * xh_px;
    let mut out = Vec::new();
    let mut closed_flags = Vec::new();
    for (pts, closed, start_end, end_end) in paths {
        let mut a = smooth(&pts, sigma, closed);
        if !closed {
            if start_end {
                a = extend(a, true, &dt, mask, params.extend, look);
            }
            if end_end {
                a = extend(a, false, &dt, mask, params.extend, look);
            }
        }
        let simp = if a.len() > 2 { rdp(&a, tol) } else { a };
        out.push(simp);
        closed_flags.push(closed);
    }
    (out, closed_flags, raw)
}

fn longest(strokes: &[Vec<Point>]) -> usize {
    let lens: Vec<f64> = strokes.iter().map(|s| np_length(&s.iter().map(|p| (p.1, p.0)).collect::<Vec<_>>())).collect();
    let mut best = 0;
    for i in 1..lens.len() {
        if lens[i] > lens[best] {
            best = i;
        }
    }
    best
}

pub fn skeleton_strokes(
    contours_em: &[Vec<Point>],
    x_height_em: f64,
    params: &SkeletonParams,
    want_raw: bool,
) -> SkeletonResult {
    let pts: Vec<Point> = contours_em.iter().flatten().copied().collect();
    if pts.is_empty() {
        return SkeletonResult::default();
    }
    let ppem = params.px_per_em;
    let xh_px = numeric::max(x_height_em * ppem, 1.0);
    let fold = |f: fn(f64, f64) -> f64, g: fn(&Point) -> f64| pts.iter().map(g).reduce(f).expect("points");
    let xmin = fold(numeric::min, |p| p.0);
    let ymax = fold(numeric::max, |p| p.1);
    let xmax = fold(numeric::max, |p| p.0);
    let ymin = fold(numeric::min, |p| p.1);
    let pad = 4.0;
    let w = ((xmax - xmin) * ppem).ceil() as usize + 8;
    let h = ((ymax - ymin) * ppem).ceil() as usize + 8;
    let to_px = |p: &Point| ((p.0 - xmin) * ppem + pad, (ymax - p.1) * ppem + pad);
    let to_em = |rc: &Point| (rc.1 / ppem - pad / ppem + xmin, ymax - (rc.0 / ppem - pad / ppem));
    let contours_px: Vec<Vec<Point>> = contours_em.iter().map(|c| c.iter().map(to_px).collect()).collect();
    let mask = rasterize(&contours_px, w, h);
    if !mask.any() {
        return SkeletonResult::default();
    }
    let (paths_rc, closed_flags, raw_rc) = mask_centerlines(&mask, xh_px, params, want_raw);
    let raw: Vec<Vec<Point>> = raw_rc.iter().map(|e| e.iter().map(to_em).collect()).collect();
    let strokes: Vec<Vec<Point>> = paths_rc.iter().map(|p| p.iter().map(to_em).collect()).collect();
    let li = if strokes.is_empty() { 0 } else { longest(&strokes) };
    let mut order: Vec<usize> = (0..strokes.len()).collect();
    let key = |i: usize| (i != li, strokes[i].iter().map(|p| p.0).reduce(numeric::min).expect("points"));
    order.sort_by(|&a, &b| {
        let (ka, kb) = (key(a), key(b));
        ka.0.cmp(&kb.0).then(ka.1.partial_cmp(&kb.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    SkeletonResult {
        strokes: order.iter().map(|&i| strokes[i].clone()).collect(),
        raw,
        closed: order.iter().map(|&i| closed_flags[i]).collect(),
    }
}

pub fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len();
    if n % 2 == 1 { values[n / 2] } else { (values[n / 2 - 1] + values[n / 2]) / 2.0 }
}

pub fn reference_px(mask: &Mask) -> f64 {
    let dt = distance_transform(mask);
    let sk = skeletonize(mask);
    let mut vals: Vec<f64> = (0..mask.rows * mask.cols).filter(|&i| sk.data[i]).map(|i| dt[i]).collect();
    let r = if vals.is_empty() { 1.0 } else { median(&mut vals) };
    numeric::max(8.0, 25.0 * numeric::max(r, 0.5))
}

pub fn mask_to_paths(mask: &Mask, px_mm: f64, x0: f64, y_top: f64) -> Vec<Vec<Point>> {
    if !mask.any() {
        return Vec::new();
    }
    let (paths_rc, _, _) = mask_centerlines(mask, reference_px(mask), &SkeletonParams::default(), false);
    paths_rc
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|p| p.into_iter().map(|(r, c)| (x0 + c * px_mm, y_top - r * px_mm)).collect())
        .collect()
}

pub fn fill_centerlines(contours: &[Vec<Point>]) -> Vec<Vec<Point>> {
    let (max_px, px_per_mm) = (1200.0, 40.0);
    let pts: Vec<Point> = contours.iter().flatten().copied().collect();
    if pts.is_empty() {
        return Vec::new();
    }
    let xmin = pts.iter().map(|p| p.0).reduce(numeric::min).expect("points");
    let xmax = pts.iter().map(|p| p.0).reduce(numeric::max).expect("points");
    let ymin = pts.iter().map(|p| p.1).reduce(numeric::min).expect("points");
    let ymax = pts.iter().map(|p| p.1).reduce(numeric::max).expect("points");
    let size = numeric::max(numeric::max(xmax - xmin, ymax - ymin), 1e-6);
    let ppm = numeric::min(px_per_mm, max_px / size);
    let pad = 3.0;
    let w = ((xmax - xmin) * ppm).ceil() as usize + 6;
    let h = ((ymax - ymin) * ppm).ceil() as usize + 6;
    let to_px = |p: &Point| ((p.0 - xmin) * ppm + pad, (ymax - p.1) * ppm + pad);
    let mask = rasterize(&contours.iter().map(|c| c.iter().map(to_px).collect()).collect::<Vec<_>>(), w, h);
    mask_to_paths(&mask, 1.0 / ppm, xmin + (0.5 - pad) / ppm, ymax - (0.5 - pad) / ppm)
}
