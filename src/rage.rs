//! The Industrial style's machine: a NIN / Pig-style driving wall of sound with its
//! own drums (distorted synth kick with a click, gated metallic snare, mechanical
//! 16th hats, toms, FM metal: anvils, sheet metal, hammer-on-pipe), a distorted
//! synth bass with sub reinforcement, waveshaped power-chord guitar chugs with
//! palm-mute envelopes, a gritty / whispered pad and noise layers (filtered noise
//! sweeps, static bursts, a ticking machine), through a small DSP chain: drive
//! with tone control, bitcrusher, ring modulator, an 8 kHz ceiling, a bus
//! compressor and a tape stop.
//!
//! It also arranges itself: menacing verses explode into wall-of-sound choruses
//! via builds, with half-time crushes, stop-start hits and single-kick breakdowns,
//! odd 7/8 bars for unease, riff ostinatos that mutate, and sudden key drops by a
//! semitone. How hard it goes follows system load and typing.
//!
//! Everything here is written to be cheap: voices that are silent cost nothing,
//! filter coefficients are updated at control rate, saturation is a rational
//! soft clipper (no `tanh`).

use crate::dsp::{adv, saw, sine, softclip, Rng, Svf};

#[path = "metal.rs"]
mod metal;

/// which band the machine is: Industrial (wall of sound) or Biker (hard rock, gallops, a song form)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Industrial,
    Biker,
    /// metal: double kick, blasts, breakdowns, growls, shred (src/metal.rs)
    Metal,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sec {
    Verse,
    Build,
    Chorus,
    Half,
    Stop,
    Break,
    /// biker: the pre-chorus
    Pre,
    /// biker: harmonized twin-guitar solo
    Solo,
    /// biker: a V-twin lope surge
    Lope,
    /// metal: a blast beat (the tempo surges)
    Blast,
    /// metal: the half-time breakdown (the pit opens)
    Mosh,
}

impl Sec {
    pub fn name(self) -> &'static str {
        match self {
            Sec::Verse => "menacing verse",
            Sec::Build => "build",
            Sec::Chorus => "wall-of-sound chorus",
            Sec::Half => "half-time crush",
            Sec::Stop => "stop-start hits",
            Sec::Break => "breakdown",
            Sec::Pre => "pre-chorus",
            Sec::Solo => "twin-guitar solo",
            Sec::Lope => "V-twin lope",
            Sec::Blast => "blast beat",
            Sec::Mosh => "breakdown",
        }
    }
    /// the header's feel line (the UI treats "RAGE" like a surge: moshing, stomping)
    pub fn feel(self) -> &'static str {
        match self {
            Sec::Verse => "verse — the machine ticks",
            Sec::Build => "build — escalating",
            Sec::Chorus => "RAGE — wall of sound",
            Sec::Half => "RAGE — half-time crush",
            Sec::Stop => "RAGE — stop-start",
            Sec::Break => "breakdown — one kick",
            Sec::Pre => "pre-chorus — building",
            Sec::Solo => "twin-guitar solo",
            Sec::Lope => "surge — V-twin lope",
            Sec::Blast => "RAGE — blast beat",
            Sec::Mosh => "RAGE — half-time breakdown",
        }
    }
    /// the Metal band's feel lines ("RAGE" opens the pit in the crowd)
    pub fn feel_metal(self) -> &'static str {
        match self {
            Sec::Verse => "thrash — double kick, chugs",
            Sec::Solo => "shred solo — twin leads",
            _ => self.feel(),
        }
    }
    /// the Biker band's feel lines (the UI treats "CHORUS" as a sing-along)
    pub fn feel_biker(self) -> &'static str {
        match self {
            Sec::Verse => "verse — the gallop",
            Sec::Pre => "pre-chorus — building",
            Sec::Chorus => "CHORUS — all together now",
            Sec::Solo => "twin-guitar solo",
            Sec::Lope => "surge — V-twin lope",
            _ => self.feel(),
        }
    }
    pub fn loud(self) -> bool {
        matches!(self, Sec::Chorus | Sec::Half | Sec::Stop | Sec::Solo | Sec::Lope | Sec::Blast | Sec::Mosh)
    }
}

/// what the engine does with a step's result
pub enum Ev {
    /// GM drum layer (key, velocity, delay in steps); also animates the drummer
    Gm(i32, i32, f32),
    /// a synth drum hit, for the drummer's animation only (GM key, velocity 0..1)
    Hit(i32, f32),
    /// a bass note, for the bassist's animation (semitone, velocity 0..1)
    Bass(i32, f32),
    /// the GM guitar wall: power chord (root semitone, velocity, length, delay in steps)
    Guitar(i32, i32, f32, f32),
    /// the conductor shouts
    Say(&'static str),
    /// a new section starts
    Section(Sec),
    /// sudden key drop by a semitone
    Drop,
    /// a band-log / thinking line
    Log(String),
    /// a GM note on one of the machine's own channels (0/1 rhythm guitars L/R, 2 choir,
    /// 3 voice oohs, 4 lead guitar, 5 harmony lead): (chan, semitone, velocity, steps, delay)
    Note(u8, i32, i32, f32, f32),
    /// pitch bend on a machine channel (0..16383, 8192 centre), after `delay` steps
    Bend(u8, i32, f32),
    /// the frontman yells (shown on stage)
    Shout(&'static str),
    /// biker: the chord plan for the next section, one chord per bar (roman numerals)
    Plan(&'static [&'static str], String),
    /// a melodic note for a player's animation (player, semitone, velocity 0..1)
    Heard(usize, i32, f32),
    /// metal: tempo surge by +bpm (Some), or back to the song's own tempo (None)
    Tempo(Option<f32>),
}

/// what the engine tells the machine every step
pub struct In {
    pub s: usize,
    pub bar: u64,
    /// chord root (semitones from the global root) and whether it is minor-ish
    pub root: i32,
    /// 0..1: system load, agitation, energy and typing, combined
    pub intensity: f32,
    /// 0..1 keyboard / mouse activity
    pub input: f32,
    /// the key (tonic pc relative to the global root) and mode
    pub tonic: i32,
    pub minor: bool,
    /// biker: the engine's V-twin surge is on
    pub surge: bool,
}

/// a formant voice: saws + breath through three vowel formants, with grit (shouts,
/// growls, "yeah!", the gang "whoa-oh")
#[derive(Default, Clone, Copy)]
struct Vox {
    on: bool,
    t: f32,
    dur: f32,
    f: f32,
    f0: f32,
    f1: f32,
    f2: f32,
    ph: [f32; 3],
    gang: bool,
    vow: [f32; 3],
    from: [f32; 3],
    to: [f32; 3],
    morph: f32,
    amp: f32,
    grit: f32,
    noise: f32,
    growl: f32,
}

pub(crate) const V_A: [f32; 3] = [780.0, 1180.0, 2450.0];
pub(crate) const V_O: [f32; 3] = [460.0, 820.0, 2500.0];
pub(crate) const V_E: [f32; 3] = [520.0, 1800.0, 2480.0];
pub(crate) const V_U: [f32; 3] = [330.0, 700.0, 2400.0];
const V_I: [f32; 3] = [300.0, 2200.0, 2900.0];

/// biker chord plans (minor / Mixolydian), one chord per bar
const B_VERSE_M: [&[&str]; 3] = [&["i", "i", "VII", "VII", "i", "i", "VI", "VII"], &["i", "VII", "i", "VII", "VI", "VI", "VII", "VII"], &["i", "i", "iv", "iv", "i", "i", "VII", "VII"]];
const B_VERSE_J: [&[&str]; 2] = [&["I", "I", "bVII", "bVII", "I", "I", "IV", "bVII"], &["I", "bVII", "I", "bVII", "IV", "IV", "bVII", "bVII"]];
const B_PRE_M: &[&str] = &["VI", "VI", "VII", "VII"];
const B_PRE_J: &[&str] = &["IV", "IV", "V", "V"];
const B_HOOK_M: [&[&str]; 3] = [&["i", "i", "VI", "VI", "VII", "VII", "i", "i"], &["i", "VI", "VII", "i", "i", "VI", "VII", "VII"], &["VI", "VI", "VII", "VII", "i", "i", "VII", "VII"]];
const B_HOOK_J: [&[&str]; 2] = [&["I", "I", "bVII", "bVII", "IV", "IV", "I", "I"], &["I", "V", "bVII", "IV", "I", "V", "bVII", "bVII"]];
const B_SOLO_M: &[&str] = &["i", "VII", "VI", "VII", "i", "VII", "VI", "VII"];
const B_SOLO_J: &[&str] = &["I", "bVII", "IV", "I", "I", "bVII", "IV", "bVII"];

// ------------------------------------------------------------------ voices

#[derive(Default, Clone, Copy)]
struct Kick {
    on: bool,
    t: f32,
    ph: f32,
    amp: f32,
    body: f32,
    dec: f32,
    drive: f32,
}

#[derive(Default, Clone, Copy)]
struct Snare {
    on: bool,
    t: f32,
    amp: f32,
    ph: [f32; 4],
    gate: f32,
    metal: f32,
}

#[derive(Default, Clone, Copy)]
struct Clang {
    on: bool,
    t: f32,
    amp: f32,
    fc: f32,
    ratio: f32,
    index: f32,
    idec: f32,
    dec: f32,
    pc: f32,
    pm: f32,
    pan: f32,
    noise: f32,
}

#[derive(Default, Clone, Copy)]
struct Tom {
    on: bool,
    t: f32,
    amp: f32,
    f0: f32,
    ph: f32,
}

#[derive(Default, Clone, Copy)]
struct Bass {
    ph: [f32; 2],
    f: f32,
    env: f32,
    peak: f32,
    att: bool,
    dec: f32,
    fenv: f32,
    bright: f32,
    drive: f32,
}

#[derive(Default, Clone, Copy)]
struct Chug {
    ph: [f32; 6],
    f: f32,
    env: f32,
    peak: f32,
    dec: f32,
    open: bool,
    fenv: f32,
}

/// a scheduled trigger inside the machine (32nd-note rolls, flams)
#[derive(Clone, Copy)]
enum Trig {
    Snare(f32),
    Kick(f32),
    Tom(f32, f32),
}

const RHYTHMS: [&str; 8] = [
    "X.x.x.xxX.x.x.xx",
    "X..xX..xX..xx.x.",
    "Xxx.Xxx.Xxx.Xx.x",
    "X.xxX.xxX.x.Xxxx",
    "Xx.xXx.x.xXx.xXx",
    "X...x.x.X...x.xx",
    "XxxxXxx.XxxxX.xx",
    "X.x..x.xX.x..xx.",
];
/// kick patterns for the wall (bit s = a kick on step s): straight four plus pushes
const KICKS: [u16; 6] = [0x1111, 0x1511, 0x1191, 0x5111, 0x1131, 0x9119];
/// stop-start hit patterns (bit s)
const STOPS: [u16; 5] = [0x0449, 0x0495, 0x1249, 0x0889, 0x2491];

pub struct Riff {
    pub hit: [bool; 16],
    pub acc: [bool; 16],
    pub off: [i32; 16],
    /// 0 Phrygian, 1 Locrian / tritone, 2 chromatic minor
    pub mode: u8,
}

impl Riff {
    pub fn mode_name(&self) -> &'static str {
        ["Phrygian", "Locrian", "chromatic minor", "Phrygian", "harmonic minor", "diminished"][self.mode as usize % 6]
    }
}

pub struct Rage {
    pub kind: Kind,
    /// biker song memory: the chorus hook (progression + gang-vocal melody) returns each time
    hook_prog: usize,
    hook: Vec<(u8, i32, u8)>,
    hook_set: bool,
    verse_prog: usize,
    lick: Vec<(u8, i32, u8)>,
    songs: u32,
    vox: Vox,
    f_vox: [Svf; 3],
    sr: f32,
    rng: Rng,
    pub sec: Sec,
    pub sec_bar: u32,
    pub sec_len: u32,
    /// this bar is cut to 7/8 (the engine skips its last two 16ths)
    pub short: bool,
    pub riff: Riff,
    kpat: usize,
    stop_pat: u16,
    choruses: u32,
    pend_sec: Option<(Sec, u32)>,
    pub int: f32,
    /// output level (fades in/out with the style)
    level: f32,
    target: f32,
    pub ev: Vec<Ev>,
    pend: Vec<(u32, Trig)>,
    // voices
    kick: Kick,
    snare: Snare,
    hat_env: f32,
    hat_amp: f32,
    hat_dec: f32,
    clang: [Clang; 3],
    ci: usize,
    tom: Tom,
    bass: Bass,
    chug: Chug,
    tick_t: f32,
    tick_amp: f32,
    // pad: grit (saws, crushed) + whisper (resonant noise)
    pad_f: [f32; 3],
    pad_ph: [f32; 3],
    grit: f32,
    grit_t: f32,
    whisper: f32,
    whisper_t: f32,
    // noise layers
    sweep: f32,
    sweep_t: f32,
    sweep_f: f32,
    sweep_to: f32,
    stat_t: f32,
    stat_gate: f32,
    stat_hold: u32,
    // bus settings (gliding)
    drive: f32,
    drive_t: f32,
    tone: f32,
    ring: f32,
    ring_t: f32,
    ring_f: f32,
    ring_ph: f32,
    crush_hold: u32,
    crush_n: u32,
    crush_v: [f32; 2],
    // filters
    f_click: Svf,
    f_snare: Svf,
    f_hat: Svf,
    f_bass: Svf,
    f_cab: [Svf; 2],
    f_grit: Svf,
    f_wh: [Svf; 2],
    f_sweep: Svf,
    f_top: [Svf; 2],
    f_top2: [Svf; 2],
    hp_cab: [f32; 2],
    tone_lp: [f32; 2],
    comp_env: f32,
    cr: u32,
    // tape stop
    tape: Vec<[f32; 2]>,
    tw: usize,
    tr: f64,
    tape_v: f32,
    tape_on: bool,
    tape_dur: f32,
    tape_t: f32,
    pub l: Vec<f32>,
    pub r: Vec<f32>,
    /// strongest drum hit in the last block (UI glitch flicker)
    pub hit_now: f32,
}

#[inline]
fn sq(t: &[f32], ph: f32) -> f32 {
    // a soft square from a sine (cheap, no aliasing worth speaking of)
    softclip(sine(t, ph) * 2.6)
}

impl Rage {
    pub fn new(sr: f32, seed: u64) -> Rage {
        let mut r = Rage {
            kind: Kind::Industrial,
            hook_prog: 0,
            hook: Vec::new(),
            hook_set: false,
            verse_prog: 0,
            lick: Vec::new(),
            songs: 0,
            vox: Vox::default(),
            f_vox: [Svf::default(); 3],
            sr,
            rng: Rng::seeded(seed ^ 0x5241_4745),
            sec: Sec::Verse,
            sec_bar: 0,
            sec_len: 4,
            short: false,
            riff: Riff { hit: [false; 16], acc: [false; 16], off: [0; 16], mode: 0 },
            kpat: 0,
            stop_pat: STOPS[0],
            choruses: 0,
            pend_sec: None,
            int: 0.3,
            level: 0.0,
            target: 0.0,
            ev: Vec::with_capacity(32),
            pend: Vec::with_capacity(16),
            kick: Kick::default(),
            snare: Snare::default(),
            hat_env: 0.0,
            hat_amp: 0.0,
            hat_dec: 0.999,
            clang: [Clang::default(); 3],
            ci: 0,
            tom: Tom::default(),
            bass: Bass::default(),
            chug: Chug::default(),
            tick_t: 1.0,
            tick_amp: 0.0,
            pad_f: [110.0, 130.0, 165.0],
            pad_ph: [0.0, 0.33, 0.66],
            grit: 0.0,
            grit_t: 0.0,
            whisper: 0.0,
            whisper_t: 0.0,
            sweep: 0.0,
            sweep_t: 0.0,
            sweep_f: 300.0,
            sweep_to: 300.0,
            stat_t: 0.0,
            stat_gate: 0.0,
            stat_hold: 0,
            drive: 2.0,
            drive_t: 2.0,
            tone: 0.5,
            ring: 0.0,
            ring_t: 0.0,
            ring_f: 90.0,
            ring_ph: 0.0,
            crush_hold: 2,
            crush_n: 0,
            crush_v: [0.0; 2],
            f_click: Svf::default(),
            f_snare: Svf::default(),
            f_hat: Svf::default(),
            f_bass: Svf::default(),
            f_cab: [Svf::default(); 2],
            f_grit: Svf::default(),
            f_wh: [Svf::default(); 2],
            f_sweep: Svf::default(),
            f_top: [Svf::default(); 2],
            f_top2: [Svf::default(); 2],
            hp_cab: [0.0; 2],
            tone_lp: [0.0; 2],
            comp_env: 0.0,
            cr: 0,
            tape: vec![[0.0; 2]; (sr * 1.2) as usize],
            tw: 0,
            tr: 0.0,
            tape_v: 1.0,
            tape_on: false,
            tape_dur: 0.5,
            tape_t: 0.0,
            l: vec![0.0; 256],
            r: vec![0.0; 256],
            hit_now: 0.0,
        };
        r.f_click.set(3200.0, 0.9, sr);
        r.f_snare.set(2100.0, 0.7, sr);
        r.f_hat.set(6400.0, 1.1, sr);
        r.f_grit.set(1300.0, 0.8, sr);
        // an 8 kHz ceiling: two cascaded 12 dB low-passes at 7 kHz (gritty, never piercing)
        for k in 0..2 {
            r.f_top[k].set(7000.0, 0.6, sr);
            r.f_top2[k].set(7000.0, 0.6, sr);
        }
        r.new_riff();
        r
    }

    /// is anything sounding (or fading)?
    pub fn active(&self) -> bool {
        self.level > 1e-4 || self.target > 0.0
    }

    /// the style has just become Industrial
    pub fn start(&mut self, kind: Kind) {
        self.kind = kind;
        self.target = 1.0;
        self.choruses = 0;
        self.new_riff();
        if kind == Kind::Biker {
            self.hook_set = false;
            self.songs += 1;
            self.verse_prog = self.rng.below(3);
            self.go(Sec::Verse, 8);
        } else if kind == Kind::Metal {
            self.verse_prog = self.rng.below(4);
            self.metal_start();
        } else {
            self.go(Sec::Verse, 4);
        }
    }
    /// the section's name for this band
    pub fn sec_name(&self) -> &'static str {
        if self.kind == Kind::Biker {
            match self.sec {
                Sec::Verse => "verse",
                Sec::Chorus => "CHORUS",
                s => s.name(),
            }
        } else if self.kind == Kind::Metal {
            match self.sec {
                Sec::Verse => "thrash riff",
                Sec::Solo => "shred solo",
                s => s.name(),
            }
        } else {
            self.sec.name()
        }
    }
    /// biker: the first verse's chords
    pub fn plan_first(&mut self, minor: bool) {
        if self.kind == Kind::Metal {
            self.metal_plan(Sec::Verse);
        } else {
            self.biker_plan(Sec::Verse, minor);
        }
    }
    pub fn feel(&self) -> &'static str {
        match self.kind {
            Kind::Biker => self.sec.feel_biker(),
            Kind::Metal => self.sec.feel_metal(),
            Kind::Industrial => self.sec.feel(),
        }
    }

    /// the style is leaving Industrial: fade the machine out
    pub fn stop(&mut self) {
        self.target = 0.0;
        self.short = false;
        self.tape_on = false;
    }

    /// `n` (next idea): a new riff, straight into a build
    pub fn next_idea(&mut self) {
        self.new_riff();
        self.kpat = self.rng.below(KICKS.len());
        if self.kind == Kind::Biker {
            // a new song: new verse riff, a new chorus hook to remember
            self.hook_set = false;
            self.songs += 1;
            self.verse_prog = self.rng.below(3);
            self.go(Sec::Pre, 4);
        } else if self.kind == Kind::Metal {
            self.metal_riff();
            self.verse_prog = self.rng.below(4);
            self.go(Sec::Blast, 4);
            self.metal_plan(Sec::Blast);
            self.ev.push(Ev::Tempo(Some(16.0)));
        } else {
            self.go(Sec::Build, 2);
        }
    }

    fn go(&mut self, s: Sec, len: u32) {
        self.sec = s;
        self.sec_bar = 0;
        self.sec_len = len;
        self.ev.push(Ev::Section(s));
        match s {
            Sec::Verse => {
                self.grit_t = 0.0;
                self.whisper_t = 1.0;
                self.drive_t = 1.4;
                self.ring_t = 0.0;
                self.sweep_t = 0.0;
                self.tone = 0.35;
            }
            Sec::Build => {
                self.grit_t = 0.35;
                self.whisper_t = 0.4;
                self.drive_t = 2.2;
                self.ring_t = 0.1;
                self.sweep_f = 260.0;
                self.sweep_to = 6500.0;
                self.sweep_t = 0.8;
                self.tone = 0.5;
            }
            Sec::Chorus | Sec::Half | Sec::Stop => {
                self.grit_t = 0.8;
                self.whisper_t = 0.0;
                self.drive_t = 3.2;
                self.ring_t = 0.32;
                self.ring_f = [55.0, 82.0, 110.0, 147.0][self.rng.below(4)];
                self.sweep_t = 0.0;
                self.tone = 0.62;
                self.crush_hold = 2 + self.rng.below(3) as u32;
            }
            Sec::Break => {
                self.grit_t = 0.0;
                self.whisper_t = 0.6;
                self.drive_t = 1.8;
                self.ring_t = 0.0;
                self.sweep_t = 0.0;
                self.tone = 0.4;
            }
            Sec::Pre | Sec::Solo | Sec::Lope | Sec::Blast | Sec::Mosh => {}
        }
        if self.kind == Kind::Metal {
            // metal: high-gain, dark tone (deep low end, nothing fizzy), no ring mod, no whisper
            self.ring_t = 0.0;
            self.grit_t = 0.0;
            self.whisper_t = 0.0;
            self.sweep_t = 0.0;
            self.drive_t = match s {
                Sec::Blast | Sec::Mosh => 3.4,
                Sec::Solo => 3.0,
                _ => 3.1,
            };
            self.tone = 0.46;
        }
        if self.kind == Kind::Biker {
            // hard rock: warm drive, no ring mod, no crushed grit, no whisper
            self.ring_t = 0.0;
            self.grit_t = 0.0;
            self.whisper_t = 0.0;
            self.sweep_t = 0.0;
            self.drive_t = match s {
                Sec::Chorus | Sec::Solo => 2.6,
                Sec::Pre | Sec::Lope => 2.4,
                _ => 2.1,
            };
            self.tone = 0.55;
        }
    }

    pub fn new_riff(&mut self) {
        let rh: Vec<char> = RHYTHMS[self.rng.below(RHYTHMS.len())].chars().collect();
        self.riff.mode = self.rng.below(3) as u8;
        let pal: &[i32] = match self.riff.mode {
            0 => &[0, 0, 0, 0, 1, 1, 3, 7, 12],
            1 => &[0, 0, 0, 6, 6, 1, 3, 12],
            _ => &[0, 0, 1, 2, 3, 0, -1, 12],
        };
        for s in 0..16 {
            self.riff.hit[s] = rh[s] != '.';
            self.riff.acc[s] = rh[s] == 'X';
            self.riff.off[s] = if rh[s] == 'X' {
                if self.rng.chance(0.15) { 12 } else { 0 }
            } else if s >= 12 || self.rng.chance(0.3) {
                pal[self.rng.below(pal.len())]
            } else {
                0
            };
        }
        self.kpat = self.rng.below(KICKS.len());
    }

    /// ostinato mutation: move, re-pitch, add or drop one or two hits (never the downbeat)
    fn mutate(&mut self) -> &'static str {
        let pal: &[i32] = match self.riff.mode {
            0 => &[0, 1, 3, 7, 12],
            1 => &[0, 6, 1, 3, 12],
            _ => &[0, 1, 2, 3, -1],
        };
        let mut what = "re-pitched a stab";
        for _ in 0..1 + self.rng.below(2) {
            let s = 1 + self.rng.below(15);
            match self.rng.below(3) {
                0 => {
                    self.riff.hit[s] = !self.riff.hit[s];
                    what = if self.riff.hit[s] { "added a hit" } else { "dropped a hit" };
                }
                1 => {
                    self.riff.hit[s] = true;
                    self.riff.off[s] = pal[self.rng.below(pal.len())];
                    what = "re-pitched a stab";
                }
                _ => {
                    let t = 1 + self.rng.below(15);
                    self.riff.hit.swap(s, t);
                    self.riff.off.swap(s, t);
                    what = "shifted the accents";
                }
            }
        }
        what
    }

    fn next_section(&mut self, c: &In) {
        let int = self.int;
        let pick = |rng: &mut Rng, w: &[(Sec, f32)]| -> Sec {
            let ws: Vec<f32> = w.iter().map(|x| x.1.max(0.0)).collect();
            w[rng.weighted(&ws)].0
        };
        let (s, len) = match self.sec {
            Sec::Verse => {
                let s = pick(&mut self.rng, &[(Sec::Build, 0.5 + int), (Sec::Verse, 0.6 - int), (Sec::Break, 0.15)]);
                (s, if s == Sec::Build { if c.input > 0.5 { 2 } else { 4 } } else if s == Sec::Verse { 4 } else { 2 })
            }
            Sec::Build => (Sec::Chorus, 8),
            Sec::Chorus => {
                if int > 0.6 {
                    let s = pick(&mut self.rng, &[(Sec::Chorus, 1.0), (Sec::Half, 0.7), (Sec::Stop, 0.6)]);
                    (s, match s { Sec::Chorus => 8, Sec::Half => 4, _ => 2 })
                } else if int > 0.3 {
                    let s = pick(&mut self.rng, &[(Sec::Half, 0.7), (Sec::Break, 0.8), (Sec::Verse, 0.8), (Sec::Stop, 0.3)]);
                    (s, match s { Sec::Half => 4, Sec::Stop => 2, Sec::Break => 4, _ => 8 })
                } else {
                    let s = pick(&mut self.rng, &[(Sec::Break, 1.0), (Sec::Verse, 1.0)]);
                    (s, if s == Sec::Break { 4 } else { 8 })
                }
            }
            Sec::Half => {
                let s = pick(&mut self.rng, &[(Sec::Chorus, 0.5 + int), (Sec::Stop, 0.4), (Sec::Break, 0.6 - int)]);
                (s, match s { Sec::Chorus => 8, Sec::Stop => 2, _ => 4 })
            }
            Sec::Stop => (Sec::Chorus, 8),
            Sec::Break => {
                let s = pick(&mut self.rng, &[(Sec::Build, 0.5 + int), (Sec::Verse, 0.7 - int)]);
                (s, if s == Sec::Build { 4 } else { 8 })
            }
            _ => (Sec::Verse, 4),
        };
        let repeat = s == Sec::Chorus && self.sec == Sec::Chorus;
        self.go(s, len);
        match s {
            Sec::Chorus => {
                self.choruses += 1;
                self.ev.push(Ev::Say(if repeat { "again!" } else { ["NOW!", "LOUDER.", "tear it down!", "HARDER."][self.rng.below(4)] }));
                if repeat && self.rng.chance(0.4) {
                    self.ev.push(Ev::Drop);
                }
                if self.rng.chance(0.35) {
                    let w = self.mutate();
                    self.ev.push(Ev::Log(format!("riff mutation: {w}")));
                }
                if self.rng.chance(0.3) {
                    self.kpat = self.rng.below(KICKS.len());
                }
            }
            Sec::Verse => {
                self.ev.push(Ev::Say(["quiet.", "wait for it.", "listen to the machine.", "low. lower."][self.rng.below(4)]));
                if self.rng.chance(0.5) {
                    self.new_riff();
                    self.ev.push(Ev::Log(format!("new riff, {}", self.riff.mode_name())));
                }
            }
            Sec::Build => self.ev.push(Ev::Say(["LOUDER.", "more. MORE.", "build it."][self.rng.below(3)])),
            Sec::Half => self.ev.push(Ev::Say(["crush it.", "slow. heavy."][self.rng.below(2)])),
            Sec::Stop => {
                self.stop_pat = STOPS[self.rng.below(STOPS.len())];
                self.ev.push(Ev::Say("break it."));
            }
            Sec::Break => self.ev.push(Ev::Say(["just the kick.", "strip it. kick only."][self.rng.below(2)])),
            _ => {}
        }
    }

    // ------------------------------------------------------------ triggers

    fn kick(&mut self, amp: f32, body: f32, dec: f32, drive: f32) {
        self.kick = Kick { on: true, t: 0.0, ph: 0.0, amp, body, dec, drive };
        self.ev.push(Ev::Hit(36, amp));
    }
    fn snare(&mut self, amp: f32, gate: f32, metal: f32) {
        self.snare = Snare { on: true, t: 0.0, amp, ph: [0.0, 0.25, 0.0, 0.5], gate, metal };
    }
    fn hat(&mut self, amp: f32, open: bool) {
        self.hat_env = 1.0;
        self.hat_amp = amp;
        let d = if open { 0.14 } else { 0.022 };
        self.hat_dec = (-1.0 / (self.sr * d)).exp();
    }
    /// metal: 0 anvil, 1 sheet metal, 2 hammer on a pipe
    fn clang(&mut self, kind: usize, amp: f32) {
        let (fc, ratio, index, idec, dec, noise) = match kind {
            0 => (self.rng.range(900.0, 1300.0), 2.76, 2.2, 0.06, 0.45, 0.2),
            1 => (self.rng.range(190.0, 260.0), 1.414, 7.0, 0.35, 0.8, 0.6),
            _ => (self.rng.range(520.0, 700.0), 3.53, 1.3, 0.03, 0.32, 0.9),
        };
        let pan = self.rng.range(-0.6, 0.6);
        self.clang[self.ci] = Clang { on: true, t: 0.0, amp, fc, ratio, index, idec, dec, pc: 0.0, pm: 0.0, pan, noise };
        self.ci = (self.ci + 1) % 3;
    }
    fn tom(&mut self, amp: f32, f0: f32) {
        self.tom = Tom { on: true, t: 0.0, amp, f0, ph: 0.0 };
    }
    fn bass(&mut self, f: f32, amp: f32, dec_s: f32, bright: f32) {
        let b = &mut self.bass;
        b.f = f;
        b.peak = amp;
        b.att = true;
        b.dec = (-1.0 / (self.sr * dec_s)).exp();
        b.fenv = 1.0;
        b.bright = bright;
    }
    fn chug(&mut self, f: f32, amp: f32, open: bool) {
        let c = &mut self.chug;
        if (c.f - f).abs() > 0.5 {
            c.f = f;
        }
        c.peak = amp;
        c.env = c.env.max(amp * 0.3);
        c.open = open;
        c.fenv = 1.0;
        c.dec = (-1.0 / (self.sr * if open { 0.9 } else { 0.075 })).exp();
    }
    fn tick(&mut self, amp: f32) {
        self.tick_t = 0.0;
        self.tick_amp = amp;
    }
    fn statics(&mut self, secs: f32) {
        self.stat_t = self.stat_t.max(secs);
    }
    fn choke(&mut self) {
        self.chug.dec = self.chug.dec.min((-1.0 / (self.sr * 0.02)).exp());
        self.bass.dec = self.bass.dec.min((-1.0 / (self.sr * 0.03)).exp());
    }
    /// tape stop over `secs`; released at the next downbeat
    fn tape_stop(&mut self, secs: f32) {
        self.tape_on = true;
        self.tape_dur = secs;
        self.tape_t = 0.0;
        self.tape_v = 1.0;
        let n = self.tape.len();
        self.tr = ((self.tw + n - 1) % n) as f64;
    }
    fn later(&mut self, steps: f32, step_len: f32, t: Trig) {
        self.pend.push(((steps * step_len) as u32, t));
    }

    // ------------------------------------------------------------ the sequencer

    /// one 16th; `step_len` in samples; `hz(semi)` maps a semitone to Hz
    pub fn step(&mut self, c: &In, step_len: f32, hz: impl Fn(f32) -> f32) {
        if self.kind == Kind::Biker {
            self.step_biker(c, step_len, hz);
            return;
        }
        if self.kind == Kind::Metal {
            self.step_metal(c, step_len, hz);
            return;
        }
        let s = c.s;
        // intensity: quick up (typing makes it rage), slow down
        let k = if c.intensity > self.int { 0.25 } else { 0.02 };
        self.int += (c.intensity - self.int) * k;
        if s == 0 {
            if self.tape_on {
                self.tape_on = false;
            }
            self.sec_bar += 1;
            let typing = c.input > 0.55;
            if self.sec_bar >= self.sec_len {
                self.next_section(c);
            } else if typing && matches!(self.sec, Sec::Verse | Sec::Break) {
                // typing hard: it rages sooner
                self.go(Sec::Build, 2);
                self.ev.push(Ev::Say("LOUDER."));
                self.ev.push(Ev::Log("typing hard → the machine escalates".into()));
            } else if c.input > 0.75 && self.sec == Sec::Build && self.sec_bar + 1 < self.sec_len {
                self.sec_len = self.sec_bar + 1;
            }
            // 7/8 bars for unease (verses and choruses, the fourth bar of four)
            let p = match self.sec {
                Sec::Verse => 0.4,
                Sec::Chorus => 0.18,
                _ => 0.0,
            };
            self.short = self.sec_bar % 4 == 3 && self.rng.chance(p);
            if self.short && self.rng.chance(0.3) {
                self.ev.push(Ev::Log(format!("a bar of 7/8 in the {}", self.sec.name())));
            }
            if self.sec == Sec::Chorus && self.sec_bar % 2 == 0 && self.sec_bar > 0 && self.rng.chance(0.3) {
                let w = self.mutate();
                self.ev.push(Ev::Log(format!("riff mutation: {w}")));
            }
        }
        let root = c.root;
        let fold = |x: i32, lo: i32| lo + (x - lo).rem_euclid(12);
        let bsemi = |off: i32| fold(root + off, -15) as f32;
        let gsemi = |off: i32| fold(root + off, -5);
        let int = self.int;
        let last_bar = self.sec_bar + 1 == self.sec_len;
        let hit = self.riff.hit[s];
        let acc = self.riff.acc[s];
        let off = self.riff.off[s];
        match self.sec {
            Sec::Verse => {
                // the ticking machine, a sparse bass pulse, a whispered pad
                if s % 2 == 0 {
                    self.tick(if s % 4 == 0 { 0.9 } else { 0.5 });
                    if s % 4 == 0 {
                        self.ev.push(Ev::Hit(42, 0.3));
                    }
                }
                if s == 0 {
                    self.kick(0.55, 52.0, 0.32, 1.6);
                }
                if s == 10 && self.sec_bar % 2 == 1 {
                    self.kick(0.4, 52.0, 0.25, 1.4);
                }
                if s == 0 || s == 10 {
                    let f = hz(bsemi(if s == 10 { off.min(1) } else { 0 }));
                    self.bass(f, 0.6, 0.38, 0.15);
                    self.ev.push(Ev::Bass(bsemi(0) as i32, 0.5));
                }
                if s == 7 && self.rng.chance(0.3) {
                    self.clang(2, 0.25);
                }
                if s == 12 && self.rng.chance(0.5) {
                    self.ev.push(Ev::Gm(37, 48, 0.0));
                }
                if s == 0 && self.sec_bar % 2 == 0 {
                    self.chug(hz(gsemi(0) as f32), 0.35, false);
                }
                if s == 14 && self.rng.chance(0.08) {
                    self.statics(0.12);
                }
            }
            Sec::Build => {
                let k = self.sec_bar;
                let prog = (k * 16 + s as u32) as f32 / (self.sec_len * 16) as f32;
                self.sweep_f = 260.0 * (6500.0f32 / 260.0).powf(prog);
                self.sweep_t = 0.4 + 0.6 * prog;
                // a breath before the drop
                if last_bar && s >= 14 {
                    self.choke();
                    self.sweep_t = 0.0;
                    return;
                }
                let kick = if last_bar { s % 2 == 0 } else { s % 4 == 0 };
                if kick {
                    self.kick(0.7 + 0.3 * prog, 54.0, 0.18, 2.4);
                }
                let every = if last_bar { 1 } else if prog > 0.5 { 2 } else { 4 };
                if s % every == 0 && (prog > 0.25 || s % 8 == 4) {
                    let a = 0.3 + 0.7 * prog;
                    self.snare(a, 0.06, 0.4);
                    self.ev.push(Ev::Gm(38, (50.0 + 70.0 * prog) as i32, 0.0));
                    if last_bar && s >= 8 {
                        // 32nds at the very end
                        self.later(0.5, step_len, Trig::Snare(a));
                    }
                }
                if s % 2 == 0 {
                    self.chug(hz(gsemi(0) as f32), 0.35 + 0.5 * prog, false);
                    self.bass(hz(bsemi(0)), 0.5 + 0.4 * prog, 0.12, 0.3 + 0.4 * prog);
                    self.ev.push(Ev::Bass(bsemi(0) as i32, 0.6));
                    if s % 4 == 0 {
                        self.ev.push(Ev::Guitar(gsemi(0), (60.0 + 50.0 * prog) as i32, 0.8, 0.0));
                    }
                }
                self.hat(0.25 + 0.4 * prog, false);
            }
            Sec::Chorus => {
                if self.sec_bar == 0 && s == 0 {
                    self.ev.push(Ev::Gm(49, 118, 0.0));
                    self.clang(1, 0.9);
                    self.statics(0.15);
                }
                // fills on the last bar: toms and metal, now and then a brutal one
                if last_bar && s >= 8 {
                    let brutal = self.choruses % 2 == 1 || int > 0.75;
                    let toms = [196.0, 165.0, 131.0, 110.0, 98.0, 82.0, 73.0, 65.0];
                    let gm = [50, 48, 47, 45, 43, 41, 43, 41];
                    let i = s - 8;
                    self.tom(1.0, toms[i]);
                    self.ev.push(Ev::Gm(gm[i], 100 + i as i32 * 3, 0.0));
                    if brutal {
                        self.kick(1.0, 55.0, 0.12, 3.4);
                        self.snare(0.9, 0.05, 0.6);
                        self.later(0.5, step_len, Trig::Tom(0.8, toms[i] * 0.9));
                        if s % 2 == 0 {
                            self.later(0.5, step_len, Trig::Kick(0.8));
                        }
                    } else if s % 2 == 1 {
                        self.clang(if s % 4 == 1 { 0 } else { 2 }, 0.6);
                    }
                    if s == 15 {
                        self.statics(0.2);
                    }
                    return;
                }
                let kp = KICKS[self.kpat] | if int > 0.7 && self.sec_bar % 4 == 3 { 0xC000 } else { 0 };
                if (kp >> s) & 1 == 1 {
                    self.kick(1.0, 55.0, 0.2, 3.2);
                }
                if s == 4 || s == 12 {
                    self.snare(1.0, 0.13, 1.0);
                    self.ev.push(Ev::Gm(if s == 4 { 38 } else { 40 }, 115, 0.0));
                    if self.rng.chance(0.25) {
                        self.clang(1, 0.55);
                    }
                    if int > 0.6 && self.rng.chance(0.3) {
                        self.ev.push(Ev::Gm(39, 90, 0.0));
                    }
                }
                let acc4 = [1.0, 0.4, 0.65, 0.4][s % 4];
                let open = s == 14 && self.sec_bar % 2 == 1;
                self.hat(0.85 * acc4, open);
                if s % 4 == 2 {
                    self.ev.push(Ev::Hit(42, 0.5));
                }
                if s % 8 == 6 && self.rng.chance(0.4) {
                    self.clang(0, 0.55);
                }
                if s % 2 == 1 && self.rng.chance(0.07 + 0.1 * int) {
                    self.clang(2, 0.45);
                }
                // bass locked to the kick: 8ths, 16ths when it rages
                let sixteenths = int > 0.6 || c.input > 0.4;
                if s % 2 == 0 || (sixteenths && hit) {
                    let o = if hit { off } else { 0 };
                    let kick_here = (kp >> s) & 1 == 1;
                    self.bass(hz(bsemi(o)), if kick_here { 1.0 } else { 0.8 }, if sixteenths { 0.07 } else { 0.11 }, 0.75);
                    self.ev.push(Ev::Bass(bsemi(o) as i32, 0.9));
                }
                // the guitar wall
                if hit {
                    let open = acc && (s == 0 && self.sec_bar % 2 == 0);
                    self.chug(hz(gsemi(off) as f32), if acc { 1.0 } else { 0.85 }, open);
                    if s % 2 == 0 || acc {
                        self.ev.push(Ev::Guitar(gsemi(off), if acc { 112 } else { 96 }, if open { 14.0 } else { 0.9 }, 0.0));
                    }
                }
            }
            Sec::Half => {
                if s == 0 {
                    self.kick(1.0, 50.0, 0.4, 3.6);
                    self.chug(hz(gsemi(0) as f32), 1.0, true);
                    self.ev.push(Ev::Guitar(gsemi(0), 118, 15.0, 0.0));
                    self.bass(hz(bsemi(0)), 1.0, 0.6, 0.6);
                    self.ev.push(Ev::Bass(bsemi(0) as i32, 1.0));
                    if self.sec_bar % 2 == 0 {
                        self.ev.push(Ev::Gm(57, 112, 0.0));
                    }
                }
                if s == 3 && self.rng.chance(0.5) || s == 10 {
                    self.kick(0.9, 52.0, 0.3, 3.2);
                    self.bass(hz(bsemi(if s == 10 { off } else { 0 })), 0.9, 0.25, 0.6);
                }
                if s == 8 {
                    self.snare(1.0, 0.22, 1.0);
                    self.clang(1, 0.8);
                    self.ev.push(Ev::Gm(38, 127, 0.0));
                }
                if s % 4 == 0 {
                    self.hat(0.6, s == 12);
                    self.ev.push(Ev::Hit(46, 0.5));
                }
                if s == 14 && hit {
                    self.chug(hz(gsemi(off) as f32), 0.8, false);
                }
            }
            Sec::Stop => {
                let on = (self.stop_pat >> s) & 1 == 1;
                if last_bar && s == 12 {
                    self.tape_stop(0.45 * step_len * 16.0 / self.sr / 2.0);
                    self.ev.push(Ev::Log("tape stop".into()));
                    return;
                }
                if last_bar && s > 12 {
                    return;
                }
                if on {
                    self.kick(1.0, 55.0, 0.16, 3.4);
                    self.snare(0.9, 0.08, 1.0);
                    self.chug(hz(gsemi(off) as f32), 1.0, false);
                    self.bass(hz(bsemi(off)), 1.0, 0.1, 0.8);
                    self.ev.push(Ev::Guitar(gsemi(off), 118, 1.2, 0.0));
                    self.ev.push(Ev::Gm(if s == 0 { 49 } else { 38 }, 120, 0.0));
                    self.ev.push(Ev::Bass(bsemi(off) as i32, 1.0));
                    self.statics(0.04);
                } else {
                    self.choke();
                }
            }
            Sec::Pre | Sec::Solo | Sec::Lope | Sec::Blast | Sec::Mosh => {}
            Sec::Break => {
                if s % 4 == 0 {
                    self.kick(1.0, 50.0, 0.45, 3.6);
                }
                if s % 4 == 2 && self.rng.chance(0.1) {
                    self.statics(0.08);
                }
                if last_bar {
                    self.sweep_f = 300.0 * (5000.0f32 / 300.0).powf(s as f32 / 16.0);
                    self.sweep_t = 0.5;
                }
            }
        }
    }

    // ------------------------------------------------------------ biker: hard rock

    fn vox(&mut self, f: f32, dur: f32, from: [f32; 3], to: [f32; 3], gang: bool, grit: f32, noise: f32, slide: (f32, f32), growl: f32) {
        self.vox = Vox { on: true, t: 0.0, dur, f, f0: f * slide.0, f1: f * slide.1, f2: f, ph: self.vox.ph, gang, vow: from, from, to, morph: 0.0, amp: if gang { 0.8 } else { 1.0 }, grit, noise, growl };
    }
    /// a gritty frontman shout
    fn shout(&mut self, what: &'static str) {
        let base = self.rng.range(190.0, 240.0);
        match what {
            "YEAH!" | "yeah!" => self.vox(base, 0.55, V_I, V_A, false, 4.0, 0.25, (0.8, 1.3), 0.0),
            "HEY!" => self.vox(base * 1.1, 0.22, V_E, V_A, true, 3.0, 0.3, (1.0, 1.05), 0.0),
            "WHOO!" => self.vox(base * 1.3, 0.6, V_U, V_O, false, 3.0, 0.3, (0.9, 1.5), 0.0),
            "ALRIGHT!" => self.vox(base, 0.7, V_A, V_I, false, 4.0, 0.25, (1.0, 1.2), 0.0),
            _ => self.vox(base * 0.5, 0.8, V_O, V_A, false, 6.0, 0.45, (0.9, 1.0), 1.0), // the growl
        }
        self.ev.push(Ev::Shout(what));
    }

    fn biker_plan(&mut self, sec: Sec, minor: bool) {
        let (chords, why): (&'static [&'static str], String) = match sec {
            Sec::Verse => {
                let p = if minor { B_VERSE_M[self.verse_prog % 3] } else { B_VERSE_J[self.verse_prog % 2] };
                (p, "verse riff".into())
            }
            Sec::Pre => (if minor { B_PRE_M } else { B_PRE_J }, "pre-chorus lift".into()),
            Sec::Chorus => {
                let callback = self.hook_set;
                if !callback {
                    self.hook_prog = self.rng.below(3);
                }
                let p = if minor { B_HOOK_M[self.hook_prog % 3] } else { B_HOOK_J[self.hook_prog % 2] };
                (p, if callback { "callback: the chorus hook returns".into() } else { "the chorus hook".into() })
            }
            Sec::Solo => (if minor { B_SOLO_M } else { B_SOLO_J }, "solo changes".into()),
            _ => return,
        };
        self.ev.push(Ev::Plan(chords, why));
    }

    /// the chorus hook: a gang-vocal melody of chord tones over two bars (made once, then remembered)
    fn make_hook(&mut self) {
        let shapes: [&[(u8, i32, u8)]; 3] = [
            &[(0, 4, 6), (6, 2, 2), (8, 0, 6), (16, 4, 6), (22, 5, 2), (24, 4, 8)],
            &[(0, 0, 4), (4, 2, 4), (8, 4, 8), (16, 5, 4), (20, 4, 4), (24, 2, 8)],
            &[(0, 4, 3), (3, 4, 3), (6, 2, 2), (8, 0, 8), (16, 2, 3), (19, 2, 3), (22, 4, 2), (24, 0, 8)],
        ];
        self.hook = shapes[self.rng.below(3)].to_vec();
        self.hook_set = true;
    }
    /// a twin-guitar lick: gallop-phrased minor-pentatonic line over two bars
    fn make_lick(&mut self) {
        let pent = [0, 2, 3, 4, 6, 7, 9, 10];
        let mut v = Vec::new();
        let mut d = pent[self.rng.below(pent.len())] as i32;
        let mut t = 0u8;
        while t < 32 {
            let len = [2u8, 1, 1, 4, 2, 2][self.rng.below(6)];
            d = (d + [-1, 1, 1, 2, -2, 0][self.rng.below(6)]).clamp(0, 11);
            v.push((t, d, len));
            t += len;
        }
        self.lick = v;
    }

    fn step_biker(&mut self, c: &In, step_len: f32, hz: impl Fn(f32) -> f32) {
        let s = c.s;
        let k = if c.intensity > self.int { 0.25 } else { 0.02 };
        self.int += (c.intensity - self.int) * k;
        let minor = c.minor;
        // look ahead: the next section's chord plan lands on its first bar
        if s == 12 && self.sec_bar + 1 >= self.sec_len && !c.surge {
            let next = self.biker_next();
            self.pend_sec = Some(next);
            self.biker_plan(next.0, minor);
        }
        if s == 0 {
            self.sec_bar += 1;
            if c.surge && self.sec != Sec::Lope {
                self.go(Sec::Lope, 64);
                self.shout("WHOO!");
            } else if !c.surge && self.sec == Sec::Lope {
                self.go(Sec::Verse, 8);
                self.biker_plan(Sec::Verse, minor);
            } else if self.sec_bar >= self.sec_len && self.sec != Sec::Lope {
                let (n, len) = self.pend_sec.take().unwrap_or((Sec::Verse, 8));
                self.biker_enter(n, len);
            }
            self.short = false;
        }
        let root = c.root;
        let fold = |x: i32, lo: i32| lo + (x - lo).rem_euclid(12);
        let bsemi = |off: i32| fold(root + off, -17);
        let gsemi = |off: i32| fold(root + off, -5);
        let last_bar = self.sec_bar + 1 == self.sec_len;
        let beat_pos = s % 4;
        let gallop = beat_pos == 0 || beat_pos == 2 || beat_pos == 3;
        let int = self.int;
        match self.sec {
            Sec::Lope => {
                // the V-twin lope: long-short low kicks + floor toms, root thuds
                match s {
                    0 | 8 => {
                        self.kick(1.0, 42.0, 0.42, 1.8);
                        self.ev.push(Ev::Gm(41, 92, 0.0));
                        self.bass(hz(bsemi(0) as f32), 1.0, 0.3, 0.4);
                        self.ev.push(Ev::Bass(bsemi(0), 0.9));
                        self.chug(hz(gsemi(0) as f32), 0.8, false);
                        self.ev.push(Ev::Note(0, gsemi(0), 100, 2.0, 0.0));
                    }
                    3 | 11 => {
                        self.kick(0.75, 47.0, 0.3, 1.6);
                        self.tom(0.6, 82.0);
                        self.ev.push(Ev::Gm(43, 74, 0.0));
                        self.bass(hz(bsemi(0) as f32), 0.75, 0.2, 0.3);
                        self.chug(hz(gsemi(0) as f32), 0.6, false);
                    }
                    12 => self.ev.push(Ev::Gm(37, 40, 0.0)),
                    _ => {}
                }
            }
            Sec::Verse | Sec::Solo => {
                // the gallop: 8th + two 16ths on guitars and bass; kick 1 & 3, snare 2 & 4
                let riff_run = s >= 12 && self.sec_bar % 2 == 1;
                let off = if riff_run { [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, if minor { 10 } else { 10 }, 0, 7, 5][s] } else { 0 };
                if gallop {
                    let acc = beat_pos == 0;
                    self.chug(hz(gsemi(off) as f32), if acc { 0.9 } else { 0.62 }, false);
                    self.bass(hz(bsemi(off) as f32), if acc { 0.95 } else { 0.75 }, 0.09, 0.45);
                    self.ev.push(Ev::Bass(bsemi(off), 0.8));
                    if acc || riff_run {
                        self.ev.push(Ev::Note(0, gsemi(off), if acc { 100 } else { 84 }, 0.8, 0.0));
                        self.ev.push(Ev::Note(1, gsemi(off), if acc { 96 } else { 80 }, 0.8, 0.03));
                    }
                }
                if s == 0 || s == 8 || (s == 10 && int > 0.5) || (s == 6 && self.sec_bar % 2 == 1) {
                    self.kick(if s % 8 == 0 { 1.0 } else { 0.8 }, 56.0, 0.24, 1.8);
                }
                if s == 4 || s == 12 {
                    self.snare(1.0, 0.2, 0.12);
                    self.ev.push(Ev::Gm(38, 110, 0.0));
                }
                if s % 2 == 0 {
                    self.hat(if s % 4 == 0 { 0.7 } else { 0.45 }, false);
                    if s % 4 == 2 {
                        self.ev.push(Ev::Hit(42, 0.4));
                    }
                }
                if self.sec_bar == 0 && s == 0 {
                    self.ev.push(Ev::Gm(49, 110, 0.0));
                }
                if self.sec == Sec::Solo {
                    self.solo_step(c, s);
                } else if s == 14 && self.sec_bar == 3 && self.rng.chance(0.35) {
                    self.shout("growl");
                }
                if last_bar && s >= 12 {
                    let toms = [165.0, 131.0, 110.0, 82.0];
                    let gm = [48, 47, 45, 43];
                    self.tom(0.9, toms[s - 12]);
                    self.ev.push(Ev::Gm(gm[s - 12], 104, 0.0));
                }
            }
            Sec::Pre => {
                // the gallop on the drums too (double kick), the guitars climbing
                if gallop {
                    self.kick(if beat_pos == 0 { 1.0 } else { 0.72 }, 56.0, 0.16, 1.8);
                    let acc = beat_pos == 0;
                    self.chug(hz(gsemi(0) as f32), if acc { 0.95 } else { 0.7 }, false);
                    self.bass(hz(bsemi(0) as f32), if acc { 1.0 } else { 0.8 }, 0.09, 0.5);
                    self.ev.push(Ev::Bass(bsemi(0), 0.85));
                    if acc {
                        self.ev.push(Ev::Note(0, gsemi(0), 104, 0.9, 0.0));
                        self.ev.push(Ev::Note(1, gsemi(0) + 7, 98, 0.9, 0.03));
                    }
                }
                let roll = last_bar && s >= 8;
                if s == 4 || s == 12 || (roll && s % 2 == 0) || (last_bar && s >= 12) {
                    let a = if roll { 0.6 + 0.1 * (s - 8) as f32 } else { 1.0 };
                    self.snare(a.min(1.0), 0.18, 0.12);
                    self.ev.push(Ev::Gm(38, (90.0 + 30.0 * a) as i32, 0.0));
                }
                if s % 2 == 0 {
                    self.hat(0.55, false);
                }
                if self.sec_bar == 0 && s == 0 {
                    self.ev.push(Ev::Gm(57, 108, 0.0));
                }
                if last_bar && s == 8 {
                    self.ev.push(Ev::Say("ALL TOGETHER NOW!"));
                }
                if last_bar && s == 14 {
                    let w = ["YEAH!", "HEY!", "ALRIGHT!"][self.rng.below(3)];
                    self.shout(w);
                    self.choke();
                }
            }
            Sec::Chorus => {
                // big ringing open chords, double-kick gallop, crash + ride, the gang "whoa-oh"
                let bar_in_hook = (self.sec_bar % 2) as u8;
                if s == 0 {
                    self.chug(hz(gsemi(0) as f32), 1.0, true);
                    self.ev.push(Ev::Note(0, gsemi(0), 112, 15.0, 0.0));
                    self.ev.push(Ev::Note(0, gsemi(0) + 7, 104, 15.0, 0.0));
                    self.ev.push(Ev::Note(1, gsemi(0) + 7, 104, 15.0, 0.05));
                    self.ev.push(Ev::Note(1, gsemi(0) + 12, 100, 15.0, 0.05));
                    self.ev.push(Ev::Gm(if self.sec_bar % 2 == 0 { 49 } else { 57 }, 116, 0.0));
                    // the gang choir holds the chord
                    let third = if c.minor || self.rng.chance(0.0) { 3 } else { 4 };
                    let r = fold(root, 0);
                    for (i, iv) in [0, third, 7].iter().enumerate() {
                        self.ev.push(Ev::Note(2, r + iv, 80 - i as i32 * 6, 15.5, 0.0));
                    }
                }
                if s == 8 && self.sec_bar % 2 == 1 {
                    self.chug(hz(gsemi(0) as f32), 0.85, true);
                }
                if gallop {
                    self.kick(if beat_pos == 0 { 1.0 } else { 0.7 }, 56.0, 0.15, 1.8);
                    self.bass(hz(bsemi(0) as f32), if beat_pos == 0 { 1.0 } else { 0.8 }, 0.1, 0.55);
                    self.ev.push(Ev::Bass(bsemi(0), 0.9));
                }
                if s == 4 || s == 12 {
                    self.snare(1.0, 0.22, 0.12);
                    self.ev.push(Ev::Gm(38, 120, 0.0));
                }
                if s % 2 == 0 {
                    self.hat(0.5, s % 4 == 2);
                    if s % 4 == 2 {
                        self.ev.push(Ev::Gm(51, 70, 0.0));
                    }
                }
                // the hook melody (gang vocal + voice oohs doubling it)
                if !self.hook_set {
                    self.make_hook();
                }
                let t = bar_in_hook * 16 + s as u8;
                let scale: [i32; 7] = if minor { [0, 2, 3, 5, 7, 8, 10] } else { [0, 2, 4, 5, 7, 9, 10] };
                let hk = self.hook.clone();
                for &(at, deg, len) in hk.iter().filter(|h| h.0 == t) {
                    let d = deg.rem_euclid(7) as usize;
                    let semi = fold(c.tonic + scale[d], 0) + 3; // around C3-C4 relative to A2
                    let f = hz(semi as f32 + 0.0);
                    let first = at == 0 || at == 16;
                    let dur = len as f32 * step_len / self.sr * 0.95;
                    self.vox(f, dur, if first { V_U } else { V_O }, if first { V_O } else { V_O }, true, 2.2, 0.12, (0.97, 1.0), 0.0);
                    self.ev.push(Ev::Note(3, semi + 12, 92, len as f32, 0.0));
                    self.ev.push(Ev::Heard(6, semi, 0.8));
                }
                if last_bar && s == 12 {
                    // a pinch squeal into whatever comes next
                    let top = fold(c.tonic + 7, 24) ;
                    self.ev.push(Ev::Note(4, top, 120, 3.0, 0.0));
                    self.ev.push(Ev::Bend(4, 16383, 0.5));
                    self.ev.push(Ev::Bend(4, 8192, 3.5));
                }
                if last_bar && s >= 13 {
                    self.tom(1.0, [131.0, 98.0, 73.0][s - 13]);
                    self.ev.push(Ev::Gm([47, 45, 41][s - 13], 112, 0.0));
                }
            }
            _ => {}
        }
    }

    /// harmonized twin leads in diatonic thirds, with bends
    fn solo_step(&mut self, c: &In, s: usize) {
        if self.lick.is_empty() || (s == 0 && self.sec_bar % 4 == 0 && self.sec_bar > 0) {
            self.make_lick();
        }
        let t = ((self.sec_bar % 2) * 16) as u8 + s as u8;
        let scale: [i32; 7] = if self.kind == Kind::Metal { [0, 2, 3, 5, 7, 8, 11] } else if c.minor { [0, 2, 3, 5, 7, 8, 10] } else { [0, 2, 4, 5, 7, 9, 10] };
        let deg_semi = |d: i32| -> i32 { scale[d.rem_euclid(7) as usize] + 12 * d.div_euclid(7) };
        let fold = |x: i32, lo: i32| lo + (x - lo).rem_euclid(12);
        let base = fold(c.tonic, 12);
        let lick = self.lick.clone();
        for &(at, d, len) in lick.iter().filter(|l| l.0 == t) {
            let a = base + deg_semi(d);
            let b = base + deg_semi(d + 2);
            self.ev.push(Ev::Note(4, a, 108, len as f32, 0.0));
            self.ev.push(Ev::Note(5, b, 100, len as f32, 0.02));
            self.ev.push(Ev::Heard(4, a, 0.9));
            if len >= 4 {
                self.ev.push(Ev::Bend(4, 12288, 1.0));
                self.ev.push(Ev::Bend(5, 12288, 1.0));
                self.ev.push(Ev::Bend(4, 8192, len as f32));
                self.ev.push(Ev::Bend(5, 8192, len as f32));
            }
            let _ = at;
        }
        if self.sec_bar == 0 && s == 0 {
            let top = fold(c.tonic + 7, 24);
            self.ev.push(Ev::Note(4, top, 124, 2.0, 0.0));
            self.ev.push(Ev::Bend(4, 16383, 0.25));
            self.ev.push(Ev::Bend(4, 8192, 2.0));
        }
    }

    fn biker_next(&mut self) -> (Sec, u32) {
        match self.sec {
            Sec::Verse => (Sec::Pre, 4),
            Sec::Pre => (Sec::Chorus, 8),
            Sec::Chorus => {
                self.choruses += 1;
                if self.choruses % 3 == 2 && self.rng.chance(0.7) {
                    (Sec::Solo, 8)
                } else if self.int > 0.75 && self.rng.chance(0.3) {
                    (Sec::Chorus, 8)
                } else {
                    (Sec::Verse, 8)
                }
            }
            Sec::Solo => (Sec::Chorus, 8),
            _ => (Sec::Verse, 8),
        }
    }

    fn biker_enter(&mut self, n: Sec, len: u32) {
        let callback = n == Sec::Chorus && self.hook_set;
        self.go(n, len);
        match n {
            Sec::Verse => {
                if self.rng.chance(0.5) {
                    let w = ["YEAH!", "growl", "ALRIGHT!"][self.rng.below(3)];
                    self.shout(w);
                }
                self.ev.push(Ev::Say(["ride!", "lock it in, riders", "gallop!"][self.rng.below(3)]));
            }
            Sec::Pre => self.ev.push(Ev::Say(["here it comes…", "lift it, brothers!"][self.rng.below(2)])),
            Sec::Chorus => {
                self.ev.push(Ev::Say(["SING IT!", "ALL TOGETHER NOW!", "everybody!"][self.rng.below(3)]));
                if callback {
                    self.ev.push(Ev::Log("the chorus hook comes back — same melody, same changes".into()));
                }
            }
            Sec::Solo => {
                self.make_lick();
                self.shout("WHOO!");
                self.ev.push(Ev::Say("twin guitars — take it!"));
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ audio

    /// render `n` samples into `self.l` / `self.r`
    pub fn render(&mut self, n: usize, tbl: &[f32]) {
        let sr = self.sr;
        let dt = 1.0 / sr;
        self.hit_now = 0.0;
        if self.l.len() < n {
            self.l.resize(n, 0.0);
            self.r.resize(n, 0.0);
        }
        for j in 0..n {
            // gliding bus settings + level
            self.level += (self.target - self.level) * 0.0004;
            self.drive += (self.drive_t - self.drive) * 0.0005;
            self.ring += (self.ring_t - self.ring) * 0.0005;
            self.grit += (self.grit_t - self.grit) * 0.0003;
            self.whisper += (self.whisper_t - self.whisper) * 0.0003;
            self.sweep += (self.sweep_t - self.sweep) * 0.002;
            if self.cr == 0 {
                self.cr = 16;
                self.control();
            }
            self.cr -= 1;
            // scheduled triggers
            if !self.pend.is_empty() {
                let mut i = 0;
                while i < self.pend.len() {
                    if self.pend[i].0 == 0 {
                        let (_, t) = self.pend.swap_remove(i);
                        match t {
                            Trig::Snare(a) => self.snare(a, 0.05, 0.5),
                            Trig::Kick(a) => self.kick(a, 55.0, 0.12, 3.2),
                            Trig::Tom(a, f) => self.tom(a, f),
                        }
                    } else {
                        self.pend[i].0 -= 1;
                        i += 1;
                    }
                }
            }
            let nz = self.rng.bi();
            // ---- drums (mono, kick never filtered)
            let mut low = 0.0;
            let mut drums = 0.0;
            if self.kick.on {
                let k = &mut self.kick;
                let t = k.t;
                let f = k.body + 150.0 * crate::dsp::fexp(-t / 0.028) + 700.0 * crate::dsp::fexp(-t / 0.003);
                adv(&mut k.ph, f * dt);
                let env = crate::dsp::fexp(-t / k.dec);
                let body = softclip(sine(tbl, k.ph) * env * k.drive) / softclip(k.drive).max(0.3);
                let click = self.f_click.run(nz).1 * crate::dsp::fexp(-t / 0.0025) * 1.4;
                low += body * k.amp * 0.62;
                drums += click * k.amp * 0.25;
                self.hit_now = self.hit_now.max(k.amp * (t < 0.05) as i32 as f32);
                k.t += dt;
                if t > k.dec * 7.0 {
                    k.on = false;
                }
            }
            if self.snare.on {
                let sn = &mut self.snare;
                let t = sn.t;
                adv(&mut sn.ph[0], 190.0 * (1.0 + 0.5 * crate::dsp::fexp(-t / 0.01)) * dt);
                adv(&mut sn.ph[1], 330.0 * dt);
                adv(&mut sn.ph[2], 545.0 * dt);
                adv(&mut sn.ph[3], 812.0 * dt);
                let body = (sine(tbl, sn.ph[0]) * 0.7 + sine(tbl, sn.ph[1]) * 0.4) * crate::dsp::fexp(-t / 0.055);
                let gate = if t < sn.gate { crate::dsp::fexp(-t / 0.35) } else { crate::dsp::fexp(-sn.gate / 0.35) * crate::dsp::fexp(-(t - sn.gate) / 0.004) };
                let bp = self.f_snare.run(nz);
                let noise = (bp.1 * 1.2 + (nz - bp.0) * 0.35) * gate;
                let metal = sq(tbl, sn.ph[2]) * sq(tbl, sn.ph[3]) * crate::dsp::fexp(-t / 0.09) * sn.metal;
                drums += softclip((body + noise + metal * 0.4) * 1.8) * sn.amp * 0.42;
                sn.t += dt;
                if t > sn.gate + 0.05 && t > 0.12 {
                    sn.on = false;
                }
            }
            if self.hat_env > 1e-3 {
                let h = self.f_hat.run(nz).1;
                drums += h * self.hat_env * self.hat_amp * 0.16;
                self.hat_env *= self.hat_dec;
            }
            if self.tom.on {
                let tm = &mut self.tom;
                let t = tm.t;
                adv(&mut tm.ph, tm.f0 * (1.0 + 0.8 * crate::dsp::fexp(-t / 0.03)) * dt);
                let x = softclip(sine(tbl, tm.ph) * crate::dsp::fexp(-t / 0.22) * 2.2) + nz * crate::dsp::fexp(-t / 0.01) * 0.3;
                drums += x * tm.amp * 0.38;
                tm.t += dt;
                if t > 1.2 {
                    tm.on = false;
                }
            }
            let (mut ml, mut mr) = (0.0f32, 0.0f32);
            for cl in self.clang.iter_mut() {
                if !cl.on {
                    continue;
                }
                let t = cl.t;
                adv(&mut cl.pm, cl.fc * cl.ratio * dt);
                adv(&mut cl.pc, cl.fc * dt);
                let idx = cl.index * crate::dsp::fexp(-t / cl.idec);
                let ph = { let r = crate::dsp::fract1(cl.pc + idx * sine(tbl, cl.pm) * 0.16); if r < 0.0 { r + 1.0 } else { r } };
                let x = (sine(tbl, ph) + nz * cl.noise * crate::dsp::fexp(-t / 0.004)) * crate::dsp::fexp(-t / cl.dec) * cl.amp * 0.22;
                ml += x * (0.5 - cl.pan * 0.5);
                mr += x * (0.5 + cl.pan * 0.5);
                cl.t += dt;
                if t > cl.dec * 6.0 {
                    cl.on = false;
                }
            }
            if self.tick_amp > 1e-3 && self.tick_t < 0.03 {
                let t = self.tick_t;
                let x = (sine(tbl, crate::dsp::fract1(t * 3100.0)) * 0.7 + nz * 0.3) * crate::dsp::fexp(-t / 0.004) * self.tick_amp * 0.16;
                ml += x * 0.6;
                mr += x * 0.4;
                self.tick_t += dt;
            }
            // ---- bass: saturated saw + square, filter env, sub sine
            let b = &mut self.bass;
            let mut wall_l = 0.0;
            let mut wall_r = 0.0;
            if b.env > 1e-4 || b.att {
                if b.att {
                    b.env += (b.peak - b.env) * 0.2;
                    if b.env >= b.peak * 0.97 {
                        b.att = false;
                    }
                } else {
                    b.env *= b.dec;
                }
                let inc = b.f * dt;
                let mut p2 = b.ph[0] + 0.5;
                if p2 >= 1.0 {
                    p2 -= 1.0;
                }
                let raw = saw(b.ph[0], inc) * 0.6 + (saw(b.ph[0], inc) - saw(p2, inc)) * 0.3;
                adv(&mut b.ph[0], inc);
                adv(&mut b.ph[1], inc * if b.f > 65.0 { 0.5 } else { 1.0 });
                let lp = self.f_bass.run(raw).0;
                let x = softclip(lp * (1.5 + 2.5 * b.bright) * self.drive.min(3.0) * 0.6);
                let sub = sine(tbl, b.ph[1]);
                low += (sub * 0.7 * b.env) + x * 0.32 * b.env;
                b.fenv *= 0.9993;
            }
            // ---- guitar chugs: waveshaped power-chord saws, double-tracked L/R
            let cg = &mut self.chug;
            if cg.env > 1e-4 || cg.peak > 0.0 {
                if cg.peak > 0.0 {
                    cg.env += (cg.peak - cg.env) * 0.35;
                    if cg.env >= cg.peak * 0.95 {
                        cg.peak = 0.0;
                    }
                } else {
                    cg.env *= cg.dec;
                }
                let f = cg.f;
                let fr = [f * 1.003, f * 1.4983 * 0.997, f * 2.0 * 1.0015, f * 0.997, f * 1.4983 * 1.003, f * 2.0 * 0.9985];
                let mut sides = [0.0f32; 2];
                for (k, &fk) in fr.iter().enumerate() {
                    let inc = fk * dt;
                    sides[k / 3] += saw(cg.ph[k], inc) * if k % 3 == 2 { 0.5 } else { 1.0 };
                    adv(&mut cg.ph[k], inc);
                }
                let e = cg.env;
                for (k, sd) in sides.iter().enumerate() {
                    let x = softclip(sd * e * 3.2) ;
                    let y = self.f_cab[k].run(x).0;
                    self.hp_cab[k] += (y - self.hp_cab[k]) * 0.016;
                    let o = (y - self.hp_cab[k]) * 0.3;
                    if k == 0 { wall_l += o } else { wall_r += o }
                }
                cg.fenv *= 0.9985;
            }
            // ---- pad: crushed saws (grit) and resonant noise (whisper)
            if self.grit > 1e-3 {
                let mut g = 0.0;
                for k in 0..3 {
                    let inc = self.pad_f[k] * dt;
                    g += saw(self.pad_ph[k], inc);
                    adv(&mut self.pad_ph[k], inc);
                }
                let g = (g * 2.0).round() * 0.5; // a few bits
                let y = self.f_grit.run(g).1 * self.grit * 0.09;
                wall_l += y;
                wall_r += y * 0.8;
            }
            let mut noise_l = 0.0;
            let mut noise_r = 0.0;
            if self.whisper > 1e-3 {
                let a = self.f_wh[0].run(nz).1;
                let b2 = self.f_wh[1].run(nz).1;
                noise_l += (a * 0.7 + b2 * 0.3) * self.whisper * 0.06;
                noise_r += (a * 0.3 + b2 * 0.7) * self.whisper * 0.06;
            }
            if self.sweep > 1e-3 {
                let y = self.f_sweep.run(nz).1 * self.sweep * 0.07;
                noise_l += y;
                noise_r += y;
            }
            if self.stat_t > 0.0 {
                if self.stat_hold == 0 {
                    self.stat_hold = 20 + self.rng.below(160) as u32;
                    self.stat_gate = if self.rng.chance(0.55) { self.rng.range(0.3, 1.0) } else { 0.0 };
                }
                self.stat_hold -= 1;
                let y = (nz * 4.0).round() * 0.25 * self.stat_gate * 0.1;
                noise_l += y;
                noise_r -= y * 0.6;
                self.stat_t -= dt;
            }
            // ---- the voice (formants on saws + breath, with grit)
            if self.vox.on {
                let v = &mut self.vox;
                let t = v.t;
                let x = (t / v.dur).min(1.0);
                // pitch: slide in, then settle
                v.f = if x < 0.25 { v.f0 + (v.f1 - v.f0) * (x / 0.25) } else { v.f1 + (v.f2 - v.f1) * ((x - 0.25) / 0.75) };
                let vib = 1.0 + 0.012 * sine(tbl, crate::dsp::fract1(t * 5.5)) + v.growl * 0.04 * sine(tbl, crate::dsp::fract1(t * 31.0));
                let inc = v.f * vib * dt;
                let mut src = saw(v.ph[0], inc);
                adv(&mut v.ph[0], inc);
                if v.gang {
                    src += saw(v.ph[1], inc * 1.011) + saw(v.ph[2], inc * 0.991);
                    adv(&mut v.ph[1], inc * 1.011);
                    adv(&mut v.ph[2], inc * 0.991);
                    src *= 0.45;
                }
                src += nz * v.noise;
                v.morph = (x * 2.2).min(1.0);
                let env = (t / 0.03).min(1.0) * if x > 0.8 { (1.0 - x) / 0.2 } else { 1.0 };
                let a = self.f_vox[0].run(src).1;
                let b2 = self.f_vox[1].run(src).1;
                let c3 = self.f_vox[2].run(src).1;
                let y = softclip((a * 1.0 + b2 * 0.7 + c3 * 0.35) * v.grit) * env * v.amp * 0.16;
                wall_l += y * 0.9;
                wall_r += y * 0.9;
                v.t += dt;
                if t >= v.dur {
                    v.on = false;
                }
            }
            // ---- buses: drive + tone on the wall, ring mod and crusher on the grit
            let dr = self.drive;
            let mut wl = softclip(wall_l * dr) * 0.8;
            let mut wr = softclip(wall_r * dr) * 0.8;
            let tc = 0.08 + 0.5 * self.tone;
            self.tone_lp[0] += (wl - self.tone_lp[0]) * tc;
            self.tone_lp[1] += (wr - self.tone_lp[1]) * tc;
            wl = self.tone_lp[0];
            wr = self.tone_lp[1];
            if self.ring > 1e-3 {
                adv(&mut self.ring_ph, self.ring_f * dt);
                let m = sine(tbl, self.ring_ph);
                let rm = self.ring;
                wl = wl * (1.0 - rm * 0.5) + wl * m * rm * 0.6;
                wr = wr * (1.0 - rm * 0.5) + wr * m * rm * 0.6;
                noise_l = noise_l * (1.0 - rm) + noise_l * m * rm * 1.4;
                noise_r = noise_r * (1.0 - rm) + noise_r * m * rm * 1.4;
            }
            if self.crush_n == 0 {
                self.crush_n = self.crush_hold;
                self.crush_v = [(noise_l * 24.0).round() / 24.0, (noise_r * 24.0).round() / 24.0];
            }
            self.crush_n -= 1;
            let (nl, nr) = (self.crush_v[0], self.crush_v[1]);
            // ---- sum, 8 kHz ceiling, glue compressor
            let lo = softclip((low + drums * 0.0) * 1.2) * 0.85;
            let tl = self.f_top[0].run(drums + ml + wl + nl).0;
            let tr = self.f_top[1].run(drums + mr + wr + nr).0;
            let mut l = self.f_top2[0].run(tl).0 + lo;
            let mut r = self.f_top2[1].run(tr).0 + lo;
            let p = l.abs().max(r.abs());
            let k = if p > self.comp_env { 0.006 } else { 0.0002 };
            self.comp_env += (p - self.comp_env) * k;
            let thr = 0.3;
            let g = if self.comp_env > thr { (thr + (self.comp_env - thr) / 4.0) / self.comp_env } else { 1.0 };
            let g = g * 0.62 * self.level;
            l *= g;
            r *= g;
            // ---- tape stop
            let n_t = self.tape.len();
            self.tape[self.tw] = [l, r];
            self.tw += 1;
            if self.tw == n_t {
                self.tw = 0;
            }
            if self.tape_on {
                let x = (self.tape_t / self.tape_dur).min(1.0);
                self.tape_v = (1.0 - x) * (1.0 - x);
                self.tape_t += dt;
                self.tr += self.tape_v as f64;
                if self.tr >= n_t as f64 {
                    self.tr -= n_t as f64;
                }
                let i0 = self.tr as usize % n_t;
                let i1 = (i0 + 1) % n_t;
                let fr = (self.tr - self.tr.floor()) as f32;
                let fade = 1.0 - x * x;
                l = (self.tape[i0][0] * (1.0 - fr) + self.tape[i1][0] * fr) * fade;
                r = (self.tape[i0][1] * (1.0 - fr) + self.tape[i1][1] * fr) * fade;
            }
            self.l[j] = l;
            self.r[j] = r;
        }
        // advance tones that live per block
    }

    /// control rate (every 16 samples): filter coefficients that move
    fn control(&mut self) {
        let sr = self.sr;
        if self.vox.on {
            let v = &mut self.vox;
            for k in 0..3 {
                v.vow[k] = v.from[k] + (v.to[k] - v.from[k]) * v.morph;
            }
            let q = [6.0, 8.0, 9.0];
            for k in 0..3 {
                self.f_vox[k].set(self.vox.vow[k], q[k], sr);
            }
        }
        let b = &self.bass;
        self.f_bass.set(120.0 + b.fenv * (300.0 + 1500.0 * b.bright), 0.9, sr);
        let c = &self.chug;
        let cut = if c.open { 3400.0 } else { 1400.0 + 2000.0 * c.fenv };
        self.f_cab[0].set(cut, 0.85, sr);
        self.f_cab[1].set(cut * 0.94, 0.85, sr);
        if self.sweep > 1e-3 || self.sweep_t > 0.0 {
            self.f_sweep.set(self.sweep_f, 2.5, sr);
        }
    }

    /// the pad follows the chord (called on chord changes)
    pub fn chord(&mut self, hz: &[f32]) {
        for k in 0..3.min(hz.len()) {
            self.pad_f[k] = hz[k];
        }
        let sr = self.sr;
        let f0 = hz.first().copied().unwrap_or(110.0);
        self.f_wh[0].set(f0 * 4.0, 9.0, sr);
        self.f_wh[1].set(f0 * 6.0, 9.0, sr);
        self.f_grit.set((f0 * 8.0).clamp(600.0, 2200.0), 0.8, sr);
    }
}
