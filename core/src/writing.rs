use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hint::black_box;

use crate::geometry::{Point, make_transform, rdp_indices};
use crate::glyphs::GlyphProvider;
use crate::layout::{DrawnStroke, PlacedGlyph};
use crate::numeric::{self, dist, hypot};
use crate::rand::vnoise;
use crate::settings::Settings;

const DRIFT_WAVELENGTH_MM: f64 = 40.0;
const JITTER_WAVELENGTH_MM: f64 = 2.5;
const RESAMPLE_MM: f64 = 0.4;
const BRIDGE_STEP_MM: f64 = 0.25;
const GLUE_EM_FRACTION: f64 = 0.02;
const DETAIL_EM_FRACTION: f64 = 0.45;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Join {
    Lift,
    Glue,
    Bridge,
}

#[derive(Debug, Clone, Copy)]
struct Elem {
    gi: usize,
    si: usize,
    rev: bool,
    join: Join,
}

type Local = HashMap<usize, Vec<Vec<Point>>>;
type DetailKey = (f64, usize, usize);

fn oriented(st: &[Point], rev: bool) -> Vec<Point> {
    if rev { st.iter().rev().copied().collect() } else { st.to_vec() }
}

fn first_of(st: &[Point], rev: bool) -> Point {
    if rev { st[st.len() - 1] } else { st[0] }
}

fn last_of(st: &[Point], rev: bool) -> Point {
    if rev { st[0] } else { st[st.len() - 1] }
}

fn fold_min(it: impl Iterator<Item = f64>) -> f64 {
    it.reduce(numeric::min).expect("non-empty")
}

fn fold_max(it: impl Iterator<Item = f64>) -> f64 {
    it.reduce(numeric::max).expect("non-empty")
}

fn diag(st: &[Point]) -> f64 {
    let w = fold_max(st.iter().map(|p| p.0)) - fold_min(st.iter().map(|p| p.0));
    let h = fold_max(st.iter().map(|p| p.1)) - fold_min(st.iter().map(|p| p.1));
    hypot(w, h)
}

fn pow(x: f64, n: f64) -> f64 {
    x.powf(black_box(n))
}

pub fn build_paths(s: &Settings, prov: &dyn GlyphProvider, glyphs: &[PlacedGlyph], scale: f64) -> Vec<DrawnStroke> {
    let xh = prov.metrics().x_height;
    let conn = &s.connections;
    let glue_d = GLUE_EM_FRACTION * xh;
    let detail_d = DETAIL_EM_FRACTION * xh;

    let mut local: Local = HashMap::new();
    for (gi, g) in glyphs.iter().enumerate() {
        if let Some(name) = &g.glyph {
            let glyph = prov.glyph(name).expect("placed glyph exists");
            let st: Vec<Vec<Point>> = glyph.strokes.into_iter().filter(|x| !x.is_empty()).collect();
            if !st.is_empty() {
                local.insert(gi, st);
            }
        }
    }

    let mut segments: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (gi, g) in glyphs.iter().enumerate() {
        segments.entry(g.segment).or_default().push(gi);
    }

    let conn_d = if conn.enabled { conn.distance * xh } else { -1.0 };
    let mut elems = Vec::new();
    for seg in segments.values() {
        elems.extend(order_segment(seg, glyphs, &local, conn_d, glue_d, detail_d));
    }
    let polylines = assemble(&elems, glyphs, &local, scale, glue_d * scale, xh * scale);
    finish(s, glyphs, polylines)
}

fn candidate_less(a: &(f64, bool, bool, usize, usize), b: &(f64, bool, bool, usize, usize)) -> bool {
    if a.0 != b.0 {
        return a.0 < b.0;
    }
    (a.1, a.2, a.3, a.4) < (b.1, b.2, b.3, b.4)
}

fn order_segment(
    seg: &[usize],
    glyphs: &[PlacedGlyph],
    local: &Local,
    conn_d: f64,
    glue_d: f64,
    detail_d: f64,
) -> Vec<Elem> {
    let mut origin: HashMap<usize, Point> = HashMap::new();
    let mut ox = 0.0;
    for &gi in seg {
        let g = &glyphs[gi];
        origin.insert(gi, (ox + g.dx_em, g.dy_em));
        ox += g.adv_em;
    }

    let mut entry: HashMap<usize, usize> = HashMap::new();
    let mut exit: HashMap<usize, usize> = HashMap::new();
    let mut orient: HashMap<(usize, usize), bool> = HashMap::new();
    let mut connected: HashSet<(usize, usize)> = HashSet::new();

    if conn_d >= 0.0 {
        for w in seg.windows(2) {
            let (ga, gb) = (w[0], w[1]);
            let (Some(la), Some(lb)) = (local.get(&ga), local.get(&gb)) else { continue };
            if glyphs[ga].hyphen || glyphs[gb].hyphen {
                continue;
            }
            let mut best: Option<(f64, bool, bool, usize, usize)> = None;
            let (ax, ay) = origin[&ga];
            let (bx, by) = origin[&gb];
            for (sa, sta) in la.iter().enumerate() {
                if sta.len() < 2 {
                    continue;
                }
                for ra in [false, true] {
                    if orient.get(&(ga, sa)).is_some_and(|&o| o != ra) {
                        continue;
                    }
                    let pa = last_of(sta, ra);
                    for (sb, stb) in lb.iter().enumerate() {
                        if stb.len() < 2 {
                            continue;
                        }
                        for rb in [false, true] {
                            let pb = first_of(stb, rb);
                            let d = hypot(pa.0 + ax - pb.0 - bx, pa.1 + ay - pb.1 - by);
                            let cand = (d, ra, rb, sa, sb);
                            if best.as_ref().is_none_or(|b| candidate_less(&cand, b)) {
                                best = Some(cand);
                            }
                        }
                    }
                }
            }
            if let Some((d, ra, rb, sa, sb)) = best
                && d <= conn_d
            {
                exit.insert(ga, sa);
                entry.insert(gb, sb);
                orient.insert((ga, sa), ra);
                orient.insert((gb, sb), rb);
                connected.insert((ga, gb));
            }
        }
    }

    let mut main: Vec<Elem> = Vec::new();
    let mut details: Vec<(usize, usize)> = Vec::new();
    let mut prev_gi: Option<usize> = None;
    for &gi in seg {
        let Some(strokes) = local.get(&gi) else {
            prev_gi = None;
            continue;
        };
        let (e, x) = (entry.get(&gi).copied(), exit.get(&gi).copied());
        let others: Vec<usize> = (0..strokes.len()).filter(|&si| Some(si) != e && Some(si) != x).collect();
        let mains: Vec<usize> = if glyphs[gi].hyphen {
            others.clone()
        } else {
            others.iter().copied().filter(|&si| diag(&strokes[si]) >= detail_d).collect()
        };
        details.extend(others.iter().filter(|si| !mains.contains(si)).map(|&si| (gi, si)));
        let mut seq: Vec<(usize, bool)> = Vec::new();
        if let Some(e) = e {
            seq.push((e, orient[&(gi, e)]));
        }
        if e.is_some() && e == x {
            details.extend(mains.iter().map(|&si| (gi, si)));
        } else {
            let cur = e.map(|e| last_of(&strokes[e], orient[&(gi, e)]));
            seq.extend(chain(strokes, &mains, cur, glue_d));
            if let Some(x) = x {
                seq.push((x, orient[&(gi, x)]));
            }
        }
        for (k, &(si, rev)) in seq.iter().enumerate() {
            let join = if k == 0 {
                if prev_gi.is_some_and(|p| connected.contains(&(p, gi))) && Some(si) == e {
                    Join::Bridge
                } else {
                    Join::Lift
                }
            } else {
                let (psi, prev) = seq[k - 1];
                let prev_end = last_of(&strokes[psi], prev);
                let start = first_of(&strokes[si], rev);
                if dist(prev_end, start) <= glue_d { Join::Glue } else { Join::Lift }
            };
            main.push(Elem { gi, si, rev, join });
        }
        prev_gi = Some(gi);
    }

    let left = |&(gi, si): &(usize, usize)| (fold_min(local[&gi][si].iter().map(|p| p.0)) + origin[&gi].0, gi, si);
    let mut keyed: Vec<(DetailKey, (usize, usize))> = details.iter().map(|d| (left(d), *d)).collect();
    keyed.sort_by(|a, b| {
        let (ka, kb) = (a.0, b.0);
        if ka.0 != kb.0 {
            return ka.0.partial_cmp(&kb.0).unwrap_or(Ordering::Equal);
        }
        (ka.1, ka.2).cmp(&(kb.1, kb.2))
    });

    let mut out = main;
    let mut last: Option<(usize, usize, bool)> = None;
    for (_, (gi, si)) in keyed {
        let st = &local[&gi][si];
        let mut rev = false;
        let mut join = Join::Lift;
        if let Some((lgi, lsi, lrev)) = last
            && lgi == gi
        {
            let end = last_of(&local[&gi][lsi], lrev);
            if dist(end, st[0]) <= glue_d {
                join = Join::Glue;
            } else if dist(end, st[st.len() - 1]) <= glue_d {
                join = Join::Glue;
                rev = true;
            }
        }
        out.push(Elem { gi, si, rev, join });
        last = Some((gi, si, rev));
    }
    out
}

fn chain(strokes: &[Vec<Point>], pool: &[usize], cur: Option<Point>, glue_d: f64) -> Vec<(usize, bool)> {
    let mut rest: Vec<usize> = pool.to_vec();
    let mut out = Vec::new();
    let mut cur = cur;
    while !rest.is_empty() {
        let mut picked = None;
        if let Some(c) = cur {
            for &si in &rest {
                let st = &strokes[si];
                if dist(c, st[0]) <= glue_d {
                    picked = Some((si, false));
                    break;
                }
                if dist(c, st[st.len() - 1]) <= glue_d {
                    picked = Some((si, true));
                    break;
                }
            }
        }
        let p = picked.unwrap_or((rest[0], false));
        let pos = rest.iter().position(|&si| si == p.0).expect("picked from rest");
        rest.remove(pos);
        out.push(p);
        cur = Some(last_of(&strokes[p.0], p.1));
    }
    out
}

fn glyph_transform(g: &PlacedGlyph, scale: f64) -> impl Fn(Point) -> Point + use<> {
    let k = scale * g.size;
    let sh = g.slant.to_radians().tan();
    let (ox, oy) = (g.x, g.y + g.voff);
    let (dx, dy) = (g.dx_em, g.dy_em);
    move |(x, y)| (ox + (dx + x + y * sh) * k, oy + (dy + y) * k)
}

fn direction(pts: &[Point], at_end: bool, look: f64) -> Option<Point> {
    let seq: Vec<Point> = if at_end { pts.iter().rev().copied().collect() } else { pts.to_vec() };
    let p0 = seq[0];
    let unit = |q: Point| {
        let v = if at_end { (p0.0 - q.0, p0.1 - q.1) } else { (q.0 - p0.0, q.1 - p0.1) };
        let n = hypot(v.0, v.1);
        (v.0 / n, v.1 / n)
    };
    for &q in &seq[1..] {
        if dist(p0, q) >= look {
            return Some(unit(q));
        }
    }
    if seq.len() > 1 && seq[seq.len() - 1] != p0 {
        return Some(unit(seq[seq.len() - 1]));
    }
    None
}

fn bridge(p0: Point, t0: Option<Point>, p1: Point, t1: Option<Point>) -> Vec<Point> {
    let d = dist(p0, p1);
    if d < 0.02 {
        return Vec::new();
    }
    let chord = ((p1.0 - p0.0) / d, (p1.1 - p0.1) / d);
    let t0 = t0.filter(|t| t.0 * chord.0 + t.1 * chord.1 > 0.0).unwrap_or(chord);
    let t1 = t1.filter(|t| t.0 * chord.0 + t.1 * chord.1 > 0.0).unwrap_or(chord);
    let h = 0.4 * d;
    let c1 = (p0.0 + t0.0 * h, p0.1 + t0.1 * h);
    let c2 = (p1.0 - t1.0 * h, p1.1 - t1.1 * h);
    let n = 2_i64.max((d / BRIDGE_STEP_MM).ceil() as i64);
    let mut out = Vec::new();
    for i in 1..n {
        let u = i as f64 / n as f64;
        let a = pow(1.0 - u, 3.0);
        let b = 3.0 * u * pow(1.0 - u, 2.0);
        let c = 3.0 * u * u * (1.0 - u);
        let e = pow(u, 3.0);
        out.push((a * p0.0 + b * c1.0 + c * c2.0 + e * p1.0, a * p0.1 + b * c1.1 + c * c2.1 + e * p1.1));
    }
    out
}

fn cut_tail(pts: &mut Vec<Point>, tags: &mut Vec<usize>, mut length: f64, floor: usize) {
    while length > 1e-9 && pts.len() - 1 > floor {
        let (a, b) = (pts[pts.len() - 2], pts[pts.len() - 1]);
        let seg = dist(a, b);
        if seg <= length {
            pts.pop();
            tags.pop();
            length -= seg;
        } else {
            let u = 1.0 - length / seg;
            let last = pts.len() - 1;
            pts[last] = (a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u);
            break;
        }
    }
}

fn cut_head(pts: Vec<Point>, mut length: f64) -> Vec<Point> {
    let mut i = 0;
    while length > 1e-9 && i + 1 < pts.len() {
        let (a, b) = (pts[i], pts[i + 1]);
        let seg = dist(a, b);
        if seg <= length {
            length -= seg;
            i += 1;
        } else {
            let u = length / seg;
            let mut out = vec![(a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u)];
            out.extend_from_slice(&pts[i + 1..]);
            return out;
        }
    }
    pts[i..].to_vec()
}

fn length(pts: &[Point]) -> f64 {
    numeric::sum(pts.windows(2).map(|w| dist(w[0], w[1])))
}

type Polyline = (Vec<Point>, Vec<usize>, BTreeSet<usize>);

fn assemble(
    elems: &[Elem],
    glyphs: &[PlacedGlyph],
    local: &Local,
    scale: f64,
    glue_mm: f64,
    xh_mm: f64,
) -> Vec<Polyline> {
    let mut out: Vec<Polyline> = Vec::new();
    let mut pts: Vec<Point> = Vec::new();
    let mut tags: Vec<usize> = Vec::new();
    let mut keep: BTreeSet<usize> = BTreeSet::new();

    for e in elems {
        let tf = glyph_transform(&glyphs[e.gi], scale);
        let mut new: Vec<Point> = oriented(&local[&e.gi][e.si], e.rev).into_iter().map(&tf).collect();
        if e.join == Join::Lift || pts.is_empty() {
            if !pts.is_empty() {
                out.push((std::mem::take(&mut pts), std::mem::take(&mut tags), std::mem::take(&mut keep)));
            }
            pts.clear();
            tags.clear();
            keep.clear();
        } else if e.join == Join::Bridge {
            let gap = dist(pts[pts.len() - 1], new[0]);
            let floor = keep.iter().copied().filter(|&i| i < pts.len()).max().unwrap_or(0);
            let want = numeric::min(numeric::max(1.5 * gap, 0.12 * xh_mm), 0.35 * xh_mm);
            let cut_a = numeric::min(want, 0.4 * length(&pts[floor..]));
            let cut_b = numeric::min(want, 0.4 * length(&new));
            cut_tail(&mut pts, &mut tags, cut_a, floor);
            new = cut_head(new, cut_b);
            let mid = bridge(pts[pts.len() - 1], direction(&pts, true, 0.3), new[0], direction(&new, false, 0.3));
            keep.insert(pts.len() - 1);
            let mut joined = mid;
            joined.extend(new);
            new = joined;
        } else if e.join == Join::Glue && dist(pts[pts.len() - 1], new[0]) <= numeric::max(glue_mm, 1e-9) {
            new.remove(0);
        }
        if !pts.is_empty() && tags[tags.len() - 1] != e.gi && !new.is_empty() {
            keep.insert(pts.len() - 1);
            keep.insert(pts.len());
        }
        tags.extend(std::iter::repeat_n(e.gi, new.len()));
        pts.extend(new);
    }
    if !pts.is_empty() {
        out.push((pts, tags, keep));
    }
    out
}

fn resample(pts: Vec<Point>, tags: Vec<usize>, keep: BTreeSet<usize>, step: f64) -> Polyline {
    if pts.len() < 2 {
        return (pts, tags, keep);
    }
    let mut npts = vec![pts[0]];
    let mut ntags = vec![tags[0]];
    let mut nkeep = BTreeSet::new();
    if keep.contains(&0) {
        nkeep.insert(0);
    }
    for i in 1..pts.len() {
        let (a, b) = (pts[i - 1], pts[i]);
        let n = (dist(a, b) / step) as i64;
        for k in 1..=n {
            let u = k as f64 / (n + 1) as f64;
            npts.push((a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u));
            ntags.push(tags[i]);
        }
        if keep.contains(&i) {
            nkeep.insert(npts.len());
        }
        npts.push(b);
        ntags.push(tags[i]);
    }
    (npts, ntags, nkeep)
}

fn finish(s: &Settings, glyphs: &[PlacedGlyph], polylines: Vec<Polyline>) -> Vec<DrawnStroke> {
    let r = &s.randomness;
    let ty = &s.typography;
    let noisy = r.enabled && (r.drift > 0.0 || r.jitter > 0.0);
    let t = make_transform(ty.rotation_deg, ty.dx, ty.dy);
    let mut seen: HashMap<(i64, i64), i64> = HashMap::new();
    let mut out = Vec::new();
    for (mut pts, mut tags, mut keep) in polylines {
        let g0 = &glyphs[tags[0]];
        let k = *seen.get(&(g0.word, g0.letter)).unwrap_or(&0);
        seen.insert((g0.word, g0.letter), k + 1);
        if noisy {
            if r.jitter > 0.0 {
                (pts, tags, keep) = resample(pts, tags, keep, RESAMPLE_MM);
            }
            let mut moved = Vec::with_capacity(pts.len());
            let mut s_len = 0.0;
            for (i, &(mut x, mut y)) in pts.iter().enumerate() {
                if i > 0 {
                    s_len += dist(pts[i - 1], pts[i]);
                }
                let g = &glyphs[tags[i]];
                if r.drift > 0.0 {
                    y += r.drift * vnoise(r.seed, "drift", x / DRIFT_WAVELENGTH_MM, &[g.line]);
                }
                if r.jitter > 0.0 {
                    let u = s_len / JITTER_WAVELENGTH_MM;
                    x += r.jitter * vnoise(r.seed, "jx", u, &[g0.word, g0.letter, k]);
                    y += r.jitter * vnoise(r.seed, "jy", u, &[g0.word, g0.letter, k]);
                }
                moved.push((x, y));
            }
            pts = moved;
        }
        let pts: Vec<Point> = pts.into_iter().map(&t).collect();
        let keep: Vec<usize> = keep.into_iter().collect();
        let idx = rdp_indices(&pts, s.printer.simplify_tol, &keep);
        let points: Vec<Point> = idx.iter().map(|&i| pts[i]).collect();
        let tags: Vec<usize> = idx.iter().map(|&i| tags[i]).collect();
        out.push(DrawnStroke {
            hyphen: tags.iter().all(|&i| glyphs[i].hyphen),
            points,
            word: g0.word,
            letter: g0.letter,
            line: g0.line,
            tags,
        });
    }
    out
}
