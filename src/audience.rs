//! The audience: the user's own processes as a little crowd on the floor below the band.
//!
//! Monitor side (`Census`): piggybacks on the monitor's existing /proc/<pid>/stat walk
//! (every 1.5 s). Only processes owned by the user are candidates; kernel threads and other
//! users' processes are only counted, never named. For the few people actually in the crowd
//! (≤ 16) it reads /proc/<pid>/io, and every few seconds it peeks at one member's open files
//! (/proc/<pid>/fd links) to borrow a file *basename* for its name tag. Nothing is stored.
//!
//! UI side: `Floor` walks, dances and sends the little people to the bar; `fig` draws one
//! 5-cell-wide, 3-row figure from ASCII only (checked by the width test).

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::time::Instant;

pub static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// panel shown (the `a` key)
pub static SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub const CAP: usize = 16;

#[derive(Clone, Default, Debug)]
pub struct Member {
    pub pid: i32,
    /// comm, printable ASCII only
    pub name: String,
    /// fraction of one core (smoothed)
    pub cpu: f32,
    /// storage reads + writes, bytes/s
    pub disk: f32,
    /// other I/O (sockets, pipes, ttys, page cache), bytes/s
    pub chat: f32,
    /// seconds without noticeable CPU
    pub idle: f32,
    /// basename of a file it has open (own processes only, refreshed rarely)
    pub file: Option<String>,
    /// exited (or rotated out): walks out
    pub gone: bool,
    /// it appeared after sysjam started (a fresh spawn)
    pub fresh: bool,
}

#[derive(Clone, Default)]
pub struct CrowdSnap {
    pub on: bool,
    pub members: Vec<Member>,
    /// kernel threads (backstage, never named)
    pub kthreads: usize,
    /// other users' processes (only counted)
    pub others: usize,
    /// all of the user's own processes
    pub own: usize,
}

/// one process from the monitor's /proc walk
pub struct Proc<'a> {
    pub pid: i32,
    pub ppid: i32,
    pub comm: &'a str,
    /// None on its first sighting
    pub cpu: Option<f32>,
}

struct Track {
    m: Member,
    io: Option<(u64, u64)>,
    gone_t: f32,
}

pub struct Census {
    uid: u32,
    uids: HashMap<i32, u32>,
    ema: HashMap<i32, (f32, f32, f32)>, // pid -> (cpu ema, idle seconds, age seconds; age < 0: there before us)
    tracks: Vec<Track>,
    files_t: f32,
    files_rr: usize,
    rot_t: f32,
    first: bool,
    rng: u64,
    buf: String,
    /// a notable process just walked in (for the band to nod to), drained by the monitor
    pub walkin: Option<String>,
    walkin_cd: f32,
    /// fresh spawns get in at most one per few seconds (shells and tickers spawn a lot)
    fresh_cd: f32,
}

fn clean(comm: &str) -> String {
    let s: String = comm.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').map(|c| if c == ' ' { '-' } else { c }).collect();
    if s.is_empty() { "?".into() } else { s }
}

/// big names the band notices walking in
const NOTABLE: &[&str] = &[
    "firefox", "chrome", "chromium", "code", "steam", "blender", "gimp", "cargo", "rustc", "gcc", "cc1", "cc1plus", "clang", "make",
    "python3", "node", "java", "docker", "obs", "thunderbird", "slack", "discord", "spotify", "vlc", "mpv", "ffmpeg", "git", "zoom",
];

fn read_io(pid: i32, buf: &mut String) -> Option<(u64, u64)> {
    buf.clear();
    use std::io::Read;
    fs::File::open(format!("/proc/{pid}/io")).ok()?.read_to_string(buf).ok()?;
    let (mut rc, mut wc, mut rb, mut wb) = (0u64, 0u64, 0u64, 0u64);
    for l in buf.lines() {
        let mut it = l.split(": ");
        let (k, v) = (it.next().unwrap_or(""), it.next().unwrap_or("0").trim().parse::<u64>().unwrap_or(0));
        match k {
            "rchar" => rc = v,
            "wchar" => wc = v,
            "read_bytes" => rb = v,
            "write_bytes" => wb = v,
            _ => {}
        }
    }
    Some((rc + wc, rb + wb))
}

/// a file basename the process has open: regular files only, nothing under /dev, /proc,
/// /sys or /run, no shared libraries, fonts or caches; at most 48 fds looked at
fn open_file(pid: i32) -> Option<String> {
    let rd = fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    let mut best: Option<String> = None;
    for e in rd.flatten().take(48) {
        let Ok(p) = fs::read_link(e.path()) else { continue };
        let s = p.to_string_lossy();
        if !s.starts_with('/') || s.contains("(deleted)") {
            continue;
        }
        if ["/dev/", "/proc/", "/sys/", "/run/", "/memfd:", "/tmp/.X", "/usr/", "/var/cache/"].iter().any(|x| s.starts_with(x)) {
            continue;
        }
        let base = s.rsplit('/').next().unwrap_or("");
        let low = base.to_ascii_lowercase();
        if base.is_empty() || base.starts_with('.') || low.contains(".so") || low.ends_with(".ttf") || low.ends_with(".otf") || low.ends_with(".cache") || low.ends_with("-journal") || low.ends_with(".lock") || low == "log" {
            continue;
        }
        if !base.chars().all(|c| c.is_ascii_graphic()) {
            continue;
        }
        best = Some(base.chars().take(16).collect());
    }
    best
}

impl Census {
    pub fn new() -> Census {
        let uid = unsafe { libc::getuid() };
        Census {
            uid,
            uids: HashMap::new(),
            ema: HashMap::new(),
            tracks: Vec::new(),
            files_t: 3.0,
            files_rr: 0,
            rot_t: 0.0,
            first: true,
            rng: 0x9E3779B97F4A7C15 ^ std::process::id() as u64,
            buf: String::with_capacity(512),
            walkin: None,
            walkin_cd: 8.0,
            fresh_cd: 0.0,
        }
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    fn uid_of(&mut self, pid: i32) -> Option<u32> {
        if let Some(u) = self.uids.get(&pid) {
            return Some(*u);
        }
        let u = fs::metadata(format!("/proc/{pid}")).ok()?.uid();
        self.uids.insert(pid, u);
        Some(u)
    }

    /// one census after each /proc walk; `dt` seconds since the last
    pub fn update(&mut self, procs: &[Proc], dt: f32) -> CrowdSnap {
        let mut snap = CrowdSnap { on: true, ..Default::default() };
        let mut cands: Vec<(i32, String, f32, bool)> = Vec::new(); // pid, name, score, fresh
        let mut alive: HashMap<i32, (f32, f32, f32)> = HashMap::with_capacity(procs.len());
        let mut live_uids: HashMap<i32, u32> = HashMap::with_capacity(procs.len());
        for p in procs {
            if p.pid == 2 || p.ppid == 2 {
                snap.kthreads += 1;
                continue;
            }
            let Some(u) = self.uid_of(p.pid) else { continue };
            live_uids.insert(p.pid, u);
            if u != self.uid {
                snap.others += 1;
                continue;
            }
            snap.own += 1;
            let cpu = p.cpu.unwrap_or(0.0);
            let born = if self.first { -1.0 } else { 0.0 };
            let (e0, i0, a0) = self.ema.get(&p.pid).copied().unwrap_or((cpu, 0.0, born));
            let e = 0.6 * e0 + 0.4 * cpu;
            let idle = if cpu < 0.01 { i0 + dt } else { 0.0 };
            let age = if a0 < 0.0 { a0 } else { a0 + dt };
            alive.insert(p.pid, (e, idle, age));
            // a fresh spawn stays "fresh" for its first 12 s, so it can still get in
            let fresh = age >= 0.0 && age < 12.0;
            let notable = NOTABLE.contains(&p.comm);
            let score = e * 3.0 + if fresh { 2.0 + if notable { 1.0 } else { 0.0 } } else { 0.0 } + 0.03;
            cands.push((p.pid, clean(p.comm), score, fresh));
        }
        self.ema = alive;
        self.uids = live_uids;
        let first = self.first;
        self.first = false;
        self.walkin_cd -= dt;
        self.fresh_cd -= dt;

        // members: exits walk out, the rest get fresh numbers
        let mut buf = std::mem::take(&mut self.buf);
        for t in self.tracks.iter_mut() {
            if t.m.gone {
                t.gone_t += dt;
                continue;
            }
            match self.ema.get(&t.m.pid) {
                None => {
                    t.m.gone = true;
                    t.gone_t = 0.0;
                }
                Some(&(e, idle, _)) => {
                    t.m.cpu = e;
                    t.m.idle = idle;
                    if let Some(now) = read_io(t.m.pid, &mut buf) {
                        if let Some(prev) = t.io {
                            let disk = now.1.saturating_sub(prev.1) as f32 / dt.max(0.1);
                            let all = now.0.saturating_sub(prev.0) as f32 / dt.max(0.1);
                            t.m.disk = 0.5 * t.m.disk + 0.5 * disk;
                            t.m.chat = 0.5 * t.m.chat + 0.5 * (all - disk).max(0.0);
                        }
                        t.io = Some(now);
                    }
                }
            }
        }
        self.buf = buf;
        self.tracks.retain(|t| !(t.m.gone && t.gone_t > 6.0));

        // fill the floor: fresh spawns first, then a CPU-weighted lottery
        let present = |tr: &Vec<Track>, pid: i32| tr.iter().any(|t| t.m.pid == pid && !t.m.gone);
        let mut pool: Vec<(i32, String, f32, bool)> = cands.into_iter().filter(|c| !present(&self.tracks, c.0)).collect();
        // one fresh spawn per few seconds, and not a second copy of a name already in
        let mut let_fresh = self.fresh_cd <= 0.0;
        pool.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
        for c in pool.iter_mut() {
            if !c.3 {
                continue;
            }
            let dup = self.tracks.iter().any(|t| !t.m.gone && t.m.name == c.1);
            if dup || !let_fresh {
                c.3 = false;
                c.2 = (c.2 - 2.0).max(0.03);
            } else {
                let_fresh = false;
            }
        }
        let active = self.tracks.iter().filter(|t| !t.m.gone).count();
        let want = if first { 8.min(CAP) } else { CAP };
        let mut room = want.saturating_sub(active);
        // a full floor makes way for fresh spawns and busy processes: the idlest go home
        let urgent = pool.iter().filter(|c| c.3 || c.2 > 1.0).count().min(2);
        for _ in room..urgent {
            if let Some(t) = self.tracks.iter_mut().filter(|t| !t.m.gone && !t.m.fresh && t.m.cpu < 0.05).max_by(|a, b| a.m.idle.partial_cmp(&b.m.idle).unwrap()) {
                t.m.gone = true;
                t.gone_t = 0.0;
                room += 1;
            }
        }
        // newcomers trickle in, a couple per census (they walk in one by one)
        let mut per_round = if first { 8 } else { 3 };
        while room > 0 && per_round > 0 && !pool.is_empty() {
            let best_fresh = pool.iter().enumerate().filter(|(_, c)| c.3).max_by(|a, b| a.1 .2.partial_cmp(&b.1 .2).unwrap()).map(|(i, _)| i);
            let i = match best_fresh {
                Some(i) => i,
                None => {
                    let tot: f32 = pool.iter().map(|c| c.2).sum();
                    let mut r = self.rand() * tot;
                    let mut k = 0;
                    for (j, c) in pool.iter().enumerate() {
                        r -= c.2;
                        k = j;
                        if r <= 0.0 {
                            break;
                        }
                    }
                    k
                }
            };
            let (pid, name, _, fresh) = pool.swap_remove(i);
            if fresh {
                self.fresh_cd = 4.0;
            }
            let (cpu, idle, _) = self.ema.get(&pid).copied().unwrap_or((0.0, 0.0, 0.0));
            if !first && self.walkin_cd <= 0.0 && (cpu > 0.3 || NOTABLE.contains(&name.as_str())) {
                self.walkin = Some(format!("{name}:{pid}"));
                self.walkin_cd = 30.0;
            }
            self.tracks.push(Track { m: Member { pid, name, cpu, idle, fresh, ..Default::default() }, io: None, gone_t: 0.0 });
            room -= 1;
            per_round -= 1;
        }

        // rotation: now and then someone idle goes home so a busier (or random) one can come
        self.rot_t += dt;
        if self.rot_t >= 12.0 && !pool.is_empty() {
            self.rot_t = 0.0;
            let busy_waiting = pool.iter().any(|c| c.2 > 0.5);
            let full = self.tracks.iter().filter(|t| !t.m.gone).count() >= CAP;
            if busy_waiting || full || self.rand() < 0.5 {
                if let Some(t) = self.tracks.iter_mut().filter(|t| !t.m.gone && !t.m.fresh && t.m.idle > 20.0).max_by(|a, b| a.m.idle.partial_cmp(&b.m.idle).unwrap()) {
                    t.m.gone = true;
                    t.gone_t = 0.0;
                }
            }
        }

        // a peek at one member's open files every ~6 s (own processes only)
        self.files_t -= dt;
        if self.files_t <= 0.0 && !self.tracks.is_empty() {
            self.files_t = 6.0;
            self.files_rr = (self.files_rr + 1) % self.tracks.len();
            let pid = self.tracks[self.files_rr].m.pid;
            if !self.tracks[self.files_rr].m.gone && self.uids.get(&pid) == Some(&self.uid) {
                let f = open_file(pid);
                if f.is_some() {
                    self.tracks[self.files_rr].m.file = f;
                }
            }
        }
        snap.members = self.tracks.iter().map(|t| t.m.clone()).collect();
        snap
    }

    /// crowd size and how lively it is, for the band (0..1 each)
    pub fn liveliness(s: &CrowdSnap) -> (f32, f32) {
        let here: Vec<&Member> = s.members.iter().filter(|m| !m.gone).collect();
        if here.is_empty() {
            return (0.0, 0.0);
        }
        let act = here.iter().map(|m| (m.cpu * 2.0 + if m.chat > 2e4 { 0.3 } else { 0.0 } + if m.disk > 2e5 { 0.3 } else { 0.0 }).min(1.0)).sum::<f32>() / here.len() as f32;
        ((here.len() as f32 / 10.0).min(1.0), act)
    }
}

// ------------------------------------------------------------------ the floor (UI)

/// figure width and the slot each one stands in
pub const FW: usize = 5;
pub const SLOT: usize = 10;
pub const BARW: usize = 8;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    Enter,
    Here,
    ToBar,
    Bar,
    Back,
    Wave,
    Leave,
}

/// how the whole crowd feels right now (from the band)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Vibe {
    /// band alive / surge / delighted conductor: cheer, hands up, jump on the beat
    Hype,
    /// a like just landed: applause
    Applause,
    /// chilling: mingle and nod
    Chill,
    Normal,
}

/// what one figure is doing this frame (drives `fig`)
#[derive(Clone, Copy, Debug)]
pub struct Look<'a> {
    pub style: &'a str,
    pub act: Act,
    pub vibe: Vibe,
    pub beats: f64,
    pub t: f32,
    pub cpu: f32,
    pub disk: bool,
    pub chat: bool,
    pub idle: f32,
    pub step: i32,
    pub beer: bool,
    pub seed: u32,
}

/// three rows of exactly FW cells, plus a vertical lift (1 = jumping)
pub fn fig(l: &Look) -> ([String; 3], bool) {
    let beat = l.beats.floor() as i64;
    let frac = l.beats.fract() as f32;
    let odd = beat.rem_euclid(2) == 1;
    let st = l.style;
    let hat = match st {
        "country" => " _n_ ",
        "baroque" => " @@@ ",
        "jazz" => " _=_ ",
        "biker" => " ___ ",
        "monk" => "  .  ",
        _ => "     ",
    };
    let walk_legs = if l.step.rem_euclid(2) == 0 { " / | " } else { " | \\ " };
    let mut r0 = hat.to_string();
    let mut r1 = " /o\\ ".to_string();
    let mut r2 = " / \\ ".to_string();
    let mut lift = false;
    match l.act {
        Act::Enter | Act::Back | Act::ToBar | Act::Leave => {
            r2 = walk_legs.into();
            if l.disk {
                r0 = " [#] ".into();
                r1 = " \\o/ ".into();
            }
            if l.act == Act::Leave && l.step.rem_euclid(4) < 2 {
                r1 = " /o/ ".into();
            }
        }
        Act::Wave => {
            r1 = if (l.t * 4.0) as i32 % 2 == 0 { " /o/ " } else { " /o| " }.into();
        }
        Act::Bar => {
            let sip = ((l.t + l.seed as f32 * 0.37) % 3.0) < 0.8;
            let mug = if l.beer { 'U' } else { 'c' };
            if sip {
                r0 = format!("{}{mug} ", &hat[..3]);
                r1 = " /o/ ".into();
            } else {
                r1 = format!(" /o{mug} ");
            }
        }
        Act::Here => match l.vibe {
            Vibe::Applause => {
                r1 = if (l.t * 6.0) as i32 % 2 == 0 { " >o< " } else { " -o- " }.into();
            }
            Vibe::Hype => match st {
                "country" => {
                    // line dance: everyone steps the same way on the beat
                    r1 = if odd { " /o- " } else { " -o\\ " }.into();
                    r2 = if odd { " /|  " } else { "  |\\ " }.into();
                }
                "baroque" => {
                    // polite golf clap
                    r1 = if frac < 0.2 { " .o. " } else { " /o\\ " }.into();
                }
                "blues" => {
                    r0 = if (l.t * 5.0) as i32 % 2 == 0 { " i   " } else { " '   " }.into();
                    r1 = " \\o\\ ".into();
                }
                _ => {
                    r1 = " \\o/ ".into();
                    if st == "techno" {
                        r0 = if odd { " * * " } else { " + + " }.into();
                    }
                    let every = l.cpu > 0.3 || (l.seed % 3 == 0);
                    lift = frac < 0.3 && (every || odd);
                }
            },
            Vibe::Chill if !l.disk && l.cpu <= 0.5 => {
                // mingle and nod along
                r1 = if frac < 0.35 { " |o| " } else { " /o\\ " }.into();
                if l.chat {
                    r0 = "   ..".into();
                }
            }
            Vibe::Normal | Vibe::Chill => {
                if l.disk {
                    r0 = " [#] ".into();
                    r1 = " \\o/ ".into();
                } else if l.cpu > 0.5 {
                    // dancing hard
                    let half = (l.beats * 2.0).floor() as i64 % 2 == 0;
                    r1 = if half { " \\o/ " } else { " /o\\ " }.into();
                    r2 = if half { " / \\ " } else { " < > " }.into();
                } else if l.cpu > 0.08 {
                    r1 = if odd { " \\o  " } else { "  o/ " }.into();
                } else if l.idle > 40.0 {
                    // sitting it out
                    r1 = "  o  ".into();
                    r2 = " _/\\_".into();
                    r0 = if (l.t as i32 + l.seed as i32) % 4 < 2 { "    z" } else { "   z " }.into();
                } else if l.chat {
                    r0 = "   ..".into();
                } else {
                    // a slow sway
                    r1 = if beat.rem_euclid(4) < 2 { " /o\\ " } else { " |o\\ " }.into();
                }
            }
        },
    }
    ([r0, r1, r2], lift)
}

/// the bar at the right end of the floor: 3 rows of BARW-1 cells (emoji width 2) + a label
pub fn bar_rows(ascii: bool) -> [String; 4] {
    if ascii {
        ["+-----+".into(), "|U   c|".into(), "+-----+".into(), "  bar  ".into()]
    } else {
        ["┌─────┐".into(), "│🍺 ☕│".into(), "┴─────┴".into(), "  bar  ".into()]
    }
}

/// a name tag, at most `w` cells: "firefox:4123" → "fire:4123"
pub fn tag(name: &str, pid: i32, w: usize) -> String {
    let p = format!(":{pid}");
    let room = w.saturating_sub(p.len()).max(2);
    let n: String = name.chars().take(room).collect();
    let s = format!("{n}{p}");
    s.chars().take(w).collect()
}

/// UI memory for each figure on the floor
pub struct Person {
    pub m: Member,
    pub x: f32,
    pub slot: usize,
    pub act: Act,
    pub t_act: f32,
    pub step: i32,
    pub beer: bool,
    pub seed: u32,
    pub stay: f32,
}

pub struct Floor {
    pub people: Vec<Person>,
    /// sent home to make room: not back for a while
    benched: HashMap<i32, f32>,
    evict_t: f32,
    last: Option<Instant>,
    pub t: f32,
    rng: u64,
}

impl Floor {
    pub fn new() -> Floor {
        Floor { people: Vec::new(), benched: HashMap::new(), evict_t: 0.0, last: None, t: 0.0, rng: 0xA5A5_1234_5678_9ABC }
    }
    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// advance everyone to now on a floor `fw` cells wide (bar excluded)
    pub fn step(&mut self, snap: &CrowdSnap, fw: usize, vibe: Vibe) {
        let now = Instant::now();
        let dt = self.last.map(|l| now.duration_since(l).as_secs_f32()).unwrap_or(0.0).min(0.5);
        self.last = Some(now);
        self.t += dt;
        let nslots = (fw / SLOT).clamp(1, CAP);
        // new data: arrivals, departures, fresh numbers
        for m in &snap.members {
            match self.people.iter_mut().find(|p| p.m.pid == m.pid) {
                Some(p) => {
                    let was_gone = p.m.gone;
                    p.m = m.clone();
                    if m.gone && !was_gone && !matches!(p.act, Act::Wave | Act::Leave) {
                        p.act = if p.x < -1.0 || p.x > fw as f32 { Act::Leave } else { Act::Wave };
                        p.t_act = 0.0;
                    }
                }
                None if !m.gone => {
                    if self.benched.get(&m.pid).map_or(false, |&u| u > self.t) {
                        continue;
                    }
                    let used: Vec<usize> = self.people.iter().filter(|p| !matches!(p.act, Act::Wave | Act::Leave)).map(|p| p.slot).collect();
                    let mut free: Vec<usize> = (0..nslots).filter(|s| !used.contains(s)).collect();
                    if free.is_empty() && (m.fresh || m.cpu > 0.3 || m.disk > 2e5) && self.evict_t <= self.t {
                        // a newcomer worth seeing: the idlest one at home goes home
                        let t = self.t;
                        if let Some(p) = self.people.iter_mut().filter(|p| p.act == Act::Here && !p.m.fresh && p.m.cpu < 0.05 && p.t_act > 4.0).max_by(|a, b| a.m.idle.partial_cmp(&b.m.idle).unwrap()) {
                            p.act = Act::Wave;
                            p.t_act = 0.0;
                            free.push(p.slot);
                            self.benched.insert(p.m.pid, t + 25.0);
                            self.evict_t = t + 3.0;
                        }
                    }
                    if free.is_empty() {
                        continue;
                    }
                    let r = self.rand();
                    let slot = free[((r * free.len() as f32) as usize).min(free.len() - 1)];
                    let from_left = (slot as f32) < nslots as f32 * 0.5;
                    let seed = (self.rand() * 1000.0) as u32;
                    let beer = self.rand() < 0.6;
                    self.people.push(Person { m: m.clone(), x: if from_left { -(FW as f32) } else { fw as f32 }, slot, act: Act::Enter, t_act: 0.0, step: 0, beer, seed, stay: 0.0 });
                }
                None => {}
            }
        }
        // people whose process vanished from the census entirely: leave too
        for p in self.people.iter_mut() {
            if !snap.members.iter().any(|m| m.pid == p.m.pid) && !matches!(p.act, Act::Wave | Act::Leave) {
                p.act = Act::Wave;
                p.t_act = 0.0;
            }
        }
        let at_bar = self.people.iter().filter(|p| matches!(p.act, Act::ToBar | Act::Bar)).count();
        let mut bar_q = 0usize;
        let mut want_bar: Option<usize> = None;
        let n = self.people.len();
        for i in 0..n {
            let r = self.rand();
            let p = &mut self.people[i];
            p.t_act += dt;
            let home = (p.slot * SLOT + (SLOT - FW) / 2) as f32;
            let speed = if p.m.fresh && p.m.cpu > 0.05 { 16.0 } else { 9.0 };
            let mut go = |p: &mut Person, target: f32| -> bool {
                let d = target - p.x;
                let s = speed * dt;
                let old = p.x.round() as i32;
                if d.abs() <= s {
                    p.x = target;
                } else {
                    p.x += s * d.signum();
                }
                if p.x.round() as i32 != old {
                    p.step += 1;
                }
                (p.x - target).abs() < 0.01
            };
            match p.act {
                Act::Enter | Act::Back => {
                    if go(p, home) {
                        p.act = Act::Here;
                        p.t_act = 0.0;
                    }
                }
                Act::Here => {
                    // idle ones wander off to the bar now and then (not while the band is on fire)
                    if vibe != Vibe::Hype && p.m.idle > 12.0 && p.t_act > 8.0 && at_bar < 2 && want_bar.is_none() && r < dt / 15.0 {
                        want_bar = Some(i);
                    }
                }
                Act::ToBar => {
                    let spot = fw as f32 - FW as f32 - 1.0 - 3.0 * bar_q as f32;
                    bar_q += 1;
                    if go(p, spot) {
                        p.act = Act::Bar;
                        p.t_act = 0.0;
                        p.stay = 6.0 + 8.0 * r;
                    }
                }
                Act::Bar => {
                    bar_q += 1;
                    if p.t_act > p.stay || vibe == Vibe::Hype {
                        p.act = Act::Back;
                        p.t_act = 0.0;
                    }
                }
                Act::Wave => {
                    if p.t_act > 1.0 {
                        p.act = Act::Leave;
                        p.t_act = 0.0;
                    }
                }
                Act::Leave => {
                    let exit = if p.x < fw as f32 * 0.5 { -(FW as f32) - 1.0 } else { fw as f32 + 1.0 };
                    go(p, exit);
                }
            }
        }
        if let Some(i) = want_bar {
            self.people[i].act = Act::ToBar;
            self.people[i].t_act = 0.0;
        }
        self.people.retain(|p| !(p.act == Act::Leave && (p.x < -(FW as f32) || p.x > fw as f32)));
        let t = self.t;
        self.benched.retain(|_, u| *u > t);
        // a narrower terminal: whoever no longer has a slot steps out
        for p in self.people.iter_mut() {
            if p.slot >= nslots && !matches!(p.act, Act::Wave | Act::Leave) {
                p.act = Act::Leave;
            }
        }
    }

    pub fn at_bar(&self) -> usize {
        self.people.iter().filter(|p| p.act == Act::Bar).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn crowd_figures_are_clean_and_fixed_width() {
        let acts = [Act::Enter, Act::Here, Act::ToBar, Act::Bar, Act::Back, Act::Wave, Act::Leave];
        let vibes = [Vibe::Hype, Vibe::Applause, Vibe::Chill, Vibe::Normal];
        for st in crate::style::STYLES {
            for &act in &acts {
                for &vibe in &vibes {
                    for k in 0..16 {
                        let l = Look { style: st.id, act, vibe, beats: k as f64 * 0.37, t: k as f32 * 0.29, cpu: (k % 4) as f32 * 0.25, disk: k % 5 == 0, chat: k % 3 == 0, idle: (k * 5) as f32, step: k as i32, beer: k % 2 == 0, seed: k as u32 };
                        let (rows, _) = fig(&l);
                        for r in rows.iter() {
                            assert_eq!(r.width(), FW, "{} {act:?} {vibe:?}: {r:?}", st.id);
                            assert_eq!(r.chars().count(), FW);
                            assert!(r.chars().all(|c| (' '..='~').contains(&c)), "non-ASCII in {r:?}");
                        }
                    }
                }
            }
        }
        for a in [true, false] {
            for r in bar_rows(a) {
                assert_eq!(r.width(), BARW - 1, "{r:?}");
            }
        }
        assert_eq!(tag("firefox", 4123, 9), "fire:4123");
        assert_eq!(tag("sh", 7, 9), "sh:7");
    }
}
