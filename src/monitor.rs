//! Cheap /proc sampling. Runs on its own thread at nice 10.

use crate::music;
use crate::shared::{Control, LeadInfo, ScreenColor};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct MonitorCfg {
    pub verbose: bool,
    pub leads: bool,
    pub clicks: f32,
    pub scale_dark: &'static [i32],
    pub scale_bright: &'static [i32],
    pub default_bright: bool,
    pub root: f32,
    pub input: bool,
}

fn read_into(path: &str, buf: &mut String) -> bool {
    buf.clear();
    match fs::File::open(path) {
        Ok(mut f) => f.read_to_string(buf).is_ok(),
        Err(_) => false,
    }
}

struct CpuSnap {
    total: u64,
    idle: u64,
}

fn parse_stat(buf: &str, out: &mut Vec<CpuSnap>) {
    out.clear();
    for line in buf.lines() {
        if !line.starts_with("cpu") {
            break;
        }
        let mut it = line.split_ascii_whitespace();
        it.next();
        let v: Vec<u64> = it.take(8).map(|x| x.parse().unwrap_or(0)).collect();
        if v.len() < 5 {
            continue;
        }
        let total: u64 = v.iter().sum();
        let idle = v[3] + v[4];
        out.push(CpuSnap { total, idle });
    }
}

fn net_bytes(buf: &str) -> u64 {
    let mut sum = 0;
    for line in buf.lines().skip(2) {
        if let Some((name, rest)) = line.split_once(':') {
            if name.trim() == "lo" {
                continue;
            }
            let f: Vec<&str> = rest.split_ascii_whitespace().collect();
            if f.len() >= 9 {
                sum += f[0].parse::<u64>().unwrap_or(0) + f[8].parse::<u64>().unwrap_or(0);
            }
        }
    }
    sum
}

fn disk_list() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/block") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if ["loop", "ram", "zram", "dm-", "md", "sr", "fd"].iter().any(|p| n.starts_with(p)) {
                continue;
            }
            v.push(n);
        }
    }
    v
}

fn disk_write_bytes(buf: &str, disks: &[String]) -> u64 {
    let mut sum = 0;
    for line in buf.lines() {
        let f: Vec<&str> = line.split_ascii_whitespace().take(10).collect();
        if f.len() >= 10 && disks.iter().any(|d| d == f[2]) {
            sum += f[9].parse::<u64>().unwrap_or(0) * 512;
        }
    }
    sum
}

fn vmstat_dirtied(buf: &str) -> u64 {
    for line in buf.lines() {
        if let Some(v) = line.strip_prefix("nr_dirtied ") {
            return v.trim().parse::<u64>().unwrap_or(0) * 4096;
        }
    }
    0
}

/// Parse /proc/<pid>/stat: (comm, utime+stime, num_threads)
fn parse_pid_stat(buf: &str) -> Option<(&str, u64, u32)> {
    let l = buf.find('(')?;
    let r = buf.rfind(')')?;
    let comm = &buf[l + 1..r];
    let mut it = buf[r + 1..].split_ascii_whitespace();
    let f: Vec<&str> = it.by_ref().take(18).collect();
    if f.len() < 18 {
        return None;
    }
    let t = f[11].parse::<u64>().ok()? + f[12].parse::<u64>().ok()?;
    let nt = f[17].parse::<u32>().unwrap_or(1);
    Some((comm, t, nt))
}

struct Rates {
    last: Option<(u64, Instant)>,
}
impl Rates {
    fn new() -> Self {
        Rates { last: None }
    }
    fn update(&mut self, v: u64, now: Instant) -> f64 {
        let r = match self.last {
            Some((pv, pt)) => {
                let dt = now.duration_since(pt).as_secs_f64().max(1e-3);
                v.saturating_sub(pv) as f64 / dt
            }
            None => 0.0,
        };
        self.last = Some((v, now));
        r
    }
}

struct ThreadScanner {
    clk: f64,
    self_pid: i32,
    procs: HashMap<i32, u64>,
    threads: HashMap<i32, u64>,
    last: Option<Instant>,
    buf: String,
}

struct Cand {
    tid: i32,
    pcomm: String,
    name: String,
    cpu: f32,
}

impl ThreadScanner {
    fn new() -> Self {
        let clk = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
        ThreadScanner {
            clk: if clk > 0.0 { clk } else { 100.0 },
            self_pid: std::process::id() as i32,
            procs: HashMap::new(),
            threads: HashMap::new(),
            last: None,
            buf: String::with_capacity(1024),
        }
    }

    /// Two-stage scan: all /proc/<pid>/stat (one small read each), then only the
    /// tasks of the few hottest processes. Total task reads capped.
    fn scan(&mut self) -> Vec<Cand> {
        let now = Instant::now();
        let dt = self.last.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        self.last = Some(now);
        let mut newp = HashMap::with_capacity(self.procs.len() + 16);
        let mut hot: Vec<(i32, String, f32, u32)> = Vec::new();
        let mut path = String::with_capacity(64);
        if let Ok(rd) = fs::read_dir("/proc") {
            for e in rd.flatten() {
                let name = e.file_name();
                let Some(pid) = name.to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
                if pid == self.self_pid {
                    continue;
                }
                path.clear();
                use std::fmt::Write;
                let _ = write!(path, "/proc/{}/stat", pid);
                let mut b = std::mem::take(&mut self.buf);
                if read_into(&path, &mut b) {
                    if let Some((comm, t, nt)) = parse_pid_stat(&b) {
                        if let Some(&pt) = self.procs.get(&pid) {
                            if dt > 0.0 {
                                let cpu = (t.saturating_sub(pt)) as f64 / (self.clk * dt);
                                if cpu >= 0.03 {
                                    hot.push((pid, comm.to_string(), cpu as f32, nt));
                                }
                            }
                        }
                        newp.insert(pid, t);
                    }
                }
                self.buf = b;
            }
        }
        self.procs = newp;
        hot.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
        hot.truncate(8);
        let mut multi = 0;

        let mut cands = Vec::new();
        let mut newt = HashMap::new();
        let mut budget = 256usize;
        for (pid, pcomm, pcpu, nt) in hot {
            if nt <= 1 {
                cands.push(Cand { tid: pid, pcomm: pcomm.clone(), name: pcomm, cpu: pcpu });
                continue;
            }
            multi += 1;
            if multi > 4 {
                continue; // cap per-thread scans to the 4 hottest multi-threaded processes
            }
            let tdir = format!("/proc/{}/task", pid);
            let Ok(rd) = fs::read_dir(&tdir) else { continue };
            for e in rd.flatten() {
                if budget == 0 {
                    break;
                }
                budget -= 1;
                let Some(tid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
                let p = format!("{}/{}/stat", tdir, tid);
                let mut b = std::mem::take(&mut self.buf);
                if read_into(&p, &mut b) {
                    if let Some((tcomm, t, _)) = parse_pid_stat(&b) {
                        if let Some(&pt) = self.threads.get(&tid) {
                            if dt > 0.0 {
                                let cpu = (t.saturating_sub(pt)) as f64 / (self.clk * dt);
                                if cpu >= 0.03 {
                                    let name = if tcomm == pcomm { pcomm.clone() } else { format!("{}/{}", pcomm, tcomm) };
                                    cands.push(Cand { tid, pcomm: pcomm.clone(), name, cpu: cpu as f32 });
                                }
                            }
                        }
                        newt.insert(tid, t);
                    }
                }
                self.buf = b;
            }
        }
        self.threads = newt;
        cands.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap());
        cands
    }
}

/// Compositor, audio-server and display threads never sing.
fn ignored(pcomm: &str, name: &str) -> bool {
    const P: &[&str] = &[
        "cosmic-comp", "cosmic-panel", "pipewire", "pipewire-pulse", "wireplumber", "pulseaudio",
        "Xwayland", "Xorg", "gnome-shell", "kwin_wayland", "kwin_x11", "sway", "mutter", "sysjam",
        "pw-cat", "pw-play", "jackd",
    ];
    let lname = name.to_ascii_lowercase();
    P.iter().any(|p| pcomm == *p || pcomm.starts_with(p))
        || ["pipewire", "data-loop", "pw-", "audio", "pulse", "alsa"].iter().any(|k| lname.contains(k))
}

fn human(b: f64) -> String {
    if b >= 1e6 {
        format!("{:.1}MB/s", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0}kB/s", b / 1e3)
    } else {
        format!("{:.0}B/s", b)
    }
}

pub fn run(cfg: MonitorCfg, ctl: Arc<Mutex<Control>>, screen: Arc<Mutex<ScreenColor>>, stop: Arc<AtomicBool>, msgs: Option<std::sync::mpsc::Receiver<String>>) {
    crate::shared::set_thread_nice(10);
    let mut buf = String::with_capacity(16 * 1024);
    let mut cpus: Vec<CpuSnap> = Vec::new();
    let mut prev_cpus: Vec<CpuSnap> = Vec::new();
    let mut disks = disk_list();
    let mut net = Rates::new();
    let mut disk = Rates::new();
    let mut dirt = Rates::new();
    let mut scanner = ThreadScanner::new();
    let mut prev_files: Option<i64> = None;
    let mut fd_rate_acc = 0.0f64;
    let mut c = Control::default();
    let bright = cfg.default_bright;
    let mut cpu_s = 0.0f32;
    // single, rare lead voice with long hold + cooldown
    let mut lead: Option<(LeadInfo, f32, f32)> = None; // (info, held s, below s)
    let mut sustain: HashMap<i32, f32> = HashMap::new();
    let mut cooldown = 0.0f32;
    let mut last_msg: Option<Instant> = None;
    // long-term smoothed metrics (nudges, not drivers)
    let (mut cpu_e, mut cores_e, mut net_e, mut file_e, mut fd_e, mut score_e) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut started = false;
    let mut screen_change = 0.0f32;
    let mut prev_raw = 0.0f32;
    let mut input = if cfg.input { Some(crate::input::Tracker::open(cfg.verbose)) } else {
        if cfg.verbose { say!("input activity: off (--no-input)"); }
        None
    };
    let (mut net_r, mut wr_r, mut fd_r) = (0.0f64, 0.0f64, 0.0f64);
    let mut cores_busy = 0usize;
    let start = Instant::now();
    let mut tick: u64 = 0;
    let tickd = Duration::from_millis(500);

    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        // --- file handles: every 0.5 s ---
        if read_into("/proc/sys/fs/file-nr", &mut buf) {
            if let Some(a) = buf.split_ascii_whitespace().next().and_then(|x| x.parse::<i64>().ok()) {
                if let Some(p) = prev_files {
                    fd_rate_acc += (a - p).unsigned_abs() as f64;
                }
                prev_files = Some(a);
            }
        }

        if tick % 2 == 0 {
            // --- 1 s metrics ---
            fd_r = fd_rate_acc;
            fd_rate_acc = 0.0;
            if read_into("/proc/stat", &mut buf) {
                std::mem::swap(&mut cpus, &mut prev_cpus);
                parse_stat(&buf, &mut cpus);
                if prev_cpus.len() == cpus.len() && !cpus.is_empty() {
                    let f = |a: &CpuSnap, b: &CpuSnap| {
                        let dt = a.total.saturating_sub(b.total) as f32;
                        if dt <= 0.0 { 0.0 } else { 1.0 - a.idle.saturating_sub(b.idle) as f32 / dt }
                    };
                    cpu_s = f(&cpus[0], &prev_cpus[0]).clamp(0.0, 1.0);
                    cores_busy = cpus[1..].iter().zip(&prev_cpus[1..]).filter(|(a, b)| f(a, b) > 0.2).count();
                }
            }
            if read_into("/proc/net/dev", &mut buf) {
                net_r = net.update(net_bytes(&buf), now);
            }
            if tick % 60 == 0 && tick > 0 {
                disks = disk_list();
            }
            let mut w = 0.0;
            if read_into("/proc/diskstats", &mut buf) {
                w = disk.update(disk_write_bytes(&buf, &disks), now);
            }
            if read_into("/proc/vmstat", &mut buf) {
                w = w.max(dirt.update(vmstat_dirtied(&buf), now));
            }
            wr_r = w;
        }

        if tick % 3 == 0 && cfg.leads {
            // --- threads every 1.5 s: only a sustained heavy thread may sing ---
            let dt = 1.5f32;
            let cands = scanner.scan();
            let heavy: Vec<&Cand> = cands.iter().filter(|c| c.cpu >= 0.40 && !ignored(&c.pcomm, &c.name)).collect();
            let mut ns = HashMap::new();
            for c in &heavy {
                ns.insert(c.tid, sustain.get(&c.tid).copied().unwrap_or(0.0) + dt);
            }
            sustain = ns;
            cooldown = (cooldown - dt).max(0.0);
            let mut msg: Option<String> = None;
            if let Some((info, held, below)) = lead.as_mut() {
                *held += dt;
                match cands.iter().find(|c| c.tid == info.tid) {
                    Some(c) => {
                        info.cpu = c.cpu;
                        if c.cpu < 0.25 { *below += dt } else { *below = 0.0 }
                    }
                    None => {
                        info.cpu *= 0.8;
                        *below += dt;
                    }
                }
                if *held >= 25.0 && *below >= 10.0 {
                    msg = Some(format!("  quiet:   {} (held {:.0}s)", info.name, held));
                    lead = None;
                    cooldown = 20.0;
                }
            } else if cooldown <= 0.0 {
                if let Some(c) = heavy.iter().filter(|c| sustain.get(&c.tid).copied().unwrap_or(0.0) >= 10.0).max_by(|a, b| a.cpu.partial_cmp(&b.cpu).unwrap()) {
                    let hash = music::fnv1a(&c.name);
                    let scale = if bright { cfg.scale_bright } else { cfg.scale_dark };
                    let (deg, _) = music::lead_pattern(hash, scale.len() as i32);
                    let f = music::semi_to_freq(cfg.root, music::deg_to_semi(scale, deg) as f32);
                    msg = Some(format!("\u{266a} singing: {} ({:.0}% cpu for >10s) -> lead ({})", c.name, c.cpu * 100.0, music::note_name(f)));
                    lead = Some((LeadInfo { tid: c.tid, name: c.name.clone(), cpu: c.cpu, hash }, 0.0, 0.0));
                }
            }
            if let Some(m) = msg {
                // rate-limited, verbose only
                if cfg.verbose && last_msg.map(|t| t.elapsed().as_secs() >= 10).unwrap_or(true) {
                    say!("{m}");
                    last_msg = Some(Instant::now());
                }
            }
        }

        let sc = *screen.lock().unwrap();
        // drain band/chord messages from the audio thread (verbose only)
        if let Some(rx) = &msgs {
            while let Ok(m) = rx.try_recv() {
                if cfg.verbose {
                    say!("{m}");
                }
            }
        }
        if tick % 2 == 0 {
            // normalised, lightly smoothed signals; the bandleader interprets them
            let raw_net = if net_r < 8192.0 { 0.0 } else { ((net_r / 8192.0).log2() / 12.6).clamp(0.0, 1.0) as f32 };
            let wl = if wr_r < 256.0 * 1024.0 { 0.0 } else { ((wr_r / 262144.0).log2() / 7.0).clamp(0.0, 1.0) as f32 };
            let fl = if fd_r < 6.0 { 0.0 } else { ((fd_r / 6.0).log2() / 7.0).clamp(0.0, 1.0) as f32 };
            let raw_score = if sc.valid { sc.warmth * 0.7 + (sc.brightness - 0.5) * 0.6 } else { 0.0 };
            let ema = |e: &mut f32, x: f32, tau: f32| {
                if started { *e += (x - *e) * (1.0 - (-1.0 / tau).exp()) } else { *e = x }
            };
            let ncores = cpus.len().saturating_sub(1).max(1) as f32;
            ema(&mut cpu_e, cpu_s.powf(0.7), 3.0);
            ema(&mut cores_e, cores_busy as f32 / ncores, 3.0);
            ema(&mut net_e, raw_net, 3.0);
            ema(&mut file_e, (wl.max(fl) + 0.25 * wl.min(fl)).min(1.0), 3.0);
            ema(&mut fd_e, fd_r as f32, 4.0);
            ema(&mut score_e, raw_score, 30.0);
            // how much the screen itself changed (static screen -> 0)
            screen_change = (screen_change * 0.85 + (raw_score - prev_raw).abs() * 0.5).min(1.0);
            prev_raw = raw_score;
            started = true;
            let act = input.as_mut().map(|t| t.poll()).unwrap_or_default();
            c.sig = crate::band::Signals { cpu: cpu_e, cores: cores_e, net: net_e, file: file_e, fd: fl, screen_change, keyboard: act.keyboard, mouse: act.mouse };
            c.warmth = score_e.clamp(-1.0, 1.0);
            c.brightness = sc.brightness;
            c.screen_valid = sc.valid;
            c.click_rate = if cfg.clicks <= 0.0 || fd_e < 4.0 { 0.0 } else { ((fd_e / 4.0).log2() * 0.6).min(3.0) };
            c.leads = [lead.as_ref().map(|l| l.0.clone()), None, None];
            c.version += 1;
            *ctl.lock().unwrap() = c.clone();

            if cfg.verbose && tick % 10 == 0 {
                let scr = if sc.valid {
                    format!("hue {:.0} bri {:.2} warm {:+.2}", sc.hue, sc.brightness, sc.warmth)
                } else {
                    "off".into()
                };
                say!(
                    "[{:5.0}s] cpu {:3.0}% cores {}/{} | net {} | write {} | fds {:.0}/s | keys {:.2} mouse {:.2} | screen {} | signals cpu {:.2} net {:.2} file {:.2} | guest: {}",
                    start.elapsed().as_secs_f32(),
                    cpu_s * 100.0,
                    cores_busy,
                    cpus.len().saturating_sub(1),
                    human(net_r),
                    human(wr_r),
                    fd_r,
                    act.keyboard,
                    act.mouse,
                    scr,
                    cpu_e,
                    net_e,
                    file_e,
                    lead.as_ref().map(|l| format!("{} ({:.0}%)", l.0.name, l.0.cpu * 100.0)).unwrap_or("-".into())
                );
            }
        }
        tick += 1;
        let el = now.elapsed();
        if el < tickd {
            std::thread::sleep(tickd - el);
        }
    }
}
