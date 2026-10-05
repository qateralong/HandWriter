use crate::numeric;

pub type Point = (f64, f64);

pub fn rdp(points: &[Point], tol: f64) -> Vec<Point> {
    rdp_indices(points, tol, &[]).into_iter().map(|i| points[i]).collect()
}

pub fn rdp_indices(points: &[Point], tol: f64, must_keep: &[usize]) -> Vec<usize> {
    let n = points.len();
    if n < 3 || tol <= 0.0 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    for &i in must_keep {
        if i < n {
            keep[i] = true;
        }
    }
    let fixed: Vec<usize> = (0..n).filter(|&i| keep[i]).collect();
    let mut stack: Vec<(usize, usize)> = fixed.windows(2).filter(|w| w[1] - w[0] > 1).map(|w| (w[0], w[1])).collect();
    while let Some((i, j)) = stack.pop() {
        let (ax, ay) = points[i];
        let (bx, by) = points[j];
        let (dx, dy) = (bx - ax, by - ay);
        let len = numeric::hypot(dx, dy);
        let (mut best, mut idx) = (-1.0_f64, 0usize);
        for (k, &(px, py)) in points.iter().enumerate().take(j).skip(i + 1) {
            let d = if len == 0.0 {
                numeric::hypot(px - ax, py - ay)
            } else {
                (dy * (px - ax) - dx * (py - ay)).abs() / len
            };
            if d > best {
                best = d;
                idx = k;
            }
        }
        if best > tol {
            keep[idx] = true;
            stack.push((i, idx));
            stack.push((idx, j));
        }
    }
    (0..n).filter(|&i| keep[i]).collect()
}

pub fn make_transform(rotation_deg: f64, dx: f64, dy: f64) -> impl Fn(Point) -> Point {
    let a = rotation_deg.to_radians();
    let (c, s) = (a.cos(), a.sin());
    move |(x, y)| (c * x - s * y + dx, s * x + c * y + dy)
}

pub fn polyline_length(points: &[Point]) -> f64 {
    numeric::sum(points.windows(2).map(|w| numeric::dist(w[0], w[1])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rdp_drops_collinear_points() {
        let pts = [(0.0, 0.0), (1.0, 0.01), (2.0, 0.0), (3.0, 5.0)];
        assert_eq!(rdp(&pts, 0.1), vec![(0.0, 0.0), (2.0, 0.0), (3.0, 5.0)]);
        assert_eq!(rdp_indices(&pts, 0.1, &[1]), vec![0, 1, 2, 3]);
        assert_eq!(rdp(&pts, 0.0).len(), 4);
    }

    #[test]
    fn transform_rotates_then_shifts() {
        let t = make_transform(90.0, 1.0, 2.0);
        let (x, y) = t((1.0, 0.0));
        assert!((x - 1.0).abs() < 1e-12 && (y - 3.0).abs() < 1e-12);
        assert_eq!(polyline_length(&[(0.0, 0.0), (3.0, 4.0), (3.0, 0.0)]), 9.0);
    }
}
