pub fn numbered(n: u64, cmd: &str) -> String {
    let body = format!("N{n} {cmd}");
    let cs = body.bytes().fold(0u8, |a, b| a ^ b);
    format!("{body}*{cs}")
}

pub fn clean_line(line: &str) -> Option<String> {
    let code = line.split(';').next().unwrap_or("").trim();
    (!code.is_empty()).then(|| code.to_string())
}

pub fn gcode_lines(text: &str) -> Vec<String> {
    text.lines().filter_map(clean_line).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Ok,
    Resend(u64),
    Busy,
    Start,
    Halted(String),
    Error(String),
    Position(f64, f64, f64),
    Other(String),
}

fn axis(text: &str, name: &str) -> Option<f64> {
    let i = text.find(name)?;
    let rest = &text[i + name.len()..];
    let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

pub fn parse_reply(line: &str) -> Reply {
    let t = line.trim();
    let low = t.to_ascii_lowercase();
    if low == "ok" || low.starts_with("ok ") {
        return Reply::Ok;
    }
    if let Some(rest) = low.strip_prefix("resend:").or_else(|| low.strip_prefix("rs ")) {
        let digits: String = rest.trim().trim_start_matches('n').chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse() {
            return Reply::Resend(n);
        }
    }
    if low == "start" {
        return Reply::Start;
    }
    if low.contains("busy:") {
        return Reply::Busy;
    }
    if low.contains("printer halted") || low.contains("kill() called") || low.starts_with("!!") {
        return Reply::Halted(t.to_string());
    }
    if low.starts_with("error:") {
        return Reply::Error(t.to_string());
    }
    if t.starts_with("X:") {
        let head = t.split(" Count").next().unwrap_or(t);
        if let (Some(x), Some(y), Some(z)) = (axis(head, "X:"), axis(head, "Y:"), axis(head, "Z:")) {
            return Reply::Position(x, y, z);
        }
    }
    Reply::Other(t.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tracker {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub absolute: bool,
}

impl Default for Tracker {
    fn default() -> Self {
        Tracker { x: 0.0, y: 0.0, z: 0.0, absolute: true }
    }
}

fn word(cmd: &str, letter: char) -> Option<f64> {
    cmd.split_whitespace().skip(1).find_map(|w| {
        let mut ch = w.chars();
        (ch.next()?.to_ascii_uppercase() == letter).then(|| ch.as_str().parse().ok()).flatten()
    })
}

impl Tracker {
    pub fn apply(&mut self, cmd: &str) -> Option<((f64, f64), (f64, f64))> {
        let head = cmd.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
        match head.as_str() {
            "G90" => self.absolute = true,
            "G91" => self.absolute = false,
            "G92" => {
                if let Some(v) = word(cmd, 'X') {
                    self.x = v;
                }
                if let Some(v) = word(cmd, 'Y') {
                    self.y = v;
                }
                if let Some(v) = word(cmd, 'Z') {
                    self.z = v;
                }
            }
            "G28" => {
                let axes: Vec<char> = cmd
                    .split_whitespace()
                    .skip(1)
                    .filter_map(|w| w.chars().next())
                    .map(|c| c.to_ascii_uppercase())
                    .filter(|c| "XYZ".contains(*c))
                    .collect();
                let all = axes.is_empty();
                if all || axes.contains(&'X') {
                    self.x = 0.0;
                }
                if all || axes.contains(&'Y') {
                    self.y = 0.0;
                }
                if all || axes.contains(&'Z') {
                    self.z = 0.0;
                }
            }
            "G0" | "G1" | "G00" | "G01" => {
                let from = (self.x, self.y);
                let step = |cur: f64, v: Option<f64>, abs: bool| v.map_or(cur, |v| if abs { v } else { cur + v });
                self.x = step(self.x, word(cmd, 'X'), self.absolute);
                self.y = step(self.y, word(cmd, 'Y'), self.absolute);
                self.z = step(self.z, word(cmd, 'Z'), self.absolute);
                if from != (self.x, self.y) {
                    return Some((from, (self.x, self.y)));
                }
            }
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_matches_marlin() {
        assert_eq!(numbered(0, "M110 N0"), "N0 M110 N0*125");
        assert_eq!(numbered(1, "G28"), format!("N1 G28*{}", "N1 G28".bytes().fold(0u8, |a, b| a ^ b)));
    }

    #[test]
    fn replies() {
        assert_eq!(parse_reply("ok"), Reply::Ok);
        assert_eq!(parse_reply("ok N12 P15 B3"), Reply::Ok);
        assert_eq!(parse_reply("Resend: 42"), Reply::Resend(42));
        assert_eq!(parse_reply("rs N7"), Reply::Resend(7));
        assert_eq!(parse_reply("echo:busy: processing"), Reply::Busy);
        assert_eq!(
            parse_reply("X:10.00 Y:-2.50 Z:4.00 E:0.00 Count X:800 Y:-200 Z:1600"),
            Reply::Position(10.0, -2.5, 4.0)
        );
        assert!(matches!(parse_reply("Error:Printer halted. kill() called!"), Reply::Halted(_)));
        assert!(matches!(parse_reply("Error:Line Number is not Last Line Number+1, Last Line: 3"), Reply::Error(_)));
    }

    #[test]
    fn tracker_follows_moves() {
        let mut t = Tracker::default();
        assert_eq!(t.apply("G92 X0 Y0 Z0"), None);
        assert_eq!(t.apply("G0 Z4.00 F600"), None);
        assert_eq!(t.apply("G0 X10 Y5 F3000"), Some(((0.0, 0.0), (10.0, 5.0))));
        t.apply("G91");
        assert_eq!(t.apply("G0 X-2"), Some(((10.0, 5.0), (8.0, 5.0))));
        t.apply("G90");
        t.apply("G1 Z-1 F600");
        assert_eq!(t.z, -1.0);
        assert_eq!(gcode_lines("; head\nG21\n\nG1 X1 Y2 ; c\n"), vec!["G21", "G1 X1 Y2"]);
    }
}
