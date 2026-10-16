//! The Celtic session (a mode of the session machine): sets of real tunes. Jigs in 6/8
//! (two jig bars per bar of the band, the eighths on the triplet grid), reels in driving
//! straight eighths, slip jigs in 9/8 (the band's bar is cut to three beats). Each tune is
//! played in AABB form: the A part is built from a trad incipit out of O'Neill's / Ryan's /
//! Aird's (the music21 corpus, embedded in the binary), the B part lifts it up the scale and
//! comes home on the A part's ending. A set is three tunes; each change of tune speeds up.
//!
//! Tin whistle or flute takes the tune with the fiddle in unison, ornamented (cuts, rolls,
//! grace notes); the bodhrán keeps the pulse; a DADGAD guitar strums open, third-less
//! voicings; a low bouzouki doubles the roots (a deep sine under it); a quiet uilleann-style
//! drone holds the tonic and fifth. No synth.

use super::{fold, Blues, Ev, In, Lead, Sec};
use crate::themes::Theme;

const DRONE: u8 = 0;
const GTR: u8 = 1;
const FIDDLE: u8 = 2;
const WHISTLE: u8 = 3;
const BOUZ: u8 = 4;

/// steps per bar of the band for each tune type (jig 2×6/8, reel 2×4/4 in 8ths, slip jig 9/8)
const BAR: [u32; 3] = [16, 16, 12];
const PART_BARS: u32 = 4;

#[derive(Clone, Copy, Debug)]
pub(super) struct TN {
    at: f32,
    deg: i32,
    alt: i32,
    len: f32,
}

#[derive(Default)]
pub(super) struct State {
    /// the set: (tune type, theme index or usize::MAX for a made-up tune)
    set: Vec<(u8, usize)>,
    tune: usize,
    part: u8,
    bar: u32,
    a: Vec<TN>,
    b: Vec<TN>,
    /// chord root (semitones above the tonic) per half bar of each part
    ca: Vec<i32>,
    cb: Vec<i32>,
    steps: [i32; 7],
    minor: bool,
    name: String,
    base_bpm: f32,
    bpm: f32,
    pos: u32,
    sets: u32,
    fresh: bool,
    whistle: bool,
}

fn is_minorish(m: &str) -> bool {
    matches!(m, "minor" | "dorian" | "phrygian")
}

fn tune_type(t: &Theme) -> Option<u8> {
    let celt = t.coll.contains("O'Neill") || t.coll.contains("Ryan") || t.coll.contains("Aird");
    if !celt {
        return None;
    }
    match t.meter {
        "6/8" if t.cat.is_empty() || t.cat.contains("jig") || t.cat.starts_with("O'Neill") => Some(0),
        "9/8" => Some(2),
        "2/2" | "2/4" | "4/4" if t.cat == "reel" => Some(1),
        _ => None,
    }
}

/// theme units (1/12 quarter) → our sixteenths
fn unit_steps(t: &Theme) -> f32 {
    match t.meter {
        "6/8" | "9/8" | "12/8" => 2.0 / 9.0,
        "2/4" => 1.0 / 3.0,
        _ => 1.0 / 6.0,
    }
}

impl Blues {
    pub(super) fn celtic_start(&mut self) {
        self.started = true;
        self.ks = State::default();
        self.ks.whistle = self.rng.chance(0.6);
        let (whistle, gtr) = (if self.ks.whistle { 78 } else { 73 }, 25);
        self.progs = [109, gtr, 110, whistle, 24];
        for (ch, p) in self.progs.iter().enumerate() {
            self.ev.push(Ev::Prog(ch as u8, *p));
        }
        self.k_new_set();
        self.k_tune_start(true);
        self.ev.push(Ev::Think(2, "following", "bodhrán: the pulse, a bass tone on the beat, the tipper between".into()));
        self.ev.push(Ev::Think(3, "following", "DADGAD guitar: open strums, no thirds".into()));
        self.ev.push(Ev::Think(4, "following", "fiddle in unison with the tune: cuts, rolls".into()));
        self.ev.push(Ev::Think(5, "following", if self.ks.whistle { "tin whistle: the tune" } else { "flute: the tune" }.into()));
        self.ev.push(Ev::Think(0, "following", "the drones: tonic and fifth, low".into()));
        self.ev.push(Ev::Think(1, "following", "low bouzouki on the roots".into()));
    }

    pub(super) fn celtic_feel(&self) -> &'static str {
        match self.sec {
            Sec::Tune(0, p, _) => ["jig — 6/8, the A part", "jig — 6/8, A again", "jig — 6/8, the B part", "jig — 6/8, B again"][p as usize % 4],
            Sec::Tune(1, p, _) => ["reel — driving 8ths, the A part", "reel — driving 8ths, A again", "reel — driving 8ths, the B part", "reel — driving 8ths, B again"][p as usize % 4],
            Sec::Tune(_, p, _) => ["slip jig — 9/8, the A part", "slip jig — 9/8, A again", "slip jig — 9/8, the B part", "slip jig — 9/8, B again"][p as usize % 4],
            _ => "session",
        }
    }

    /// a set of three tunes: all jigs, all reels, jigs into a reel, or slip jigs into a jig
    fn k_new_set(&mut self) {
        let all = crate::themes::all();
        let kinds: [&[u8]; 5] = [&[0, 0, 0], &[1, 1, 1], &[0, 0, 1], &[2, 2, 0], &[0, 1, 1]];
        let w = [1.0, 0.8 + self.int, 1.2, 0.5, 0.8];
        let ks = kinds[self.rng.weighted(&w)];
        let mut set = Vec::new();
        let mut minor: Option<bool> = None;
        for &k in ks {
            let cands: Vec<usize> = (0..all.len())
                .filter(|&i| tune_type(&all[i]) == Some(k) && minor.map_or(true, |m| is_minorish(all[i].mode) == m) && !set.iter().any(|x: &(u8, usize)| x.1 == i))
                .collect();
            let idx = if cands.is_empty() { usize::MAX } else { cands[self.rng.below(cands.len())] };
            if minor.is_none() && idx != usize::MAX {
                minor = Some(is_minorish(all[idx].mode));
            }
            set.push((k, idx));
        }
        let first = set[0].1;
        let mode = if first == usize::MAX { if self.rng.chance(0.5) { "dorian" } else { "major" } } else { all[first].mode };
        // session keys: D / G / A major, E / A dorian, A / D mixolydian, E / B minor
        let pcs: &[i32] = match mode {
            "dorian" => &[4, 9, 2],
            "mixolydian" => &[9, 2, 7],
            "minor" | "phrygian" => &[4, 11, 9],
            _ => &[2, 7, 9],
        };
        self.ks.minor = is_minorish(mode);
        self.home = (pcs[self.rng.below(pcs.len())], self.ks.minor);
        self.ks.set = set;
        self.ks.tune = 0;
        self.ks.sets += 1;
        let k0 = self.ks.set[0].0 as usize;
        self.ks.base_bpm = [self.rng.range(104.0, 114.0), self.rng.range(100.0, 110.0), self.rng.range(100.0, 108.0)][k0];
        self.ks.bpm = self.ks.base_bpm;
        if self.ks.sets > 1 {
            self.ev.push(Ev::Key(self.home.0, self.home.1));
        }
    }

    /// load tune k of the set: A part from the incipit, B part lifted, chords from the melody
    fn k_tune_start(&mut self, first: bool) {
        let all = crate::themes::all();
        let (kind, idx) = self.ks.set[self.ks.tune];
        let bar = BAR[kind as usize] as f32;
        let part = bar * PART_BARS as f32;
        let mut inc: Vec<TN> = Vec::new();
        if idx != usize::MAX {
            let th = &all[idx];
            self.ks.steps = th.steps();
            self.ks.name = th.name();
            let us = unit_steps(th);
            // the downbeat lands on our bar line; the pickup is dropped
            let half = if kind == 2 { bar } else { bar / 2.0 };
            let mut at = th.pickup as f32 * us - half;
            if at < -half {
                at = 0.0;
            }
            for n in &th.notes {
                let len = n.len as f32 * us;
                if let Some(d) = n.deg {
                    if at >= 0.0 {
                        inc.push(TN { at, deg: d, alt: n.alt, len });
                    }
                }
                at += len;
            }
        } else {
            self.ks.steps = if self.ks.minor { [0, 2, 3, 5, 7, 9, 10] } else { [0, 2, 4, 5, 7, 9, 11] };
            self.ks.name = "a tune of the band's own".into();
        }
        if inc.len() < 6 {
            inc = self.k_invent(kind, part - bar);
        }
        let ilen = inc.iter().map(|n| n.at + n.len).fold(0.0, f32::max).max(bar / 2.0);
        // A: the incipit, repeated to fill the part, the last bar a cadence home
        let mut a = Vec::new();
        let mut off = 0.0;
        while off < part - bar {
            for n in &inc {
                let at = off + n.at;
                if at < part - bar {
                    a.push(TN { at, len: n.len.min(part - bar - at), ..*n });
                }
            }
            off += ilen;
        }
        a.extend(self.k_cadence(kind, part - bar));
        // B: up a fourth on the scale for the first half, then the A part's second half
        let mut b: Vec<TN> = a.iter().filter(|n| n.at < part / 2.0).map(|n| TN { deg: n.deg + 3, ..*n }).collect();
        b.extend(a.iter().filter(|n| n.at >= part / 2.0).copied());
        self.ks.ca = self.k_chords(&a, part);
        self.ks.cb = self.k_chords(&b, part);
        self.ks.a = a;
        self.ks.b = b;
        self.ks.part = 0;
        self.ks.bar = 0;
        self.ks.fresh = true;
        let n = self.ks.tune as u8;
        self.sec = Sec::Tune(kind, 0, n);
        // each new tune of the set lifts the tempo
        if !first {
            let lift = if kind == 1 { 6.0 } else { 4.0 };
            self.ks.bpm = (self.ks.bpm + lift).min(128.0);
        }
        self.ev.push(Ev::Bpm(self.ks.bpm));
        let kname = ["jig", "reel", "slip jig"][kind as usize];
        self.ev.push(Ev::Log(format!("{kname}: {} (AABB, {:.0} bpm)", self.ks.name, self.ks.bpm)));
        let line: &'static str = match (first, kind) {
            (true, 0) => "a jig to start us off",
            (true, 1) => "a few reels, lads",
            (true, _) => "a slip jig, nice and easy",
            (false, 1) => "into the reel — lift it!",
            _ => ["and change!", "hup!", "next tune — lift it!"][self.rng.below(3)],
        };
        self.ev.push(Ev::Say(line, if first { 0 } else { 2 }));
        self.k_plan();
        self.ev.push(Ev::Section(self.sec));
    }

    /// a made-up modal tune line when the corpus has nothing of this kind
    fn k_invent(&mut self, kind: u8, len: f32) -> Vec<TN> {
        let cell = if kind == 1 { 1.0 } else { 4.0 / 3.0 };
        let mut v = Vec::new();
        let mut d = 4i32;
        let mut at = 0.0;
        while at < len {
            let long = self.rng.chance(0.15);
            let l = if long { cell * 2.0 } else { cell };
            d = (d + [-2, -1, -1, 1, 1, 2, 0][self.rng.below(7)]).clamp(0, 11);
            v.push(TN { at, deg: d, alt: 0, len: l });
            at += l;
        }
        v
    }

    fn k_cadence(&mut self, kind: u8, at: f32) -> Vec<TN> {
        let cell = if kind == 1 { 1.0 } else { 4.0 / 3.0 };
        let bar = BAR[kind as usize] as f32;
        let run = [4, 2, 1, 0];
        let mut v: Vec<TN> = run.iter().enumerate().map(|(k, &d)| TN { at: at + k as f32 * cell, deg: d + 7, alt: 0, len: cell }).collect();
        let t0 = at + 4.0 * cell;
        v.push(TN { at: t0, deg: 7, alt: 0, len: (bar - (t0 - at)).max(cell) * 0.95 });
        v
    }

    /// a chord for each half bar from the melody under it
    fn k_chords(&self, mel: &[TN], part: f32) -> Vec<i32> {
        let half = 8.0f32.min(part / 8.0).max(4.0);
        let n = (part / half).round() as usize;
        let mut out = Vec::with_capacity(n);
        for k in 0..n {
            let t0 = k as f32 * half;
            let d = mel.iter().find(|x| x.at >= t0 - 0.01 && x.at < t0 + half).map_or(0, |x| x.deg).rem_euclid(7);
            let r = match d {
                0 | 2 | 4 => 0,
                3 | 5 => 5,
                _ => if self.ks.minor || self.ks.steps[6] == 10 { 10 } else { 7 },
            };
            out.push(r);
        }
        out
    }

    /// the part's chords for the harmony (one per bar of the band)
    fn k_plan(&mut self) {
        let ch = if self.ks.part >= 2 { &self.ks.cb } else { &self.ks.ca };
        let per = (ch.len() / PART_BARS as usize).max(1);
        let minor = self.ks.minor;
        let mut v: Vec<&'static str> = Vec::new();
        for b in 0..PART_BARS as usize {
            let r = ch.get(b * per).copied().unwrap_or(0);
            v.push(match (r, minor) {
                (0, false) => "I",
                (0, true) => "i",
                (5, _) => "IV",
                (7, _) => "V",
                (10, true) => "VII",
                _ => "bVII",
            });
        }
        self.ev.push(Ev::Plan(v));
    }

    pub(super) fn celtic_step(&mut self, i: &In) {
        let Sec::Tune(kind, _, _) = self.sec else {
            self.sec = Sec::Tune(0, 0, 0);
            return;
        };
        let s = i.s;
        let bar_len = BAR[kind as usize];
        if s >= bar_len as usize {
            return;
        }
        if s == 0 {
            if self.ks.fresh {
                self.ks.fresh = false;
            } else {
                self.ks.bar += 1;
                if self.ks.bar >= PART_BARS {
                    self.ks.bar = 0;
                    self.ks.part += 1;
                    if self.ks.part >= 4 {
                        // the tune is done: the next of the set, or a new set
                        self.ks.tune += 1;
                        if self.ks.tune >= self.ks.set.len() {
                            self.k_new_set();
                            self.ev.push(Ev::Say(["good man yourself!", "lovely — another set?", "that's the stuff"][self.rng.below(3)], 1));
                            self.k_tune_start(true);
                        } else {
                            self.k_tune_start(false);
                        }
                        return self.celtic_step(i);
                    }
                    let n = self.ks.tune as u8;
                    self.sec = Sec::Tune(kind, self.ks.part, n);
                    if self.ks.part == 2 {
                        self.k_plan();
                    } else if self.ks.part == 0 {
                        self.k_plan();
                    }
                    self.ev.push(Ev::Section(self.sec));
                }
            }
        }
        let pos = (self.ks.bar * bar_len + s as u32) as f32;
        let t = i.tonic;
        let part = self.ks.part;
        let chords = if part >= 2 { &self.ks.cb } else { &self.ks.ca };
        let half = 8.0f32.min(bar_len as f32 * PART_BARS as f32 / 8.0).max(4.0);
        let cr = chords.get((pos / half) as usize).copied().unwrap_or(0);
        let r = t + cr;
        self.k_drums(kind, s, part);
        self.k_guitar(kind, s, r, t);
        if s == 0 {
            self.k_low(r, t);
        }
        self.k_tune(pos, t, part);
    }

    fn k_drums(&mut self, kind: u8, s: usize, part: u8) {
        let lift = if part % 2 == 1 { 6 } else { 0 };
        if kind == 1 {
            // reel: an 8th per step, bass tone on the beats, the tipper between
            if s % 2 == 0 {
                self.drum(41, if s % 4 == 0 { 70 } else { 54 } + lift, 0.0);
            } else {
                self.drum(45, 34 + lift, 0.0);
            }
        } else if s % 4 == 0 {
            // jig: down-up-down on the triplet eighths, accent on the beat
            let tri = 4.0 / 3.0;
            self.drum(41, if s == 0 { 74 } else { 64 } + lift, 0.0);
            self.drum(45, 34 + lift, tri);
            self.drum(45, 46 + lift, 2.0 * tri);
        }
        if s == 0 && self.ks.bar == 0 && part == 0 {
            self.drum(52, 40, 0.0); // a splash at the top of a tune (the band's "hup")
        }
    }

    fn k_guitar(&mut self, kind: u8, s: usize, r: i32, t: i32) {
        // DADGAD: root, fifth, octave, the 4th / 9th rings open (no thirds)
        let root = fold(r, -5, 6);
        let sus = if (r - t).rem_euclid(12) == 0 { 5 } else { 2 };
        let voicing = [root, root + 7, root + 12, root + 12 + sus];
        let strum = |me: &mut Blues, v: i32, d: f32, n: usize| {
            for (k, &x) in voicing.iter().take(n).enumerate() {
                me.note(GTR, x, v - k as i32 * 2, 2.5, d + k as f32 * 0.04, 3);
            }
        };
        if kind == 1 {
            if s % 2 == 0 {
                strum(self, if s % 4 == 0 { 60 } else { 46 }, 0.0, if s % 4 == 0 { 4 } else { 2 });
            }
        } else if s % 4 == 0 {
            strum(self, if s == 0 { 62 } else { 54 }, 0.0, 4);
            strum(self, 40, 8.0 / 3.0, 2);
        }
    }

    fn k_low(&mut self, r: i32, t: i32) {
        // the low bouzouki: root + fifth for the bar; a sine an octave down; the drones each part
        let b = fold(r, -10, 1);
        self.note(BOUZ, b, 70, 14.0, 0.0, 1);
        self.note(BOUZ, b + 7, 56, 14.0, 0.03, 1);
        self.ev.push(Ev::Sub(b - 12, 78));
        if self.ks.bar == 0 {
            let d = fold(t, -12, -1);
            let len = (BAR[0] * PART_BARS) as f32 - 0.5;
            self.note(DRONE, d, 48, len, 0.0, 0);
            self.note(DRONE, d + 7, 40, len, 0.02, 0);
        }
    }

    fn k_semi(&self, t: i32, deg: i32, alt: i32) -> i32 {
        let base = fold(t, 15, 26);
        let s = base + self.ks.steps[deg.rem_euclid(7) as usize] + 12 * deg.div_euclid(7) + alt;
        if s > 41 { s - 12 } else { s }
    }

    fn k_tune(&mut self, pos: f32, t: i32, part: u8) {
        let mel = if part >= 2 { &self.ks.b } else { &self.ks.a };
        let notes: Vec<TN> = mel.iter().copied().filter(|n| n.at >= pos && n.at < pos + 1.0).collect();
        let second = part % 2 == 1;
        for n in notes {
            let d = n.at - pos;
            let semi = self.k_semi(t, n.deg, n.alt);
            let up = self.k_semi(t, n.deg + 1, 0);
            let down = self.k_semi(t, n.deg - 1, 0);
            let on_beat = (n.at % 4.0).abs() < 0.05 || (n.at % 4.0 - 4.0).abs() < 0.05;
            let v = if on_beat { 86 } else { 72 };
            // the whistle / flute has the tune (an occasional cut on long notes)
            if n.len >= 2.5 && self.rng.chance(0.3) {
                self.note(WHISTLE, up, 50, 0.12, d, 5);
                self.note(WHISTLE, semi, v, n.len * 0.9 - 0.14, d + 0.14, 5);
            } else {
                self.note(WHISTLE, semi, v, n.len * 0.9, d, 5);
            }
            // the fiddle in unison: rolls on long notes, cuts on the beat, more the second time
            let fs = if semi > 34 { semi - 12 } else { semi };
            let (fu, fd) = (if semi > 34 { up - 12 } else { up }, if semi > 34 { down - 12 } else { down });
            if n.len >= 2.6 && self.rng.chance(if second { 0.6 } else { 0.35 }) {
                let l3 = n.len / 3.0;
                self.note(FIDDLE, fs, v - 4, l3 * 0.9, d, 4);
                self.note(FIDDLE, fu, 44, 0.12, d + l3, 4);
                self.note(FIDDLE, fs, v - 10, l3 * 0.9 - 0.14, d + l3 + 0.14, 4);
                self.note(FIDDLE, fd, 40, 0.12, d + 2.0 * l3, 4);
                self.note(FIDDLE, fs, v - 8, l3 * 0.9 - 0.14, d + 2.0 * l3 + 0.14, 4);
            } else {
                if on_beat && self.rng.chance(if second { 0.45 } else { 0.25 }) {
                    self.note(FIDDLE, fu, 46, 0.12, d, 4);
                    self.note(FIDDLE, fs, v - 6, n.len * 0.92 - 0.14, d + 0.14, 4);
                } else {
                    self.note(FIDDLE, fs, v - 6, n.len * 0.92, d, 4);
                }
            }
        }
        let _ = Lead::Fiddle;
    }
}
