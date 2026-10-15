//! Keyboard / mouse activity, counts only — never which keys.
//!
//! Sources, best first:
//! 1. evdev: read-only /dev/input/event* for keyboard/pointer devices. Works when the
//!    user is in the `input` group, or when the binary is installed setgid `input`:
//!    [`early_open`] runs first thing in `main`, opens the nodes, then permanently drops
//!    the elevated gid (setresgid(rgid, rgid, rgid)) and verifies it can't be regained.
//! 2. no-permission fallbacks, combined: per-device USB HID report counters
//!    (/sys/bus/usb/devices/*/urbnum, world-readable) and the specific /proc/interrupts
//!    lines for i8042 and i2c-hid touchpads/keyboards.
//! 3. aggregate /proc/interrupts lines (xhci_hcd / ehci / i2c_designware) with an idle
//!    baseline, gated against USB storage and USB network traffic. Approximate.
//!
//! Only "an event happened" is ever counted; key codes are never stored or logged.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

/// Combined keyboard + mouse activity, ~0..1 after a few seconds of smoothing.
/// (One signal: on most machines the two counts come from the same source anyway.)
#[derive(Clone, Copy, Debug, Default)]
pub struct Activity {
    pub input: f32,
}

struct Dev {
    f: File,
    kb: bool,
    name: String,
    dead: bool,
}

/// What `early_open` found, before any thread exists.
struct Early {
    devs: Vec<Dev>,
    /// evdev nodes that exist but could not be opened, and why
    denied: Option<String>,
    /// opened through an elevated (setgid) gid that has since been dropped
    setgid: bool,
}

static EARLY: Mutex<Option<Early>> = Mutex::new(None);

fn secure() -> bool {
    unsafe { libc::getauxval(libc::AT_SECURE) != 0 }
}

fn die(m: &str) -> ! {
    eprintln!("sysjam: {m}");
    std::process::exit(1);
}

/// Call first thing in `main`, before any thread or library is started.
/// Opens the input devices (possibly via a setgid `input` gid), then drops every
/// elevated id for good. Aborts if the drop can't be verified.
pub fn early_open(want: bool) {
    let (mut rg, mut eg, mut sg) = (0, 0, 0);
    let (mut ru, mut eu, mut su) = (0, 0, 0);
    unsafe {
        libc::getresgid(&mut rg, &mut eg, &mut sg);
        libc::getresuid(&mut ru, &mut eu, &mut su);
    }
    let gid_up = eg != rg || sg != rg;
    let uid_up = eu != ru || su != ru;
    let mut e = Early { devs: Vec::new(), denied: None, setgid: false };
    if want {
        // test hook, never honoured when privileged
        let dir = match std::env::var_os("SYSJAM_INPUT_DIR") {
            Some(d) if !secure() && !gid_up && !uid_up => PathBuf::from(d),
            _ => PathBuf::from("/dev/input"),
        };
        let (devs, denied) = open_evdev(&dir);
        e.devs = devs;
        e.denied = denied;
        e.setgid = gid_up && !e.devs.is_empty();
    }
    unsafe {
        // gid first: once the uid is dropped we couldn't change it any more
        if gid_up && libc::setresgid(rg, rg, rg) != 0 {
            die(&format!("could not drop setgid privileges: {}", std::io::Error::last_os_error()));
        }
        if uid_up && libc::setresuid(ru, ru, ru) != 0 {
            die(&format!("could not drop setuid privileges: {}", std::io::Error::last_os_error()));
        }
        let (mut a, mut b, mut c) = (0, 0, 0);
        libc::getresgid(&mut a, &mut b, &mut c);
        if a != rg || b != rg || c != rg {
            die("setgid drop did not stick (gid still elevated); refusing to run");
        }
        let (mut a, mut b, mut c) = (0, 0, 0);
        libc::getresuid(&mut a, &mut b, &mut c);
        if a != ru || b != ru || c != ru {
            die("setuid drop did not stick; refusing to run");
        }
        // and it must not be possible to get the old ids back
        if gid_up && libc::setresgid(u32::MAX, eg, u32::MAX) == 0 {
            die("elevated gid could be regained after dropping it; refusing to run");
        }
        if uid_up && libc::setresuid(u32::MAX, eu, u32::MAX) == 0 {
            die("elevated uid could be regained after dropping it; refusing to run");
        }
    }
    // The kernel keeps a setgid process non-dumpable, so other processes of the same
    // user can't ptrace us and lift the open input fds. Leave it that way.
    *EARLY.lock().unwrap() = Some(e);
}

fn sys_read(p: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// Lowest word of a sysfs capability bitmap ("120013" or "... 3 0 0 0").
fn cap_bits(s: &str) -> u64 {
    s.split_whitespace().last().and_then(|w| u64::from_str_radix(w, 16).ok()).unwrap_or(0)
}

const EV_KEY: u64 = 1 << 1;
const EV_REL: u64 = 1 << 2;
const EV_ABS: u64 = 1 << 3;
const EV_REP: u64 = 1 << 20;

/// Classify an input device from its sysfs dir (inputN): (keyboard, pointer, name).
fn classify(sys: &Path) -> Option<(bool, bool, String)> {
    let name = sys_read(sys.join("name"))?;
    let l = name.to_ascii_lowercase();
    let ev = sys_read(sys.join("capabilities/ev")).map(|s| cap_bits(&s)).unwrap_or(0);
    const SKIP: [&str; 12] = ["power button", "sleep button", "lid switch", "video bus", "hdmi", "headphone", "speaker", "mic", "accel", "wmi hotkeys", "dp,pcm", "jack"];
    if SKIP.iter().any(|s| l.contains(s)) {
        return Some((false, false, name));
    }
    let kb = (ev & EV_KEY != 0 && ev & EV_REP != 0) || l.contains("keyboard") || l.contains("kbd");
    let ptr = ev & EV_REL != 0 || ["mouse", "touchpad", "trackpoint", "trackpad", "pointing"].iter().any(|s| l.contains(s)) || (ev & EV_ABS != 0 && l.contains("pen"));
    Some((kb, ptr, name))
}

fn open_evdev(dir: &Path) -> (Vec<Dev>, Option<String>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return (Vec::new(), Some(format!("{} doesn't exist", dir.display())));
    };
    let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("event")).collect();
    names.sort_by_key(|n| n[5..].parse::<u32>().unwrap_or(u32::MAX));
    let (mut out, mut eacces, mut other) = (Vec::new(), 0, None);
    for n in &names {
        let (kb, ptr, name) = match classify(&Path::new("/sys/class/input").join(n).join("device")) {
            Some(c) => c,
            None => (true, false, n.clone()), // no sysfs info (test fifo): count as keys
        };
        if !kb && !ptr {
            continue; // never open what we don't need
        }
        let opened = std::os::unix::fs::OpenOptionsExt::custom_flags(File::options().read(true), libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOCTTY).open(dir.join(n));
        match opened {
            Ok(f) => out.push(Dev { f, kb, name, dead: false }),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => eacces += 1,
            Err(e) => other = Some(e.to_string()),
        }
    }
    let denied = if !out.is_empty() {
        None
    } else if eacces > 0 {
        let grp = fs::metadata(dir.join(&names[0])).ok().map(|m| {
            use std::os::unix::fs::MetadataExt;
            let g = unsafe { libc::getgrgid(m.gid()) };
            let gn = if g.is_null() { m.gid().to_string() } else { unsafe { std::ffi::CStr::from_ptr((*g).gr_name) }.to_string_lossy().into_owned() };
            format!("{gn} {:o}", m.mode() & 0o777)
        });
        Some(format!(
            "permission denied on {eacces} keyboard/pointer node{} (/dev/input/event* is root:{}, and you're not in that group)",
            if eacces == 1 { "" } else { "s" },
            grp.unwrap_or_else(|| "input".into())
        ))
    } else if let Some(e) = other {
        Some(e)
    } else if names.is_empty() {
        Some(format!("no event devices in {}", dir.display()))
    } else {
        Some("no keyboard or pointer among the event devices".into())
    };
    (out, denied)
}

// ---------------------------------------------------------------- fallbacks

#[derive(Clone)]
enum Probe {
    /// /sys/bus/usb/devices/X/urbnum
    Urb(PathBuf),
    /// sum of these /proc/interrupts line ids
    Irq(Vec<String>),
}

struct Counter {
    probe: Probe,
    kb: bool,
    ptr: bool,
    /// aggregate controller line (shared with other devices): stronger baseline + gating
    agg: bool,
    prev: Option<u64>,
    floor: Option<f32>,
}

enum Source {
    Evdev(Vec<Dev>),
    Counters(Vec<Counter>),
}

pub struct Tracker {
    src: Option<Source>,
    /// short label for the TUI header
    label: String,
    note: Option<String>,
    kb_e: f32,
    last: Instant,
    /// USB disks / NICs whose traffic also lands on the xhci line
    usb_io: Vec<PathBuf>,
    usb_io_prev: u64,
}

fn irq_path() -> String {
    match std::env::var("SYSJAM_INTERRUPTS") {
        Ok(p) if !secure() => p,
        _ => "/proc/interrupts".into(),
    }
}

/// (irq id, lowercase device names, total count) for every numbered line
fn irq_lines() -> Option<Vec<(String, String, u64)>> {
    let text = fs::read_to_string(irq_path()).ok()?;
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let mut parts = line.split_whitespace();
        let Some(id) = parts.next().map(|x| x.trim_end_matches(':').to_string()) else { continue };
        if !id.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let mut total = 0u64;
        let mut tail = Vec::new();
        for p in parts {
            if tail.is_empty() {
                if let Ok(n) = p.parse::<u64>() {
                    total += n;
                    continue;
                }
            }
            tail.push(p);
        }
        // tail = chip, hwirq-type, then the device name(s); keep the names only
        let names = if tail.len() > 2 { tail[2..].join(" ") } else { tail.join(" ") };
        out.push((id, names.to_ascii_lowercase(), total));
    }
    Some(out)
}

const HID_IRQ: [&str; 14] = ["i8042", "i2c_hid", "i2c-hid", "keyboard", "touchpad", "mouse", "elan", "syna", "alps", "msft0001", "pnp0c50", "pnp0c51", "cyap", "atml"];
const AGG_IRQ: [&str; 6] = ["xhci", "ehci", "ohci", "uhci", "designware", "dw_i2c"];

/// USB devices that carry a HID keyboard / pointer, classified by their input children.
fn usb_hid() -> Vec<(PathBuf, bool, bool, String)> {
    let mut by_dev: Vec<(PathBuf, bool, bool, String)> = Vec::new();
    let Ok(rd) = fs::read_dir("/sys/class/input") else { return by_dev };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if !n.starts_with("input") {
            continue;
        }
        let Ok(real) = fs::canonicalize(e.path()) else { continue };
        let Some((kb, ptr, name)) = classify(&real) else { continue };
        if !kb && !ptr {
            continue;
        }
        // walk up to the usb_device (the dir with urbnum + idVendor)
        let mut p = real.as_path();
        let mut dev = None;
        while let Some(up) = p.parent() {
            if up.join("urbnum").exists() && up.join("idVendor").exists() {
                dev = Some(up.to_path_buf());
                break;
            }
            p = up;
        }
        let Some(dev) = dev else { continue };
        // skip devices that also stream audio (headsets): their urbnum is all isochronous traffic
        let audio = fs::read_dir(&dev).map(|rd| rd.flatten().any(|i| sys_read(i.path().join("bInterfaceClass")).as_deref() == Some("01"))).unwrap_or(false);
        if audio {
            continue;
        }
        let product = sys_read(dev.join("product")).unwrap_or(name);
        match by_dev.iter_mut().find(|x| x.0 == dev) {
            Some(x) => {
                x.1 |= kb;
                x.2 |= ptr;
            }
            None => by_dev.push((dev, kb, ptr, product)),
        }
    }
    by_dev
}

/// sysfs stat files of USB-attached disks and NICs (traffic that also shows on xhci)
fn usb_io_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for (dir, stat) in [("/sys/block", "stat"), ("/sys/class/net", "statistics")] {
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                if fs::canonicalize(e.path()).map(|p| p.to_string_lossy().contains("/usb")).unwrap_or(false) {
                    if stat == "stat" {
                        v.push(e.path().join("stat"));
                    } else {
                        v.push(e.path().join("statistics/rx_packets"));
                        v.push(e.path().join("statistics/tx_packets"));
                    }
                }
            }
        }
    }
    v
}

fn usb_io_count(v: &[PathBuf]) -> u64 {
    v.iter().filter_map(|p| sys_read(p)).map(|s| s.split_whitespace().filter_map(|x| x.parse::<u64>().ok()).sum::<u64>()).sum()
}

impl Tracker {
    /// Picks the input source and logs which one (always: it explains the keys meter).
    pub fn open(_verbose: bool) -> Tracker {
        let early = EARLY.lock().unwrap().take();
        let mut t = Tracker { src: None, label: "none".into(), note: None, kb_e: 0.0, last: Instant::now(), usb_io: Vec::new(), usb_io_prev: 0 };
        let (devs, denied, setgid) = match early {
            Some(e) => (e.devs, e.denied, e.setgid),
            None => {
                let (d, why) = open_evdev(Path::new("/dev/input"));
                (d, why, false)
            }
        };
        if !devs.is_empty() {
            let n = devs.len();
            let names: Vec<&str> = devs.iter().map(|d| d.name.as_str()).collect();
            if setgid {
                say!("input: evdev via setgid input (privileges dropped) — {n} device{}: {}", if n == 1 { "" } else { "s" }, names.join(", "));
                t.label = format!("evdev×{n} setgid");
            } else {
                say!("input: evdev ({n} device{}: {})", if n == 1 { "" } else { "s" }, names.join(", "));
                t.label = format!("evdev×{n}");
            }
            say!("input: counting events only, never key codes");
            t.src = Some(Source::Evdev(devs));
            return t;
        }
        if let Some(why) = &denied {
            say!("input: evdev unavailable: {why}");
            if why.contains("permission") {
                say!("input: to opt in: `sudo usermod -aG input $USER` and log in again (or the setgid install in the README); sysjam counts events only, never key codes");
            }
        }
        // no-permission fallbacks
        let mut cs = Vec::new();
        let mut desc = Vec::new();
        for (dev, kb, ptr, name) in usb_hid() {
            desc.push(format!("usb-hid {name} ({})", kind(kb, ptr)));
            cs.push(Counter { probe: Probe::Urb(dev.join("urbnum")), kb, ptr, agg: false, prev: None, floor: None });
        }
        let lines = irq_lines().unwrap_or_default();
        for (id, names, _) in &lines {
            if names.contains("virtio") || !HID_IRQ.iter().any(|h| names.contains(h)) {
                continue;
            }
            let (kb, ptr) = if names.contains("i8042") {
                (id != "12", id == "12")
            } else {
                let kb = names.contains("keyboard") || names.contains("kbd");
                (kb, !kb)
            };
            let short = names.split([' ', ',']).find(|w| HID_IRQ.iter().any(|h| w.contains(h))).unwrap_or(names).to_string();
            desc.push(format!("irq {id} {short} ({})", kind(kb, ptr)));
            cs.push(Counter { probe: Probe::Irq(vec![id.clone()]), kb, ptr, agg: false, prev: None, floor: None });
        }
        if !cs.is_empty() {
            let any_usb = desc.iter().any(|d| d.starts_with("usb"));
            let any_irq = desc.iter().any(|d| d.starts_with("irq"));
            t.label = match (any_usb, any_irq) {
                (true, true) => "usb-hid+irq".into(),
                (true, false) => "usb-hid".into(),
                _ => if lines.iter().any(|l| l.1.contains("i8042")) { "irq i8042".into() } else { "irq hid".into() },
            };
            say!("input: irq fallback: {}", desc.join(", "));
            t.src = Some(Source::Counters(cs));
            return t;
        }
        let agg: Vec<(String, String)> = lines.iter().filter(|l| AGG_IRQ.iter().any(|a| l.1.contains(a))).map(|l| (l.0.clone(), l.1.clone())).collect();
        if !agg.is_empty() {
            let usb: Vec<String> = agg.iter().filter(|a| !a.1.contains("designware") && !a.1.contains("dw_i2c")).map(|a| a.0.clone()).collect();
            let i2c: Vec<String> = agg.iter().filter(|a| a.1.contains("designware") || a.1.contains("dw_i2c")).map(|a| a.0.clone()).collect();
            let mut what = Vec::new();
            if !usb.is_empty() {
                let n = if agg.iter().any(|a| a.1.contains("xhci")) { "xhci" } else { "ehci" };
                what.push(format!("{n} (irq {})", usb.join("+")));
                t.label = format!("irq {n}≈");
                cs.push(Counter { probe: Probe::Irq(usb), kb: true, ptr: true, agg: true, prev: None, floor: None });
            }
            if !i2c.is_empty() {
                what.push(format!("i2c designware (irq {})", i2c.join("+")));
                if t.label == "none" {
                    t.label = "irq i2c≈".into();
                }
                cs.push(Counter { probe: Probe::Irq(i2c), kb: false, ptr: true, agg: true, prev: None, floor: None });
            }
            t.usb_io = usb_io_paths();
            t.usb_io_prev = usb_io_count(&t.usb_io);
            say!("input: irq fallback: {} — shared controller line(s), idle baseline calibrated, USB disk/net traffic ignored; approximate", what.join(", "));
            t.src = Some(Source::Counters(cs));
            return t;
        }
        let n = "no readable keyboard/mouse counters (no evdev access, no HID/i8042/xhci lines in /proc/interrupts)";
        say!("input: none — {n}");
        t.note = Some(n.into());
        t
    }

    #[allow(dead_code)]
    pub fn why_off(&self) -> Option<&str> {
        self.note.as_deref()
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Poll deltas and return smoothed activity. Safe to call ~2× per second.
    pub fn poll(&mut self) -> Activity {
        let dt = self.last.elapsed().as_secs_f32().max(0.05);
        self.last = Instant::now();
        let (mut kb, mut ms) = (0.0f32, 0.0f32);
        let (kdiv, mdiv);
        match &mut self.src {
            Some(Source::Evdev(devs)) => {
                for d in devs.iter_mut().filter(|d| !d.dead) {
                    let (k, m) = drain_events(d);
                    if d.kb {
                        kb += k as f32 / dt;
                        ms += m as f32 / dt;
                    } else {
                        ms += (k + m) as f32 / dt;
                    }
                }
                // ~6 key presses/s (fast typing) saturates; continuous motion is milder
                (kdiv, mdiv) = (6.0, 300.0);
            }
            Some(Source::Counters(cs)) => {
                let irqs = if cs.iter().any(|c| matches!(c.probe, Probe::Irq(_))) { irq_lines() } else { None };
                // USB storage / network moving? then the shared xhci count isn't input
                let io = usb_io_count(&self.usb_io);
                let io_busy = io != self.usb_io_prev;
                self.usb_io_prev = io;
                for c in cs.iter_mut() {
                    let cur = match &c.probe {
                        Probe::Urb(p) => sys_read(p).and_then(|s| s.parse::<u64>().ok()),
                        Probe::Irq(ids) => irqs.as_ref().map(|v| v.iter().filter(|l| ids.contains(&l.0)).map(|l| l.2).sum()),
                    };
                    let Some(cur) = cur else { continue };
                    let prev = c.prev.replace(cur);
                    let Some(prev) = prev else { continue };
                    let r = cur.saturating_sub(prev) as f32 / dt;
                    if c.agg && io_busy {
                        continue; // neither counts nor teaches the baseline
                    }
                    // idle baseline: falls fast, creeps up slowly (shared controller lines over
                    // ~5 min; dedicated HID lines over ~30 min, so sustained typing isn't absorbed)
                    let fl = c.floor.get_or_insert(r);
                    let tau = if c.agg { 300.0 } else { 1800.0 };
                    if r < *fl {
                        *fl += (r - *fl) * 0.5;
                    } else {
                        *fl += (r - *fl) * (dt / tau).min(1.0);
                    }
                    let margin = if c.agg { 2.0 + 0.08 * *fl } else { 0.5 + 0.05 * *fl };
                    let x = (r - *fl - margin).max(0.0);
                    match (c.kb, c.ptr) {
                        (true, false) => kb += x,
                        (false, true) => ms += x,
                        _ => {
                            // mixed line: a trickle is typing, a stream is pointer motion
                            let k = x.min(20.0);
                            kb += k;
                            ms += x - k;
                        }
                    }
                }
                // ~2 reports / interrupts per keystroke; pointers report at 100-250 Hz
                (kdiv, mdiv) = (12.0, 180.0);
            }
            None => return Activity::default(),
        }
        let raw = (kb / kdiv + ms / mdiv).clamp(0.0, 1.0);
        // ~3 s smoothing so a pause doesn't instantly drop, and a burst doesn't spike
        let a = 1.0 - (-dt / 3.0).exp();
        self.kb_e += (raw - self.kb_e) * a;
        Activity { input: self.kb_e }
    }
}

fn kind(kb: bool, ptr: bool) -> &'static str {
    match (kb, ptr) {
        (true, true) => "kbd+pointer",
        (true, false) => "kbd",
        _ => "pointer",
    }
}

/// Drain available input_event structs; returns (key presses, motion/button events).
/// Only the event type and "non-zero" are looked at — the key code is never read.
fn drain_events(d: &mut Dev) -> (u32, u32) {
    // struct input_event is 24 bytes on 64-bit (timeval 16 + type 2 + code 2 + value 4)
    const SZ: usize = 24;
    let mut buf = [0u8; SZ * 64];
    let (mut k, mut m) = (0u32, 0u32);
    loop {
        match d.f.read(&mut buf) {
            Ok(0) => break,
            Ok(got) => {
                for ev in buf[..got - got % SZ].chunks_exact(SZ) {
                    let typ = u16::from_ne_bytes([ev[16], ev[17]]);
                    let val = i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]);
                    match typ {
                        1 if val == 1 => k += 1,           // EV_KEY press (not release / autorepeat)
                        2 => m += 1,                       // EV_REL motion / wheel
                        3 if val != 0 => m += 1,           // EV_ABS (touchpads)
                        _ => {}
                    }
                }
                if got < buf.len() {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                d.dead = true; // unplugged (ENODEV)
                break;
            }
        }
    }
    (k, m)
}
