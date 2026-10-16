//! The audience: the user's own processes as a little crowd on the floor below the band.
//!
//! Monitor side (`Census`): piggybacks on the monitor's existing /proc/<pid>/stat walk
//! (every 1.5 s). Only processes owned by the user are candidates; kernel threads and other
//! users' processes are only counted, never named. For the few people actually in the crowd
//! (≤ 48, as many as the floor holds) it reads /proc/<pid>/io, and every few seconds it peeks at one member's open files
//! (/proc/<pid>/fd links) to borrow a file *basename* for its name tag. Nothing is stored.
//!
//! UI side: `Floor` walks, dances and sends the little people to the bar; `fig` draws one
//! 5-cell-wide, 3-row figure from ASCII only (checked by the width test).

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::time::Instant;

pub static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// panel on screen right now (decided by the UI from the terminal size, or forced)
pub static SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// the `a` key: 0 = automatic (only on a big enough terminal), 1 = forced on, 2 = forced off
pub static FORCE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// hard ceiling on the crowd (the census reads /proc/<pid>/io for each one)
pub const CAP: usize = 48;
/// how many the floor on screen can hold right now (set by the UI from its stacked rows)
pub static WANT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(16);
fn want_cap() -> usize {
    WANT.load(std::sync::atomic::Ordering::Relaxed).clamp(1, CAP)
}

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
        let want = if first { 8.min(want_cap()) } else { want_cap() };
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
            let full = self.tracks.iter().filter(|t| !t.m.gone).count() >= want_cap();
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
pub const FW: usize = 7;
pub const SLOT: usize = 12;
pub const BARW: usize = 8;
/// rows of one figure (hair/hat, head, body, knees, feet)
pub const FH: usize = 5;

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
    /// hash of the exe name: the same program always looks and dances the same
    pub id: u32,
    /// the band is surging (jumps, stomps, kicks)
    pub surge: bool,
    /// a big sing-along chorus (Biker): fists up
    pub chorus: bool,
}

/// FNV-1a of the exe name
pub fn name_id(name: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h ^ (h >> 15)
}

/// a person's fixed looks and habits, all from the name hash
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Persona {
    pub hair: usize,
    pub top: usize,
    /// 0 none, 1 headphones, 2 earring, 3 scarf, 4 bag, 5 flower
    pub acc: usize,
    pub tall: bool,
    /// 0 bounce, 1 sway, 2 two-step, 3 headbang, 4 robot, 5 twist, 6 kicker
    pub dance: usize,
    /// wears the style's hat (half the crowd dresses up)
    pub themed: bool,
}

pub fn persona(id: u32) -> Persona {
    let b = |s: u32, n: u32| ((id >> s) % n) as usize;
    Persona { hair: b(0, HAIR.len() as u32), top: b(5, TOPS.len() as u32), acc: b(9, 6), tall: (id >> 13) & 3 != 0, dance: b(16, 7), themed: (id >> 20) & 1 == 1 }
}

const HAIR: [&str; 11] = ["  ___  ", "  ,,,  ", " ~~~~~ ", "  ^^^  ", "  (@)  ", "  ===  ", "  ///  ", "  .-.  ", "  ~~~  ", "  ___> ", "  \\\\\\  "];
const TOPS: [&str; 10] = ["[#]", "{=}", "(:)", "|v|", "<+>", "[=]", "(~)", "[*]", "{o}", "|:|"];

/// legs: (knees, feet), 7 cells each
mod legs {
    pub const STAND: [&str; 2] = ["  | |  ", " _| |_ "];
    pub const TAP_R: [&str; 2] = ["  | |  ", " _| |/ "];
    pub const TAP_L: [&str; 2] = ["  | |  ", " \\| |_ "];
    pub const BOUNCE: [&str; 2] = ["  > <  ", " _/ \\_ "];
    pub const STEP_R: [&str; 2] = ["  | \\  ", " _|  \\_"];
    pub const STEP_L: [&str; 2] = ["  / |  ", " _/  |_"];
    pub const PASS: [&str; 2] = ["  | |  ", "  |_|_ "];
    pub const CROSS: [&str; 2] = ["   X   ", "  / \\_ "];
    pub const KICK_R: [&str; 2] = ["  |__/ ", " _|    "];
    pub const KICK_L: [&str; 2] = [" \\__|  ", "    |_ "];
    pub const KNEE_R: [&str; 2] = ["  |_,  ", " _|    "];
    pub const KNEE_L: [&str; 2] = ["  ,_|  ", "    |_ "];
    pub const STOMP_R: [&str; 2] = ["  | |  ", " _| |_*"];
    pub const STOMP_L: [&str; 2] = ["  | |  ", "*_| |_ "];
    pub const HEEL_R: [&str; 2] = ["  | |  ", " _| |\\ "];
    pub const SHUF_A: [&str; 2] = ["  / \\  ", " _/ \\_ "];
    pub const SHUF_B: [&str; 2] = ["  \\ /  ", " _\\ /_ "];
    pub const POINT: [&str; 2] = ["  | \\  ", " _|  '-"];
    pub const TOES: [&str; 2] = ["  | |  ", "  ^ ^  "];
    pub const TUCK: [&str; 2] = ["  > <  ", "  ' '  "];
    pub const SIT: [&str; 2] = ["  |_   ", " _/ \\_ "];
}

/// footwork for this instant: (legs, lift = both feet off the ground, x shift)
fn footwork(l: &Look, p: &Persona, hype: bool) -> ([&'static str; 2], bool, i32) {
    use legs::*;
    let b = l.beats;
    let beat = b.floor() as i64;
    let frac = b.fract() as f32;
    let half = (b * 2.0).floor() as i64; // eighths
    let on = frac < 0.22;
    if l.surge {
        // surge: jumps with both feet off the ground, kicks, stomps, knee bounces
        return match (beat.rem_euclid(4), p.dance % 3) {
            (_, 0) => if on { (TUCK, true, 0) } else { (BOUNCE, false, 0) },
            (0 | 2, 1) => if on { (STOMP_R, false, 0) } else { (BOUNCE, false, 0) },
            (_, 1) => if on { (STOMP_L, false, 0) } else { (BOUNCE, false, 0) },
            (k, _) => if half.rem_euclid(2) == 0 { (if k % 2 == 0 { KICK_R } else { KICK_L }, false, 0) } else { (TUCK, frac > 0.5 && frac < 0.75, 0) },
        };
    }
    if hype {
        // per-style footwork
        return match l.style {
            // line-dance grapevine (step, cross behind, step, touch; and back) + heel-toe
            "country" => {
                let k = beat.rem_euclid(8);
                match k {
                    0 => (STEP_R, false, 1),
                    1 => (CROSS, false, 1),
                    2 => (STEP_R, false, 2),
                    3 => (if on { TAP_L } else { STAND }, false, 2),
                    4 => (HEEL_R, false, 1),
                    5 => (TAP_R, false, 1),
                    6 => (STEP_L, false, 0),
                    _ => (if on { TAP_R } else { STAND }, false, 0),
                }
            }
            // shuffle (heels in/out on the eighths) and the running man (knees up)
            "techno" | "house" => {
                if p.dance % 2 == 0 {
                    (if half.rem_euclid(2) == 0 { SHUF_A } else { SHUF_B }, false, 0)
                } else {
                    (match half.rem_euclid(4) { 0 => KNEE_R, 1 => PASS, 2 => KNEE_L, _ => PASS }, false, 0)
                }
            }
            // swing step: side step-touch, rock on 2 and 4
            "blues" | "jazz" => match beat.rem_euclid(4) {
                0 => (STEP_R, false, 1),
                1 => (if on { TAP_L } else { BOUNCE }, false, 1),
                2 => (STEP_L, false, 0),
                _ => (if on { TAP_R } else { BOUNCE }, false, 0),
            },
            // minuet: point the toe, rise on the toes, a small step
            "baroque" => match beat.rem_euclid(6) {
                0 | 3 => (POINT, false, 0),
                1 | 4 => (TOES, false, 0),
                2 => (STEP_R, false, 1),
                _ => (STEP_L, false, 0),
            },
            // biker stomp: heavy alternate stomps on every beat
            "biker" | "industrial" => (if on { if beat % 2 == 0 { STOMP_R } else { STOMP_L } } else { BOUNCE }, false, 0),
            // jig: hop-kicks on the eighths, little hops off the ground
            "celtic" => match half.rem_euclid(4) {
                0 => (KICK_R, false, 0),
                1 => (TUCK, frac.fract() < 0.5, 0),
                2 => (KICK_L, false, 0),
                _ => (TUCK, true, 0),
            },
            _ => if on && (p.dance % 2 == 0 || beat % 2 == 0) { (TUCK, true, 0) } else { (BOUNCE, false, 0) },
        };
    }
    // everyday dancing by personality, feet on the beat
    match p.dance {
        0 => (if on { BOUNCE } else { STAND }, false, 0),
        1 => (if beat.rem_euclid(4) < 2 { TAP_R } else { TAP_L }, false, (beat.rem_euclid(4) >= 2) as i32),
        2 => (match beat.rem_euclid(4) { 0 => STEP_R, 1 => STAND, 2 => STEP_L, _ => STAND }, false, 0),
        3 => (if on { TAP_R } else { STAND }, false, 0),
        4 => (if beat % 2 == 0 { STAND } else { PASS }, false, 0),
        5 => (if beat % 2 == 0 { SHUF_A } else { SHUF_B }, false, 0),
        _ => (if on { if beat % 2 == 0 { KICK_R } else { KICK_L } } else { STAND }, false, 0),
    }
}

/// FH rows of exactly FW cells, a lift (1 = jumping) and an x shift (dance steps)
pub fn fig(l: &Look) -> ([String; FH], bool, i32) {
    use legs::*;
    let p = persona(l.id);
    let beat = l.beats.floor() as i64;
    let frac = l.beats.fract() as f32;
    let odd = beat.rem_euclid(2) == 1;
    let st = l.style;
    let blink = ((l.t * 0.6 + (l.id % 97) as f32 * 0.31) % 4.0) < 0.15;
    // hair or hat
    let mut hair = HAIR[p.hair].to_string();
    if p.themed {
        let h = match st {
            "country" => Some("  _n_  "),
            "baroque" => Some("  @@@  "),
            "jazz" => Some("  _=_  "),
            "biker" => Some("  ___  "),
            "monk" => Some("   .   "),
            "techno" => Some(if odd { " * * * " } else { " + + + " }),
            _ => None,
        };
        if let Some(h) = h {
            hair = h.to_string();
        }
    }
    if p.acc == 5 {
        hair.replace_range(5..6, "*"); // a flower behind the ear
    }
    // expression: from the music (vibe) and the process's own activity
    let hype = l.act == Act::Here && l.vibe == Vibe::Hype;
    let (mut eye, mut mouth) = match (l.act, l.vibe) {
        (Act::Wave | Act::Leave, _) => ('^', '-'),
        (Act::Bar, _) => ('-', 'o'),
        (_, Vibe::Applause) => ('^', 'v'),
        (Act::Here, Vibe::Hype) if st == "industrial" => if odd { ('>', 'O') } else { ('x', 'O') },
        (Act::Here, Vibe::Hype) => if odd { ('^', 'O') } else { ('*', 'o') },
        _ if l.disk => ('@', '_'),
        _ if l.cpu > 0.5 => ('>', 'o'),
        _ if l.idle > 40.0 => ('-', '.'),
        _ if l.chat => ('\'', 'o'),
        _ if l.cpu > 0.08 => ('^', '_'),
        (_, Vibe::Chill) => if frac < 0.35 { ('^', '_') } else { ('-', '_') },
        _ => ('o', '_'),
    };
    if blink && eye != '-' {
        eye = '-';
    }
    if l.surge && l.act == Act::Here {
        mouth = 'O';
    }
    let (lb, rb) = if p.acc == 1 { ('d', 'b') } else { ('(', ')') };
    let ear = if p.acc == 2 { '\'' } else { ' ' };
    let head = format!(" {lb}{eye}{mouth}{eye}{rb}{ear}");
    // body: arms + clothes
    let top = TOPS[p.top];
    let mut tc: Vec<char> = top.chars().collect();
    if p.acc == 3 {
        tc[0] = '~'; // scarf
    }
    let top: String = tc.into_iter().collect();
    let bag = if p.acc == 4 { 'o' } else { ' ' };
    let arms = |la: char, ra: char| format!(" {la}{top}{ra}{bag}");
    let mut body;
    let mut lift = false;
    let mut dx = 0;
    let mut lg: [&str; 2];
    match l.act {
        Act::Enter | Act::Back | Act::ToBar | Act::Leave => {
            // a 4-frame walking cycle, arms swinging against the legs
            lg = match l.step.rem_euclid(4) {
                0 => STEP_R,
                1 => PASS,
                2 => STEP_L,
                _ => PASS,
            };
            body = if l.step.rem_euclid(4) < 2 { arms('\\', '\\') } else { arms('/', '/') };
            if l.disk {
                hair = "  [#]  ".into();
                body = arms('\\', '/');
            }
            if l.act == Act::Leave && l.step.rem_euclid(4) < 2 {
                body = arms('/', '/');
            }
        }
        Act::Wave => {
            body = if (l.t * 4.0) as i32 % 2 == 0 { arms('/', '/') } else { arms('/', '|') };
            lg = if (l.t * 4.0) as i32 % 2 == 0 { TAP_R } else { STAND };
        }
        Act::Bar => {
            let sip = ((l.t + l.id as f32 * 0.0037) % 3.0) < 0.8;
            let mug = if l.beer { 'U' } else { 'c' };
            body = if sip { format!(" /{top}{mug}{bag}") } else { format!(" /{top}\\{mug}") };
            // tapping a foot at the bar
            lg = if frac < 0.22 { TAP_R } else { STAND };
        }
        Act::Here => {
            let (f, up, x) = footwork(l, &p, hype);
            lg = f;
            lift = up;
            dx = x;
            body = match l.vibe {
                Vibe::Applause => if (l.t * 6.0) as i32 % 2 == 0 { arms('>', '<') } else { arms('-', '-') },
                Vibe::Hype => match st {
                    // moshing: headbang (hair flips on the beat), flailing fists, shoved about
                    "industrial" => {
                        hair = if frac < 0.35 { "  \\\\\\  ".into() } else { "  ///  ".into() };
                        dx += [0, 1, -1, 1, 0, -1][((beat + (l.id % 5) as i64) as usize) % 6];
                        if odd { arms('\\', '/') } else { arms('/', '\\') }
                    }
                    // fists up for the chorus, the horns \m/ the rest of the time
                    "biker" if l.chorus => {
                        hair = if frac < 0.5 { " \\o o/ ".into() } else { " \\o_o/ ".into() };
                        arms('|', '|')
                    }
                    "biker" if (l.id >> 3) % 3 != 0 => {
                        hair = if odd { "\\m/ \\m/".into() } else { " \\m/   ".into() };
                        if odd { arms('|', '|') } else { arms('|', '\\') }
                    }
                    "country" => if odd { arms('/', '-') } else { arms('-', '\\') },
                    "baroque" => if frac < 0.2 { arms('.', '.') } else { arms('/', '\\') },
                    "blues" => arms('\\', '\\'),
                    _ => arms('\\', '/'),
                },
                _ if l.disk => {
                    hair = "  [#]  ".into();
                    arms('\\', '/')
                }
                _ if l.cpu > 0.5 => if (l.beats * 2.0).floor() as i64 % 2 == 0 { arms('\\', '/') } else { arms('/', '\\') },
                _ if l.idle > 40.0 => {
                    // sitting it out
                    lg = SIT;
                    lift = false;
                    dx = 0;
                    hair = if (l.t as i32 + (l.id % 7) as i32) % 4 < 2 { "      z".into() } else { "     z ".into() };
                    arms('/', '\\')
                }
                _ => match p.dance {
                    3 => if frac < 0.3 { arms('/', '\\') } else { arms('|', '|') }, // headbang
                    4 => if odd { arms('-', '/') } else { arms('\\', '-') },     // robot
                    _ if l.cpu > 0.08 => if odd { arms('\\', '\\') } else { arms('/', '/') },
                    _ => if beat.rem_euclid(4) < 2 { arms('/', '\\') } else { arms('|', '\\') },
                },
            };
            // the blues: a swing step on the shuffle (step, kick on the swung "and", step back)
            if st == "blues" && l.idle <= 40.0 && l.vibe != Vibe::Applause {
                let b4 = beat.rem_euclid(4) as usize;
                lg = if frac < 0.66 { [STEP_R, PASS, STEP_L, PASS][b4] } else { [KICK_L, TAP_R, KICK_R, TAP_L][b4] };
                dx = [0, 1, 0, -1][b4];
                lift = false;
                if hype {
                    body = if b4 % 2 == 0 { arms('\\', '-') } else { arms('-', '/') };
                }
            }
            if l.chat && !hype {
                hair.replace_range(5..7, "..");
            }
        }
    }
    let rows = if p.tall {
        [hair, head, body, lg[0].to_string(), lg[1].to_string()]
    } else {
        // shorter: no knee row
        ["       ".to_string(), hair, head, body, lg[1].to_string()]
    };
    (rows, lift, dx)
}

/// something held up overhead now and then (beer, wine, an eggplant, a phone filming;
/// lighters at the blues, glowsticks at techno)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Item {
    Beer,
    Wine,
    Eggplant,
    Phone,
    Lighter,
    Glow,
}

impl Item {
    /// (glyph, cells): emoji are 2 cells wide; ASCII fallbacks otherwise
    pub fn glyph(self, ascii: bool, emoji: bool) -> (&'static str, usize) {
        match (self, ascii || !emoji) {
            (Item::Beer, false) => ("🍺", 2),
            (Item::Wine, false) => ("🍷", 2),
            (Item::Eggplant, false) => ("🍆", 2),
            (Item::Phone, false) => ("📱", 2),
            (Item::Beer, true) => ("U", 1),
            (Item::Wine, true) => ("Y", 1),
            (Item::Eggplant, true) => ("@", 1),
            (Item::Phone, true) => ("[]", 2),
            (Item::Lighter, _) => ("i", 1),
            (Item::Glow, _) => (if ascii { "|" } else { "│" }, 1),
        }
    }
}

/// who holds what overhead right now (occasional, mostly in big moments); the x sway
pub fn item(l: &Look) -> Option<(Item, i32)> {
    if l.act != Act::Here || l.idle > 40.0 {
        return None;
    }
    let big = l.vibe == Vibe::Hype || l.surge || l.chorus;
    let id = l.id;
    // a few at a time: each person has a window of ~6 s every ~30 s, more often in big moments
    let period = if big { 14.0 } else { 40.0 };
    let ph = ((l.t + (id % 997) as f32 * 0.71) / period).fract();
    let win = if big { 0.45 } else { 0.18 };
    if ph > win || (id >> 7) % 3 == 0 {
        return None;
    }
    let it = match l.style {
        "blues" => Item::Lighter,
        "techno" => Item::Glow,
        _ if big && (id >> 11) % 3 == 0 => Item::Phone,
        _ => [Item::Beer, Item::Beer, Item::Wine, Item::Eggplant, Item::Phone][((id >> 14) % 5) as usize],
    };
    let beat = l.beats.floor() as i64;
    let sway = if beat.rem_euclid(2) == 0 { 0 } else { 1 };
    Some((it, sway))
}

/// a back-row figure: 3 rows (head, body, legs), smaller and further away
pub fn fig_small(l: &Look) -> [String; 3] {
    let beat = l.beats.floor() as i64;
    let odd = beat.rem_euclid(2) == 1;
    let hype = l.act == Act::Here && l.vibe == Vibe::Hype;
    let head = match (l.act, hype, l.style) {
        (Act::Here, true, "industrial") => if odd { "   x   " } else { "   o   " },
        (Act::Here, true, _) => if odd { "  \\o/  " } else { "   o   " },
        _ => "   o   ",
    };
    let body = match l.act {
        Act::Bar => "  /|U  ",
        Act::Here if hype && l.style == "biker" => if odd { "  m|m  " } else { "  /|\\  " },
        Act::Here if hype => if odd { "   |   " } else { "  /|\\  " },
        Act::Enter | Act::Back | Act::ToBar | Act::Leave => if l.step.rem_euclid(2) == 0 { "  /|\\  " } else { "  \\|/  " },
        _ => "  /|\\  ",
    };
    let legs = match l.act {
        Act::Enter | Act::Back | Act::ToBar | Act::Leave => if l.step.rem_euclid(2) == 0 { "  / \\  " } else { "   |\\  " },
        Act::Here if hype && odd => "  ' '  ",
        _ => "  / \\  ",
    };
    [head.to_string(), body.to_string(), legs.to_string()]
}

/// the bar at the right end of the floor: 3 rows of BARW-1 cells (emoji width 2) + a label
pub fn bar_rows(ascii: bool) -> [String; 4] {
    if ascii {
        ["+-----+".into(), "|U   c|".into(), "+-----+".into(), "  bar  ".into()]
    } else {
        ["┌─────┐".into(), "│🍺 ☕│".into(), "┴─────┴".into(), "  bar  ".into()]
    }
}

/// a name tag, at most `w` cells: the exe name only (no PIDs), truncated
pub fn tag(name: &str, w: usize) -> String {
    name.chars().take(w).collect()
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
    /// depth: 0 back (by the bar, hazy) .. rows-1 front (at the stage)
    pub z: f32,
}

/// the floor's places: stacked rows of slots, back rows staggered half a slot so the people
/// behind look out between the heads in front. Returns (row, x) per slot; row rows-1 = front
pub fn slots(fw: usize, rows: usize) -> Vec<(usize, f32)> {
    let mut v = Vec::new();
    // fill order: the front row first, then the rows behind (a small crowd stands at the stage)
    for row in (0..rows.max(1)).rev() {
        let stagger = (rows - 1 - row) % 2 == 1;
        let off = if stagger { SLOT / 2 } else { 0 };
        let n = (fw.saturating_sub(off) / SLOT).max(1);
        for c in 0..n {
            v.push((row, (off + c * SLOT + (SLOT - FW) / 2) as f32));
        }
    }
    v
}

pub struct Floor {
    pub people: Vec<Person>,
    /// sent home to make room: not back for a while
    benched: HashMap<i32, f32>,
    evict_t: f32,
    last: Option<Instant>,
    pub t: f32,
    rng: u64,
    /// stacked rows of people on the floor (from the panel height)
    pub rows: usize,
}

impl Floor {
    pub fn new() -> Floor {
        Floor { people: Vec::new(), benched: HashMap::new(), evict_t: 0.0, last: None, t: 0.0, rng: 0xA5A5_1234_5678_9ABC, rows: 1 }
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
        let places = slots(fw, self.rows);
        let nslots = places.len().clamp(1, CAP);
        WANT.store(nslots, std::sync::atomic::Ordering::Relaxed);
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
                    free.sort_unstable();
                    let r = self.rand();
                    // the frontmost row with room, a random place in it
                    let front = places[free[0]].0;
                    let row_free: Vec<usize> = free.iter().copied().filter(|&s| places[s].0 == front).collect();
                    let slot = row_free[((r * row_free.len() as f32) as usize).min(row_free.len() - 1)];
                    let from_left = places[slot].1 < fw as f32 * 0.5;
                    // looks and habits come from the exe name, so the same program is always the same person
                    let seed = name_id(&m.name);
                    let beer = (seed >> 24) % 5 < 3;
                    self.people.push(Person { m: m.clone(), x: if from_left { -(FW as f32) } else { fw as f32 }, slot, act: Act::Enter, t_act: 0.0, step: 0, beer, seed, stay: 0.0, z: places[slot].0 as f32 });
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
        // one walker back from the bar at a time, so they don't walk through each other
        let mut backing = self.people.iter().any(|p| p.act == Act::Back);
        let mut want_bar: Option<usize> = None;
        let mut want_move: Option<usize> = None;
        // one wanderer at a time (each step of a walk repaints a whole figure)
        let walking = self.people.iter().any(|p| matches!(p.act, Act::Enter | Act::Back | Act::ToBar));
        let n = self.people.len();
        for i in 0..n {
            let r = self.rand();
            let p = &mut self.people[i];
            p.t_act += dt;
            let home = places.get(p.slot).map_or(0.0, |s| s.1);
            let speed = if p.m.fresh && p.m.cpu > 0.05 { 16.0 } else { 9.0 };
            let go = |p: &mut Person, target: f32| -> bool {
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
                    } else if p.t_act > 10.0 && want_move.is_none() && !walking && r > 1.0 - dt / 60.0 {
                        // now and then someone wanders to another place, a row forward or back
                        want_move = Some(i);
                    }
                }
                Act::ToBar => {
                    let spot = fw as f32 - FW as f32 - 1.0 - (FW + 1) as f32 * bar_q as f32;
                    bar_q += 1;
                    if go(p, spot) {
                        p.act = Act::Bar;
                        p.t_act = 0.0;
                        p.stay = 6.0 + 8.0 * r;
                    }
                }
                Act::Bar => {
                    bar_q += 1;
                    if (p.t_act > p.stay || vibe == Vibe::Hype) && !backing {
                        backing = true;
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
        if let Some(i) = want_move {
            // the music pulls people forward, a calm band lets them drift back
            let row = places.get(self.people[i].slot).map_or(0, |s| s.0);
            let used: Vec<usize> = self.people.iter().filter(|p| !matches!(p.act, Act::Wave | Act::Leave)).map(|p| p.slot).collect();
            let dir: i32 = match vibe {
                Vibe::Hype | Vibe::Applause => 1,
                Vibe::Chill => -1,
                Vibe::Normal => if self.rand() < 0.5 { 1 } else { -1 },
            };
            let to_row = row as i32 + dir;
            let x = self.people[i].x;
            let cand = (0..nslots).filter(|s| !used.contains(s) && places[*s].0 as i32 == to_row).min_by(|a, b| (places[*a].1 - x).abs().partial_cmp(&(places[*b].1 - x).abs()).unwrap());
            if let Some(s) = cand {
                let p = &mut self.people[i];
                p.slot = s;
                p.act = Act::Back;
                p.t_act = 0.0;
            }
        }
        // depth: each place is in a row; the bar is at the back, so drinkers drift back
        // (and come forward again to their row)
        for p in self.people.iter_mut() {
            let row = places.get(p.slot).map_or(0, |s| s.0) as f32;
            let target = match p.act {
                Act::Here | Act::Enter | Act::Back => row,
                Act::Bar | Act::ToBar => 0.0,
                Act::Wave | Act::Leave => p.z,
            };
            let k = (dt * 0.9).min(1.0);
            p.z += (target - p.z) * k;
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
        let names = ["firefox", "bash", "cargo", "node", "sysjam", "tmux: server", "kworker", "x", "python3", "gnome-shell", "Xorg", "zsh"];
        let mut looks = std::collections::HashSet::new();
        for name in names {
            let id = name_id(name);
            assert_eq!(id, name_id(name));
            looks.insert(format!("{:?}", persona(id)));
            for st in crate::style::STYLES {
                for &act in &acts {
                    for &vibe in &vibes {
                        for k in 0..24 {
                            let l = Look { style: st.id, act, vibe, beats: k as f64 * 0.37, t: k as f32 * 0.29, cpu: (k % 4) as f32 * 0.25, disk: k % 5 == 0, chat: k % 3 == 0, idle: (k * 5) as f32, step: k as i32, beer: k % 2 == 0, id, surge: k % 7 == 0, chorus: k % 5 == 0 };
                            let (rows, _, dx) = fig(&l);
                            assert!(dx.abs() <= 3);
                            for r in fig_small(&l).iter() {
                                assert_eq!(r.width(), FW, "small {r:?}");
                                assert!(r.chars().all(|c| (' '..='~').contains(&c)), "non-ASCII in {r:?}");
                            }
                            for r in rows.iter() {
                                assert_eq!(r.width(), FW, "{} {name} {act:?} {vibe:?}: {r:?}", st.id);
                                assert_eq!(r.chars().count(), FW);
                                assert!(r.chars().all(|c| (' '..='~').contains(&c)), "non-ASCII in {r:?}");
                            }
                        }
                    }
                }
            }
        }
        assert!(looks.len() >= 10, "names should look different: {}", looks.len());
        for a in [true, false] {
            for r in bar_rows(a) {
                assert_eq!(r.width(), BARW - 1, "{r:?}");
            }
        }
        assert_eq!(tag("firefox", 4), "fire");
        assert_eq!(tag("sh", 9), "sh");
    }
}
