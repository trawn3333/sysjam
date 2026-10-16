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
    /// a hard cut in progress: samples left of the 25 ms fade, then every voice is killed
    /// and the expression goes back to this
    choke: Option<(usize, f32)>,
}

/// the guest musician's channel (its own synthesizer)
pub const GUEST: usize = 16;

pub struct Gm {
    synth: Synthesizer,
    /// measurement only (SYSJAM_RMS): channel 10 on its own synthesizer so drums can be metered apart
    drum: Option<Synthesizer>,
    pub dl: Vec<f32>,
    pub dr: Vec<f32>,
    chans: [Chan; 17],
    /// the guest musician: channel `GUEST` on its own small synthesizer, rendered only
    /// while it plays (and a release tail after)
    guest: Synthesizer,
    gl: Vec<f32>,
    gr: Vec<f32>,
    guest_tail: usize,
    pub l: Vec<f32>,
    pub r: Vec<f32>,
    sr: f32,
    /// the modeled electric guitars and basses (src/guitar.rs); `SYSJAM_GUITAR=gm` keeps the samples
    rig: Option<crate::guitar::Rig>,
    vol: [f32; 17],
    panv: [f32; 17],
}

impl Gm {
    pub fn new(sf: &Arc<SoundFont>, sr: i32, polyphony: usize) -> Result<Gm, String> {
        let mut st = SynthesizerSettings::new(sr);
        st.maximum_polyphony = polyphony;
        st.enable_reverb_and_chorus = false; // we have our own reverb
        st.block_size = 64;
        let synth = Synthesizer::new(sf, &st).map_err(|e| format!("{e:?}"))?;
        let drum = if std::env::var_os("SYSJAM_RMS").is_some() { Synthesizer::new(sf, &st).ok() } else { None };
        let mut gs = SynthesizerSettings::new(sr);
        gs.maximum_polyphony = 8;
        gs.enable_reverb_and_chorus = false;
        gs.block_size = 64;
        let guest = Synthesizer::new(sf, &gs).map_err(|e| format!("{e:?}"))?;
        let mut g = Gm { synth, drum, dl: vec![0.0; 64], dr: vec![0.0; 64], chans: [Chan::default(); 17], guest, gl: vec![0.0; 64], gr: vec![0.0; 64], guest_tail: 0, l: vec![0.0; 64], r: vec![0.0; 64], sr: sr as f32, rig: None, vol: [0.85; 17], panv: [0.5; 17] };
        if std::env::var("SYSJAM_GUITAR").map_or(true, |v| v != "gm") {
            g.rig = Some(crate::guitar::Rig::new(sr as f32));
        }
        for ch in 0..17 {
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
    /// the synthesizer and its MIDI channel for one of our channels
    fn sy(&mut self, ch: i32) -> (&mut Synthesizer, i32) {
        if ch == GUEST as i32 {
            return (&mut self.guest, 0);
        }
        match self.drum.as_mut() {
            Some(d) if ch == 9 => (d, 9),
            _ => (&mut self.synth, ch),
        }
    }
    fn cc(&mut self, ch: i32, cc: i32, v: i32) {
        let (s, c) = self.sy(ch);
        s.process_midi_message(c, 0xB0, cc, v.clamp(0, 127));
    }
    /// this channel's notes go to the guitar model: Some(bass?)
    fn model(&self, ch: usize) -> Option<bool> {
        if ch == 9 || self.rig.is_none() {
            return None;
        }
        crate::guitar::modeled(self.chans[ch].program)
    }
    /// the style sets the amp preset
    pub fn set_style(&mut self, id: &str) {
        if let Some(r) = self.rig.as_mut() {
            r.set_style(id);
        }
    }
    pub fn program(&mut self, ch: usize, p: u8, gain: f32) {
        if self.model(ch).is_some() {
            if let Some(r) = self.rig.as_mut() {
                r.all_off(ch);
            }
        }
        let v = (gain * 110.0).clamp(0.0, 127.0) / 127.0;
        self.vol[ch] = v * v;
        let (s, c) = self.sy(ch as i32);
        s.process_midi_message(c, 0xC0, p as i32, 0);
        self.cc(ch as i32, 7, (gain * 110.0) as i32);
        self.chans[ch].program = p;
    }
    /// pan (CC10), 0 left .. 127 right
    pub fn pan(&mut self, ch: usize, v: i32) {
        self.panv[ch] = v.clamp(0, 127) as f32 / 127.0;
        self.cc(ch as i32, 10, v);
    }
    pub fn note_on(&mut self, ch: usize, key: i32, vel: i32, now: u64) {
        if !(0..128).contains(&key) || self.chans[ch].choke.is_some() {
            // nothing starts during a hard cut
            return;
        }
        sound_log(now as f64 / self.sr as f64, "gm", ch, self.chans[ch].program as i32, key, vel);
        if let Some(bass) = self.model(ch) {
            self.rig.as_mut().unwrap().note_on(ch, key, vel.clamp(1, 127), bass);
            self.chans[ch].last_note = now;
            self.chans[ch].active += 1;
            return;
        }
        let (s, c) = self.sy(ch as i32);
        s.note_on(c, key, vel.clamp(1, 127));
        if ch == GUEST {
            self.guest_tail = (self.sr * 5.0) as usize;
        }
        self.chans[ch].last_note = now;
        self.chans[ch].active += 1;
    }
    /// pitch bend, 0..16383 (8192 = centre; default range ±2 semitones)
    pub fn bend(&mut self, ch: usize, v: i32) {
        let v = v.clamp(0, 16383);
        if let Some(r) = self.rig.as_mut() {
            r.bend(ch, v);
        }
        let (s, c) = self.sy(ch as i32);
        s.process_midi_message(c, 0xE0, v & 0x7F, v >> 7);
    }
    pub fn note_off(&mut self, ch: usize, key: i32, now: u64) {
        if !(0..128).contains(&key) {
            return;
        }
        if self.model(ch).is_some() {
            self.rig.as_mut().unwrap().note_off(ch, key);
        }
        let (s, c) = self.sy(ch as i32);
        s.note_off(c, key);
        self.chans[ch].last_note = now;
        self.chans[ch].active = self.chans[ch].active.saturating_sub(1);
    }
    /// a hard cut (user style switch): every channel fades to silence in ~25 ms, then every
    /// voice is killed so nothing rings into the next style
    pub fn choke_all(&mut self) {
        let n = (self.sr * 0.025) as usize;
        for ch in 0..17 {
            let c = &mut self.chans[ch];
            let back = c.target;
            c.target = 0.0;
            c.rate = (c.expr / 0.025).max(1e-3);
            c.choke = Some((n, back));
        }
    }
    pub fn all_off(&mut self, ch: usize) {
        if let Some(r) = self.rig.as_mut() {
            r.all_off(ch);
        }
        let (s, c) = self.sy(ch as i32);
        s.note_off_all_channel(c, false);
        self.chans[ch].active = 0;
    }
    /// Expression fade (CC11) towards `target` (0..1) over `secs`.
    pub fn fade(&mut self, ch: usize, target: f32, secs: f32) {
        let c = &mut self.chans[ch];
        if let Some((left, _)) = c.choke {
            // mid-cut: this is where the channel goes once the cut is done
            c.choke = Some((left, target));
            return;
        }
        c.target = target;
        c.rate = ((target - c.expr).abs() / secs.max(0.05)).max(1e-3);
    }
    /// set the expression at once (a channel about to be reused after a hard reset)
    pub fn snap(&mut self, ch: usize, v: f32) {
        let c = &mut self.chans[ch];
        if let Some((left, _)) = c.choke {
            c.choke = Some((left, v));
            return;
        }
        c.expr = v;
        c.target = v;
        let s = (v.sqrt() * 127.0).round() as i32;
        c.sent = s;
        self.cc(ch as i32, 11, s);
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
        for ch in 0..17 {
            if let Some((left, back)) = self.chans[ch].choke {
                if left <= n {
                    let (s, c) = self.sy(ch as i32);
                    s.note_off_all_channel(c, true);
                    if let Some(r) = self.rig.as_mut() {
                        r.cut_ch(ch);
                    }
                    let cc = &mut self.chans[ch];
                    cc.choke = None;
                    cc.active = 0;
                    cc.expr = back;
                    cc.target = back;
                    let v = (back.sqrt() * 127.0).round() as i32;
                    cc.sent = v;
                    self.cc(ch as i32, 11, v);
                    continue;
                }
                self.chans[ch].choke = Some((left - n, back));
            }
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
        let t0 = pt();
        self.synth.render(&mut self.l[..n], &mut self.r[..n]);
        eprof(2, t0);
        let t1 = pt();
        if let Some(r) = self.rig.as_mut() {
            let mut lv = [0.0f32; 17];
            for (k, c) in self.chans.iter().enumerate() {
                lv[k] = self.vol[k] * c.expr * RIG_LEVEL;
            }
            r.render(n, &lv, &self.panv);
            for i in 0..n {
                self.l[i] += r.l[i];
                self.r[i] += r.r[i];
            }
        }
        eprof(3, t1);
        if let Some(d) = self.drum.as_mut() {
            d.render(&mut self.dl[..n], &mut self.dr[..n]);
        }
        if self.guest_tail > 0 {
            self.guest_tail = self.guest_tail.saturating_sub(n);
            if self.chans[GUEST].active > 0 {
                self.guest_tail = self.guest_tail.max(n);
            }
            self.guest.render(&mut self.gl[..n], &mut self.gr[..n]);
            for i in 0..n {
                self.l[i] += self.gl[i];
                self.r[i] += self.gr[i];
            }
        }
    }
}

/// engine profile (ns) for the bench, only with SYSJAM_EPROF=1: 0 total render, 1 steps,
/// 2 GM synth, 3 guitar rig, 4 voice pool, 5 rage, 6 mix loop (pad + bed + fx + limiter)
pub static EPROF: [std::sync::atomic::AtomicU64; 7] = [const { std::sync::atomic::AtomicU64::new(0) }; 7];
pub static EPROF_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// a start time when profiling, else nothing (no clock reads on the audio thread)
#[inline]
pub fn pt() -> Option<std::time::Instant> {
    if EPROF_ON.load(std::sync::atomic::Ordering::Relaxed) { Some(std::time::Instant::now()) } else { None }
}
#[inline]
pub fn eprof(k: usize, t: Option<std::time::Instant>) {
    if let Some(t) = t {
        EPROF[k].fetch_add(t.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
    }
}
pub fn eprof_line(secs: f64) -> String {
    let n = ["total", "steps", "gm", "guitar", "pool", "rage", "mix"];
    (0..7).map(|k| format!("{} {:.2}%", n[k], EPROF[k].load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e9 / secs * 100.0)).collect::<Vec<_>>().join(", ")
}

/// the modeled guitars' level against the GM samples (matched by ear and RMS)
const RIG_LEVEL: f32 = 0.4;

/// measurement: every note that actually starts sounding (SYSJAM_SOUNDLOG=path):
/// time, source (gm / pool), channel or owner, program, key, velocity
pub fn sound_log(t: f64, src: &str, ch: usize, prog: i32, key: i32, vel: i32) {
    use std::io::Write;
    static LOG: std::sync::OnceLock<Option<std::sync::Mutex<std::io::BufWriter<std::fs::File>>>> = std::sync::OnceLock::new();
    let l = LOG.get_or_init(|| std::env::var_os("SYSJAM_SOUNDLOG").and_then(|p| std::fs::File::create(p).ok()).map(|f| std::sync::Mutex::new(std::io::BufWriter::new(f))));
    if let Some(m) = l {
        if let Ok(mut w) = m.lock() {
            let _ = writeln!(w, "{t:.4} {src} {ch} {prog} {key} {vel}");
        }
    }
}
