//! DSP primitives shared by the bed synth.

pub const TBL: usize = 2048;

pub struct Rng(pub u32);
impl Rng {
    pub fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    pub fn f(&mut self) -> f32 {
        (self.next() >> 8) as f32 / 16777216.0
    }
    pub fn bi(&mut self) -> f32 {
        self.f() * 2.0 - 1.0
    }
}

#[inline]
pub fn sine(t: &[f32], ph: f32) -> f32 {
    let p = ph * TBL as f32;
    // via i32: a float -> usize cast costs several instructions (no unsigned convert)
    let i = (p as i32).max(0) as usize;
    let fr = p - i as f32;
    let a = t[i & (TBL - 1)];
    let b = t[(i + 1) & (TBL - 1)];
    a + (b - a) * fr
}

/// e^x for the per-sample envelopes, without the libm call: 2^(x·log2 e) split into an
/// exponent (bits) and a degree-5 polynomial; relative error ~1e-6 (1e-5 at e^-80) (inaudible: the same
/// curves), flushed to ~0 below e^-87
#[inline]
pub fn fexp(x: f32) -> f32 {
    let y = (x * std::f32::consts::LOG2_E).clamp(-126.0, 126.0);
    let n = y as i32;
    let n = if (n as f32) > y { n - 1 } else { n };
    let f = y - n as f32;
    let p = 1.0 + f * (0.693_151_36 + f * (0.240_164_14 + f * (0.055_800_49 + f * (0.009_016_63 + f * 0.001_867_21))));
    p * f32::from_bits(((n + 127) as u32) << 23)
}

/// `x % 1.0` (and `x.fract()`) without the libm call: the same result for |x| < 2^31
#[inline]
pub fn fract1(x: f32) -> f32 {
    x - (x as i32) as f32
}

#[inline]
pub fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

#[inline]
pub fn saw(ph: f32, inc: f32) -> f32 {
    2.0 * ph - 1.0 - blep(ph, inc)
}

#[inline]
pub fn adv(ph: &mut f32, inc: f32) {
    *ph += inc;
    if *ph >= 1.0 {
        *ph -= 1.0;
    }
}

#[derive(Default, Clone, Copy)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}
impl Svf {
    pub fn set(&mut self, fc: f32, q: f32, sr: f32) {
        let fc = fc.clamp(20.0, sr * 0.45);
        let g = (std::f32::consts::PI * fc / sr).tan();
        let k = 1.0 / q;
        self.a1 = 1.0 / (1.0 + g * (g + k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }
    /// returns (lowpass, bandpass)
    #[inline]
    pub fn run(&mut self, v0: f32) -> (f32, f32) {
        let v3 = v0 - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1)
    }
}

struct Comb {
    buf: Vec<f32>,
    i: usize,
    store: f32,
}
impl Comb {
    fn new(n: usize) -> Self {
        Comb { buf: vec![0.0; n], i: 0, store: 0.0 }
    }
    #[inline]
    fn run(&mut self, x: f32, fb: f32, damp: f32) -> f32 {
        let y = self.buf[self.i];
        self.store = y * (1.0 - damp) + self.store * damp;
        self.buf[self.i] = x + self.store * fb;
        self.i += 1;
        if self.i == self.buf.len() {
            self.i = 0;
        }
        y
    }
}
struct Allpass {
    buf: Vec<f32>,
    i: usize,
}
impl Allpass {
    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let b = self.buf[self.i];
        let y = b - x;
        self.buf[self.i] = x + b * 0.5;
        self.i += 1;
        if self.i == self.buf.len() {
            self.i = 0;
        }
        y
    }
}
pub struct Reverb {
    cl: Vec<Comb>,
    cr: Vec<Comb>,
    al: Vec<Allpass>,
    ar: Vec<Allpass>,
}
impl Reverb {
    pub fn new(sr: f32) -> Self {
        let s = sr / 44100.0;
        let sz = |n: usize| ((n as f32) * s) as usize;
        let combs = [1116usize, 1277, 1422, 1557];
        let aps = [556usize, 341];
        Reverb {
            cl: combs.iter().map(|&n| Comb::new(sz(n))).collect(),
            cr: combs.iter().map(|&n| Comb::new(sz(n + 23))).collect(),
            al: aps.iter().map(|&n| Allpass { buf: vec![0.0; sz(n)], i: 0 }).collect(),
            ar: aps.iter().map(|&n| Allpass { buf: vec![0.0; sz(n + 23)], i: 0 }).collect(),
        }
    }
    #[inline]
    pub fn run(&mut self, x: f32) -> (f32, f32) {
        let (fb, d) = (0.86, 0.35);
        let mut l = 0.0;
        let mut r = 0.0;
        for c in &mut self.cl {
            l += c.run(x, fb, d);
        }
        for c in &mut self.cr {
            r += c.run(x, fb, d);
        }
        for a in &mut self.al {
            l = a.run(l);
        }
        for a in &mut self.ar {
            r = a.run(r);
        }
        (l * 0.25, r * 0.25)
    }
}


/// Gentle look-ahead-free peak limiter + soft clip.
pub struct Limiter {
    env: f32,
    att: f32,
    rel: f32,
    thr: f32,
}
impl Limiter {
    pub fn new(sr: f32) -> Self {
        Limiter { env: 0.0, att: 1.0 - (-1.0 / (sr * 0.004)).exp(), rel: 1.0 - (-1.0 / (sr * 0.35)).exp(), thr: 0.6 }
    }
    /// a limiter with its own threshold (before the soft clip; 1.25 → peaks near −1 dBFS)
    pub fn with_thr(sr: f32, thr: f32) -> Self {
        Limiter { thr, ..Limiter::new(sr) }
    }
    #[inline]
    pub fn run(&mut self, l: f32, r: f32) -> (f32, f32) {
        let p = l.abs().max(r.abs());
        let k = if p > self.env { self.att } else { self.rel };
        self.env += (p - self.env) * k;
        let g = if self.env > self.thr { self.thr / self.env } else { 1.0 };
        (softclip(l * g), softclip(r * g))
    }
}

#[inline]
pub fn softclip(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

pub fn sine_table() -> Vec<f32> {
    (0..TBL).map(|i| (i as f32 / TBL as f32 * std::f32::consts::TAU).sin()).collect()
}

impl Rng {
    pub fn seeded(seed: u64) -> Rng {
        let mut x = (seed ^ 0x9E37_79B9_7F4A_7C15).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 31;
        let v = (x as u32) | 1;
        let mut r = Rng(v);
        for _ in 0..8 {
            r.next();
        }
        r
    }
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() as usize) % n }
    }
    pub fn chance(&mut self, p: f32) -> bool {
        self.f() < p
    }
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
    /// index chosen by weights (all >= 0)
    pub fn weighted(&mut self, w: &[f32]) -> usize {
        let t: f32 = w.iter().sum();
        if t <= 0.0 {
            return self.below(w.len());
        }
        let mut x = self.f() * t;
        for (i, &v) in w.iter().enumerate() {
            if x < v {
                return i;
            }
            x -= v;
        }
        w.len() - 1
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fexp_matches_exp() {
        let mut worst = 0.0f32;
        let mut x = -80.0f32;
        while x < 8.0 {
            let (a, b) = (super::fexp(x), x.exp());
            worst = worst.max(((a - b) / b).abs());
            x += 0.0137;
        }
        assert!(worst < 1.5e-5, "worst {worst}");
    }
}
