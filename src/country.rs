//! The Country band (a mode of the session machine): train beat, two-step and shuffle
//! feels; boom-chick bass alternating root and fifth with walk-ups into the changes;
//! chicken-pickin' Telecaster (staccato, muted clucks, double stops, B-bender pre-bends);
//! pedal steel sliding and swelling into chord tones; fiddle and harmonica fills; honky-tonk
//! piano on the backbeat; cowbell on the train. I–IV–V in major, with a Mixolydian bVII
//! now and then, and a call-and-response chorus (fiddle or harp calls, the Tele answers).
//! No synth at all.

use super::{fold, Blues, Ev, In, Lead, Sec, SWING};

/// channels in this mode
const PIANO: u8 = 0;
const STEEL: u8 = 1;
const TELE: u8 = 2;
const SOLO: u8 = 3;
const BASS: u8 = 4;

const VERSES: [&[&str]; 4] = [
    &["I", "I", "IV", "I", "I", "I", "V", "V"],
    &["I", "IV", "I", "V", "I", "IV", "V", "I"],
    &["I", "I", "IV", "IV", "I", "I", "V", "I"],
    &["I", "bVII", "IV", "I", "I", "bVII", "V", "I"],
];
const CHORUSES: [&[&str]; 3] = [
    &["IV", "IV", "I", "I", "V", "V", "I", "I"],
    &["I", "IV", "V", "I", "I", "IV", "V", "I"],
    &["IV", "I", "V", "I", "IV", "I", "V", "V"],
];
const BREAK: &[&str] = &["I", "I", "IV", "IV", "I", "I", "V", "V"];

#[derive(Clone, Copy)]
pub(super) struct PN {
    /// position in 16ths from the phrase start
    at: f32,
    semi: i32,
    len: f32,
    /// 0 none, 1 slide up a semitone, 3 whole-tone pre-bend, 4 steel swell
    bend: u8,
    vel: i32,
}

#[derive(Default)]
pub(super) struct State {
    bar: u32,
    len: u32,
    verses: u32,
    breaks: u32,
    prog: usize,
    chords: Vec<&'static str>,
    /// the phrase being played: notes, its start (absolute 16th), channel, player
    ph: Vec<PN>,
    ph_at: u64,
    ph_ch: u8,
    ph_who: usize,
    call: Vec<PN>,
    abs: u64,
    count_in: bool,
    mixo: bool,
    next: Option<Sec>,
    soloist: u8,
    s: usize,
}

impl Blues {
    pub(super) fn country_start(&mut self) {
        self.started = true;
        self.sec = Sec::Verse(0);
        self.cs = State::default();
        self.cs.count_in = true;
        self.cs.soloist = self.rng.below(2) as u8;
        // honky-tonk / acoustic piano, steel, Tele, fiddle (or harmonica), fingered / acoustic bass
        let piano = [3u8, 0, 1][self.rng.below(3)];
        let bass = [33u8, 32][self.rng.below(2)];
        let solo = if self.cs.soloist == 0 { 110 } else { 22 };
        self.progs = [piano, 26, 27, solo, bass];
        for (ch, p) in self.progs.iter().enumerate() {
            self.ev.push(Ev::Prog(ch as u8, *p));
        }
        // country keys: G, D, A, E, C (major)
        self.home = ([7, 2, 9, 4, 0][self.rng.below(5)], false);
        self.cs.prog = self.rng.below(VERSES.len());
        self.cs.chords = VERSES[self.cs.prog].to_vec();
        self.cs.mixo = self.cs.prog == 3;
        self.cs.len = 8;
        self.ev.push(Ev::Plan(self.cs.chords.clone()));
        self.ev.push(Ev::Think(2, "following", "count-in on the sticks, then the train beat: snare 16ths, cowbell".into()));
        self.ev.push(Ev::Think(1, "following", "boom-chick: root on 1, the fifth on 3, walk-ups into the changes".into()));
        self.ev.push(Ev::Think(3, "following", "honky-tonk piano on the backbeat".into()));
        self.ev.push(Ev::Think(4, "following", "chicken pickin': staccato, clucks, double stops".into()));
        self.ev.push(Ev::Think(0, "following", "pedal steel: slides and swells into the chords".into()));
        self.ev.push(Ev::Think(5, "following", if self.cs.soloist == 0 { "fiddle fills between the lines" } else { "harmonica fills between the lines" }.into()));
    }

    pub(super) fn country_feel(&self) -> &'static str {
        match self.sec {
            Sec::Verse(0) => "train beat — boom-chick",
            Sec::Verse(1) => "two-step — boom-chick",
            Sec::Verse(_) => "shuffle — boom-chick, swung",
            Sec::Chorus => "CHORUS — call and response",
            Sec::Break(Lead::Fiddle) => "SOLO — fiddle break",
            Sec::Break(Lead::Tele) => "SOLO — chicken-pickin' Telecaster",
            Sec::Break(Lead::Steel) => "SOLO — pedal steel",
            Sec::Break(_) => "SOLO — harmonica break",
            _ => "train beat — boom-chick",
        }
    }

    fn c_note(&mut self, ch: u8, semi: i32, vel: i32, len: f32, d: f32, who: usize) {
        // the shuffle swings the off 8ths (2 sixteenths late → two thirds of the beat)
        let d = if matches!(self.sec, Sec::Verse(2)) && self.cs.s % 4 == 2 && d < 0.01 { d + SWING - 2.0 } else { d };
        self.note(ch, semi, vel, len, d, who);
    }

    fn c_next(&mut self) -> Sec {
        match self.sec {
            Sec::Verse(_) => Sec::Chorus,
            Sec::Chorus => {
                self.cs.verses += 1;
                if self.cs.verses % 2 == 0 || (self.int > 0.6 && self.rng.chance(0.4)) {
                    let order = [Lead::Tele, Lead::Fiddle, Lead::Steel, Lead::Tele, if self.cs.soloist == 0 { Lead::Fiddle } else { Lead::Harp }];
                    let l = order[self.cs.breaks as usize % order.len()];
                    self.cs.breaks += 1;
                    Sec::Break(l)
                } else {
                    Sec::Verse(self.c_feel())
                }
            }
            _ => Sec::Verse(self.c_feel()),
        }
    }
    /// train beat when the band is hot, two-step mid, a shuffle now and then
    fn c_feel(&mut self) -> u8 {
        let w = [0.4 + self.int, 0.7, 0.35];
        self.rng.weighted(&w) as u8
    }

    fn c_plan(&mut self, sec: Sec) {
        let c: Vec<&'static str> = match sec {
            Sec::Verse(_) => {
                self.cs.prog = self.rng.below(VERSES.len());
                VERSES[self.cs.prog].to_vec()
            }
            Sec::Chorus => CHORUSES[self.rng.below(CHORUSES.len())].to_vec(),
            _ => BREAK.to_vec(),
        };
        self.cs.mixo = c.contains(&"bVII");
        self.cs.chords = c.clone();
        self.ev.push(Ev::Plan(c));
    }

    fn c_enter(&mut self, n: Sec) {
        let prev = self.sec;
        self.sec = n;
        self.cs.bar = 0;
        self.cs.len = 8;
        self.cs.ph.clear();
        match n {
            Sec::Break(l) => {
                if matches!(l, Lead::Fiddle | Lead::Harp) {
                    let p = if l == Lead::Fiddle { 110 } else { 22 };
                    self.progs[SOLO as usize] = p;
                    self.ev.push(Ev::Prog(SOLO, p));
                }
                let cue: &'static str = match l {
                    Lead::Tele => "pick it, cowboy!",
                    Lead::Fiddle => "take it, fiddle!",
                    Lead::Steel => "steel guitar!",
                    _ => "blow that harp!",
                };
                self.ev.push(Ev::Say(cue, 1));
                self.ev.push(Ev::Think(l.player(), "soloing", format!("{} break: 8 bars over I–IV–V", l.name())));
            }
            Sec::Chorus => {
                self.ev.push(Ev::Say(["sing it, y'all!", "here's the chorus!", "everybody now!"][self.rng.below(3)], 2));
                let caller = if self.progs[SOLO as usize] == 22 { "harmonica" } else { "fiddle" };
                self.ev.push(Ev::Log(format!("chorus: the {caller} calls, the Telecaster answers")));
            }
            Sec::Verse(f) => {
                if let Sec::Break(l) = prev {
                    self.ev.push(Ev::Think(l.player(), "following", "back in the band".into()));
                }
                let line: &'static str = match f {
                    0 => "all aboard — train beat!",
                    1 => "grab a partner — two-step!",
                    _ => "shuffle it, partner",
                };
                self.ev.push(Ev::Say(line, 0));
                self.ev.push(Ev::Think(2, "following", match f {
                    0 => "train beat: snare 16ths, accents on 2 and 4, cowbell",
                    1 => "two-step: kick on 1 and 3, cross-stick on 2 and 4",
                    _ => "shuffle: swung 8ths on the hat, backbeat",
                }.into()));
            }
            _ => {}
        }
        self.ev.push(Ev::Section(n));
    }

    pub(super) fn country_step(&mut self, i: &In) {
        let s = i.s;
        self.cs.s = s;
        if s == 0 {
            if self.cs.count_in {
                self.cs.count_in = false;
                self.cs.bar = u32::MAX;
            }
            self.cs.bar = self.cs.bar.wrapping_add(1);
            if self.cs.bar == 0 && self.cs.abs == 0 {
                self.ev.push(Ev::Section(self.sec));
                self.ev.push(Ev::Say("Howdy folks! grab a partner — country night", 2));
            } else if self.cs.bar >= self.cs.len {
                let n = self.cs.next.take().unwrap_or(Sec::Verse(0));
                self.c_enter(n);
            }
        }
        // look ahead: the next section's chords a beat before the bar line
        if s == 12 && self.cs.bar + 1 == self.cs.len {
            let n = self.c_next();
            self.cs.next = Some(n);
            self.c_plan(n);
        }
        let bar = self.cs.bar.min(self.cs.len - 1) as usize;
        let t = i.tonic;
        let r = i.root;
        let next = self.cs.chords.get((bar + 1) % 8).map(|c| c.to_string());
        let changes = next.as_deref().map_or(false, |c| Some(c) != self.cs.chords.get(bar).copied());
        let next_root = next.as_deref().map(|c| crate::harmony::roman(c, crate::harmony::Key { tonic: t, minor: false }).root).unwrap_or(r);
        self.c_drums(s, bar);
        self.c_bass(s, r, next_root, changes, bar);
        self.c_piano(s, r, bar);
        self.c_steel(s, r, bar);
        self.c_lead(s, t, r, bar);
        self.cs.abs += 1;
    }

    fn c_drums(&mut self, s: usize, bar: usize) {
        let feel = if let Sec::Verse(f) = self.sec { f } else if self.sec == Sec::Chorus { 0 } else { 1 };
        let last = bar + 1 == self.cs.len as usize;
        match feel {
            0 => {
                // the train: brushed/snare 16ths, accents on the "and"s and on 2 & 4
                let v = match s % 4 {
                    0 => 44,
                    2 => 58,
                    _ => 30,
                } + if s == 4 || s == 12 { 40 } else { 0 };
                self.drum(38, (v as f32 * (0.85 + 0.3 * self.int)) as i32, 0.0);
                if s % 8 == 0 {
                    self.drum(36, if s == 0 { 92 } else { 80 }, 0.0);
                }
                if s % 4 == 2 && self.int > 0.35 {
                    self.drum(56, if s % 8 == 2 { 46 } else { 38 }, 0.0);
                }
                if s == 4 || s == 12 {
                    self.drum(44, 44, 0.0);
                }
            }
            1 => {
                // two-step: kick 1 & 3, cross-stick 2 & 4, hats on the 8ths
                if s % 8 == 0 {
                    self.drum(36, if s == 0 { 92 } else { 82 }, 0.0);
                }
                if s == 4 || s == 12 {
                    self.drum(if self.sec == Sec::Chorus { 38 } else { 37 }, 82, 0.0);
                }
                if s % 2 == 0 {
                    self.drum(42, if s % 4 == 0 { 58 } else { 42 }, 0.0);
                }
            }
            _ => {
                // shuffle: quarter + the swung "and" on the hat, backbeat, kick 1 & 3
                if s % 4 == 0 {
                    self.drum(42, 56, 0.0);
                    self.drum(42, 40, SWING);
                    if s % 8 == 0 {
                        self.drum(36, 86, 0.0);
                    } else {
                        self.drum(38, 86, 0.0);
                    }
                }
            }
        }
        if bar == 0 && s == 0 && self.cs.abs > 32 {
            self.drum(49, 76, 0.0);
        }
        if last && s >= 12 && s % 2 == 0 {
            self.drum([48, 45][(s - 12) / 2], 74, 0.0);
        }
    }

    fn c_bass(&mut self, s: usize, r: i32, next: i32, changes: bool, bar: usize) {
        let root = fold(r, -17, -6);
        let fifth = if root + 7 > -3 { root - 5 } else { root + 7 };
        let two_step = matches!(self.sec, Sec::Verse(1) | Sec::Break(_));
        // a walk-up into the next chord on the last beat
        if changes && s >= 8 && bar % 2 == 1 {
            let tgt = fold(next, -17, -6);
            let steps = [root, root + 2, root + 4, tgt - 1];
            if s % 2 == 0 {
                let k = (s - 8) / 2;
                let n = if k == 3 { tgt - 1 } else { steps[k].min(tgt + 2) };
                self.c_note(BASS, n, 78, 1.6, 0.0, 1);
                self.ev.push(Ev::Sub(n, 70));
            }
            return;
        }
        let hit = if two_step { s % 4 == 0 } else { s % 8 == 0 };
        if hit {
            let n = if (s / 4) % 2 == 0 { if bar % 2 == 0 || s != 0 { root } else { fifth } } else { fifth };
            let n = if s == 0 { root } else { n };
            self.c_note(BASS, n, if s == 0 { 92 } else { 80 }, if two_step { 3.2 } else { 6.0 }, 0.0, 1);
            self.ev.push(Ev::Sub(n, if s == 0 { 90 } else { 76 }));
        }
    }

    fn c_piano(&mut self, s: usize, r: i32, bar: usize) {
        let soloing = matches!(self.sec, Sec::Break(_));
        // the "chick": triads on 2 and 4 (and the "and" of 4 in the train)
        if s == 4 || s == 12 || (s == 14 && matches!(self.sec, Sec::Verse(0)) && self.rng.chance(0.4)) {
            let base = fold(r, 10, 21);
            let v = if soloing { 46 } else { 58 };
            for (k, iv) in [0, 4, 7].iter().enumerate() {
                self.c_note(PIANO, base + iv, v - k as i32 * 3, 1.2, 0.0, 3);
            }
        }
        // honky-tonk fill: a grace slide from the b3 into the 3, then a run down
        if bar % 4 == 3 && s == 8 && !soloing && self.rng.chance(0.5) {
            let b = fold(r, 22, 33);
            self.c_note(PIANO, b + 3, 54, 0.3, 0.0, 3);
            self.c_note(PIANO, b + 4, 64, 1.0, 0.25, 3);
            for (k, iv) in [0, -3, -5, -8].iter().enumerate() {
                self.c_note(PIANO, b + iv, 56, 1.0, 2.0 + 1.0 * k as f32, 3);
            }
        }
    }

    fn c_steel(&mut self, s: usize, r: i32, bar: usize) {
        if matches!(self.sec, Sec::Break(Lead::Steel)) {
            return;
        }
        // pedal steel: slides into a 3rd + 5th (or 6th + root) on each chord, a swell
        if s == 0 && (bar % 2 == 0 || self.rng.chance(0.4)) && self.rng.chance(0.85) {
            let base = fold(r, 12, 23);
            let (a, b) = if self.rng.chance(0.6) { (4, 7) } else { (9, 12) };
            let v = if self.sec == Sec::Chorus { 60 } else { 50 };
            self.c_note(STEEL, base + a, v, 14.0, 0.0, 0);
            self.c_note(STEEL, base + b, v - 6, 14.0, 0.02, 0);
            self.ev.push(Ev::Bend(STEEL, 4096, 0.0));
            for k in 1..=6 {
                self.ev.push(Ev::Bend(STEEL, 4096 + 683 * k, k as f32 * 0.35));
            }
            // the end of the phrase: a slow slide down
            if bar % 4 == 3 {
                for k in 1..=4 {
                    self.ev.push(Ev::Bend(STEEL, 8192 - 1024 * k, 10.0 + k as f32 * 0.8));
                }
                self.ev.push(Ev::Bend(STEEL, 8192, 15.5));
            }
        }
    }

    /// a country line: 8ths and 16ths on the major pentatonic (plus the b7 in Mixolydian)
    fn c_phrase(&mut self, t: i32, r: i32, lo: i32, hi: i32, heat: f32, bars: usize, chicken: bool) -> Vec<PN> {
        let pcs: &[i32] = if self.cs.mixo { &[0, 2, 4, 7, 9, 10] } else { &[0, 2, 4, 7, 9] };
        let ladder: Vec<i32> = (lo..=hi).filter(|&x| pcs.contains(&(x - t).rem_euclid(12))).collect();
        if ladder.is_empty() {
            return Vec::new();
        }
        const CELLS: [&[(f32, f32)]; 6] = [
            &[(0.0, 2.0), (2.0, 2.0)],
            &[(0.0, 1.0), (1.0, 1.0), (2.0, 2.0)],
            &[(0.0, 4.0)],
            &[(0.0, 3.0), (3.0, 1.0)],
            &[(0.0, 1.0), (1.0, 1.0), (2.0, 1.0), (3.0, 1.0)],
            &[],
        ];
        let w = [1.0, 0.5 + heat, 0.6, 0.5, 0.2 + heat * 1.2, 0.35];
        let mut k = ((0.3 + 0.4 * heat) * ladder.len() as f32) as i32;
        let mut out = Vec::new();
        let beats = bars * 4;
        for b in 0..beats {
            let cell = if b + 1 == beats { CELLS[2] } else { CELLS[self.rng.weighted(&w)] };
            for &(at, len) in cell {
                k = (k + [-2, -1, -1, 1, 1, 2, 0][self.rng.below(7)]).clamp(0, ladder.len() as i32 - 1);
                let mut semi = ladder[k as usize];
                if b + 1 == beats {
                    // land on the chord's root or third
                    let ct = [(r - t).rem_euclid(12), (r + 4 - t).rem_euclid(12)];
                    if let Some(&x) = ladder.iter().filter(|&&x| ct.contains(&(x - t).rem_euclid(12))).min_by_key(|&&x| (x - semi).abs()) {
                        semi = x;
                    }
                }
                let bend = if chicken && len <= 2.0 && self.rng.chance(0.18) { 3 } else if !chicken && len >= 3.0 && self.rng.chance(0.4) { 1 } else { 0 };
                let vel = if chicken { if self.rng.chance(0.3) { 34 } else { 84 } } else { 72 };
                out.push(PN { at: b as f32 * 4.0 + at, semi, len: if chicken { len.min(0.6) } else { len * 0.92 }, bend, vel });
                // chicken pickin' double stop: a sixth below on the beat
                if chicken && at == 0.0 && b % 2 == 0 && self.rng.chance(0.4) {
                    out.push(PN { at: b as f32 * 4.0, semi: semi - 5, len: 0.5, bend: 0, vel: 70 });
                }
            }
        }
        out
    }

    fn c_lead(&mut self, s: usize, t: i32, r: i32, bar: usize) {
        if s == 0 {
            let heat = (0.25 + 0.4 * self.int).min(1.0);
            let pick: Option<(Vec<PN>, u8, usize)> = match self.sec {
                Sec::Verse(_) => match bar {
                    3 | 7 if self.rng.chance(0.8) => {
                        // a fill: fiddle / harp in bar 3, the Tele in bar 7 (or the other way)
                        let tele = (bar == 7) ^ (self.cs.verses % 2 == 1);
                        if tele {
                            Some((self.c_phrase(t, r, 5, 22, heat, 1, true), TELE, 4))
                        } else {
                            Some((self.c_phrase(t, r, 12, 26, heat, 1, false), SOLO, 5))
                        }
                    }
                    _ => None,
                },
                Sec::Chorus => match bar {
                    // call (2 bars), response (2 bars), the call again, the answer that resolves
                    0 => {
                        let p = self.c_phrase(t, r, 12, 26, heat, 2, false);
                        self.cs.call = p.clone();
                        Some((p, SOLO, 5))
                    }
                    2 | 6 => Some((self.c_phrase(t, r, 5, 22, heat + 0.1, 2, true), TELE, 4)),
                    4 => Some((self.cs.call.clone(), SOLO, 5)),
                    _ => None,
                },
                Sec::Break(l) if bar % 2 == 0 => {
                    let h = (heat + 0.12 * bar as f32 / 2.0).min(1.0);
                    let (lo, hi) = l.range();
                    let (ch, who, chicken) = match l {
                        Lead::Tele => (TELE, 4, true),
                        Lead::Steel => (STEEL, 0, false),
                        _ => (SOLO, 5, false),
                    };
                    Some((self.c_phrase(t, r, lo, hi, h, 2, chicken), ch, who))
                }
                _ => None,
            };
            if let Some((p, ch, who)) = pick {
                self.cs.ph = p;
                self.cs.ph_at = self.cs.abs;
                self.cs.ph_ch = ch;
                self.cs.ph_who = who;
            }
        }
        // the Tele's chicken "cluck" on 2 and 4 when it isn't playing a line
        if self.cs.ph.is_empty() || self.cs.ph_ch != TELE {
            if (s == 4 || s == 12) && !matches!(self.sec, Sec::Break(Lead::Tele)) {
                let b = fold(r, 7, 18);
                self.c_note(TELE, b + 4, 40, 0.3, 0.0, 4);
                self.c_note(TELE, b + 7, 36, 0.3, 0.01, 4);
            }
        }
        if self.cs.ph.is_empty() {
            return;
        }
        let rel = (self.cs.abs - self.cs.ph_at) as f32;
        let (ch, who) = (self.cs.ph_ch, self.cs.ph_who);
        let notes: Vec<PN> = self.cs.ph.iter().copied().filter(|n| n.at >= rel && n.at < rel + 1.0).collect();
        for n in notes {
            let d = n.at - rel;
            self.c_note(ch, n.semi, n.vel, n.len, d, who);
            match n.bend {
                3 => self.bend(ch, 3, d, n.len),
                1 => self.bend(ch, 1, d, n.len),
                _ => {}
            }
        }
        let end = self.cs.ph.iter().map(|n| n.at + n.len).fold(0.0, f32::max);
        if rel + 1.0 >= end {
            self.cs.ph.clear();
        }
    }
}
