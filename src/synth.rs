//! The engine: a small jam band of machine musicians playing over a
//! full-spectrum oscillator bed (sub drone, bass, detuned pad, air) plus an
//! optional General MIDI (SF2) layer.

use crate::band::{Entry, Leader, Motif, Persona, State, P};
use crate::dsp::*;
use crate::gm::{prog_gain, role_weight, Gm, Role, GM_NAMES, ROLES};
use crate::harmony::{voice_lead, Chord, Harmony, Key};
use crate::music::semi_to_freq;
use crate::shared::Control;
use rustysynth::SoundFont;
use std::collections::VecDeque;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use crate::ui::Mind;
use crate::harmony::{pc_name, roman_of};

const CR: usize = 32; // control-rate block
const CHUNK: usize = 64; // GM/event chunk
const PAD_V: usize = 4;
const MAX_T: usize = 6;
const SPREAD: [f32; PAD_V] = [0.0, -1.0, 1.0, -0.5];

pub struct EngineCfg {
    pub sr: f32,
    pub root: f32,
    pub minor: bool,
    pub bpm: f32,
    pub master: f32,
    pub clicks: f32,
    pub chord_bars: u32,
    pub seed: u64,
    pub soundfont: Option<Arc<SoundFont>>,
    pub gm_layers: usize,
    pub sub: f32,
    pub tx: Option<Sender<String>>,
    pub ui: Option<Arc<Mutex<crate::ui::BandSnap>>>,
}

#[derive(Default, Clone, Copy)]
struct PadVoice {
    ph: f32,
    inc: f32,
    gain: f32,
    drift: f32,
    gl: f32,
    gr: f32,
}
#[derive(Default, Clone, Copy)]
struct PadTone {
    semi: f32,
    target: f32,
    gain: f32,
    on: bool,
    voices: [PadVoice; PAD_V],
}
#[derive(Default, Clone, Copy)]
struct Bell {
    on: bool,
    ph1: f32,
    ph2: f32,
    inc1: f32,
    inc2: f32,
    amp: f32,
    att: f32,
    dec: f32,
    gl: f32,
    gr: f32,
}
#[derive(Default, Clone, Copy)]
struct Click {
    on: bool,
    ph1: f32,
    ph2: f32,
    inc1: f32,
    inc2: f32,
    amp: f32,
    dec: f32,
    noise: f32,
    gl: f32,
    gr: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum BassMode {
    Sustain,
    Pulse,
    Groove,
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum Comp {
    Sparse,
    Arp,
    ArpUD,
    Broken,
    Offbeat,
}

struct Slot {
    role: Role,
    chans: [usize; 2],
    cur: usize,
    program: Option<u8>,
    on: bool,
    want: bool,
    next_rotate: u64,
    recent: VecDeque<u8>,
}

#[derive(Clone, Copy)]
struct Ev {
    at: u64,
    ch: usize,
    key: i32,
    vel: i32, // 0 = note off
}

pub struct Engine {
    sr: f32,
    tbl: Vec<f32>,
    rng: Rng,
    root: f32,
    master: f32,
    sub_level: f32,
    last_ctl: u64,
    midi_root: i32,
    click_level: f32,
    chord_bars: u32,
    tx: Option<Sender<String>>,
    pub ctl: Control,
    // clock
    step_len: f32,
    step_pos: f32,
    step: u64,
    now: u64,
    // musicians' brains
    harm: Harmony,
    leader: Leader,
    cur_bpc: u32,
    bars_since_change: u32,
    history: VecDeque<String>,
    persona: [Persona; 6],
    on: [bool; 6],
    bass_mode: BassMode,
    bass_pat: [i32; 16], // 0 none, 1 root, 2 fifth, 3 octave, 4 approach
    drum_pat: [[i32; 16]; 4], // kick, snare, hat, aux
    fill: bool,
    crash_next: bool,
    comp: Comp,
    motif: Motif,
    lead_calls: u32,
    reply: Option<(Motif, u64)>,
    entry_wait: u32,
    // bed
    tones: [PadTone; MAX_T],
    pad_f: [Svf; 2],
    padl: [f32; CR],
    padr: [f32; CR],
    cr_count: usize,
    cutoff: f32,
    detune: f32,
    saw_mix: f32,
    voices_f: f32,
    pad_norm: f32,
    lfo_ph: f32,
    drift: [f32; 3],
    sub_ph: [f32; 2],
    sub_semi: f32,
    sub_target: f32,
    bass_ph: f32,
    bass_inc: f32,
    bass_env: f32,
    bass_peak: f32,
    bass_att: f32,
    bass_dec: f32,
    bass_attack: bool,
    bass_f: Svf,
    kick_on: bool,
    kick_t: f32,
    kick_ph: f32,
    kick_amp: f32,
    hat_env: f32,
    hat_amp: f32,
    hat_hp: f32,
    hat_prev: f32,
    bells: [Bell; 12],
    clicks: [Click; 4],
    click_gap: f32,
    air_f: Svf,
    air: f32,
    air_target: f32,
    ind_gain: f32,
    ind_target: f32,
    ind_steps: u32,
    ind_cd: f32,
    ind_env: f32,
    ind_pat: u16,
    ind_f: Svf,
    ind_f2: Svf,
    ind_freq: f32,
    riser: f32,
    riser_t: f32,
    riser_f: Svf,
    guest_on: bool,
    guest_gain: f32,
    guest_ph: [f32; 3],
    guest_semi: f32,
    guest_hash: u64,
    guest_cpu: f32,
    guest_vib: f32,
    guest_f: Svf,
    rev: Reverb,
    dl: Vec<f32>,
    dr: Vec<f32>,
    di: usize,
    lim: Limiter,
    fade: f32,
    fade_target: f32,
    fade_step: f32,
    // GM
    gm: Option<Gm>,
    slots: Vec<Slot>,
    gm_layers: usize,
    evs: Vec<Ev>,
    pad_held: Vec<(usize, i32)>,
    recent_global: VecDeque<u8>,
    msg_budget: f32,
    // live UI (only touched when `ui` is Some)
    ui: Option<Arc<Mutex<crate::ui::BandSnap>>>,
    seed: u64,
    bpm: f32,
    minds: [Mind; 7],
    heard_notes: [VecDeque<(u64, String)>; 7],
    hist_ch: VecDeque<Chord>,
    lv: [f32; 4],
    lv_n: u32,
    lp: [f32; 3],
    last_pub: u64,
    pub_ver: u64,
}

fn pan(p: f32) -> (f32, f32) {
    let a = (p.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (a.cos(), a.sin())
}

impl Engine {
    pub fn new(c: EngineCfg) -> Engine {
        let mut rng = Rng::seeded(c.seed);
        let tbl = sine_table();
        let sr = c.sr;
        let mut tones = [PadTone::default(); MAX_T];
        for (ti, t) in tones.iter_mut().enumerate() {
            for (vi, v) in t.voices.iter_mut().enumerate() {
                v.ph = rng.f();
                v.drift = rng.f();
                let (l, r) = pan(((ti * 3 + vi) % 5) as f32 / 2.0 - 1.0);
                v.gl = l * 0.9;
                v.gr = r * 0.9;
            }
        }
        let step_len = sr * 60.0 / c.bpm / 4.0;
        let dlen = (step_len * 3.0) as usize;
        let persona = [(); 6].map(|_| Persona::roll(&mut rng));
        let gm = c.soundfont.as_ref().and_then(|sf| Gm::new(sf, sr as i32, 28).ok());
        let slots = ROLES
            .iter()
            .enumerate()
            .map(|(i, &role)| Slot {
                role,
                chans: [[0, 1], [2, 3], [4, 5], [6, 7], [8, 10]][i],
                cur: 0,
                program: None,
                on: false,
                want: false,
                next_rotate: 0,
                recent: VecDeque::new(),
            })
            .collect();
        crate::harmony::set_root_pc((69.0 + 12.0 * (c.root / 440.0).log2()).round() as i32);
        let harm = Harmony::new(c.seed, c.minor);
        let mut e = Engine {
            sr,
            tbl,
            root: c.root,
            master: c.master,
            sub_level: c.sub,
            last_ctl: 0,
            midi_root: (69.0 + 12.0 * (c.root / 440.0).log2()).round() as i32,
            click_level: c.clicks,
            chord_bars: c.chord_bars.max(1),
            tx: c.tx,
            ctl: Control::default(),
            step_len,
            step_pos: 0.0,
            step: 0,
            now: 0,
            leader: Leader::new(c.seed),
            cur_bpc: c.chord_bars.max(1) * 2,
            bars_since_change: 0,
            history: VecDeque::new(),
            persona,
            on: [true, true, false, true, false, true],
            bass_mode: BassMode::Sustain,
            bass_pat: [0; 16],
            drum_pat: [[0; 16]; 4],
            fill: false,
            crash_next: false,
            comp: Comp::Sparse,
            motif: Motif::random(&mut rng, 0.5, 32),
            lead_calls: 0,
            reply: None,
            entry_wait: 0,
            tones,
            pad_f: [Svf::default(); 2],
            padl: [0.0; CR],
            padr: [0.0; CR],
            cr_count: 0,
            cutoff: 900.0,
            detune: 5.0,
            saw_mix: 0.3,
            voices_f: 3.0,
            pad_norm: 0.2,
            lfo_ph: 0.0,
            drift: [rng.f(), rng.f(), rng.f()],
            sub_ph: [0.0, 0.5],
            sub_semi: -12.0,
            sub_target: -12.0,
            bass_ph: 0.0,
            bass_inc: 55.0 / sr,
            bass_env: 0.0,
            bass_peak: 0.0,
            bass_att: 0.001,
            bass_dec: 0.9999,
            bass_attack: false,
            bass_f: Svf::default(),
            kick_on: false,
            kick_t: 0.0,
            kick_ph: 0.0,
            kick_amp: 0.0,
            hat_env: 0.0,
            hat_amp: 0.0,
            hat_hp: 0.0,
            hat_prev: 0.0,
            bells: [Bell::default(); 12],
            clicks: [Click::default(); 4],
            click_gap: 0.0,
            air_f: Svf::default(),
            air: 0.0,
            air_target: 0.0,
            ind_gain: 0.0,
            ind_target: 0.0,
            ind_steps: 0,
            ind_cd: 20.0,
            ind_env: 0.0,
            ind_pat: 0,
            ind_f: Svf::default(),
            ind_f2: Svf::default(),
            ind_freq: 3000.0,
            riser: 0.0,
            riser_t: 1.0,
            riser_f: Svf::default(),
            guest_on: false,
            guest_gain: 0.0,
            guest_ph: [0.0, 0.33, 0.66],
            guest_semi: 24.0,
            guest_hash: 0,
            guest_cpu: 0.0,
            guest_vib: 0.0,
            guest_f: Svf::default(),
            rev: Reverb::new(sr),
            dl: vec![0.0; dlen.max(1)],
            dr: vec![0.0; dlen.max(1)],
            di: 0,
            lim: Limiter::new(sr),
            fade: 0.0,
            fade_target: 1.0,
            fade_step: 1.0 / (sr * 3.0),
            gm,
            slots,
            gm_layers: c.gm_layers.clamp(1, 4),
            evs: Vec::with_capacity(256),
            pad_held: Vec::new(),
            recent_global: VecDeque::new(),
            msg_budget: 3.0,
            ui: c.ui.clone(),
            seed: c.seed,
            bpm: c.bpm,
            minds: {
                let names = ["drone", "bassist", "drummer", "keys", "lead", "colour", "guest"];
                std::array::from_fn(|i| Mind { name: names[i], status: if i == 0 { "following" } else { "laying out" }, think: "listening".into(), ..Default::default() })
            },
            heard_notes: Default::default(),
            hist_ch: VecDeque::new(),
            lv: [0.0; 4],
            lv_n: 0,
            lp: [0.0; 3],
            last_pub: 0,
            pub_ver: 0,
            harm,
            rng,
        };
        let ch = e.harm.cur;
        e.on_chord(ch, None, true);
        e
    }

    /// seconds of audio rendered
    pub fn time(&self) -> f32 {
        self.now as f32 / self.sr
    }

    pub fn has_gm(&self) -> bool {
        self.gm.is_some()
    }

    fn think(&mut self, i: usize, status: &'static str, t: impl Into<String>) {
        if self.ui.is_some() {
            self.minds[i].status = status;
            self.minds[i].think = t.into();
        }
    }
    fn set_think(&mut self, i: usize, t: impl Into<String>) {
        if self.ui.is_some() {
            self.minds[i].think = t.into();
        }
    }
    fn note_name(&self, semi: i32) -> String {
        let midi = self.midi_root + semi;
        format!("{}{}", pc_name(semi.rem_euclid(12)), midi.div_euclid(12) - 1)
    }
    /// a player sounded something (UI only)
    fn heard(&mut self, i: usize, label: String, vel: f32) {
        if self.ui.is_none() {
            return;
        }
        let q = &mut self.heard_notes[i];
        q.push_back((self.now, label));
        while q.len() > 6 {
            q.pop_front();
        }
        self.minds[i].activity = (self.minds[i].activity + 0.15 + 0.35 * vel.clamp(0.0, 1.0)).min(1.0);
    }
    fn heard_semi(&mut self, i: usize, semi: i32, vel: f32) {
        if self.ui.is_some() {
            let n = self.note_name(semi);
            self.heard(i, n, vel);
        }
    }

    /// Publish a snapshot for the TUI (never blocks the audio thread).
    fn publish(&mut self) {
        let Some(ui) = self.ui.clone() else { return };
        let Ok(mut snap) = ui.try_lock() else { return };
        let key = self.key();
        let mut prog = Vec::new();
        let hn = self.hist_ch.len();
        for (i, c) in self.hist_ch.iter().enumerate() {
            let when = if i + 1 == hn { 0 } else { -1 };
            if hn - i <= 3 {
                prog.push((c.name(), roman_of(*c, key), when));
            }
        }
        for c in self.harm.upcoming(4) {
            prog.push((c.name(), roman_of(c, key), 1));
        }
        let lv = |a: f32| if a > 0.0 { 10.0 * a.log10() } else { -99.0 };
        let n = self.lv_n.max(1) as f32;
        let levels = [lv(self.lv[0] / n), lv(self.lv[1] / n), lv(self.lv[2] / n), lv(self.lv[3] / n)];
        self.lv = [0.0; 4];
        self.lv_n = 0;
        // instruments + recent notes
        let gm_name = |r: Role, sl: &[Slot]| -> Option<String> {
            sl.iter().find(|s| s.role == r && s.on).and_then(|s| s.program).map(|p| format!("{} ({p})", GM_NAMES[p as usize]))
        };
        let has_gm = self.gm.is_some();
        let inst = [
            "sub drone + detuned pad".to_string(),
            match gm_name(Role::Bass, &self.slots) { Some(g) => format!("sine/saw bass + {g}"), None => "sine/saw bass".into() },
            if has_gm { "synth kick + GM kit".into() } else { "synth kick + hats".into() },
            match (gm_name(Role::Pad, &self.slots), gm_name(Role::Comp, &self.slots)) {
                (Some(a), Some(b)) => format!("{a} / {b}"),
                (Some(a), None) | (None, Some(a)) => a,
                _ => "—".into(),
            },
            gm_name(Role::Lead, &self.slots).unwrap_or_else(|| "bells".into()),
            match gm_name(Role::Colour, &self.slots) { Some(g) => format!("{g} + bells"), None => "bells / shimmer".into() },
            if self.guest_on { "saw/sine guest voice".into() } else { "—".into() },
        ];
        let recent = (self.sr * 5.0) as u64;
        for i in 0..7 {
            self.minds[i].instrument = inst[i].clone();
            let now = self.now;
            self.minds[i].notes = self.heard_notes[i].iter().filter(|(t, _)| now.saturating_sub(*t) < recent).map(|x| x.1.clone()).collect();
        }
        self.pub_ver += 1;
        *snap = crate::ui::BandSnap {
            key: key.name(),
            prog,
            bpm: self.bpm,
            bar: self.step / 16,
            beat: ((self.step % 16) / 4) as u32,
            state: self.leader.state.name(),
            energy: self.leader.energy,
            agitation: self.leader.agitation,
            minds: self.minds.to_vec(),
            levels,
            seed: self.seed,
            time: self.time(),
            version: self.pub_ver,
        };
    }

    fn say(&mut self, m: String) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(m);
        }
    }
    /// rate-limited band chatter (chord/key lines bypass this)
    fn band_say(&mut self, m: String) {
        if self.msg_budget >= 1.0 {
            self.msg_budget -= 1.0;
            self.say(m);
        }
    }

    pub fn set_control(&mut self, c: &Control) {
        if c.version == self.ctl.version {
            return;
        }
        let first = self.ctl.version == 0;
        self.ctl = c.clone();
        let mut msgs = Vec::new();
        let dt = if first { 0.0 } else { ((self.now - self.last_ctl) as f32 / self.sr).clamp(0.05, 3.0) };
        self.last_ctl = self.now;
        let req = self.leader.listen(&c.sig, dt, &mut msgs);
        self.harm.warmth = c.warmth;
        if req.tension {
            self.harm.request_tension();
        }
        if req.modulate {
            self.harm.request_modulation();
        }
        if req.resolve {
            self.harm.request_resolve();
        }
        for m in msgs {
            self.say(m);
        }
        // guest musician: a sustained heavy thread from the monitor
        match &c.leads[0] {
            Some(l) => {
                if !self.guest_on || self.guest_hash != l.hash {
                    self.guest_hash = l.hash;
                    self.guest_semi = self.guest_pitch();
                }
                self.guest_on = true;
                if self.ui.is_some() {
                    let semi = self.guest_semi.round() as i32;
                    let n = self.note_name(semi);
                    self.think(6, "soloing", format!("thread {} at {:.0}% cpu sits in → long {n}", l.name, l.cpu * 100.0));
                    self.heard(6, n, 0.6);
                }
                self.guest_cpu = l.cpu;
            }
            None => {
                if self.guest_on {
                    self.think(6, "laying out", "my thread calmed down → bowing out");
                }
                self.guest_on = false
            }
        }
    }

    pub fn fade_out(&mut self, secs: f32) {
        self.fade_target = 0.0;
        self.fade_step = 1.0 / (self.sr * secs);
    }
    pub fn faded_out(&self) -> bool {
        self.fade_target == 0.0 && self.fade <= 0.0
    }

    // ---------------------------------------------------------------- helpers

    fn key(&self) -> Key {
        self.harm.key
    }
    fn chord(&self) -> Chord {
        self.harm.cur
    }
    /// fold a semitone (relative to A2) into [lo, hi]
    fn fold(mut s: i32, lo: i32, hi: i32) -> i32 {
        while s < lo {
            s += 12;
        }
        while s > hi {
            s -= 12;
        }
        s
    }
    /// pitch of motif degree `deg` relative to the current chord root, in a register
    fn motif_pitch(&self, deg: i32, strong: bool, center: i32) -> i32 {
        let key = self.key();
        let ch = self.chord();
        let rd = key.degree_of(ch.root);
        let mut s = key.deg_semi(rd + deg);
        if strong {
            // snap to the nearest chord tone on strong beats
            let pcs = ch.pcs();
            let mut best = s;
            let mut bd = 99;
            for d in -2..=2 {
                if pcs.contains(&(s + d).rem_euclid(12)) && (d as i32).abs() < bd {
                    bd = (d as i32).abs();
                    best = s + d;
                }
            }
            s = best;
        }
        // keep the line near the register centre, by octaves
        while s - center > 7 {
            s -= 12;
        }
        while center - s > 7 {
            s += 12;
        }
        s
    }
    fn guest_pitch(&self) -> f32 {
        let pcs = self.chord().pcs();
        let pc = pcs[(self.guest_hash % pcs.len() as u64) as usize];
        Self::fold(pc + 24 + ((self.guest_hash >> 8) % 2) as i32 * 7, 19, 33) as f32
    }

    fn slot(&self, r: Role) -> &Slot {
        self.slots.iter().find(|s| s.role == r).unwrap()
    }
    fn slot_ch(&self, r: Role) -> Option<usize> {
        if self.gm.is_none() {
            return None;
        }
        let s = self.slot(r);
        if s.on && s.program.is_some() { Some(s.chans[s.cur]) } else { None }
    }
    /// Schedule a GM note (semitones relative to A2), humanised.
    fn note(&mut self, ch: usize, semi: i32, vel: i32, steps: f32, delay_steps: f32) {
        let key = self.midi_root + semi;
        let vel = (vel + self.rng.below(11) as i32 - 5).clamp(1, 127);
        let jitter = (self.rng.f() * 0.012 * self.sr) as u64;
        let at = self.now + (delay_steps.max(0.0) * self.step_len) as u64 + jitter;
        let off = at + (steps.max(0.25) * self.step_len) as u64;
        self.evs.push(Ev { at, ch, key, vel });
        self.evs.push(Ev { at: off, ch, key, vel: 0 });
    }
    fn role_note(&mut self, r: Role, semi: i32, vel: i32, steps: f32, delay: f32) {
        if let Some(ch) = self.slot_ch(r) {
            self.note(ch, semi, vel, steps, delay);
            let who = match r {
                Role::Bass => 1,
                Role::Pad | Role::Comp => 3,
                Role::Lead => 4,
                Role::Colour => 5,
            };
            self.heard_semi(who, semi, vel as f32 / 100.0);
        }
    }
    fn drum(&mut self, key: i32, vel: i32, delay: f32) {
        if self.ui.is_some() {
            let n = match key {
                35 | 36 => "kick",
                37 => "rim",
                38 | 40 => "snare",
                42 | 44 => "hat",
                46 => "open hat",
                49 | 57 => "crash",
                51 | 59 => "ride",
                45 | 47 | 48 | 50 => "tom",
                70 => "maracas",
                82 => "shaker",
                _ => "perc",
            };
            self.heard(2, n.into(), vel as f32 / 90.0);
        }
        if self.gm.is_some() {
            self.note(9, key - self.midi_root, vel, 1.0, delay);
        }
    }

    // ------------------------------------------------------------- the clock

    fn on_step(&mut self) {
        let s = (self.step % 16) as usize;
        let bar = self.step / 16;
        self.step += 1;
        if s == 0 {
            self.leader.on_bar();
            self.msg_budget = (self.msg_budget + 0.7).min(3.0);
            if bar % 4 == 0 {
                self.on_phrase();
            }
            self.on_bar_line(bar);
        }
        self.bass_step(s);
        self.drums_step(s, bar);
        self.keys_step(s, bar);
        self.lead_step(s, bar);
        self.colour_step(s, bar);
        self.industrial_step(s);
    }

    /// Colour player's industrial texture: only when the band hears real chaos
    /// (or a deliberate colour move). Short, gated, rhythmic; never a blanket.
    fn industrial_step(&mut self, s: usize) {
        if self.ind_steps > 0 {
            self.ind_steps -= 1;
            let calm = self.leader.agitation < 0.45 && !self.leader.chaos;
            if self.ind_steps == 0 || calm {
                self.ind_steps = 0;
                self.ind_target = 0.0;
                self.ind_cd = 25.0;
                self.band_say("colour player lets the industrial noise go".into());
                self.set_think(5, "chaos is easing → letting the industrial noise go");
            } else if (self.ind_pat >> s) & 1 == 1 {
                self.ind_env = 0.6 + 0.4 * self.rng.f();
                self.heard(5, "noise".into(), 0.7);
            }
            return;
        }
        if s != 0 || self.ind_cd > 0.0 {
            return;
        }
        let deliberate = std::mem::take(&mut self.leader.industrial_move) && self.leader.state == State::Alive && self.leader.agitation > 0.4;
        if self.leader.chaos || deliberate {
            let bars = 1 + self.rng.below(3) as u32;
            self.ind_steps = 16 * bars;
            // 4-7 hits per bar, always one on the downbeat
            let mut pat: u16 = 1;
            for _ in 0..(3 + self.rng.below(4)) {
                pat |= 1 << self.rng.below(16);
            }
            self.ind_pat = pat;
            self.ind_freq = self.rng.range(1600.0, 4200.0);
            self.ind_target = if deliberate && !self.leader.chaos { 0.6 } else { 1.0 };
            self.band_say(format!("colour player brings in industrial noise ({bars} bar{})", if bars == 1 { "" } else { "s" }));
            let why = if self.leader.chaos { "the machine sounds chaotic" } else { "a deliberate colour move" };
            self.think(5, "soloing", format!("{why} → gated industrial noise for {bars} bar{}", if bars == 1 { "" } else { "s" }));
        }
    }

    fn meander(&self) -> bool {
        matches!(self.leader.state, State::Chill)
    }

    fn on_phrase(&mut self) {
        let before = self.leader.state;
        let mut msgs = Vec::new();
        let req = self.leader.on_phrase(&mut msgs);
        if req.modulate {
            self.harm.request_modulation();
        }
        if req.resolve {
            self.harm.request_resolve();
        }
        for m in msgs {
            self.say(m);
        }
        let changed = before != self.leader.state;
        if changed {
            match self.leader.entry {
                Some(Entry::Riser) if matches!(self.leader.state, State::Awaken | State::Alive) => {
                    self.riser_t = 0.0;
                    self.entry_wait = 1;
                }
                Some(Entry::Fill) if matches!(self.leader.state, State::Awaken | State::Alive) => {
                    self.fill = true;
                    self.entry_wait = 1;
                }
                _ => self.entry_wait = 0,
            }
        }
        // every musician listens and decides
        for (i, p) in [P::Drone, P::Bass, P::Drums, P::Keys, P::Lead, P::Colour].into_iter().enumerate() {
            let inv = self.leader.invite(p);
            let was = self.on[i];
            let now = if p == P::Drone { true } else { self.persona[i].decide(was, inv, &mut self.rng) };
            self.on[i] = now;
            if now != was {
                let m = match (p, now) {
                    (P::Lead, false) => "lead resolves the melody and lays out".to_string(),
                    (p, true) => format!("{} joins", p.name()),
                    (p, false) => format!("{} lays out", p.name()),
                };
                if p == P::Lead && !now {
                    // resolve: one long chord-root note
                    let r = self.motif_pitch(0, true, 31);
                    self.role_note(Role::Lead, r, 55, 12.0, 0.0);
                    let n = self.note_name(r);
                    self.think(4, "laying out", format!("resolving the motif to the root ({n}), then resting"));
                }
                self.band_say(m);
            }
        }
        // bassist mode (always present, never silent)
        let inv = self.leader.invite(P::Bass) + (self.persona[1].eager - 0.5) * 0.1;
        let mode = if inv < 0.45 { BassMode::Sustain } else if inv < 0.8 { BassMode::Pulse } else { BassMode::Groove };
        if mode != self.bass_mode {
            self.bass_mode = mode;
            let m = match mode {
                BassMode::Sustain => "bassist settles into long notes",
                BassMode::Pulse => "bassist starts pulsing on the roots",
                BassMode::Groove => "bassist starts a groove",
            };
            self.band_say(m.into());
        }
        self.make_bass_pattern();
        self.make_drum_pattern();
        // keys comping style for this phrase
        let w: &[f32; 5] = match self.leader.state {
            State::Chill => &[0.6, 0.3, 0.0, 0.1, 0.0],
            State::Awaken => &[0.3, 0.35, 0.05, 0.25, 0.05],
            State::Alive => &[0.05, 0.25, 0.3, 0.2, 0.2],
            State::Subside => &[0.5, 0.3, 0.0, 0.2, 0.0],
        };
        self.comp = [Comp::Sparse, Comp::Arp, Comp::ArpUD, Comp::Broken, Comp::Offbeat][self.rng.weighted(w)];
        // drummer fills before phrase ends sometimes
        let pf = match self.leader.state {
            State::Alive => 0.35,
            State::Awaken => 0.15,
            _ => 0.0,
        };
        if self.on[2] && self.rng.chance(pf) {
            self.fill = true;
        }
        self.phrase_minds();
    }

    /// Phrase-level summary of what each player decided and why (UI only).
    fn phrase_minds(&mut self) {
        if self.ui.is_none() {
            return;
        }
        let st = self.leader.state.name();
        let sig = self.ctl.sig;
        let solo = self.leader.soloist;
        let lone = self.leader.lone;
        let brk = self.leader.breakdown > 0;
        let trade = self.leader.trade;
        // bassist
        let inv = self.leader.invite(P::Bass);
        let (bs, bt) = match self.bass_mode {
            BassMode::Sustain => ("following", format!("only long roots — {st}, invite {inv:.2}")),
            BassMode::Pulse => ("following", format!("pulsing roots/fifths with the drummer (invite {inv:.2})")),
            BassMode::Groove => ("comping", if sig.file > 0.4 { format!("disk/file churn {:.0}% → busier syncopated groove", sig.file * 100.0) } else { "locking a syncopated groove".into() }),
        };
        let bs = if brk { "laying out" } else { bs };
        let bt = if brk { "breakdown → dropping to long notes".into() } else { bt };
        self.think(1, bs, bt);
        // drummer
        let d = self.leader.invite(P::Drums);
        let (ds, dt): (&'static str, String) = if !self.on[2] {
            ("laying out", format!("not invited yet (invite {d:.2})"))
        } else if brk {
            ("laying out", "breakdown → out for a phrase".into())
        } else if self.leader.state == State::Subside {
            ("following", "thinning out bar by bar".into())
        } else if self.fill {
            ("following", "fill into the next phrase".into())
        } else if self.bass_mode == BassMode::Groove {
            ("following", format!("kicks follow the bassist's syncopations (cpu {:.0}%)", sig.cpu * 100.0))
        } else {
            ("comping", format!("simple time-keeping at {st} energy"))
        };
        self.think(2, ds, dt);
        // keys
        let (ks, kt): (&'static str, String) = if !self.on[3] {
            ("laying out", "waiting for space".into())
        } else if solo == Some(P::Keys) || lone == Some(P::Keys) {
            ("soloing", "bandleader gave me the floor → taking the melody".into())
        } else if trade && self.on[4] {
            ("trading", "trading 2-bar phrases with the lead".into())
        } else {
            let style = match self.comp {
                Comp::Sparse => "sparse stabs, lots of space",
                Comp::Arp => "rolling arpeggio",
                Comp::ArpUD => "up-and-down arpeggio",
                Comp::Broken => "broken chord pattern",
                Comp::Offbeat => "offbeat chord stabs",
            };
            ("comping", format!("{style}; pad holds the common tones"))
        };
        self.think(3, ks, kt);
        // lead (status only; the think line comes from motif development)
        let ls: &'static str = if !self.on[4] {
            "laying out"
        } else if solo == Some(P::Lead) || lone == Some(P::Lead) {
            "soloing"
        } else if trade {
            "trading"
        } else {
            "comping"
        };
        if !self.on[4] {
            if self.minds[4].status != "laying out" {
                self.think(4, ls, "resting, listening to the band");
            }
        } else {
            self.minds[4].status = ls;
        }
        // colour
        let cs: &'static str = if self.ind_steps > 0 {
            "soloing"
        } else if !self.on[5] {
            "laying out"
        } else if solo == Some(P::Colour) || lone == Some(P::Colour) {
            "soloing"
        } else {
            "following"
        };
        if self.ind_steps == 0 {
            let ct = if !self.on[5] {
                "only a stray shimmer now and then".to_string()
            } else if sig.net > 0.4 {
                format!("network at {:.0}% → shimmer clouds", sig.net * 100.0)
            } else if self.meander() {
                "slow landscape swells".into()
            } else {
                "listening for the lead's calls".into()
            };
            self.think(5, cs, ct);
        } else {
            self.minds[5].status = cs;
        }
    }

    fn on_bar_line(&mut self, bar: u64) {
        if self.entry_wait > 0 && bar % 4 == 1 {
            self.entry_wait = 0;
        }
        // drone/landscape player: air swells, shimmer clouds
        if self.meander() && self.rng.chance(0.1) {
            for k in 0..(3 + self.rng.below(3)) {
                let pcs = self.chord().pcs();
                let pc = pcs[self.rng.below(pcs.len())];
                let semi = pc + 36 + 12 * self.rng.below(2) as i32;
                self.bell(semi as f32, 0.5, k as f32 * 0.15);
            }
        }
        self.gm_manage();
        // harmony
        self.bars_since_change += 1;
        let meander = self.meander();
        if let Some(ch) = self.harm.on_bar(self.cur_bpc, meander) {
            let note = ch.note.clone();
            let kc = ch.key_changed;
            self.on_chord(ch.chord, note, false);
            if kc {
                self.band_say(format!("key change -> {}", ch.key.name()));
            }
        }
    }

    fn on_chord(&mut self, c: Chord, note: Option<String>, init: bool) {
        self.bars_since_change = 0;
        // harmonic rhythm depends on the arrangement state
        let cb = self.chord_bars;
        self.cur_bpc = match self.leader.state {
            State::Chill => cb * 2 + self.rng.below(2) as u32,
            State::Awaken => cb,
            State::Alive => if self.rng.chance(0.5) { (cb / 2).max(1) } else { cb },
            State::Subside => cb + self.rng.below(2) as u32,
        };
        self.history.push_back(c.name());
        while self.history.len() > 4 {
            self.history.pop_front();
        }
        self.hist_ch.push_back(c);
        while self.hist_ch.len() > 4 {
            self.hist_ch.pop_front();
        }
        let hist: Vec<String> = self.history.iter().cloned().collect();
        let line = format!(
            "Key: {} | {}{}",
            self.key().name(),
            hist.join(" \u{2192} "),
            note.clone().map(|n| format!("   ({n})")).unwrap_or_default()
        );
        self.say(line);
        // drone player: voice-led pad, sub follows the root
        let mut pcs = c.pcs();
        if self.meander() && pcs.len() == 3 && self.rng.chance(0.3) {
            pcs.push((c.root + 14).rem_euclid(12)); // add9 colour in the landscape
        }
        let n = match self.leader.state {
            State::Alive => 5,
            State::Chill => 4,
            _ => 4,
        } + self.rng.below(2);
        let prev: Vec<i32> = self.tones.iter().filter(|t| t.on).map(|t| t.target.round() as i32).collect();
        let v = voice_lead(&prev, &pcs, n.min(MAX_T), 2, 27);
        for (i, t) in self.tones.iter_mut().enumerate() {
            if i < v.len() {
                t.target = v[i] as f32;
                if init || !t.on {
                    t.semi = t.target;
                }
                t.on = true;
            } else {
                t.on = false;
            }
        }
        self.sub_target = Self::fold(c.root, -22, -11) as f32;
        if self.ui.is_some() {
            let sub = self.note_name(Self::fold(c.root, -22, -11));
            let why = match &note {
                _ if self.meander() => "slow drift".to_string(),
                _ => format!("{} voices, smooth", v.len()),
            };
            self.think(0, "following", format!("voice-leading pad into {}, sub glides to {sub} · {why}", c.name()));
            let names: Vec<String> = v.iter().map(|&x| self.note_name(x)).collect();
            self.heard_notes[0].clear();
            let now = self.now;
            self.heard_notes[0].push_back((now, sub));
            for n in names {
                self.heard_notes[0].push_back((now, n));
            }
            self.minds[0].activity = (self.minds[0].activity + 0.4).min(1.0);
        }
        if init {
            self.sub_semi = self.sub_target;
        }
        if self.guest_on {
            self.guest_semi = self.guest_pitch();
        }
        // bassist: sustain mode strikes on chord changes
        if self.bass_mode == BassMode::Sustain {
            self.bass_note(0, 0.75, 0.25, 5.0);
        }
        // keys: GM pad voice leading, common tones held
        if let Some(ch) = self.slot_ch(Role::Pad) {
            let prev: Vec<i32> = self.pad_held.iter().filter(|h| h.0 == ch).map(|h| h.1).collect();
            let nv = voice_lead(&prev, &c.pcs(), 4, 7, 29);
            let now = self.now;
            let mut keep = Vec::new();
            let held = std::mem::take(&mut self.pad_held);
            for (hc, k) in held {
                if hc == ch && nv.contains(&k) {
                    keep.push((hc, k));
                } else if let Some(g) = self.gm.as_mut() {
                    g.note_off(hc, self.midi_root + k, now);
                }
            }
            for &k in &nv {
                if !keep.iter().any(|h| h.1 == k) {
                    let vel = 42 + self.rng.below(14) as i32;
                    if let Some(g) = self.gm.as_mut() {
                        g.note_on(ch, self.midi_root + k, vel, now);
                    }
                    keep.push((ch, k));
                }
            }
            self.pad_held = keep;
        }
    }

    // -------------------------------------------------------------- bassist

    fn make_bass_pattern(&mut self) {
        let busy = self.persona[1].busy;
        let mut p = [0i32; 16];
        match self.bass_mode {
            BassMode::Sustain => {}
            BassMode::Pulse => {
                p[0] = 1;
                p[8] = if self.rng.chance(0.5) { 2 } else { 1 };
                if self.rng.chance(0.3 * busy) {
                    p[14] = 4;
                }
            }
            BassMode::Groove => {
                const W: [f32; 16] = [1.0, 0.1, 0.3, 0.55, 0.2, 0.15, 0.65, 0.25, 0.9, 0.1, 0.4, 0.6, 0.25, 0.3, 0.7, 0.35];
                for i in 0..16 {
                    if i == 0 || self.rng.chance(W[i] * (0.4 + busy * 0.6)) {
                        p[i] = [1, 1, 1, 2, 2, 3][self.rng.below(6)];
                    }
                }
                p[0] = 1;
                p[15] = if self.rng.chance(0.5) { 4 } else { p[15] };
            }
        }
        self.bass_pat = p;
    }

    /// kind: 0 root, 1 fifth, 2 octave, 3 approach to next root
    fn bass_note(&mut self, kind: i32, vel: f32, att: f32, dec: f32) {
        let c = self.chord();
        let root = Self::fold(c.root, -17, -6); // ~41-78 Hz
        let semi = match kind {
            1 => root + (c.fifth() - c.root).rem_euclid(12),
            2 => root + 12,
            3 => {
                let next = self.harm.peek_next(self.meander());
                let nr = Self::fold(next.root, -17, -6);
                nr + if self.rng.chance(0.5) { -1 } else { 2 }
            }
            _ => root,
        };
        self.heard_semi(1, semi, vel);
        if kind == 3 && self.ui.is_some() {
            let next = self.harm.peek_next(self.meander());
            self.set_think(1, format!("walking an approach note into {}", next.name()));
        }
        let f = semi_to_freq(self.root, semi as f32);
        self.bass_inc = f / self.sr;
        self.bass_attack = true;
        self.bass_peak = vel;
        self.bass_att = 1.0 - (-1.0 / (self.sr * att)).exp();
        self.bass_dec = (-1.0 / (self.sr * dec)).exp();
        if self.bass_mode == BassMode::Groove && self.slot_ch(Role::Bass).is_some() {
            let v = (vel * 80.0) as i32;
            self.role_note(Role::Bass, semi + 12, v, 2.0, 0.0);
        }
    }

    fn bass_step(&mut self, s: usize) {
        if self.entry_wait > 0 {
            return;
        }
        let v = self.bass_pat[s];
        if v == 0 {
            return;
        }
        let change_next = self.bars_since_change + 1 >= self.cur_bpc;
        let kind = match v {
            4 if change_next => 3,
            4 => 1,
            2 => 1,
            3 => 2,
            _ => 0,
        };
        let (vel, att, dec) = match self.bass_mode {
            BassMode::Pulse => (0.8, 0.03, 1.0),
            BassMode::Groove => (if s % 4 == 0 { 0.9 } else { 0.7 }, 0.008, 0.35 + 0.1 * self.rng.f()),
            BassMode::Sustain => (0.75, 0.25, 5.0),
        };
        self.bass_note(kind, vel, att, dec);
    }

    // -------------------------------------------------------------- drummer

    fn make_drum_pattern(&mut self) {
        let d = if self.on[2] { self.leader.invite(P::Drums) } else { 0.0 };
        let ps = self.persona[2];
        let mut k = [0i32; 16];
        let mut sn = [0i32; 16];
        let mut h = [0i32; 16];
        let mut a = [0i32; 16];
        if d > 0.15 {
            k[0] = 70;
            if d > 0.5 {
                k[8] = 60;
            }
            for &i in &[3usize, 6, 10, 11, 14] {
                // the drummer listens to the bassist's syncopations
                let follow = if self.bass_pat[i] != 0 { 0.5 } else { 0.0 };
                if self.rng.chance(((d - 0.6) * ps.busy * 1.5).max(0.0) + follow * (d - 0.5).max(0.0)) {
                    k[i] = 45;
                }
            }
            if d > 0.6 {
                sn[4] = 45;
                sn[12] = 50;
                for i in [7usize, 15] {
                    if self.rng.chance(ps.busy * 0.3) {
                        sn[i] = 18;
                    }
                }
            } else if d > 0.4 && self.rng.chance(0.5) {
                sn[12] = 35;
            }
            if d > 0.3 {
                let sixteenths = d > 0.8 && ps.busy > 0.55;
                for i in 0..16 {
                    if i % 2 == 0 || sixteenths {
                        h[i] = if i % 4 == 0 { 34 } else { 24 } + if sixteenths && i % 2 == 1 { -6 } else { 0 };
                    }
                }
            }
            if ps.register > 0.2 && d > 0.35 {
                for i in 0..16 {
                    a[i] = if i % 2 == 1 { 16 } else { 10 };
                }
            }
        }
        self.drum_pat = [k, sn, h, a];
    }

    fn drums_step(&mut self, s: usize, bar: u64) {
        let in_subside = self.leader.state == State::Subside;
        if s == 0 && in_subside {
            self.make_drum_pattern(); // thin out bar by bar
        }
        let last_bar = bar % 4 == 3;
        let swing = if s % 2 == 1 { self.persona[2].swing * 0.33 } else { 0.0 };
        if self.crash_next && s == 0 && self.on[2] {
            self.crash_next = false;
            self.drum(49, 38, 0.0);
        }
        if self.fill && last_bar && s >= 8 {
            if s == 15 {
                self.fill = false;
                self.crash_next = true;
            }
            if s % 2 == 0 {
                let tom = [50, 48, 47, 45][((s - 8) / 2) % 4];
                let v = 34 + (s as i32 - 8) * 3;
                self.drum(tom, v, 0.0);
                self.kick(0.25);
            }
            return;
        }
        if self.entry_wait > 0 || !self.on[2] {
            return;
        }
        let [k, sn, h, a] = self.drum_pat;
        if k[s] > 0 {
            self.drum(36, k[s], swing);
            self.kick(k[s] as f32 / 90.0);
        }
        if sn[s] > 0 {
            let key = if self.persona[2].reply > 0.5 { 37 } else { 38 };
            self.drum(key, sn[s], swing);
        }
        if h[s] > 0 {
            let key = if self.persona[2].eager > 0.65 { 51 } else { 42 };
            self.drum(key, h[s], swing);
            if self.gm.is_none() {
                self.hat_env = 1.0;
                self.hat_amp = h[s] as f32 / 40.0;
            }
        }
        if a[s] > 0 {
            self.drum(if self.persona[2].busy > 0.5 { 82 } else { 70 }, a[s], swing);
        }
    }

    fn kick(&mut self, v: f32) {
        self.kick_on = true;
        self.kick_t = 0.0;
        self.kick_ph = 0.0;
        self.kick_amp = v.clamp(0.0, 1.0);
    }

    // ----------------------------------------------------------------- keys

    fn keys_step(&mut self, s: usize, bar: u64) {
        if !self.on[3] {
            return;
        }
        let soloing = self.leader.soloist == Some(P::Keys) || self.leader.lone == Some(P::Keys)
            || (self.leader.trade && (bar / 2) % 2 == 1 && self.on[4]);
        if soloing {
            // keys takes the melody (trading with the lead)
            let m = self.motif.clone();
            let t = ((bar % 2) * 16) as u8 + s as u8;
            for n in m.notes.iter().filter(|n| n.at == t) {
                let p = self.motif_pitch(n.deg + 2, n.at % 4 == 0, 26);
                self.role_note(Role::Comp, p, n.vel - 8, n.len as f32, 0.0);
            }
            return;
        }
        if self.entry_wait > 0 {
            return;
        }
        let c = self.chord();
        let pcs = c.pcs();
        let base = Self::fold(c.root, 12, 23);
        let mut tones: Vec<i32> = pcs.iter().map(|&pc| base + (pc - c.root).rem_euclid(12)).collect();
        tones.sort();
        let n = tones.len();
        let up: Vec<i32> = tones.iter().cloned().chain(tones.iter().map(|t| t + 12)).collect();
        let v = 40 + (self.leader.invite(P::Keys) * 25.0) as i32 + if s % 4 == 0 { 6 } else { 0 };
        match self.comp {
            Comp::Sparse => {
                if s == 0 || (s == 8 && self.rng.chance(0.5)) {
                    let p = up[self.rng.below(up.len())];
                    self.role_note(Role::Comp, p, v - 6, 8.0, 0.0);
                }
            }
            Comp::Arp => {
                if s % 2 == 0 {
                    let p = up[(s / 2) % up.len()];
                    self.role_note(Role::Comp, p, v, 3.0, 0.0);
                }
            }
            Comp::ArpUD => {
                let len = up.len() * 2 - 2;
                let i = s % len.max(1);
                let p = if i < up.len() { up[i] } else { up[len - i] };
                self.role_note(Role::Comp, p, v - 4, 1.5, 0.0);
            }
            Comp::Broken => {
                if s % 2 == 0 {
                    let order = [0usize, 2, 1, 2];
                    let p = tones[order[(s / 2) % 4] % n];
                    self.role_note(Role::Comp, p, v, 2.5, 0.0);
                }
            }
            Comp::Offbeat => {
                if s % 4 == 2 {
                    for &p in tones.iter().take(3) {
                        self.role_note(Role::Comp, p + 12, v - 8, 1.0, 0.0);
                    }
                }
            }
        }
    }

    // ----------------------------------------------------------------- lead

    fn lead_step(&mut self, s: usize, bar: u64) {
        if !self.on[4] || self.entry_wait > 0 {
            return;
        }
        let soloing = self.leader.soloist == Some(P::Lead) || self.leader.lone == Some(P::Lead);
        let state = self.leader.state;
        // call (bars 0-1) / rest (bars 2-3) unless soloing
        let phase = bar % 4;
        let calling = soloing || phase < 2 || (state == State::Alive && self.persona[4].busy > 0.65 && phase == 2);
        if self.leader.trade && (bar / 2) % 2 == 1 && !soloing {
            return; // the keys have this phrase
        }
        if s == 0 && phase == 0 {
            let density = match state {
                State::Chill => 0.3,
                State::Awaken => 0.5,
                State::Alive => 0.7,
                State::Subside => 0.35,
            } * (0.7 + 0.6 * self.persona[4].busy);
            let (m, what) = self.motif.develop(&mut self.rng, density);
            self.motif = m;
            self.lead_calls += 1;
            if what == "new idea" {
                self.band_say("lead starts a new melodic idea".into());
            }
            if self.ui.is_some() {
                let st = if soloing { "soloing" } else if self.leader.trade { "trading" } else { "comping" };
                let t = match what {
                    "new idea" => "a fresh motif — new idea".to_string(),
                    w => format!("developing the motif: {w}{}", if state == State::Chill { " (fragments only)" } else { "" }),
                };
                self.think(4, st, t);
            }
        }
        if s == 0 && phase == 2 && !soloing {
            // the colour player may answer the call
            if self.on[5] && self.rng.chance(0.25 + 0.5 * self.persona[5].reply) {
                let (m, _) = self.motif.develop(&mut self.rng, 0.4);
                self.reply = Some((m, bar));
                self.band_say("colour player answers the lead".into());
                self.think(5, "trading", "heard the lead's call → answering it a 3rd up");
            }
        }
        if !calling {
            return;
        }
        let t = ((phase % 2) * 16) as u8 + s as u8;
        let notes: Vec<_> = self.motif.notes.iter().filter(|n| n.at == t).cloned().collect();
        let center = 31 + (self.persona[4].register * 4.0) as i32; // ~E5
        for (i, n) in notes.iter().enumerate() {
            if state == State::Chill && !soloing && (n.at > 8 || i > 0) {
                continue; // just fragments while chilling
            }
            let p = self.motif_pitch(n.deg, n.at % 4 == 0, center);
            if self.slot_ch(Role::Lead).is_some() {
                self.role_note(Role::Lead, p, n.vel - 10, n.len as f32, 0.0);
            } else {
                self.bell(p as f32 + 12.0, 0.6, 0.0);
                self.heard_semi(4, p + 12, 0.6);
            }
        }
    }

    // --------------------------------------------------------------- colour

    fn colour_step(&mut self, s: usize, bar: u64) {
        // shimmer: listens to the network
        let net = self.ctl.sig.net;
        let p = (0.01 + 0.22 * net) * if self.on[5] { 1.0 } else { 0.4 };
        if self.rng.chance(p) {
            let pcs = self.chord().pcs();
            let pc = pcs[self.rng.below(pcs.len())];
            let oct = 12 * self.rng.below(2) as i32;
            self.bell((pc + 36 + oct) as f32, 0.45 + 0.4 * net, 0.0);
            if net > 0.45 && self.ui.is_some() && self.ind_steps == 0 && self.reply.is_none() {
                self.set_think(5, format!("heard a network burst ({:.0}%) → adding shimmer", net * 100.0));
            }
        }
        if !self.on[5] {
            return;
        }
        // answer to the lead
        if let Some((m, b0)) = self.reply.clone() {
            let rel = (bar.saturating_sub(b0) * 16) as i64 + s as i64;
            if rel >= 32 {
                self.reply = None;
            } else {
                for n in m.notes.iter().filter(|n| n.at as i64 == rel) {
                    let p = self.motif_pitch(n.deg + 2, n.at % 4 == 0, 36);
                    self.role_note(Role::Colour, p, n.vel - 15, n.len as f32 + 2.0, 0.0);
                }
            }
        }
        // landscape swells
        if s == 0 {
            let lone = self.leader.lone == Some(P::Colour) || self.leader.soloist == Some(P::Colour);
            let pr = if lone { 0.8 } else if self.meander() { 0.3 } else { 0.15 };
            if self.rng.chance(pr) {
                let pcs = self.chord().pcs();
                let pc = pcs[self.rng.below(pcs.len())];
                let semi = Self::fold(pc + 24, 24, 40);
                let len = 16.0 * (1 + self.rng.below(3)) as f32;
                let v = 38 + self.rng.below(15) as i32;
                let d = self.rng.f() * 2.0;
                self.role_note(Role::Colour, semi, v, len, d);
                if self.rng.chance(0.4) {
                    let pc2 = pcs[self.rng.below(pcs.len())];
                    self.role_note(Role::Colour, Self::fold(pc2 + 24, 24, 40), 34, len, 4.0);
                }
            }
        }
    }

    // --------------------------------------------------- GM layer manager

    fn pick_program(&mut self, role: Role) -> u8 {
        let w: Vec<f32> = (0..128u8)
            .map(|p| {
                let mut w = role_weight(role, p);
                let sl = self.slots.iter().find(|s| s.role == role).unwrap();
                if sl.recent.contains(&p) || self.recent_global.contains(&p) {
                    w *= 0.05;
                }
                if self.slots.iter().any(|s| s.role != role && s.on && s.program == Some(p)) {
                    w = 0.0;
                }
                w
            })
            .collect();
        self.rng.weighted(&w) as u8
    }

    fn gm_manage(&mut self) {
        if self.gm.is_none() {
            return;
        }
        let state = self.leader.state;
        // which layers the musicians want
        let lone = self.leader.lone;
        for i in 0..self.slots.len() {
            let want = match self.slots[i].role {
                Role::Pad => self.on[3] || self.meander(),
                Role::Comp => self.on[3] && (self.leader.invite(P::Keys) > 0.45 || lone == Some(P::Keys) || self.leader.soloist == Some(P::Keys)),
                Role::Lead => self.on[4],
                Role::Colour => self.on[5],
                Role::Bass => self.bass_mode == BassMode::Groove && self.persona[1].eager > 0.35,
            };
            self.slots[i].want = want;
        }
        let order: [Role; 4] = match state {
            State::Chill => [Role::Pad, Role::Colour, Role::Comp, Role::Lead],
            State::Awaken => [Role::Pad, Role::Comp, Role::Lead, Role::Colour],
            State::Alive => [Role::Comp, Role::Lead, Role::Pad, Role::Colour],
            State::Subside => [Role::Pad, Role::Colour, Role::Comp, Role::Lead],
        };
        let budget = self.gm_layers + (state == State::Alive) as usize;
        let mut allowed: Vec<Role> = Vec::new();
        for r in order {
            if allowed.len() < budget && self.slot(r).want {
                allowed.push(r);
            }
        }
        if self.slot(Role::Bass).want {
            allowed.push(Role::Bass);
        }
        let now = self.now;
        let sr = self.sr as u64;
        for i in 0..self.slots.len() {
            let role = self.slots[i].role;
            let ok = allowed.contains(&role);
            if ok && !self.slots[i].on {
                // bring a layer in on a silent channel
                let s = &self.slots[i];
                let ch = if self.gm.as_ref().unwrap().silent(s.chans[s.cur], now) { s.cur } else { 1 - s.cur };
                if !self.gm.as_ref().unwrap().silent(self.slots[i].chans[ch], now) {
                    continue;
                }
                let p = self.pick_program(role);
                let chn = self.slots[i].chans[ch];
                let fade = self.rng.range(4.0, 9.0);
                if let Some(g) = self.gm.as_mut() {
                    g.program(chn, p, prog_gain(p));
                    g.fade(chn, 1.0, fade);
                }
                let sl = &mut self.slots[i];
                sl.cur = ch;
                sl.program = Some(p);
                sl.on = true;
                sl.next_rotate = now + sr * (20 + (self.rng.next() % 21) as u64);
                sl.recent.push_back(p);
                if sl.recent.len() > 8 {
                    sl.recent.pop_front();
                }
                self.recent_global.push_back(p);
                if self.recent_global.len() > 16 {
                    self.recent_global.pop_front();
                }
                self.say(format!("+ {} ({}) [{:?}]", GM_NAMES[p as usize], p, role));
            } else if !ok && self.slots[i].on {
                let s = &self.slots[i];
                let chn = s.chans[s.cur];
                let fade = self.rng.range(6.0, 12.0);
                if let Some(g) = self.gm.as_mut() {
                    g.fade(chn, 0.0, fade);
                }
                let p = s.program.unwrap_or(0);
                self.slots[i].on = false;
                // release held notes after the fade
                let at = now + (fade * self.sr) as u64;
                let held: Vec<(usize, i32)> = self.pad_held.iter().cloned().filter(|h| h.0 == chn).collect();
                for (hc, k) in held {
                    self.evs.push(Ev { at, ch: hc, key: self.midi_root + k, vel: 0 });
                }
                self.pad_held.retain(|h| h.0 != chn);
                self.say(format!("\u{2212} {} ({}) [{:?}]", GM_NAMES[p as usize], p, role));
            } else if ok && self.slots[i].on && now >= self.slots[i].next_rotate {
                // crossfade to a new instrument on the other (silent) channel
                let s = &self.slots[i];
                let other = 1 - s.cur;
                let och = s.chans[other];
                if !self.gm.as_ref().unwrap().silent(och, now) {
                    self.slots[i].next_rotate = now + sr * 2;
                    continue;
                }
                let old_ch = s.chans[s.cur];
                let old_p = s.program.unwrap_or(0);
                let p = self.pick_program(role);
                let fin = self.rng.range(5.0, 9.0);
                let fout = self.rng.range(7.0, 12.0);
                if let Some(g) = self.gm.as_mut() {
                    g.program(och, p, prog_gain(p));
                    g.fade(och, 1.0, fin);
                    g.fade(old_ch, 0.0, fout);
                }
                // pads: re-strike the voicing on the new channel, release the old later
                let at = now + (fout * self.sr) as u64;
                let held: Vec<(usize, i32)> = self.pad_held.iter().cloned().filter(|h| h.0 == old_ch).collect();
                let mut moved = Vec::new();
                for (hc, k) in held {
                    self.evs.push(Ev { at, ch: hc, key: self.midi_root + k, vel: 0 });
                    if let Some(g) = self.gm.as_mut() {
                        g.note_on(och, self.midi_root + k, 44, now);
                    }
                    moved.push((och, k));
                }
                self.pad_held.retain(|h| h.0 != old_ch);
                self.pad_held.extend(moved);
                let sl = &mut self.slots[i];
                sl.cur = other;
                sl.program = Some(p);
                sl.next_rotate = now + sr * (20 + (self.rng.next() % 21) as u64);
                sl.recent.push_back(p);
                if sl.recent.len() > 8 {
                    sl.recent.pop_front();
                }
                self.recent_global.push_back(p);
                if self.recent_global.len() > 16 {
                    self.recent_global.pop_front();
                }
                self.say(format!("+ {} ({})  \u{2212} {} ({}) [{:?}]", GM_NAMES[p as usize], p, GM_NAMES[old_p as usize], old_p, role));
            }
        }
    }

    // ------------------------------------------------------------ bed voices

    fn bell(&mut self, semi: f32, gain: f32, delay_s: f32) {
        let _ = delay_s;
        self.heard_semi(5, semi.round() as i32, gain);
        let sr = self.sr;
        let f = semi_to_freq(self.root, semi);
        let idx = self.bells.iter().position(|b| !b.on).unwrap_or_else(|| {
            let mut m = 0;
            for (i, b) in self.bells.iter().enumerate() {
                if b.amp < self.bells[m].amp {
                    m = i;
                }
            }
            m
        });
        let (l, r) = pan(self.rng.bi() * 0.8);
        let dec = 0.6 + self.rng.f() * 1.2;
        let a = gain * (0.6 + 0.4 * self.rng.f());
        let b = &mut self.bells[idx];
        b.on = true;
        b.inc1 = f * (3.0f32 / 1200.0).exp2() / sr;
        b.inc2 = f * (-3.0f32 / 1200.0).exp2() / sr;
        b.amp = a;
        b.att = 0.0;
        b.dec = (-1.0 / (sr * dec)).exp();
        b.gl = l;
        b.gr = r;
    }

    fn trigger_click(&mut self) {
        let sr = self.sr;
        let f = 1300.0 + self.rng.f() * 1400.0;
        let a = (0.6 + 0.4 * self.rng.f()) * self.click_level;
        let dec = (-1.0 / (sr * (0.012 + 0.02 * self.rng.f()))).exp();
        let (l, r) = pan(self.rng.bi() * 0.7);
        if let Some(c) = self.clicks.iter_mut().find(|c| !c.on) {
            *c = Click { on: true, ph1: 0.0, ph2: 0.25, inc1: f / sr, inc2: f * 2.31 / sr, amp: a, dec, noise: 0.3, gl: l, gr: r };
        }
    }

    fn control_rate(&mut self) {
        let sr = self.sr;
        let k = 1.0 - (-(CR as f32) / (sr * 6.0)).exp();
        let e = self.leader.energy;
        let warm = self.ctl.warmth;
        let bri = if self.ctl.screen_valid { self.ctl.brightness } else { 0.5 };
        // drone player's own slow drift
        for (i, d) in self.drift.iter_mut().enumerate() {
            *d = (*d + CR as f32 / (sr * [73.0, 117.0, 41.0][i])) % 1.0;
        }
        let d1 = sine(&self.tbl, self.drift[0]);
        let d2 = sine(&self.tbl, self.drift[1]);
        let d3 = sine(&self.tbl, self.drift[2]);
        let tc = 2000.0 * (e * 1.2).exp2() * (0.85 + 0.3 * bri) * (1.0 + 0.15 * warm);
        self.cutoff += (tc - self.cutoff) * k;
        self.detune += ((5.0 + 4.0 * e) - self.detune) * k;
        self.saw_mix += ((0.42 + 0.15 * warm + 0.15 * e).clamp(0.2, 0.7) - self.saw_mix) * k;
        self.voices_f += ((3.0 + (e > 0.5) as i32 as f32) - self.voices_f) * k;
        self.lfo_ph = (self.lfo_ph + (0.04 + 0.08 * e) * CR as f32 / sr) % 1.0;
        let lfo = sine(&self.tbl, self.lfo_ph);
        let fc = self.cutoff * (1.0 + 0.1 * lfo) * (1.0 + 0.25 * d1 + 0.1 * d2);
        self.pad_f[0].set(fc, 0.7, sr);
        self.pad_f[1].set(fc * 1.04, 0.7, sr);
        let glide = 1.0 - (-(CR as f32) / (sr * 2.5)).exp();
        let mut active = 0.0f32;
        let drift_inc = 0.06 * CR as f32 / sr;
        let vf = self.voices_f.round();
        for (ti, t) in self.tones.iter_mut().enumerate() {
            let swell = if ti < 2 { 1.0 } else { 0.6 + 0.4 * sine(&self.tbl, (self.drift[0] * (1.0 + ti as f32 * 0.37) + ti as f32 * 0.23) % 1.0) };
            let tg = if t.on { swell * if ti == 0 { 1.0 } else { 0.85 } } else { 0.0 };
            t.gain += (tg - t.gain) * k * 2.0;
            t.semi += (t.target - t.semi) * glide;
            let base = semi_to_freq(self.root, t.semi);
            for (vi, v) in t.voices.iter_mut().enumerate() {
                let vt = if (vi as f32) < vf { 1.0 } else { 0.0 };
                v.gain += (vt - v.gain) * k;
                v.drift = (v.drift + drift_inc * (0.7 + 0.15 * vi as f32)) % 1.0;
                let dr = sine(&self.tbl, v.drift) * 0.35;
                let cents = (SPREAD[vi] + dr * if vi == 0 { 0.4 } else { 1.0 }) * self.detune;
                v.inc = base * (cents / 1200.0).exp2() / sr;
                if t.gain * v.gain > 1e-4 {
                    active += t.gain * v.gain;
                }
            }
        }
        self.pad_norm = 0.6 / active.max(1.0).sqrt();
        // sub drone glides to the chord root
        let sg = 1.0 - (-(CR as f32) / (sr * 1.2)).exp();
        self.sub_semi += (self.sub_target - self.sub_semi) * sg;
        // bass filter follows its envelope
        self.bass_f.set(140.0 + 520.0 * self.bass_env * (0.4 + e), 0.8, sr);
        // air layer: silent by default (kept only as a hook)
        self.air += (self.air_target - self.air) * k * 0.5;
        self.air_f.set(9000.0 + 2500.0 * d3, 0.6, sr);
        // industrial texture
        self.ind_cd -= CR as f32 / sr;
        let ik = 1.0 - (-(CR as f32) / (sr * 0.8)).exp();
        self.ind_gain += (self.ind_target - self.ind_gain) * ik;
        self.ind_f.set(self.ind_freq, 1.8, sr);
        self.ind_f2.set(self.ind_freq * 2.37, 6.0, sr);
        // riser
        if self.riser_t < 1.0 {
            self.riser_t += CR as f32 / (16.0 * self.step_len);
            self.riser = self.riser_t.min(1.0).powi(2);
            self.riser_f.set(400.0 * (5.0 * self.riser_t).exp2(), 1.2, sr);
            if self.riser_t >= 1.0 {
                self.riser = 0.0;
            }
        }
        // guest (thread lead)
        let gk = 1.0 - (-(CR as f32) / (sr * 6.0)).exp();
        let gt = if self.guest_on { 0.45 + 0.3 * self.guest_cpu.min(1.0) } else { 0.0 };
        self.guest_gain += (gt - self.guest_gain) * gk;
        self.guest_vib = (self.guest_vib + 4.5 * CR as f32 / sr) % 1.0;
        // click scheduling
        self.click_gap -= CR as f32 / sr;
        if self.click_level > 0.0 && self.ctl.click_rate > 0.0 && self.click_gap <= 0.0 && self.rng.f() < self.ctl.click_rate * CR as f32 / sr {
            self.trigger_click();
            self.click_gap = 0.07;
        }
    }

    fn render_pad_block(&mut self) {
        let m = self.saw_mix;
        let tbl = &self.tbl[..];
        let mut l = [0f32; CR];
        let mut r = [0f32; CR];
        for t in self.tones.iter_mut() {
            for v in t.voices.iter_mut() {
                let g = t.gain * v.gain;
                let (inc, mut ph) = (v.inc, v.ph);
                if g < 1e-4 {
                    ph += inc * CR as f32;
                    v.ph = ph - ph.floor();
                    continue;
                }
                let (gs, gn) = (g * m, g * (1.0 - m));
                for k in 0..CR {
                    let s = saw(ph, inc) * gs + sine(tbl, ph) * gn;
                    ph += inc;
                    if ph >= 1.0 {
                        ph -= 1.0;
                    }
                    l[k] += s * v.gl;
                    r[k] += s * v.gr;
                }
                v.ph = ph;
            }
        }
        self.padl = l;
        self.padr = r;
    }

    /// Render interleaved stereo frames.
    pub fn render(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let ui_on = self.ui.is_some();
        let sr = self.sr;
        let mut fi = 0;
        while fi < frames {
            let n = CHUNK.min(frames - fi);
            // clock: steps fall on chunk boundaries (~1.5 ms resolution)
            self.step_pos += n as f32;
            while self.step_pos >= self.step_len {
                self.step_pos -= self.step_len;
                self.on_step();
            }
            // GM events due in this chunk
            let end = self.now + n as u64;
            if self.gm.is_some() && !self.evs.is_empty() {
                let now = self.now;
                let mut i = 0;
                while i < self.evs.len() {
                    if self.evs[i].at < end {
                        let e = self.evs.swap_remove(i);
                        let g = self.gm.as_mut().unwrap();
                        if e.vel > 0 { g.note_on(e.ch, e.key, e.vel, now) } else { g.note_off(e.ch, e.key, now) }
                    } else {
                        i += 1;
                    }
                }
            }
            let gm_on = if let Some(g) = self.gm.as_mut() {
                g.render(n);
                true
            } else {
                false
            };
            for j in 0..n {
                if self.cr_count == 0 {
                    self.control_rate();
                    self.render_pad_block();
                    self.cr_count = CR;
                }
                let ci = CR - self.cr_count;
                self.cr_count -= 1;
                let tbl = &self.tbl;
                // pad (low-mids)
                let pl = self.pad_f[0].run(self.padl[ci] * self.pad_norm).0;
                let pr = self.pad_f[1].run(self.padr[ci] * self.pad_norm).0;
                // sub drone 30-60 Hz: detuned sine pair + a little 2nd/3rd harmonic
                let f = semi_to_freq(self.root, self.sub_semi);
                let i1 = f / sr;
                let i2 = (f + 0.45) / sr;
                adv(&mut self.sub_ph[0], i1);
                adv(&mut self.sub_ph[1], i2);
                let p1 = self.sub_ph[0];
                let sub = (sine(tbl, p1) + sine(tbl, self.sub_ph[1])) * 0.5
                    + sine(tbl, (p1 * 2.0).fract()) * 0.22
                    + sine(tbl, (p1 * 3.0).fract()) * 0.06;
                let sub = sub * self.sub_level * 0.16;
                // bass voice 40-120 Hz
                let mut bass = 0.0;
                if self.bass_env > 1e-4 || self.bass_attack {
                    if self.bass_attack {
                        self.bass_env += (self.bass_peak - self.bass_env) * self.bass_att;
                        if self.bass_env >= self.bass_peak * 0.97 {
                            self.bass_attack = false;
                        }
                    } else {
                        self.bass_env *= self.bass_dec;
                    }
                    let e = self.bass_env;
                    let raw = saw(self.bass_ph, self.bass_inc) * 0.35 + sine(tbl, self.bass_ph) * 0.9;
                    adv(&mut self.bass_ph, self.bass_inc);
                    bass = self.bass_f.run(raw).0 * e * 0.3;
                }
                // kick
                let mut kick = 0.0;
                if self.kick_on {
                    let t = self.kick_t;
                    let kf = 45.0 + 90.0 * (-t / 0.035).exp();
                    kick = sine(tbl, self.kick_ph) * self.kick_amp * (-t / 0.28).exp();
                    self.kick_ph = (self.kick_ph + kf / sr) % 1.0;
                    self.kick_t += 1.0 / sr;
                    if t > 1.2 {
                        self.kick_on = false;
                    }
                    kick *= 0.3;
                }
                // bells
                let (mut bl, mut br) = (0.0f32, 0.0f32);
                for b in self.bells.iter_mut() {
                    if !b.on {
                        continue;
                    }
                    b.att += (1.0 - b.att) * 0.02;
                    let s = (sine(tbl, b.ph1) + sine(tbl, b.ph2)) * 0.5 * b.amp * b.att;
                    adv(&mut b.ph1, b.inc1);
                    adv(&mut b.ph2, b.inc2);
                    b.amp *= b.dec;
                    if b.amp < 1e-4 {
                        b.on = false;
                    }
                    bl += s * b.gl;
                    br += s * b.gr;
                }
                bl *= 0.07;
                br *= 0.07;
                // clicks
                let (mut cl, mut cr) = (0.0f32, 0.0f32);
                for c in self.clicks.iter_mut() {
                    if !c.on {
                        continue;
                    }
                    let nz = self.rng.bi() * c.noise;
                    let s = (sine(tbl, c.ph1) * 0.7 + sine(tbl, c.ph2) * 0.3 + nz * 0.25) * c.amp;
                    adv(&mut c.ph1, c.inc1);
                    adv(&mut c.ph2, c.inc2);
                    c.amp *= c.dec;
                    c.noise *= 0.97;
                    if c.amp < 1e-4 {
                        c.on = false;
                    }
                    cl += s * c.gl;
                    cr += s * c.gr;
                }
                cl *= 0.04;
                cr *= 0.04;
                // air / breath (highs) + riser
                let nz = self.rng.bi();
                let mut air = 0.0;
                if self.air > 1e-4 {
                    air = self.air_f.run(nz).1 * self.air * 0.07;
                }
                // industrial: gated band-passed noise + a metallic resonance, ~0.018 peak
                if self.ind_gain > 1e-4 && self.ind_env > 1e-4 {
                    let g = self.ind_gain * self.ind_env;
                    let x = self.ind_f.run(nz).1 + 0.5 * self.ind_f2.run(nz).1;
                    air += x * g * 0.018;
                    self.ind_env *= 0.9991; // ~25 ms decay at 44.1k
                }
                let mut ris = 0.0;
                if self.riser > 0.0 {
                    ris = self.riser_f.run(self.rng.bi()).1 * self.riser * 0.05;
                }
                // synth hats (only without GM drums)
                let mut hat = 0.0;
                if self.hat_env > 1e-4 {
                    let h = nz - self.hat_prev + 0.6 * self.hat_hp;
                    self.hat_prev = nz;
                    self.hat_hp = h;
                    hat = h * self.hat_env * self.hat_amp * 0.02;
                    self.hat_env *= 0.9985;
                }
                // guest thread lead
                let (mut gl, mut gr) = (0.0, 0.0);
                if self.guest_gain > 1e-4 {
                    let semi = self.guest_semi + sine(tbl, self.guest_vib) * 0.08;
                    let f = semi_to_freq(self.root, semi) / sr;
                    let s = (saw(self.guest_ph[0], f * 1.0035) + saw(self.guest_ph[1], f * 0.9965)) * 0.25 + sine(tbl, self.guest_ph[2]) * 0.6;
                    adv(&mut self.guest_ph[0], f * 1.0035);
                    adv(&mut self.guest_ph[1], f * 0.9965);
                    adv(&mut self.guest_ph[2], f);
                    if self.cr_count == CR - 1 {
                        self.guest_f.set(f * sr * 2.5, 0.8, sr);
                    }
                    let y = self.guest_f.run(s).0 * self.guest_gain * 0.07;
                    gl = y * 0.6;
                    gr = y * 0.4;
                }
                // GM
                let (ml, mr) = if gm_on {
                    let g = self.gm.as_ref().unwrap();
                    (g.l[j] * 4.0, g.r[j] * 4.0)
                } else {
                    (0.0, 0.0)
                };
                // fx
                let dly_in = (bl + br) * 0.5 + (gl + gr) * 0.3;
                let dl = self.dl[self.di];
                let drr = self.dr[self.di];
                self.dl[self.di] = dly_in + drr * 0.4;
                self.dr[self.di] = dl * 0.85;
                self.di += 1;
                if self.di == self.dl.len() {
                    self.di = 0;
                }
                let send = (pl + pr) * 0.25 + (bl + br) * 0.6 + (dl + drr) * 0.5 + (ml + mr) * 0.3 + (gl + gr) * 0.3 + (cl + cr) * 0.3 + ris;
                let (rl, rr) = self.rev.run(send * 0.5);
                let low = sub + bass + kick; // mono low end, never high-passed
                let mut l = pl + low + bl + cl + ml + gl + air + hat + ris + dl * 0.3 + rl;
                let mut r = pr + low + br + cr + mr + gr - air * 0.7 + hat + ris + drr * 0.3 + rr;
                if self.fade < self.fade_target {
                    self.fade = (self.fade + self.fade_step).min(self.fade_target);
                } else if self.fade > self.fade_target {
                    self.fade = (self.fade - self.fade_step).max(self.fade_target);
                }
                let g = self.master * self.fade * self.fade * 1.9;
                l *= g;
                r *= g;
                let (ol, or) = self.lim.run(l, r);
                out[(fi + j) * 2] = ol;
                out[(fi + j) * 2 + 1] = or;
                if ui_on {
                    // three one-pole splits: <60 Hz, <250 Hz, <3 kHz
                    let m = 0.5 * (ol + or);
                    self.lp[0] += (m - self.lp[0]) * 0.0085;
                    self.lp[1] += (m - self.lp[1]) * 0.035;
                    self.lp[2] += (m - self.lp[2]) * 0.35;
                    let (a, b2, c) = (self.lp[0], self.lp[1], self.lp[2]);
                    self.lv[0] += a * a;
                    self.lv[1] += (b2 - a) * (b2 - a);
                    self.lv[2] += (c - b2) * (c - b2);
                    self.lv[3] += (m - c) * (m - c);
                    self.lv_n += 1;
                }
            }
            self.now = end;
            fi += n;
        }
        if ui_on && self.now.saturating_sub(self.last_pub) as f32 >= self.sr * 0.1 {
            let dt = (self.now - self.last_pub) as f32 / self.sr;
            self.last_pub = self.now;
            for m in self.minds.iter_mut() {
                m.activity *= (-dt / 0.8).exp();
            }
            self.publish();
        }
    }
}
