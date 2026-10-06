use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::paths::log_path;

const MAX_BYTES: u64 = 1_000_000;
const BACKUPS: usize = 3;

static FILE: Mutex<Option<File>> = Mutex::new(None);
static CONSOLE: Mutex<bool> = Mutex::new(false);

pub fn setup(console: bool) {
    *CONSOLE.lock().expect("log lock") = console;
    *FILE.lock().expect("log lock") = OpenOptions::new().create(true).append(true).open(log_path()).ok();
}

fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86400, secs % 86400);
    let (y, m, d) = civil(days as i64);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn rotate() {
    let path = log_path();
    if fs::metadata(&path).map(|m| m.len()).unwrap_or(0) < MAX_BYTES {
        return;
    }
    for i in (1..BACKUPS).rev() {
        let _ = fs::rename(path.with_extension(format!("log.{i}")), path.with_extension(format!("log.{}", i + 1)));
    }
    let _ = fs::rename(&path, path.with_extension("log.1"));
    *FILE.lock().expect("log lock") = OpenOptions::new().create(true).append(true).open(&path).ok();
}

pub fn write(level: &str, name: &str, msg: &str) {
    let line = format!("{} {level} {name}: {msg}\n", timestamp());
    rotate();
    if let Some(f) = FILE.lock().expect("log lock").as_mut() {
        let _ = f.write_all(line.as_bytes());
    }
    if *CONSOLE.lock().expect("log lock") && level != "INFO" {
        eprint!("{line}");
    }
}

pub fn info(name: &str, msg: &str) {
    write("INFO", name, msg);
}

pub fn warn(name: &str, msg: &str) {
    write("WARNING", name, msg);
}

pub fn error(name: &str, msg: &str) {
    write("ERROR", name, msg);
}
