//! Chord-progression engine: phrase templates + a Markov chord graph seeded
//! from them, with detours (secondary dominants, borrowed chords, colours) and
//! pivot-chord modulations. Fully independent of the OS; system events can
//! only *request* tension / modulation / resolution.

use crate::dsp::Rng;
use std::collections::VecDeque;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Q {
    Maj,
    Min,
    Dim,
    Sus2,
    Sus4,
    Dom7,
    Maj7,
    Min7,
    Add9,
    MinAdd9,
}

impl Q {
    pub fn intervals(self) -> &'static [i32] {
        match self {
            Q::Maj => &[0, 4, 7],
            Q::Min => &[0, 3, 7],
            Q::Dim => &[0, 3, 6],
            Q::Sus2 => &[0, 2, 7],
            Q::Sus4 => &[0, 5, 7],
            Q::Dom7 => &[0, 4, 7, 10],
            Q::Maj7 => &[0, 4, 7, 11],
            Q::Min7 => &[0, 3, 7, 10],
            Q::Add9 => &[0, 4, 7, 14],
            Q::MinAdd9 => &[0, 3, 7, 14],
        }
    }
    fn suffix(self) -> &'static str {
        match self {
            Q::Maj => "",
            Q::Min => "m",
            Q::Dim => "dim",
            Q::Sus2 => "sus2",
            Q::Sus4 => "sus4",
            Q::Dom7 => "7",
            Q::Maj7 => "maj7",
            Q::Min7 => "m7",
            Q::Add9 => "add9",
            Q::MinAdd9 => "m(add9)",
        }
    }
    fn minorish(self) -> bool {
        matches!(self, Q::Min | Q::Min7 | Q::MinAdd9 | Q::Dim)
    }
}

const FLATS: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];
const SHARPS: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
/// pitch class (C = 0) of semitone 0, i.e. of --root
static ROOT_PC: AtomicI32 = AtomicI32::new(9);
/// spell with sharps (follows the current key signature)
static USE_SHARPS: AtomicBool = AtomicBool::new(false);
/// C major / A minor: usual chromatic spellings (C# Eb F# G# Bb)
static NATURAL: AtomicBool = AtomicBool::new(true);
const NATURALS: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "G#", "A", "Bb", "B"];
/// leading tone (raised 7th) of the current minor key, relative to the root; -1 in major
static LEAD7: AtomicI32 = AtomicI32::new(-1);
pub fn set_root_pc(midi: i32) {
    ROOT_PC.store(midi.rem_euclid(12), Ordering::Relaxed);
}
fn spell_for(key: &Key) {
    // C-based tonic; sharp keys: G D A E B F# major and their relative minors
    let c = (key.tonic + ROOT_PC.load(Ordering::Relaxed)).rem_euclid(12);
    let maj = if key.minor { (c + 3) % 12 } else { c };
    // F#/Gb: sharps for the major key, flats for its relative (Eb minor, not D# minor)
    USE_SHARPS.store(matches!(maj, 7 | 2 | 9 | 4 | 11) || (maj == 6 && !key.minor), Ordering::Relaxed);
    NATURAL.store(maj == 0, Ordering::Relaxed);
    LEAD7.store(if key.minor { (key.tonic + 11).rem_euclid(12) } else { -1 }, Ordering::Relaxed);
}

/// pitch class relative to the global root (A = 0) -> name
pub fn pc_name(pc: i32) -> &'static str {
    let i = (pc + ROOT_PC.load(Ordering::Relaxed)).rem_euclid(12) as usize;
    // the raised 7th of a minor key is always a sharp/natural (C# in D minor, not Db)
    if pc.rem_euclid(12) == LEAD7.load(Ordering::Relaxed) {
        return SHARPS[i];
    }
    if NATURAL.load(Ordering::Relaxed) {
        NATURALS[i]
    } else if USE_SHARPS.load(Ordering::Relaxed) {
        SHARPS[i]
    } else {
        FLATS[i]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chord {
    pub root: i32, // pitch class relative to global root, 0..11
    pub q: Q,
}

/// Roman numeral of `c` in `key` (b/# for non-diatonic roots, lower case for minor).
pub fn roman_of(c: Chord, key: Key) -> String {
    const R: [&str; 12] = ["I", "bII", "II", "bIII", "III", "IV", "#IV", "V", "bVI", "VI", "bVII", "VII"];
    const RM: [&str; 12] = ["I", "bII", "II", "III", "#III", "IV", "#IV", "V", "VI", "#VI", "VII", "#VII"];
    let d = (c.root - key.tonic).rem_euclid(12) as usize;
    let base = if key.minor { RM[d] } else { R[d] };
    let mut r = if c.q.minorish() { base.to_lowercase() } else { base.to_string() };
    r.push_str(match c.q {
        Q::Dim => "\u{b0}",
        Q::Dom7 | Q::Min7 => "7",
        Q::Maj7 => "maj7",
        Q::Sus2 => "sus2",
        Q::Sus4 => "sus4",
        Q::Add9 | Q::MinAdd9 => "add9",
        _ => "",
    });
    r
}

impl Chord {
    pub fn name(&self) -> String {
        format!("{}{}", pc_name(self.root), self.q.suffix())
    }
    /// pitch classes (0..11, relative to global root)
    pub fn pcs(&self) -> Vec<i32> {
        self.q.intervals().iter().map(|i| (self.root + i).rem_euclid(12)).collect()
    }
    pub fn fifth(&self) -> i32 {
        let iv = self.q.intervals();
        (self.root + iv[2]).rem_euclid(12)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub tonic: i32, // pc relative to global root
    pub minor: bool,
}

const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR: [i32; 7] = [0, 2, 3, 5, 7, 8, 10];

impl Key {
    pub fn scale(&self) -> [i32; 7] {
        let s = if self.minor { MINOR } else { MAJOR };
        let mut o = [0; 7];
        for i in 0..7 {
            o[i] = (self.tonic + s[i]).rem_euclid(12);
        }
        o
    }
    pub fn name(&self) -> String {
        let prev = (USE_SHARPS.load(Ordering::Relaxed), NATURAL.load(Ordering::Relaxed), LEAD7.load(Ordering::Relaxed));
        spell_for(self);
        let n = format!("{} {}", pc_name(self.tonic), if self.minor { "minor" } else { "major" });
        USE_SHARPS.store(prev.0, Ordering::Relaxed);
        NATURAL.store(prev.1, Ordering::Relaxed);
        LEAD7.store(prev.2, Ordering::Relaxed);
        n
    }
    /// pentatonic subset (for sparse melodic material)
    pub fn penta(&self) -> [i32; 5] {
        let s = if self.minor { [0, 3, 5, 7, 10] } else { [0, 2, 4, 7, 9] };
        let mut o = [0; 5];
        for i in 0..5 {
            o[i] = (self.tonic + s[i]).rem_euclid(12);
        }
        o
    }
    /// Ascending semitone offset (from the global root) of scale degree `d`
    /// (0 = tonic; may be negative or > 6).
    pub fn deg_semi(&self, d: i32) -> i32 {
        let s = if self.minor { MINOR } else { MAJOR };
        self.tonic + s[d.rem_euclid(7) as usize] + 12 * d.div_euclid(7)
    }
    /// Scale degree index of a pitch class (nearest if not in the scale).
    pub fn degree_of(&self, pc: i32) -> i32 {
        let sc = self.scale();
        let mut best = 0;
        let mut bd = 99;
        for (i, &s) in sc.iter().enumerate() {
            let d = (s - pc).rem_euclid(12).min((pc - s).rem_euclid(12));
            if d < bd {
                bd = d;
                best = i as i32;
            }
        }
        best
    }
    pub fn contains(&self, c: &Chord) -> bool {
        let sc = self.scale();
        c.pcs().iter().all(|p| sc.contains(p))
    }
}

/// Parse a roman numeral relative to a key: I ii iii IV V vi vii° (major),
/// i ii° III iv v VI VII (minor); prefixes b/#, suffixes 7 maj7 sus2 sus4 add9,
/// secondary "V/x".
pub fn roman(s: &str, key: Key) -> Chord {
    if let Some((a, b)) = s.split_once('/') {
        let target = roman(b, key);
        let mut c = roman(a, Key { tonic: target.root, minor: false });
        c.root = c.root.rem_euclid(12);
        return c;
    }
    let mut t = s;
    let mut shift = 0;
    while let Some(r) = t.strip_prefix('b') {
        shift -= 1;
        t = r;
    }
    while let Some(r) = t.strip_prefix('#') {
        shift += 1;
        t = r;
    }
    let up = t.to_ascii_uppercase();
    let (deg, len) = [("VII", 6), ("VI", 5), ("IV", 3), ("V", 4), ("III", 2), ("II", 1), ("I", 0)]
        .iter()
        .find(|(p, _)| up.starts_with(p))
        .map(|(p, d)| (*d, p.len()))
        .unwrap_or((0, 1));
    let upper = t.as_bytes()[0].is_ascii_uppercase();
    let rest = &t[len..];
    let base = if key.minor { MINOR } else { MAJOR };
    let root = (key.tonic + base[deg] + shift).rem_euclid(12);
    let q = if rest.contains('°') || rest.contains("dim") {
        Q::Dim
    } else if rest.contains("maj7") {
        Q::Maj7
    } else if rest.contains("sus2") {
        Q::Sus2
    } else if rest.contains("sus4") {
        Q::Sus4
    } else if rest.contains("add9") {
        if upper { Q::Add9 } else { Q::MinAdd9 }
    } else if rest.contains('7') {
        if upper { Q::Dom7 } else { Q::Min7 }
    } else if upper {
        Q::Maj
    } else {
        Q::Min
    };
    Chord { root, q }
}

pub struct Template {
    pub minor: bool,
    pub chords: &'static [&'static str],
    pub w: f32,
}

pub const TEMPLATES: &[Template] = &[
    Template { minor: false, chords: &["I", "V", "vi", "IV"], w: 3.0 },
    Template { minor: false, chords: &["vi", "IV", "I", "V"], w: 2.5 },
    Template { minor: false, chords: &["ii", "V", "I", "I"], w: 1.5 },
    Template { minor: false, chords: &["I", "vi", "IV", "V"], w: 2.0 },
    Template { minor: false, chords: &["I", "IV", "vi", "V"], w: 1.5 },
    Template { minor: false, chords: &["IV", "I", "V", "vi"], w: 1.2 },
    Template { minor: false, chords: &["I", "iii", "IV", "V"], w: 1.0 },
    Template { minor: false, chords: &["I", "V", "vi", "iii", "IV", "I", "IV", "V"], w: 1.5 },
    Template { minor: false, chords: &["I", "IV", "ii", "V", "I", "vi", "ii", "V"], w: 1.0 },
    Template { minor: false, chords: &["IV", "V", "iii", "vi"], w: 1.0 },
    Template { minor: true, chords: &["i", "VI", "III", "VII"], w: 3.0 },
    Template { minor: true, chords: &["i", "iv", "v", "i"], w: 1.5 },
    Template { minor: true, chords: &["i", "VII", "VI", "VII"], w: 2.0 },
    Template { minor: true, chords: &["i", "VII", "VI", "V"], w: 2.0 }, // Andalusian
    Template { minor: true, chords: &["i", "iv", "VII", "III"], w: 1.5 },
    Template { minor: true, chords: &["i", "III", "VII", "iv"], w: 1.2 },
    Template { minor: true, chords: &["VI", "VII", "i", "i"], w: 1.2 },
    Template { minor: true, chords: &["i", "VI", "iv", "V"], w: 1.0 },
    Template { minor: true, chords: &["i", "VII", "VI", "V", "i", "VII", "VI", "V"], w: 1.0 },
    Template { minor: true, chords: &["i", "v", "VI", "III", "iv", "i", "iv", "v"], w: 1.0 },
];

#[derive(Clone, Debug)]
pub struct Change {
    pub chord: Chord,
    pub key: Key,
    pub key_changed: bool,
    pub note: Option<String>,
}

/// style-flavoured phrase templates (0 drone, 1 jazz, 2 rock, 3 chorale, 4 modal,
/// 5 12-bar blues (one bar per chord), 6 country, 7 techno loops)
pub const STYLE_TEMPLATES: &[(u8, Template)] = &[
    (5, Template { minor: false, chords: &["I7", "I7", "I7", "I7", "IV7", "IV7", "I7", "I7", "V7", "IV7", "I7", "V7"], w: 3.0 }),
    (5, Template { minor: false, chords: &["I7", "IV7", "I7", "I7", "IV7", "IV7", "I7", "I7", "V7", "IV7", "I7", "V7"], w: 2.5 }), // quick change
    (5, Template { minor: false, chords: &["I7", "IV7", "I7", "I7", "IV7", "IV7", "I7", "VI7", "ii7", "V7", "I7", "V7"], w: 1.5 }), // turnaround via VI7–ii–V
    (5, Template { minor: false, chords: &["I7", "I7", "I7", "I7", "IV7", "IV7", "I7", "I7", "V7", "IV7", "I7", "IV7"], w: 1.0 }), // I–IV turnaround
    (5, Template { minor: true, chords: &["i7", "i7", "i7", "i7", "iv7", "iv7", "i7", "i7", "VI7", "V7", "i7", "V7"], w: 3.0 }),
    (5, Template { minor: true, chords: &["i7", "iv7", "i7", "i7", "iv7", "iv7", "i7", "i7", "VI7", "V7", "i7", "V7"], w: 2.0 }),
    (6, Template { minor: false, chords: &["I", "IV", "V", "I"], w: 3.0 }),
    (6, Template { minor: false, chords: &["I", "vi", "IV", "V"], w: 2.5 }),
    (6, Template { minor: false, chords: &["I", "I", "IV", "I", "V", "V", "I", "I"], w: 2.0 }),
    (6, Template { minor: false, chords: &["I", "IV", "I", "V7"], w: 1.5 }),
    (6, Template { minor: false, chords: &["I", "bVII", "IV", "I"], w: 1.0 }), // mixolydian
    (6, Template { minor: true, chords: &["VI", "VII", "III", "III"], w: 1.0 }), // relative-major I–IV–V from the minor side
    (7, Template { minor: true, chords: &["i7", "i7", "i7", "i7"], w: 3.0 }),
    (7, Template { minor: true, chords: &["i7", "i7", "VII", "VII"], w: 2.0 }),
    (7, Template { minor: true, chords: &["i7", "iv7", "i7", "iv7"], w: 2.0 }),
    (7, Template { minor: true, chords: &["i7", "VI", "i7", "VII"], w: 1.5 }),
    (7, Template { minor: false, chords: &["vi7", "vi7", "IV", "IV"], w: 1.5 }),
    (7, Template { minor: false, chords: &["Imaj7", "Imaj7", "ii7", "ii7"], w: 1.5 }),
    (0, Template { minor: false, chords: &["I", "I", "II", "I"], w: 2.0 }), // Lydian II
    (0, Template { minor: false, chords: &["Iadd9", "IVmaj7", "Iadd9", "Isus2"], w: 1.5 }),
    (0, Template { minor: true, chords: &["i", "i", "VI", "i"], w: 2.0 }),
    (0, Template { minor: true, chords: &["isus2", "iv", "isus2", "VII"], w: 1.2 }),
    (1, Template { minor: false, chords: &["ii7", "V7", "Imaj7", "Imaj7"], w: 3.0 }),
    (1, Template { minor: false, chords: &["Imaj7", "vi7", "ii7", "V7"], w: 2.5 }),
    (1, Template { minor: false, chords: &["iii7", "vi7", "ii7", "V7"], w: 2.0 }),
    (1, Template { minor: false, chords: &["Imaj7", "IV7", "iii7", "VI7", "ii7", "V7", "Imaj7", "Imaj7"], w: 1.5 }),
    (1, Template { minor: true, chords: &["ii°", "V7", "i7", "i7"], w: 2.5 }),
    (1, Template { minor: true, chords: &["i7", "iv7", "VII7", "IIImaj7"], w: 2.0 }),
    (2, Template { minor: false, chords: &["I", "bVII", "IV", "I"], w: 3.0 }), // Mixolydian
    (2, Template { minor: false, chords: &["I", "I", "bVII", "IV"], w: 2.0 }),
    (2, Template { minor: false, chords: &["I", "IV", "I", "V"], w: 1.5 }),
    (2, Template { minor: true, chords: &["i", "VII", "VI", "VII"], w: 2.0 }),
    (2, Template { minor: true, chords: &["i", "i", "VII", "IV"], w: 1.5 }), // Dorian IV
    (3, Template { minor: false, chords: &["I", "IV", "V", "I"], w: 3.0 }),
    (3, Template { minor: false, chords: &["I", "vi", "ii", "V7", "I", "IV", "V7", "I"], w: 2.5 }),
    (3, Template { minor: false, chords: &["I", "V", "vi", "iii", "IV", "I", "ii", "V7"], w: 2.0 }), // Pachelbel-ish
    (3, Template { minor: false, chords: &["I", "ii", "V7", "I"], w: 2.0 }),
    (3, Template { minor: true, chords: &["i", "iv", "V7", "i"], w: 3.0 }),
    (3, Template { minor: true, chords: &["i", "VI", "ii°", "V7"], w: 2.0 }),
    (4, Template { minor: false, chords: &["I", "bVII", "I", "bVII"], w: 2.5 }),
    (4, Template { minor: false, chords: &["I", "IV", "bVII", "I"], w: 2.0 }),
    (4, Template { minor: true, chords: &["i", "IV", "i", "IV"], w: 2.0 }), // Dorian
    (4, Template { minor: true, chords: &["i", "bII", "i", "VII"], w: 2.0 }), // Phrygian
    (4, Template { minor: true, chords: &["i", "VII", "i", "v"], w: 1.5 }),
];

pub struct Harmony {
    /// style flavour: None = plain templates; Some(f) favours STYLE_TEMPLATES of f
    pub flavour: Option<u8>,
    /// jazz: turn triads into sevenths
    pub sevenths: bool,
    /// style's major/minor lean (−1..1) and modulation rate
    pub major_bias: f32,
    pub mod_rate: f32,
    rng: Rng,
    pub key: Key,
    home: Key,
    queue: VecDeque<(Chord, Option<String>)>,
    pub cur: Chord,
    bars: u32,
    phrases_away: u32,
    settle: u32,
    tension: bool,
    /// a user style switch: drop the plan, change chord at the next bar with the new flavour
    restart: bool,
    modulate: bool,
    resolve: bool,
    pub warmth: f32,
    last_template: usize,
    key_after: Option<(Key, usize)>,
    /// human-readable harmony events (new home key etc.), drained by the engine
    pub events: Vec<String>,
    /// memory: the last 32 chords (with their key), phrase plans, cadences, tension
    hist: VecDeque<(Chord, Key)>,
    phrases: VecDeque<PastPhrase>,
    cadences: VecDeque<Goal>,
    tension_hist: VecDeque<f32>,
    phrase_no: u32,
    style_tpl: VecDeque<usize>,
    /// where the band is heading, for the UI ("cadence home to C in C major: Am → F → G7 → C")
    pub plan_desc: String,
    /// new plans to log, drained by the engine
    pub plans: Vec<String>,
}

impl Harmony {
    pub fn new(seed: u64, minor: bool) -> Self {
        let key = Key { tonic: 0, minor };
        spell_for(&key);
        let cur = roman(if minor { "i" } else { "I" }, key);
        Harmony {
            flavour: None,
            sevenths: false,
            major_bias: 0.0,
            mod_rate: 1.0,
            rng: Rng::seeded(seed ^ 0x4841524d),
            key,
            home: key,
            queue: VecDeque::new(),
            restart: false,
            cur,
            bars: 0,
            phrases_away: 0,
            settle: 0,
            tension: false,
            modulate: false,
            resolve: false,
            warmth: 0.0,
            last_template: usize::MAX,
            key_after: None,
            events: Vec::new(),
            hist: VecDeque::new(),
            phrases: VecDeque::new(),
            cadences: VecDeque::new(),
            tension_hist: VecDeque::new(),
            phrase_no: 0,
            style_tpl: VecDeque::new(),
            plan_desc: String::new(),
            plans: Vec::new(),
        }
    }

    pub fn request_tension(&mut self) {
        self.tension = true;
    }
    pub fn request_modulation(&mut self) {
        self.modulate = true;
    }
    pub fn request_resolve(&mut self) {
        self.resolve = true;
    }

    /// The chord that will follow the current one (for bass approach notes).
    pub fn peek_next(&mut self, meander: bool) -> Chord {
        if self.queue.is_empty() {
            self.plan(meander);
        }
        self.queue.front().map(|c| c.0).unwrap_or(self.cur)
    }

    /// chords already planned for this phrase (for display)
    pub fn home(&self) -> Key {
        self.home
    }

    pub fn upcoming(&self, n: usize) -> Vec<Chord> {
        self.queue.iter().take(n).map(|c| c.0).collect()
    }

    fn colourise(&mut self, c: Chord, p: f32) -> Chord {
        if !self.rng.chance(p) {
            return c;
        }
        let q = match c.q {
            Q::Maj => [Q::Sus2, Q::Sus4, Q::Add9, Q::Maj7][self.rng.below(4)],
            Q::Min => [Q::Min7, Q::MinAdd9, Q::Sus2, Q::Min7][self.rng.below(4)],
            q => q,
        };
        let cand = Chord { root: c.root, q };
        // keep colours diatonic unless it's a sus (sus is always fine)
        if matches!(q, Q::Sus2 | Q::Sus4) || self.key.contains(&cand) { cand } else { c }
    }

    /// Markov chord graph (bigrams from all templates of this mode).
    fn markov_next(&mut self, from: Chord) -> Chord {
        let key = self.key;
        let mut cand: Vec<(Chord, f32)> = Vec::new();
        for t in TEMPLATES.iter().filter(|t| t.minor == key.minor) {
            let n = t.chords.len();
            for i in 0..n {
                let a = roman(t.chords[i], key);
                if a.root == from.root && a.q.minorish() == from.q.minorish() {
                    let b = roman(t.chords[(i + 1) % n], key);
                    if b.root != from.root {
                        cand.push((b, t.w));
                    }
                }
            }
        }
        if cand.is_empty() {
            return roman(if key.minor { "i" } else { "I" }, key);
        }
        let w: Vec<f32> = cand.iter().map(|c| c.1).collect();
        cand[self.rng.weighted(&w)].0
    }

    fn plan(&mut self, meander: bool) {
        let before = self.plan_desc.clone();
        self.plan_inner(meander);
        if self.plan_desc != before && !self.plan_desc.is_empty() {
            self.plans.push(self.plan_desc.clone());
        }
    }

    fn plan_inner(&mut self, meander: bool) {
        // form-driven styles never meander off their form
        let meander = meander && !matches!(self.flavour, Some(5) | Some(6) | Some(7));
        // 12-bar blues and techno loops hold their form: no modulations or cadential detours
        let locked = matches!(self.flavour, Some(5) | Some(7));
        if locked {
            self.modulate = false;
            self.resolve = false;
            self.home = self.key;
        }
        let key = self.key;
        let tonic = roman(if key.minor { "i" } else { "I" }, key);
        let dom = roman("V7", key); // major V even in minor (harmonic minor)
        if self.resolve {
            self.resolve = false;
            self.tension = false;
            self.modulate = false;
            self.settle = 4; // stay home for a while

            if self.rng.chance(0.2) && self.key != self.home {
                // resolve all the way home
                self.modulate_to(self.home, "return home");
                return;
            }
            self.queue.push_back((dom, Some("cadence V7".into())));
            self.queue.push_back((tonic, Some("resolves to tonic".into())));
            self.plan_desc = format!("resolving home: {} → {}", dom.name(), tonic.name());
            self.cadences.push_back(Goal::Authentic);
            return;
        }
        // a key that has lasted a few phrases may become the new home
        if self.key != self.home && self.key_after.is_none() && self.phrases_away >= 3 && self.rng.chance(0.4) {
            let old = self.home;
            self.home = self.key;
            self.phrases_away = 0;
            self.settle = 2;
            self.events.push(format!("new home key: {} (was {})", self.key.name(), old.name()));
            // confirm it cadentially
            self.queue.push_back((dom, Some(format!("cadence into the new home {}", self.key.name()))));
            self.queue.push_back((tonic, None));
            self.plan_desc = format!("settling into {} as the new home: {} → {}", self.key.name(), dom.name(), tonic.name());
            return;
        }
        // a gentle pull back home only after a long time away
        let away_pull = if self.key != self.home { (self.phrases_away as f32 - 6.0).max(0.0) * 0.03 } else { 0.0 };
        self.settle = self.settle.saturating_sub(1);
        if !locked && (self.modulate || (self.settle == 0 && self.rng.chance((if meander { 0.03 } else { 0.05 }) * self.mod_rate + away_pull))) {
            self.modulate = false;
            let p_major = (0.5 + 0.35 * self.warmth + 0.4 * self.major_bias).clamp(0.08, 0.92);
            let target = if self.key != self.home && self.rng.chance(0.15 + away_pull) {
                self.home
            } else {
                let r = self.rng.f();
                if r < 0.25 {
                    // relative major/minor pivot
                    Key { tonic: (key.tonic + if key.minor { 3 } else { -3 }).rem_euclid(12), minor: !key.minor }
                } else if r < 0.38 && self.rng.chance(if key.minor { p_major } else { 1.0 - p_major }) {
                    // parallel major/minor (screen colour decides which way it leans)
                    Key { tonic: key.tonic, minor: !key.minor }
                } else {
                    // around the circle of fifths
                    let step = if self.rng.chance(0.5) { 7 } else { 5 };
                    Key { tonic: (key.tonic + step).rem_euclid(12), minor: !self.rng.chance(p_major) }
                }
            };
            self.modulate_to(target, "modulation");
            return;
        }
        if meander && self.plan_goal(true) {
            // drift without a destination (novelty-aware), then colour it
            let n = self.queue.len();
            for i in n.saturating_sub(4)..n {
                let c = self.queue[i].0;
                let mut cc = self.colourise(c, 0.35);
                cc = self.seventh(cc);
                if self.rng.chance(0.06) {
                    cc = self.borrowed();
                    self.queue[i].1 = Some(format!("borrowed {}", cc.name()));
                }
                self.queue[i].0 = cc;
            }
            let shown: Vec<Chord> = self.queue.iter().skip(n.saturating_sub(4)).map(|q| q.0).collect();
            self.plan_desc = format!("drifting in {}: {}", self.key.name(), Self::names(&shown));
            self.phrases_away += (self.key != self.home) as u32;
            return;
        }
        if meander {
            // wander on the Markov graph, lots of colour, occasional borrowed chord
            let mut c = self.cur;
            for _ in 0..4 {
                c = self.markov_next(c);
                let mut cc = self.colourise(c, 0.35);
                cc = self.seventh(cc);
                let mut note = None;
                if self.rng.chance(0.06) {
                    cc = self.borrowed();
                    note = Some(format!("borrowed {}", cc.name()));
                }
                self.queue.push_back((cc, note));
            }
            self.phrases_away += (self.key != self.home) as u32;
            return;
        }
        // a style's own phrase shapes
        if let Some(f) = self.flavour {
            // forms (12-bar, country, techno loops) always; a style riff now and then otherwise
            let form = locked || f == 6;
            if form || self.rng.chance(0.3) {
                let idx: Vec<usize> = (0..STYLE_TEMPLATES.len())
                    .filter(|&i| STYLE_TEMPLATES[i].0 == f && STYLE_TEMPLATES[i].1.minor == key.minor && (form || !self.style_tpl.contains(&i)))
                    .collect();
                if !idx.is_empty() {
                    let w: Vec<f32> = idx.iter().map(|&i| STYLE_TEMPLATES[i].1.w * if roman(STYLE_TEMPLATES[i].1.chords[0], key).root == self.cur.root { 0.6 } else { 1.0 }).collect();
                    let ti = idx[self.rng.weighted(&w)];
                    self.style_tpl.push_back(ti);
                    while self.style_tpl.len() > 3 {
                        self.style_tpl.pop_front();
                    }
                    let t = &STYLE_TEMPLATES[ti].1;
                    let chords: Vec<Chord> = t.chords.iter().map(|c| self.seventh(roman(c, key))).collect();
                    let names = Self::names(&chords);
                    let kn = key.name();
                    let (first, last) = (chords[0], chords[chords.len() - 1]);
                    self.plan_desc = match f {
                        5 => format!("12-bar in {kn}, turnaround {} → back to {}", last.name(), first.name()),
                        6 => format!("country changes in {kn}, heading home to {}: {names}", roman("I", key).name()),
                        7 => format!("loop in {kn}, sitting on {}: {names}", first.name()),
                        _ => format!("{} riff in {kn}: {names}", ["drone", "jazz", "rock", "chorale", "modal", "", "", ""][f as usize % 8]),
                    };
                    let note = format!("{} phrase", ["drone", "jazz", "rock", "chorale", "modal", "12-bar blues", "country", "loop"][f as usize % 8]);
                    self.push_phrase(chords, Some(note), if f == 7 { Goal::Drift } else { Goal::Authentic }, false);
                    self.phrases_away += (self.key != self.home) as u32;
                    return;
                }
            }
        }
        // a phrase with a destination (beam search over the chord graph)
        if self.plan_goal(false) {
            self.phrases_away += (self.key != self.home) as u32;
            return;
        }
        // template phrase, chosen with Markov-ish affinity to the current chord
        let idx: Vec<usize> = (0..TEMPLATES.len()).filter(|&i| TEMPLATES[i].minor == key.minor).collect();
        let w: Vec<f32> = idx
            .iter()
            .map(|&i| {
                let t = &TEMPLATES[i];
                let first = roman(t.chords[0], key);
                let mut w = t.w;
                if i == self.last_template {
                    w *= 0.3;
                }
                if first.root == self.cur.root {
                    w *= 0.5; // avoid repeating the same chord across the barline
                }
                w
            })
            .collect();
        let ti = idx[self.rng.weighted(&w)];
        self.last_template = ti;
        let t = &TEMPLATES[ti];
        let mut phrase: Vec<(Chord, Option<String>)> = t.chords.iter().map(|s| (roman(s, key), None)).collect();
        if self.rng.chance(0.13) {
            // a detour inside this phrase
            let i = 1 + self.rng.below(phrase.len() - 1);
            match self.rng.below(3) {
                0 => {
                    // secondary dominant of the chord at i
                    let target = phrase[i].0;
                    if target.root != tonic.root {
                        let sd = Chord { root: (target.root + 7).rem_euclid(12), q: Q::Dom7 };
                        phrase[i - 1] = (sd, Some(format!("secondary dominant {} (V/{})", sd.name(), target.name())));
                    }
                }
                1 => {
                    let b = self.borrowed();
                    phrase[i] = (b, Some(format!("borrowed {}", b.name())));
                }
                _ => {
                    for p in phrase.iter_mut() {
                        p.0 = self.colourise(p.0, 0.5);
                    }
                    phrase[0].1 = Some("colour phrase".into());
                }
            }
        }
        for p in phrase {
            let c = self.colourise(p.0, 0.1);
            let c = self.seventh(c);
            self.queue.push_back((c, p.1));
        }
        self.phrases_away += (self.key != self.home) as u32;
    }

    /// jazz: every triad grows a seventh (dominant on V)
    fn seventh(&self, c: Chord) -> Chord {
        if !self.sevenths {
            return c;
        }
        let deg = (c.root - self.key.tonic).rem_euclid(12);
        let q = match c.q {
            Q::Maj if deg == 7 || !self.key.contains(&Chord { root: c.root, q: Q::Maj7 }) => Q::Dom7,
            Q::Maj | Q::Add9 => Q::Maj7,
            Q::Min | Q::MinAdd9 => Q::Min7,
            q => q,
        };
        Chord { root: c.root, q }
    }

    fn borrowed(&mut self) -> Chord {
        let key = self.key;
        let opts: &[&str] = if key.minor { &["IV", "V", "bII"] } else { &["bVI", "bVII", "iv", "bIII"] };
        let s = opts[self.rng.below(opts.len())];
        roman(s, key)
    }

    fn modulate_to(&mut self, to: Key, why: &str) {
        let from = self.key;
        // pivot: a chord diatonic to both keys (prefer one near the old tonic)
        let mut pivot = None;
        for s in ["vi", "IV", "ii", "iii", "I", "V", "i", "iv", "VI", "III", "VII"] {
            let c = roman(s, from);
            if c.q != Q::Dim && to.contains(&c) && from.contains(&c) {
                pivot = Some(c);
                break;
            }
        }
        let new_tonic = self.seventh(roman(if to.minor { "i" } else { "I" }, to));
        let new_dom = roman("V7", to);
        // predominant of the new key, the one closest to the pivot
        let from_c = pivot.unwrap_or(self.cur);
        let pds: Vec<Chord> = (if to.minor { vec!["iv", "ii°", "VI"] } else { vec!["ii", "IV"] }).into_iter().map(|s| self.seventh(roman(s, to))).filter(|c| *c != from_c).collect();
        let pd = pds.iter().copied().min_by_key(|c| vl(&from_c, c).0 - vl(&from_c, c).1).unwrap_or(new_dom);
        let mut path = Vec::new();
        if let Some(p) = pivot {
            self.queue.push_back((p, Some(format!("{why}: pivot {}", p.name()))));
            path.push(format!("{} (pivot)", p.name()));
        }
        self.queue.push_back((pd, None));
        path.push(pd.name());
        path.push(new_dom.name());
        path.push(new_tonic.name());
        self.plan_desc = format!("heading to {}: {}", to.name(), path.join(" → "));
        self.cadences.push_back(Goal::Authentic);
        self.queue.push_back((new_dom, None));
        self.queue.push_back((new_tonic, Some(format!("{why}: {} -> {}", from.name(), to.name()))));
        self.key_after = Some((to, self.queue.len()));
        self.phrases_away = 0;
    }

    /// Called on every bar line. Returns a Change when the chord moves.
    pub fn restart(&mut self) {
        self.queue.clear();
        self.key_after = None;
        self.tension = false;
        self.restart = true;
    }
    pub fn restarting(&self) -> bool {
        self.restart
    }

    pub fn on_bar(&mut self, bars_per_chord: u32, meander: bool) -> Option<Change> {
        let r = self.on_bar_inner(bars_per_chord, meander);
        spell_for(&self.key);
        r
    }

    fn on_bar_inner(&mut self, bars_per_chord: u32, meander: bool) -> Option<Change> {
        self.bars += 1;
        let tension_now = self.tension;
        if self.bars < bars_per_chord.max(1) && !tension_now && !self.restart {
            return None;
        }
        self.bars = 0;
        self.restart = false;
        if tension_now {
            self.tension = false;
            // dominant / secondary dominant of what comes next, or a borrowed chord
            let next = self.peek_next(meander);
            let key = self.key;
            let tonic = roman(if key.minor { "i" } else { "I" }, key);
            let (c, why) = if self.rng.chance(0.25) {
                let b = self.borrowed();
                (b, format!("tension: borrowed {}", b.name()))
            } else if next.root == tonic.root || self.rng.chance(0.4) {
                let d = roman("V7", key);
                self.queue.push_front((tonic, None));
                (d, format!("tension: {} (V7)", d.name()))
            } else {
                let sd = Chord { root: (next.root + 7).rem_euclid(12), q: Q::Dom7 };
                (sd, format!("tension: {} (V/{})", sd.name(), next.name()))
            };
            return Some(self.apply(c, Some(why)));
        }
        if self.queue.is_empty() {
            self.plan(meander);
        }
        let (c, note) = self.queue.pop_front().unwrap_or((self.cur, None));
        if c == self.cur && note.is_none() && self.key_after.is_none() {
            // same chord again: just let it ring longer
            return None;
        }
        if let Some((k, n)) = self.key_after {
            // key switches when we reach the new tonic
            if n <= 1 {
                self.key_after = None;
                let ch = self.apply(c, note);
                let changed = self.key != k;
                self.key = k;
                return Some(Change { key: k, key_changed: changed, ..ch });
            } else {
                self.key_after = Some((k, n - 1));
            }
        }
        Some(self.apply(c, note))
    }

    fn apply(&mut self, c: Chord, note: Option<String>) -> Change {
        self.cur = c;
        let t = self.tension_of(c);
        self.hist.push_back((c, self.key));
        self.tension_hist.push_back(t);
        while self.hist.len() > 32 {
            self.hist.pop_front();
        }
        while self.tension_hist.len() > 32 {
            self.tension_hist.pop_front();
        }
        Change { chord: c, key: self.key, key_changed: false, note }
    }
}

// ------------------------------------------------------------ goal-directed planner

/// harmonic function of a chord in the planner's graph
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Func {
    T,
    PD,
    D,
    /// secondary dominant: wants to resolve to this root next
    SecD(i32),
    /// borrowed / modal colour
    Bor,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Goal {
    Authentic,
    Half,
    Deceptive,
    Plagal,
    Climax,
    Drift,
}

#[derive(Clone)]
pub struct PastPhrase {
    pub key: Key,
    pub chords: Vec<Chord>,
    pub callback: bool,
    pub no: u32,
}

fn circ(a: i32, b: i32) -> i32 {
    let d = (a - b).rem_euclid(12);
    d.min(12 - d)
}

/// rough voice-leading cost and common tones between two chords
fn vl(a: &Chord, b: &Chord) -> (i32, i32) {
    let (pa, pb) = (a.pcs(), b.pcs());
    let mut cost = 0;
    for &x in &pa {
        cost += pb.iter().map(|&y| circ(x, y)).min().unwrap_or(0);
    }
    let common = pa.iter().filter(|x| pb.contains(x)).count() as i32;
    (cost, common)
}

impl Harmony {
    /// the chord vocabulary of a key for this style: (chord, function, tension 0..1, label)
    fn vocab(&self, key: Key) -> Vec<(Chord, Func, f32)> {
        let f = self.flavour;
        let jazz = f == Some(1) || self.sevenths;
        let modal = matches!(f, Some(2) | Some(4));
        let drone = f == Some(0);
        let mut v: Vec<(&str, Func, f32)> = if key.minor {
            vec![("i", Func::T, 0.05), ("III", Func::T, 0.25), ("iv", Func::PD, 0.45), ("VI", Func::PD, 0.35), ("ii°", Func::PD, 0.5), ("V7", Func::D, 0.8), ("v", Func::D, 0.5), ("VII", Func::Bor, 0.5)]
        } else {
            vec![("I", Func::T, 0.05), ("vi", Func::T, 0.25), ("iii", Func::T, 0.3), ("IV", Func::PD, 0.45), ("ii", Func::PD, 0.4), ("V", Func::D, 0.75), ("V7", Func::D, 0.8)]
        };
        if !key.minor {
            if modal || drone {
                v.push(("bVII", Func::Bor, 0.45));
            } else {
                v.push(("bVII", Func::Bor, 0.5));
                v.push(("bVI", Func::Bor, 0.55));
                v.push(("iv", Func::PD, 0.55));
            }
        }
        if f == Some(3) && !key.minor {
            v.push(("vii°", Func::D, 0.85));
        }
        let mut out: Vec<(Chord, Func, f32)> = v.iter().map(|(s, fu, t)| (roman(s, key), *fu, *t)).collect();
        if !drone {
            // secondary dominants of ii, IV, V, vi (major) / iv, VI, III (minor)
            let targets: &[&str] = if key.minor { &["iv", "VI", "III"] } else { &["ii", "IV", "V", "vi"] };
            for t in targets {
                let tc = roman(t, key);
                out.push((Chord { root: (tc.root + 7).rem_euclid(12), q: Q::Dom7 }, Func::SecD(tc.root), 0.7));
            }
        }
        if jazz {
            let tonic = roman(if key.minor { "i" } else { "I" }, key);
            // tritone substitute (bII7 → I) and the backdoor dominant (bVII7 → I)
            out.push((Chord { root: (tonic.root + 1).rem_euclid(12), q: Q::Dom7 }, Func::D, 0.85));
            out.push((Chord { root: (tonic.root + 10).rem_euclid(12), q: Q::Dom7 }, Func::D, 0.65));
        }
        if modal {
            // rock / modal styles lean on IV and bVII, and V7 is rarer
            for x in out.iter_mut() {
                if x.0.q == Q::Dom7 && x.1 == Func::D {
                    x.2 += 0.1;
                }
            }
        }
        out.into_iter().map(|(c, fu, t)| (self.seventh(c), fu, t)).collect()
    }

    /// style vocabulary: how often a → b appears in this flavour's templates and in the general ones
    fn bigram_w(&self, key: Key, a: Chord, b: Chord) -> f32 {
        let same = |x: Chord, y: Chord| x.root == y.root && x.q.minorish() == y.q.minorish();
        let mut w = 0.0;
        let mut add = |t: &Template, k: f32| {
            if t.minor != key.minor {
                return;
            }
            let n = t.chords.len();
            for i in 0..n {
                if same(roman(t.chords[i], key), a) && same(roman(t.chords[(i + 1) % n], key), b) {
                    w += k * t.w;
                }
            }
        };
        if let Some(f) = self.flavour {
            for (fl, t) in STYLE_TEMPLATES {
                if *fl == f {
                    add(t, 1.0);
                }
            }
        }
        for t in TEMPLATES {
            add(t, 0.4);
        }
        w
    }

    /// pick where the next phrase is going, from what happened before
    fn choose_goal(&mut self, meander: bool) -> Goal {
        if meander {
            return Goal::Drift;
        }
        let last = self.cadences.back().copied();
        let auth_run = self.cadences.iter().rev().take_while(|g| **g == Goal::Authentic).count();
        let th = self.tension_hist.iter().rev().take(8).sum::<f32>() / self.tension_hist.len().clamp(1, 8) as f32;
        let modal = matches!(self.flavour, Some(2) | Some(4) | Some(0));
        // an open (half-cadence) phrase is answered by a closed one
        if matches!(last, Some(Goal::Half) | Some(Goal::Climax)) || th > 0.55 {
            return if modal && self.rng.chance(0.5) { Goal::Plagal } else { Goal::Authentic };
        }
        let mut w = [3.0f32, 2.0, 1.0, 1.0, 0.8];
        if auth_run >= 2 {
            w[0] = 0.4; // cadence fatigue: leave the next one open
        }
        if modal {
            w[3] = 3.0;
            w[2] = 0.3;
        }
        if self.flavour == Some(3) {
            w[1] += 1.0;
            w[2] += 0.8;
        }
        [Goal::Authentic, Goal::Half, Goal::Deceptive, Goal::Plagal, Goal::Climax][self.rng.weighted(&w)]
    }

    /// Beam search over the chord graph: `n` chords from the current one, the last one
    /// fixed to the goal chord, scored by function, voice leading, novelty against the
    /// history, the style's vocabulary and a tension arc.
    fn search(&mut self, key: Key, n: usize, goal: Option<Chord>, pen: Option<Chord>, arc: &[f32]) -> Vec<Chord> {
        let voc = self.vocab(key);
        let hist: Vec<Chord> = self.hist.iter().map(|h| h.0).collect();
        let func_of = |c: &Chord| voc.iter().find(|v| v.0 == *c).map(|v| (v.1, v.2)).unwrap_or((Func::Bor, 0.5));
        let mut beam: Vec<(f32, Vec<Chord>)> = vec![(0.0, Vec::new())];
        for pos in 0..n {
            let mut next: Vec<(f32, Vec<Chord>)> = Vec::new();
            for (sc, path) in &beam {
                let prev = path.last().copied().unwrap_or(self.cur);
                let (pf, _) = func_of(&prev);
                let mut cands: Vec<(Chord, Func, f32)> = voc.clone();
                if pos == n - 1 {
                    if let Some(g) = goal {
                        let (gf, gt) = func_of(&g);
                        cands = vec![(g, gf, gt)];
                    }
                } else if pos == n - 2 {
                    if let Some(p) = pen {
                        let (pf2, pt) = func_of(&p);
                        cands = vec![(p, pf2, pt)];
                    }
                }
                for &(c, cf, ct) in &cands {
                    if c == prev && pos + 1 < n {
                        continue;
                    }
                    let mut s = match (pf, cf) {
                        (Func::T, Func::PD) => 0.8,
                        (Func::PD, Func::D) => 1.0,
                        (Func::D, Func::T) => 1.0,
                        (Func::T, Func::D) => 0.3,
                        (Func::PD, Func::T) => 0.25,
                        (Func::D, Func::PD) => -0.6,
                        (Func::Bor, Func::T) | (Func::Bor, Func::PD) => 0.4,
                        (Func::PD, Func::PD) => 0.15,
                        _ => 0.0,
                    };
                    if let Func::SecD(t) = pf {
                        s += if c.root == t { 1.6 } else { -1.5 };
                    }
                    if pf == Func::D && (c.root == (prev.root + 5).rem_euclid(12) || c.root == (prev.root - 1).rem_euclid(12)) {
                        s += 0.5; // falls a fifth / tritone sub slides down a semitone
                    }
                    let (cost, common) = vl(&prev, &c);
                    s += 0.15 * common as f32 - 0.08 * cost as f32;
                    // novelty: recent chords, recent bigrams, A-B-A-B loops (history + this path)
                    let mut seq: Vec<Chord> = hist.iter().rev().take(16).rev().cloned().collect();
                    seq.extend(path.iter().cloned());
                    let k = seq.len();
                    let recent = seq.iter().rev().take(8).filter(|x| **x == c).count();
                    s -= 0.3 * recent as f32;
                    let mut bg = 0;
                    for i in 1..k {
                        if seq[i - 1] == prev && seq[i] == c {
                            bg += 1;
                        }
                    }
                    s -= 0.45 * bg as f32;
                    if k >= 3 && seq[k - 2] == c && seq[k - 1] == prev && seq[k - 3] == c {
                        s -= 2.5;
                    }
                    if k >= 3 && seq[k - 2] == c && seq[k - 3] == prev {
                        s -= 2.5;
                    }
                    s += 0.35 * (1.0 + self.bigram_w(key, prev, c)).ln();
                    let target = arc.get(pos).copied().unwrap_or(0.4);
                    s -= 1.2 * (ct - target).abs();
                    s += 0.5 * self.rng.f();
                    let mut np = path.clone();
                    np.push(c);
                    next.push((sc + s, np));
                }
            }
            next.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            next.truncate(16);
            if next.is_empty() {
                break;
            }
            beam = next;
        }
        beam.into_iter().next().map(|b| b.1).unwrap_or_default()
    }

    fn names(cs: &[Chord]) -> String {
        cs.iter().map(|c| c.name()).collect::<Vec<_>>().join(" → ")
    }

    /// a phrase with a destination: returns false when the plain paths should run
    fn plan_goal(&mut self, meander: bool) -> bool {
        let key = self.key;
        let tonic = self.seventh(roman(if key.minor { "i" } else { "I" }, key));
        let dom = roman("V7", key);
        let kn = key.name();
        // callback: reprise a phrase from a few phrases ago (transposed into this key)
        if !meander && self.phrase_no >= 4 && self.rng.chance(0.12) {
            let cands: Vec<usize> = (0..self.phrases.len())
                .filter(|&i| {
                    let p = &self.phrases[i];
                    !p.callback && self.phrase_no - p.no >= 3 && p.key.minor == key.minor && p.chords.len() >= 4
                })
                .collect();
            if !cands.is_empty() {
                let p = self.phrases[cands[self.rng.below(cands.len())]].clone();
                let sh = key.tonic - p.key.tonic;
                let chords: Vec<Chord> = p.chords.iter().map(|c| Chord { root: (c.root + sh).rem_euclid(12), q: c.q }).collect();
                let what = if p.no == 0 { "the opening vamp".to_string() } else { format!("the phrase from {} phrases ago", self.phrase_no - p.no) };
                self.plan_desc = format!("callback to {what}: {}", Self::names(&chords));
                self.push_phrase(chords, Some(format!("callback to {what}")), Goal::Authentic, true);
                return true;
            }
        }
        let goal = self.choose_goal(meander);
        let n = if !meander && self.rng.chance(0.25) { 8 } else { 4 };
        let arc: Vec<f32> = match (goal, n) {
            (Goal::Drift, _) => vec![0.25, 0.35, 0.3, 0.2],
            (Goal::Half, 4) => vec![0.15, 0.4, 0.55, 0.8],
            (Goal::Climax, 4) => vec![0.2, 0.4, 0.6, 0.55],
            (Goal::Deceptive, 4) => vec![0.2, 0.45, 0.8, 0.25],
            (_, 4) => vec![0.15, 0.45, 0.8, 0.05],
            (Goal::Half, _) => vec![0.1, 0.25, 0.35, 0.45, 0.35, 0.5, 0.6, 0.8],
            (Goal::Deceptive, _) => vec![0.1, 0.25, 0.4, 0.3, 0.45, 0.55, 0.8, 0.25],
            (Goal::Climax, _) => vec![0.1, 0.3, 0.4, 0.5, 0.45, 0.6, 0.7, 0.55],
            _ => vec![0.1, 0.3, 0.4, 0.25, 0.45, 0.55, 0.8, 0.05],
        };
        let jazz = self.flavour == Some(1) || self.sevenths;
        let (g, pen, what): (Option<Chord>, Option<Chord>, String) = match goal {
            Goal::Authentic => {
                let pen = if jazz && self.rng.chance(0.3) {
                    let tt = Chord { root: (tonic.root + 1).rem_euclid(12), q: Q::Dom7 };
                    let bd = Chord { root: (tonic.root + 10).rem_euclid(12), q: Q::Dom7 };
                    if self.rng.chance(0.5) { tt } else { bd }
                } else {
                    dom
                };
                let kind = if pen == dom { "cadence" } else if pen.root == (tonic.root + 1).rem_euclid(12) { "tritone-sub cadence" } else { "backdoor cadence" };
                (Some(tonic), Some(pen), format!("{kind} home to {}", tonic.name()))
            }
            Goal::Half => (Some(dom), None, format!("open phrase, half cadence on {}", dom.name())),
            Goal::Deceptive => {
                let vi = self.seventh(roman(if key.minor { "VI" } else { "vi" }, key));
                (Some(vi), Some(dom), format!("fake-out: {} → {} (deceptive)", dom.name(), vi.name()))
            }
            Goal::Plagal => {
                let iv = self.seventh(roman(if key.minor { "iv" } else { "IV" }, key));
                let bvii = roman(if key.minor { "VII" } else { "bVII" }, key);
                let p = if matches!(self.flavour, Some(2) | Some(4)) && self.rng.chance(0.5) { bvii } else { iv };
                (Some(tonic), Some(p), format!("plagal landing {} → {}", p.name(), tonic.name()))
            }
            Goal::Climax => {
                let pick = if key.minor { "VI" } else if self.rng.chance(0.5) { "IV" } else { "bVI" };
                let c = self.seventh(roman(pick, key));
                (Some(c), None, format!("climbing to a peak on {}", c.name()))
            }
            Goal::Drift => (None, None, "drifting".to_string()),
        };
        let path = self.search(key, n, g, pen, &arc);
        if path.len() < 2 {
            return false;
        }
        self.plan_desc = format!("{what} in {kn}: {}", Self::names(&path));
        self.push_phrase(path, Some(what), goal, false);
        true
    }

    fn push_phrase(&mut self, chords: Vec<Chord>, note: Option<String>, goal: Goal, callback: bool) {
        for (i, c) in chords.iter().enumerate() {
            self.queue.push_back((*c, if i == 0 { note.clone() } else { None }));
        }
        if goal != Goal::Drift {
            self.cadences.push_back(goal);
            while self.cadences.len() > 8 {
                self.cadences.pop_front();
            }
        }
        self.phrases.push_back(PastPhrase { key: self.key, chords, callback, no: self.phrase_no });
        while self.phrases.len() > 12 {
            self.phrases.pop_front();
        }
        self.phrase_no += 1;
    }

    /// tension of a chord in the current key (for the history)
    fn tension_of(&self, c: Chord) -> f32 {
        self.vocab(self.key).iter().find(|v| v.0 == c).map(|v| v.2).unwrap_or(0.55)
    }
}

/// Smooth voice leading: move each voice to the nearest chord tone, keep
/// common tones, make sure root and third are present. Pitches are semitones
/// relative to the global root (any octave).
pub fn voice_lead(prev: &[i32], pcs: &[i32], n: usize, lo: i32, hi: i32) -> Vec<i32> {
    let near = |p: i32, pc: i32| -> i32 {
        let base = p - (p - pc).rem_euclid(12);
        let up = base + 12;
        let c = if (p - base).abs() <= (up - p).abs() { base } else { up };
        if c < lo { c + 12 } else if c > hi { c - 12 } else { c }
    };
    let mut out: Vec<i32> = Vec::with_capacity(n);
    let mut src: Vec<i32> = prev.to_vec();
    if src.is_empty() {
        // initial open voicing
        let mut p = lo + (pcs[0] - lo).rem_euclid(12);
        for i in 0..n {
            src.push(p);
            p += if i % 2 == 0 { 7 } else { 5 };
        }
    }
    while src.len() < n {
        let l = *src.last().unwrap();
        src.push(l + 5);
    }
    src.truncate(n);
    for &p in &src {
        let best = pcs.iter().map(|&pc| near(p, pc)).min_by_key(|c| (c - p).abs()).unwrap();
        out.push(best);
    }
    // ensure coverage of the first min(n, pcs.len()) chord tones (root, third, fifth, colour)
    for (pi, &pc) in pcs.iter().enumerate().take(n.min(pcs.len())) {
        if out.iter().any(|&o| o.rem_euclid(12) == pc) {
            continue;
        }
        // move a voice whose pc is duplicated, nearest to the missing pc
        let mut best: Option<(usize, i32)> = None;
        for i in 0..out.len() {
            let dup = out.iter().filter(|&&o| o.rem_euclid(12) == out[i].rem_euclid(12)).count() > 1;
            let is_needed = pcs.iter().take(pi).any(|&q| q == out[i].rem_euclid(12)) && !dup;
            if !dup || is_needed {
                continue;
            }
            let c = near(out[i], pc);
            if best.map(|b| (c - out[i]).abs() < (b.1 - out[b.0]).abs()).unwrap_or(true) {
                best = Some((i, c));
            }
        }
        if let Some((i, c)) = best {
            out[i] = c;
        }
    }
    out.sort();
    out
}

// ------------------------------------------------------------ player keys

/// Church modes a player can think in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Ionian,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Aeolian,
}
impl Mode {
    pub fn steps(self) -> [i32; 7] {
        match self {
            Mode::Ionian => [0, 2, 4, 5, 7, 9, 11],
            Mode::Dorian => [0, 2, 3, 5, 7, 9, 10],
            Mode::Phrygian => [0, 1, 3, 5, 7, 8, 10],
            Mode::Lydian => [0, 2, 4, 6, 7, 9, 11],
            Mode::Mixolydian => [0, 2, 4, 5, 7, 9, 10],
            Mode::Aeolian => [0, 2, 3, 5, 7, 8, 10],
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Mode::Ionian => "major",
            Mode::Dorian => "Dorian",
            Mode::Phrygian => "Phrygian",
            Mode::Lydian => "Lydian",
            Mode::Mixolydian => "Mixolydian",
            Mode::Aeolian => "minor",
        }
    }
}

/// How a player's key relates to the band key (so it follows modulations).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rel {
    Same,
    Relative,
    Parallel,
    FifthUp,
    FifthDown,
    Brighter,
    Darker,
    FifthDorian,
    /// bitonal: a tritone away
    Tritone,
    /// bitonal: a major third up
    MajThird,
}
impl Rel {
    pub fn bitonal(self) -> bool {
        matches!(self, Rel::Tritone | Rel::MajThird)
    }
    pub fn describe(self) -> &'static str {
        match self {
            Rel::Same => "the band key",
            Rel::Relative => "the relative key",
            Rel::Parallel => "the parallel key",
            Rel::FifthUp => "a fifth up",
            Rel::FifthDown => "a fifth down",
            Rel::Brighter => "a brighter mode",
            Rel::Darker => "a darker mode",
            Rel::FifthDorian => "Dorian a fifth up",
            Rel::Tritone => "bitonal, a tritone away",
            Rel::MajThird => "bitonal, a major third up",
        }
    }
}

/// A player's own key: tonic (relative to the global root) and mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PKey {
    pub tonic: i32,
    pub mode: Mode,
}
impl PKey {
    pub fn of(band: Key, rel: Rel) -> PKey {
        let t = band.tonic;
        let bm = if band.minor { Mode::Aeolian } else { Mode::Ionian };
        let (tonic, mode) = match rel {
            Rel::Same => (t, bm),
            Rel::Relative => if band.minor { (t + 3, Mode::Ionian) } else { (t + 9, Mode::Aeolian) },
            Rel::Parallel => (t, if band.minor { Mode::Ionian } else { Mode::Aeolian }),
            Rel::FifthUp => (t + 7, bm),
            Rel::FifthDown => (t + 5, bm),
            Rel::Brighter => (t, if band.minor { Mode::Dorian } else { Mode::Lydian }),
            Rel::Darker => (t, if band.minor { Mode::Phrygian } else { Mode::Mixolydian }),
            Rel::FifthDorian => (t + 7, Mode::Dorian),
            Rel::Tritone => (t + 6, bm),
            Rel::MajThird => (t + 4, bm),
        };
        PKey { tonic: tonic.rem_euclid(12), mode }
    }
    pub fn contains(&self, pc: i32) -> bool {
        let d = (pc - self.tonic).rem_euclid(12);
        self.mode.steps().contains(&d)
    }
    /// Name spelled for this key's own signature.
    pub fn name(&self) -> String {
        // approximate the signature by the mode's parent major / minor key
        let k = Key { tonic: self.tonic, minor: matches!(self.mode, Mode::Aeolian | Mode::Dorian | Mode::Phrygian) };
        let prev = (USE_SHARPS.load(Ordering::Relaxed), NATURAL.load(Ordering::Relaxed), LEAD7.load(Ordering::Relaxed));
        spell_for(&k);
        LEAD7.store(-1, Ordering::Relaxed);
        let n = format!("{} {}", pc_name(self.tonic), self.mode.name());
        USE_SHARPS.store(prev.0, Ordering::Relaxed);
        NATURAL.store(prev.1, Ordering::Relaxed);
        LEAD7.store(prev.2, Ordering::Relaxed);
        n
    }
}
