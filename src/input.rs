//! Keyboard / mouse activity, counts only — never which keys.
//! Prefer cheap /proc/interrupts deltas; fall back to read-only /dev/input/event*
//! when the process can open them (e.g. user is in the `input` group). Fail soft.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::time::Instant;

/// Keyboard + mouse rates, each ~0..1 after a few seconds of smoothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Activity {
    pub keyboard: f32,
    pub mouse: f32,
}

enum Source {
    Irq(Vec<(usize, bool)>), // (column, is_keyboard)
    Evdev(Vec<(File, bool)>), // (fd, is_keyboard-ish)
}

pub struct Tracker {
    src: Option<Source>,
    note: Option<&'static str>,
    prev: Vec<u64>,
    kb_e: f32,
    ms_e: f32,
    last: Instant,
}

impl Tracker {
    pub fn open(verbose: bool) -> Tracker {
        let mut t = Tracker {
            src: None,
            note: None,
            prev: Vec::new(),
            kb_e: 0.0,
            ms_e: 0.0,
            last: Instant::now(),
        };
        if let Some(irq) = try_irq() {
            t.prev = irq.1;
            t.src = Some(Source::Irq(irq.0));
            if verbose {
                say!("input activity: /proc/interrupts ({})", irq.2);
            }
        } else if let Some(ev) = try_evdev() {
            let n = ev.len();
            t.prev = vec![0; n];
            t.src = Some(Source::Evdev(ev));
            if verbose {
                say!("input activity: /dev/input/event* ({n} device{})", if n == 1 { "" } else { "s" });
            }
        } else {
            t.note = Some("no readable keyboard/mouse counters (/proc/interrupts had no hid/i8042 lines; /dev/input not readable — add yourself to the `input` group if you want this)");
            if verbose {
                say!("input activity: off ({})", t.note.unwrap());
            }
        }
        t
    }

    pub fn why_off(&self) -> Option<&'static str> {
        self.note
    }

    /// Poll deltas and return smoothed activity. Safe to call ~2× per second.
    pub fn poll(&mut self) -> Activity {
        let dt = self.last.elapsed().as_secs_f32().max(0.05);
        self.last = Instant::now();
        let (mut kb, mut ms) = (0.0f32, 0.0f32);
        match &mut self.src {
            Some(Source::Irq(cols)) => {
                if let Some(cur) = read_irq_counts(cols) {
                    for (i, &c) in cur.iter().enumerate() {
                        let d = c.saturating_sub(self.prev.get(i).copied().unwrap_or(c)) as f32 / dt;
                        if cols[i].1 {
                            kb += d;
                        } else {
                            ms += d;
                        }
                    }
                    self.prev = cur;
                }
            }
            Some(Source::Evdev(devs)) => {
                for (f, is_kb) in devs.iter_mut() {
                    let n = drain_events(f) as f32 / dt;
                    if *is_kb {
                        kb += n;
                    } else {
                        ms += n;
                    }
                }
            }
            None => {}
        }
        // map rates to 0..1. irq: ~2 interrupts per keystroke (press+release), PS/2 mouse
        // ~3 per packet. evdev: we count key presses/repeats and motion events.
        // ~6 keystrokes/s (fast typing) saturates; continuous mouse motion is milder.
        let (kdiv, mdiv) = match self.src {
            Some(Source::Irq(_)) => (12.0, 240.0),
            _ => (6.0, 300.0),
        };
        let kb_raw = (kb / kdiv).clamp(0.0, 1.0);
        let ms_raw = (ms / mdiv).clamp(0.0, 1.0);
        // ~3 s smoothing so a pause doesn't instantly drop, and a burst doesn't spike
        let a = 1.0 - (-dt / 3.0).exp();
        self.kb_e += (kb_raw - self.kb_e) * a;
        self.ms_e += (ms_raw - self.ms_e) * a;
        Activity { keyboard: self.kb_e, mouse: self.ms_e }
    }
}

fn irq_path() -> String {
    // test hook: point at a fake interrupts file
    std::env::var("SYSJAM_INTERRUPTS").unwrap_or_else(|_| "/proc/interrupts".into())
}

/// Parse /proc/interrupts: (irq, is_keyboard, total count) for i8042 / HID lines.
fn parse_irqs() -> Option<Vec<(String, bool, u64)>> {
    let text = fs::read_to_string(irq_path()).ok()?;
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let mut parts = line.split_whitespace();
        let irq = parts.next()?.trim_end_matches(':').to_string();
        let mut total = 0u64;
        let mut tail = String::new();
        for p in parts {
            if tail.is_empty() {
                if let Ok(n) = p.parse::<u64>() {
                    total += n;
                    continue;
                }
            }
            tail.push_str(p);
            tail.push(' ');
        }
        let t = tail.to_ascii_lowercase();
        let kb = if t.contains("i8042") {
            irq != "12" // irq1 = keyboard, irq12 = PS/2 mouse/touchpad
        } else if t.contains("hid") || t.contains("elan") || t.contains("syna") || t.contains("touchpad") {
            false // i2c-hid touchpads etc.
        } else {
            continue;
        };
        out.push((irq, kb, total));
    }
    Some(out)
}

fn try_irq() -> Option<(Vec<(usize, bool)>, Vec<u64>, String)> {
    let v = parse_irqs()?;
    if v.is_empty() {
        return None;
    }
    let cols = v.iter().enumerate().map(|(i, x)| (i, x.1)).collect();
    let prev = v.iter().map(|x| x.2).collect();
    let label = v.iter().map(|x| format!("irq {} {}", x.0, if x.1 { "kbd" } else { "pointer" })).collect::<Vec<_>>().join(", ");
    Some((cols, prev, label))
}

fn read_irq_counts(cols: &[(usize, bool)]) -> Option<Vec<u64>> {
    let v = parse_irqs()?;
    (v.len() == cols.len()).then(|| v.iter().map(|x| x.2).collect())
}

fn try_evdev() -> Option<Vec<(File, bool)>> {
    let dir = fs::read_dir(std::env::var("SYSJAM_INPUT_DIR").unwrap_or_else(|_| "/dev/input".into())).ok()?;
    let mut out = Vec::new();
    for e in dir.flatten() {
        let name = e.file_name();
        let n = name.to_string_lossy();
        if !n.starts_with("event") {
            continue;
        }
        let path: PathBuf = e.path();
        let f = match std::os::unix::fs::OpenOptionsExt::custom_flags(File::options().read(true), libc::O_NONBLOCK | libc::O_CLOEXEC).open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        // name heuristic via sysfs; fall back to the device node name
        let sys = format!("/sys/class/input/{n}/device/name");
        let label = fs::read_to_string(&sys).unwrap_or_else(|_| n.to_string()).to_ascii_lowercase();
        let is_kb = label.contains("keyboard") || label.contains("kbd");
        let is_ms = label.contains("mouse") || label.contains("touchpad") || label.contains("trackpoint");
        if !(is_kb || is_ms) {
            // skip pure power/lid/consumer keys
            continue;
        }
        // non-blocking
        unsafe {
            let fd = f.as_raw_fd();
            let fl = libc::fcntl(fd, libc::F_GETFL);
            if fl >= 0 {
                let _ = libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
            }
        }
        out.push((f, is_kb || !is_ms));
    }
    (!out.is_empty()).then_some(out)
}

/// Drain available input_event structs; return the number of useful events.
/// We only count EV_KEY (1) press/release and EV_REL (2) / EV_ABS (3) motion —
/// never the key code content beyond "there was an event".
fn drain_events(f: &mut File) -> u32 {
    // struct input_event is 24 bytes on 64-bit (timeval 16 + type 2 + code 2 + value 4)
    let mut buf = [0u8; 24 * 64];
    let mut n = 0u32;
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(got) => {
                let mut i = 0;
                while i + 24 <= got {
                    let typ = u16::from_ne_bytes([buf[i + 16], buf[i + 17]]);
                    let val = i32::from_ne_bytes([buf[i + 20], buf[i + 21], buf[i + 22], buf[i + 23]]);
                    // KEY: count press (1) and repeat (2), ignore release (0)
                    // REL/ABS: any non-zero motion
                    if (typ == 1 && val != 0) || (typ == 2) || (typ == 3 && val != 0) {
                        n += 1;
                    }
                    i += 24;
                }
                if got < buf.len() {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => {
                let _ = f.seek(SeekFrom::End(0));
                break;
            }
        }
    }
    n
}
