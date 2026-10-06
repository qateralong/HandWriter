use blake2::Blake2bVar;
use blake2::digest::{Update, VariableOutput};

fn hash01(text: &str) -> f64 {
    let mut h = Blake2bVar::new(8).expect("valid output size");
    h.update(text.as_bytes());
    let mut out = [0u8; 8];
    h.finalize_variable(&mut out).expect("output buffer size");
    u64::from_le_bytes(out) as f64 / 2.0_f64.powi(64)
}

fn key_text(seed: i64, channel: &str, keys: &[i64]) -> String {
    let mut s = format!("{seed}|{channel}|");
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            s.push('|');
        }
        s.push_str(&k.to_string());
    }
    s
}

pub fn rnd(seed: i64, channel: &str, keys: &[i64]) -> f64 {
    hash01(&key_text(seed, channel, keys))
}

pub fn urnd(seed: i64, channel: &str, keys: &[i64]) -> f64 {
    2.0 * rnd(seed, channel, keys) - 1.0
}

pub fn pick(seed: i64, channel: &str, n: i64, keys: &[i64]) -> i64 {
    ((rnd(seed, channel, keys) * n as f64) as i64).min(n - 1)
}

pub fn vnoise(seed: i64, channel: &str, t: f64, keys: &[i64]) -> f64 {
    let i = t.floor() as i64;
    let f = t - i as f64;
    let mut ka = keys.to_vec();
    ka.push(i);
    let a = urnd(seed, channel, &ka);
    *ka.last_mut().expect("key pushed") = i + 1;
    let b = urnd(seed, channel, &ka);
    let s = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    a + (b - a) * s
}
