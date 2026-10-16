//! The Metal band (a third personality of the rage machine): down-tuned, palm-muted chugs with a
//! deep low end, Phrygian / harmonic-minor / diminished riffs, double kick, blast beats and
//! half-time breakdowns, growls and screams with the odd gang chant, shred solos and harmonized
//! twin leads. Tempo surges ride the song form: the blast pushes the tempo up, the breakdown
//! brings it back down. Synth is nearly absent (an occasional dark pad from the colour seat).

use super::{Ev, In, Kind, Rage, Sec, Trig, V_A, V_E, V_O, V_U};

/// riff rhythms: X = accented open power chord, x = palm-muted chug ('.' rest)
const M_RHYTHMS: [&str; 8] = [
    "XxxxXxx.XxxxX.xx",
    "X.xxX.xxX.xxXxxx",
    "Xxx.xxX.xxX.Xxxx",
    "XxXxXxxxXxXxXxxx",
    "X..xX..xXx.xX.x.",
    "Xxxx.xxxXxxx.xXx",
    "XxxXxxXxXxxXxxXx",
    "X.x.xxX.x.xxX.xx",
];
/// breakdown chug patterns (bit s): syncopated, the kick doubles every chug
const M_BREAKS: [u16; 5] = [0x2499, 0x0A49, 0x2929, 0x1249, 0x4A25];

/// chord plans per section (natural minor romans; V is the harmonic-minor dominant)
const M_VERSE: [&[&str]; 4] = [
    &["i", "i", "bII", "i", "i", "i", "VI", "V"],
    &["i", "VI", "bII", "i", "i", "VI", "V", "V"],
    &["i", "i", "bv", "i", "i", "bII", "#vii°", "V"],
    &["i", "bII", "i", "bII", "VI", "bII", "V", "V"],
];
const M_BLAST: [&[&str]; 2] = [&["i", "i", "bII", "bII"], &["i", "bv", "i", "bII"]];
const M_BREAK: [&[&str]; 2] = [&["i", "i", "i", "i"], &["i", "i", "bII", "i"]];
const M_SOLO: &[&str] = &["i", "iv", "V", "i", "VI", "bII", "V", "V"];

impl Rage {
    pub(super) fn metal_start(&mut self) {
        self.metal_riff();
        self.go(Sec::Verse, 8);
    }

    /// a riff from one of the three metal modes
    pub(super) fn metal_riff(&mut self) {
        let rh: Vec<char> = M_RHYTHMS[self.rng.below(M_RHYTHMS.len())].chars().collect();
        self.riff.mode = 3 + self.rng.below(3) as u8;
        let pal: &[i32] = match self.riff.mode {
            3 => &[0, 0, 0, 0, 1, 1, 3, 5, 7, -2],  // Phrygian: the b2 grind
            4 => &[0, 0, 0, 2, 3, 7, 8, -1, 1],     // harmonic minor / Phrygian dominant
            _ => &[0, 0, 0, 3, 6, 9, 1, 4],         // diminished: minor thirds and the tritone
        };
        for s in 0..16 {
            self.riff.hit[s] = rh[s] != '.';
            self.riff.acc[s] = rh[s] == 'X';
            self.riff.off[s] = if rh[s] == 'X' || s < 4 {
                0
            } else if s >= 10 || self.rng.chance(0.35) {
                pal[self.rng.below(pal.len())]
            } else {
                0
            };
        }
        self.kpat = self.rng.below(M_BREAKS.len());
    }

    pub(super) fn metal_plan(&mut self, sec: Sec) {
        let (chords, why): (&'static [&'static str], &str) = match sec {
            Sec::Verse => (M_VERSE[self.verse_prog % 4], "thrash riff"),
            Sec::Blast => (M_BLAST[self.rng.below(2)], "blast"),
            Sec::Mosh => (M_BREAK[self.rng.below(2)], "breakdown"),
            Sec::Solo => (M_SOLO, "solo changes (harmonic minor)"),
            _ => return,
        };
        self.ev.push(Ev::Plan(chords, why.into()));
    }

    fn metal_next(&mut self) -> (Sec, u32) {
        let int = self.int;
        match self.sec {
            Sec::Verse => {
                self.choruses += 1;
                if self.choruses == 1 {
                    // the first blast comes early: the style's signature within ~15 s
                    (Sec::Blast, 4)
                } else if self.choruses % 4 == 3 {
                    (Sec::Solo, 8)
                } else if self.rng.chance(0.35 + 0.5 * int) {
                    (Sec::Blast, 4)
                } else {
                    (Sec::Mosh, 4)
                }
            }
            Sec::Blast => if self.rng.chance(0.55) { (Sec::Mosh, 4) } else { (Sec::Verse, 8) },
            Sec::Mosh => (Sec::Verse, 8),
            Sec::Solo => if self.rng.chance(0.5 + 0.3 * int) { (Sec::Blast, 4) } else { (Sec::Mosh, 4) },
            _ => (Sec::Verse, 8),
        }
    }

    fn metal_enter(&mut self, n: Sec, len: u32) {
        self.go(n, len);
        match n {
            Sec::Verse => {
                if self.rng.chance(0.5) {
                    self.metal_riff();
                    self.ev.push(Ev::Log(format!("new riff, {}", self.riff.mode_name())));
                    self.verse_prog = self.rng.below(4);
                }
                self.ev.push(Ev::Tempo(None));
                self.ev.push(Ev::Say(["chug it.", "lock it in.", "heavier!", "riff!"][self.rng.below(4)]));
            }
            Sec::Blast => {
                self.ev.push(Ev::Tempo(Some(14.0 + 10.0 * self.int)));
                self.ev.push(Ev::Say(["BLAST!", "FASTER!", "go go GO!"][self.rng.below(3)]));
                self.mshout("scream");
            }
            Sec::Mosh => {
                self.kpat = self.rng.below(M_BREAKS.len());
                self.ev.push(Ev::Tempo(None));
                self.ev.push(Ev::Say(["BREAKDOWN!", "open the pit!", "MOSH!"][self.rng.below(3)]));
            }
            Sec::Solo => {
                self.make_lick();
                self.ev.push(Ev::Tempo(None));
                self.ev.push(Ev::Say(["SHRED IT!", "take it!", "solo!"][self.rng.below(3)]));
            }
            _ => {}
        }
    }

    /// growl, scream, or the gang "HEY!"
    fn mshout(&mut self, what: &'static str) {
        let base = self.rng.range(170.0, 210.0);
        match what {
            "scream" => self.vox(base * 1.45, 0.75, V_A, V_E, false, 6.5, 0.5, (1.0, 0.9), 0.35),
            "HEY!" => self.vox(base * 1.05, 0.24, V_E, V_A, true, 3.2, 0.3, (1.0, 1.05), 0.0),
            _ => self.vox(base * 0.45, 0.9, V_O, V_U, false, 7.0, 0.5, (0.95, 1.0), 1.0),
        }
        self.ev.push(Ev::Shout(match what {
            "scream" => "RAAAH!",
            "HEY!" => "HEY!",
            _ => "BLEGH!",
        }));
    }

    pub(super) fn step_metal(&mut self, c: &In, step_len: f32, hz: impl Fn(f32) -> f32) {
        debug_assert_eq!(self.kind, Kind::Metal);
        let s = c.s;
        let k = if c.intensity > self.int { 0.25 } else { 0.02 };
        self.int += (c.intensity - self.int) * k;
        if s == 12 && self.sec_bar + 1 >= self.sec_len {
            let next = self.metal_next();
            self.pend_sec = Some(next);
            self.metal_plan(next.0);
        }
        if s == 0 {
            self.sec_bar += 1;
            if self.sec_bar >= self.sec_len {
                let (n, len) = self.pend_sec.take().unwrap_or((Sec::Verse, 8));
                self.metal_enter(n, len);
            }
            self.short = false;
        }
        let root = c.root;
        let fold = |x: i32, lo: i32| lo + (x - lo).rem_euclid(12);
        // down-tuned: the bass sits around C#1-C2 (35-65 Hz), the chugs an octave up
        let bsemi = |off: i32| fold(root + off, -20);
        let gsemi = |off: i32| fold(root + off, -10);
        let last_bar = self.sec_bar + 1 == self.sec_len;
        let int = self.int;
        let (hit, acc, off) = (self.riff.hit[s], self.riff.acc[s], self.riff.off[s]);
        match self.sec {
            Sec::Verse => {
                // thrash: the riff on guitars + bass, the kick locked to it, double kick when hot
                let dbl = int > 0.45 || self.sec_bar >= 4;
                if hit {
                    self.chug(hz(gsemi(off) as f32), if acc { 1.0 } else { 0.72 }, false);
                    self.bass(hz(bsemi(off) as f32), if acc { 1.0 } else { 0.8 }, 0.08, 0.5);
                    self.ev.push(Ev::Bass(bsemi(off), 0.85));
                    if acc || s % 4 == 0 || off != 0 {
                        self.ev.push(Ev::Note(0, gsemi(off), if acc { 110 } else { 88 }, 0.7, 0.0));
                        self.ev.push(Ev::Note(1, gsemi(off), if acc { 106 } else { 84 }, 0.7, 0.03));
                        if acc {
                            self.ev.push(Ev::Note(0, gsemi(off) + 7, 100, 0.7, 0.0));
                        }
                    }
                }
                if (dbl && (hit || s % 2 == 0)) || (!dbl && (acc || s == 0 || s == 8)) {
                    self.kick(if s % 4 == 0 { 1.0 } else { 0.72 }, 60.0, 0.11, 2.2);
                }
                if s == 4 || s == 12 {
                    self.snare(1.0, 0.16, 0.2);
                    self.ev.push(Ev::Gm(38, 116, 0.0));
                }
                if s % 2 == 0 {
                    self.hat(if s % 4 == 0 { 0.6 } else { 0.4 }, false);
                    if s % 4 == 0 {
                        self.ev.push(Ev::Gm(51, 64, 0.0));
                    }
                }
                if s == 0 && self.sec_bar % 4 == 0 {
                    self.ev.push(Ev::Gm(if self.sec_bar == 0 { 49 } else { 52 }, 112, 0.0));
                }
                // growled lines in the odd bars, now and then a gang chant
                if s == 0 && self.sec_bar % 2 == 1 && self.rng.chance(0.6) {
                    self.mshout("growl");
                } else if s == 12 && self.sec_bar == 3 && self.rng.chance(0.25) {
                    self.mshout("HEY!");
                }
                if last_bar && s >= 12 {
                    let toms = [147.0, 110.0, 82.0, 65.0];
                    self.tom(1.0, toms[s - 12]);
                    self.ev.push(Ev::Gm([48, 45, 43, 41][s - 12], 110, 0.0));
                    self.kick(0.8, 60.0, 0.1, 2.2);
                }
            }
            Sec::Blast => {
                // blast beat: kick on every 16th, snare on the off 8ths, cymbal on the 8ths;
                // tremolo-picked guitars and bass on the riff's notes
                self.kick(if s % 2 == 0 { 0.9 } else { 0.7 }, 62.0, 0.08, 2.4);
                if s % 2 == 1 {
                    self.snare(0.85, 0.1, 0.25);
                    if s % 4 == 3 {
                        self.ev.push(Ev::Gm(38, 100, 0.0));
                    }
                }
                if s % 2 == 0 {
                    self.hat(0.5, false);
                    if s % 4 == 0 {
                        self.ev.push(Ev::Gm(if s == 0 { 52 } else { 51 }, if s == 0 { 112 } else { 80 }, 0.0));
                    }
                }
                let o = if s >= 8 { self.riff.off[(s + 4) % 16].min(3) } else { 0 };
                self.chug(hz(gsemi(o) as f32), 0.8, false);
                self.bass(hz(bsemi(o) as f32), 0.85, 0.06, 0.55);
                if s % 2 == 0 {
                    self.ev.push(Ev::Bass(bsemi(o), 0.8));
                }
                if s % 4 == 0 {
                    self.ev.push(Ev::Note(0, gsemi(o), 100, 1.0, 0.0));
                    self.ev.push(Ev::Note(1, gsemi(o) + 7, 94, 1.0, 0.03));
                }
                if s == 0 && self.sec_bar % 2 == 1 && self.rng.chance(0.7) {
                    self.mshout("scream");
                }
            }
            Sec::Mosh => {
                // the half-time breakdown: snare on 3, chugs on the open low string, the kick
                // doubling every chug, china on the quarters, gang chants
                let pat = M_BREAKS[self.kpat % M_BREAKS.len()];
                let on = (pat >> s) & 1 == 1;
                if on {
                    let o = if s == 14 && self.sec_bar % 2 == 1 { 1 } else { 0 };
                    self.chug(hz(gsemi(o) as f32), if s == 0 { 1.0 } else { 0.9 }, false);
                    self.bass(hz(bsemi(o) as f32), 1.0, 0.16, 0.45);
                    self.kick(1.0, 55.0, 0.18, 2.4);
                    self.ev.push(Ev::Bass(bsemi(o), 1.0));
                    self.ev.push(Ev::Note(0, gsemi(o), 116, 1.2, 0.0));
                    self.ev.push(Ev::Note(1, gsemi(o), 112, 1.2, 0.03));
                    self.ev.push(Ev::Note(0, gsemi(o) + 7, 104, 1.2, 0.0));
                } else if s % 2 == 0 {
                    self.choke();
                }
                if s == 8 {
                    self.snare(1.0, 0.24, 0.3);
                    self.ev.push(Ev::Gm(38, 127, 0.0));
                }
                if s % 4 == 0 {
                    self.ev.push(Ev::Gm(if s == 0 { 52 } else { 53 }, if s == 0 { 118 } else { 70 }, 0.0));
                    self.hat(0.45, s == 0);
                }
                if s == 12 && self.sec_bar % 2 == 1 {
                    self.mshout("HEY!");
                    self.later(2.0, step_len, Trig::Snare(0.5));
                } else if s == 0 && self.sec_bar % 2 == 0 && self.rng.chance(0.5) {
                    self.mshout("growl");
                }
                if last_bar && s >= 12 {
                    self.choke();
                }
            }
            Sec::Solo => {
                // double kick under a shred run, then twin leads in thirds
                if s % 2 == 0 || int > 0.6 {
                    self.kick(if s % 4 == 0 { 0.95 } else { 0.7 }, 60.0, 0.1, 2.2);
                }
                if s == 4 || s == 12 {
                    self.snare(1.0, 0.16, 0.2);
                    self.ev.push(Ev::Gm(38, 112, 0.0));
                }
                if s % 4 == 0 {
                    self.ev.push(Ev::Gm(51, 70, 0.0));
                }
                if s % 2 == 0 || hit {
                    self.chug(hz(gsemi(0) as f32), if s % 4 == 0 { 0.85 } else { 0.6 }, false);
                    self.bass(hz(bsemi(0) as f32), 0.85, 0.08, 0.5);
                    if s % 4 == 0 {
                        self.ev.push(Ev::Bass(bsemi(0), 0.8));
                        self.ev.push(Ev::Note(0, gsemi(0), 92, 0.8, 0.0));
                    }
                }
                if self.sec_bar < 4 {
                    self.shred_step(c, s);
                } else {
                    self.solo_step(c, s);
                }
                if last_bar && s >= 12 {
                    self.tom(1.0, [131.0, 98.0, 82.0, 65.0][s - 12]);
                    self.ev.push(Ev::Gm([47, 45, 43, 41][s - 12], 112, 0.0));
                }
            }
            _ => {}
        }
    }

    /// shred: four-note sequences in 16ths up and down the harmonic-minor scale
    fn shred_step(&mut self, c: &In, s: usize) {
        const HM: [i32; 7] = [0, 2, 3, 5, 7, 8, 11];
        let deg_semi = |d: i32| HM[d.rem_euclid(7) as usize] + 12 * d.div_euclid(7);
        let fold = |x: i32, lo: i32| lo + (x - lo).rem_euclid(12);
        let base = fold(c.tonic, 12);
        let bar = self.sec_bar as i32;
        let g = (s / 4) as i32;
        let up = bar % 2 == 0;
        let start = if up { g + 4 * (bar / 2) } else { 10 - g - 4 * (bar / 2) };
        let d = start + if up { [0, 1, 2, 0][s % 4] } else { [0, -1, -2, 0][s % 4] };
        let d = d.clamp(0, 13);
        if bar == 3 && s >= 12 {
            if s == 12 {
                let top = base + deg_semi(11);
                self.ev.push(Ev::Note(4, top, 120, 4.0, 0.0));
                self.ev.push(Ev::Bend(4, 12288, 0.5));
                self.ev.push(Ev::Bend(4, 8192, 3.5));
                self.ev.push(Ev::Heard(4, top, 1.0));
            }
            return;
        }
        let n = base + deg_semi(d);
        self.ev.push(Ev::Note(4, n, if s % 4 == 0 { 112 } else { 96 }, 0.9, 0.0));
        if s % 2 == 0 {
            self.ev.push(Ev::Heard(4, n, 0.9));
        }
    }
}
