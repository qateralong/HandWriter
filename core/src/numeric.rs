pub fn sum<I: IntoIterator<Item = f64>>(values: I) -> f64 {
    let mut total = 0.0_f64;
    let mut c = 0.0_f64;
    for x in values {
        let t = total + x;
        if total.abs() >= x.abs() {
            c += (total - t) + x;
        } else {
            c += (x - t) + total;
        }
        total = t;
    }
    if c != 0.0 && c.is_finite() {
        total += c;
    }
    total
}

struct DoubleLength {
    hi: f64,
    lo: f64,
}

fn dl_mul(x: f64, y: f64) -> DoubleLength {
    let hi = x * y;
    DoubleLength { hi, lo: x.mul_add(y, -hi) }
}

fn dl_fast_sum(a: f64, b: f64) -> DoubleLength {
    let x = a + b;
    let z = x - a;
    DoubleLength { hi: x, lo: b - z }
}

fn vector_norm(vec: &mut [f64], max: f64, found_nan: bool) -> f64 {
    if max.is_infinite() {
        return max;
    }
    if found_nan {
        return f64::NAN;
    }
    if max == 0.0 || vec.len() <= 1 {
        return max;
    }
    let max_e = frexp_exp(max);
    if max_e < -1023 {
        for x in vec.iter_mut() {
            *x /= f64::MIN_POSITIVE;
        }
        return f64::MIN_POSITIVE * vector_norm(vec, max / f64::MIN_POSITIVE, found_nan);
    }
    let scale = ldexp1(-max_e);
    let (mut csum, mut frac1, mut frac2) = (1.0_f64, 0.0_f64, 0.0_f64);
    for &v in vec.iter() {
        let x = v * scale;
        let pr = dl_mul(x, x);
        let sm = dl_fast_sum(csum, pr.hi);
        csum = sm.hi;
        frac1 += pr.lo;
        frac2 += sm.lo;
    }
    let mut h = (csum - 1.0 + (frac1 + frac2)).sqrt();
    let pr = dl_mul(-h, h);
    let sm = dl_fast_sum(csum, pr.hi);
    csum = sm.hi;
    frac1 += pr.lo;
    frac2 += sm.lo;
    let x = csum - 1.0 + (frac1 + frac2);
    h += x / (2.0 * h);
    h / scale
}

fn frexp_exp(v: f64) -> i32 {
    let bits = v.abs().to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        let lz = (bits << 12).leading_zeros() as i32;
        -1022 - lz
    } else {
        exp - 1022
    }
}

fn ldexp1(e: i32) -> f64 {
    f64::from_bits(((e + 1023) as u64) << 52)
}

pub fn dist(p: (f64, f64), q: (f64, f64)) -> f64 {
    let mut d = [(p.0 - q.0).abs(), (p.1 - q.1).abs()];
    let found_nan = d[0].is_nan() || d[1].is_nan();
    let mut max = 0.0_f64;
    for &x in &d {
        if x > max {
            max = x;
        }
    }
    vector_norm(&mut d, max, found_nan)
}

pub fn hypot(x: f64, y: f64) -> f64 {
    dist((x, y), (0.0, 0.0))
}

pub fn min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

pub fn max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

pub fn repr(v: f64) -> String {
    if v.is_nan() {
        return "nan".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let a = v.abs();
    if a != 0.0 && !(1e-4..1e16).contains(&a) {
        let s = format!("{v:e}");
        let (mant, exp) = s.split_once('e').expect("exponent form");
        let exp: i32 = exp.parse().expect("integer exponent");
        let sign = if exp < 0 { '-' } else { '+' };
        return format!("{mant}e{sign}{:02}", exp.abs());
    }
    format!("{v:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_matches_python() {
        assert_eq!(repr(1.0), "1.0");
        assert_eq!(repr(-3.5), "-3.5");
        assert_eq!(repr(0.15), "0.15");
        assert_eq!(repr(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(repr(1e16), "1e+16");
        assert_eq!(repr(1e15), "1000000000000000.0");
        assert_eq!(repr(1e-5), "1e-05");
        assert_eq!(repr(0.0001), "0.0001");
        assert_eq!(repr(-0.0), "-0.0");
        assert_eq!(repr(1.5e300), "1.5e+300");
    }

    #[test]
    fn sum_is_compensated() {
        assert_eq!(sum(std::iter::repeat_n(0.1, 10)), 1.0);
        assert_eq!(sum([1e100, 1.0, -1e100, 1.0]), 2.0);
        assert_eq!(sum([]), 0.0);
    }

    #[test]
    fn dist_is_exact_on_simple_cases() {
        assert_eq!(dist((0.0, 0.0), (3.0, 4.0)), 5.0);
        assert_eq!(dist((1.0, 1.0), (1.0, 1.0)), 0.0);
        assert_eq!(dist((0.0, 0.0), (0.0, -2.5)), 2.5);
    }
}
