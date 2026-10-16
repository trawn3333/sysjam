//! The beat clock the UI animates from, shared through atomics (no locks).
//!
//! The audio thread publishes, per 16th-note step, the engine sample at which the
//! step starts and its length, plus every note-on per player; after each block it
//! also publishes when the last rendered sample will actually be heard (ALSA
//! queue delay, or the pacing of the null / WAV sinks). The UI turns "now" into
//! the sample being heard and reads the beat phase and recent hits from that, so
//! what moves on screen lines up with what comes out of the speakers.

use std::sync::atomic::{AtomicU64, Ordering::*};
use std::sync::OnceLock;
use std::time::Instant;

static EPOCH: OnceLock<Instant> = OnceLock::new();

/// nanoseconds since the process clock epoch
pub fn ns() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

static RATE: AtomicU64 = AtomicU64::new(44100);
/// smoothed "heard ns − sample · 1e9 / rate" (i64 bits) and the last rendered sample
static OFFSET: AtomicU64 = AtomicU64::new(0);
static LAST: AtomicU64 = AtomicU64::new(0);
static HAVE: AtomicU64 = AtomicU64::new(0);
static LATENCY_US: AtomicU64 = AtomicU64::new(0);

pub fn set_rate(r: u32) {
    RATE.store(r as u64, Relaxed);
}

/// audio thread, after writing a block: sample `rendered` (end of the block) will be
/// heard `delay_s` from now
pub fn anchor(rendered: u64, delay_s: f64) {
    let rate = RATE.load(Relaxed) as f64;
    let now = ns() as f64;
    let off = now + delay_s * 1e9 - rendered as f64 * 1e9 / rate;
    let prev = f64::from_bits(OFFSET.load(Relaxed));
    let sm = if HAVE.load(Relaxed) == 0 || (off - prev).abs() > 30e6 { off } else { prev + (off - prev) * 0.05 };
    OFFSET.store(sm.to_bits(), Relaxed);
    LAST.store(rendered, Relaxed);
    LATENCY_US.store((delay_s * 1e6).max(0.0) as u64, Relaxed);
    HAVE.store(1, Release);
}

/// output latency last measured (µs)
pub fn latency_us() -> u64 {
    LATENCY_US.load(Relaxed)
}

/// the engine sample being heard at `t_ns` (None before audio starts)
pub fn heard_at(t_ns: u64) -> Option<f64> {
    if HAVE.load(Acquire) == 0 {
        return None;
    }
    let rate = RATE.load(Relaxed) as f64;
    let off = f64::from_bits(OFFSET.load(Relaxed));
    let s = (t_ns as f64 - off) * rate / 1e9;
    Some(s.min(LAST.load(Relaxed) as f64).max(0.0))
}

// ---------------------------------------------------------------- steps

const N: usize = 64;
struct Step {
    /// step index + 1 (0 = empty), written last
    step: AtomicU64,
    sample: AtomicU64,
    len: AtomicU64,
    meta: AtomicU64,
}
#[allow(clippy::declare_interior_mutable_const)]
const EMPTY: Step = Step { step: AtomicU64::new(0), sample: AtomicU64::new(0), len: AtomicU64::new(0), meta: AtomicU64::new(0) };
static RING: [Step; N] = [EMPTY; N];
static HEAD: AtomicU64 = AtomicU64::new(0);

/// conducting feel of the moment
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Feel {
    #[default]
    Normal,
    Half,
    Double,
    Surge,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Meta {
    pub feel: Feel,
    /// beats per bar the conductor beats (4, 3 or 2)
    pub meter: u8,
    /// 12/8 shuffle (blues)
    pub shuffle: bool,
}

fn pack(m: Meta) -> u64 {
    (m.feel as u64) | (m.meter as u64) << 8 | (m.shuffle as u64) << 16
}
fn unpack(x: u64) -> Meta {
    Meta {
        feel: match x & 0xff {
            1 => Feel::Half,
            2 => Feel::Double,
            3 => Feel::Surge,
            _ => Feel::Normal,
        },
        meter: ((x >> 8) & 0xff).max(1) as u8,
        shuffle: (x >> 16) & 1 == 1,
    }
}

/// audio thread: 16th-note step `step` starts at engine sample `sample`, lasting `len` samples
pub fn step(step: u64, sample: u64, len: f32, meta: Meta) {
    let h = HEAD.load(Relaxed) as usize;
    let e = &RING[h % N];
    e.step.store(0, Release);
    e.sample.store(sample, Relaxed);
    e.len.store((len as f64).to_bits(), Relaxed);
    e.meta.store(pack(meta), Relaxed);
    e.step.store(step + 1, Release);
    HEAD.store((h + 1) as u64, Release);
}

/// where the music is at engine sample `s`
#[derive(Clone, Copy, Debug, Default)]
pub struct Pos {
    /// 16th-note steps since the start (fractional)
    pub steps: f64,
    /// samples per step (tempo)
    pub len: f64,
    pub meta: Meta,
}

impl Pos {
    /// quarter-note beats since the start
    pub fn beats(&self) -> f64 {
        self.steps / 4.0
    }
    pub fn bpm(&self, rate: f64) -> f64 {
        rate * 60.0 / (self.len * 4.0)
    }
    /// position within the bar, in beats (0..4)
    pub fn in_bar(&self) -> f64 {
        (self.steps / 4.0).rem_euclid(4.0)
    }
}

pub fn rate() -> f64 {
    RATE.load(Relaxed) as f64
}

/// the beat position at engine sample `s`, from the newest step that started at or before it
pub fn pos_at(s: f64) -> Option<Pos> {
    let h = HEAD.load(Acquire) as usize;
    let mut best: Option<Pos> = None;
    for k in 1..=N.min(h) {
        let e = &RING[(h - k) % N];
        let st = e.step.load(Acquire);
        if st == 0 {
            continue;
        }
        let sample = e.sample.load(Relaxed);
        let len = f64::from_bits(e.len.load(Relaxed));
        let meta = e.meta.load(Relaxed);
        if e.step.load(Acquire) != st || len <= 0.0 {
            continue;
        }
        let p = Pos { steps: (st - 1) as f64 + (s - sample as f64) / len, len, meta: unpack(meta) };
        best = Some(p);
        if sample as f64 <= s {
            break;
        }
    }
    best
}

// ---------------------------------------------------------------- hits

pub const PLAYERS: usize = 7;
const H: usize = 16;
struct Hit {
    at: AtomicU64,
    /// key << 8 | velocity, written before `at`
    info: AtomicU64,
}
#[allow(clippy::declare_interior_mutable_const)]
const NOHIT: Hit = Hit { at: AtomicU64::new(0), info: AtomicU64::new(0) };
#[allow(clippy::declare_interior_mutable_const)]
const ROW: [Hit; H] = [NOHIT; H];
static HITS: [[Hit; H]; PLAYERS] = [ROW; PLAYERS];
static HHEAD: [AtomicU64; PLAYERS] = [const { AtomicU64::new(0) }; PLAYERS];

/// audio thread: player `p` plays a note at engine sample `at` (key: MIDI-ish pitch, vel 0..127)
pub fn hit(p: usize, at: u64, key: i32, vel: i32) {
    if p >= PLAYERS {
        return;
    }
    let h = HHEAD[p].load(Relaxed) as usize;
    let e = &HITS[p][h % H];
    e.at.store(0, Release);
    e.info.store(((key.clamp(0, 255) as u64) << 8) | vel.clamp(0, 127) as u64, Relaxed);
    e.at.store(at.max(1), Release);
    HHEAD[p].store((h + 1) as u64, Release);
}

/// the latest hit of player `p` heard by sample `s`: (age in samples, key, velocity, hit count)
pub fn last_hit(p: usize, s: f64) -> Option<(f64, i32, i32, u64)> {
    let h = HHEAD[p].load(Acquire) as usize;
    let mut best: Option<(f64, i32, i32, u64)> = None;
    for k in 1..=H.min(h) {
        let e = &HITS[p][(h - k) % H];
        let at = e.at.load(Acquire);
        if at == 0 {
            continue;
        }
        let info = e.info.load(Relaxed);
        let age = s - at as f64;
        // notes are scheduled ahead: only count the ones already heard
        if age >= 0.0 && best.map(|b| age < b.0).unwrap_or(true) {
            best = Some((age, (info >> 8) as i32, (info & 0xff) as i32, (h - k) as u64));
        }
    }
    best
}

/// player `p`'s hits heard by sample `s` within `max_age` samples, newest first:
/// (age in samples, key, velocity, hit count)
pub fn recent_hits(p: usize, s: f64, max_age: f64) -> Vec<(f64, i32, i32, u64)> {
    let mut v = Vec::new();
    if p >= PLAYERS {
        return v;
    }
    let h = HHEAD[p].load(Acquire) as usize;
    for k in 1..=H.min(h) {
        let e = &HITS[p][(h - k) % H];
        let at = e.at.load(Acquire);
        if at == 0 {
            continue;
        }
        let info = e.info.load(Relaxed);
        let age = s - at as f64;
        if age >= 0.0 && age <= max_age {
            v.push((age, (info >> 8) as i32, (info & 0xff) as i32, (h - k) as u64));
        }
    }
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    v
}

/// samples from `s` until the next note anyone plays (notes are scheduled ahead), so the UI
/// can put a frame exactly on it
pub fn next_hit(s: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    for p in 0..PLAYERS {
        let h = HHEAD[p].load(Acquire) as usize;
        for k in 1..=H.min(h) {
            let at = HITS[p][(h - k) % H].at.load(Acquire);
            if at == 0 {
                continue;
            }
            let d = at as f64 - s;
            if d > 0.0 && best.map_or(true, |b| d < b) {
                best = Some(d);
            }
        }
    }
    best
}

/// the sample of the newest note anyone has sounded by sample `s` (0 when none)
pub fn latest_heard(s: f64) -> f64 {
    let mut best = 0.0f64;
    for p in 0..PLAYERS {
        let h = HHEAD[p].load(Acquire) as usize;
        for k in 1..=H.min(h) {
            let at = HITS[p][(h - k) % H].at.load(Acquire) as f64;
            if at > best && at <= s {
                best = at;
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_and_hits() {
        set_rate(48000);
        for i in 0..20u64 {
            step(i, i * 1000, 1000.0, Meta { meter: 4, ..Default::default() });
        }
        let p = pos_at(5500.0).unwrap();
        assert!((p.steps - 5.5).abs() < 1e-9);
        let p = pos_at(19999.0).unwrap();
        assert!((p.steps - 19.999).abs() < 1e-9);
        hit(2, 3000, 36, 100);
        hit(2, 9000, 38, 90);
        assert_eq!(last_hit(2, 5000.0).unwrap().1, 36);
        assert_eq!(last_hit(2, 9500.0).unwrap().1, 38);
        assert!(last_hit(2, 2000.0).is_none());
    }
}

/// when the sink started its playback clock (Null/Wav sinks; sync measurement only)
pub static SINK_T0: AtomicU64 = AtomicU64::new(0);
