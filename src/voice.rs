//! A small shared pool of synthesised voices the players design patches for:
//! subtractive (detuned saw/square/PWM into a resonant filter with envelope),
//! FM (2-4 operators), wavetable morph and Karplus-Strong pluck.
//! Capped at `POOL` voices; steals the quietest, de-clicking the stolen tail.

use crate::dsp::{adv, saw, sine, Rng};

pub const POOL: usize = 10;
const WT: usize = 2048;
const KS_MAX: usize = 2400;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Sub,
    Fm,
    Wave,
    Pluck,
}

#[derive(Clone, Copy, Debug)]
pub struct Patch {
    pub kind: Kind,
    // subtractive
    pub osc: u8, // 0 saw, 1 square, 2 pwm
    pub detune: f32, // cents
    pub cutoff: f32, // Hz
    pub res: f32, // 0..0.95
    pub env_amt: f32, // octaves of cutoff sweep
    // fm
    pub ops: u8,
    pub ratio: [f32; 4],
    pub index: f32,
    // wavetable
    pub wa: u8,
    pub wb: u8,
    pub morph: f32, // Hz
    // pluck
    pub damp: f32,
    pub bright: f32,
    // envelope (s) and level
    pub a: f32,
    pub d: f32,
    pub s: f32,
    pub r: f32,
    pub gain: f32,
}

impl Default for Patch {
    fn default() -> Self {
        Patch { kind: Kind::Sub, osc: 0, detune: 8.0, cutoff: 1200.0, res: 0.3, env_amt: 1.5, ops: 2, ratio: [1.0, 2.0, 3.0, 1.0], index: 2.0, wa: 0, wb: 1, morph: 0.2, damp: 0.995, bright: 0.5, a: 0.01, d: 0.4, s: 0.5, r: 0.5, gain: 0.5 }
    }
}

const WAVE_NAMES: [&str; 5] = ["sine", "soft saw", "hollow square", "vocal", "glass"];

impl Patch {
    /// Human description for the TUI, e.g. "synth FM 3-op ratio 3.5 bell".
    pub fn describe(&self) -> String {
        let shape = if self.s < 0.25 && self.d < 1.2 { "pluck" } else if self.s < 0.3 { "bell" } else if self.a > 0.3 { "pad" } else { "lead" };
        match self.kind {
            Kind::Sub => {
                let o = ["2×saw", "2×square", "PWM"][self.osc as usize % 3];
                format!("synth subtractive {o} ±{:.0}c, res {:.1} {shape}", self.detune, self.res)
            }
            Kind::Fm => {
                let r = self.ratio[1];
                let rs = if (r - r.round()).abs() < 0.01 { format!("{r:.0}") } else { format!("{r:.2}").trim_end_matches('0').to_string() };
                let shape = if (r - r.round()).abs() > 0.01 && self.s < 0.5 { "bell" } else { shape };
                format!("synth FM {}-op ratio {rs} {shape}", self.ops)
            }
            Kind::Wave => format!("synth wavetable {}→{} morph {:.2} Hz", WAVE_NAMES[self.wa as usize % 5], WAVE_NAMES[self.wb as usize % 5], self.morph),
            Kind::Pluck => format!("synth Karplus-Strong pluck, bright {:.1}", self.bright),
        }
    }
}

#[derive(Clone)]
struct Voice {
    on: bool,
    owner: usize, // slot index
    p: Patch,
    inc: f32,
    vel: f32,
    start: u64,
    off: u64,
    t: f32, // seconds since start
    env: f32,
    rel: bool,
    ph: [f32; 4],
    lfo: f32,
    ic1: f32,
    ic2: f32,
    g: f32,
    ks: Vec<f32>,
    ks_len: usize,
    ks_pos: usize,
    gl: f32,
    gr: f32,
    tail_l: f32,
    tail_r: f32,
    last_l: f32,
    last_r: f32,
}

impl Voice {
    fn new() -> Self {
        Voice { on: false, owner: 0, p: Patch::default(), inc: 0.0, vel: 0.0, start: 0, off: 0, t: 0.0, env: 0.0, rel: false, ph: [0.0; 4], lfo: 0.0, ic1: 0.0, ic2: 0.0, g: 0.1, ks: vec![0.0; KS_MAX], ks_len: 100, ks_pos: 0, gl: 0.7, gr: 0.7, tail_l: 0.0, tail_r: 0.0, last_l: 0.0, last_r: 0.0 }
    }
}

pub struct Pool {
    v: Vec<Voice>,
    waves: Vec<Vec<f32>>,
    sr: f32,
    rng: Rng,
    pub l: Vec<f32>,
    pub r: Vec<f32>,
}

fn wrap(x: f32) -> f32 {
    x - x.floor()
}

impl Pool {
    pub fn new(sr: f32, seed: u64) -> Pool {
        // band-limited single-cycle tables (16 partials)
        let mut waves = Vec::new();
        let amps: [fn(usize) -> f32; 5] = [
            |n| if n == 1 { 1.0 } else { 0.0 },
            |n| 1.0 / n as f32,
            |n| if n % 2 == 1 { 1.0 / n as f32 } else { 0.0 },
            |n| match n { 1 => 0.6, 2 => 0.3, 3 => 0.9, 4 => 0.4, 5 => 0.7, 8 => 0.25, 9 => 0.3, _ => 0.03 },
            |n| match n { 1 => 1.0, 3 => 0.45, 6 => 0.3, 10 => 0.2, 13 => 0.15, _ => 0.0 },
        ];
        for a in amps {
            let mut t = vec![0f32; WT];
            for (i, x) in t.iter_mut().enumerate() {
                let ph = i as f32 / WT as f32 * std::f32::consts::TAU;
                *x = (1..=16).map(|n| a(n) * (ph * n as f32).sin()).sum();
            }
            let m = t.iter().fold(0f32, |m, &x| m.max(x.abs())).max(1e-6);
            t.iter_mut().for_each(|x| *x /= m);
            waves.push(t);
        }
        Pool { v: (0..POOL).map(|_| Voice::new()).collect(), waves, sr, rng: Rng::seeded(seed ^ 0x766f), l: vec![0.0; 64], r: vec![0.0; 64] }
    }

    pub fn active(&self) -> usize {
        self.v.iter().filter(|v| v.on).count()
    }

    /// Start a note `at` (sample time), gate for `dur` samples.
    pub fn note(&mut self, owner: usize, p: &Patch, freq: f32, vel: f32, at: u64, dur: u64, pan: f32) {
        // free voice, else the quietest (released first)
        let idx = self.v.iter().position(|v| !v.on).unwrap_or_else(|| {
            let mut best = 0;
            let mut score = f32::MAX;
            for (i, v) in self.v.iter().enumerate() {
                let s = v.env * v.vel * if v.rel { 0.3 } else { 1.0 };
                if s < score {
                    score = s;
                    best = i;
                }
            }
            best
        });
        let sr = self.sr;
        let v = &mut self.v[idx];
        // de-click: carry the stolen voice's last output and let it decay
        v.tail_l = if v.on { v.last_l } else { 0.0 };
        v.tail_r = if v.on { v.last_r } else { 0.0 };
        v.on = true;
        v.owner = owner;
        v.p = *p;
        v.inc = freq / sr;
        v.vel = vel.clamp(0.0, 1.0);
        v.start = at;
        v.off = at + dur.max(1);
        v.t = 0.0;
        v.env = 0.0;
        v.rel = false;
        v.ph = [0.0, 0.25, 0.5, 0.75];
        v.ic1 = 0.0;
        v.ic2 = 0.0;
        let a = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
        v.gl = a.cos();
        v.gr = a.sin();
        if p.kind == Kind::Pluck {
            let len = ((sr / freq.max(20.0)) as usize).clamp(2, KS_MAX - 1);
            v.ks_len = len;
            v.ks_pos = 0;
            let mut lp = 0.0;
            for i in 0..len {
                let n = self.rng.bi();
                lp += (n - lp) * (0.15 + 0.85 * p.bright);
                v.ks[i] = lp;
            }
        }
    }

    /// Release every voice of an owner (e.g. its slot changed sound).
    pub fn release_owner(&mut self, owner: usize, now: u64) {
        for v in self.v.iter_mut() {
            if v.on && v.owner == owner && v.off > now {
                v.off = now.max(v.start + 1);
            }
        }
    }

    pub fn render(&mut self, now: u64, n: usize, tbl: &[f32]) {
        for x in self.l[..n].iter_mut().chain(self.r[..n].iter_mut()) {
            *x = 0.0;
        }
        let sr = self.sr;
        let dt = 1.0 / sr;
        for v in self.v.iter_mut() {
            if !v.on {
                continue;
            }
            let p = v.p;
            let ra = 1.0 / (p.a.max(0.002) * sr);
            let kd = (-1.0 / (p.d.max(0.01) * sr)).exp();
            let kr = (-1.0 / (p.r.max(0.01) * sr)).exp();
            for j in 0..n {
                let tnow = now + j as u64;
                if tnow < v.start {
                    continue;
                }
                if !v.rel && tnow >= v.off {
                    v.rel = true;
                }
                // envelope
                if v.rel {
                    v.env *= kr;
                } else if v.t < p.a {
                    v.env = (v.env + ra).min(1.0);
                } else {
                    v.env = p.s + (v.env - p.s) * kd;
                }
                v.t += dt;
                let e = v.env;
                let x = match p.kind {
                    Kind::Sub => {
                        let det = (p.detune / 1200.0).exp2();
                        let i1 = v.inc * det;
                        let i2 = v.inc / det;
                        let o = match p.osc {
                            0 => saw(v.ph[0], i1) + saw(v.ph[1], i2),
                            1 => saw(v.ph[0], i1) - saw(wrap(v.ph[0] + 0.5), i1) + saw(v.ph[1], i2) - saw(wrap(v.ph[1] + 0.5), i2),
                            _ => {
                                let pw = 0.5 + 0.4 * sine(tbl, v.lfo);
                                saw(v.ph[0], i1) - saw(wrap(v.ph[0] + pw), i1) + 0.5 * saw(v.ph[1], i2)
                            }
                        } * 0.4;
                        adv(&mut v.ph[0], i1);
                        adv(&mut v.ph[1], i2);
                        adv(&mut v.lfo, 0.3 * dt);
                        // TPT state-variable low-pass; coefficient refreshed every 16 samples
                        if j % 16 == 0 {
                            let fc = (p.cutoff * (p.env_amt * e).exp2()).min(sr * 0.42);
                            v.g = (std::f32::consts::PI * fc / sr).tan();
                        }
                        let g = v.g;
                        let k = 2.0 - 2.0 * p.res.min(0.95);
                        let a1 = 1.0 / (1.0 + g * (g + k));
                        let v3 = o - v.ic2;
                        let v1 = a1 * v.ic1 + g * a1 * v3;
                        let v2 = v.ic2 + g * v1;
                        v.ic1 = 2.0 * v1 - v.ic1;
                        v.ic2 = 2.0 * v2 - v.ic2;
                        v2
                    }
                    Kind::Fm => {
                        // chain: op[n-1] -> ... -> op[1] -> carrier op[0]; index decays with the envelope
                        let idx = p.index * (0.35 + 0.65 * e);
                        let mut m = 0.0;
                        for o in (1..p.ops.clamp(2, 4) as usize).rev() {
                            m = sine(tbl, wrap(v.ph[o] + m)) * idx * 0.16;
                            adv(&mut v.ph[o], v.inc * p.ratio[o]);
                        }
                        let c = sine(tbl, wrap(v.ph[0] + m));
                        adv(&mut v.ph[0], v.inc * p.ratio[0]);
                        c * 0.8
                    }
                    Kind::Wave => {
                        let mo = 0.5 + 0.5 * sine(tbl, v.lfo);
                        adv(&mut v.lfo, p.morph * dt);
                        let pos = v.ph[0] * WT as f32;
                        let i = pos as usize & (WT - 1);
                        let fr = pos - pos.floor();
                        let ta = &self.waves[p.wa as usize % 5];
                        let tb = &self.waves[p.wb as usize % 5];
                        let i2 = (i + 1) & (WT - 1);
                        let a = ta[i] + (ta[i2] - ta[i]) * fr;
                        let b = tb[i] + (tb[i2] - tb[i]) * fr;
                        adv(&mut v.ph[0], v.inc);
                        (a + (b - a) * mo) * 0.7
                    }
                    Kind::Pluck => {
                        let i = v.ks_pos;
                        let i2 = if i + 1 >= v.ks_len { 0 } else { i + 1 };
                        let y = v.ks[i];
                        // loop filter: w = 0.5 is the classic two-point average (dark); brighter = less smoothing
                        let w = 0.5 - 0.4 * p.bright;
                        v.ks[i] = p.damp * ((1.0 - w) * y + w * v.ks[i2]);
                        v.ks_pos = i2;
                        y * 1.2
                    }
                };
                let s = x * e * v.vel * p.gain;
                v.tail_l *= 0.97;
                v.tail_r *= 0.97;
                let ol = s * v.gl + v.tail_l;
                let or = s * v.gr + v.tail_r;
                v.last_l = ol;
                v.last_r = or;
                self.l[j] += ol;
                self.r[j] += or;
                if v.rel && v.env < 1e-4 && v.tail_l.abs() < 1e-5 {
                    v.on = false;
                    break;
                }
            }
        }
    }
}
