//! A modeled electric guitar (and bass) in place of the GM guitar samples.
//!
//! Strings: an extended Karplus-Strong plucked string per note (Jaffe & Smith): a pick-shaped
//! noise burst with a pick-position comb, a fractional delay line that follows pitch bends and
//! vibrato, a one-pole loop filter for brightness / damping and a loss for the decay time; palm
//! mutes damp hard and short, a note-off damps the string, chords are strummed (each string a few
//! ms after the last).
//!
//! Amp, per channel: a noise gate (high gain), a compressor (country), a tight high-pass and a mid
//! boost before the clipper (a "screamer" in front), a 2x-oversampled asymmetric soft clipper with
//! a band limit before it (no aliasing fizz), power-supply sag, a Fender/Marshall-style tone stack
//! (bass shelf, mid peak/scoop, treble shelf), a speaker-cabinet EQ (box thump, presence, a steep
//! roll-off from ~5 kHz, so nothing piercing), then slapback, spring reverb or chorus.
//! The bass gets a DI blended with a little drive.
//!
//! The stage layout (pre-boost, oversampled asymmetric clipping, tone stack, cab) follows the
//! classic designs as documented by the Guitarix project (GPL, guitarix.org), whose amp and
//! tone-stack models were studied for this; no Guitarix code is used.

const MAXD: usize = 2048;
const VOICES: usize = 14;
/// strings per channel (a guitar's six)
const STRINGS: usize = 6;

#[derive(Clone, Copy, Default)]
struct Bq {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Bq {
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
    fn set(&mut self, b: [f32; 3], a: [f32; 3]) {
        self.b0 = b[0] / a[0];
        self.b1 = b[1] / a[0];
        self.b2 = b[2] / a[0];
        self.a1 = a[1] / a[0];
        self.a2 = a[2] / a[0];
    }
    /// RBJ cookbook: 0 lowpass, 1 highpass, 2 peaking, 3 low shelf, 4 high shelf
    fn new(kind: u8, f: f32, q: f32, db: f32, sr: f32) -> Bq {
        let mut b = Bq::default();
        let w = std::f32::consts::TAU * (f / sr).min(0.49);
        let (s, c) = w.sin_cos();
        let al = s / (2.0 * q);
        let a = 10f32.powf(db / 40.0);
        let (bb, aa) = match kind {
            0 => ([(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0], [1.0 + al, -2.0 * c, 1.0 - al]),
            1 => ([(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0], [1.0 + al, -2.0 * c, 1.0 - al]),
            2 => ([1.0 + al * a, -2.0 * c, 1.0 - al * a], [1.0 + al / a, -2.0 * c, 1.0 - al / a]),
            3 => {
                let sq = 2.0 * a.sqrt() * al;
                ([a * ((a + 1.0) - (a - 1.0) * c + sq), 2.0 * a * ((a - 1.0) - (a + 1.0) * c), a * ((a + 1.0) - (a - 1.0) * c - sq)], [(a + 1.0) + (a - 1.0) * c + sq, -2.0 * ((a - 1.0) + (a + 1.0) * c), (a + 1.0) + (a - 1.0) * c - sq])
            }
            _ => {
                let sq = 2.0 * a.sqrt() * al;
                ([a * ((a + 1.0) + (a - 1.0) * c + sq), -2.0 * a * ((a - 1.0) + (a + 1.0) * c), a * ((a + 1.0) + (a - 1.0) * c - sq)], [(a + 1.0) - (a - 1.0) * c + sq, 2.0 * ((a - 1.0) - (a + 1.0) * c), (a + 1.0) - (a - 1.0) * c - sq])
            }
        };
        b.set(bb, aa);
        b
    }
}

#[inline]
fn tanh_f(x: f32) -> f32 {
    // rational tanh, accurate to ~1e-3 in |x| < 4.97, clamped outside
    let x = x.clamp(-4.97, 4.97);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// one style's guitar sound
#[derive(Clone, Copy, Debug)]
pub struct Preset {
    pub name: &'static str,
    /// pre-gain into the clipper (1 = clean)
    gain: f32,
    /// clipper asymmetry (even harmonics)
    bias: f32,
    /// tight high-pass before the clipper (Hz)
    hp: f32,
    /// mid boost before the clipper (dB at 800 Hz)
    boost: f32,
    /// tone stack: bass, mid, treble (dB)
    tone: [f32; 3],
    /// cab roll-off (Hz)
    cab: f32,
    /// noise gate threshold (0 = off)
    gate: f32,
    /// compressor amount (0 = off)
    comp: f32,
    sag: f32,
    /// 0 none, 1 slapback, 2 spring reverb, 3 chorus
    fx: u8,
    fx_mix: f32,
    /// string brightness (pick), 0..1, and sustain (s)
    bright: f32,
    sustain: f32,
    /// palm-mute low single notes / chugs
    mute: bool,
    out: f32,
}

pub const PRESETS: [Preset; 8] = [
    // country: a clean Tele twang, squashed, with slapback
    Preset { name: "clean Tele twang, compressed, slapback", gain: 1.3, bias: 0.05, hp: 90.0, boost: 0.0, tone: [-1.0, 1.5, 2.5], cab: 5000.0, gate: 0.0, comp: 0.7, sag: 0.0, fx: 1, fx_mix: 0.32, bright: 0.85, sustain: 2.2, mute: false, out: 0.9 },
    // blues: edge of breakup, spring reverb
    Preset { name: "edge-of-breakup crunch, spring reverb", gain: 4.0, bias: 0.12, hp: 80.0, boost: 3.0, tone: [1.5, 2.0, 0.5], cab: 4600.0, gate: 0.0, comp: 0.2, sag: 0.35, fx: 2, fx_mix: 0.28, bright: 0.7, sustain: 2.6, mute: false, out: 0.7 },
    // biker: fuzzy crunch
    Preset { name: "fuzzy crunch", gain: 11.0, bias: 0.25, hp: 100.0, boost: 2.0, tone: [2.0, 1.0, -1.0], cab: 4200.0, gate: 0.012, comp: 0.0, sag: 0.4, fx: 0, fx_mix: 0.0, bright: 0.65, sustain: 3.0, mute: true, out: 0.42 },
    // metal: tight high gain, gated, scooped (double-tracked by the band's L/R channels)
    Preset { name: "tight high-gain, gated, scooped, double-tracked", gain: 26.0, bias: 0.08, hp: 150.0, boost: 6.0, tone: [3.0, -5.0, 1.5], cab: 4800.0, gate: 0.02, comp: 0.0, sag: 0.15, fx: 0, fx_mix: 0.0, bright: 0.75, sustain: 3.0, mute: true, out: 0.3 },
    // industrial: harsh and gated
    Preset { name: "harsh, gated", gain: 32.0, bias: 0.3, hp: 180.0, boost: 4.0, tone: [0.0, 3.0, 2.5], cab: 5200.0, gate: 0.03, comp: 0.0, sag: 0.0, fx: 0, fx_mix: 0.0, bright: 0.8, sustain: 2.0, mute: true, out: 0.26 },
    // jazz: a warm archtop, neck pickup, tone rolled off
    Preset { name: "warm archtop", gain: 1.1, bias: 0.02, hp: 70.0, boost: 0.0, tone: [2.0, 0.5, -6.0], cab: 3200.0, gate: 0.0, comp: 0.15, sag: 0.0, fx: 2, fx_mix: 0.12, bright: 0.45, sustain: 2.0, mute: false, out: 1.0 },
    // everything else: clean with chorus
    Preset { name: "clean chorus", gain: 1.2, bias: 0.03, hp: 85.0, boost: 0.0, tone: [0.0, 0.0, 2.0], cab: 5000.0, gate: 0.0, comp: 0.3, sag: 0.0, fx: 3, fx_mix: 0.45, bright: 0.7, sustain: 2.4, mute: false, out: 0.95 },
    // bass: DI with a little drive
    Preset { name: "bass DI + light drive", gain: 2.0, bias: 0.05, hp: 30.0, boost: 0.0, tone: [2.0, 0.0, -3.0], cab: 3800.0, gate: 0.0, comp: 0.4, sag: 0.0, fx: 0, fx_mix: 0.0, bright: 0.45, sustain: 3.5, mute: false, out: 1.2 },
];

pub fn preset_for(style: &str) -> usize {
    match style {
        "country" => 0,
        "blues" => 1,
        "biker" => 2,
        "metal" => 3,
        "industrial" => 4,
        "jazz" => 5,
        _ => 6,
    }
}

/// GM programs played by the model: electric guitars, and fingered / picked bass
pub fn modeled(prog: u8) -> Option<bool> {
    match prog {
        26..=30 => Some(false),
        33 | 34 => Some(true),
        _ => None,
    }
}

struct Voice {
    on: bool,
    ch: usize,
    key: i32,
    buf: Box<[f32; MAXD]>,
    w: usize,
    /// base delay (samples) for the note, before bend / vibrato
    len: f32,
    /// loop filter state and coefficient (brightness), loss per round trip
    lp: f32,
    damp: f32,
    loss: f32,
    /// samples to wait (strum stagger), samples sounding
    wait: u32,
    age: u32,
    amp: f32,
    /// released: the string is being damped
    off: bool,
    /// excitation still to inject
    exc: Vec<f32>,
    vib: f32,
    /// the pluck's level
    a0: f32,
}

struct Amp {
    p: Preset,
    bass: bool,
    /// stages that do something for this preset
    use_boost: bool,
    use_tone: [bool; 3],
    os: bool,
    pre_hp: Bq,
    boost: Bq,
    band: Bq,
    up_lp: Bq,
    tone: [Bq; 3],
    cab: [Bq; 3],
    ncab: usize,
    env: f32,
    gate_env: f32,
    gate_g: f32,
    comp_env: f32,
    sag_env: f32,
    x1: f32,
    // fx
    dl: Vec<f32>,
    dw: usize,
    spring: [f32; 3],
    lfo: f32,
    /// samples since anything was heard (stop processing when idle)
    idle: u32,
}

impl Amp {
    fn new(p: Preset, bass: bool, sr: f32) -> Amp {
        Amp {
            p,
            bass,
            use_boost: p.boost.abs() > 0.05,
            use_tone: [p.tone[0].abs() > 0.05, p.tone[1].abs() > 0.05, !bass || p.tone[2].abs() > 0.05],
            // a clean amp barely clips: no need to oversample it
            os: p.gain > 2.5,
            pre_hp: Bq::new(1, p.hp, 0.707, 0.0, sr),
            boost: Bq::new(2, 800.0, 0.9, p.boost, sr),
            // band limit before the clipper (no fizz / aliasing)
            band: Bq::new(0, if p.gain > 8.0 { 6500.0 } else { 9000.0 }, 0.707, 0.0, sr),
            // the 2x decimation filter (running at 2 sr)
            up_lp: Bq::new(0, 0.42 * sr, 0.707, 0.0, 2.0 * sr),
            tone: [Bq::new(3, 120.0, 0.707, p.tone[0], sr), Bq::new(2, if p.tone[1] < 0.0 { 650.0 } else { 900.0 }, 0.8, p.tone[1], sr), Bq::new(4, if bass { 3000.0 } else { 2400.0 }, 0.707, p.tone[2] + if bass { 0.0 } else { 2.5 }, sr)],
            // the box thump and a steep roll-off (the presence lift lives in the treble shelf)
            cab: if bass {
                [Bq::new(2, 70.0, 1.0, 2.0, sr), Bq::new(0, p.cab, 0.707, 0.0, sr), Bq::default()]
            } else {
                [Bq::new(2, 115.0, 1.4, 3.0, sr), Bq::new(0, p.cab, 0.8, 0.0, sr), Bq::new(0, p.cab * 1.25, 0.6, 0.0, sr)]
            },
            ncab: if bass { 2 } else { 3 },
            env: 0.0,
            gate_env: 0.0,
            gate_g: 1.0,
            comp_env: 0.0,
            sag_env: 0.0,
            x1: 0.0,
            dl: vec![0.0; ((sr * 0.16) as usize).next_power_of_two()],
            dw: 0,
            spring: [0.0; 3],
            lfo: 0.0,
            idle: 0,
        }
    }

    #[inline]
    fn clip(&self, x: f32) -> f32 {
        let b = self.p.bias;
        tanh_f(x + b) - tanh_f(b)
    }

    /// one block (n <= 64) through the whole chain, stage by stage (coefficients stay in
    /// registers; much cheaper than sample by sample through a dozen filters)
    fn run_block(&mut self, buf: &mut [f32], sr: f32) {
        let p = self.p;
        let n = buf.len();
        // gate (on the input, high gain only) and compressor
        if p.gate > 0.0 || p.comp > 0.0 {
            for x in buf.iter_mut() {
                let ax = x.abs();
                if p.gate > 0.0 {
                    self.gate_env = if ax > self.gate_env { ax } else { self.gate_env * 0.9993 };
                    let tg = if self.gate_env > p.gate { 1.0 } else { 0.0 };
                    self.gate_g += (tg - self.gate_g) * if tg > self.gate_g { 0.02 } else { 0.0015 };
                    *x *= self.gate_g;
                }
                if p.comp > 0.0 {
                    self.comp_env += (ax - self.comp_env) * if ax > self.comp_env { 0.01 } else { 0.0004 };
                    *x *= (1.0 + p.comp * 1.5) / (1.0 + p.comp * 6.0 * self.comp_env);
                }
            }
        }
        for x in buf.iter_mut() {
            *x = self.pre_hp.run(*x);
        }
        if self.use_boost {
            for x in buf.iter_mut() {
                *x = self.boost.run(*x);
            }
        }
        let mut dry = [0.0f32; 64];
        if self.bass {
            dry[..n].copy_from_slice(buf);
        }
        // sag: a hard-driven amp loses a little gain (from the last block's level)
        let g = p.gain / (1.0 + p.sag * 4.0 * self.sag_env);
        if self.os {
            // 2x oversampled clipper, band-limited in front
            for x in buf.iter_mut() {
                let xin = self.band.run(*x) * g;
                let mid = 0.5 * (xin + self.x1);
                self.x1 = xin;
                let _ = self.up_lp.run(self.clip(mid));
                *x = self.up_lp.run(self.clip(xin));
            }
        } else {
            for x in buf.iter_mut() {
                *x = self.clip(*x * g);
            }
        }
        if self.bass {
            // the bass: mostly the clean DI, a little of the drive
            for (x, d) in buf.iter_mut().zip(dry.iter()) {
                *x = *d * 0.75 + *x * 0.35;
            }
        } else if p.sag > 0.0 {
            let m = buf.iter().map(|x| x.abs()).sum::<f32>() / n as f32;
            self.sag_env += (m - self.sag_env) * (1.0 - 0.9995f32.powi(n as i32));
        }
        for (t, on) in self.tone.iter_mut().zip(self.use_tone) {
            if on {
                for x in buf.iter_mut() {
                    *x = t.run(*x);
                }
            }
        }
        for c in self.cab[..self.ncab].iter_mut() {
            for x in buf.iter_mut() {
                *x = c.run(*x);
            }
        }
        // fx (a power-of-two delay line: masks, no divisions)
        let m = self.dl.len() - 1;
        match p.fx {
            1 => {
                // slapback ~110 ms, one repeat
                let d = (sr * 0.11) as usize;
                for x in buf.iter_mut() {
                    let e = self.dl[(self.dw.wrapping_sub(d)) & m];
                    self.dl[self.dw] = *x;
                    self.dw = (self.dw + 1) & m;
                    *x += e * p.fx_mix;
                }
            }
            2 => {
                // spring: a short dispersive loop (allpass chirps) feeding back, darkened
                let d = (sr * 0.037) as usize;
                for x in buf.iter_mut() {
                    let mut s = self.dl[(self.dw.wrapping_sub(d)) & m];
                    for (k, z) in self.spring.iter_mut().enumerate() {
                        let a = 0.6 - k as f32 * 0.1;
                        let o = *z - a * s;
                        *z = s + a * o;
                        s = o;
                    }
                    self.lfo = self.lfo * 0.7 + s * 0.3;
                    self.dl[self.dw] = *x + self.lfo * 0.72;
                    self.dw = (self.dw + 1) & m;
                    *x += self.lfo * p.fx_mix;
                }
            }
            3 => {
                // chorus: a 7 ms delay swept by 2 ms at 0.8 Hz (triangle LFO)
                for x in buf.iter_mut() {
                    self.lfo += 0.8 / sr;
                    if self.lfo > 1.0 {
                        self.lfo -= 1.0;
                    }
                    let tri = 4.0 * (self.lfo - 0.5).abs() - 1.0;
                    let d = sr * (0.007 + 0.002 * tri);
                    let i = d as usize;
                    let fr = d - i as f32;
                    let a = self.dl[self.dw.wrapping_sub(i) & m];
                    let b = self.dl[self.dw.wrapping_sub(i + 1) & m];
                    self.dl[self.dw] = *x;
                    self.dw = (self.dw + 1) & m;
                    *x += (a + (b - a) * fr) * p.fx_mix;
                }
            }
            _ => {}
        }
        let mut peak = 0.0f32;
        for x in buf.iter_mut() {
            *x *= p.out;
            peak = peak.max(x.abs());
        }
        self.idle = if peak > 1e-5 { 0 } else { self.idle.saturating_add(n as u32) };
    }
}

/// per-channel state from the GM layer: program, expression, volume, pan, bend
#[derive(Clone, Copy)]
struct Ch {
    bend: f32,
    last_on: u64,
    strum: u32,
}

pub struct Rig {
    sr: f32,
    voices: Vec<Voice>,
    amps: Vec<Option<Amp>>,
    ch: [Ch; 17],
    preset: usize,
    rng: u32,
    t: u64,
    pub l: Vec<f32>,
    pub r: Vec<f32>,
    bus: Vec<f32>,
}

impl Rig {
    pub fn new(sr: f32) -> Rig {
        let voices = (0..VOICES).map(|_| Voice { on: false, ch: 0, key: 0, buf: Box::new([0.0; MAXD]), w: 0, len: 100.0, lp: 0.0, damp: 0.5, loss: 0.99, wait: 0, age: 0, amp: 0.0, off: false, exc: Vec::with_capacity(MAXD), vib: 0.0, a0: 0.0 }).collect();
        Rig { sr, voices, amps: (0..17).map(|_| None).collect(), ch: [Ch { bend: 0.0, last_on: 0, strum: 0 }; 17], preset: 6, rng: 0x1234_5678, t: 0, l: vec![0.0; 64], r: vec![0.0; 64], bus: vec![0.0; 17 * 64] }
    }

    pub fn set_style(&mut self, style: &str) {
        let p = preset_for(style);
        if p != self.preset {
            self.preset = p;
            // the amps are rebuilt for the new style on their next note
            for a in self.amps.iter_mut() {
                if let Some(x) = a {
                    if !x.bass {
                        *a = None;
                    }
                }
            }
        }
    }

    pub fn preset_name(&self) -> &'static str {
        PRESETS[self.preset].name
    }

    fn rnd(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / 8388608.0 - 1.0
    }

    pub fn note_on(&mut self, ch: usize, key: i32, vel: i32, bass: bool) {
        let sr = self.sr;
        let pi = if bass { 7 } else { self.preset };
        let p = PRESETS[pi];
        if self.amps[ch].is_none() {
            self.amps[ch] = Some(Amp::new(p, bass, sr));
        }
        // strum: notes landing together on one channel go down the strings a few ms apart
        let c = &mut self.ch[ch];
        if self.t.saturating_sub(c.last_on) < (sr * 0.004) as u64 {
            c.strum += 1;
        } else {
            c.strum = 0;
        }
        c.last_on = self.t;
        let wait = c.strum * (sr * if p.gain > 8.0 { 0.004 } else { 0.009 }) as u32;
        // the same string again: re-pluck it; a guitar has six strings, so a seventh note on
        // the channel takes the oldest one; else a free voice, else the quietest
        let on_ch = self.voices.iter().filter(|v| v.on && v.ch == ch).count();
        let oldest = || self.voices.iter().enumerate().filter(|(_, v)| v.on && v.ch == ch).max_by_key(|(_, v)| v.age).map(|(i, _)| i);
        let vi = self.voices.iter().position(|v| v.on && v.ch == ch && v.key == key).or_else(|| if on_ch >= STRINGS { oldest() } else { None }).or_else(|| self.voices.iter().position(|v| !v.on)).unwrap_or_else(|| {
            let mut best = 0;
            for (i, v) in self.voices.iter().enumerate() {
                if v.amp * (1.0 - v.off as u8 as f32 * 0.7) < self.voices[best].amp * (1.0 - self.voices[best].off as u8 as f32 * 0.7) {
                    best = i;
                }
            }
            best
        });
        let f = 440.0 * 2f32.powf((key - 69) as f32 / 12.0);
        let len = (sr / f).clamp(4.0, (MAXD - 8) as f32);
        let v01 = (vel as f32 / 127.0).clamp(0.05, 1.0);
        // palm mute: the low chugs of the heavy styles (accents ring open)
        let muted = p.mute && key < 52 && vel < 112;
        let bright = (p.bright * (0.6 + 0.5 * v01)).min(0.97) * if muted { 0.55 } else { 1.0 };
        let sustain = if muted { 0.18 } else { p.sustain * if key > 76 { 0.7 } else { 1.0 } };
        // loss per period for a T60 of `sustain`
        let loss = 10f32.powf(-3.0 / (sustain * f)).min(0.9999);
        // excitation: noise shaped by the pick (lowpassed by brightness) with the pick
        // position comb (a node at 1/8 .. 1/5 of the string)
        let n = len as usize;
        // reuse the voice's buffer (no allocation on the audio thread)
        let mut exc = std::mem::take(&mut self.voices[vi].exc);
        exc.clear();
        let mut lp = 0.0;
        let a = 0.15 + 0.8 * bright;
        for _ in 0..n {
            let r = self.rnd();
            lp += (r - lp) * a;
            exc.push(lp);
        }
        let pp = (n as f32 * (0.13 + 0.05 * self.rnd().abs())) as usize;
        for i in (pp..n).rev() {
            exc[i] -= exc[i - pp] * 0.9;
        }
        let mean = exc.iter().sum::<f32>() / n.max(1) as f32;
        let amp = v01 * 0.5;
        for e in exc.iter_mut() {
            *e = (*e - mean) * amp;
        }
        let v = &mut self.voices[vi];
        v.on = true;
        v.ch = ch;
        v.key = key;
        v.len = len;
        v.lp = 0.0;
        v.damp = 1.0 - bright * 0.5;
        v.loss = loss;
        v.wait = wait;
        v.age = 0;
        v.amp = amp;
        v.off = false;
        v.exc = exc;
        v.vib = 0.0;
        v.a0 = amp;
        if v.buf.iter().any(|x| x.abs() > 1e-3) && !muted {
            // a re-pluck of a ringing string: it keeps a little of what was there
            for x in v.buf.iter_mut() {
                *x *= 0.3;
            }
        } else {
            v.buf.fill(0.0);
        }
    }

    pub fn note_off(&mut self, ch: usize, key: i32) {
        for v in self.voices.iter_mut() {
            if v.on && v.ch == ch && v.key == key && !v.off {
                v.off = true;
            }
        }
    }

    /// the end of a hard cut on one channel: its strings stop dead
    pub fn cut_ch(&mut self, ch: usize) {
        for v in self.voices.iter_mut() {
            if v.ch == ch {
                v.on = false;
                v.buf.fill(0.0);
            }
        }
        self.amps[ch] = None;
    }

    pub fn all_off(&mut self, ch: usize) {
        for v in self.voices.iter_mut() {
            if v.on && v.ch == ch {
                v.off = true;
            }
        }
    }

    /// pitch bend, MIDI value 0..16383 (±2 semitones)
    pub fn bend(&mut self, ch: usize, v: i32) {
        self.ch[ch].bend = (v - 8192) as f32 / 8192.0 * 2.0;
    }

    /// hard cut: everything silent at once (a short fade inside the gm choke)
    pub fn cut_all(&mut self) {
        for v in self.voices.iter_mut() {
            v.on = false;
            v.exc.clear();
            v.buf.fill(0.0);
        }
        for a in self.amps.iter_mut() {
            *a = None;
        }
    }

    pub fn active(&self) -> bool {
        self.voices.iter().any(|v| v.on) || self.amps.iter().any(|a| a.as_ref().is_some_and(|a| a.idle < 48000))
    }

    /// render `n` samples; `gain(ch)` = channel level (volume × expression), `pan(ch)` 0..1
    pub fn render(&mut self, n: usize, level: &[f32; 17], pan: &[f32; 17]) {
        self.l[..n].fill(0.0);
        self.r[..n].fill(0.0);
        if !self.active() {
            self.t += n as u64;
            return;
        }
        let sr = self.sr;
        self.bus[..17 * n].fill(0.0);
        for v in self.voices.iter_mut() {
            if !v.on {
                continue;
            }
            let bend = self.ch[v.ch].bend;
            let mut base = v.len / 2f32.powf(bend / 12.0);
            // a gentle vibrato on long notes (set per block: smooth enough at 64 samples)
            if v.age > (sr * 0.45) as u32 {
                v.vib += 5.5 * n as f32 / sr;
                base *= 1.0 - 0.004 * (v.vib * std::f32::consts::TAU).sin();
            }
            let off_loss = if v.off { 0.9 } else { 1.0 };
            // strum stagger: the string starts later in this block (or not at all)
            let i0 = (v.wait as usize).min(n);
            v.wait -= i0 as u32;
            let mut peak = 0.0f32;
            if i0 < n {
                // the loop with its state in locals (one fractional read, one write per sample)
                let bus = &mut self.bus[v.ch * n + i0..(v.ch + 1) * n];
                let buf = &mut v.buf[..];
                let exc = &v.exc[..];
                let (mut w, mut lp, mut age) = (v.w, v.lp, v.age);
                let g = 1.0 - v.damp * 0.9;
                let k = v.loss * off_loss;
                let rd = MAXD as f32 - base;
                for out in bus.iter_mut() {
                    let rp = w as f32 + rd;
                    let ri = rp as i32 as usize;
                    let fr = rp - ri as f32;
                    let a = buf[ri & (MAXD - 1)];
                    let b = buf[(ri + 1) & (MAXD - 1)];
                    let y = a + (b - a) * fr;
                    // loop filter: one-pole lowpass (brightness), loss (decay)
                    lp += (y - lp) * g;
                    let mut s = lp * k;
                    if (age as usize) < exc.len() {
                        s += exc[age as usize];
                    }
                    buf[w] = s;
                    w = (w + 1) & (MAXD - 1);
                    age = age.saturating_add(1);
                    *out += y;
                    peak = peak.max(y.abs());
                }
                v.w = w;
                v.lp = lp;
                v.age = age;
            }
            // done at -60 dB below the pluck
            if v.age as usize > v.exc.len() + 64 && peak < 2e-4f32.max(v.a0 * 1e-3) {
                v.on = false;
            }
            v.amp = peak;
        }
        for ch in 0..17 {
            let Some(a) = self.amps[ch].as_mut() else { continue };
            if a.idle > (sr * 1.0) as u32 && self.bus[ch * n..(ch + 1) * n].iter().all(|x| x.abs() < 1e-6) {
                continue;
            }
            let g = level[ch];
            let (gl, gr) = ((1.0 - pan[ch]).sqrt() * g, pan[ch].sqrt() * g);
            let blk = &mut self.bus[ch * n..(ch + 1) * n];
            for c in blk.chunks_mut(64) {
                a.run_block(c, sr);
            }
            for (i, y) in blk.iter().enumerate() {
                self.l[i] += y * gl;
                self.r[i] += y * gr;
            }
        }
        self.t += n as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plucks_ring_and_decay_without_blowing_up() {
        let mut r = Rig::new(48000.0);
        for (k, st) in ["country", "blues", "biker", "metal", "industrial", "jazz", "house"].iter().enumerate() {
            r.set_style(st);
            r.note_on(1, 40 + k as i32, 100, false);
            r.note_on(1, 47 + k as i32, 100, false);
            r.note_on(2, 33, 100, true);
            let lv = [0.8; 17];
            let pan = [0.5; 17];
            let mut peak = 0.0f32;
            let mut e = 0.0;
            for _ in 0..750 {
                r.render(64, &lv, &pan);
                for i in 0..64 {
                    peak = peak.max(r.l[i].abs());
                    e += r.l[i] * r.l[i];
                }
            }
            assert!(peak.is_finite() && peak < 4.0, "{st}: peak {peak}");
            assert!(e > 1e-3, "{st}: silent");
            r.cut_all();
        }
    }
}

