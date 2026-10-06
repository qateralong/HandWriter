pub mod protocol;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use handwriter_core::drawing::pipeline::MachineMap;
use handwriter_core::gcode::fmt;
use handwriter_core::i18n::tr;
use serde_json::{Value, json};

use crate::logs;
use protocol::{Reply, Tracker, numbered, parse_reply};

pub static PRINTER: LazyLock<Printer> = LazyLock::new(Printer::new);

const LOG_KEEP: usize = 400;
const HISTORY_KEEP: usize = 2000;
const POLL_EVERY: Duration = Duration::from_secs(2);
const SILENCE_WARN: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    Disconnected,
    Connecting,
    Idle,
    Printing,
    Paused,
    Halted,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Disconnected => "disconnected",
            Status::Connecting => "connecting",
            Status::Idle => "idle",
            Status::Printing => "printing",
            Status::Paused => "paused",
            Status::Halted => "halted",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Pen {
    pub up: f64,
    pub down: f64,
    pub end: f64,
    pub feed_z: f64,
    pub feed_travel: f64,
}

impl Pen {
    fn threshold(&self) -> f64 {
        (self.up + self.down) / 2.0
    }
}

pub struct Job {
    pub label: String,
    pub kind: String,
    pub part: usize,
    pub test: bool,
    pub lines: Vec<String>,
    pub map: MachineMap,
    pub pen: Pen,
}

struct JobInfo {
    label: String,
    kind: String,
    part: usize,
    total: usize,
    done: usize,
    started: Instant,
}

pub struct Shared {
    status: Status,
    port: String,
    baud: u32,
    message: String,
    firmware: String,
    tracker: Tracker,
    reported: Option<(f64, f64, f64)>,
    job: Option<JobInfo>,
    last_map: Option<MachineMap>,
    last_pen: Option<Pen>,
    finished: Option<(String, usize)>,
    segs_gen: u64,
    segs: Vec<[f64; 4]>,
    segs_owner: Option<(String, usize, bool)>,
    log: VecDeque<(u64, String)>,
    log_seq: u64,
}

impl Shared {
    fn new() -> Self {
        Shared {
            status: Status::Disconnected,
            port: String::new(),
            baud: 115200,
            message: String::new(),
            firmware: String::new(),
            tracker: Tracker::default(),
            reported: None,
            job: None,
            last_map: None,
            last_pen: None,
            finished: None,
            segs_gen: 0,
            segs: Vec::new(),
            segs_owner: None,
            log: VecDeque::new(),
            log_seq: 0,
        }
    }

    fn log(&mut self, text: String) {
        self.log_seq += 1;
        self.log.push_back((self.log_seq, text));
        while self.log.len() > LOG_KEEP {
            self.log.pop_front();
        }
    }
}

enum Msg {
    Send(Vec<String>),
    Start(Box<Job>),
    Pause,
    Resume,
    Stop,
    Emergency,
    Disconnect,
}

pub struct Printer {
    shared: Arc<Mutex<Shared>>,
    tx: Mutex<Option<Sender<Msg>>>,
}

impl Default for Printer {
    fn default() -> Self {
        Self::new()
    }
}

pub fn list_ports() -> Vec<Value> {
    let mut out = Vec::new();
    for p in serialport::available_ports().unwrap_or_default() {
        let (label, likely) = match &p.port_type {
            serialport::SerialPortType::UsbPort(u) => {
                let name = u.product.clone().or_else(|| u.manufacturer.clone()).unwrap_or_else(|| "USB".into());
                let likely = matches!((u.vid, u.pid), (0x1a86, 0x7523) | (0x1a86, 0x55d4) | (0x0483, 0x5740));
                (format!("{} — {} ({:04x}:{:04x})", p.port_name, name, u.vid, u.pid), likely)
            }
            _ if cfg!(target_os = "linux") && p.port_name.starts_with("/dev/ttyS") => continue,
            _ => (p.port_name.clone(), false),
        };
        out.push(json!({"path": p.port_name, "label": label, "likely": likely}));
    }
    out.sort_by_key(|v| !v["likely"].as_bool().unwrap_or(false));
    out
}

fn open_error(path: &str, e: &serialport::Error) -> String {
    let text = e.to_string();
    if text.to_lowercase().contains("permission") || text.contains("EACCES") {
        if cfg!(target_os = "linux") {
            return format!(
                "Нет доступа к порту {path}. Добавь себя в группу uucp (sudo usermod -aG uucp $USER) и перезайди в систему."
            );
        }
        return format!("Нет доступа к порту {path}: {text}");
    }
    let lower = text.to_lowercase();
    if lower.contains("not found") || lower.contains("no such file") || lower.contains("cannot find") {
        if cfg!(windows) {
            return format!("Порт {path} не найден. Проверь кабель и драйвер CH340.");
        }
        return format!("Порт {path} не найден. Проверь кабель.");
    }
    format!("Порт {path} не открылся: {text}")
}

impl Printer {
    pub fn new() -> Self {
        Printer { shared: Arc::new(Mutex::new(Shared::new())), tx: Mutex::new(None) }
    }

    fn send_msg(&self, m: Msg) -> Result<(), String> {
        match self.tx.lock().expect("printer lock").as_ref() {
            Some(tx) => tx.send(m).map_err(|_| "Принтер не подключён".to_string()),
            None => Err("Принтер не подключён".into()),
        }
    }

    pub fn connect(&self, path: &str, baud: u32) -> Result<(), String> {
        {
            let st = self.shared.lock().expect("printer lock").status;
            if !matches!(st, Status::Disconnected | Status::Halted) {
                return Err("Принтер уже подключён".into());
            }
        }
        if let Some(tx) = self.tx.lock().expect("printer lock").take() {
            let _ = tx.send(Msg::Disconnect);
        }
        let port =
            serialport::new(path, baud).timeout(Duration::from_millis(20)).open().map_err(|e| open_error(path, &e))?;
        self.attach(port, path, baud)
    }

    fn attach(&self, port: Box<dyn serialport::SerialPort>, path: &str, baud: u32) -> Result<(), String> {
        let (tx, rx) = channel();
        {
            let mut sh = self.shared.lock().expect("printer lock");
            *sh = Shared { log: std::mem::take(&mut sh.log), log_seq: sh.log_seq, ..Shared::new() };
            sh.status = Status::Connecting;
            sh.port = path.to_string();
            sh.baud = baud;
            sh.message = "Подключаюсь…".into();
            sh.log(format!("— подключение к {path}, {baud} бод"));
        }
        logs::info("handwriter.printer", &format!("Подключение к {path}, {baud}"));
        let shared = self.shared.clone();
        std::thread::Builder::new()
            .name("printer".into())
            .spawn(move || Worker::new(port, rx, shared).run())
            .map_err(|e| e.to_string())?;
        *self.tx.lock().expect("printer lock") = Some(tx);
        Ok(())
    }

    pub fn disconnect(&self) -> Result<(), String> {
        if matches!(self.shared.lock().expect("printer lock").status, Status::Printing | Status::Paused) {
            return Err("Идёт печать: сначала стоп".into());
        }
        let r = self.send_msg(Msg::Disconnect);
        self.tx.lock().expect("printer lock").take();
        r
    }

    pub fn command(&self, lines: Vec<String>) -> Result<(), String> {
        let st = self.shared.lock().expect("printer lock").status;
        if st == Status::Printing {
            return Err("Идёт печать: сначала пауза или стоп".into());
        }
        if !matches!(st, Status::Idle | Status::Paused | Status::Connecting) {
            return Err("Принтер не готов".into());
        }
        self.send_msg(Msg::Send(lines))
    }

    pub fn start(&self, job: Job) -> Result<(), String> {
        let mut sh = self.shared.lock().expect("printer lock");
        if sh.status != Status::Idle {
            return Err(match sh.status {
                Status::Printing | Status::Paused => "Уже идёт печать".into(),
                _ => "Принтер не подключён или не готов".into(),
            });
        }
        self.send_msg(Msg::Start(Box::new(job)))?;
        sh.status = Status::Printing;
        Ok(())
    }

    pub fn pause(&self) -> Result<(), String> {
        self.send_msg(Msg::Pause)
    }

    pub fn resume(&self) -> Result<(), String> {
        self.send_msg(Msg::Resume)
    }

    pub fn stop(&self) -> Result<(), String> {
        self.send_msg(Msg::Stop)
    }

    pub fn emergency(&self) -> Result<(), String> {
        self.send_msg(Msg::Emergency)
    }

    pub fn status(&self, segs_gen: u64, segs_from: usize, log_from: u64) -> Value {
        let sh = self.shared.lock().expect("printer lock");
        let t = sh.tracker;
        let pen_down = sh.last_pen.map(|p| t.z <= p.threshold());
        let map = sh.last_map.as_ref();
        let sheet = map.map(|m| {
            let p = m.to_sheet((t.x, t.y));
            json!([p.0, p.1])
        });
        let (from, reset) = if segs_gen == sh.segs_gen { (segs_from.min(sh.segs.len()), false) } else { (0, true) };
        let job = sh.job.as_ref().map(|j| {
            let elapsed = j.started.elapsed().as_secs_f64();
            let frac = if j.total > 0 { j.done as f64 / j.total as f64 } else { 0.0 };
            let remaining = (frac > 0.02).then(|| elapsed * (1.0 - frac) / frac);
            json!({"label": j.label, "kind": j.kind, "part": j.part, "total": j.total, "done": j.done,
                   "percent": (frac * 1000.0).round() / 10.0, "elapsed_s": elapsed.round(), "remaining_s": remaining.map(f64::round)})
        });
        json!({
            "status": sh.status.as_str(),
            "connected": !matches!(sh.status, Status::Disconnected),
            "port": sh.port, "baud": sh.baud, "message": sh.message, "firmware": sh.firmware,
            "alert": sh.status == Status::Halted || sh.message.starts_with("Связь") || sh.message.starts_with("Принтер перезагрузился"),
            "position": {"x": t.x, "y": t.y, "z": t.z},
            "reported": sh.reported.map(|(x, y, z)| json!({"x": x, "y": y, "z": z})),
            "pen_down": pen_down,
            "sheet_position": sheet,
            "job": job,
            "finished": sh.finished.as_ref().map(|(k, p)| json!({"kind": k, "part": p})),
            "segs": {"gen": sh.segs_gen, "reset": reset, "from": from, "items": &sh.segs[from..],
                     "kind": sh.segs_owner.as_ref().map(|o| o.0.clone()), "part": sh.segs_owner.as_ref().map(|o| o.1),
                     "test": sh.segs_owner.as_ref().map(|o| o.2)},
            "log": sh.log.iter().filter(|(n, _)| *n > log_from).map(|(n, t)| {
                let t = if t.starts_with('—') { tr(t) } else { t.clone() };
                json!([n, t])
            }).collect::<Vec<_>>(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Src {
    Job,
    Manual,
    Poll,
    Service,
}

enum Prio {
    Line(String),
    LiftIfDown,
    Return,
}

struct RunJob {
    lines: Vec<String>,
    next: usize,
    paused: bool,
    pen: Pen,
    saved: Option<(f64, f64, f64)>,
}

struct Awake(Option<Sender<bool>>);

impl Awake {
    fn new() -> Self {
        let (tx, rx) = channel::<bool>();
        let spawned = std::thread::Builder::new().name("printer-awake".into()).spawn(move || {
            let mut guard = None;
            while let Ok(on) = rx.recv() {
                if !on {
                    guard = None;
                } else if guard.is_none() {
                    guard = keepawake::Builder::default()
                        .display(false)
                        .idle(true)
                        .sleep(true)
                        .reason("Печать по кабелю")
                        .app_name("HandWriter")
                        .app_reverse_domain("local.handwriter.app")
                        .create()
                        .ok();
                }
            }
            drop(guard);
        });
        Awake(spawned.ok().map(|_| tx))
    }

    fn set(&self, on: bool) {
        if let Some(tx) = &self.0 {
            let _ = tx.send(on);
        }
    }
}

struct Worker {
    port: Box<dyn serialport::SerialPort>,
    rx: Receiver<Msg>,
    sh: Arc<Mutex<Shared>>,
    buf: Vec<u8>,
    n: u64,
    history: VecDeque<(u64, String, Src)>,
    resend: VecDeque<(u64, String, Src)>,
    resend_pending: bool,
    waiting: Option<(u64, String, Src, Instant)>,
    warned: bool,
    manual: VecDeque<String>,
    prio: VecDeque<Prio>,
    job: Option<RunJob>,
    awake: Awake,
    last_poll: Instant,
    ready: bool,
}

impl Worker {
    fn new(port: Box<dyn serialport::SerialPort>, rx: Receiver<Msg>, sh: Arc<Mutex<Shared>>) -> Self {
        Worker {
            port,
            rx,
            sh,
            buf: Vec::new(),
            n: 0,
            history: VecDeque::new(),
            resend: VecDeque::new(),
            resend_pending: false,
            waiting: None,
            warned: false,
            manual: VecDeque::new(),
            prio: VecDeque::new(),
            job: None,
            awake: Awake::new(),
            last_poll: Instant::now(),
            ready: false,
        }
    }

    fn sh(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.sh.lock().expect("printer lock")
    }

    fn set(&self, status: Status, message: &str) {
        let mut sh = self.sh();
        sh.status = status;
        sh.message = message.to_string();
    }

    fn read_lines(&mut self) -> Result<Vec<String>, String> {
        let mut tmp = [0u8; 1024];
        match self.port.read(&mut tmp) {
            Ok(0) => {}
            Ok(k) => self.buf.extend_from_slice(&tmp[..k]),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.to_string()),
        }
        let mut out = Vec::new();
        while let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            let text = String::from_utf8_lossy(&line).trim().to_string();
            if !text.is_empty() {
                out.push(text);
            }
        }
        Ok(out)
    }

    fn write_raw(&mut self, text: &str) -> Result<(), String> {
        self.port.write_all(format!("{text}\n").as_bytes()).map_err(|e| e.to_string())?;
        self.port.flush().map_err(|e| e.to_string())
    }

    fn send(&mut self, n: u64, cmd: String, src: Src) -> Result<(), String> {
        let line = numbered(n, &cmd);
        self.write_raw(&line)?;
        if src != Src::Poll {
            self.sh().log(format!("> {cmd}"));
        }
        self.history.push_back((n, cmd.clone(), src));
        while self.history.len() > HISTORY_KEEP {
            self.history.pop_front();
        }
        self.waiting = Some((n, cmd, src, Instant::now()));
        self.warned = false;
        Ok(())
    }

    fn send_new(&mut self, cmd: String, src: Src) -> Result<(), String> {
        let n = self.n;
        self.n += 1;
        self.send(n, cmd, src)
    }

    fn startup(&mut self) -> Result<(), String> {
        let begin = Instant::now();
        let mut last_seen: Option<Instant> = None;
        while begin.elapsed() < Duration::from_secs(5) {
            let lines = self.read_lines()?;
            let mut started = false;
            for l in lines {
                last_seen = Some(Instant::now());
                if parse_reply(&l) == Reply::Start {
                    started = true;
                }
                if l.starts_with("FIRMWARE_NAME") || l.contains("Marlin") {
                    self.sh().firmware = l.clone();
                }
                self.sh().log(format!("< {l}"));
            }
            if started || last_seen.is_some_and(|t| t.elapsed() > Duration::from_millis(1500)) {
                break;
            }
        }
        self.n = 0;
        self.send_new("M110 N0".into(), Src::Service)
    }

    fn pen_threshold(&self) -> f64 {
        self.job
            .as_ref()
            .map(|j| j.pen.threshold())
            .or_else(|| self.sh().last_pen.map(|p| p.threshold()))
            .unwrap_or(1.5)
    }

    fn on_ok(&mut self) {
        let Some((_, cmd, src, _)) = self.waiting.take() else { return };
        if self.resend_pending {
            self.resend_pending = false;
            return;
        }
        if !self.ready {
            self.ready = true;
            self.set(Status::Idle, "Подключено");
            self.prio.push_back(Prio::Line("M115".into()));
        }
        let thresh = self.pen_threshold();
        let mut sh = self.sh.lock().expect("printer lock");
        let z_before = sh.tracker.z;
        let seg = sh.tracker.apply(&cmd);
        if src == Src::Job {
            if let Some(j) = sh.job.as_mut() {
                j.done += 1;
            }
            if let (Some((a, b)), Some(map)) = (seg, sh.last_map.clone())
                && z_before <= thresh
            {
                let (p, q) = (map.to_sheet(a), map.to_sheet(b));
                sh.segs.push([p.0, p.1, q.0, q.1]);
            }
        }
    }

    fn handle(&mut self, line: &str) {
        let reply = parse_reply(line);
        let quiet =
            matches!(reply, Reply::Ok | Reply::Position(..)) && matches!(self.waiting, Some((_, _, Src::Poll, _)));
        if !quiet {
            self.sh().log(format!("< {line}"));
        }
        match reply {
            Reply::Ok => self.on_ok(),
            Reply::Busy => {
                if let Some(w) = self.waiting.as_mut() {
                    w.3 = Instant::now();
                }
            }
            Reply::Resend(k) => {
                self.resend_pending = self.waiting.is_some();
                let lines: Vec<(u64, String, Src)> = self.history.iter().filter(|(n, _, _)| *n >= k).cloned().collect();
                self.resend = lines.into();
            }
            Reply::Start => {
                if self.ready {
                    self.abort_job("Принтер перезагрузился — печать прервана");
                    self.waiting = None;
                    self.resend.clear();
                    self.n = 0;
                    let _ = self.send_new("M110 N0".into(), Src::Service);
                }
            }
            Reply::Halted(text) => {
                self.abort_job("");
                self.waiting = None;
                self.prio.clear();
                self.manual.clear();
                if self.sh().status != Status::Halted {
                    let text = text.trim_start_matches("Error:").trim_end_matches('.').trim();
                    self.set(
                        Status::Halted,
                        &format!("Принтер остановлен ({text}). Перезапусти принтер и подключись снова."),
                    );
                }
            }
            Reply::Error(_) => {}
            Reply::Position(x, y, z) => {
                let idle = self.job.is_none();
                let mut sh = self.sh();
                sh.reported = Some((x, y, z));
                if idle {
                    sh.tracker.x = x;
                    sh.tracker.y = y;
                    sh.tracker.z = z;
                }
            }
            Reply::Other(text) => {
                if text.starts_with("FIRMWARE_NAME") {
                    self.sh().firmware = text;
                }
            }
        }
    }

    fn abort_job(&mut self, message: &str) {
        self.job = None;
        self.awake.set(false);
        let mut sh = self.sh();
        sh.job = None;
        if !message.is_empty() {
            sh.status = Status::Idle;
            sh.message = message.to_string();
        }
    }

    fn message(&mut self, m: Msg) -> bool {
        match m {
            Msg::Send(lines) => self.manual.extend(lines),
            Msg::Start(job) => {
                let job = *job;
                {
                    let mut sh = self.sh();
                    sh.segs_gen += 1;
                    sh.segs.clear();
                    sh.last_map = Some(job.map.clone());
                    sh.last_pen = Some(job.pen);
                    sh.segs_owner = Some((job.kind.clone(), job.part, job.test));
                    sh.finished = None;
                    sh.job = Some(JobInfo {
                        label: job.label.clone(),
                        kind: job.kind.clone(),
                        part: job.part,
                        total: job.lines.len(),
                        done: 0,
                        started: Instant::now(),
                    });
                    sh.status = Status::Printing;
                    sh.message = format!("Печать: {}", job.label);
                    sh.log(format!("— начало печати: {}", job.label));
                }
                logs::info("handwriter.printer", &format!("Печать по кабелю: {}", job.label));
                self.awake.set(true);
                self.job = Some(RunJob { lines: job.lines, next: 0, paused: false, pen: job.pen, saved: None });
            }
            Msg::Pause => {
                if let Some(j) = self.job.as_mut()
                    && !j.paused
                {
                    j.paused = true;
                    self.prio.push_back(Prio::LiftIfDown);
                    self.set(Status::Paused, "Пауза: карандаш поднят");
                }
            }
            Msg::Resume => {
                if let Some(j) = self.job.as_mut()
                    && j.paused
                {
                    j.paused = false;
                    self.prio.push_back(Prio::Return);
                    self.set(Status::Printing, "Печать продолжается");
                }
            }
            Msg::Stop => {
                if let Some(j) = self.job.take() {
                    let end = j.pen.end;
                    let fz = j.pen.feed_z;
                    self.resend.clear();
                    self.manual.clear();
                    self.prio.clear();
                    self.prio.push_back(Prio::Line("M410".into()));
                    self.prio.push_back(Prio::Line("G90".into()));
                    self.prio.push_back(Prio::Line(format!("G0 Z{} F{}", fmt(end), fz.round() as i64)));
                    self.awake.set(false);
                    let mut sh = self.sh();
                    sh.job = None;
                    sh.status = Status::Idle;
                    sh.message = "Печать остановлена, карандаш поднят".into();
                    sh.log("— печать остановлена".into());
                }
            }
            Msg::Emergency => {
                let _ = self.write_raw("M112");
                self.job = None;
                self.awake.set(false);
                self.waiting = None;
                self.prio.clear();
                self.manual.clear();
                self.resend.clear();
                let mut sh = self.sh();
                sh.job = None;
                sh.status = Status::Halted;
                sh.message = "Аварийная остановка (M112). Перезапусти принтер и подключись снова.".into();
                sh.log("> M112 (аварийная остановка)".into());
                logs::warn("handwriter.printer", "Аварийная остановка M112");
            }
            Msg::Disconnect => return false,
        }
        true
    }

    fn next_send(&mut self) -> Result<(), String> {
        if self.waiting.is_some() || !self.ready && self.n > 0 {
            return Ok(());
        }
        if let Some((n, cmd, src)) = self.resend.pop_front() {
            return self.send(n, cmd, src);
        }
        if self.sh().status == Status::Halted {
            return Ok(());
        }
        while let Some(p) = self.prio.pop_front() {
            let cmd = match p {
                Prio::Line(l) => Some(l),
                Prio::LiftIfDown => {
                    let t = self.sh().tracker;
                    let pen = self.job.as_ref().map(|j| j.pen);
                    if let (Some(j), Some(pen)) = (self.job.as_mut(), pen) {
                        j.saved = Some((t.x, t.y, t.z));
                        (t.z <= pen.threshold()).then(|| format!("G0 Z{} F{}", fmt(pen.up), pen.feed_z.round() as i64))
                    } else {
                        None
                    }
                }
                Prio::Return => {
                    let saved = self.job.as_mut().and_then(|j| j.saved.take().map(|s| (s, j.pen)));
                    if let Some(((x, y, z), pen)) = saved {
                        self.prio.push_front(Prio::Line(format!("G1 Z{} F{}", fmt(z), pen.feed_z.round() as i64)));
                        self.prio.push_front(Prio::Line(format!(
                            "G0 X{} Y{} F{}",
                            fmt(x),
                            fmt(y),
                            pen.feed_travel.round() as i64
                        )));
                        Some("G90".to_string())
                    } else {
                        None
                    }
                }
            };
            if let Some(cmd) = cmd {
                return self.send_new(cmd, Src::Service);
            }
        }
        if let Some(j) = self.job.as_mut()
            && !j.paused
        {
            if j.next < j.lines.len() {
                let cmd = j.lines[j.next].clone();
                j.next += 1;
                return self.send_new(cmd, Src::Job);
            }
            self.finish_job();
            return Ok(());
        }
        if let Some(cmd) = self.manual.pop_front() {
            return self.send_new(cmd, Src::Manual);
        }
        if self.job.is_none() && self.last_poll.elapsed() > POLL_EVERY {
            self.last_poll = Instant::now();
            return self.send_new("M114".into(), Src::Poll);
        }
        Ok(())
    }

    fn finish_job(&mut self) {
        self.job = None;
        self.awake.set(false);
        let mut sh = self.sh();
        if let Some(j) = sh.job.take() {
            sh.finished = Some((j.kind.clone(), j.part));
            sh.log(format!("— готово: {}", j.label));
            sh.message = format!("Готово: {}", j.label);
            logs::info("handwriter.printer", &format!("Печать завершена: {}", j.label));
        }
        sh.status = Status::Idle;
    }

    fn run(mut self) {
        let result = (|| -> Result<(), String> {
            self.startup()?;
            loop {
                while let Ok(m) = self.rx.try_recv() {
                    if !self.message(m) {
                        return Ok(());
                    }
                }
                for line in self.read_lines()? {
                    self.handle(&line);
                }
                if let Some((_, cmd, _, since)) = &self.waiting
                    && !self.warned
                    && since.elapsed() > SILENCE_WARN
                {
                    let text = format!("Принтер не отвечает на «{cmd}» больше 20 с");
                    self.warned = true;
                    self.sh().log(format!("— {text}"));
                }
                self.next_send()?;
            }
        })();
        self.awake.set(false);
        let mut sh = self.sh.lock().expect("printer lock");
        sh.job = None;
        match result {
            Ok(()) => {
                sh.status = Status::Disconnected;
                sh.message = "Отключено".into();
                sh.log("— отключено".into());
            }
            Err(e) => {
                sh.status = Status::Disconnected;
                sh.message = format!("Связь с принтером потеряна: {e}");
                sh.log(format!("— связь потеряна: {e}"));
                logs::warn("handwriter.printer", &format!("Связь потеряна: {e}"));
            }
        }
    }
}
