//! General MIDI layer on top of rustysynth (pure Rust SF2 player).

use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

pub const GM_NAMES: [&str; 128] = [
    "Acoustic Grand Piano", "Bright Acoustic Piano", "Electric Grand Piano", "Honky-tonk Piano", "Electric Piano 1", "Electric Piano 2", "Harpsichord", "Clavinet",
    "Celesta", "Glockenspiel", "Music Box", "Vibraphone", "Marimba", "Xylophone", "Tubular Bells", "Dulcimer",
    "Drawbar Organ", "Percussive Organ", "Rock Organ", "Church Organ", "Reed Organ", "Accordion", "Harmonica", "Tango Accordion",
    "Nylon Guitar", "Steel Guitar", "Jazz Guitar", "Clean Guitar", "Muted Guitar", "Overdriven Guitar", "Distortion Guitar", "Guitar Harmonics",
    "Acoustic Bass", "Fingered Bass", "Picked Bass", "Fretless Bass", "Slap Bass 1", "Slap Bass 2", "Synth Bass 1", "Synth Bass 2",
    "Violin", "Viola", "Cello", "Contrabass", "Tremolo Strings", "Pizzicato Strings", "Orchestral Harp", "Timpani",
    "String Ensemble 1", "String Ensemble 2", "Synth Strings 1", "Synth Strings 2", "Choir Aahs", "Voice Oohs", "Synth Voice", "Orchestra Hit",
    "Trumpet", "Trombone", "Tuba", "Muted Trumpet", "French Horn", "Brass Section", "Synth Brass 1", "Synth Brass 2",
    "Soprano Sax", "Alto Sax", "Tenor Sax", "Baritone Sax", "Oboe", "English Horn", "Bassoon", "Clarinet",
    "Piccolo", "Flute", "Recorder", "Pan Flute", "Blown Bottle", "Shakuhachi", "Whistle", "Ocarina",
    "Square Lead", "Saw Lead", "Calliope Lead", "Chiff Lead", "Charang Lead", "Voice Lead", "Fifths Lead", "Bass+Lead",
    "New Age Pad", "Warm Pad", "Polysynth Pad", "Choir Pad", "Bowed Pad", "Metallic Pad", "Halo Pad", "Sweep Pad",
    "FX Rain", "FX Soundtrack", "FX Crystal", "FX Atmosphere", "FX Brightness", "FX Goblins", "FX Echoes", "FX Sci-fi",
    "Sitar", "Banjo", "Shamisen", "Koto", "Kalimba", "Bagpipe", "Fiddle", "Shanai",
    "Tinkle Bell", "Agogo", "Steel Drums", "Woodblock", "Taiko Drum", "Melodic Tom", "Synth Drum", "Reverse Cymbal",
    "Guitar Fret Noise", "Breath Noise", "Seashore", "Bird Tweet", "Telephone Ring", "Helicopter", "Applause", "Gunshot",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Pad,
    Comp,
    Lead,
    Colour,
    Bass,
}
pub const ROLES: [Role; 5] = [Role::Pad, Role::Comp, Role::Lead, Role::Colour, Role::Bass];

/// How well a program suits a role. Every one of the 128 programs has a
/// meaningful weight (>= 0.3) in at least two roles, so the whole GM set is in play.
pub fn role_weight(role: Role, p: u8) -> f32 {
    let p = p as usize;
    // per GM family (8 programs each): [Pad, Comp, Lead, Colour, Bass]
    const FAM: [[f32; 5]; 16] = [
        [0.4, 1.5, 0.5, 0.5, 0.0], // piano
        [0.3, 1.3, 0.7, 1.3, 0.0], // chromatic percussion
        [1.2, 1.0, 0.6, 0.4, 0.0], // organ
        [0.3, 1.3, 0.8, 0.6, 0.0], // guitar
        [0.0, 0.4, 0.35, 0.3, 1.5], // bass
        [1.4, 0.6, 1.0, 0.5, 0.0], // strings
        [1.6, 0.4, 0.5, 0.8, 0.0], // ensemble
        [0.8, 0.5, 1.0, 0.4, 0.0], // brass
        [0.5, 0.4, 1.3, 0.4, 0.0], // reed
        [0.4, 0.3, 1.4, 0.7, 0.0], // pipe
        [0.4, 0.6, 1.3, 0.6, 0.0], // synth lead
        [1.8, 0.3, 0.3, 0.8, 0.0], // synth pad
        [1.0, 0.3, 0.3, 1.6, 0.0], // synth effects
        [0.3, 1.2, 1.0, 0.8, 0.0], // ethnic
        [0.0, 0.7, 0.3, 1.2, 0.0], // percussive
        [0.5, 0.3, 0.3, 1.0, 0.0], // sound effects
    ];
    let r = match role {
        Role::Pad => 0,
        Role::Comp => 1,
        Role::Lead => 2,
        Role::Colour => 3,
        Role::Bass => 4,
    };
    let mut w = FAM[p / 8][r];
    // a few individual fits
    match (role, p) {
        (Role::Bass, 43 | 58 | 70 | 32..=39) => w = w.max(1.2),
        (Role::Comp, 45 | 46) => w = 1.6,
        (Role::Comp, 4 | 5) => w = 2.0,
        (Role::Colour, 47 | 55 | 119) => w = 1.0,
        (Role::Lead, 85 | 73 | 75 | 79) => w = 1.8,
        (Role::Pad, 52 | 53 | 88..=95) => w = w.max(2.0),
        _ => {}
    }
    w
}

/// Per-player signature palettes (GM program ranges); players draw most of their
/// instruments from one of these, plus occasional wildcards from anywhere.
pub const PALETTES: [(&str, &[(u8, u8)]); 8] = [
    ("ethnic plucks", &[(104, 108), (15, 15), (24, 25), (45, 46), (110, 111)]),
    ("brass, reeds & organs", &[(16, 23), (56, 71)]),
    ("synth leads & FX", &[(80, 87), (96, 103), (38, 39), (50, 51), (62, 63)]),
    ("mallets & bells", &[(8, 15), (108, 108), (112, 114), (98, 98)]),
    ("strings & voices", &[(40, 55), (85, 85), (91, 91)]),
    ("keys & guitars", &[(0, 7), (24, 31)]),
    ("winds & pipes", &[(64, 79), (109, 109)]),
    ("oddballs & sound FX", &[(115, 127), (47, 47), (55, 55), (97, 97), (100, 103)]),
];

pub fn in_palette(pal: usize, p: u8) -> bool {
    PALETTES[pal % PALETTES.len()].1.iter().any(|&(a, b)| p >= a && p <= b)
}

/// GM drum kits (bank 128): program, name
pub const KITS: [(u8, &str); 8] = [(0, "Standard"), (8, "Room"), (16, "Power"), (24, "Electronic"), (25, "TR-808"), (32, "Jazz"), (40, "Brush"), (48, "Orchestra")];

/// GM percussion colour sets the drummer can pick for its auxiliary hits.
pub const PERC_SETS: [(&str, &[i32]); 5] = [
    ("shakers", &[82, 70, 69]),
    ("congas & bongos", &[62, 63, 64, 60, 61]),
    ("agogo, claves & woodblocks", &[67, 68, 75, 76, 77]),
    ("triangle & tambourine", &[81, 80, 54, 53]),
    ("timbales & cowbell", &[65, 66, 56, 78, 79]),
];

/// Per-program loudness trim. Oddballs are only slightly trimmed now so they are audible.
pub fn prog_gain(p: u8) -> f32 {
    match p {
        120..=127 => 0.8,
        55 | 29 | 30 | 84 | 87 => 0.75,
        56..=67 => 0.8,
        109 | 111 | 22 | 21 | 23 => 0.8,
        16..=19 => 0.8,
        80..=83 | 86 => 0.75,
        112..=118 => 0.9,
        8..=15 | 104..=108 => 1.0,
        _ => 0.92,
    }
}

pub fn find_soundfont(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(PathBuf::from(p));
    }
    let mut c: Vec<PathBuf> = vec![
        "/usr/share/sounds/sf2/FluidR3_GM.sf2".into(),
        "/usr/share/soundfonts/FluidR3_GM.sf2".into(),
        "/usr/share/sounds/sf2/default-GM.sf2".into(),
        "/usr/share/soundfonts/default.sf2".into(),
        "/usr/share/sounds/sf2/TimGM6mb.sf2".into(),
        "/usr/share/sounds/sf2/GeneralUser_GS.sf2".into(),
    ];
    if let Ok(rd) = std::fs::read_dir("/usr/share/soundfonts") {
        let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("sf2")).unwrap_or(false)).collect();
        v.sort();
        c.extend(v);
    }
    if let Some(h) = std::env::var_os("HOME") {
        let d = PathBuf::from(h).join(".local/share/sysjam");
        if let Ok(rd) = std::fs::read_dir(&d) {
            let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("sf2")).unwrap_or(false)).collect();
            v.sort();
            c.extend(v);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            c.push(d.join("TimGM6mb.sf2"));
            c.push(d.join("assets/TimGM6mb.sf2"));
            // running from a source checkout: target/<profile>/sysjam or dist/sysjam
            for up in [d.join("../assets/TimGM6mb.sf2"), d.join("../../assets/TimGM6mb.sf2"), d.join("../../../assets/TimGM6mb.sf2")] {
                c.push(up);
            }
            if let Ok(rd) = std::fs::read_dir(d) {
                let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("sf2")).unwrap_or(false)).collect();
                v.sort();
                c.extend(v);
            }
        }
    }
    c.into_iter().find(|p| p.is_file())
}

pub fn load(path: &PathBuf) -> Result<Arc<SoundFont>, String> {
    let mut f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    SoundFont::new(&mut f).map(Arc::new).map_err(|e| format!("{}: {e:?}", path.display()))
}

#[derive(Clone, Copy, Default)]
struct Chan {
    program: u8,
    expr: f32,
    target: f32,
    rate: f32, // expression units per second
    sent: i32,
    last_note: u64,
    active: u32,
}

pub struct Gm {
    synth: Synthesizer,
    /// measurement only (SYSJAM_RMS): channel 10 on its own synthesizer so drums can be metered apart
    drum: Option<Synthesizer>,
    pub dl: Vec<f32>,
    pub dr: Vec<f32>,
    chans: [Chan; 16],
    pub l: Vec<f32>,
    pub r: Vec<f32>,
    sr: f32,
}

impl Gm {
    pub fn new(sf: &Arc<SoundFont>, sr: i32, polyphony: usize) -> Result<Gm, String> {
        let mut st = SynthesizerSettings::new(sr);
        st.maximum_polyphony = polyphony;
        st.enable_reverb_and_chorus = false; // we have our own reverb
        st.block_size = 64;
        let synth = Synthesizer::new(sf, &st).map_err(|e| format!("{e:?}"))?;
        let drum = if std::env::var_os("SYSJAM_RMS").is_some() { Synthesizer::new(sf, &st).ok() } else { None };
        let mut g = Gm { synth, drum, dl: vec![0.0; 64], dr: vec![0.0; 64], chans: [Chan::default(); 16], l: vec![0.0; 64], r: vec![0.0; 64], sr: sr as f32 };
        for ch in 0..16 {
            g.cc(ch, 11, 0);
            g.cc(ch, 7, 100);
            g.cc(ch, 91, 0);
            g.cc(ch, 93, 0);
            g.chans[ch as usize].sent = 0;
        }
        g.cc(9, 11, 127);
        g.chans[9].expr = 1.0;
        g.chans[9].target = 1.0;
        g.chans[9].sent = 127;
        Ok(g)
    }
    fn sy(&mut self, ch: i32) -> &mut Synthesizer {
        match self.drum.as_mut() {
            Some(d) if ch == 9 => d,
            _ => &mut self.synth,
        }
    }
    fn cc(&mut self, ch: i32, cc: i32, v: i32) {
        self.sy(ch).process_midi_message(ch, 0xB0, cc, v.clamp(0, 127));
    }
    pub fn program(&mut self, ch: usize, p: u8, gain: f32) {
        self.sy(ch as i32).process_midi_message(ch as i32, 0xC0, p as i32, 0);
        self.cc(ch as i32, 7, (gain * 110.0) as i32);
        self.chans[ch].program = p;
    }
    pub fn note_on(&mut self, ch: usize, key: i32, vel: i32, now: u64) {
        if !(0..128).contains(&key) {
            return;
        }
        self.sy(ch as i32).note_on(ch as i32, key, vel.clamp(1, 127));
        self.chans[ch].last_note = now;
        self.chans[ch].active += 1;
    }
    pub fn note_off(&mut self, ch: usize, key: i32, now: u64) {
        if !(0..128).contains(&key) {
            return;
        }
        self.sy(ch as i32).note_off(ch as i32, key);
        self.chans[ch].last_note = now;
        self.chans[ch].active = self.chans[ch].active.saturating_sub(1);
    }
    pub fn all_off(&mut self, ch: usize) {
        self.sy(ch as i32).note_off_all_channel(ch as i32, false);
        self.chans[ch].active = 0;
    }
    /// Expression fade (CC11) towards `target` (0..1) over `secs`.
    pub fn fade(&mut self, ch: usize, target: f32, secs: f32) {
        let c = &mut self.chans[ch];
        c.target = target;
        c.rate = ((target - c.expr).abs() / secs.max(0.05)).max(1e-3);
    }
    pub fn expr(&self, ch: usize) -> f32 {
        self.chans[ch].expr
    }
    /// A channel is safe for a program change: faded out, no held notes, release tails gone.
    pub fn silent(&self, ch: usize, now: u64) -> bool {
        let c = &self.chans[ch];
        c.expr <= 0.001 && c.target <= 0.0 && c.active == 0 && now.saturating_sub(c.last_note) as f32 > self.sr * 4.0
    }
    /// Advance CC ramps by `n` samples and render them.
    pub fn render(&mut self, n: usize) {
        let dt = n as f32 / self.sr;
        for ch in 0..16 {
            let c = self.chans[ch];
            if c.expr != c.target {
                let step = c.rate * dt;
                let e = if c.expr < c.target { (c.expr + step).min(c.target) } else { (c.expr - step).max(c.target) };
                self.chans[ch].expr = e;
                // perceptual curve
                let v = (e.sqrt() * 127.0).round() as i32;
                if v != c.sent {
                    self.cc(ch as i32, 11, v);
                    self.chans[ch].sent = v;
                }
            }
        }
        self.synth.render(&mut self.l[..n], &mut self.r[..n]);
        if let Some(d) = self.drum.as_mut() {
            d.render(&mut self.dl[..n], &mut self.dr[..n]);
        }
    }
}
