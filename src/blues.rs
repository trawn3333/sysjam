//! The Blues style's band: a swinging 12-bar shuffle (I7–IV7–V7, quick change now and
//! then, a turnaround) where every player swings the same triplet grid: the "and" of
//! each beat lands two thirds of the way through it (a 2:1, ~66 % swing ratio) for the
//! drums, the boogie-woogie walking bass (1-3-5-6-b7-6-5-3), the piano's stabs on the
//! "and"s and boogie left hand, the guitar's 5-6 shuffle chug and call-and-response
//! licks, the warm organ bed and the harmonica's wails and bends.
//!
//! Melody comes only from blues scales: the minor blues (1 b3 4 b5 5 b7), the major
//! blues (1 2 b3 3 5 6) and Mixolydian over the V7. The "synth seat" plays featured
//! solos instead: 1-3 choruses of harmonica (first), slide / lead guitar, tenor sax or a
//! piano break, with AAB phrasing (a riff, the riff again over the IV, an answer) and
//! intensity climbing chorus by chorus while the band backs off.
//!
//! The machine only emits events (GM notes, bends, drum hits, lines for the conductor);
//! synth.rs plays them.

use crate::dsp::Rng;

#[path = "country.rs"]
mod country;
#[path = "celtic.rs"]
mod celtic;

/// which band the session machine is: the 12-bar shuffle, the country band, or the Celtic session
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Blues,
    Country,
    Celtic,
}

/// minor blues, major blues (semitones above the tonic)
const MINB: [i32; 6] = [0, 3, 5, 6, 7, 10];
const MAJB: [i32; 6] = [0, 2, 3, 4, 7, 9];
/// the form, in chord roots above the tonic (I7 / IV7 / V7); quick change swaps bar 2 to IV
const FORM: [i32; 12] = [0, 0, 0, 0, 5, 5, 0, 0, 7, 5, 0, 7];
const ROMAN: [&str; 12] = ["I7", "I7", "I7", "I7", "IV7", "IV7", "I7", "I7", "V7", "IV7", "I7", "V7"];
const ROMAN_QC: [&str; 12] = ["I7", "IV7", "I7", "I7", "IV7", "IV7", "I7", "I7", "V7", "IV7", "I7", "V7"];

/// a swung "and": two thirds of a beat (4 sixteenths) = 2.667 sixteenths
pub const SWING: f32 = 4.0 * 2.0 / 3.0;

/// machine channels (synth.rs maps them to GM channels)
pub const PIANO: u8 = 0;
pub const ORGAN: u8 = 1;
pub const GUITAR: u8 = 2;
pub const SOLO: u8 = 3;
pub const BASS: u8 = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lead {
    Harp,
    Guitar,
    Slide,
    Sax,
    Piano,
    /// country: fiddle, Telecaster (chicken pickin'), pedal steel, harmonica
    Fiddle,
    Tele,
    Steel,
}

impl Lead {
    pub fn name(self) -> &'static str {
        match self {
            Lead::Harp => "harp",
            Lead::Guitar => "guitar",
            Lead::Slide => "slide guitar",
            Lead::Sax => "tenor sax",
            Lead::Piano => "piano",
            Lead::Fiddle => "fiddle",
            Lead::Tele => "Telecaster",
            Lead::Steel => "pedal steel",
        }
    }
    /// which player takes it (3 keys, 4 lead guitar, 5 the colour / "synth" seat)
    pub fn player(self) -> usize {
        match self {
            Lead::Piano => 3,
            Lead::Guitar | Lead::Tele => 4,
            Lead::Steel => 0,
            _ => 5,
        }
    }
    fn prog(self) -> u8 {
        match self {
            Lead::Harp => 22,
            Lead::Sax => 66,
            Lead::Slide | Lead::Guitar => 27,
            Lead::Piano => 0,
            Lead::Fiddle => 110,
            Lead::Tele => 27,
            Lead::Steel => 26,
        }
    }
    fn range(self) -> (i32, i32) {
        match self {
            Lead::Harp => (12, 31),
            Lead::Sax => (3, 24),
            Lead::Guitar | Lead::Slide => (7, 29),
            Lead::Piano => (12, 33),
            Lead::Fiddle => (10, 31),
            Lead::Tele => (5, 26),
            Lead::Steel => (8, 28),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sec {
    /// the band's chorus: harp calls, guitar answers
    Groove,
    /// a featured solo: (lead, chorus k of n)
    Solo(Lead, u8, u8),
    /// country: a verse (0 train beat, 1 two-step, 2 shuffle)
    Verse(u8),
    /// country: the call-and-response chorus
    Chorus,
    /// country: an instrumental break (fiddle, Telecaster, steel, harmonica)
    Break(Lead),
    /// celtic: a tune (0 jig, 1 reel, 2 slip jig; part 0..4 = A A B B; tune n of the set)
    Tune(u8, u8, u8),
}

pub enum Ev {
    /// (machine channel, semitone rel A2, velocity, length in 16ths, delay in 16ths, player)
    Note(u8, i32, i32, f32, f32, usize),
    /// pitch bend (channel, 0..16383, delay)
    Bend(u8, i32, f32),
    /// GM drum (key, velocity, delay)
    Drum(i32, i32, f32),
    /// a program change on a machine channel
    Prog(u8, u8),
    /// the conductor speaks (text, 0 calm / 1 point / 2 surge)
    Say(&'static str, u8),
    /// a player's thought (player, status, text)
    Think(usize, &'static str, String),
    Log(String),
    /// next chorus' chords (one per bar)
    Plan(Vec<&'static str>),
    Section(Sec),
    /// a new home key (pitch class above C, minor?)
    Key(i32, bool),
    /// ramp the tempo to this bpm over a bar or two
    Bpm(f32),
    /// the deep sine under the bass (semitone rel A2, velocity)
    Sub(i32, i32),
}

pub struct In {
    pub s: usize,
    /// the band key's tonic and the current chord root (semitone space)
    pub tonic: i32,
    pub root: i32,
    /// 0..1: energy, load and typing
    pub intensity: f32,
    pub bpm: f32,
}

#[derive(Clone, Copy)]
struct N {
    /// triplet units from the phrase start (3 per beat)
    u: u16,
    semi: i32,
    len: u16,
    /// 0 none, 1 scoop up a semitone, 2 wail (scoop + vibrato), 3 bend up a whole tone
    bend: u8,
}

pub struct Blues {
    rng: Rng,
    pub ev: Vec<Ev>,
    pub sec: Sec,
    /// bar within the 12-bar form (None = the count-in bar)
    pub form: Option<usize>,
    pub chorus: u32,
    pub progs: [u8; 5],
    quick: bool,
    next_quick: bool,
    /// the phrase being played on the solo channel / guitar: (notes, start bar in form, channel, player)
    phrase: Vec<N>,
    ph_bar: usize,
    ph_ch: u8,
    ph_who: usize,
    /// the A phrase, remembered for the repeat (AAB)
    riff_a: Vec<N>,
    solos: u32,
    pub int: f32,
    started: bool,
    pub mode: Mode,
    /// the home key the machine wants (pitch class above C, minor)
    pub home: (i32, bool),
    cs: country::State,
    ks: celtic::State,
}

fn fold(p: i32, lo: i32, hi: i32) -> i32 {
    let mut x = lo + (p - lo).rem_euclid(12);
    if x > hi {
        x -= 12;
    }
    x
}

impl Blues {
    pub fn new(seed: u64) -> Blues {
        Blues {
            rng: Rng::seeded(seed ^ 0xB1_0E5),
            ev: Vec::new(),
            sec: Sec::Groove,
            form: None,
            chorus: 0,
            progs: [0, 17, 27, 22, 33],
            quick: false,
            next_quick: false,
            phrase: Vec::new(),
            ph_bar: 0,
            ph_ch: SOLO,
            ph_who: 5,
            riff_a: Vec::new(),
            solos: 0,
            int: 0.5,
            started: false,
            mode: Mode::Blues,
            home: (4, false),
            cs: country::State::default(),
            ks: celtic::State::default(),
        }
    }

    pub fn active(&self) -> bool {
        self.started
    }

    /// start: the band's instruments, the first chorus' chords
    pub fn start(&mut self, mode: Mode) {
        self.mode = mode;
        match mode {
            Mode::Country => return self.country_start(),
            Mode::Celtic => return self.celtic_start(),
            Mode::Blues => {}
        }
        self.started = true;
        self.form = None;
        self.chorus = 0;
        self.solos = 0;
        self.sec = Sec::Groove;
        self.phrase.clear();
        let piano = [0u8, 1, 3][self.rng.below(3)];
        let organ = [16u8, 17, 18][self.rng.below(3)];
        let gtr = [27u8, 26, 29][self.rng.below(3)];
        let bass = [32u8, 33, 34][self.rng.below(3)];
        self.progs = [piano, organ, gtr, 22, bass];
        for (ch, p) in self.progs.iter().enumerate() {
            self.ev.push(Ev::Prog(ch as u8, *p));
        }
        self.next_quick = self.rng.chance(0.4);
        self.ev.push(Ev::Plan(if self.next_quick { ROMAN_QC.to_vec() } else { ROMAN.to_vec() }));
        self.ev.push(Ev::Think(2, "following", "count-in on the sticks, then the shuffle: ride triplets, backbeat on 2 and 4".into()));
        self.ev.push(Ev::Think(1, "following", "boogie-woogie walk, swung 8ths: 1-3-5-6-b7-6-5-3".into()));
        self.ev.push(Ev::Think(3, "following", "stabs on the \"and\"s, a boogie left hand".into()));
        self.ev.push(Ev::Think(4, "following", "5-6 shuffle chug, answering the harp".into()));
        self.ev.push(Ev::Think(5, "following", "blues harp: calls between the changes".into()));
        self.ev.push(Ev::Think(0, "following", "warm organ bed under it all".into()));
    }

    pub fn stop(&mut self) {
        self.started = false;
    }

    pub fn feel(&self) -> &'static str {
        match self.mode {
            Mode::Country => return self.country_feel(),
            Mode::Celtic => return self.celtic_feel(),
            Mode::Blues => {}
        }
        match self.sec {
            Sec::Groove => "shuffle — 12-bar, swung triplets",
            Sec::Solo(l, k, _) => {
                const T: [[&str; 3]; 5] = [
                    ["SOLO — harp, chorus 1", "SOLO — harp, chorus 2", "SOLO — harp, chorus 3"],
                    ["SOLO — guitar, chorus 1", "SOLO — guitar, chorus 2", "SOLO — guitar, chorus 3"],
                    ["SOLO — slide guitar, chorus 1", "SOLO — slide guitar, chorus 2", "SOLO — slide guitar, chorus 3"],
                    ["SOLO — tenor sax, chorus 1", "SOLO — tenor sax, chorus 2", "SOLO — tenor sax, chorus 3"],
                    ["SOLO — piano break, chorus 1", "SOLO — piano break, chorus 2", "SOLO — piano break, chorus 3"],
                ];
                let li = match l {
                    Lead::Harp => 0,
                    Lead::Guitar => 1,
                    Lead::Slide => 2,
                    Lead::Sax => 3,
                    _ => 4,
                };
                T[li][(k as usize).clamp(1, 3) - 1]
            }
            _ => "shuffle — 12-bar, swung triplets",
        }
    }
    pub fn sec_name(&self) -> &'static str {
        match self.sec {
            Sec::Groove => "shuffle",
            Sec::Solo(..) => "solo",
            Sec::Verse(_) => "verse",
            Sec::Chorus => "chorus",
            Sec::Break(_) => "break",
            Sec::Tune(_, p, _) => ["A part", "A part again", "B part", "B part again"][p as usize % 4],
        }
    }
    pub fn soloist(&self) -> Option<usize> {
        match self.sec {
            Sec::Solo(l, ..) | Sec::Break(l) => Some(l.player()),
            _ => None,
        }
    }
    /// the GM program behind each stage player (0 organ, 1 bass, 3 piano, 4 guitar, 5 solo seat)
    pub fn prog_of(&self, player: usize) -> Option<u8> {
        if self.mode != Mode::Blues {
            return self.mode_prog_of(player);
        }
        match player {
            0 => Some(self.progs[ORGAN as usize]),
            1 => Some(self.progs[BASS as usize]),
            3 => Some(self.progs[PIANO as usize]),
            4 => Some(self.progs[GUITAR as usize]),
            5 => Some(self.progs[SOLO as usize]),
            _ => None,
        }
    }

    /// the machine's GM channel → stage player map and mixer (volume, pan) for its mode
    fn mode_prog_of(&self, player: usize) -> Option<u8> {
        let ch = match (self.mode, player) {
            (Mode::Country, 0) => 1,
            (Mode::Country, 3) => 0,
            (Mode::Country, 4) => 2,
            (Mode::Country, 5) => 3,
            (Mode::Celtic, 0) => 0,
            (Mode::Celtic, 3) => 1,
            (Mode::Celtic, 4) => 2,
            (Mode::Celtic, 5) => 3,
            (_, 1) => 4,
            _ => return None,
        };
        Some(self.progs[ch])
    }
    pub fn mix(&self, ch: u8) -> (f32, i32) {
        let c = ch as usize % 5;
        match self.mode {
            Mode::Blues => ([0.55, 0.36, 0.42, 0.58, 0.72][c], [40, 76, 30, 92, 64][c]),
            // piano, steel, Tele, fiddle / harmonica, bass
            Mode::Country => ([0.46, 0.4, 0.5, 0.56, 0.74][c], [44, 84, 30, 96, 64][c]),
            // pipes drone, DADGAD guitar, fiddle, whistle / flute, bouzouki
            Mode::Celtic => ([0.2, 0.5, 0.5, 0.5, 0.5][c], [64, 40, 84, 50, 70][c]),
        }
    }
    /// the clock's grid: triplets (jigs, the shuffle) and beats per bar (slip jig 3)
    pub fn triplet(&self) -> bool {
        match self.mode {
            Mode::Blues => true,
            Mode::Country => matches!(self.sec, Sec::Verse(2)),
            Mode::Celtic => matches!(self.sec, Sec::Tune(0 | 2, ..)),
        }
    }
    pub fn meter(&self) -> u8 {
        if self.mode == Mode::Celtic && matches!(self.sec, Sec::Tune(2, ..)) { 3 } else { 4 }
    }
    /// does the machine set the tempo itself (Celtic sets speed up tune by tune)
    pub fn owns_tempo(&self) -> bool {
        self.mode == Mode::Celtic
    }
    pub fn plan_why(&self) -> &'static str {
        match self.mode {
            Mode::Blues => "12-bar shuffle",
            Mode::Country => match self.sec {
                Sec::Chorus => "call-and-response chorus",
                Sec::Break(_) => "the break",
                _ => "country changes",
            },
            Mode::Celtic => "the tune's changes",
        }
    }

    fn note(&mut self, ch: u8, semi: i32, vel: i32, len: f32, d: f32, who: usize) {
        self.ev.push(Ev::Note(ch, semi, vel.clamp(1, 127), len, d, who));
    }
    fn drum(&mut self, key: i32, vel: i32, d: f32) {
        self.ev.push(Ev::Drum(key, vel.clamp(1, 127), d));
    }

    /// what comes after this chorus
    fn next_section(&mut self) {
        let next = match self.sec {
            Sec::Solo(l, k, n) if k < n => Sec::Solo(l, k + 1, n),
            Sec::Solo(..) | Sec::Verse(_) | Sec::Chorus | Sec::Break(_) | Sec::Tune(..) => Sec::Groove,
            Sec::Groove => {
                // a solo after every chorus or two of groove (the first one soon)
                if self.chorus >= 1 && (self.solos == 0 || self.rng.chance(0.65)) {
                    let order = [Lead::Harp, Lead::Guitar, Lead::Harp, Lead::Sax, Lead::Piano, Lead::Slide, Lead::Harp];
                    let l = order[self.solos as usize % order.len()];
                    self.solos += 1;
                    let n = 1 + (self.int * 2.2 + self.rng.f() * 0.9) as u8;
                    Sec::Solo(l, 1, n.clamp(1, 3))
                } else {
                    Sec::Groove
                }
            }
        };
        let prev = self.sec;
        self.sec = next;
        match next {
            Sec::Solo(l, 1, n) => {
                if l != Lead::Piano && l != Lead::Guitar {
                    self.progs[SOLO as usize] = l.prog();
                    self.ev.push(Ev::Prog(SOLO, l.prog()));
                }
                let cue: &'static str = match l {
                    Lead::Harp => "take it, harp!",
                    Lead::Guitar => "take it, guitar!",
                    Lead::Slide => "slide it, brother!",
                    Lead::Sax => "blow, sax, blow!",
                    _ => "piano, take it!",
                };
                self.ev.push(Ev::Say(cue, 1));
                self.ev.push(Ev::Log(format!("{} solo: {n} chorus{} of 12-bar, the band backs off", l.name(), if n == 1 { "" } else { "es" })));
                self.ev.push(Ev::Think(l.player(), "soloing", format!("{} solo: riff, riff again, the answer (AAB), climbing", l.name())));
            }
            Sec::Solo(l, k, _) => {
                self.ev.push(Ev::Say(if k == 3 { "Lord have mercy!" } else { "one more time!" }, 2));
                self.ev.push(Ev::Think(l.player(), "soloing", format!("chorus {k}: higher, harder, more bends")));
            }
            Sec::Verse(_) | Sec::Chorus | Sec::Break(_) | Sec::Tune(..) => {}
            Sec::Groove => {
                if let Sec::Solo(l, ..) = prev {
                    self.ev.push(Ev::Say("Lord have mercy!", 2));
                    self.ev.push(Ev::Think(l.player(), "following", "back in the band".into()));
                    if l != Lead::Harp {
                        self.progs[SOLO as usize] = 22;
                        self.ev.push(Ev::Prog(SOLO, 22));
                    }
                } else {
                    self.ev.push(Ev::Say(["shuffle it!", "yeah, that's the groove", "keep it swingin'"][self.rng.below(3)], 0));
                }
            }
        }
        self.ev.push(Ev::Section(next));
    }

    /// one sixteenth
    pub fn step(&mut self, i: &In) {
        self.int += (i.intensity - self.int) * 0.02;
        match self.mode {
            Mode::Country => return self.country_step(i),
            Mode::Celtic => return self.celtic_step(i),
            Mode::Blues => {}
        }
        let s = i.s;
        if s == 0 {
            // the bar line
            self.form = Some(match self.form {
                None => 0,
                Some(11) => 0,
                Some(b) => b + 1,
            });
            if self.form == Some(0) {
                self.chorus += 1;
                self.quick = self.next_quick;
                if self.chorus > 1 {
                    self.next_section();
                } else {
                    self.ev.push(Ev::Section(self.sec));
                    self.ev.push(Ev::Say("Lord have mercy… let's play the blues", 2));
                }
            }
        }
        let Some(bar) = self.form else {
            // the count-in: sticks on the beats
            if s % 4 == 0 {
                self.drum(37, 50 + (s as i32 / 4) * 8, 0.0);
            }
            return;
        };
        // look ahead: the next chorus' chords go to the harmony a beat before the bar line
        if bar == 11 && s == 12 {
            self.next_quick = self.rng.chance(0.4);
            self.ev.push(Ev::Plan(if self.next_quick { ROMAN_QC.to_vec() } else { ROMAN.to_vec() }));
        }
        let solo = matches!(self.sec, Sec::Solo(..));
        let back = if solo { 0.78 } else { 1.0 };
        let t = i.tonic;
        let r = i.root;
        let beat = s / 4;
        let next_root = {
            let nb = (bar + 1) % 12;
            let f = if self.quick && nb == 1 { 5 } else { FORM[nb] };
            t + f
        };
        if s % 4 == 0 {
            self.drums(bar, beat, solo);
            self.bass(r, beat, back, bar);
            self.piano(r, next_root, beat, bar, back);
            self.organ(r, beat, bar, back);
            self.guitar(r, beat, bar, back);
            self.lead(t, r, beat, bar);
        }
    }

    // ------------------------------------------------------------ the rhythm section

    fn drums(&mut self, bar: usize, beat: usize, solo: bool) {
        let hot = self.int > 0.45 || solo;
        let ride = if hot { 51 } else { 42 };
        let backbeat = beat == 1 || beat == 3;
        let lift = if solo { if let Sec::Solo(_, k, _) = self.sec { k as i32 * 4 } else { 0 } } else { 0 };
        // the shuffle: quarter + the swung "and" (and now and then the middle triplet, softly)
        self.drum(ride, if backbeat { 64 } else { 56 } + lift, 0.0);
        self.drum(ride, 42 + lift, SWING);
        if self.rng.chance(0.12) {
            self.drum(ride, 22, SWING / 2.0);
        }
        if ride == 51 && backbeat {
            self.drum(44, 40, 0.0); // hi-hat foot on 2 and 4
        }
        // backbeat snare, accented; a ghost on the "and" now and then
        if backbeat {
            self.drum(38, 92 + lift, 0.0);
        } else if self.rng.chance(0.18) {
            self.drum(38, 26, SWING);
        }
        // the kick: 1 and 3, pushing into 3 and into the next bar
        if beat == 0 || beat == 2 {
            self.drum(36, if beat == 0 { 86 } else { 74 }, 0.0);
        }
        if beat == 1 && self.rng.chance(0.45) {
            self.drum(36, 58, SWING); // the push into 3
        }
        if beat == 3 && self.rng.chance(0.3) && bar != 11 {
            self.drum(36, 54, SWING); // anticipating the next bar
        }
        // a crash on the top of the form, a triplet fill into it
        if bar == 0 && beat == 0 && self.chorus > 1 {
            self.drum(49, 78, 0.0);
        }
        if bar == 11 && beat == 3 {
            let toms = [50, 48, 47, 45, 43, 41];
            let k = self.rng.below(3);
            for j in 0..3 {
                self.drum(toms[(k + j) % 6], 70 + j as i32 * 6, j as f32 * 4.0 / 3.0);
            }
        }
    }

    fn bass(&mut self, r: i32, beat: usize, back: f32, bar: usize) {
        // boogie-woogie walk in swung 8ths: 1-3-5-6-b7-6-5-3
        const WALK: [i32; 8] = [0, 4, 7, 9, 10, 9, 7, 4];
        let root = fold(r, -17, -6);
        let (a, b) = (WALK[beat * 2], WALK[beat * 2 + 1]);
        // the last turnaround bar walks up to the V
        let (a, b) = if bar == 11 && beat == 3 { (7, 10) } else { (a, b) };
        let acc = if beat == 1 || beat == 3 { 6 } else { 0 };
        let v = ((84 + acc) as f32 * back.max(0.9)) as i32;
        self.note(BASS, root + a, v, 2.2, 0.0, 1);
        self.note(BASS, root + b, v - 12, 1.2, SWING, 1);
    }

    fn piano(&mut self, r: i32, next: i32, beat: usize, bar: usize, back: f32) {
        let soloing = matches!(self.sec, Sec::Solo(Lead::Piano, ..));
        let guitar_solo = matches!(self.sec, Sec::Solo(Lead::Guitar | Lead::Slide, ..));
        // the boogie left hand: in the piano break and under a guitar solo
        if soloing || guitar_solo {
            let lh = fold(r, -5, 6);
            let six = if beat % 2 == 0 { 7 } else { 9 };
            let v = if soloing { 64 } else { 50 };
            self.note(PIANO, lh, v, 1.8, 0.0, 3);
            self.note(PIANO, lh + six, v - 6, 1.8, 0.0, 3);
            self.note(PIANO, lh, v - 10, 1.0, SWING, 3);
            self.note(PIANO, lh + six, v - 14, 1.0, SWING, 3);
        }
        if soloing {
            return;
        }
        // right-hand stabs on the "and"s (3, b7, 9 shells), accented on 2 and 4;
        // the "and" of 4 anticipates the next chord when it changes
        let target = if beat == 3 && next.rem_euclid(12) != r.rem_euclid(12) { next } else { r };
        let base = fold(target, 9, 20);
        let chance = match beat {
            1 | 3 => 0.8,
            _ => 0.35,
        } * if back < 1.0 { 0.5 } else { 1.0 };
        if self.rng.chance(chance) {
            let v = ((if beat % 2 == 1 { 62 } else { 50 }) as f32 * back) as i32;
            let shell = if self.rng.chance(0.5) { [4, 10, 14] } else { [10, 16, 21] };
            for (k, iv) in shell.iter().enumerate() {
                self.note(PIANO, base + iv, v - k as i32 * 3, 0.9, SWING, 3);
            }
        }
        // a little triplet tremolo / roll into the turnaround
        if bar == 11 && beat == 2 && back >= 1.0 && self.rng.chance(0.5) {
            let b = fold(r, 21, 32);
            for j in 0..3 {
                self.note(PIANO, b + [7, 4, 0][j], 52, 1.0, j as f32 * 4.0 / 3.0, 3);
            }
        }
    }

    fn organ(&mut self, r: i32, beat: usize, bar: usize, back: f32) {
        // a warm bed: the dom7 held through the bar, re-voiced on each change, swelling
        if beat != 0 {
            return;
        }
        let _ = bar;
        let base = fold(r, 3, 14);
        let v = (44.0 * back) as i32;
        for iv in [0, 4, 7, 10] {
            self.note(ORGAN, base + iv, v, 15.5, 0.0, 0);
        }
    }

    fn guitar(&mut self, r: i32, beat: usize, bar: usize, back: f32) {
        // guitar solo or answering lick: the lead() part plays it
        if matches!(self.sec, Sec::Solo(Lead::Guitar | Lead::Slide, ..)) || (self.ph_ch == GUITAR && !self.phrase.is_empty() && self.in_phrase(bar)) {
            return;
        }
        // the 5-6 shuffle chug, both 8ths of every beat
        let root = fold(r, -5, 6);
        let six = if beat % 2 == 0 { 7 } else if bar % 2 == 1 && beat == 3 && self.rng.chance(0.4) { 10 } else { 9 };
        let v = ((if beat % 2 == 1 { 66 } else { 58 }) as f32 * back) as i32;
        for (d, dv) in [(0.0, 0), (SWING, -10)] {
            self.note(GUITAR, root, v + dv, 1.3, d, 4);
            self.note(GUITAR, root + six, v + dv - 4, 1.3, d + 0.03, 4);
        }
    }

    // ------------------------------------------------------------ melody

    fn in_phrase(&self, bar: usize) -> bool {
        let rel = (bar + 12 - self.ph_bar) % 12;
        let units = self.phrase.iter().map(|n| n.u + n.len).max().unwrap_or(0) as usize;
        rel * 12 < units.max(1)
    }

    /// a phrase in blues scales: rhythm from swung templates, pitches walking a ladder
    fn make_phrase(&mut self, t: i32, r: i32, lo: i32, hi: i32, heat: f32, v_bar: bool) -> Vec<N> {
        const RH: [&[(u16, u16)]; 7] = [
            &[(0, 2), (2, 1), (3, 2), (5, 1), (6, 6)],
            &[(2, 1), (3, 3), (6, 2), (8, 1), (9, 6)],
            &[(0, 1), (1, 1), (2, 1), (3, 1), (4, 1), (5, 1), (6, 6)],
            &[(0, 9), (9, 1), (11, 1), (12, 6)],
            &[(0, 1), (2, 1), (3, 1), (5, 1), (6, 1), (8, 1), (9, 4)],
            &[(0, 2), (2, 1), (3, 2), (5, 1), (6, 2), (8, 1), (9, 2), (11, 1), (12, 6)],
            &[(2, 1), (3, 1), (5, 1), (6, 1), (8, 1), (9, 1), (11, 1), (12, 1), (14, 1), (15, 6)],
        ];
        let w = [1.0, 1.0, 0.4 + heat, 0.8 + heat * 0.5, 0.3 + heat * 1.2, 0.6 + heat, 0.2 + heat * 1.4];
        let rh = RH[self.rng.weighted(&w)];
        // the scale: minor blues mostly, major blues for a sweeter line; Mixolydian over the V7
        let pcs: Vec<i32> = if v_bar {
            [0, 2, 4, 5, 7, 9, 10].iter().map(|p| (p + r - t).rem_euclid(12)).collect()
        } else if self.rng.chance(0.35) {
            MAJB.to_vec()
        } else {
            MINB.to_vec()
        };
        let mut ladder: Vec<i32> = (lo..=hi).filter(|&x| pcs.contains(&(x - t).rem_euclid(12))).collect();
        if ladder.is_empty() {
            ladder.push(fold(t, lo, hi));
        }
        let top = ladder.len() as f32;
        let mut k = ((0.25 + 0.5 * heat + self.rng.f() * 0.2) * top) as i32;
        let riff = rh.len() >= 7 && self.rng.chance(0.5);
        let mut out = Vec::new();
        let n = rh.len();
        for (j, &(u, len)) in rh.iter().enumerate() {
            if riff {
                k += if j % 2 == 0 { 1 } else { -1 }; // a repeated two-note figure
            } else {
                k += [-2, -1, -1, 1, 1, 2, 0][self.rng.below(7)];
            }
            k = k.clamp(0, ladder.len() as i32 - 1);
            let mut semi = ladder[k as usize];
            if j + 1 == n {
                // land on a chord tone (root, 3rd or 5th of the chord, if it's in the scale)
                let ct: Vec<i32> = [0, 4, 7].iter().map(|iv| (r + iv - t).rem_euclid(12)).filter(|p| pcs.contains(p) || *p == 0).collect();
                if let Some(&x) = ladder.iter().filter(|&&x| ct.contains(&(x - t).rem_euclid(12))).min_by_key(|&&x| (x - semi).abs()) {
                    semi = x;
                }
            }
            // the V7's leading tone only while the V7 lasts (the phrase may run into the IV)
            if v_bar && u >= 12 && (semi - t).rem_euclid(12) == 11 {
                semi += 1;
            }
            let pc = (semi - t).rem_euclid(12);
            let bend = if len >= 6 && self.rng.chance(0.4 + heat * 0.4) {
                2
            } else if (pc == 3 || pc == 6 || pc == 10) && self.rng.chance(0.6) {
                1
            } else if pc == 5 && self.rng.chance(0.3 + heat * 0.3) && pcs.contains(&7) {
                // bend the 4 up to the 5
                semi += 2;
                3
            } else {
                0
            };
            out.push(N { u, semi, len, bend });
        }
        out
    }

    /// over the IV chord the riff's major third becomes the blue third
    fn over_iv(a: &[N], t: i32) -> Vec<N> {
        a.iter().map(|n| {
            let mut m = *n;
            if (m.semi - t).rem_euclid(12) == 4 {
                m.semi -= 1;
            }
            m
        }).collect()
    }

    fn lead(&mut self, t: i32, r: i32, beat: usize, bar: usize) {
        // decide at the top of each phrase bar
        if beat == 0 {
            match self.sec {
                Sec::Solo(l, k, n) => {
                    let heat = (0.25 + 0.3 * (k - 1) as f32 + 0.2 * self.int + if k == n { 0.15 } else { 0.0 }).min(1.0);
                    let (lo0, hi0) = l.range();
                    // climbing: each chorus a little higher
                    let lift = (k as i32 - 1) * 3;
                    let (lo, hi) = (lo0 + lift, hi0 + lift.min(5));
                    let (ch, who) = match l {
                        Lead::Piano => (PIANO, 3),
                        Lead::Guitar => (GUITAR, 4),
                        _ => (SOLO, 5),
                    };
                    let a = match bar {
                        0 => {
                            let p = self.make_phrase(t, r, lo, hi, heat, false);
                            self.riff_a = p.clone();
                            Some(p)
                        }
                        2 if heat > 0.6 => Some(self.make_phrase(t, r, lo, hi, heat, false)),
                        4 => Some(Self::over_iv(&self.riff_a, t)),
                        6 if heat > 0.5 => Some(self.make_phrase(t, r, lo, hi, heat * 0.8, false)),
                        8 => Some(self.make_phrase(t, r, lo, hi, (heat + 0.15).min(1.0), true)),
                        10 => Some(self.make_phrase(t, r, lo, hi, heat * 0.7, false)),
                        _ => None,
                    };
                    if let Some(p) = a {
                        self.phrase = p;
                        self.ph_bar = bar;
                        self.ph_ch = ch;
                        self.ph_who = who;
                    }
                }
                Sec::Verse(_) | Sec::Chorus | Sec::Break(_) | Sec::Tune(..) => {}
                Sec::Groove => {
                    // harp calls (A, A over the IV, B), the guitar answers
                    let heat = 0.2 + 0.3 * self.int;
                    let a = match bar {
                        0 if self.rng.chance(0.85) => {
                            let p = self.make_phrase(t, r, 12, 28, heat, false);
                            self.riff_a = p.clone();
                            Some((p, SOLO, 5))
                        }
                        4 if !self.riff_a.is_empty() => Some((Self::over_iv(&self.riff_a, t), SOLO, 5)),
                        8 if self.rng.chance(0.85) => Some((self.make_phrase(t, r, 12, 28, heat, true), SOLO, 5)),
                        2 | 6 | 10 if self.rng.chance(0.6) => {
                            let p: Vec<N> = self.make_phrase(t, r, 7, 24, heat * 0.8, false).into_iter().filter(|n| n.u < 12).collect();
                            Some((p, GUITAR, 4))
                        }
                        _ => None,
                    };
                    if let Some((p, ch, who)) = a {
                        if ch == GUITAR && bar == 2 {
                            self.ev.push(Ev::Think(4, "following", "answering the harp: a quick lick, then back to the chug".into()));
                        }
                        self.phrase = p;
                        self.ph_bar = bar;
                        self.ph_ch = ch;
                        self.ph_who = who;
                    }
                }
            }
        }
        if self.phrase.is_empty() {
            return;
        }
        // play this beat's notes of the phrase
        let rel = ((bar + 12 - self.ph_bar) % 12) as u16;
        let u0 = rel * 12 + beat as u16 * 3;
        let ch = self.ph_ch;
        let who = self.ph_who;
        let vel0 = match self.sec {
            Sec::Solo(_, k, _) => 78 + k as i32 * 6,
            _ => 70,
        };
        let notes: Vec<N> = self.phrase.iter().copied().filter(|n| n.u >= u0 && n.u < u0 + 3).collect();
        for n in notes {
            let d = (n.u - u0) as f32 * 4.0 / 3.0;
            let len = n.len as f32 * 4.0 / 3.0 * 0.92;
            let accent = if n.u % 3 == 0 { 6 } else { 0 };
            self.note(ch, n.semi, vel0 + accent, len, d, who);
            self.bend(ch, n.bend, d, len);
        }
        if u0 + 3 >= self.phrase.iter().map(|n| n.u + n.len).max().unwrap_or(0) {
            self.phrase.clear();
        }
    }

    /// bends on the lead channel (±2 semitone range: 8192 = centre, 4096 = a semitone down)
    fn bend(&mut self, ch: u8, kind: u8, d: f32, len: f32) {
        if ch == PIANO {
            return; // a piano doesn't bend (and the left hand shares the channel)
        }
        match kind {
            1 => {
                for k in 0..=4 {
                    self.ev.push(Ev::Bend(ch, 4096 + 1024 * k, d + k as f32 * 0.2));
                }
            }
            2 => {
                // scoop up, then a wailing vibrato
                for k in 0..=3 {
                    self.ev.push(Ev::Bend(ch, 4096 + 1365 * k, d + k as f32 * 0.25));
                }
                let mut x = d + 1.0;
                let mut up = true;
                while x < d + len - 0.3 {
                    self.ev.push(Ev::Bend(ch, if up { 8192 + 900 } else { 8192 - 700 }, x));
                    up = !up;
                    x += 0.55;
                }
                self.ev.push(Ev::Bend(ch, 8192, d + len));
            }
            3 => {
                // a whole-tone bend up into the note
                for k in 0..=4 {
                    self.ev.push(Ev::Bend(ch, 2048 * k, d + k as f32 * 0.3));
                }
            }
            _ => self.ev.push(Ev::Bend(ch, 8192, d)),
        }
    }
}
