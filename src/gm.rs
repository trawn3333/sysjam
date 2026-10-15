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

/// How well a program suits a role (0 = never). Everything gets a small
/// weight somewhere so that, over time, every instrument can appear.
pub fn role_weight(role: Role, p: u8) -> f32 {
    let p = p as usize;
    match role {
        Role::Pad => match p {
            88..=95 => 3.0,
            48..=51 => 2.5,
            52..=54 => 1.8,
            44 => 1.2,
            19 | 20 | 16 => 0.6,
            60 | 61 => 0.5,
            40..=43 => 0.6,
            21 | 23 => 0.3,
            97 | 99 | 100 | 102 => 0.8,
            _ => 0.0,
        },
        Role::Comp => match p {
            4 | 5 => 2.0,
            0..=3 => 1.2,
            6 | 7 => 0.5,
            8..=13 => 1.6,
            14 | 15 => 0.8,
            24..=27 => 1.2,
            28 | 31 => 0.6,
            45 | 46 => 1.6,
            104..=108 => 0.9,
            114 => 0.6,
            16 | 17 => 0.4,
            29 | 30 => 0.1,
            36 | 37 => 0.05,
            _ => 0.0,
        },
        Role::Lead => match p {
            73..=75 | 77 | 79 => 2.0,
            72 | 76 | 78 => 1.0,
            68..=71 => 1.2,
            64..=67 => 0.6,
            40 | 41 | 110 => 0.9,
            42 => 0.8,
            56 | 57 | 59 | 60 => 0.5,
            80..=84 | 86 | 87 => 0.6,
            22 => 0.4,
            11 | 12 => 0.7,
            85 => 1.0,
            109 | 111 => 0.25,
            58 | 61..=63 => 0.15,
            _ => 0.0,
        },
        Role::Colour => match p {
            96..=103 => 1.8,
            52..=54 => 1.2,
            8..=14 => 1.0,
            91 | 92 | 94 => 1.0,
            104..=108 => 0.9,
            112..=119 => 0.5,
            46 | 47 => 0.6,
            31 => 0.6,
            120..=123 => 0.25,
            124..=127 => 0.04,
            55 => 0.1,
            _ => 0.05, // anything, very rarely
        },
        Role::Bass => match p {
            32..=35 => 2.0,
            38 | 39 => 1.5,
            36 | 37 => 0.3,
            43 => 0.6,
            _ => 0.0,
        },
    }
}

/// Per-program loudness trim so oddballs sit back in the mix.
pub fn prog_gain(p: u8) -> f32 {
    match p {
        120..=127 => 0.45,
        55 | 29 | 30 | 84 | 87 => 0.5,
        56..=67 => 0.65,
        109 | 111 | 22 | 21 | 23 => 0.6,
        16..=19 => 0.7,
        80..=83 | 86 => 0.6,
        112..=118 => 0.75,
        _ => 0.9,
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
        let mut g = Gm { synth, chans: [Chan::default(); 16], l: vec![0.0; 64], r: vec![0.0; 64], sr: sr as f32 };
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
    fn cc(&mut self, ch: i32, cc: i32, v: i32) {
        self.synth.process_midi_message(ch, 0xB0, cc, v.clamp(0, 127));
    }
    pub fn program(&mut self, ch: usize, p: u8, gain: f32) {
        self.synth.process_midi_message(ch as i32, 0xC0, p as i32, 0);
        self.cc(ch as i32, 7, (gain * 110.0) as i32);
        self.chans[ch].program = p;
    }
    pub fn note_on(&mut self, ch: usize, key: i32, vel: i32, now: u64) {
        if !(0..128).contains(&key) {
            return;
        }
        self.synth.note_on(ch as i32, key, vel.clamp(1, 127));
        self.chans[ch].last_note = now;
        self.chans[ch].active += 1;
    }
    pub fn note_off(&mut self, ch: usize, key: i32, now: u64) {
        if !(0..128).contains(&key) {
            return;
        }
        self.synth.note_off(ch as i32, key);
        self.chans[ch].last_note = now;
        self.chans[ch].active = self.chans[ch].active.saturating_sub(1);
    }
    pub fn all_off(&mut self, ch: usize) {
        self.synth.note_off_all_channel(ch as i32, false);
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
    }
}
