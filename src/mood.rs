//! The conductor's feelings: continuous valence / arousal plus a blend of 16 named
//! moods with inertia. The emotional rules here are the heuristic teacher the tiny
//! brain (brain.rs) was distilled from (tools/train_conductor.py mirrors them), and
//! the fallback when the model is unsure.

pub const N: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mood {
    #[default]
    Content,
    Delighted,
    Proud,
    Amused,
    Inspired,
    Nostalgic,
    Focused,
    Impatient,
    Frustrated,
    Annoyed,
    Bored,
    Sleepy,
    Anxious,
    Surprised,
    Tender,
    Melancholy,
}

pub const ALL: [Mood; N] = [
    Mood::Content,
    Mood::Delighted,
    Mood::Proud,
    Mood::Amused,
    Mood::Inspired,
    Mood::Nostalgic,
    Mood::Focused,
    Mood::Impatient,
    Mood::Frustrated,
    Mood::Annoyed,
    Mood::Bored,
    Mood::Sleepy,
    Mood::Anxious,
    Mood::Surprised,
    Mood::Tender,
    Mood::Melancholy,
];

/// (valence, arousal) prototype of each mood, −1..1
pub const PROTO: [(f32, f32); N] = [
    (0.5, -0.2),
    (0.9, 0.6),
    (0.7, 0.3),
    (0.6, 0.4),
    (0.7, 0.7),
    (0.2, -0.3),
    (0.1, 0.3),
    (-0.3, 0.5),
    (-0.6, 0.6),
    (-0.5, 0.3),
    (-0.3, -0.6),
    (0.0, -0.9),
    (-0.4, 0.8),
    (0.1, 0.9),
    (0.6, -0.5),
    (-0.6, -0.4),
];

impl Mood {
    pub fn name(self) -> &'static str {
        ["content", "delighted", "proud", "amused", "inspired", "nostalgic", "focused", "impatient", "frustrated", "annoyed", "bored", "sleepy", "anxious", "surprised", "tender", "melancholy"][self as usize]
    }
    pub fn idx(self) -> usize {
        self as usize
    }
}

/// what the conductor does about it
pub const ACTIONS: [&str; 6] = ["none", "hush", "lay out", "cue down", "invite", "build"];
/// what he says about it
pub const INTENTS: [&str; 8] = ["none", "praise", "scold", "encourage", "joke", "reminisce", "hype", "soothe"];

/// feature vector layout (32 inputs), shared with tools/train_conductor.py
pub const NF: usize = 32;
pub mod f {
    pub const ENERGY: usize = 0;
    pub const AGIT: usize = 1;
    pub const CLUTTER: usize = 2;
    pub const DENS: usize = 3;
    pub const OVERLAP: usize = 4;
    pub const STEP: usize = 5;
    pub const LOUD: usize = 6;
    pub const CHILL: usize = 7;
    pub const AWAKEN: usize = 8;
    pub const ALIVE: usize = 9;
    pub const SUBSIDE: usize = 10;
    pub const CHAOS: usize = 11;
    pub const BREAK: usize = 12;
    pub const QUOTE: usize = 13;
    pub const MINOR: usize = 14;
    pub const TEMPO: usize = 15;
    pub const CPU: usize = 16;
    pub const INPUT: usize = 17;
    pub const NET: usize = 18;
    pub const DISK: usize = 19;
    pub const SPIKE: usize = 20;
    pub const STUBBORN: usize = 21;
    pub const SINCE: usize = 22;
    pub const ACTIVE: usize = 23;
    pub const SURGE: usize = 24;
    pub const BUILD: usize = 25;
    pub const LIKE: usize = 26;
    pub const RELAX: usize = 27;
    pub const STRICT: usize = 28;
    pub const CADENCE: usize = 29;
    pub const TENSION: usize = 30;
    pub const ONE: usize = 31;
}

/// The emotional rules: per-mood scores from the features (the teacher).
pub fn rule_scores(x: &[f32; NF]) -> [f32; N] {
    use f::*;
    let cl = x[CLUTTER];
    let not_cl = (1.0 - cl).max(0.0);
    let mut s = [0f32; N];
    s[0] = 0.6 * not_cl * (x[AWAKEN] + x[ALIVE]) + 0.3 * x[CADENCE] + 0.15;
    s[1] = 1.4 * x[LIKE] + 0.4 * x[BUILD] + 0.3 * x[CADENCE] * x[ENERGY];
    s[2] = 0.5 * x[CADENCE] * not_cl + 0.35 * x[ENERGY] * not_cl * x[ALIVE];
    s[3] = 0.45 * x[SPIKE] * (1.0 - x[CHAOS]) + 0.25 * x[INPUT] + 0.15 * x[NET];
    s[4] = 0.5 * x[ENERGY] * not_cl * x[ALIVE] + 0.4 * x[TENSION] * not_cl + 0.35 * x[SURGE];
    s[5] = 1.1 * x[QUOTE];
    s[6] = 0.45 * x[ALIVE] + 0.3 * x[AGIT] * not_cl + 0.1;
    s[7] = 0.5 * (1.0 - x[ENERGY]) * x[AGIT] + 0.3 * x[INPUT] * x[CHILL];
    s[8] = 0.9 * x[STUBBORN] + 0.6 * (cl - 0.6).max(0.0);
    s[9] = 0.75 * cl + 0.3 * x[STEP];
    s[10] = 0.7 * (1.0 - x[ENERGY]) * (1.0 - x[AGIT]) * (x[AWAKEN] + x[SUBSIDE]) * x[SINCE];
    s[11] = 0.8 * x[CHILL] * (1.0 - x[AGIT]) * (1.0 - x[INPUT]);
    s[12] = 0.95 * x[CHAOS] + 0.3 * x[CPU] * x[AGIT];
    s[13] = 1.1 * x[SPIKE];
    s[14] = 0.5 * x[CHILL] * (1.0 - x[MINOR]) + 0.3 * x[SUBSIDE] * (1.0 - x[MINOR]);
    s[15] = 0.5 * x[MINOR] * (1.0 - x[ENERGY]) + 0.25 * x[SUBSIDE] * x[MINOR];
    s
}

pub fn softmax<const K: usize>(z: &[f32; K], gain: f32) -> [f32; K] {
    let m = z.iter().cloned().fold(f32::MIN, f32::max);
    let mut o = [0f32; K];
    let mut sum = 0.0;
    for i in 0..K {
        o[i] = ((z[i] - m) * gain).exp();
        sum += o[i];
    }
    for v in o.iter_mut() {
        *v /= sum.max(1e-9);
    }
    o
}

/// the teacher's intervention class
pub fn rule_action(x: &[f32; NF]) -> usize {
    use f::*;
    let strict = 1.0 + x[STRICT];
    let cl = x[CLUTTER] * strict;
    if x[SURGE] < 0.5 && cl > 0.87 {
        3
    } else if cl > 0.6 && x[STUBBORN] > 0.3 {
        2
    } else if cl > 0.3 {
        1
    } else if cl < 0.08 && x[ACTIVE] < 0.5 && x[CHILL] < 0.5 {
        4
    } else if x[ENERGY] > 0.6 && x[AGIT] > 0.6 && x[TENSION] < 0.4 && x[SURGE] < 0.5 {
        5
    } else {
        0
    }
}

/// the teacher's speech intent, from the dominant mood
pub fn rule_intent(dom: usize) -> usize {
    match dom {
        0 | 1 | 2 => 1,
        8 | 9 => 2,
        7 | 10 => 3,
        3 => 4,
        5 => 5,
        4 | 13 => 6,
        11 | 12 | 14 | 15 => 7,
        _ => 0,
    }
}

/// the blended state with inertia
#[derive(Clone)]
pub struct MoodState {
    pub w: [f32; N],
    pub v: f32,
    pub a: f32,
}

impl Default for MoodState {
    fn default() -> Self {
        let mut w = [0.0; N];
        w[Mood::Focused.idx()] = 0.6;
        w[Mood::Content.idx()] = 0.4;
        let mut m = MoodState { w, v: 0.0, a: 0.0 };
        m.va();
        m
    }
}

impl MoodState {
    /// drift towards a target distribution (rate: 0..1 per update)
    pub fn toward(&mut self, t: &[f32; N], rate: f32) {
        for i in 0..N {
            self.w[i] += (t[i] - self.w[i]) * rate;
        }
        self.norm();
        self.va();
    }
    /// a sudden feeling (surprise, a like): jumps in, then fades with the inertia
    pub fn impulse(&mut self, m: Mood, amt: f32) {
        self.w[m.idx()] += amt;
        self.norm();
        self.va();
    }
    fn norm(&mut self) {
        let s: f32 = self.w.iter().map(|x| x.max(0.0)).sum();
        for x in self.w.iter_mut() {
            *x = x.max(0.0) / s.max(1e-6);
        }
    }
    fn va(&mut self) {
        self.v = (0..N).map(|i| self.w[i] * PROTO[i].0).sum();
        self.a = (0..N).map(|i| self.w[i] * PROTO[i].1).sum();
    }
    pub fn dom(&self) -> (Mood, f32) {
        let (i, w) = self.w.iter().enumerate().fold((0, 0.0f32), |b, (i, &w)| if w > b.1 { (i, w) } else { b });
        (ALL[i], w)
    }
    pub fn of(&self, m: Mood) -> f32 {
        self.w[m.idx()]
    }
    /// how strict he is right now (multiplies the style's strictness)
    pub fn strictness(&self) -> f32 {
        use Mood::*;
        (1.0 + 0.5 * (self.of(Frustrated) + self.of(Annoyed) + self.of(Anxious)) - 0.35 * (self.of(Content) + self.of(Tender) + self.of(Delighted))).clamp(0.7, 1.4)
    }
    /// how keen he is to invite players back in
    pub fn generosity(&self) -> f32 {
        use Mood::*;
        (1.0 + self.of(Inspired) + self.of(Delighted) + 0.5 * self.of(Proud) - self.of(Annoyed) - self.of(Frustrated)).clamp(0.4, 2.0)
    }
    /// baton size 0.4..1
    pub fn energy(&self) -> f32 {
        (0.7 + 0.35 * self.a).clamp(0.4, 1.0)
    }
}
