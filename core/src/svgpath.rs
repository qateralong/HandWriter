use std::f64::consts::PI;
use std::fmt;
use std::ops::{Add, Mul, Neg, Sub};
use std::sync::LazyLock;

use regex::Regex;

use crate::geometry::Point;

#[derive(Debug, Clone, PartialEq)]
pub struct PathError(pub String);

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PathError {}

pub trait Pen {
    fn move_to(&mut self, p: Point);
    fn line_to(&mut self, p: Point);
    fn curve_to(&mut self, c1: Point, c2: Point, p: Point);
    fn qcurve_to(&mut self, c: Point, p: Point);
    fn close_path(&mut self);
    fn end_path(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct C {
    re: f64,
    im: f64,
}

impl C {
    fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    fn from_xy(x: f64, y: f64) -> Self {
        Self::new(x + 0.0 * y, y)
    }

    fn scale(self, k: f64) -> Self {
        Self::new(self.re * k, self.im * k)
    }

    fn add_re(self, x: f64) -> Self {
        Self::new(self.re + x, self.im)
    }

    fn imag_unit(y: f64) -> Self {
        Self::new(0.0 * y, y)
    }

    fn point(self) -> Point {
        (self.re, self.im)
    }

    fn is_close(self, b: C, rel_tol: f64, abs_tol: f64) -> bool {
        let a = self;
        if a.re == b.re && a.im == b.im {
            return true;
        }
        if a.re.is_infinite() || a.im.is_infinite() || b.re.is_infinite() || b.im.is_infinite() {
            return false;
        }
        let diff = (a.re - b.re).hypot(a.im - b.im);
        diff <= rel_tol * b.re.hypot(b.im) || diff <= rel_tol * a.re.hypot(a.im) || diff <= abs_tol
    }
}

impl Add for C {
    type Output = C;
    fn add(self, o: C) -> C {
        C::new(self.re + o.re, self.im + o.im)
    }
}

impl Sub for C {
    type Output = C;
    fn sub(self, o: C) -> C {
        C::new(self.re - o.re, self.im - o.im)
    }
}

#[derive(Debug, Clone, Copy)]
enum N {
    I(i64),
    F(f64),
}

impl N {
    fn f(self) -> f64 {
        match self {
            N::I(i) => i as f64,
            N::F(x) => x,
        }
    }
}

impl Mul for N {
    type Output = N;
    fn mul(self, o: N) -> N {
        match (self, o) {
            (N::I(a), N::I(b)) => N::I(a * b),
            _ => N::F(self.f() * o.f()),
        }
    }
}

impl Add for N {
    type Output = N;
    fn add(self, o: N) -> N {
        match (self, o) {
            (N::I(a), N::I(b)) => N::I(a + b),
            _ => N::F(self.f() + o.f()),
        }
    }
}

impl Neg for N {
    type Output = N;
    fn neg(self) -> N {
        match self {
            N::I(a) => N::I(-a),
            N::F(x) => N::F(-x),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Transform([N; 6]);

const IDENTITY: Transform = Transform([N::I(1), N::I(0), N::I(0), N::I(1), N::I(0), N::I(0)]);

fn norm_sin_cos(v: f64) -> N {
    const EPS: f64 = 1e-15;
    if v.abs() < EPS {
        N::I(0)
    } else if v > 1.0 - EPS {
        N::I(1)
    } else if v < -1.0 + EPS {
        N::I(-1)
    } else {
        N::F(v)
    }
}

impl Transform {
    fn scaling(x: f64, y: f64) -> Self {
        Transform([N::F(x), N::I(0), N::I(0), N::F(y), N::I(0), N::I(0)])
    }

    fn transform(self, other: [N; 6]) -> Self {
        let [xx1, xy1, yx1, yy1, dx1, dy1] = other;
        let [xx2, xy2, yx2, yy2, dx2, dy2] = self.0;
        Transform([
            xx1 * xx2 + xy1 * yx2,
            xx1 * xy2 + xy1 * yy2,
            yx1 * xx2 + yy1 * yx2,
            yx1 * xy2 + yy1 * yy2,
            xx2 * dx1 + yx2 * dy1 + dx2,
            xy2 * dx1 + yy2 * dy1 + dy2,
        ])
    }

    fn rotate(self, angle: f64) -> Self {
        let c = norm_sin_cos(angle.cos());
        let s = norm_sin_cos(angle.sin());
        self.transform([c, s, -s, c, N::I(0), N::I(0)])
    }

    fn scale(self, x: f64, y: f64) -> Self {
        self.transform([N::F(x), N::I(0), N::I(0), N::F(y), N::I(0), N::I(0)])
    }

    fn map(self, pt: C) -> C {
        let [xx, xy, yx, yy, dx, dy] = self.0;
        let (x, y) = (N::F(pt.re), N::F(pt.im));
        let rx = (xx * x + yx * y + dx).f();
        let ry = (xy * x + yy * y + dy).f();
        C::from_xy(rx, ry)
    }
}

struct Arc {
    current: C,
    rx: f64,
    ry: f64,
    angle: f64,
    large: bool,
    sweep: bool,
    target: C,
}

struct ArcParams {
    center: C,
    theta1: f64,
    theta_arc: f64,
}

impl Arc {
    fn parametrize(&mut self) -> Option<ArcParams> {
        let mut rx = self.rx.abs();
        let mut ry = self.ry.abs();
        if rx == 0.0 || ry == 0.0 {
            return None;
        }
        if self.target == self.current {
            return None;
        }
        let mid = (self.current - self.target).scale(0.5);
        let tm = IDENTITY.rotate(-self.angle).map(mid);
        let square_rx = rx * rx;
        let square_ry = ry * ry;
        let square_x = tm.re * tm.re;
        let square_y = tm.im * tm.im;
        let radii_scale = square_x / square_rx + square_y / square_ry;
        if radii_scale > 1.0 {
            rx *= radii_scale.sqrt();
            ry *= radii_scale.sqrt();
            self.rx = rx;
            self.ry = ry;
        }
        let pt = Transform::scaling(1.0 / rx, 1.0 / ry).rotate(-self.angle);
        let mut p1 = pt.map(self.current);
        let mut p2 = pt.map(self.target);
        let mut delta = p2 - p1;
        let d = delta.re * delta.re + delta.im * delta.im;
        let sfs = crate::numeric::max(1.0 / d - 0.25, 0.0);
        let mut sf = sfs.sqrt();
        if self.sweep == self.large {
            sf = -sf;
        }
        delta = delta.scale(sf);
        let mut center = (p1 + p2).scale(0.5);
        center = center + C::new(-delta.im, delta.re);
        p1 = p1 - center;
        p2 = p2 - center;
        let theta1 = p1.im.atan2(p1.re);
        let theta2 = p2.im.atan2(p2.re);
        let mut theta_arc = theta2 - theta1;
        if theta_arc < 0.0 && self.sweep {
            theta_arc += 2.0 * PI;
        } else if theta_arc > 0.0 && !self.sweep {
            theta_arc -= 2.0 * PI;
        }
        Some(ArcParams { center, theta1, theta_arc })
    }

    fn draw(mut self, pen: &mut dyn Pen) {
        let Some(ap) = self.parametrize() else { return };
        let pt = IDENTITY.rotate(self.angle).scale(self.rx, self.ry);
        let n = (ap.theta_arc / (0.5 * PI + 0.001)).abs().ceil() as i64;
        for i in 0..n {
            let start = ap.theta1 + (i as f64 * ap.theta_arc) / n as f64;
            let end = ap.theta1 + ((i + 1) as f64 * ap.theta_arc) / n as f64;
            let t = (4.0 / 3.0) * (0.25 * (end - start)).tan();
            if !t.is_finite() {
                return;
            }
            let (ss, cs) = (start.sin(), start.cos());
            let (se, ce) = (end.sin(), end.cos());
            let mut p1 = C::new(cs - t * ss, ss + t * cs);
            p1 = p1 + ap.center;
            let mut target = C::new(ce, se);
            target = target + ap.center;
            let mut p2 = target;
            p2 = p2 + C::new(t * se, -t * ce);
            pen.curve_to(pt.map(p1).point(), pt.map(p2).point(), pt.map(target).point());
        }
    }
}

const FLOAT_PATTERN: &str =
    r"[-+]?(?:(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][-+]?[0-9]+)?|(?:\.[0-9]+(?:[eE][-+]?[0-9]+)?))";

static FLOAT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(FLOAT_PATTERN).expect("valid regex"));
static BOOL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new("^[01]").expect("valid regex"));

const COMMANDS: &str = "MmZzLlHhVvCcSsQqTtAa";

fn split_commands(d: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, ch) in d.char_indices() {
        if COMMANDS.contains(ch) {
            out.push(&d[start..i]);
            out.push(&d[i..i + 1]);
            start = i + 1;
        }
    }
    out.push(&d[start..]);
    out
}

fn tokenize_arc_arguments(arcdef: &str, out: &mut Vec<String>) -> Result<(), PathError> {
    let mut raw: Vec<String> = arcdef.split([',', ' ', '\t']).filter(|s| !s.is_empty()).map(String::from).collect();
    if raw.is_empty() {
        return Err(PathError(format!("Not enough arguments: '{arcdef}'")));
    }
    raw.reverse();
    let mut i = 0;
    while let Some(arg) = raw.pop() {
        let m = if i == 3 || i == 4 { BOOL_RE.find(&arg) } else { FLOAT_RE.find(&arg) };
        let Some(m) = m else {
            return Err(PathError(format!("Invalid argument: {arg:?}")));
        };
        out.push(m.as_str().to_string());
        let rest = &arg[m.end()..];
        if !rest.is_empty() {
            raw.push(rest.to_string());
        }
        i = if i == 6 { 0 } else { i + 1 };
    }
    if i != 0 {
        return Err(PathError(format!("Not enough arguments: '{arcdef}'")));
    }
    Ok(())
}

fn tokenize(d: &str) -> Result<Vec<String>, PathError> {
    let mut out = Vec::new();
    let mut arc_cmd: Option<&str> = None;
    for x in split_commands(d) {
        if x.len() == 1 && COMMANDS.contains(x) {
            arc_cmd = if x == "A" || x == "a" { Some(x) } else { None };
            out.push(x.to_string());
            continue;
        }
        if let Some(cmd) = arc_cmd {
            tokenize_arc_arguments(x, &mut out).map_err(|_| PathError(format!("Invalid arc command: '{cmd}{x}'")))?;
        } else {
            out.extend(FLOAT_RE.find_iter(x).map(|m| m.as_str().to_string()));
        }
    }
    Ok(out)
}

fn is_command(t: &str) -> bool {
    t.len() == 1 && COMMANDS.contains(t)
}

struct Tokens {
    items: Vec<String>,
}

impl Tokens {
    fn pop(&mut self) -> Result<String, PathError> {
        self.items.pop().ok_or_else(|| PathError("pop from empty list".into()))
    }

    fn num(&mut self) -> Result<f64, PathError> {
        let t = self.pop()?;
        t.parse::<f64>().map_err(|_| PathError(format!("could not convert string to float: {t:?}")))
    }

    fn flag(&mut self) -> Result<bool, PathError> {
        let t = self.pop()?;
        t.parse::<i64>().map(|v| v != 0).map_err(|_| PathError(format!("invalid literal for int(): {t:?}")))
    }

    fn point(&mut self) -> Result<C, PathError> {
        let x = self.num()?;
        let y = self.num()?;
        Ok(C::from_xy(x, y))
    }
}

pub fn parse_path(d: &str, pen: &mut dyn Pen) -> Result<(), PathError> {
    let mut items = tokenize(d)?;
    items.reverse();
    let mut el = Tokens { items };
    if let Some(first) = el.items.last()
        && first != "M"
        && first != "m"
    {
        return Err(PathError(format!("Path must start with a moveto command: {d:?}")));
    }

    let mut current = C::new(0.0, 0.0);
    let mut start: Option<C> = None;
    let mut open = false;
    let mut command: Option<char> = None;
    let mut last_command: Option<char>;
    let mut last_control = C::new(0.0, 0.0);
    let mut absolute = true;

    while let Some(top) = el.items.last() {
        if is_command(top) {
            last_command = command;
            let c = el.pop()?.chars().next().expect("one-char command");
            absolute = c.is_ascii_uppercase();
            command = Some(c.to_ascii_uppercase());
        } else {
            if command.is_none() {
                return Err(PathError(format!("Unallowed implicit command in {d}")));
            }
            last_command = command;
        }
        let cmd = command.expect("command is set");

        if cmd != 'M'
            && cmd != 'Z'
            && !open
            && let Some(s) = start
        {
            pen.move_to(s.point());
            open = true;
        }

        match cmd {
            'M' => {
                let pos = el.point()?;
                current = if absolute { pos } else { current + pos };
                if open {
                    pen.end_path();
                }
                pen.move_to(current.point());
                start = Some(current);
                open = true;
                command = Some('L');
            }
            'Z' => {
                if open {
                    let s = start.expect("open subpath has a start");
                    if !current.is_close(s, 1e-15, 1e-15) {
                        pen.line_to(s.point());
                    }
                    pen.close_path();
                    current = s;
                    open = false;
                }
                command = None;
            }
            'L' => {
                let mut pos = el.point()?;
                if !absolute {
                    pos = pos + current;
                }
                pen.line_to(pos.point());
                current = pos;
            }
            'H' => {
                let x = el.num()?;
                let mut pos = C::imag_unit(current.im).add_re(x);
                if !absolute {
                    pos = pos.add_re(current.re);
                }
                pen.line_to(pos.point());
                current = pos;
            }
            'V' => {
                let y = el.num()?;
                let mut pos = C::imag_unit(y).add_re(current.re);
                if !absolute {
                    pos = pos + C::imag_unit(current.im);
                }
                pen.line_to(pos.point());
                current = pos;
            }
            'C' => {
                let mut c1 = el.point()?;
                let mut c2 = el.point()?;
                let mut end = el.point()?;
                if !absolute {
                    c1 = c1 + current;
                    c2 = c2 + current;
                    end = end + current;
                }
                pen.curve_to(c1.point(), c2.point(), end.point());
                current = end;
                last_control = c2;
            }
            'S' => {
                let c1 =
                    if matches!(last_command, Some('C' | 'S')) { current + current - last_control } else { current };
                let mut c2 = el.point()?;
                let mut end = el.point()?;
                if !absolute {
                    c2 = c2 + current;
                    end = end + current;
                }
                pen.curve_to(c1.point(), c2.point(), end.point());
                current = end;
                last_control = c2;
            }
            'Q' => {
                let mut c = el.point()?;
                let mut end = el.point()?;
                if !absolute {
                    c = c + current;
                    end = end + current;
                }
                pen.qcurve_to(c.point(), end.point());
                current = end;
                last_control = c;
            }
            'T' => {
                let c =
                    if matches!(last_command, Some('Q' | 'T')) { current + current - last_control } else { current };
                let mut end = el.point()?;
                if !absolute {
                    end = end + current;
                }
                pen.qcurve_to(c.point(), end.point());
                current = end;
                last_control = c;
            }
            'A' => {
                let rx = el.num()?.abs();
                let ry = el.num()?.abs();
                let rotation = el.num()?;
                let large = el.flag()?;
                let sweep = el.flag()?;
                let mut end = el.point()?;
                if !absolute {
                    end = end + current;
                }
                Arc { current, rx, ry, angle: rotation.to_radians(), large, sweep, target: end }.draw(pen);
                current = end;
            }
            _ => unreachable!("only path commands are tokenized"),
        }
    }
    if open {
        pen.end_path();
    }
    Ok(())
}
