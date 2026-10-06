use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use handwriter_core::drawing::pipeline::MachineMap;
use handwriter_core::settings::Settings;
use serialport::{SerialPort, TTYPort};

use super::{Job, Pen, Printer};

#[derive(Default)]
struct Fake {
    accepted: Vec<String>,
    resends: usize,
    halted: bool,
}

fn checksum(s: &str) -> u8 {
    s.bytes().fold(0, |a, b| a ^ b)
}

fn run_fake(mut port: TTYPort, fake: Arc<Mutex<Fake>>, corrupt_every: usize, delay: Duration) {
    port.set_timeout(Duration::from_millis(20)).unwrap();
    port.write_all(b"start\necho: External Reset\n").unwrap();
    let mut buf = Vec::new();
    let mut expected = 0u64;
    let mut count = 0usize;
    let mut tmp = [0u8; 512];
    loop {
        match port.read(&mut tmp) {
            Ok(0) => return,
            Ok(k) => buf.extend_from_slice(&tmp[..k]),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(_) => return,
        }
        while let Some(i) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=i).collect();
            let line = String::from_utf8_lossy(&line).trim().to_string();
            let mut out = String::new();
            if line == "M112" {
                fake.lock().unwrap().halted = true;
                out += "Error:Printer halted. kill() called!\n";
            } else if let Some((body, cs)) = line.rsplit_once('*') {
                let (n, cmd) = body[1..].split_once(' ').unwrap();
                let n: u64 = n.parse().unwrap();
                count += 1;
                let bad = corrupt_every > 0 && count.is_multiple_of(corrupt_every);
                if bad || checksum(body).to_string() != cs {
                    fake.lock().unwrap().resends += 1;
                    out += &format!(
                        "Error:checksum mismatch, Last Line: {}\nResend: {expected}\nok\n",
                        expected.wrapping_sub(1)
                    );
                } else if cmd.starts_with("M110") {
                    expected = n + 1;
                    out += "ok\n";
                } else if n != expected {
                    out += &format!("Error:Line Number is not Last Line Number+1\nResend: {expected}\nok\n");
                } else {
                    expected = n + 1;
                    fake.lock().unwrap().accepted.push(cmd.to_string());
                    if cmd == "M114" {
                        out += "X:0.00 Y:0.00 Z:0.00 E:0.00 Count X:0 Y:0 Z:0\n";
                    }
                    if cmd.starts_with("G1") {
                        std::thread::sleep(delay);
                    }
                    out += "ok\n";
                }
            }
            if port.write_all(out.as_bytes()).is_err() {
                return;
            }
        }
    }
}

fn setup(corrupt_every: usize, delay: Duration) -> (Printer, Arc<Mutex<Fake>>) {
    let (master, slave) = TTYPort::pair().expect("pty pair");
    let fake = Arc::new(Mutex::new(Fake::default()));
    let f = fake.clone();
    std::thread::spawn(move || run_fake(master, f, corrupt_every, delay));
    let printer = Printer::new();
    printer.attach(Box::new(slave), "pty", 115200).unwrap();
    wait(&printer, "idle", 10);
    (printer, fake)
}

fn status(p: &Printer) -> String {
    p.status(0, 0, u64::MAX)["status"].as_str().unwrap().to_string()
}

fn wait(p: &Printer, want: &str, secs: u64) {
    let t = Instant::now();
    while status(p) != want {
        assert!(t.elapsed() < Duration::from_secs(secs), "status {} instead of {want}", status(p));
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn job(lines: Vec<String>) -> Job {
    let s = Settings::default();
    Job {
        label: "test".into(),
        kind: "text".into(),
        part: 0,
        test: false,
        lines,
        map: MachineMap::for_text(&s),
        pen: Pen { up: 4.0, down: -1.0, end: 14.0, feed_z: 600.0, feed_travel: 3000.0 },
    }
}

fn square_lines(n: usize) -> Vec<String> {
    let mut v = vec!["G90".to_string(), "G92 X0 Y0 Z0".into(), "G0 Z4.00 F600".into()];
    for i in 0..n {
        let x = (i % 50) as f64;
        v.push(format!("G0 X{x:.2} Y{:.2} F3000", i as f64 * 0.1));
        v.push("G1 Z-1.00 F600".into());
        v.push(format!("G1 X{:.2} Y{:.2} F1200", x + 1.0, i as f64 * 0.1));
        v.push("G0 Z4.00 F600".into());
    }
    v.push("G0 Z14.00 F600".into());
    v.push("M400".into());
    v
}

fn without_service(v: &[String]) -> Vec<String> {
    v.iter().filter(|c| !matches!(c.as_str(), "M114" | "M115")).cloned().collect()
}

#[test]
fn delivers_job_exactly_with_resends() {
    let (p, fake) = setup(7, Duration::ZERO);
    let lines = square_lines(300);
    p.start(job(lines.clone())).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    wait(&p, "idle", 60);
    let f = fake.lock().unwrap();
    assert!(f.resends > 100, "resends {} accepted {} {}", f.resends, f.accepted.len(), p.status(0, 0, 0));
    assert_eq!(without_service(&f.accepted), lines);
    let st = p.status(0, 0, 0);
    assert_eq!(st["finished"]["kind"], "text");
    assert_eq!(st["segs"]["items"].as_array().unwrap().len(), 300);
}

#[test]
fn pause_lifts_and_returns() {
    let (p, fake) = setup(0, Duration::from_millis(5));
    let lines = square_lines(400);
    p.start(job(lines.clone())).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    p.pause().unwrap();
    wait(&p, "paused", 5);
    std::thread::sleep(Duration::from_millis(300));
    let before = fake.lock().unwrap().accepted.len();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(fake.lock().unwrap().accepted.len(), before);
    p.command(vec!["G91".into(), "G0 X5".into(), "G90".into()]).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    p.resume().unwrap();
    wait(&p, "idle", 60);
    let acc = without_service(&fake.lock().unwrap().accepted);
    let i = acc.iter().position(|c| c == "G91").expect("manual move");
    assert_eq!(acc[i - 1], "G0 Z4.00 F600");
    assert_eq!(acc[i + 3], "G90");
    assert!(acc[i + 4].starts_with("G0 X"));
    assert!(acc[i + 5].starts_with("G1 Z"));
    let mut rest: Vec<String> = acc[..i - 1].to_vec();
    rest.extend_from_slice(&acc[i + 6..]);
    let lifted = acc[i - 1] == "G0 Z4.00 F600" && acc[i - 2] == "G1 Z-1.00 F600";
    if lifted {
        assert_eq!(rest, lines);
    } else {
        let mut with_lift = lines.clone();
        let k = acc[..i - 1].len();
        with_lift.insert(k, "G0 Z4.00 F600".into());
        assert_eq!(acc[..i].to_vec(), with_lift[..i].to_vec());
    }
}

#[test]
fn stop_lifts_and_emergency_halts() {
    let (p, fake) = setup(0, Duration::from_millis(5));
    p.start(job(square_lines(400))).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(p.disconnect().is_err());
    p.stop().unwrap();
    wait(&p, "idle", 5);
    std::thread::sleep(Duration::from_millis(300));
    let acc = without_service(&fake.lock().unwrap().accepted);
    let n = acc.len();
    assert_eq!(&acc[n - 3..], ["M410", "G90", "G0 Z14.00 F600"]);
    p.emergency().unwrap();
    wait(&p, "halted", 5);
    std::thread::sleep(Duration::from_millis(100));
    assert!(fake.lock().unwrap().halted);
    assert!(p.command(vec!["G0 X1".into()]).is_err());
}
