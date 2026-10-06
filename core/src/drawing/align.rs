use std::collections::{BTreeMap, VecDeque};

use indexmap::IndexMap;

use crate::checks::check_bounds;
use crate::drawing::passes::intersect;
use crate::drawing::passes::{corner, corner_name};
use crate::drawing::pipeline::{DrawingComposition, PassPart, file_stem};
use crate::drawing::place::Rect;
use crate::gcode::{fmt, generate_gcode};
use crate::geometry::Point;
use crate::numeric::round_to;

const SIZES: [(f64, i32); 3] = [(14.0, 4), (12.0, 3), (10.0, 2)];
const ZERO_TICK: f64 = 2.8;

#[derive(Debug, Clone, PartialEq)]
pub struct AlignMark {
    pub id: usize,
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub ticks: i32,
    pub scale_rot: i64,
    pub pointer_rot: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignReading {
    pub mark: usize,
    pub x: f64,
    pub y: f64,
}

fn part_by_rot(c: &DrawingComposition, rot: i64) -> Option<&PassPart> {
    c.parts.iter().find(|p| p.rotation == rot)
}

fn seam_point(c: &DrawingComposition, a: i64, b: i64) -> Option<Point> {
    let root = c.split.as_ref()?;
    for n in root.seams() {
        let (ax, other) = (n.axis, 1 - n.axis);
        let lo = if other == 0 { n.core.0 } else { n.core.1 };
        let hi = if other == 0 { n.core.2 } else { n.core.3 };
        let steps = 40;
        let mut hits = Vec::new();
        for k in 0..=steps {
            let t = lo + (hi - lo) * k as f64 / steps as f64;
            let mut p = [0.0, 0.0];
            p[ax] = n.s;
            p[other] = t;
            let (mut l, mut h) = (p, p);
            l[ax] -= 1e-6;
            h[ax] += 1e-6;
            let (ra, rb) = (n.leaf_at((l[0], l[1])).rotation, n.leaf_at((h[0], h[1])).rotation);
            if (ra, rb) == (Some(a), Some(b)) || (ra, rb) == (Some(b), Some(a)) {
                hits.push((p[0], p[1]));
            }
        }
        if !hits.is_empty() {
            return Some(hits[hits.len() / 2]);
        }
    }
    None
}

fn find(parent: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while parent[r] != r {
        r = parent[r];
    }
    parent[i] = r;
    r
}

pub fn align_marks(c: &DrawingComposition) -> Vec<AlignMark> {
    if c.parts.len() < 2 || !c.errors.is_empty() {
        return Vec::new();
    }
    for (size, ticks) in SIZES {
        let h = size / 2.0 + 0.5;
        let mut pairs: Vec<(bool, f64, usize, usize, Rect)> = Vec::new();
        for i in 0..c.parts.len() {
            for j in i + 1..c.parts.len() {
                let (ra, rb) = (c.parts[i].rotation, c.parts[j].rotation);
                let (Some(&a), Some(&b)) = (c.pass_rects.get(&ra), c.pass_rects.get(&rb)) else { continue };
                let Some(r) = intersect(a, b) else { continue };
                let inner = (r.0 + h, r.1 + h, r.2 - h, r.3 - h);
                if inner.2 < inner.0 || inner.3 < inner.1 {
                    continue;
                }
                pairs.push((seam_point(c, ra, rb).is_some(), (r.2 - r.0) * (r.3 - r.1), i, j, inner));
            }
        }
        pairs.sort_by(|x, y| y.0.cmp(&x.0).then(y.1.total_cmp(&x.1)));
        let mut parent: Vec<usize> = (0..c.parts.len()).collect();
        let mut out: Vec<AlignMark> = Vec::new();
        for (_, _, i, j, inner) in pairs {
            let (fi, fj) = (find(&mut parent, i), find(&mut parent, j));
            if fi == fj {
                continue;
            }
            let (ra, rb) = (c.parts[i].rotation, c.parts[j].rotation);
            let center = ((inner.0 + inner.2) / 2.0, (inner.1 + inner.3) / 2.0);
            let target = seam_point(c, ra, rb).unwrap_or(center);
            let clamp = |p: Point| (p.0.clamp(inner.0, inner.2), p.1.clamp(inner.1, inner.3));
            let free = |p: Point| {
                out.iter().all(|o: &AlignMark| (o.x - p.0).abs() > size + 1.0 || (o.y - p.1).abs() > size + 1.0)
            };
            let Some(p) = [clamp(target), center].into_iter().find(|&p| free(p)) else { continue };
            parent[fi] = fj;
            out.push(AlignMark { id: 0, x: p.0, y: p.1, size, ticks, scale_rot: ra, pointer_rot: rb });
        }
        let root = find(&mut parent, 0);
        if (0..c.parts.len()).all(|k| find(&mut parent, k) == root) {
            out.sort_by_key(|m| {
                (
                    part_by_rot(c, m.scale_rot).map_or(0, |p| p.index),
                    part_by_rot(c, m.pointer_rot).map_or(0, |p| p.index),
                )
            });
            for (k, m) in out.iter_mut().enumerate() {
                m.id = k + 1;
            }
            return out;
        }
    }
    Vec::new()
}

fn dot(x: f64, y: f64) -> Vec<Point> {
    let r = 0.45;
    (0..=8)
        .map(|k| {
            let a = k as f64 * std::f64::consts::PI / 4.0;
            (x + r * a.cos(), y + r * a.sin())
        })
        .collect()
}

pub fn scale_strokes(m: &AlignMark) -> Vec<Vec<Point>> {
    let n = m.ticks;
    let h = m.size / 2.0;
    let band = h - ZERO_TICK;
    let tick = |i: i32| {
        if i == 0 {
            ZERO_TICK
        } else if i.abs() == n {
            2.0
        } else {
            1.2
        }
    };
    let mut out = Vec::new();
    for i in -n..=n {
        let x = m.x + i as f64;
        out.push(vec![(x, m.y + band), (x, m.y + band + tick(i))]);
    }
    out.push(dot(m.x + n as f64, m.y + band + tick(n) + 0.9));
    for j in -n..=n {
        let y = m.y + j as f64;
        out.push(vec![(m.x + band, y), (m.x + band + tick(j), y)]);
    }
    out.push(dot(m.x + band + tick(n) + 0.9, m.y + n as f64));
    for k in 0..m.id {
        let x = m.x - h + 0.5 + k as f64 * 0.8;
        out.push(vec![(x, m.y - h + 0.3), (x, m.y - h + 1.3)]);
    }
    out
}

pub fn pointer_strokes(m: &AlignMark) -> Vec<Vec<Point>> {
    let h = m.size / 2.0;
    vec![vec![(m.x, m.y - h + 2.0), (m.x, m.y + h + 0.5)], vec![(m.x - h + 2.0, m.y), (m.x + h + 0.5, m.y)]]
}

pub fn align_part_strokes(marks: &[AlignMark], rot: i64) -> Vec<Vec<Point>> {
    let mut out = Vec::new();
    for m in marks {
        if m.scale_rot == rot {
            out.extend(scale_strokes(m));
        }
        if m.pointer_rot == rot {
            out.extend(pointer_strokes(m));
        }
    }
    out
}

pub fn align_filename(c: &DrawingComposition, part: &PassPart) -> String {
    format!("{}_pass{}_rot{}_align.gcode", file_stem(c), part.index, part.rotation)
}

pub fn make_align_gcode(c: &DrawingComposition, marks: &[AlignMark], part: &PassPart) -> Result<String, Vec<String>> {
    if !c.errors.is_empty() {
        return Err(c.errors.clone());
    }
    let sheet = align_part_strokes(marks, part.rotation);
    if sheet.is_empty() {
        return Err(vec![format!("Проход {}: на стыках нет места для меток совмещения", part.index)]);
    }
    let strokes = c.part_strokes(part, Some(&sheet));
    let settings = c.part_settings(part.rotation);
    let errors = check_bounds(&strokes, &settings, 5);
    if !errors.is_empty() {
        return Err(errors.into_iter().map(|m| format!("Тест совмещения, проход {}: {m}", part.index)).collect());
    }
    let n = c.parts.len();
    let cn = corner(part.rotation);
    let header = vec![
        format!("ALIGNMENT TEST PASS {}/{n}: blank sheet, placed exactly as for the drawing", part.index),
        format!(
            "sheet rotated {} deg, corner {cn} ({}) at the stops, pencil at zero",
            part.rotation,
            crate::gcode::ascii(corner_name(cn))
        ),
        format!("zero correction dx {} dy {} mm (pass coordinates)", fmt(part.dx), fmt(part.dy)),
        "scales: 1 mm ticks, longest tick = 0, dot marks the + side; pointer = single long line".into(),
    ];
    Ok(generate_gcode(&strokes, &settings, &header, None))
}

pub fn align_correction(
    c: &DrawingComposition,
    marks: &[AlignMark],
    readings: &[AlignReading],
) -> Result<IndexMap<String, (f64, f64)>, Vec<String>> {
    let mut sums: BTreeMap<(i64, i64), (f64, f64, usize)> = BTreeMap::new();
    for r in readings {
        let Some(m) = marks.iter().find(|m| m.id == r.mark) else {
            return Err(vec![format!("Нет метки {}", r.mark)]);
        };
        let e = sums.entry((m.scale_rot, m.pointer_rot)).or_insert((0.0, 0.0, 0));
        e.0 += r.x;
        e.1 += r.y;
        e.2 += 1;
    }
    let first = c.parts.first().ok_or_else(|| vec!["Нет проходов".to_string()])?.rotation;
    let mut err: BTreeMap<i64, (f64, f64)> = BTreeMap::from([(first, (0.0, 0.0))]);
    let mut queue = VecDeque::from([first]);
    while let Some(r) = queue.pop_front() {
        let base = err[&r];
        for (&(a, b), &(sx, sy, k)) in &sums {
            let d = (sx / k as f64, sy / k as f64);
            let (next, v) = if a == r {
                (b, (base.0 + d.0, base.1 + d.1))
            } else if b == r {
                (a, (base.0 - d.0, base.1 - d.1))
            } else {
                continue;
            };
            if let std::collections::btree_map::Entry::Vacant(e) = err.entry(next) {
                e.insert(v);
                queue.push_back(next);
            }
        }
    }
    let missing: Vec<String> =
        c.parts.iter().filter(|p| !err.contains_key(&p.rotation)).map(|p| p.index.to_string()).collect();
    if !missing.is_empty() {
        return Err(vec![format!("Нет замеров, которые связывают проход {} с проходом 1", missing.join(", "))]);
    }
    let mut out = c.settings.drawing.split.offsets.clone();
    for p in &c.parts {
        let e = err[&p.rotation];
        let o = c.to_machine((0.0, 0.0), p.rotation);
        let q = c.to_machine((-e.0, -e.1), p.rotation);
        let old = out.get(&p.rotation.to_string()).copied().unwrap_or((0.0, 0.0));
        out.insert(p.rotation.to_string(), (round_to(old.0 + q.0 - o.0, 2), round_to(old.1 + q.1 - o.1, 2)));
    }
    Ok(out)
}
