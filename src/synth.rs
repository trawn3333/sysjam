//! The engine: a small jam band of machine musicians playing over a
//! full-spectrum oscillator bed (sub drone, bass, detuned pad, air) plus an
//! optional General MIDI (SF2) layer.

use crate::band::{Entry, Leader, Motif, Persona, State, P};
use crate::dsp::*;
use crate::gm::{in_palette, prog_gain, role_weight, Gm, Role, GM_NAMES, KITS, PALETTES, PERC_SETS, ROLES};
use crate::voice::{Kind, Patch, Pool};
use crate::harmony::{voice_lead, Chord, Harmony, Key};
use crate::music::semi_to_freq;
use crate::shared::Control;
use rustysynth::SoundFont;
use std::collections::VecDeque;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use crate::ui::Mind;
use crate::harmony::{pc_name, roman_of, Mode, PKey, Rel};
use crate::conductor::{Conductor, Order, Pose};
use crate::style::{BassFeel, DrumFeel, Harm, Scale, Style, STYLES};
use std::sync::atomic::Ordering::Relaxed;

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
    /// --bpm given: never change the tempo
    pub bpm_lock: bool,
    /// --style: None = auto (wanders, starts at House Band)
    pub style: Option<usize>,
    /// --tempo-bias: sticky bpm offset on the living tempo
    pub tempo_bias: f32,
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
enum Feel {
    Normal,
    Half,
    Double,
    /// V-twin lope during a tempo surge
    Surge,
}

/// oscillator patches the players can design for themselves
const BASS_PATCHES: [(&str, f32, f32); 4] = [("round sine bass", 0.08, 1.0), ("warm saw bass", 0.35, 1.0), ("growl bass", 0.6, 2.2), ("soft pluck bass", 0.2, 1.4)];
/// (name, partial ratio, detune cents, decay scale)
const BELL_PATCHES: [(&str, f32, f32, f32); 4] = [("glass bells", 1.0, 3.0, 1.0), ("octave chimes", 2.0, 2.0, 1.3), ("metal bells", 3.01, 4.0, 0.8), ("detuned soft bells", 1.0, 11.0, 1.6)];
/// (name, base Hz, decay s)
const KICK_PATCHES: [(&str, f32, f32); 3] = [("deep kick", 45.0, 0.28), ("tight kick", 58.0, 0.16), ("boomy kick", 40.0, 0.45)];

/// A melodic player's own per-phrase decision: play, hold one long tone, or rest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RMode {
    Play,
    Hold,
    Rest,
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
    recent: VecDeque<u8>,
    /// when this slot last took a new sound
    changed_at: u64,
    /// Some = this layer plays a self-designed synth patch instead of GM
    synth: Option<Patch>,
    desc: String,
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
    pub_now: bool,
    pub_ver: u64,
    // living tempo
    bpm_lock: bool,
    lock_bpm: f32,
    // conductor style (←/→) and the user's tempo bias (↑/↓)
    style: usize,
    style_auto: bool,
    /// ring position: 0 = auto, 1.. = STYLES
    style_sel: usize,
    style_from: Option<usize>,
    style_left: u32,
    swap_at: [u32; 6],
    /// user style switch being measured: (start sample, from, to); finished times in s
    sw_t0: Option<(u64, usize, usize)>,
    /// fast (user) switch: short fades for layers coming and going until then; header clears then
    fast_until: u64,
    sw_ramp_end: u64,
    sf_clear: u64,
    pub sw_times: Vec<(String, f32)>,
    style_kit: bool,
    auto_next: u64,
    cli_bias: f32,
    key_bias: f32,
    bias_last: u64,
    bias_applied: f32,
    bias_ack: u64,
    n_style: u32,
    n_nudge: u32,
    // style feels: techno section (0 groove, 1 breakdown, 2 build, 3 drop), acid line, train/two-step
    tsec: u8,
    tsec_left: u32,
    tsec_ph: u32,
    tsec_len: u32,
    acid_pat: [(i8, u8); 16],
    acid_cut: f32,
    acid_cut_t: f32,
    acid_env: f32,
    acid_on: bool,
    train: bool,
    n_tsec: [u32; 4],
    n_bends: u32,
    bpm_from: f32,
    bpm_to: f32,
    ramp_len: u32,
    ramp_pos: u32,
    feel: Feel,
    feel_phrases: u32,
    /// semitone offset that keeps sub/bass at the same absolute register whatever the root
    reg: i32,
    // tempo-following echo
    dlen: usize,
    dlen_target: usize,
    // inspiration: pending reason per player, time of last sound change, dwell
    insp: [Option<String>; 7],
    hold: [u64; 7],
    since: [u64; 7],
    dwell: [u64; 7],
    prev_sig: crate::band::Signals,
    bass_patch: usize,
    bell_patch: usize,
    kick_patch: usize,
    bass_saw: f32,
    bass_drive: f32,
    bass_saw_t: f32,
    bass_drive_t: f32,
    kick_b: f32,
    kick_d: f32,
    // synth voices, per-player keys, palettes
    pool: Pool,
    rel: [Rel; 7],
    rough: [f32; 7],
    palette: [usize; 7],
    bass_gm_ok: bool,
    kit: usize,
    perc_set: usize,
    gm_used: [u32; 128],
    synth_used: u32,
    gm_used_kits: u32,
    pad_prev: Vec<i32>,
    pad_pcs: Vec<i32>,
    // surge episodes: phrases left, earliest next start, throttle-rev swell
    surge_left: u32,
    surge_test: bool,
    surge_cool: u64,
    rev_on: bool,
    rev_up: bool,
    rev_t: f32,
    rev_ph: [f32; 2],
    rev_lp: [f32; 2],
    // note density bookkeeping: notes per player per arrangement state, seconds per state
    dens: [[u32; 7]; 4],
    st_time: [f64; 4],
    // restraint + conductor
    cond: Conductor,
    cond_log: bool,
    rmode: [RMode; 7],
    rest_run: [u32; 7],
    play_run: [u32; 7],
    prev_tied: [f32; 7],
    // per-source RMS meter (SYSJAM_RMS, bench only), split by drums playing / resting
    meter: Option<Box<Meter>>,
    last_drum: u64,
    // synth vs the rest: when a GM / lead note stops sounding, per player
    gm_until: [u64; 7],
    mel_until: [u64; 7],
    space_prev: [Space; 7],
    bed_g: f32,
    bed_fc: f32,
    drum_minimal: bool,
    space_said: [u64; 7],
    space_tick: u32,
    // themes: one quote at a time, passed around, always cited
    quote: Option<Quote>,
    quote_cool: u64,
    quote_test: bool,
    quote_hist: VecDeque<String>,
    recent_themes: VecDeque<usize>,
    n_quotes: u32,
    quote_log: Vec<String>,
}

/// How a player treats the theme it quotes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Var {
    Straight,
    Fragment,
    Augment,
    Diminish,
    Invert,
    /// first half; another player answers with the second half
    Call,
    Answer,
    /// the bassist plays it low
    Low,
}
impl Var {
    fn desc(self) -> &'static str {
        match self {
            Var::Straight => "a straight quote",
            Var::Fragment => "a fragment",
            Var::Augment => "augmented, as a slow pad melody",
            Var::Diminish => "in diminution, twice as fast",
            Var::Invert => "inverted",
            Var::Call => "the first phrase, as a call",
            Var::Answer => "the answer phrase",
            Var::Low => "low in the bass",
        }
    }
}
struct Quote {
    theme: usize,
    player: usize,
    var: Var,
    /// (absolute step, semitone, length in steps)
    ev: Vec<(u64, i32, f32)>,
    next: usize,
    start: u64,
    end: u64,
    pass: u32,
    cite: String,
    what: String,
}

/// Where the synth voices (designed patches + the drone pad bed) sit in the mix right now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Space {
    /// GM instruments / the lead / a quote are up front: duck and close the filter
    Duck,
    /// drums are playing: stay back as a soft pad
    Back,
    /// drums are out and nobody else is up front: come forward a little
    Stretch,
}
/// synth trim vs GM (−10 dB), then per-space offsets
const SYN_TRIM: f32 = 0.316;
const SPACE_GAIN: [f32; 3] = [0.40, 0.80, 1.58]; // Duck −8 dB, Back −2 dB, Stretch +4 dB
/// slot (pool owner) → player
const OWNER_PL: [usize; 5] = [3, 3, 4, 5, 1];

pub const METER_SRC: [&str; 11] = ["synth patches", "drone pad bed", "bells", "sub drone", "synth bass", "synth kick", "synth hats", "GM melodic", "GM drums", "guest", "total"];
#[derive(Default)]
struct Meter {
    p: [[f64; 11]; 2],
    n: [f64; 2],
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
        // echo buffer long enough for 3 steps at 50 bpm; active length follows the tempo
        let dmax = (sr * 60.0 / 50.0 / 4.0 * 3.0) as usize + 4;
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
                recent: VecDeque::new(),
                changed_at: 0,
                synth: None,
                desc: String::new(),
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
            dl: vec![0.0; dmax],
            dr: vec![0.0; dmax],
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
            pub_now: false,
            pub_ver: 0,
            bpm_lock: c.bpm_lock,
            lock_bpm: c.bpm - c.tempo_bias,
            style: c.style.unwrap_or(0),
            style_auto: c.style.is_none(),
            style_sel: c.style.map(|x| x + 1).unwrap_or(0),
            style_from: None,
            style_left: 0,
            swap_at: [0; 6],
            sw_t0: None,
            fast_until: 0,
            sw_ramp_end: 0,
            sf_clear: 0,
            sw_times: Vec::new(),
            style_kit: false,
            auto_next: (c.sr * 360.0) as u64,
            cli_bias: c.tempo_bias,
            key_bias: 0.0,
            bias_last: 0,
            bias_applied: c.tempo_bias,
            bias_ack: 0,
            n_style: 0,
            n_nudge: 0,
            tsec: 0,
            tsec_left: 0,
            tsec_ph: 0,
            tsec_len: 1,
            acid_pat: [(-1, 0); 16],
            acid_cut: 0.3,
            acid_cut_t: 0.3,
            acid_env: 0.0,
            acid_on: false,
            train: false,
            n_tsec: [0; 4],
            n_bends: 0,
            bpm_from: c.bpm,
            bpm_to: c.bpm,
            ramp_len: 0,
            ramp_pos: 0,
            feel: Feel::Normal,
            feel_phrases: 0,
            reg: 45 - (69.0 + 12.0 * (c.root / 440.0).log2()).round() as i32,
            dlen: dlen.clamp(1, dmax),
            dlen_target: dlen.clamp(1, dmax),
            insp: Default::default(),
            hold: [0; 7],
            since: [0; 7],
            dwell: [(sr * 9.0) as u64; 7],
            prev_sig: Default::default(),
            bass_patch: 1,
            bell_patch: 0,
            kick_patch: 0,
            bass_saw: 0.35,
            bass_drive: 1.0,
            bass_saw_t: 0.35,
            bass_drive_t: 1.0,
            kick_b: 45.0,
            kick_d: 0.28,
            pool: Pool::new(sr, c.seed),
            rel: [Rel::Same; 7],
            rough: [0.0; 7],
            palette: [0; 7],
            bass_gm_ok: true,
            kit: 0,
            perc_set: 0,
            gm_used: [0; 128],
            synth_used: 0,
            gm_used_kits: 0,
            pad_prev: Vec::new(),
            pad_pcs: Vec::new(),
            surge_left: 0,
            surge_test: std::env::var_os("SYSJAM_SURGE_TEST").is_some(),
            surge_cool: (sr * 40.0) as u64,
            rev_on: false,
            rev_up: true,
            rev_t: 0.0,
            rev_ph: [0.0; 2],
            rev_lp: [0.0; 2],
            dens: [[0; 7]; 4],
            st_time: [0.0; 4],
            cond: Conductor::new(),
            cond_log: std::env::var_os("SYSJAM_COND_LOG").is_some(),
            rmode: [RMode::Play; 7],
            rest_run: [0; 7],
            play_run: [0; 7],
            prev_tied: [0.0; 7],
            meter: std::env::var_os("SYSJAM_RMS").map(|_| Box::default()),
            last_drum: 0,
            gm_until: [0; 7],
            mel_until: [0; 7],
            space_prev: [Space::Back; 7],
            bed_g: SYN_TRIM,
            bed_fc: 0.7,
            drum_minimal: false,
            space_said: [0; 7],
            space_tick: 0,
            quote: None,
            quote_cool: (c.sr * if std::env::var_os("SYSJAM_QUOTE_TEST").is_some() { 15.0 } else { 50.0 }) as u64,
            quote_test: std::env::var_os("SYSJAM_QUOTE_TEST").is_some(),
            quote_hist: VecDeque::new(),
            recent_themes: VecDeque::new(),
            n_quotes: 0,
            quote_log: Vec::new(),
            harm,
            rng,
        };
        // per-run signature palettes, drawn by personality (no two players share one)
        let mut used = Vec::new();
        for pl in [3usize, 4, 5, 1] {
            let ps = e.persona[pl];
            let w: Vec<f32> = (0..PALETTES.len())
                .map(|k| {
                    let w = match k {
                        0 => 0.3 + ps.curiosity * 0.5 + ps.reply * 0.3,
                        1 => 0.3 + ps.eager,
                        2 => 0.2 + ps.curiosity,
                        3 => 0.3 + ps.busy,
                        4 => 0.3 + (1.0 - ps.busy),
                        5 => 0.4 + ps.busy * 0.5,
                        6 => 0.3 + (1.0 - ps.eager),
                        _ => 0.1 + ps.curiosity * ps.curiosity,
                    };
                    if used.contains(&k) { 0.0 } else { w }
                })
                .collect();
            let k = e.rng.weighted(&w);
            used.push(k);
            e.palette[pl] = k;
        }
        e.perc_set = e.rng.below(PERC_SETS.len());
        e.style_harmony();
        let ch = e.harm.cur;
        e.on_chord(ch, None, true);
        e
    }

    /// palette names per player (for the start-up log)
    pub fn palettes(&self) -> String {
        [(3, "keys"), (4, "lead"), (5, "colour")].iter().map(|&(i, n)| format!("{n}: {}", PALETTES[self.palette[i]].0)).collect::<Vec<_>>().join(", ")
    }

    /// distinct GM programs used, GM picks, synth patches designed
    pub fn sound_stats(&self) -> (usize, u32, u32) {
        (self.gm_used.iter().filter(|&&n| n > 0).count(), self.gm_used.iter().sum(), self.synth_used)
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
            if self.now >= self.hold[i] {
                self.minds[i].think = t.into();
            }
        }
    }
    fn set_think(&mut self, i: usize, t: impl Into<String>) {
        if self.ui.is_some() && self.now >= self.hold[i] {
            self.minds[i].think = t.into();
        }
    }
    fn note_name(&self, semi: i32) -> String {
        let midi = self.midi_root + semi;
        format!("{}{}", pc_name(semi.rem_euclid(12)), midi.div_euclid(12) - 1)
    }
    /// a player sounded something (UI only)
    fn st_idx(&self) -> usize {
        match self.leader.state {
            State::Chill => 0,
            State::Awaken => 1,
            State::Alive => 2,
            State::Subside => 3,
        }
    }
    fn count(&mut self, i: usize) {
        let st = self.st_idx();
        self.dens[st][i] += 1;
        self.cond.bar_notes[i] += 1;
    }
    /// who is crowding whom: same register within a beat, or on top of the lead
    fn track(&mut self, i: usize, semi: i32) {
        if !(3..=5).contains(&i) {
            return;
        }
        let win = (self.step_len * 8.0) as u64; // two beats
        let now = self.now;
        for j in 3..=5 {
            if j == i {
                continue;
            }
            let (sj, t) = self.cond.last_semi[j];
            if t > 0 && now.saturating_sub(t) < win && ((sj - semi).rem_euclid(12) <= 2 || (sj - semi).abs() <= 7) {
                self.cond.ov_hits += 1;
                if j == 4 {
                    self.cond.step_hits[i] += 1;
                }
            }
        }
        self.cond.last_semi[i] = (semi, now.max(1));
    }
    /// who (other than `pl`) is up front right now: a sounding GM melodic note, or the lead
    fn front_of(&self, pl: usize) -> Option<usize> {
        let tail = (self.step_len * 4.0) as u64;
        let now = self.now;
        if self.quote_front(pl) {
            return self.quote_player();
        }
        if pl != 4 && self.on[4] && self.mel_until[4] + tail > now {
            return Some(4);
        }
        (3..6).find(|&j| j != pl && self.gm_until[j] + tail > now)
    }
    fn quote_front(&self, pl: usize) -> bool {
        self.quote_player().map(|q| q != pl).unwrap_or(false)
    }
    fn quote_owner(&self, pl: usize) -> bool {
        self.quote_player() == Some(pl)
    }
    fn quote_player(&self) -> Option<usize> {
        self.quote.as_ref().filter(|q| q.start <= self.step + 16).map(|q| q.player)
    }
    /// the drummer is actually grooving (minimal ticks don't count: that's the synth's chance)
    fn drums_playing(&self) -> bool {
        (self.rmode[2] == RMode::Play || self.feel == Feel::Surge) && self.last_drum > 0 && (self.now.saturating_sub(self.last_drum) as f32) < self.step_len * 16.0
    }
    /// where player `pl`'s synth voices sit (pl 0 = the drone pad bed)
    fn space_for(&self, pl: usize) -> Space {
        if self.front_of(pl).is_some() {
            // techno relaxes synth-in-back: never ducked below the back level
            if self.sty().relax { Space::Back } else { Space::Duck }
        } else if self.drums_playing() {
            Space::Back
        } else {
            Space::Stretch
        }
    }
    /// control rate: push the space decision into the pool owners and the pad bed
    fn apply_space(&mut self) {
        let sr = self.sr;
        let coef = |fc: f32| 1.0 - (-std::f32::consts::TAU * fc / sr).exp();
        let surge = self.feel == Feel::Surge;
        let front = 10f32.powf(self.sty().syn_front / 20.0);
        for o in 0..5 {
            let pl = OWNER_PL[o];
            let sp = self.space_for(pl);
            let mut g = SYN_TRIM * SPACE_GAIN[sp as usize];
            if sp != Space::Stretch {
                g *= front;
            }
            if surge {
                g = SYN_TRIM * 1.25; // surges keep their energy
            }
            self.pool.og[o] = g;
            self.pool.oa[o] = match sp {
                Space::Duck if !surge => coef(900.0),
                Space::Back if !surge => coef(2600.0),
                _ => 1.0,
            };
        }
        let sp = self.space_for(0);
        let (g, fc) = match sp {
            Space::Duck => (SYN_TRIM * 0.45, 0.45),
            Space::Back => (SYN_TRIM * 0.8, 0.7),
            Space::Stretch => (SYN_TRIM * 1.4, 1.0),
        };
        let k = 1.0 - (-((4 * CR) as f32) / (sr * 0.4)).exp();
        self.bed_g += (g - self.bed_g) * k;
        self.bed_fc += (fc - self.bed_fc) * k;
    }
    fn has_synth(&self, pl: usize) -> bool {
        self.slots.iter().any(|s| s.on && s.synth.is_some() && Self::player_of(s.role) == pl)
    }
    /// once a bar: the synth players (and the drone) say what the space means to them
    fn space_bar(&mut self) {
        for pl in [0usize, 3, 4, 5] {
            if pl != 0 && !self.has_synth(pl) {
                continue;
            }
            let sp = self.space_for(pl);
            let was = self.space_prev[pl];
            if sp == was {
                continue;
            }
            self.space_prev[pl] = sp;
            let front = self.front_of(pl).map(|j| self.pname(j)).unwrap_or_else(|| "the lead".into());
            let t = match (pl, sp, was) {
                (0, Space::Stretch, _) => "drums are out → letting the drone bloom a little".to_string(),
                (0, Space::Duck, _) => format!("{front} is talking → the drone pulls way back"),
                (0, Space::Back, _) => "drums are in → drone stays low and dark".to_string(),
                (_, Space::Stretch, _) => if self.drum_minimal && self.on[2] { "drums are minimal, my turn to stretch".to_string() } else { "drums are out, my turn to stretch".to_string() },
                (_, Space::Duck, _) => format!("{front} is up front → ducking under it, filter closed"),
                (_, Space::Back, Space::Stretch) => "drums are back → sinking into a soft pad".to_string(),
                (_, Space::Back, _) => format!("{front} is done → back to a quiet pad under the drums"),
            };
            if pl == 0 || self.rmode[pl] != RMode::Rest {
                let st = if sp == Space::Stretch { "stretching" } else { self.minds[pl].status };
                self.think(pl, st, t.clone());
                if sp == Space::Stretch && pl != 0 && self.now >= self.space_said[pl] {
                    self.space_said[pl] = self.now + (self.sr * 40.0) as u64;
                    self.band_say(format!("{}: “{t}”", self.pname(pl)));
                }
            }
        }
    }

    /// The restraint gate every melodic note passes: the player's own rest/hold
    /// decision, the conductor's lay-out and hush, and extra thinning for synth voices.
    fn gate(&mut self, who: usize, vel: i32, steps: f32, synth: bool) -> Option<(i32, f32)> {
        if !(3..=5).contains(&who) || self.quote_owner(who) {
            return Some((vel, steps));
        }
        if self.cond.lay[who] > 0 || self.rmode[who] == RMode::Rest {
            return None;
        }
        if self.rmode[who] == RMode::Hold && steps < 8.0 {
            return None;
        }
        let (mut vel, mut steps) = (vel, steps);
        let h = self.cond.hushv[who];
        if h < 1.0 {
            if steps < 4.0 && self.rng.chance(1.0 - h) {
                return None;
            }
            vel = (vel as f32 * (0.55 + 0.45 * h)) as i32;
        }
        // self-designed synth patches are the easiest to overdo: by default a background
        // pad — fewer, longer notes; only when the drums are out may they do more
        if synth && self.feel != Feel::Surge && !self.quote_owner(who) {
            match self.space_for(who) {
                Space::Duck => {
                    if steps < 8.0 {
                        return None; // lay out under the GM / lead, keep only long tones
                    }
                    vel = (vel as f32 * 0.8) as i32;
                }
                Space::Back => {
                    if steps < 4.0 && self.rng.chance(0.6) {
                        return None;
                    }
                    steps = (steps * 2.0).max(4.0);
                }
                Space::Stretch => {
                    if steps < 3.0 && self.rng.chance(0.15) {
                        return None;
                    }
                    steps *= 1.2;
                }
            }
        }
        Some((vel, steps))
    }
    fn heard(&mut self, i: usize, label: String, vel: f32) {
        self.count(i);
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
        self.track(i, semi);
        if self.ui.is_some() {
            let n = self.note_name(semi);
            self.heard(i, n, vel);
        } else {
            self.count(i);
        }
    }

    /// notes per minute per player in each arrangement state (bench / measurement)
    pub fn density_report(&self) -> String {
        let names = ["drone", "bass", "drums", "keys", "lead", "colour", "guest"];
        let mut out = String::new();
        for (st, sn) in ["chilling", "awakening", "alive", "subsiding"].iter().enumerate() {
            let t = self.st_time[st];
            if t < 10.0 {
                continue;
            }
            let per: Vec<String> = (0..7).map(|i| format!("{} {:.0}", names[i], self.dens[st][i] as f64 * 60.0 / t)).collect();
            let tot: u32 = self.dens[st].iter().sum();
            out.push_str(&format!("density {sn} ({t:.0}s): total {:.0}/min | {}\n", tot as f64 * 60.0 / t, per.join(", ")));
        }
        out
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
        let gm_name = |r: Role, sl: &[Slot]| -> Option<String> { sl.iter().find(|s| s.role == r && s.on).map(|s| s.desc.clone()).filter(|d| !d.is_empty()) };
        let has_gm = self.gm.is_some();
        let inst = [
            "sub drone + detuned pad".to_string(),
            match gm_name(Role::Bass, &self.slots) { Some(g) => format!("synth {} + {g}", BASS_PATCHES[self.bass_patch].0), None => format!("synth {}", BASS_PATCHES[self.bass_patch].0) },
            if has_gm { format!("synth {} + GM {} kit · {}", KICK_PATCHES[self.kick_patch].0, KITS[self.kit].1, PERC_SETS[self.perc_set].0) } else { format!("synth {} + hats", KICK_PATCHES[self.kick_patch].0) },
            match (gm_name(Role::Pad, &self.slots), gm_name(Role::Comp, &self.slots)) {
                (Some(a), Some(b)) => format!("{a} / {b}"),
                (Some(a), None) | (None, Some(a)) => a,
                _ => "—".into(),
            },
            gm_name(Role::Lead, &self.slots).unwrap_or_else(|| format!("synth {}", BELL_PATCHES[self.bell_patch].0)),
            match gm_name(Role::Colour, &self.slots) { Some(g) => format!("{g} + {}", BELL_PATCHES[self.bell_patch].0), None => format!("synth {}", BELL_PATCHES[self.bell_patch].0) },
            if self.guest_on { "saw/sine guest voice".into() } else { "—".into() },
        ];
        let recent = (self.sr * 5.0) as u64;
        let sig = self.ctl.sig;
        let pct = |x: f32| format!("{:.0}%", x * 100.0);
        let tied = [
            if self.ctl.screen_valid { format!("screen {}", if self.ctl.warmth >= 0.0 { "warm" } else { "cool" }) } else { "screen colour".into() },
            format!("disk/files {}", pct(sig.file)),
            format!("cpu {}", pct(sig.cpu)),
            format!("input {}", pct(sig.input)),
            format!("cores {}", pct(sig.cores)),
            format!("net {}", pct(sig.net)),
            if self.guest_on { format!("its thread {}", pct(self.guest_cpu)) } else { "busiest thread".into() },
        ];
        let band = key;
        for i in 0..7 {
            self.minds[i].key = match i {
                2 => "—".into(),
                6 => if self.guest_on { band.name() } else { "—".into() },
                _ => PKey::of(band, self.rel[i]).name(),
            };
            self.minds[i].away = self.rel[i] != Rel::Same && i != 2 && i != 6;
            self.minds[i].instrument = inst[i].clone();
            self.minds[i].tied = tied[i].clone();
            if i < 6 && self.minds[i].traits.is_empty() {
                let p = self.persona[i];
                self.minds[i].traits = format!("curious {:.1} eager {:.1} busy {:.1}", p.curiosity, p.eager, p.busy);
            }
            let now = self.now;
            self.minds[i].addressed = self.cond.target == Some(i) && now.saturating_sub(self.cond.target_t) < (self.sr * 4.0) as u64;
            self.minds[i].notes = self.heard_notes[i].iter().filter(|(t, _)| now.saturating_sub(*t) < recent).map(|x| x.1.clone()).collect();
        }
        self.pub_ver += 1;
        *snap = crate::ui::BandSnap {
            key: key.name(),
            prog,
            bpm: self.bpm,
            bpm_to: (self.ramp_pos < self.ramp_len && (self.bpm_to - self.bpm).abs() >= 0.5).then_some(self.bpm_to),
            bpm_locked: self.bpm_lock,
            feel: match self.feel {
                Feel::Normal => "",
                Feel::Half => "half-time",
                Feel::Double => "double-time",
                Feel::Surge => "surge — V-twin lope",
            },
            home: (self.harm.home() != key).then(|| self.harm.home().name()),
            bar: self.step / 16,
            beat: ((self.step % 16) / 4) as u32,
            state: self.leader.state.name(),
            energy: self.leader.energy,
            agitation: self.leader.agitation,
            minds: self.minds.to_vec(),
            cond: crate::ui::CondSnap {
                pose: self.cond.pose,
                speech: self.cond.speech.clone(),
                target: self.cond.target.filter(|_| self.now.saturating_sub(self.cond.target_t) < (self.sr * 4.0) as u64).map(|i| ["drone", "bass", "drums", "keys", "lead", "colour", "guest"][i]),
                count: self.cond.total(),
                clutter: self.cond.clutter,
            },
            quote: self.quote.as_ref().filter(|q| q.start <= self.step + 16).map(|q| {
                let prog = ((self.step.saturating_sub(q.start)) as f32 / (q.end - q.start).max(1) as f32).clamp(0.0, 1.0);
                (self.pname(q.player), q.cite.clone(), q.what.clone(), prog)
            }),
            cites: self.quote_hist.iter().cloned().collect(),
            levels,
            seed: self.seed,
            time: self.time(),
            version: self.pub_ver,
            style: self.style_name(),
            style_from: self.style_from.map(|k| STYLES[k].name.to_string()),
            tempo_bias: self.tempo_bias(),
            hat: self.sty().hat,
            face: self.sty().face,
            style_id: self.sty().id,
        };
    }

    fn sty(&self) -> &'static Style {
        &STYLES[self.style]
    }

    pub fn steer_stats(&self) -> String {
        format!(
            "{} style switches, {} tempo nudges; now {}, tempo bias {:+.0}, {:.0} bpm; techno sections groove/breakdown/build/drop {:?}; {} blue-note bends",
            self.n_style, self.n_nudge, self.style_name(), self.tempo_bias(), self.bpm, self.n_tsec, self.n_bends
        )
    }

    pub fn style_name(&self) -> String {
        format!("{}{}", if self.style_auto { "auto · " } else { "" }, self.sty().name)
    }

    pub fn tempo_bias(&self) -> f32 {
        self.cli_bias + self.key_bias
    }

    /// style → harmony flavour
    fn style_harmony(&mut self) {
        let st = self.sty();
        self.harm.flavour = match st.harm {
            Harm::Pop => None,
            Harm::Drone => Some(0),
            Harm::Jazz => Some(1),
            Harm::Rock => Some(2),
            Harm::Chorale => Some(3),
            Harm::Modal => Some(4),
            Harm::Blues => Some(5),
            Harm::Country => Some(6),
            Harm::Loop => Some(7),
        };
        self.harm.sevenths = st.harm == Harm::Jazz;
        self.harm.major_bias = st.major;
        self.harm.mod_rate = st.mod_rate;
    }

    /// keyboard steering from the TUI (or SYSJAM_KEYS): drained every step
    fn steer(&mut self) {
        let t = crate::shared::KEY_TEMPO.swap(0, Relaxed);
        if t != 0 {
            self.nudge(t);
        }
        let k = crate::shared::KEY_STYLE.swap(0, Relaxed);
        if k != 0 {
            self.cycle_style(k);
        }
    }

    /// ↑/↓: ±7 bpm of user bias on the living tempo, ramped over a bar.
    fn nudge(&mut self, n: i32) {
        self.n_nudge += 1;
        let old = self.tempo_bias();
        self.key_bias = (self.key_bias + 7.0 * n as f32).clamp(-70.0 - self.cli_bias, 70.0 - self.cli_bias);
        self.bias_last = self.now;
        let d = self.tempo_bias() - old;
        let base = if self.ramp_pos < self.ramp_len { self.bpm_to } else { self.bpm };
        let to = (base + d).clamp(50.0, 160.0).round();
        let sty = self.sty();
        let ack = self.now >= self.bias_ack;
        if (to - base).abs() < 0.5 {
            if ack {
                self.cond_say(if to >= 159.0 { "that's as fast as we go!".into() } else { "that's as slow as we go…".into() }, Pose::Calm, None);
                self.bias_ack = self.now + (self.sr * 1.5) as u64;
            }
            return;
        }
        self.bpm_from = self.bpm;
        self.bpm_to = to;
        self.ramp_len = 16;
        self.ramp_pos = 0;
        self.bias_applied = self.tempo_bias();
        let b = self.tempo_bias();
        self.say(format!("~ you: tempo {} → bias {b:+.0}, {:.0} → {to:.0} bpm over a bar", if d > 0.0 { "up" } else { "down" }, self.bpm));
        if ack {
            let line = if d > 0.0 { sty.faster } else { sty.slower };
            self.cond_say(line.to_string(), if d > 0.0 { Pose::Surge } else { Pose::Calm }, None);
            self.bias_ack = self.now + (self.sr * 1.5) as u64;
        }
        self.hold[2] = 0;
        let st = self.minds[2].status;
        self.think(2, st, format!("conductor: “{}” → taking it to {to:.0} bpm", if d > 0.0 { sty.faster } else { sty.slower }));
        self.hold[2] = self.now + (self.sr * 4.0) as u64;
    }

    /// key bias holds for a minute after the last press, then fades (half-life ~90 s)
    fn bias_decay(&mut self) {
        if self.key_bias == 0.0 || self.now < self.bias_last + (self.sr * 60.0) as u64 {
            return;
        }
        let dt = 16.0 * self.step_len / self.sr;
        self.key_bias *= (-dt / 130.0).exp();
        if self.key_bias.abs() < 0.5 {
            self.key_bias = 0.0;
        }
    }

    /// ←/→: step through auto + the styles
    fn cycle_style(&mut self, n: i32) {
        let ring = STYLES.len() as i32 + 1;
        if self.style_auto {
            // leaving auto: step from the style it is currently playing, never back onto auto
            let mut p = (self.style as i32 + 1 + n).rem_euclid(ring);
            if p == 0 {
                p = (p + n.signum()).rem_euclid(ring);
            }
            self.style_sel = p as usize;
        } else {
            self.style_sel = (self.style_sel as i32 + n).rem_euclid(ring) as usize;
        }
        if self.style_sel == 0 {
            self.style_auto = true;
            self.auto_next = self.now + (self.sr * self.rng.range(90.0, 150.0)) as u64;
            let nm = self.sty().name;
            self.cond_say(format!("conductor's choice — I'll steer from here ({nm} for now)"), Pose::Calm, None);
            self.say(format!("♫ style: auto (the conductor wanders between styles; {nm} for now)"));
            return;
        }
        self.style_auto = false;
        let to = self.style_sel - 1;
        if to == self.style {
            let nm = self.sty().name;
            self.cond_say(format!("{nm} it is — we're already there"), Pose::Calm, None);
            return;
        }
        self.switch_style(to, "you asked");
        self.sw_t0 = Some((self.now, self.style_from.unwrap_or(self.style), to));
    }

    /// measurement: a user switch is done when the tempo sits in the new style's range,
    /// the kit and every playing layer were replaced after the press and the old sounds are gone
    fn switch_settled(&self, t0: u64) -> Result<(), &'static str> {
        let s = self.sty();
        if self.step < self.sw_ramp_end {
            return Err("tempo ramp");
        }
        if self.style_kit || self.swap_at.iter().any(|&x| x > 0) {
            return Err("staggered swaps");
        }
        if self.harm.restarting() {
            return Err("harmony");
        }
        if self.insp[1..6].iter().any(|w| w.as_deref().is_some_and(|w| w.starts_with("conductor went"))) {
            return Err("inspiration pending");
        }
        if !self.bpm_lock && self.feel != Feel::Surge {
            let cur = self.bpm - self.bias_applied;
            if cur < s.tempo[0].0 - 1.0 || cur > s.tempo[2].1 + 1.0 {
                return Err("tempo range");
            }
        }
        if let Some(g) = self.gm.as_ref() {
            if !s.kits.is_empty() && !s.kits.contains(&self.kit) {
                return Err("kit");
            }
            for sl in self.slots.iter().filter(|sl| sl.on) {
                if sl.changed_at < t0 {
                    return Err("layer not swapped");
                }
                if sl.synth.is_none() && sl.program.is_some() && (g.expr(sl.chans[1 - sl.cur]) > 0.01 || g.expr(sl.chans[sl.cur]) < 0.99) {
                    return Err("fade");
                }
            }
        }
        Ok(())
    }

    fn switch_watch(&mut self) {
        let Some((t0, from, to)) = self.sw_t0 else { return };
        if self.style != to {
            self.sw_t0 = None;
            return;
        }
        let secs = (self.now - t0) as f32 / self.sr;
        let r = self.switch_settled(t0);
        if std::env::var_os("SYSJAM_SWDBG").is_some() && self.step % 16 == 0 {
            eprintln!("switch {secs:.1}s: {r:?}");
        }
        if r.is_ok() || secs > 180.0 {
            let tag = format!("{} → {}", STYLES[from].name, STYLES[to].name);
            self.say(format!("~ switch {tag} fully done in {secs:.2} s"));
            self.sw_times.push((tag, secs));
            self.sw_t0 = None;
        }
    }

    fn switch_style(&mut self, to: usize, why: &str) {
        let from = self.style;
        self.n_style += 1;
        self.style = to;
        self.style_from = Some(from);
        self.style_left = 3;
        self.style_harmony();
        let s = self.sty();
        self.cond_say(s.hello.to_string(), Pose::Point, None);
        self.cond.pose_until = self.now + (self.sr * 6.0) as u64;
        self.say(format!("♫ style: {} → {} ({why}) — {}", STYLES[from].name, s.name, s.desc));
        self.pub_now = true;
        if why == "you asked" {
            self.instant_switch();
            return;
        }
        // staggered handover: rhythm section at the next phrase, the rest over 1-2
        for i in 1..6 {
            self.swap_at[i] = if i <= 2 { 1 } else { 1 + self.rng.below(2) as u32 };
            self.hold[i] = 0;
            let st = self.minds[i].status;
            self.think(i, st, format!("conductor went {} → listening, changing over at the phrase", s.name));
            self.hold[i] = self.now + (self.sr * 5.0) as u64;
        }
    }

    /// A user-pressed switch happens now: harmony from the next beat, tempo within a beat,
    /// drum feel + kit + pattern at once, every playing layer swapped with a ~0.1 s fade
    /// (held notes re-struck on the new sound). No cooldowns, dwell or phrase waits.
    fn instant_switch(&mut self) {
        let s = self.sty();
        let name = s.name;
        self.style_left = 0;
        self.swap_at = [0; 6];
        self.sf_clear = self.now + (self.step_len * 16.0) as u64;
        self.fast_until = self.now + (self.sr * 8.0) as u64;
        self.harm.restart();
        self.bars_since_change = self.cur_bpc;
        if self.feel != Feel::Normal {
            self.surge_left = 0;
            self.feel = Feel::Normal;
            self.feel_phrases = 0;
        }
        if !self.bpm_lock {
            let t = s.tempo;
            let (lo, hi) = match self.leader.state {
                State::Chill => t[0],
                State::Awaken => t[1],
                State::Alive => t[2],
                State::Subside => (t[0].0, t[1].0.max(t[0].1)),
            };
            let mood = (0.3 + 0.5 * self.leader.agitation).clamp(0.0, 1.0);
            let to = ((lo + (hi - lo) * mood).round() + self.tempo_bias()).clamp(50.0, 160.0).round();
            self.bias_applied = self.tempo_bias();
            if (to - self.bpm).abs() >= 1.0 {
                self.bpm_from = self.bpm;
                self.bpm_to = to;
                self.ramp_len = 4;
                self.ramp_pos = 0;
                self.sw_ramp_end = self.step + 4;
                self.say(format!("~ tempo {:.0} → {to:.0} bpm within a beat ({name})", self.bpm));
            }
        }
        // rhythm section
        self.acid_sync();
        if s.relax {
            self.tsec = 0;
            self.tsec_left = 4;
            self.tsec_len = 4;
            self.tsec_ph = 0;
        }
        let why = format!("conductor went {name}");
        self.insp[2] = Some(why.clone());
        self.instant_kit();
        self.make_drum_pattern();
        self.bass_gm_ok = s.gm_bass && self.gm.is_some();
        if !self.acid_on && !self.slot_on(Role::Bass) {
            let p = (self.bass_patch + 1 + self.rng.below(BASS_PATCHES.len() - 1)) % BASS_PATCHES.len();
            self.bass_patch = p;
            self.bass_saw_t = BASS_PATCHES[p].1;
            self.bass_drive_t = BASS_PATCHES[p].2;
            self.insp[1] = Some(why.clone());
            self.changed_sound(1, format!("switching to a {} patch", BASS_PATCHES[p].0));
        }
        if !self.slot_on(Role::Colour) {
            let p = (self.bell_patch + 1 + self.rng.below(BELL_PATCHES.len() - 1)) % BELL_PATCHES.len();
            self.bell_patch = p;
        }
        // every playing layer at once
        for i in 0..self.slots.len() {
            if !self.slots[i].on {
                continue;
            }
            let pl = Self::player_of(self.slots[i].role);
            self.insp[pl] = Some(why.clone());
            self.slot_instant(i);
        }
        for i in 1..6 {
            if self.insp[i].as_deref() == Some(why.as_str()) {
                self.insp[i] = None;
            }
        }
    }

    /// the drummer grabs one of the style's kits right now
    fn instant_kit(&mut self) {
        let s = self.sty();
        if self.gm.is_none() || s.kits.is_empty() {
            self.insp[2] = None;
            return;
        }
        let pick: Vec<usize> = s.kits.iter().cloned().filter(|&k| k != self.kit || s.kits.len() == 1).collect();
        self.kit = pick[self.rng.below(pick.len())];
        let pf = s.perc;
        self.perc_set = if !pf.is_empty() { pf[self.rng.below(pf.len())] } else { self.rng.below(PERC_SETS.len()) };
        let (prog, name) = KITS[self.kit];
        if let Some(g) = self.gm.as_mut() {
            g.program(9, prog, 0.9);
        }
        self.gm_used_kits += 1;
        self.style_kit = false;
        let ps = PERC_SETS[self.perc_set].0;
        self.changed_sound(2, format!("switching to the GM {name} kit with {ps}"));
    }

    /// swap one layer's sound now: ~0.1 s fades, held pad notes re-struck on the new sound
    fn slot_instant(&mut self, i: usize) {
        let role = self.slots[i].role;
        let pl = Self::player_of(role);
        let old = self.slots[i].desc.clone();
        let what = if role != Role::Bass && self.want_synth(pl) {
            self.slot_cut(i);
            let d = self.slot_synth(i);
            if role == Role::Pad {
                let left = self.cur_bpc.saturating_sub(self.bars_since_change).max(1);
                self.strike_synth_pad(i, left);
            }
            format!("designing {}", d.trim_start_matches("synth "))
        } else {
            match self.slot_gm_fast(i) {
                Some((p, _)) => {
                    let nm = GM_NAMES[p as usize].to_lowercase();
                    let art = if nm.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
                    format!("picking up {art} {nm}")
                }
                None => return,
            }
        };
        let d = self.slots[i].desc.clone();
        self.say(format!("+ {d}  \u{2212} {old} [{role:?}]"));
        self.changed_sound(pl, what);
    }

    /// fade the slot's current sound out in ~0.1 s (GM: CC11 + note-offs, synth: fast release)
    fn slot_cut(&mut self, i: usize) {
        let now = self.now;
        if self.slots[i].synth.is_some() {
            self.pool.cut_owner(i, now);
            self.slots[i].synth = None;
            return;
        }
        if self.gm.is_none() || self.slots[i].program.is_none() {
            return;
        }
        let chn = self.slots[i].chans[self.slots[i].cur];
        self.cut_channel(chn);
    }

    fn cut_channel(&mut self, chn: usize) {
        if let Some(g) = self.gm.as_mut() {
            g.fade(chn, 0.0, 0.1);
        }
        let at = self.now + (self.sr * 0.13) as u64;
        // everything still sounding on it stops once it is faded (all-notes-off event)
        self.evs.push(Ev { at, ch: chn, key: -1, vel: 0 });
        self.pad_held.retain(|h| h.0 != chn);
    }

    /// GM program onto the pair's other channel right now (hard reset), fading in over 80 ms
    fn slot_gm_fast(&mut self, i: usize) -> Option<(u8, bool)> {
        self.gm.as_ref()?;
        let now = self.now;
        let from_synth = self.slots[i].synth.is_some() || self.slots[i].program.is_none();
        let s = &self.slots[i];
        let old_ch = s.chans[s.cur];
        let other = 1 - s.cur;
        let och = s.chans[other];
        let role = s.role;
        let (p, wild) = self.pick_program(role);
        let held: Vec<(usize, i32)> = if from_synth { Vec::new() } else { self.pad_held.iter().cloned().filter(|h| h.0 == old_ch).collect() };
        if from_synth {
            self.pool.cut_owner(i, now);
            self.slots[i].synth = None;
        }
        // the old channel goes, even if it was still fading from an earlier change
        if !from_synth || self.gm.as_ref().is_some_and(|g| g.expr(old_ch) > 0.0) {
            self.cut_channel(old_ch);
        }
        // drop anything still queued for the channel we take over
        self.evs.retain(|e| !(e.ch == och && e.vel > 0));
        if let Some(g) = self.gm.as_mut() {
            g.all_off(och);
            g.snap(och, 0.0);
            g.program(och, p, prog_gain(p));
            g.fade(och, 1.0, 0.08);
            for &(_, k) in &held {
                g.note_on(och, self.midi_root + k, 44, now);
            }
        }
        self.pad_held.extend(held.iter().map(|&(_, k)| (och, k)));
        let sl = &mut self.slots[i];
        sl.cur = other;
        sl.program = Some(p);
        sl.changed_at = now;
        sl.synth = None;
        sl.desc = format!("GM {} ({})", GM_NAMES[p as usize], p);
        sl.recent.push_back(p);
        if sl.recent.len() > 8 {
            sl.recent.pop_front();
        }
        self.recent_global.push_back(p);
        if self.recent_global.len() > 16 {
            self.recent_global.pop_front();
        }
        self.gm_used[p as usize] += 1;
        Some((p, wild))
    }

    fn has_slot(&self, pl: usize) -> bool {
        self.slots.iter().any(|s| s.on && Self::player_of(s.role) == pl)
    }

    /// phrase boundary: style transitions and auto wandering
    fn style_phrase(&mut self) {
        let name = self.sty().name;
        if self.style_left > 0 {
            self.style_left -= 1;
            if self.style_left == 0 {
                self.style_from = None;
                self.say(format!("~ the band has settled into {name}"));
            }
        }
        for i in 1..6 {
            if self.swap_at[i] == 0 {
                continue;
            }
            self.swap_at[i] -= 1;
            if self.swap_at[i] > 0 {
                continue;
            }
            let why = format!("conductor went {name}");
            self.hold[i] = 0;
            let st = self.minds[i].status;
            match i {
                1 => {
                    let gm_bass = self.sty().gm_bass && self.gm.is_some();
                    self.bass_gm_ok = gm_bass;
                    if gm_bass && !self.slot_on(Role::Bass) {
                        self.think(1, st, format!("{why} → bringing a GM bass in over the deep sub"));
                    } else {
                        self.insp[1] = Some(why);
                        self.dwell[1] = 0;
                    }
                }
                3 | 4 if !self.has_slot(i) => {
                    self.think(i, st, format!("{why} → I'll come in with something from the {name} palette"));
                }
                2 => {
                    self.style_kit = !self.sty().kits.is_empty();
                    self.insp[2] = Some(why);
                    self.dwell[2] = 0;
                    self.make_drum_pattern();
                }
                _ => {
                    self.insp[i] = Some(why);
                    self.dwell[i] = 0;
                }
            }
        }
        self.techno_phrase();
        if self.style_auto && self.style_left == 0 && self.now >= self.auto_next && self.leader.breakdown == 0 {
            let cur = self.style;
            let w: Vec<f32> = (0..STYLES.len()).map(|k| if k == cur { 0.0 } else if k == 0 { 1.5 } else { 1.0 }).collect();
            let to = self.rng.weighted(&w);
            self.auto_next = self.now + (self.sr * self.rng.range(300.0, 540.0)) as u64;
            self.switch_style(to, "auto: the conductor's mood wandered");
        }
    }

    /// Techno arrangement on phrase boundaries: 16/32-bar grooves, breakdowns, builds and
    /// drops, driven by the system's intensity. Also keeps the acid patch in place.
    fn techno_phrase(&mut self) {
        self.acid_sync();
        if !self.sty().relax {
            self.tsec = 0;
            self.tsec_left = 0;
            return;
        }
        self.techno_sections();
    }

    fn acid_sync(&mut self) {
        let acid = self.sty().bass_feel == BassFeel::Acid;
        if acid != self.acid_on {
            self.acid_on = acid;
            if acid {
                self.bass_saw_t = 1.0;
                self.bass_drive_t = 1.7;
                self.make_acid();
            } else {
                self.bass_saw_t = BASS_PATCHES[self.bass_patch].1;
                self.bass_drive_t = BASS_PATCHES[self.bass_patch].2;
            }
        }
    }

    fn techno_sections(&mut self) {
        self.tsec_ph += 1;
        if self.tsec_left > 1 {
            self.tsec_left -= 1;
            if self.tsec == 2 && self.tsec_left == 1 {
                self.cond_say("here it comes…".into(), Pose::Surge, None);
            }
            if self.acid_on && self.rng.chance(0.3) {
                self.make_acid();
            }
            return;
        }
        let e = (0.6 * self.leader.energy + 0.4 * self.leader.agitation).clamp(0.0, 1.0);
        let hot = e > 0.55;
        let (next, len, line): (u8, u32, &str) = match self.tsec {
            0 | 3 if e > 0.3 && self.rng.chance(0.25 + 0.6 * e) => (1, if hot { 2 } else { 4 }, "breakdown — kick out, let it breathe"),
            0 | 3 => (0, if self.rng.chance(0.5) { 4 } else { 8 }, ""),
            1 => (2, if hot { 4 } else { 8 }, "build it up…"),
            _ => (3, if hot { 8 } else { 4 }, "DROP!"),
        };
        let was = self.tsec;
        self.tsec = next;
        self.tsec_left = len;
        self.tsec_len = len;
        self.tsec_ph = 0;
        self.n_tsec[next as usize] += 1;
        self.make_acid();
        let name = ["groove", "breakdown", "build", "drop"][next as usize];
        if next != was || next != 0 {
            self.say(format!("~ techno: {name} for {} bars (intensity {e:.2})", len * 4));
            if !line.is_empty() {
                self.cond_say(line.into(), if next == 3 { Pose::Surge } else { Pose::Point }, None);
            }
            let st = self.minds[2].status;
            self.hold[2] = 0;
            let t = match next {
                1 => "breakdown → kick out, just the pad and the stabs".to_string(),
                2 => format!("build → kick back in, snare roll rising over {} bars", len * 4),
                3 => "the drop → full four-on-the-floor, claps on 2 and 4".to_string(),
                _ => format!("groove → four-on-the-floor for {} bars", len * 4),
            };
            self.think(2, st, t);
            self.hold[2] = self.now + (self.sr * 6.0) as u64;
            self.hold[1] = 0;
            let st = self.minds[1].status;
            let cut = ["filter half open", "filter closed down, dub roots", "filter sweeping open", "filter wide, accents"][next as usize];
            self.think(1, st, format!("acid line: {cut}"));
        }
    }

    /// a new 303-style 16-step line: roots, octaves, b7 / b3 / 5ths, accents (bit 1) and slides (bit 2)
    fn make_acid(&mut self) {
        let mut p = [(-1i8, 0u8); 16];
        let notes = [0i8, 0, 0, 12, 7, 10, 3, 0, 12];
        let dens = match self.tsec {
            1 => 0.2,
            3 => 0.75,
            _ => 0.55,
        };
        for (i, x) in p.iter_mut().enumerate() {
            if i == 0 || self.rng.chance(dens) {
                let n = notes[self.rng.below(notes.len())];
                let mut fl = 0u8;
                if i % 4 == 0 || self.rng.chance(0.2) {
                    fl |= 1;
                }
                if i > 0 && self.rng.chance(0.22) {
                    fl |= 2;
                }
                *x = (if i == 0 { 0 } else { n }, fl);
            }
        }
        self.acid_pat = p;
    }

    /// techno: drone, bass, drums + at most two melodic layers (4-5 layers, no clutter)
    fn layer_cap(&mut self) {
        if !self.sty().relax {
            return;
        }
        let mut n = (3..6).filter(|&i| self.on[i] && self.rmode[i] != RMode::Rest).count();
        for i in [5usize, 4] {
            if n <= 2 {
                break;
            }
            if self.on[i] && self.rmode[i] != RMode::Rest {
                self.rmode[i] = RMode::Rest;
                n -= 1;
                let st = self.minds[i].status;
                self.hold[i] = 0;
                self.think(i, st, "techno keeps it to 4-5 layers → sitting out, the stabs and the acid have it");
            }
        }
    }

    /// style bass lines; true when handled
    fn style_bass(&mut self, s: usize, bar: u64) -> bool {
        let feel = self.sty().bass_feel;
        if feel == BassFeel::Band || (feel != BassFeel::Acid && self.bass_mode == BassMode::Sustain) {
            return false;
        }
        let c = self.chord();
        let root = Self::fold(c.root, -17 + self.reg, -6 + self.reg);
        let third = if c.pcs().contains(&(c.root + 3).rem_euclid(12)) { 3 } else { 4 };
        let change_next = self.bars_since_change + 1 >= self.cur_bpc;
        match feel {
            BassFeel::Walk if s % 4 == 0 => {
                let beat = s / 4;
                let semi = match beat {
                    0 => root,
                    1 => root + if self.rng.chance(0.5) { third } else { 7 },
                    2 => root + if self.rng.chance(0.6) { 7 } else { 12 },
                    _ => {
                        let next = self.harm.peek_next(self.meander());
                        let nr = Self::fold(next.root, -17 + self.reg, -6 + self.reg);
                        if change_next { nr + if self.rng.chance(0.5) { -1 } else { 1 } } else { root + if self.rng.chance(0.5) { 9 } else { 5 } }
                    }
                };
                if beat == 3 && change_next {
                    self.set_think(1, "walking: chromatic approach into the next chord");
                }
                self.heard_semi(1, semi, 0.8);
                self.bass_sound(semi, if beat == 0 { 0.85 } else { 0.72 }, 0.01, 0.32, 3.6);
            }
            BassFeel::Boogie if s % 4 == 0 => {
                let idx = (bar % 2) as usize * 4 + s / 4;
                let deg = [0, third, 7, 9, 10, 9, 7, third][idx];
                let semi = root + deg;
                self.heard_semi(1, semi, 0.8);
                self.bass_sound(semi, if s == 0 { 0.88 } else { 0.74 }, 0.008, 0.28, 3.4);
            }
            BassFeel::BoomChick if s == 0 || s == 8 || (s == 12 && change_next) => {
                let semi = match s {
                    0 => root,
                    8 => root + if bar % 2 == 0 { -5 } else { 7 },
                    _ => root + 2, // walk-up into the change
                };
                self.heard_semi(1, semi, 0.8);
                self.bass_sound(semi, if s == 0 { 0.9 } else { 0.72 }, 0.006, 0.3, 3.0);
            }
            BassFeel::Acid => {
                if self.tsec == 1 {
                    // breakdown: dub roots every other bar
                    if s == 0 && bar % 2 == 0 {
                        self.heard_semi(1, root, 0.7);
                        self.bass_sound(root, 0.7, 0.02, 2.5, 8.0);
                    }
                    return true;
                }
                let (n, fl) = self.acid_pat[s];
                if n < 0 {
                    return true;
                }
                let semi = root + n as i32;
                let acc = fl & 1 != 0;
                if fl & 2 != 0 && self.bass_env > 0.05 {
                    // slide: glide the pitch, no retrigger
                    self.bass_inc = semi_to_freq(self.root, semi as f32) / self.sr;
                } else {
                    self.heard_semi(1, semi, 0.7);
                    self.bass_sound(semi, if acc { 0.85 } else { 0.62 }, 0.003, 0.13, 0.6);
                }
                self.acid_env = if acc { 1.0 } else { 0.55 };
            }
            _ => {}
        }
        true
    }

    /// style drum feels; true when handled
    fn style_drums(&mut self, s: usize, bar: u64) {
        let full = self.rmode[2] == RMode::Play;
        match self.sty().drum_feel {
            DrumFeel::Band => {}
            DrumFeel::Shuffle => {
                // triplet shuffle: the "and" of each beat lands 2/3 of the way through it
                let sh = 0.667;
                let ride = if full { 51 } else { 42 };
                if s % 4 == 0 {
                    self.drum(ride, if full { 30 } else { 20 }, 0.0);
                }
                if s % 4 == 2 {
                    self.drum(ride, if full { 20 } else { 13 }, sh);
                }
                if s == 0 || (full && s == 8) {
                    self.drum(36, if s == 0 { 62 } else { 50 }, 0.0);
                    self.kick(if s == 0 { 0.6 } else { 0.48 });
                }
                if full && s == 10 && self.rng.chance(0.3) {
                    self.drum(36, 34, sh);
                }
                if s == 4 || s == 12 {
                    self.drum(38, if full { 44 } else { 22 }, 0.0);
                }
            }
            DrumFeel::Train => {
                if s == 0 || s == 8 {
                    self.drum(36, if s == 0 { 62 } else { 52 }, 0.0);
                    self.kick(if s == 0 { 0.6 } else { 0.5 });
                }
                if !full {
                    if s == 4 || s == 12 {
                        self.drum(37, 24, 0.0);
                    }
                } else if self.train {
                    // train beat: brushed sixteenths on the snare, accents on 2 and 4
                    let v = match s {
                        4 | 12 => 38,
                        _ if s % 2 == 0 => 18,
                        _ => 12,
                    };
                    self.drum(38, v, 0.0);
                } else {
                    // two-step: snare on 2 and 4, eighth-note hats
                    if s == 4 || s == 12 {
                        self.drum(38, 42, 0.0);
                    }
                    if s % 2 == 0 {
                        self.drum(42, if s % 4 == 0 { 22 } else { 16 }, 0.0);
                    }
                }
            }
            DrumFeel::FourFloor => {
                let sec = self.tsec;
                if sec == 1 {
                    // breakdown: no kick; a soft off-beat hat in its last phrase only
                    if self.tsec_left == 1 && s % 4 == 2 {
                        self.drum(42, 14, 0.0);
                    }
                    return;
                }
                if s % 4 == 0 {
                    // deep synthesized sub kick, ~50 Hz
                    self.kick_with(if full { 1.0 } else { 0.82 }, 50.0, 0.34);
                    self.last_drum = self.now.max(1);
                    if self.ui.is_none() {
                        self.count(2);
                    } else {
                        self.heard(2, "sub kick".into(), 0.9);
                    }
                }
                if s % 4 == 2 {
                    if full {
                        self.drum(46, 28, 0.0);
                    } else {
                        self.drum(42, 16, 0.0);
                    }
                }
                if full && (s == 4 || s == 12) {
                    self.drum(39, 40, 0.0);
                }
                if sec == 3 && s % 2 == 1 {
                    self.drum(42, 10, 0.0);
                }
                if sec == 2 {
                    // build: a snare roll that tightens over the build's final phrase
                    let last = self.tsec_left <= 1;
                    let b = bar % 4;
                    let every = if !last { if s % 8 == 0 { 1 } else { 0 } } else { [4, 2, 1, 1][b as usize] };
                    if every > 0 && s % every == 0 {
                        let ramp = if last { (b as i32 * 16 + s as i32) / 2 } else { 0 };
                        self.drum(38, (16 + ramp).min(60), 0.0);
                    }
                }
            }
        }
    }

    /// style comping (techno dub stabs, country chick, blues shuffle comp); true when handled
    fn style_comp(&mut self, s: usize, bar: u64) -> bool {
        let sty = self.sty();
        let c = self.chord();
        let pcs = c.pcs();
        let base = Self::fold(c.root, 12, 23);
        let mut tones: Vec<i32> = pcs.iter().map(|&pc| base + (pc - c.root).rem_euclid(12)).collect();
        tones.sort();
        let hold = self.rmode[3] == RMode::Hold;
        if sty.relax {
            // dub chord stabs on the off-beat, into the echo
            let hit = s == 6 && (!hold || bar % 2 == 0) || (s == 14 && self.tsec == 3 && self.rng.chance(0.4));
            if hit && self.tsec != 2 || (self.tsec == 2 && s == 6) {
                let v = if hold { 38 } else { 50 };
                for &t in tones.iter().take(4) {
                    self.role_note(Role::Comp, t, v, 0.6, 0.0);
                }
                if s == 6 && bar % 8 == 0 {
                    self.set_think(3, "dub stab on the off-beat → the echo answers");
                }
            }
            return true;
        }
        if hold {
            return false;
        }
        match sty.drum_feel {
            DrumFeel::Train if s == 4 || s == 12 => {
                // the chick: a quick strum on 2 and 4
                for (k, &t) in tones.iter().enumerate() {
                    self.role_note(Role::Comp, t + 12, 42, 1.5, k as f32 * 0.05);
                }
                true
            }
            DrumFeel::Train => true,
            DrumFeel::Shuffle => {
                if s == 4 || s == 12 || (s == 10 && self.rng.chance(0.35)) {
                    let d = if s == 10 { 0.667 } else { 0.0 };
                    for &t in tones.iter() {
                        self.role_note(Role::Comp, t, if s == 10 { 30 } else { 40 }, 1.2, d);
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// pentatonic / pelog styles: snap a melodic note into the five-note scale
    fn scale_snap(&self, semi: i32) -> i32 {
        let k = self.key();
        let set: &[i32] = match self.sty().scale {
            Scale::Diatonic => return semi,
            Scale::Blues if k.minor => &[0, 3, 5, 6, 7, 10],
            Scale::Blues => &[0, 3, 4, 5, 6, 7, 10],
            Scale::Penta if k.minor => &[0, 3, 5, 7, 10],
            Scale::Penta => &[0, 2, 4, 7, 9],
            Scale::Pelog if k.minor => &[0, 1, 3, 7, 8],
            Scale::Pelog => &[0, 2, 4, 7, 9],
        };
        for d in [0, -1, 1, -2, 2] {
            if set.contains(&(semi + d - k.tonic).rem_euclid(12)) {
                return semi + d;
            }
        }
        semi
    }

    /// A player hears something that might make it reach for a new sound.
    fn inspire(&mut self, i: usize, why: String, p: f32) {
        if !(1..6).contains(&i) || self.insp[i].is_some() {
            return;
        }
        // just changed sound: not ready to be inspired again yet
        if self.now < self.since[i] + (self.sr * 15.0) as u64 {
            return;
        }
        if self.rng.chance(p * (0.5 + self.persona[i].curiosity)) {
            self.insp[i] = Some(why);
        }
    }

    /// Time on the same sound slowly turns into boredom (curious players get bored sooner).
    fn boredom(&mut self) {
        for i in 1..6 {
            if self.insp[i].is_some() {
                continue;
            }
            let age = (self.now.saturating_sub(self.since[i])) as f32 / self.sr;
            if age < 18.0 {
                continue;
            }
            let p = (age - 18.0) / (600.0 * (1.2 - self.persona[i].curiosity));
            if self.rng.chance(p.min(0.5)) {
                self.insp[i] = Some(format!("bored after {age:.0}s on the same sound"));
            }
        }
    }

    /// inspired, and the minimum dwell on the current sound has passed
    fn ready(&self, i: usize) -> bool {
        self.insp[i].is_some() && self.now >= self.since[i] + self.dwell[i]
    }

    fn changed_sound(&mut self, i: usize, what: String) {
        let why = self.insp[i].take().unwrap_or_else(|| "felt like it".into());
        if why.starts_with("conductor went") {
            let who = self.pname(i);
            self.say(format!("~ {who}: {why} → {what}"));
        }
        self.since[i] = self.now;
        self.dwell[i] = (self.sr * self.rng.range(8.0, 10.0)) as u64;
        let st = self.minds[i].status;
        self.hold[i] = 0;
        self.think(i, st, format!("{why} → {what}"));
        // keep the reason on screen for a while
        self.hold[i] = self.now + (self.sr * 10.0) as u64;
    }

    /// Oscillator players design new patches when inspired (applied click-free).
    fn patch_manage(&mut self) {
        let gm_bass = self.slot_on(Role::Bass);
        if self.ready(1) && !gm_bass && self.gm.is_some() && !self.rng.chance(self.persona[1].synth) {
            // reach for a GM bass layer (it joins when the groove comes)
            self.bass_gm_ok = true;
            self.changed_sound(1, "reaching for a GM bass to layer on the groove".into());
        }
        if self.ready(1) && !gm_bass {
            self.bass_gm_ok = self.gm.is_some() && self.rng.chance(0.3);
            let p = (self.bass_patch + 1 + self.rng.below(BASS_PATCHES.len() - 1)) % BASS_PATCHES.len();
            self.bass_patch = p;
            self.bass_saw_t = BASS_PATCHES[p].1;
            self.bass_drive_t = BASS_PATCHES[p].2;
            self.changed_sound(1, format!("designing a new patch: {}", BASS_PATCHES[p].0));
            self.say(format!("~ bassist designs a new patch: {}", BASS_PATCHES[p].0));
        }
        let forced = std::mem::take(&mut self.style_kit) && self.ready(2);
        if self.ready(2) && self.gm.is_some() && (forced || !self.rng.chance(self.persona[2].synth * self.sty().synth.min(1.5))) {
            // GM kit variation + a percussion colour set
            let st = self.leader.state;
            let w: Vec<f32> = KITS
                .iter()
                .enumerate()
                .map(|(k, kit)| {
                    let base = match (st, kit.1) {
                        (State::Chill | State::Subside, "Brush" | "Jazz" | "Room" | "Orchestra") => 1.5,
                        (State::Alive, "Power" | "Electronic" | "TR-808" | "Standard") => 1.5,
                        (State::Awaken, "Room" | "Standard" | "TR-808" | "Jazz") => 1.2,
                        _ => 0.3,
                    };
                    let fav = self.sty().kits.contains(&k);
                    if k == self.kit && !(forced && fav) { 0.0 } else if fav { f32::max(base, 0.5) * 10.0 } else { base }
                })
                .collect();
            self.kit = self.rng.weighted(&w);
            let pf = self.sty().perc;
            self.perc_set = if !pf.is_empty() && self.rng.chance(0.8) { pf[self.rng.below(pf.len())] } else { self.rng.below(PERC_SETS.len()) };
            let (prog, name) = KITS[self.kit];
            if let Some(g) = self.gm.as_mut() {
                g.program(9, prog, 0.9);
            }
            self.gm_used_kits += 1;
            let ps = PERC_SETS[self.perc_set].0;
            self.changed_sound(2, format!("switching to the GM {name} kit with {ps}"));
            self.say(format!("~ drummer picks up the GM {name} kit + {ps}"));
        }
        if self.ready(2) {
            let p = (self.kick_patch + 1 + self.rng.below(KICK_PATCHES.len() - 1)) % KICK_PATCHES.len();
            self.kick_patch = p;
            self.changed_sound(2, format!("retuning the kit: {}", KICK_PATCHES[p].0));
            self.say(format!("~ drummer retunes: {}", KICK_PATCHES[p].0));
        }
        if self.ready(5) && !self.slot_on(Role::Colour) {
            let p = (self.bell_patch + 1 + self.rng.below(BELL_PATCHES.len() - 1)) % BELL_PATCHES.len();
            self.bell_patch = p;
            self.changed_sound(5, format!("designing a new patch: {}", BELL_PATCHES[p].0));
            self.say(format!("~ colour player designs a new patch: {}", BELL_PATCHES[p].0));
        }
    }

    fn player_of(r: Role) -> usize {
        match r {
            Role::Bass => 1,
            Role::Pad | Role::Comp => 3,
            Role::Lead => 4,
            Role::Colour => 5,
        }
    }

    fn slot_on(&self, r: Role) -> bool {
        self.slots.iter().any(|s| s.role == r && s.on)
    }

    /// Bandleader's tempo: follows the arrangement and mood, only on phrase
    /// boundaries, always as a smooth ramp over 2-8 bars. Also feel moves.
    fn rev(&mut self, up: bool) {
        self.rev_on = true;
        self.rev_up = up;
        self.rev_t = 0.0;
    }

    /// Rare surge episodes: the tempo opens up like a throttle and the drummer and
    /// bassist drop into a lopsided V-twin "potato-potato" lope. True while surging.
    fn surge_phrase(&mut self) -> bool {
        if self.feel == Feel::Surge {
            self.surge_left = self.surge_left.saturating_sub(1);
            if self.surge_left > 0 {
                if self.rng.chance(0.6) {
                    self.rev(true);
                }
                self.hold[2] = 0;
                self.hold[1] = 0;
                self.think(2, "soloing", format!("V-twin lope at {:.0} bpm: long-short low kicks + floor toms, {} phrase{} to go", self.bpm, self.surge_left, if self.surge_left == 1 { "" } else { "s" }));
                self.think(1, "following", "deep root thuds on the lope, long-short");
                self.pin_surge();
                return true;
            }
            self.feel = Feel::Normal;
            self.surge_cool = self.now + (self.sr * 150.0) as u64;
            self.bass_saw_t = BASS_PATCHES[self.bass_patch].1;
            self.bass_drive_t = BASS_PATCHES[self.bass_patch].2;
            self.rev(false);
            if !self.bpm_lock {
                let t = self.sty().tempo;
                let (lo, hi) = match self.leader.state {
                    State::Chill => t[0],
                    State::Awaken => t[1],
                    State::Alive => t[2],
                    State::Subside => (t[0].0 + 6.0, t[1].1 - 6.0),
                };
                let to = ((lo + hi) * 0.5f32 + self.tempo_bias()).clamp(50.0, 160.0).round();
                self.bias_applied = self.tempo_bias();
                self.bpm_from = self.bpm;
                self.bpm_to = to;
                self.ramp_len = 6 * 16;
                self.ramp_pos = 0;
                self.say(format!("~ surge winds down: {:.0} → {to:.0} bpm over 6 bars", self.bpm));
            } else {
                self.say("~ surge winds down".into());
            }
            self.make_drum_pattern();
            self.hold[1] = 0;
            self.hold[2] = 0;
            self.think(2, "following", "easing off the throttle → back to the band's groove");
            self.think(1, "following", "letting the lope go, back to the changes");
            return false;
        }
        if self.step < 16 * 4 * 3 || self.now < self.surge_cool || self.leader.breakdown > 0 {
            return false;
        }
        let kb = self.ctl.sig.input;
        let st = self.leader.state;
        // never out of a calm band, unless the typing is racing
        if matches!(st, State::Chill | State::Subside) && kb <= 0.6 && !self.surge_test {
            return false;
        }
        let mut p = 0.012;
        if st == State::Alive {
            p += 0.05;
        }
        if kb > 0.6 {
            p += 0.25;
        }
        if self.leader.chaos {
            p += 0.03;
        }
        p *= self.sty().surge;
        if self.surge_test {
            p = 1.0;
        }
        if !self.rng.chance(p) {
            return false;
        }
        let why = if kb > 0.6 { "the typing is racing" } else if st == State::Alive { "the band is running hot" } else { "felt like opening the throttle" };
        self.feel = Feel::Surge;
        self.surge_left = 2 + self.rng.below(3) as u32;
        self.on[1] = true;
        self.on[2] = true;
        self.entry_wait = 0;
        self.bass_saw_t = 0.45;
        self.bass_drive_t = 2.0;
        self.rev(true);
        let to = if self.bpm_lock { self.bpm } else { (self.rng.range(120.0, 150.0) + self.tempo_bias() * 0.5).clamp(60.0, 160.0).round() };
        if !self.bpm_lock {
            self.bpm_from = self.bpm;
            self.bpm_to = to;
            self.ramp_len = 2 * 16;
            self.ramp_pos = 0;
        }
        self.say(format!("~ SURGE: {why} → tempo {:.0} → {to:.0} bpm, drummer and bassist drop into a V-twin lope for {} phrases", self.bpm, self.surge_left));
        self.hold[2] = 0;
        self.hold[1] = 0;
        self.think(2, "soloing", format!("{why} → opening the throttle to {to:.0} bpm: V-twin lope, long-short low kicks + floor toms"));
        self.think(1, "following", "locking to the lope: deep root thuds, long-short, growl patch");
        self.pin_surge();
        true
    }

    /// keep the drummer's / bassist's surge thoughts on screen for the phrase
    fn pin_surge(&mut self) {
        let until = self.now + (self.sr * 8.0) as u64;
        self.hold[1] = until;
        self.hold[2] = until;
    }

    fn tempo_phrase(&mut self, changed: bool) {
        if self.surge_phrase() {
            return;
        }
        // feel (half / double time) — allowed even with a locked tempo
        if self.feel_phrases > 0 {
            self.feel_phrases -= 1;
            if self.feel_phrases == 0 && self.feel != Feel::Normal {
                self.feel = Feel::Normal;
                self.say("~ drummer goes back to the straight feel".into());
                self.think(2, "following", "back to the straight feel");
            }
        } else if self.on[2] && self.leader.breakdown == 0 {
            let st = self.leader.state;
            let p = if st == State::Alive { 0.12 } else { 0.07 };
            if self.rng.chance(p) {
                let f = if st == State::Alive && self.bpm < 100.0 && self.rng.chance(0.6) { Feel::Double } else { Feel::Half };
                self.feel = f;
                self.feel_phrases = 1 + self.rng.below(2) as u32;
                let n = if f == Feel::Half { "half-time" } else { "double-time" };
                self.say(format!("~ drummer switches to a {n} feel"));
                self.think(2, "following", format!("{n} feel for a phrase or two"));
            }
        }
        if self.ramp_pos < self.ramp_len {
            return;
        }
        let bias = self.tempo_bias();
        if self.bpm_lock {
            // --bpm: the living tempo is fixed, the user's bias still rides on it
            let to = (self.lock_bpm + bias).clamp(50.0, 160.0).round();
            if (to - self.bpm).abs() >= 1.0 {
                self.bpm_from = self.bpm;
                self.bpm_to = to;
                self.ramp_len = 2 * 16;
                self.ramp_pos = 0;
                self.say(format!("~ tempo {:.0} → {to:.0} bpm (locked {:.0}, bias {bias:+.0})", self.bpm, self.lock_bpm));
            }
            self.bias_applied = bias;
            return;
        }
        let st = self.leader.state;
        let sty = self.sty();
        // the living tempo, without the user's bias
        let cur = self.bpm - self.bias_applied;
        let t = sty.tempo;
        let (lo, hi) = match st {
            State::Chill => t[0],
            State::Awaken => t[1],
            State::Alive => t[2],
            State::Subside => ((cur - 14.0).max(t[0].0), (cur - 4.0).max(t[0].0 + 2.0)),
        };
        let outside = cur < lo - 1.0 || cur > hi + 1.0;
        let drift = (bias - self.bias_applied).abs() >= 3.0;
        // let the run's random starting tempo breathe for a couple of phrases
        if self.step < 16 * 4 * 2 && !changed && !drift {
            return;
        }
        if !(changed || outside || drift || self.rng.chance(0.25 * sty.tempo_rate)) {
            return;
        }
        let mood = (0.2 + 0.5 * self.leader.agitation + 0.3 * self.rng.f()).clamp(0.0, 1.0);
        let mut to = (lo + (hi - lo) * mood).round();
        // never more than ~24 bpm in one ramp
        to = to.clamp(cur - 24.0, cur + 24.0);
        to = (to + bias).clamp(50.0, 160.0).round();
        self.bias_applied = bias;
        let d = (to - self.bpm).abs();
        if d < 3.0 {
            return;
        }
        let bars = (2.0 + d / 4.0).clamp(2.0, 8.0).round() as u32;
        self.bpm_from = self.bpm;
        self.bpm_to = to;
        self.ramp_len = bars * 16;
        self.ramp_pos = 0;
        let tag = if bias.abs() >= 0.5 { format!(", bias {bias:+.0}") } else { String::new() };
        self.say(format!("~ tempo {:.0} → {:.0} bpm over {bars} bars ({}, {}{tag})", self.bpm, to, st.name(), sty.name));
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
        // each player's tied signal: a sharp rise can inspire a new sound
        let (a, b) = (self.prev_sig, c.sig);
        let spikes = [
            (1, a.file, b.file, "disk/file activity spiked"),
            (2, a.cpu, b.cpu, "the CPU spiked"),
            (3, a.input, b.input, "someone started typing"),
            (4, a.cores, b.cores, "more cores woke up"),
            (5, a.net, b.net, "a network burst"),
        ];
        for (i, x0, x1, why) in spikes {
            if x0 < 0.45 && x1 >= 0.6 {
                self.inspire(i, format!("inspired by {why}"), 0.6);
            }
        }
        self.prev_sig = c.sig;
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
                if !self.guest_on {
                    let n = l.name.clone();
                    self.inspire(5, format!("thread {n} sat in as a guest"), 0.4);
                    self.inspire(4, format!("thread {n} sat in as a guest"), 0.25);
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
        if s.on && s.synth.is_none() && s.program.is_some() { Some(s.chans[s.cur]) } else { None }
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
        let Some(si) = self.slots.iter().position(|s| s.role == r) else { return };
        if !self.slots[si].on {
            return;
        }
        let who = Self::player_of(r);
        let synth = self.slots[si].synth.is_some();
        let Some((vel, steps)) = self.gate(who, vel, steps, synth) else { return };
        // the bass anchors the band key; everybody else plays in its own key
        let semi = if r == Role::Bass { semi } else { self.in_key(who, semi, vel) };
        if let Some(p) = self.slots[si].synth {
            let jitter = (self.rng.f() * 0.012 * self.sr) as u64;
            let at = self.now + (delay.max(0.0) * self.step_len) as u64 + jitter;
            let dur = (steps.max(0.25) * self.step_len) as u64;
            let pan = [-0.45, 0.35, 0.15, 0.6, 0.0][si % 5] + self.rng.bi() * 0.15;
            let f = semi_to_freq(self.root, semi as f32);
            let v = (vel as f32 / 110.0).clamp(0.05, 1.0);
            self.pool.note(si, &p, f, v, at, dur, pan);
            self.mel_until[who] = self.mel_until[who].max(at + dur);
        } else if let Some(ch) = self.slot_ch(r) {
            self.note(ch, semi, vel, steps, delay);
            // blues: scoop up into the blue notes (b3, b5) with a pitch bend
            if matches!(r, Role::Lead | Role::Colour) && self.sty().scale == Scale::Blues {
                let pc = (semi - self.key().tonic).rem_euclid(12);
                if (pc == 3 || pc == 6) && self.rng.chance(0.7) {
                    let at = self.now + (delay.max(0.0) * self.step_len) as u64;
                    let span = (self.step_len * 0.9) as u64;
                    self.n_bends += 1;
                    // ±2 semitone range: start a semitone flat, glide up over ~a step
                    for k in 0..=4u64 {
                        let v = 4096 + (4096 * k / 4) as i32;
                        self.evs.push(Ev { at: at + span * k / 4, ch, key: 0, vel: -(v + 1) });
                    }
                }
            }
            let end = self.now + ((delay.max(0.0) + steps.max(0.25)) * self.step_len) as u64;
            self.gm_until[who] = self.gm_until[who].max(end);
            self.mel_until[who] = self.mel_until[who].max(end);
        } else {
            return;
        }
        self.heard_semi(who, semi, vel as f32 / 100.0);
    }

    /// Map a band-key pitch into player `i`'s own key (transposed if bitonal,
    /// then snapped to its mode) and listen for roughness against bass and pad.
    fn in_key(&mut self, i: usize, semi: i32, vel: i32) -> i32 {
        // five-note / blues scales shape the melody players; chords stay intact
        let semi = if i >= 4 { self.scale_snap(semi) } else { semi };
        let rel = self.rel[i];
        if rel == Rel::Same {
            return semi;
        }
        let band = self.key();
        let k = PKey::of(band, rel);
        let mut s = semi;
        if rel.bitonal() {
            let mut off = (k.tonic - band.tonic).rem_euclid(12);
            if off > 6 {
                off -= 12;
            }
            s += off;
        }
        if !k.contains(s) {
            if k.contains(s + 1) {
                s += 1;
            } else if k.contains(s - 1) {
                s -= 1;
            }
        }
        let bass = self.chord().root;
        let d = (s - bass).rem_euclid(12);
        let mut r = if d == 1 || d == 6 { 1.0 } else { 0.0 };
        for &pc in &self.pad_pcs {
            let dd = (s - pc).rem_euclid(12);
            if dd == 1 || dd == 11 {
                r += 0.5;
            }
        }
        self.rough[i] += r * (vel as f32 / 90.0).clamp(0.2, 1.2);
        s
    }

    /// Phrase-level key decisions: wander to a related key, or come home.
    fn key_phrase(&mut self) {
        let band = self.key();
        let st = self.leader.state;
        let chaos = self.leader.chaos;
        for i in [3usize, 4, 5] {
            if !self.on[i] {
                if self.rel[i] != Rel::Same {
                    self.rel[i] = Rel::Same;
                }
                continue;
            }
            let adv = self.persona[i].adventure;
            if self.rel[i] != Rel::Same {
                // phrase end: most players come back home
                let p = match st {
                    State::Chill => 0.6,
                    State::Subside => 0.55,
                    State::Awaken => 0.4,
                    State::Alive => 0.3,
                } * (1.2 - adv * 0.5);
                if self.rng.chance(p) {
                    let was = PKey::of(band, self.rel[i]).name();
                    self.rel[i] = Rel::Same;
                    self.think(i, self.minds[i].status, format!("phrase end → leaving {was}, back with the band in {}", band.name()));
                    self.say(format!("~ {} comes home to {}", ["", "", "", "keys", "lead", "colour"][i], band.name()));
                }
                continue;
            }
            let base = match st {
                State::Chill => 0.04,
                State::Awaken => 0.14,
                State::Alive => 0.28,
                State::Subside => 0.07,
            } + if chaos { 0.15 } else { 0.0 };
            if !self.rng.chance(base * (0.3 + adv)) {
                continue;
            }
            let bit = if adv > 0.6 && (st == State::Alive || chaos) { 0.35 } else { 0.0 };
            let opts = [
                (Rel::Relative, 0.2),
                (Rel::Parallel, 0.15),
                (Rel::FifthUp, 0.15),
                (Rel::FifthDown, 0.1),
                (Rel::Brighter, 0.2),
                (Rel::Darker, 0.12),
                (Rel::FifthDorian, 0.12),
                (Rel::Tritone, bit * 0.5),
                (Rel::MajThird, bit * 0.5),
            ];
            let opts: Vec<(Rel, f32)> = if self.style == 0 { opts.to_vec() } else { self.sty().rels.to_vec() };
            let w: Vec<f32> = opts.iter().map(|o| o.1).collect();
            let rel = opts[self.rng.weighted(&w)].0;
            self.rel[i] = rel;
            self.rough[i] = 0.0;
            let k = PKey::of(band, rel).name();
            let why = if rel.bitonal() { "feeling bold" } else { "looking for colour" };
            self.think(i, self.minds[i].status, format!("drifting to {k} over the band's {} ({}, {why})", band.name(), rel.describe()));
            self.say(format!("~ {} drifts to {k} over {}", ["", "", "", "keys", "lead", "colour"][i], band.name()));
        }
    }

    /// Every bar: a player hearing too much roughness against bass/pad steers back.
    fn key_listen(&mut self) {
        let band = self.key();
        for i in [3usize, 4, 5] {
            if self.rel[i] != Rel::Same {
                let tol = 1.5 + 4.0 * self.persona[i].adventure;
                if self.rough[i] > tol {
                    let was = PKey::of(band, self.rel[i]).name();
                    self.rel[i] = Rel::Same;
                    self.hold[i] = 0;
                    self.think(i, self.minds[i].status, format!("heard roughness against the bass/pad in {was} → steering back to {}", band.name()));
                    self.say(format!("~ {} steers back to {} (too rough)", ["", "", "", "keys", "lead", "colour"][i], band.name()));
                }
            }
            self.rough[i] *= 0.5;
        }
    }

    /// cadence: everyone converges on the band key
    fn key_converge(&mut self, why: &str) {
        let band = self.key();
        for i in [3usize, 4, 5] {
            if self.rel[i] != Rel::Same && self.rng.chance(0.85) {
                self.rel[i] = Rel::Same;
                self.hold[i] = 0;
                self.think(i, self.minds[i].status, format!("converging on {} for the {why}", band.name()));
            }
        }
    }
    fn drum(&mut self, key: i32, vel: i32, delay: f32) {
        self.last_drum = self.now.max(1);
        if self.ui.is_none() {
            self.count(2);
        } else {
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
                69 => "cabasa",
                60 | 61 => "bongo",
                62..=64 => "conga",
                65 | 66 => "timbale",
                67 | 68 => "agogo",
                75 => "claves",
                76 | 77 => "woodblock",
                78 | 79 => "cuica",
                80 | 81 => "triangle",
                53 => "ride bell",
                54 => "tambourine",
                56 => "cowbell",
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
        self.steer();
        self.switch_watch();
        if self.ramp_pos < self.ramp_len {
            // smooth tempo ramp, one tiny change per step (the clock never jumps)
            self.ramp_pos += 1;
            let x = self.ramp_pos as f32 / self.ramp_len as f32;
            let sm = x * x * (3.0 - 2.0 * x);
            self.bpm = self.bpm_from + (self.bpm_to - self.bpm_from) * sm;
            self.step_len = self.sr * 60.0 / self.bpm / 4.0;
            self.dlen_target = ((self.step_len * 3.0) as usize).clamp(1, self.dl.len() - 1);
        }
        let s = (self.step % 16) as usize;
        let bar = self.step / 16;
        self.step += 1;
        if s == 0 {
            if self.sf_clear > 0 && self.now >= self.sf_clear {
                self.sf_clear = 0;
                self.style_from = None;
                let n = self.sty().name;
                self.say(format!("~ the band has switched to {n}"));
            }
            self.leader.on_bar();
            self.bias_decay();
            self.msg_budget = (self.msg_budget + 0.7).min(3.0);
            if bar % 4 == 0 {
                self.on_phrase();
            }
            self.on_bar_line(bar);
            self.conductor_bar(bar);
            self.space_bar();
        }
        // a user style switch lands on the next beat: new layers and the new harmony
        if s % 4 == 0 && s != 0 && self.harm.restarting() {
            self.gm_manage();
            self.harmony_bar();
        }
        self.quote_step(bar * 16 + s as u64);
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
        let ind = self.sty().industrial;
        let st = self.leader.state;
        let deliberate = (std::mem::take(&mut self.leader.industrial_move) && st == State::Alive && self.leader.agitation > 0.4)
            || (ind >= 2.0 && matches!(st, State::Awaken | State::Alive) && self.rng.chance(0.03));
        if ind > 0.0 && (self.leader.chaos || deliberate) {
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
            self.inspire(1, "heard the industrial noise".into(), 0.3);
            self.inspire(2, "heard the industrial noise".into(), 0.15);
            let why = if self.leader.chaos { "the machine sounds chaotic" } else { "a deliberate colour move" };
            self.think(5, "soloing", format!("{why} → gated industrial noise for {bars} bar{}", if bars == 1 { "" } else { "s" }));
        }
    }

    fn meander(&self) -> bool {
        matches!(self.leader.state, State::Chill)
    }

    fn on_phrase(&mut self) {
        let before = self.leader.state;
        {
            let c = &mut self.cond;
            for i in 0..7 {
                c.lay[i] = c.lay[i].saturating_sub(1);
                if c.hush_ph[i] > 0 {
                    c.hush_ph[i] -= 1;
                    if c.hush_ph[i] == 0 {
                        c.hushv[i] = 1.0;
                    }
                }
                c.offenses[i] *= 0.7;
            }
            c.drone_cue = c.drone_cue.saturating_sub(1);
        }
        self.style_phrase();
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
        self.tempo_phrase(changed);
        self.key_phrase();
        if changed {
            let st = self.leader.state.name();
            for i in 1..6 {
                self.inspire(i, format!("inspired by the move to {st}"), 0.55);
            }
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
        // techno: kick and acid bass never leave (the arrangement does the dynamics)
        if self.sty().relax {
            self.on[1] = true;
            self.on[2] = true;
        }
        // bassist mode (always present, never silent)
        let inv = self.leader.invite(P::Bass) + (self.persona[1].eager - 0.5) * 0.1;
        let mode = if self.cond.drone_cue > 0 || inv < 0.45 { BassMode::Sustain } else if inv < 0.8 { BassMode::Pulse } else { BassMode::Groove };
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
        self.drums_phrase();
        self.make_drum_pattern();
        // keys comping style for this phrase
        let w: &[f32; 5] = match self.leader.state {
            State::Chill => &[0.9, 0.0, 0.0, 0.1, 0.0],
            State::Awaken => &[0.55, 0.25, 0.0, 0.15, 0.05],
            State::Alive => &[0.25, 0.3, 0.1, 0.15, 0.2],
            State::Subside => &[0.75, 0.15, 0.0, 0.1, 0.0],
        };
        self.comp = [Comp::Sparse, Comp::Arp, Comp::ArpUD, Comp::Broken, Comp::Offbeat][self.rng.weighted(w)];
        // drummer fills before phrase ends sometimes
        let pf = match self.leader.state {
            State::Alive => 0.15,
            State::Awaken => 0.04,
            _ => 0.0,
        };
        if self.on[2] && self.rmode[2] == RMode::Play && self.rng.chance(pf) && self.cond.lay[2] == 0 {
            self.fill = true;
        }
        self.phrase_minds();
        self.restraint_phrase();
        self.layer_cap();
        if let Some(q) = self.quote.as_ref() {
            let pl = q.player;
            self.rmode[pl] = RMode::Play; // keep the quoting player on
        }
        let bar = self.step / 16;
        self.quote_phrase(bar);
    }

    fn pname(&self, i: usize) -> String {
        let n = ["drone", "bass", "drums", "keys", "lead", "colour", "guest"][i];
        let synth = self.slots.iter().any(|s| s.on && s.synth.is_some() && Self::player_of(s.role) == i);
        if synth && (3..6).contains(&i) { format!("{n} (synth)") } else { n.to_string() }
    }

    /// melodic notes per bar (keys + lead + colour) the conductor considers tidy
    fn melodic_target(&self) -> f32 {
        if self.feel == Feel::Surge {
            return 5.0;
        }
        self.sty().density
            * match self.leader.state {
                State::Chill => 1.5,
                State::Awaken => 4.0,
                State::Alive => 8.0,
                State::Subside => 2.5,
            }
    }

    /// Each melodic player asks itself: do I have something useful to add?
    /// Play, hold one long tone, or rest for this phrase.
    fn restraint_phrase(&mut self) {
        let st = self.leader.state;
        let surge = self.feel == Feel::Surge;
        let target = self.melodic_target();
        let m: f32 = self.cond.dens[3] + self.cond.dens[4] + self.cond.dens[5];
        for i in 3..6 {
            let p = [P::Keys, P::Lead, P::Colour][i - 3];
            let featured = self.leader.soloist == Some(p) || self.leader.lone == Some(p);
            let mode;
            let mut why = String::new();
            if !self.on[i] {
                mode = RMode::Rest; // the bandleader already has it out; keep its own line
            } else if self.cond.lay[i] > 0 {
                mode = RMode::Rest;
                let n = self.cond.lay[i];
                why = format!("conductor said lay out → resting, {n} phrase{} to go", if n == 1 { "" } else { "s" });
            } else if featured {
                mode = RMode::Play;
            } else {
                let tied = self.tied_level(i);
                let sig_new = ((tied - self.prev_tied[i]).abs() * 3.0).min(0.4);
                self.prev_tied[i] = tied;
                let fresh = if self.now.saturating_sub(self.since[i]) < (self.sr * 12.0) as u64 { 0.12 } else { 0.0 };
                let fatigue = 0.07 * self.play_run[i].saturating_sub(2) as f32;
                let crowd = (((m - self.cond.dens[i]) / target) - 0.6).max(0.0) * 0.5;
                let redundant = match i {
                    3 if self.on[4] && self.rmode[4] == RMode::Play => 0.08,
                    5 if self.rmode[3] == RMode::Play && self.rmode[4] == RMode::Play => 0.12,
                    _ => 0.0,
                };
                let base = match st {
                    State::Chill => -0.42,
                    State::Awaken => -0.12,
                    State::Alive => 0.12,
                    State::Subside => -0.3,
                } - if surge { 0.1 } else { 0.0 }
                    + 0.4 * (self.sty().density - 1.0);
                let pe = self.persona[i];
                let want = base + 0.45 * tied + sig_new + fresh + 0.25 * self.leader.invite(p) + 0.2 * (pe.eager - 0.5) - fatigue - crowd - redundant + self.rng.bi() * 0.08;
                mode = if want > 0.1 { RMode::Play } else if want > -0.22 { RMode::Hold } else { RMode::Rest };
                why = match mode {
                    RMode::Rest if crowd > 0.15 => format!("band is busy enough ({m:.0} notes/bar) → nothing useful to add, resting"),
                    RMode::Rest if fatigue > 0.1 => format!("played {} phrases straight → giving it a rest", self.play_run[i]),
                    RMode::Rest if st == State::Chill => "chilling — the drone and bass have it, resting".into(),
                    RMode::Rest => "nothing new to say → resting".into(),
                    RMode::Hold => match i {
                        3 => "holding one long chord tone under the drone".into(),
                        4 => "one long note at the top of the phrase, then space".into(),
                        _ => "just a slow swell now and then".into(),
                    },
                    RMode::Play if tied > 0.45 => format!("my signal is up ({}) → worth playing", self.ctl_tied_name(i)),
                    RMode::Play if fresh > 0.0 => "new sound to show off → playing".into(),
                    RMode::Play if sig_new > 0.15 => "something changed in my signal → playing".into(),
                    RMode::Play => String::new(),
                };
            }
            match mode {
                RMode::Rest => {
                    self.rest_run[i] += 1;
                    self.play_run[i] = 0;
                }
                _ => {
                    self.play_run[i] += 1;
                    self.rest_run[i] = 0;
                }
            }
            self.rmode[i] = mode;
            if self.on[i] && !why.is_empty() {
                let status = match mode {
                    RMode::Rest => "laying out",
                    RMode::Hold => "holding",
                    RMode::Play => self.minds[i].status,
                };
                self.think(i, status, why);
            }
        }
    }

    fn ctl_tied_name(&self, i: usize) -> String {
        let g = self.ctl.sig;
        match i {
            3 => format!("input {:.0}%", g.input * 100.0),
            4 => format!("cores {:.0}%", g.cores * 100.0),
            5 => format!("net {:.0}%", g.net * 100.0),
            _ => String::new(),
        }
    }

    fn cond_say(&mut self, speech: String, pose: Pose, target: Option<usize>) {
        self.cond.speech = speech.clone();
        self.cond.pose = pose;
        self.cond.pose_until = self.now + (self.sr * 3.5) as u64;
        self.cond.target = target;
        self.cond.target_t = self.now;
        self.say(format!("♩ conductor: {speech}"));
    }

    /// put an order into effect
    fn obey(&mut self, i: usize, o: Order) {
        match o {
            Order::Hush(n) => {
                self.cond.hushv[i] = 0.5;
                self.cond.hush_ph[i] = n;
            }
            Order::LayOut(n) => {
                self.cond.lay[i] = n;
                self.rmode[i] = RMode::Rest;
                self.rest_run[i] = 0;
                self.play_run[i] = 0;
                let now = self.now;
                for si in 0..self.slots.len() {
                    if Self::player_of(self.slots[si].role) == i {
                        self.pool.release_owner(si, now);
                    }
                }
            }
        }
    }

    /// The conductor addresses one player; stubborn ones grumble and comply a bar late.
    fn order(&mut self, i: usize, o: Order, bar: u64) {
        self.order_say(i, o, bar, None);
    }
    fn order_say(&mut self, i: usize, o: Order, bar: u64, speech: Option<String>) {
        let who = self.pname(i);
        let (what, ok) = match o {
            Order::Hush(_) => (self.sty().hush, "ok, quieter"),
            Order::LayOut(n) => (if n == 1 { "lay out a phrase" } else { "lay out 2 phrases" }, "ok, resting"),
        };
        let pose = if matches!(o, Order::Hush(_)) { Pose::Shh } else { Pose::Point };
        let said = speech.unwrap_or_else(|| format!("{who}, {what}"));
        let what = said.strip_prefix(&format!("{who}, ")).unwrap_or(&said).to_string();
        self.cond_say(said, pose, Some(i));
        match o {
            Order::Hush(_) => self.cond.n_hush += 1,
            Order::LayOut(_) => self.cond.n_lay += 1,
        }
        self.cond.offenses[i] += 1.0;
        let pe = self.persona[i];
        let stubborn = 0.6 * pe.eager + 0.4 * pe.busy;
        self.hold[i] = 0;
        if stubborn > 0.6 && self.rng.chance(stubborn * 0.8) {
            let gr = ["aw, I was just getting going… fine", "one more lick? …ok, ok", "but this is the good part… fine"][self.rng.below(3)];
            self.think(i, self.minds[i].status, format!("conductor: {who}, {what} → “{gr}”"));
            self.say(format!("  {who}: “{gr}”"));
            self.cond.pending.push((i, o, bar + 1));
            self.cond.n_grumble += 1;
        } else {
            let st = if matches!(o, Order::LayOut(_)) { "laying out" } else { self.minds[i].status };
            self.think(i, st, format!("conductor: {who}, {what} → “{ok}”"));
            self.say(format!("  {who}: “{ok}”"));
            self.obey(i, o);
        }
        self.hold[i] = self.now + (self.sr * 8.0) as u64;
    }

    /// Every bar the conductor listens to the whole band and steps in if needed.
    fn conductor_bar(&mut self, bar: u64) {
        let now = self.now;
        {
            let c = &mut self.cond;
            for i in 0..7 {
                c.dens[i] = c.dens[i] * 0.55 + c.bar_notes[i] as f32 * 0.45;
                c.bar_notes[i] = 0;
                c.step_on[i] = c.step_on[i] * 0.55 + c.step_hits[i] as f32 * 0.45;
                c.step_hits[i] = 0;
            }
            c.overlap = c.overlap * 0.55 + c.ov_hits as f32 * 0.45;
            c.ov_hits = 0;
            let n = c.cm[4].max(1.0);
            let tot = (c.cm[0] + c.cm[1] + c.cm[2] + c.cm[3]) / n;
            let mh = if tot > 1e-12 { (c.cm[2] + c.cm[3]) / n / tot } else { 0.0 };
            c.mh = c.mh * 0.6 + mh * 0.4;
            c.loud = c.loud * 0.6 + 10.0 * tot.max(1e-9).log10() * 0.4;
            c.cm = [0.0; 5];
        }
        // stubborn players comply now
        let due: Vec<(usize, Order)> = {
            let (d, keep): (Vec<_>, Vec<_>) = self.cond.pending.drain(..).partition(|p| p.2 <= bar);
            self.cond.pending = keep;
            d.into_iter().map(|p| (p.0, p.1)).collect()
        };
        for (i, o) in due {
            self.obey(i, o);
            let who = self.pname(i);
            self.hold[i] = 0;
            let st = if matches!(o, Order::LayOut(_)) { "laying out" } else { self.minds[i].status };
            self.think(i, st, format!("conductor: {who} → “ok, ok”, complying"));
            self.hold[i] = now + (self.sr * 6.0) as u64;
        }
        let st = self.leader.state;
        let surge = self.feel == Feel::Surge;
        let target = self.melodic_target();
        let c = &self.cond;
        let m = c.dens[3] + c.dens[4] + c.dens[5];
        let stepping = c.step_on[3].max(c.step_on[5]);
        let clutter = 0.6 * (m / target - 1.0).max(0.0)
            + 0.12 * (c.overlap - 1.5).max(0.0)
            + 0.25 * (stepping - 0.5).max(0.0)
            + 2.5 * (c.mh - 0.33).max(0.0)
            + 0.1 * (c.loud + 14.0).max(0.0);
        self.cond.clutter = clutter;
        if self.cond_log && bar % 4 == 0 {
            let c = &self.cond;
            let msg = format!("cond {:.0}s {} m {:.1}/{:.0} ov {:.1} step {:.1} mh {:.2} loud {:.1} → clutter {:.2} | modes {:?}", self.time(), st.name(), m, target, c.overlap, stepping, c.mh, c.loud, clutter, &self.rmode[3..6]);
            eprintln!("{msg}");
        }
        // resting pose / idle speech
        if now >= self.cond.pose_until {
            let base = if surge {
                Pose::Surge
            } else if st == State::Chill {
                Pose::Rest
            } else {
                Pose::Calm
            };
            if base != self.cond.pose {
                self.cond.pose = base;
                self.cond.speech = match base {
                    Pose::Surge => "open it up!".into(),
                    Pose::Rest => "the drone has it — I'll sit this one out".into(),
                    _ => match st {
                        State::Alive => "keep it tight, leave some space".into(),
                        State::Subside => "let it breathe… easing down".into(),
                        _ => "nice and easy".into(),
                    },
                };
                self.cond.target = None;
            }
        }
        if now < self.cond.cool {
            return;
        }
        let phrase = bar % 4 == 0;
        let featured = |e: &Engine, i: usize| {
            let p = [P::Keys, P::Lead, P::Colour][i - 3];
            e.leader.soloist == Some(p) || e.leader.lone == Some(p) || e.quote_owner(i)
        };
        // synth vs the others: the synth is the first to be hushed when anyone else plays
        let drums_on = self.drums_playing();
        for i in 3..6 {
            if !self.has_synth(i) || self.rmode[i] != RMode::Play || self.cond.lay[i] > 0 || featured(self, i) || now < self.cond.nag_cool[i] || surge || self.quote_owner(i) {
                continue;
            }
            let front = self.front_of(i);
            if (front.is_some() || drums_on) && self.cond.dens[i] > 1.5 {
                let who = self.pname(i);
                let speech = match front {
                    Some(j) => format!("{who}, back to the pad — let the {} through", self.pname(j)),
                    None => format!("{who}, sit back under the drums"),
                };
                let o = if self.cond.offenses[i] >= 1.5 { Order::LayOut(1) } else { Order::Hush(2) };
                self.order_say(i, o, bar, Some(speech));
                self.cond.cool = now + (self.sr * 8.0) as u64;
                self.cond.nag_cool[i] = now + (self.sr * 24.0) as u64;
                return;
            }
        }
        // drums are for alive moments and surges
        if phrase && !surge && st != State::Alive && self.rmode[2] == RMode::Play && self.cond.dens[2] > 5.0 && now >= self.cond.nag_cool[2] {
            let syn = (3..6).any(|i| self.has_synth(i));
            let speech = if syn { "drums, give it some air — synth, take the space" } else { "drums, give it some air" };
            self.order_say(2, Order::LayOut(2), bar, Some(speech.into()));
            self.cond.cool = now + (self.sr * 10.0) as u64;
            self.cond.nag_cool[2] = now + (self.sr * 40.0) as u64;
            return;
        }
        // drums laid out: invite the synth in to stretch
        if phrase && !surge && st != State::Chill && (self.rmode[2] != RMode::Play || !self.on[2]) && !drums_on && self.rng.chance(0.6) {
            let cand: Vec<usize> = (3..6).filter(|&i| self.on[i] && self.has_synth(i) && self.rmode[i] != RMode::Play && self.cond.lay[i] == 0 && self.front_of(i).is_none()).collect();
            if !cand.is_empty() {
                let i = cand[self.rng.below(cand.len())];
                let who = self.pname(i);
                self.cond_say(format!("drums are out — {who}, stretch a little"), Pose::Point, Some(i));
                self.cond.n_invite += 1;
                self.rmode[i] = RMode::Play;
                self.rest_run[i] = 0;
                self.cond.hushv[i] = 0.85;
                self.cond.hush_ph[i] = 1;
                self.hold[i] = 0;
                self.think(i, "stretching", format!("conductor: drums are out — stretch → “my turn”"));
                self.hold[i] = now + (self.sr * 8.0) as u64;
                self.cond.cool = now + (self.sr * 12.0) as u64;
                return;
            }
        }
        if clutter > 1.3 / self.sty().strict && phrase {
            // way too much: a cue to the whole band
            self.cond.n_cue += 1;
            self.cond.cool = now + (self.sr * 25.0) as u64;
            if matches!(st, State::Chill | State::Subside) && !surge {
                self.cond_say("everyone drop to the drone".into(), Pose::Shh, None);
                for i in 2..6 {
                    self.obey(i, Order::LayOut(2));
                }
                self.cond.drone_cue = 2;
            } else {
                let n = 1 + self.rng.below(2) as u32;
                self.cond_say(format!("bass and drums only{}", if n > 1 { ", 2 phrases" } else { "" }), Pose::Point, None);
                for i in 3..6 {
                    self.obey(i, Order::LayOut(n));
                }
            }
            for i in 2..6 {
                self.hold[i] = 0;
                if self.cond.lay[i] > 0 {
                    self.think(i, "laying out", "conductor's cue → “ok, resting”");
                    self.hold[i] = now + (self.sr * 8.0) as u64;
                }
            }
            return;
        }
        if clutter > 0.45 / self.sty().strict {
            // pick the player doing the most damage
            let mut best: Option<(usize, f32)> = None;
            for i in 3..6 {
                if !self.on[i] || self.cond.lay[i] > 0 || now < self.cond.nag_cool[i] || featured(self, i) {
                    continue;
                }
                let synth = self.slots.iter().any(|s| s.on && s.synth.is_some() && Self::player_of(s.role) == i);
                // the synth goes first whenever anyone else is playing
                let others = self.front_of(i).is_some() || self.drums_playing();
                let sc = self.cond.dens[i] / target + 0.5 * self.cond.step_on[i] + if synth { if others { 0.7 } else { 0.15 } } else { 0.0 };
                if sc > 0.15 && best.map(|b| sc > b.1).unwrap_or(true) {
                    best = Some((i, sc));
                }
            }
            if let Some((i, _)) = best {
                let o = if self.cond.offenses[i] >= 1.2 || clutter > 0.9 { Order::LayOut(1 + self.rng.below(2) as u32) } else { Order::Hush(2) };
                self.order(i, o, bar);
                self.cond.cool = now + (self.sr * 8.0) as u64;
                self.cond.nag_cool[i] = now + (self.sr * 20.0) as u64;
            }
            return;
        }
        if clutter < 0.12 / self.sty().strict && phrase && st != State::Chill && !surge && self.rng.chance(0.5) {
            // space opened up: invite someone back
            let cand: Vec<usize> = (3..6).filter(|&i| self.on[i] && self.rest_run[i] >= 2 && self.cond.lay[i] == 0).collect();
            if !cand.is_empty() {
                let i = cand[self.rng.below(cand.len())];
                let who = self.pname(i);
                self.cond_say(format!("{who}, come back in — softly"), Pose::Point, Some(i));
                self.cond.n_invite += 1;
                self.rmode[i] = RMode::Play;
                self.rest_run[i] = 0;
                self.cond.hushv[i] = 0.75;
                self.cond.hush_ph[i] = 1;
                self.hold[i] = 0;
                self.think(i, "following", format!("conductor: {who}, come back in → “with pleasure”"));
                self.hold[i] = now + (self.sr * 8.0) as u64;
                self.cond.cool = now + (self.sr * 12.0) as u64;
            }
        }
    }

    /// RMS per source (dBFS, post master gain, pre limiter), drums playing vs resting
    pub fn rms_report(&self) -> String {
        let Some(m) = self.meter.as_ref() else { return String::new() };
        let db = |p: f64, n: f64| if n < 1.0 || p <= 0.0 { -120.0 } else { 10.0 * (p / n).log10() };
        let mut out = String::new();
        for (c, cn) in ["drums resting", "drums playing"].iter().enumerate() {
            let secs = m.n[c] / self.sr as f64;
            let per: Vec<String> = METER_SRC.iter().enumerate().map(|(k, n)| format!("{n} {:.1}", db(m.p[c][k], m.n[c]))).collect();
            let syn = m.p[c][0] + m.p[c][1] + m.p[c][2];
            out.push_str(&format!("rms {cn} ({secs:.0}s): {} | synth(patches+pad+bells) {:.1} vs GM melodic {:.1} dB\n", per.join(", "), db(syn, m.n[c]), db(m.p[c][7], m.n[c])));
        }
        out
    }

    pub fn conductor_stats(&self) -> String {
        let c = &self.cond;
        format!("conductor: {} interventions ({} hush, {} lay out, {} cues, {} invites back), {} grumbles", c.total(), c.n_hush, c.n_lay, c.n_cue, c.n_invite, c.n_grumble)
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
        if self.meander() && self.rng.chance(0.03) {
            for k in 0..(2 + self.rng.below(2)) {
                let pcs = self.chord().pcs();
                let pc = pcs[self.rng.below(pcs.len())];
                let semi = pc + 36 + 12 * self.rng.below(2) as i32;
                self.bell_for(0, semi as f32, 0.4, k as f32 * 0.15);
            }
        }
        self.key_listen();
        self.boredom();
        self.gm_manage();
        self.patch_manage();
        self.bars_since_change += 1;
        self.harmony_bar();
    }

    fn harmony_bar(&mut self) {
        let meander = self.meander();
        if let Some(ch) = self.harm.on_bar(self.cur_bpc, meander) {
            let note = ch.note.clone();
            let kc = ch.key_changed;
            let cad = kc || note.as_deref().map(|n| n.contains("cadence") || n.contains("resolves") || n.contains("modulation")).unwrap_or(false);
            if cad {
                self.key_converge(if kc { "key change" } else { "cadence" });
            }
            self.on_chord(ch.chord, note, false);
            if kc {
                self.band_say(format!("key change -> {}", ch.key.name()));
                let k = ch.key.name();
                for i in 1..6 {
                    self.inspire(i, format!("inspired by the modulation to {k}"), 0.5);
                }
            }
        }
        for e in std::mem::take(&mut self.harm.events) {
            self.say(format!("~ {e}"));
            for i in 1..6 {
                self.inspire(i, "the band found a new home key".into(), 0.3);
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
        // 12-bar blues: one bar per chord, always
        if self.sty().harm == Harm::Blues {
            self.cur_bpc = 1;
        }
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
        self.sub_target = Self::fold(c.root, -22 + self.reg, -11 + self.reg) as f32;
        // drone density: the sub glide plus each pad voice that actually moves
        let moved = v.iter().filter(|x| !prev.contains(x)).count() as u32 + 1;
        let sti = self.st_idx();
        self.dens[sti][0] += moved;
        if self.ui.is_some() {
            let sub = self.note_name(Self::fold(c.root, -22 + self.reg, -11 + self.reg));
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
        self.pad_pcs = c.pcs();
        // keys resting: let the pad go too (the drone carries it)
        if self.cond.lay[3] > 0 || self.rmode[3] == RMode::Rest {
            let now = self.now;
            for (hc, k) in std::mem::take(&mut self.pad_held) {
                if let Some(g) = self.gm.as_mut() {
                    g.note_off(hc, self.midi_root + k, now);
                }
            }
            if let Some(si) = self.slots.iter().position(|s| s.role == Role::Pad && s.synth.is_some()) {
                self.pool.release_owner(si, now);
            }
            return;
        }
        // keys: synth pad re-voiced on every chord (in the keys player's own key)
        if let Some(si) = self.slots.iter().position(|s| s.role == Role::Pad && s.on && s.synth.is_some()) {
            let bpc = self.cur_bpc;
            self.strike_synth_pad(si, bpc);
        }
        // keys: GM pad voice leading, common tones held
        if let Some(ch) = self.slot_ch(Role::Pad) {
            let prev: Vec<i32> = self.pad_held.iter().filter(|h| h.0 == ch).map(|h| h.1).collect();
            let nv = voice_lead(&prev, &c.pcs(), 4, 7, 29);
            let nv: Vec<i32> = nv.into_iter().map(|k| self.in_key(3, k, 45)).collect();
            let mut nv2: Vec<i32> = Vec::new();
            for k in nv {
                if !nv2.contains(&k) {
                    nv2.push(k);
                }
            }
            let nv = nv2;
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
                    self.count(3);
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
        let root = Self::fold(c.root, -17 + self.reg, -6 + self.reg); // ~41-78 Hz
        let semi = match kind {
            1 => root + (c.fifth() - c.root).rem_euclid(12),
            2 => root + 12,
            3 => {
                let next = self.harm.peek_next(self.meander());
                let nr = Self::fold(next.root, -17 + self.reg, -6 + self.reg);
                nr + if self.rng.chance(0.5) { -1 } else { 2 }
            }
            _ => root,
        };
        self.heard_semi(1, semi, vel);
        if kind == 3 && self.ui.is_some() {
            let next = self.harm.peek_next(self.meander());
            self.set_think(1, format!("walking an approach note into {}", next.name()));
        }
        self.bass_sound(semi, vel, att, dec, 2.0);
    }
    fn bass_sound(&mut self, semi: i32, vel: f32, att: f32, dec: f32, gm_len: f32) {
        let f = semi_to_freq(self.root, semi as f32);
        self.bass_inc = f / self.sr;
        self.bass_attack = true;
        self.bass_peak = vel;
        self.bass_att = 1.0 - (-1.0 / (self.sr * att)).exp();
        self.bass_dec = (-1.0 / (self.sr * dec)).exp();
        if (self.bass_mode == BassMode::Groove || (self.sty().gm_bass && self.bass_mode == BassMode::Pulse)) && self.slot_ch(Role::Bass).is_some() {
            let v = (vel * 80.0) as i32;
            self.role_note(Role::Bass, semi + 12, v, gm_len, 0.0);
        }
    }
    /// the bassist plays a given pitch (theme quotes)
    fn bass_play(&mut self, semi: i32, vel: f32, att: f32, dec: f32, len: f32) {
        self.heard_semi(1, semi, vel);
        self.bass_sound(semi, vel, att, dec, len);
    }

    fn bass_step(&mut self, s: usize) {
        if self.quote_owner(1) {
            return; // playing the theme low instead
        }
        if self.feel == Feel::Surge {
            match s {
                0 | 8 => self.bass_note(0, 0.95, 0.004, 0.34),
                3 | 11 => self.bass_note(0, 0.72, 0.004, 0.2),
                6 if self.rng.chance(0.3) => self.bass_note(1, 0.5, 0.004, 0.15),
                _ => {}
            }
            return;
        }
        if self.entry_wait > 0 {
            return;
        }
        if self.style_bass(s, self.step / 16) {
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

    /// The drummer's own per-phrase restraint: drums are for alive moments and surges.
    /// Rest (out), Hold (minimal: a soft downbeat and a couple of ticks) or Play.
    fn drums_phrase(&mut self) {
        let st = self.leader.state;
        let surge = self.feel == Feel::Surge;
        let was = self.rmode[2];
        let mut why = String::new();
        let mode = if !self.on[2] {
            RMode::Rest
        } else if self.cond.lay[2] > 0 {
            why = "conductor has me out → resting".into();
            RMode::Rest
        } else if surge {
            RMode::Play
        } else if self.sty().relax {
            // techno: the kick stays (minimal while chilling), out only in breakdowns
            why = match self.tsec {
                1 => "breakdown → kick out".into(),
                2 => "build → kick back, rolling".into(),
                3 => "the drop → full four-on-the-floor".into(),
                _ if st == State::Chill => "minimal techno: sub kick and a soft off-beat hat".into(),
                _ => "four-on-the-floor, off-beat hats, claps on 2 and 4".into(),
            };
            match self.tsec {
                1 => RMode::Rest,
                0 if st == State::Chill => RMode::Hold,
                _ => RMode::Play,
            }
        } else {
            let tied = self.tied_level(2);
            let fatigue = 0.25 * self.play_run[2].saturating_sub(2) as f32;
            // weights: rest, minimal, play
            let mut w = match st {
                State::Chill => [1.0, 0.0, 0.0],
                State::Subside => [0.65, 0.35, 0.0],
                State::Awaken => [0.5, 0.42, 0.08],
                State::Alive => [0.18, 0.32, 0.5],
            };
            w[2] += 0.25 * (tied - 0.5).max(0.0) * (st == State::Alive) as i32 as f32;
            let dm = self.sty().drums;
            w[2] *= dm;
            if dm < 1.0 {
                w[1] *= dm.sqrt();
            } else {
                w[0] /= dm;
            }
            w[0] += fatigue;
            let m = [RMode::Rest, RMode::Hold, RMode::Play][self.rng.weighted(&w)];
            why = match m {
                RMode::Rest if st == State::Chill => "chilling — no drums needed".into(),
                RMode::Rest if fatigue > 0.2 => format!("played {} phrases → dropping out, let the synth have a go", self.play_run[2]),
                RMode::Rest => "sitting this phrase out — space for the others".into(),
                RMode::Hold => "minimal: a soft downbeat and a tick, nothing more".into(),
                RMode::Play => format!("the band is alive (cpu {:.0}%) → a light groove", self.ctl.sig.cpu * 100.0),
            };
            m
        };
        match mode {
            RMode::Rest => {
                self.rest_run[2] += 1;
                self.play_run[2] = 0;
            }
            _ => {
                self.play_run[2] += 1;
                self.rest_run[2] = 0;
            }
        }
        self.rmode[2] = mode;
        self.drum_minimal = mode == RMode::Hold;
        if self.sty().drum_feel == DrumFeel::Train && self.rng.chance(0.3) {
            self.train = !self.train;
            if mode == RMode::Play {
                why = if self.train { "train beat: brushed sixteenths, accents on 2 and 4".into() } else { "two-step: kick on 1 and 3, snare on 2 and 4".into() };
            }
        }
        if self.on[2] && !why.is_empty() && (mode != was || self.rng.chance(0.3)) {
            let status = match mode {
                RMode::Rest => "laying out",
                RMode::Hold => "holding",
                RMode::Play => "grooving",
            };
            self.think(2, status, why);
        }
    }

    fn make_drum_pattern(&mut self) {
        let mode = self.rmode[2];
        let d = if self.on[2] && mode != RMode::Rest { self.leader.invite(P::Drums) } else { 0.0 };
        let ps = self.persona[2];
        let mut k = [0i32; 16];
        let mut sn = [0i32; 16];
        let mut h = [0i32; 16];
        let mut a = [0i32; 16];
        if mode == RMode::Hold && d > 0.1 {
            // minimal: soft downbeat, a tick on 2 and 4 (or a shaker pair), maybe a rim
            k[0] = 48;
            if self.rng.chance(0.5) {
                h[4] = 18;
                h[12] = 20;
            } else {
                a[6] = 12;
                a[14] = 12;
            }
            if d > 0.5 && self.rng.chance(0.3) {
                sn[12] = 22;
            }
        } else if d > 0.15 {
            k[0] = 66;
            if d > 0.6 {
                k[8] = 55;
            }
            for &i in &[3usize, 6, 10, 14] {
                // the drummer listens to the bassist's syncopations
                let follow = if self.bass_pat[i] != 0 { 0.35 } else { 0.0 };
                if self.rng.chance(((d - 0.75) * ps.busy).max(0.0) + follow * (d - 0.6).max(0.0)) {
                    k[i] = 40;
                }
            }
            if d > 0.7 {
                sn[4] = 42;
                sn[12] = 46;
                if self.rng.chance(ps.busy * 0.15) {
                    sn[15] = 16;
                }
            } else if d > 0.45 && self.rng.chance(0.5) {
                sn[12] = 32;
            }
            if d > 0.4 {
                // quarters; eighths only when really alive — no sixteenths outside surges
                let every = if d > 0.78 { 2 } else { 4 };
                for i in (0..16).step_by(every) {
                    h[i] = if i % 4 == 0 { 30 } else { 20 };
                }
            }
            if ps.register > 0.2 && d > 0.7 {
                // auxiliary percussion, two off-beats per bar
                a[6] = 13;
                a[14] = 13;
            }
            let sty = self.sty();
            if sty.perc_heavy && d > 0.3 {
                // hand drums carry the cycle: a 3+3+2 figure, quietly
                for (i, v) in [(0usize, 16), (3, 12), (6, 14), (10, 12), (12, 13)] {
                    a[i] = v;
                }
                h = [0; 16];
            }
            if sty.id == "biker" && d > 0.4 {
                // heavy low end: big kick on 1 and the "and" of 3
                k[0] = 80;
                k[10] = k[10].max(52);
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
        let swing = if s % 2 == 1 { (self.persona[2].swing * 0.33 + self.sty().swing).min(0.4) } else { 0.0 };
        let resting = self.rmode[2] == RMode::Rest && self.feel != Feel::Surge;
        if resting {
            self.fill = false;
            self.crash_next = false;
        }
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
        if self.entry_wait > 0 || !self.on[2] || self.cond.lay[2] > 0 || resting {
            return;
        }
        if self.feel == Feel::Surge {
            // V-twin "potato-potato": uneven long-short pairs (gaps of 3 and 5 sixteenths)
            match s {
                0 | 8 => {
                    self.kick_with(1.0, 38.0, 0.42);
                    self.drum(41, 92, 0.0);
                }
                3 | 11 => {
                    self.kick_with(0.72, 47.0, 0.3);
                    self.drum(43, 74, 0.12);
                }
                14 if bar % 2 == 1 => self.drum(41, 58, 0.0),
                12 => self.drum(37, 34, 0.0),
                _ => {}
            }
            return;
        }
        if self.sty().drum_feel != DrumFeel::Band {
            self.style_drums(s, bar);
            return;
        }
        // half-time stretches the bar's pattern over two bars, double-time plays it twice
        let s = match self.feel {
            Feel::Surge | Feel::Normal => s,
            Feel::Half => {
                if s % 2 == 1 {
                    return;
                }
                (bar % 2) as usize * 8 + s / 2
            }
            Feel::Double => (s * 2) % 16 + (s >= 8) as usize,
        };
        let s = s % 16;
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
            let key = if self.sty().ride { if s % 4 == 0 { 51 } else { 59 } } else if self.persona[2].eager > 0.65 { 51 } else { 42 };
            self.drum(key, h[s], swing);
            if self.gm.is_none() {
                self.hat_env = 1.0;
                self.hat_amp = h[s] as f32 / 40.0;
            }
        }
        if a[s] > 0 {
            let set = PERC_SETS[self.perc_set].1;
            let key = set[(s / 2 + bar as usize) % set.len()];
            self.drum(key, a[s], swing);
        }
    }

    /// a kick with its own body pitch and length (latched per hit)
    fn kick_with(&mut self, v: f32, base: f32, dec: f32) {
        self.kick(v);
        self.kick_b = base;
        self.kick_d = dec;
    }

    fn kick(&mut self, v: f32) {
        self.kick_on = true;
        self.kick_t = 0.0;
        self.kick_ph = 0.0;
        self.kick_amp = v.clamp(0.0, 1.0);
        // patch is latched per hit, so a patch change never bends a ringing kick
        let (_, kb, kd) = KICK_PATCHES[self.kick_patch];
        self.kick_b = kb;
        self.kick_d = kd;
    }

    // ----------------------------------------------------------------- keys

    fn keys_step(&mut self, s: usize, bar: u64) {
        if !self.on[3] || self.quote_owner(3) {
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
        if self.entry_wait > 0 || self.cond.lay[3] > 0 || self.rmode[3] == RMode::Rest {
            return;
        }
        if self.sty().id != "house" && self.style_comp(s, bar) {
            return;
        }
        if self.rmode[3] == RMode::Hold {
            // one sustained chord tone per chord, nothing else
            if s == 0 && self.bars_since_change == 0 {
                let c = self.chord();
                let pcs = c.pcs();
                let pc = pcs[(1 + self.rng.below(2)).min(pcs.len() - 1)];
                let p = Self::fold(pc, 14, 25);
                let len = (self.cur_bpc.max(1) * 16) as f32;
                self.role_note(Role::Comp, p, 42, len, 0.0);
            }
            return;
        }
        let alive = self.leader.state == State::Alive;
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
                if s == 0 || (s == 8 && alive && self.rng.chance(0.25)) {
                    let p = up[self.rng.below(up.len())];
                    self.role_note(Role::Comp, p, v - 6, 12.0, 0.0);
                }
            }
            Comp::Arp => {
                // quarter notes; eighths only when the band is alive
                let every = if alive { 2 } else { 4 };
                if s % every == 0 {
                    let p = up[(s / every) % up.len()];
                    self.role_note(Role::Comp, p, v, 6.0, 0.0);
                }
            }
            Comp::ArpUD => {
                if s % 2 == 0 {
                    let len = up.len() * 2 - 2;
                    let i = (s / 2) % len.max(1);
                    let p = if i < up.len() { up[i] } else { up[len - i] };
                    self.role_note(Role::Comp, p, v - 4, 3.0, 0.0);
                }
            }
            Comp::Broken => {
                if s % 4 == 0 {
                    let order = [0usize, 2, 1, 2];
                    let p = tones[order[(s / 4) % 4] % n];
                    self.role_note(Role::Comp, p, v, 5.0, 0.0);
                }
            }
            Comp::Offbeat => {
                if s % 8 == 4 {
                    for &p in tones.iter().take(3) {
                        self.role_note(Role::Comp, p + 12, v - 8, 1.0, 0.0);
                    }
                }
            }
        }
    }

    // ----------------------------------------------------------------- lead

    fn lead_step(&mut self, s: usize, bar: u64) {
        if !self.on[4] || self.entry_wait > 0 || self.cond.lay[4] > 0 || self.rmode[4] == RMode::Rest || self.quote_owner(4) {
            return;
        }
        let soloing = self.leader.soloist == Some(P::Lead) || self.leader.lone == Some(P::Lead);
        let state = self.leader.state;
        let phase = bar % 4;
        if self.rmode[4] == RMode::Hold && !soloing {
            // one long note at the top of the phrase, then space
            if s == 0 && phase == 0 {
                let center = 31 + (self.persona[4].register * 4.0) as i32;
                let deg = [0, 2, 4][self.rng.below(3)];
                let p = self.motif_pitch(deg, true, center);
                if self.slot_on(Role::Lead) {
                    self.role_note(Role::Lead, p, 48, 24.0, 0.0);
                } else {
                    self.bell_for(4, p as f32 + 12.0, 0.45, 0.0);
                }
            }
            return;
        }
        // call (bar 0, plus bar 1 when alive) / rest for the remainder unless soloing
        let calling = soloing || phase == 0 || (state == State::Alive && phase == 1) || (state == State::Alive && self.persona[4].busy > 0.75 && phase == 2);
        if self.leader.trade && (bar / 2) % 2 == 1 && !soloing {
            return; // the keys have this phrase
        }
        if s == 0 && phase == 0 {
            let density = match state {
                State::Chill => 0.15,
                State::Awaken => 0.3,
                State::Alive => 0.45,
                State::Subside => 0.2,
            } * (0.7 + 0.6 * self.persona[4].busy);
            let (m, what) = self.motif.develop(&mut self.rng, density);
            self.motif = m;
            self.lead_calls += 1;
            if what == "new idea" {
                self.band_say("lead starts a new melodic idea".into());
                self.inspire(3, "heard the lead's new idea".into(), 0.3);
                self.inspire(5, "heard the lead's new idea".into(), 0.2);
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
            let blues = self.sty().harm == Harm::Blues;
            if self.on[5] && self.rng.chance(if blues { 0.9 } else { 0.25 + 0.5 * self.persona[5].reply }) {
                let (m, _) = self.motif.develop(&mut self.rng, 0.4);
                self.reply = Some((m, bar));
                self.band_say("colour player answers the lead".into());
                self.think(5, "trading", if blues { "blues call-and-response → answering the lead's lick" } else { "heard the lead's call → answering it a 3rd up" });
                self.inspire(4, "liked the colour player's answer".into(), 0.25);
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
            if self.slot_on(Role::Lead) {
                self.role_note(Role::Lead, p, n.vel - 10, n.len as f32, 0.0);
            } else {
                self.bell_for(4, p as f32 + 12.0, 0.6, 0.0);
            }
        }
    }

    // --------------------------------------------------------------- colour

    fn colour_step(&mut self, s: usize, bar: u64) {
        if self.cond.lay[5] > 0 || self.rmode[5] == RMode::Rest || self.quote_owner(5) {
            return;
        }
        // shimmer: listens to the network (a few bells per bar at most)
        let net = self.ctl.sig.net;
        let p = (0.003 + 0.06 * net) * if self.on[5] { 1.0 } else { 0.3 };
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
                    self.role_note(Role::Colour, p, n.vel - 15, n.len as f32 * 1.5 + 2.0, 0.0);
                }
            }
        }
        // landscape swells
        if s == 0 {
            let lone = self.leader.lone == Some(P::Colour) || self.leader.soloist == Some(P::Colour);
            let pr = if lone { 0.6 } else if self.meander() { 0.12 } else { 0.08 };
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

    // ------------------------------------------------------------- themes

    /// Phrase line: rarely, a player gets inspired to quote a public-domain theme.
    fn quote_phrase(&mut self, bar: u64) {
        if self.quote.is_some() || self.now < self.quote_cool || self.feel == Feel::Surge {
            return;
        }
        let th = crate::themes::all();
        if th.is_empty() {
            return;
        }
        let st = self.leader.state;
        let p = if self.quote_test {
            1.0
        } else {
            self.sty().quote
                * match st {
                    State::Chill => 0.025,
                    State::Awaken => 0.11,
                    State::Alive => 0.09,
                    State::Subside => 0.035,
                }
        };
        if !self.rng.chance(p) {
            return;
        }
        // who: lead, keys, colour (if playing), or the bassist
        let mut cand: Vec<(usize, f32)> = vec![(1, 0.35)];
        for (i, w) in [(3usize, 0.8f32), (4, 1.0), (5, 0.6)] {
            if self.on[i] && self.cond.lay[i] == 0 {
                cand.push((i, w * (0.6 + self.persona[i].curiosity)));
            }
        }
        let ws: Vec<f32> = cand.iter().map(|c| c.1).collect();
        let pl = cand[self.rng.weighted(&ws)].0;
        let alive = st == State::Alive;
        let var = match pl {
            1 => Var::Low,
            3 => [Var::Augment, Var::Straight, Var::Fragment][self.rng.weighted(&[0.45, 0.3, 0.25])],
            4 => {
                let w = [0.32, 0.22, if alive { 0.14 } else { 0.0 }, 0.1, if self.on[3] || self.on[5] { 0.2 } else { 0.0 }];
                [Var::Straight, Var::Fragment, Var::Diminish, Var::Invert, Var::Call][self.rng.weighted(&w)]
            }
            _ => [Var::Augment, Var::Fragment, Var::Invert][self.rng.weighted(&[0.5, 0.3, 0.2])],
        };
        // which theme: one the player's key suits (major/minor), not heard recently
        let k = self.player_key(pl);
        let minor = matches!(k.mode, Mode::Aeolian | Mode::Dorian | Mode::Phrygian);
        let w: Vec<f32> = th.iter().enumerate().map(|(i, t)| if self.recent_themes.contains(&i) { 0.0 } else { t.fame() * if t.minorish() == minor { 2.0 } else { 1.0 } * if self.sty().quote_src.iter().any(|q| t.composer.contains(q) || t.coll.contains(q)) { 4.0 } else { 1.0 } }).collect();
        let ti = self.rng.weighted(&w);
        self.recent_themes.push_back(ti);
        if self.recent_themes.len() > 40 {
            self.recent_themes.pop_front();
        }
        self.start_quote(ti, pl, var, bar * 16, 0);
    }

    fn player_key(&self, pl: usize) -> PKey {
        if pl == 1 || pl == 0 {
            PKey::of(self.key(), Rel::Same)
        } else {
            PKey::of(self.key(), self.rel[pl])
        }
    }

    /// Transpose the theme into the player's key and mode, fit it to the tempo
    /// (a quarter note is one beat) and apply the variation.
    fn build_quote(&mut self, ti: usize, pl: usize, var: Var, from: u64) -> Option<(Vec<(u64, i32, f32)>, u64, u64)> {
        let th = &crate::themes::all()[ti];
        let mut notes: Vec<crate::themes::TNote> = th.notes.clone();
        let total: u32 = notes.iter().map(|n| n.len).sum();
        let mut pick = th.pickup;
        match var {
            Var::Fragment | Var::Call => {
                let mut acc = 0;
                let mut cut = notes.len();
                for (i, n) in notes.iter().enumerate() {
                    acc += n.len;
                    if acc * 2 >= total && i >= 3 {
                        cut = i + 1;
                        break;
                    }
                }
                notes.truncate(cut);
            }
            Var::Answer => {
                let mut acc = 0;
                let mut cut = 0;
                for (i, n) in notes.iter().enumerate() {
                    acc += n.len;
                    if acc * 2 >= total && i >= 3 {
                        cut = i + 1;
                        break;
                    }
                }
                if notes.len() - cut < 3 {
                    cut = notes.len() / 2;
                }
                notes.drain(..cut);
                while notes.first().map(|n| n.deg.is_none()).unwrap_or(false) {
                    notes.remove(0);
                }
                pick = 0;
            }
            _ => {}
        }
        let factor = match var {
            Var::Augment => 2.0,
            Var::Low => if self.bass_mode == BassMode::Groove { 1.0 } else { 2.0 },
            Var::Diminish => 0.5,
            _ => 1.0,
        };
        let d0 = notes.iter().find_map(|n| n.deg)?;
        let k = self.player_key(pl);
        let ts = k.mode.steps();
        let tminor = matches!(k.mode, Mode::Aeolian | Mode::Dorian | Mode::Phrygian);
        let keep_alt = tminor == th.minorish();
        let mut raw: Vec<(f32, Option<i32>, f32)> = Vec::new();
        let mut pos = 0.0f32;
        for n in &notes {
            let len = n.len as f32 * factor / 3.0; // 1/12 quarter → steps (4 per beat)
            let semi = n.deg.map(|d| {
                let (d, alt) = if var == Var::Invert { (2 * d0 - d, -n.alt) } else { (d, n.alt) };
                let oct = d.div_euclid(7);
                k.tonic + 12 * oct + ts[d.rem_euclid(7) as usize] + if keep_alt { alt } else { 0 }
            });
            raw.push((pos, semi, len));
            pos += len;
        }
        // register: centre the motif where this player lives
        let center = match pl {
            1 => -11 + self.reg,
            3 => 19,
            4 => 31 + (self.persona[4].register * 4.0) as i32,
            _ => 33,
        };
        let sem: Vec<i32> = raw.iter().filter_map(|r| r.1).collect();
        let mean = sem.iter().sum::<i32>() as f32 / sem.len().max(1) as f32;
        let shift = ((center as f32 - mean) / 12.0).round() as i32 * 12;
        let cap = match var {
            Var::Augment | Var::Low => 128.0,
            _ => 64.0,
        };
        let ps = ((pick as f32 * factor / 3.0).round() as u64) % 16;
        let start = if ps > 0 { from + 16 - ps } else { from };
        let mut ev = Vec::new();
        for (p, semi, len) in raw {
            if p >= cap {
                break;
            }
            if let Some(s) = semi {
                let l = len.min(cap - p);
                ev.push((start + p.round() as u64, s + shift, l));
            }
        }
        if ev.len() < 3 {
            return None;
        }
        let end = start + pos.min(cap).ceil() as u64;
        Some((ev, start, end))
    }

    fn start_quote(&mut self, ti: usize, pl: usize, var: Var, from: u64, pass: u32) {
        let Some((ev, start, end)) = self.build_quote(ti, pl, var, from) else { return };
        let th = &crate::themes::all()[ti];
        let cite = th.citation();
        let who = self.pname(pl);
        let kname = self.player_key(pl).name();
        let what = if pass == 0 { var.desc().to_string() } else { format!("{} (passed on)", var.desc()) };
        self.quote = Some(Quote { theme: ti, player: pl, var, ev, next: 0, start, end, pass, cite: cite.clone(), what: what.clone() });

        self.n_quotes += 1;
        // the quoting player is featured for the duration
        self.rmode[pl] = RMode::Play;
        self.cond.lay[pl] = 0;
        self.cond.hushv[pl] = 1.0;
        let phrases = ((end - from) / 64 + 1) as u32;
        for i in 3..6 {
            if i != pl {
                self.cond.hushv[i] = self.cond.hushv[i].min(0.7);
                self.cond.hush_ph[i] = self.cond.hush_ph[i].max(phrases);
            }
        }
        self.hold[pl] = 0;
        let verb = if pass == 0 { "quoting" } else { "picking up" };
        self.think(pl, "quoting", format!("♫ {verb} {cite} — {what}, in {kname}"));
        self.hold[pl] = self.now + ((end - self.step.min(end)) as f32 * self.step_len) as u64;
        let t = self.time();
        let line = format!("♫ [{:02}:{:02}] {who} {} {cite} — {what}, in {kname}", (t as u32) / 60, (t as u32) % 60, if pass == 0 { "quotes" } else { "picks up" });
        self.say(line.clone());
        self.quote_log.push(line);
        let ts = format!("{:02}:{:02}", (t as u32) / 60, (t as u32) % 60);
        self.quote_hist.push_back(format!("{ts} {who}: {cite}"));
        while self.quote_hist.len() > 8 {
            self.quote_hist.pop_front();
        }
        let speech = if pass == 0 {
            format!("{} — {who} has {} · everyone support", self.sty().praise, th.name())
        } else {
            format!("{who} picks it up — keep it soft")
        };
        self.cond_say(speech, Pose::Point, Some(pl));
        self.cond.pose_until = self.now + (self.sr * 6.0) as u64;
        for i in [3usize, 4, 5] {
            if i != pl && self.on[i] && self.rmode[i] != RMode::Rest {
                self.think(i, "following", format!("{who} is quoting {} → supporting softly", th.name()));
            }
        }
    }

    /// every step: sound the quote's notes, then pass the motif on or let it go
    fn quote_step(&mut self, step: u64) {
        let Some(q) = self.quote.as_mut() else { return };
        let mut due = Vec::new();
        while q.next < q.ev.len() && q.ev[q.next].0 <= step {
            due.push(q.ev[q.next]);
            q.next += 1;
        }
        let (pl, var, done, ti, pass, end) = (q.player, q.var, step >= q.end && q.next >= q.ev.len(), q.theme, q.pass, q.end);
        let _ = end;
        for (_, semi, len) in due {
            self.quote_note(pl, var, semi, len);
        }
        if !done {
            return;
        }
        self.quote = None;
        // pass it around: someone answers or picks up a fragment
        let mut nxt: Option<(usize, Var)> = None;
        if pass < 2 {
            let others: Vec<usize> = [1usize, 3, 4, 5].into_iter().filter(|&i| i != pl && (i == 1 || (self.on[i] && self.cond.lay[i] == 0))).collect();
            if !others.is_empty() && (var == Var::Call || self.rng.chance(0.4)) {
                let i = others[self.rng.below(others.len())];
                let v = if var == Var::Call { Var::Answer } else if i == 1 { Var::Low } else { [Var::Fragment, Var::Invert, Var::Augment][self.rng.weighted(&[0.6, 0.2, 0.2])] };
                nxt = Some((i, v));
            }
        }
        if let Some((i, v)) = nxt {
            let from = (step / 16 + 1) * 16;
            self.start_quote(ti, i, v, from, pass + 1);
            return;
        }
        let secs = if self.quote_test { 25.0 } else { self.rng.range(100.0, 200.0) };
        self.quote_cool = self.now + (self.sr * secs) as u64;
        for i in 3..6 {
            if self.cond.hushv[i] < 1.0 && self.cond.hush_ph[i] > 0 {
                self.cond.hush_ph[i] = 1;
            }
        }
        self.cond_say("lovely. back to it".into(), Pose::Calm, None);
        let who = self.pname(pl);
        self.think(pl, self.minds[pl].status, format!("let the theme go — back to my own lines"));
        self.band_say(format!("{who} lets the theme go"));
    }

    fn quote_note(&mut self, pl: usize, var: Var, semi: i32, len: f32) {
        let pad = matches!(var, Var::Augment) || (var == Var::Low && len >= 6.0);
        let l = if pad { len + 1.0 } else { (len * 0.92).max(0.8) };
        match pl {
            1 => {
                let vel = if pad { 0.65 } else { 0.72 };
                let (att, dec) = if pad { (0.08, 4.0) } else { (0.01, 0.6 + 0.08 * len) };
                self.bass_play(semi, vel, att, dec, l);
            }
            3 => {
                let r = if self.slot_on(Role::Comp) { Some(Role::Comp) } else if self.slot_on(Role::Pad) { Some(Role::Pad) } else { None };
                match r {
                    Some(r) => self.role_note(r, semi, if pad { 44 } else { 50 }, l, 0.0),
                    None => self.bell_for(3, semi as f32 + 12.0, 0.4, 0.0),
                }
            }
            4 => {
                if self.slot_on(Role::Lead) {
                    self.role_note(Role::Lead, semi, if pad { 46 } else { 54 }, l, 0.0);
                } else {
                    self.bell_for(4, semi as f32 + 12.0, 0.5, 0.0);
                }
            }
            _ => {
                if self.slot_on(Role::Colour) {
                    self.role_note(Role::Colour, semi, if pad { 42 } else { 48 }, l, 0.0);
                } else {
                    self.bell_for(5, semi as f32, 0.42, 0.0);
                }
            }
        }
    }

    pub fn quote_report(&self) -> Vec<String> {
        let mut v = self.quote_log.clone();
        v.push(format!("quotes: {}", self.n_quotes));
        v
    }

    // --------------------------------------------------- GM layer manager

    fn pick_program(&mut self, role: Role) -> (u8, bool) {
        let pl = Self::player_of(role);
        let pal = self.palette[pl];
        let wild = role != Role::Bass && self.rng.chance(if self.style == 0 { 0.12 } else { 0.04 });
        let si = self.slots.iter().position(|s| s.role == role).unwrap();
        let ri = match role {
            Role::Pad => 0,
            Role::Comp => 1,
            Role::Lead => 2,
            Role::Colour => 3,
            Role::Bass => 4,
        };
        let sp = self.sty().progs[ri];
        let w: Vec<f32> = (0..128u8)
            .map(|p| {
                let rw = role_weight(role, p);
                let mut w = if wild { rw.max(0.25) } else { rw * if in_palette(pal, p) { 4.0 } else { 1.0 } };
                if !wild && sp.contains(&p) {
                    w = rw.max(0.3) * 60.0;
                }
                if self.slots[si].recent.contains(&p) || self.recent_global.contains(&p) {
                    w *= 0.05;
                }
                if self.slots.iter().any(|s| s.role != role && s.on && s.synth.is_none() && s.program == Some(p)) {
                    w = 0.0;
                }
                w
            })
            .collect();
        (self.rng.weighted(&w) as u8, wild)
    }

    /// level of a player's tied machine signal (0..1)
    fn tied_level(&self, pl: usize) -> f32 {
        let s = self.ctl.sig;
        match pl {
            0 => s.screen_change,
            1 => s.file,
            2 => s.cpu,
            3 => s.input,
            4 => s.cores,
            5 => s.net,
            _ => self.guest_cpu,
        }
        .clamp(0.0, 1.0)
    }

    fn want_synth(&mut self, pl: usize) -> bool {
        self.gm.is_none() || self.rng.chance((self.persona[pl.min(5)].synth * self.sty().synth).clamp(0.02, 0.95))
    }

    /// A player designs its own synth patch from its personality and tied signal.
    fn design_patch(&mut self, role: Role, pl: usize) -> Patch {
        let ps = self.persona[pl.min(5)];
        let tied = self.tied_level(pl);
        let w: [f32; 4] = match role {
            Role::Pad => [1.0, 0.5, 1.2, 0.15],
            Role::Comp => [0.6, 1.0, 0.4, 1.2],
            Role::Lead => [1.2, 0.9, 0.7, 0.3],
            Role::Colour => [0.3, 1.2, 1.0, 0.8],
            Role::Bass => [1.0, 0.2, 0.0, 0.0],
        };
        let kw = self.sty().kinds;
        let w = [w[0] * kw[0], w[1] * kw[1], w[2] * kw[2], w[3] * kw[3]];
        let kind = [Kind::Sub, Kind::Fm, Kind::Wave, Kind::Pluck][self.rng.weighted(&w)];
        // background voices by default: slow attacks, long releases
        let (a, d, s, r) = match role {
            Role::Pad => (0.9 + ps.register.abs() * 0.9, 2.0, 0.75, 2.4),
            Role::Comp => (0.08 + 0.1 * (1.0 - ps.busy), 0.6 + 0.5 * (1.0 - ps.busy), 0.45, 0.9),
            Role::Lead => (0.12, 0.8, 0.65, 0.9),
            Role::Colour => (0.6, 1.6, 0.45, 2.0),
            Role::Bass => (0.005, 0.4, 0.6, 0.2),
        };
        let mut p = Patch { kind, a, d, s, r, ..Default::default() };
        p.gain = match role {
            Role::Pad => 0.15,
            Role::Comp => 0.2,
            Role::Lead => 0.2,
            Role::Colour => 0.15,
            Role::Bass => 0.22,
        };
        match kind {
            Kind::Sub => {
                p.osc = if ps.busy > 0.6 { 0 } else if self.rng.chance(0.5) { 2 } else { 1 };
                p.detune = 3.0 + 18.0 * (ps.swing / 0.35) * self.rng.range(0.5, 1.0);
                let base = match role {
                    Role::Pad => 700.0,
                    Role::Comp => 1300.0,
                    Role::Lead => 1800.0,
                    Role::Colour => 2400.0,
                    Role::Bass => 350.0,
                };
                // softer filter: lower cutoff, gentler resonance and sweep
                p.cutoff = base * (0.45 + 0.8 * tied);
                p.res = 0.1 + 0.4 * ps.curiosity;
                p.env_amt = 0.3 + 1.4 * ps.busy * self.rng.range(0.5, 1.0);
            }
            Kind::Fm => {
                p.ops = 2 + (ps.curiosity * 2.99) as u8;
                let r: &[f32] = if ps.curiosity > 0.55 { &[3.5, 1.41, 2.76, 7.0, 0.5] } else { &[1.0, 2.0, 3.0, 4.0, 0.5] };
                p.ratio = [1.0, r[self.rng.below(r.len())], r[self.rng.below(r.len())], r[self.rng.below(r.len())]];
                p.index = 0.7 + 2.2 * tied + 1.0 * ps.busy;
            }
            Kind::Wave => {
                p.wa = self.rng.below(5) as u8;
                p.wb = ((p.wa as usize + 1 + self.rng.below(4)) % 5) as u8;
                p.morph = 0.05 + 0.6 * tied + 0.2 * self.rng.f();
            }
            Kind::Pluck => {
                p.damp = 0.990 + 0.0095 * (1.0 - ps.busy);
                p.bright = (0.15 + 0.5 * tied.max(self.rng.f() * 0.5)).min(0.7);
                p.a = 0.002;
                p.s = 1.0;
                p.d = 1.0;
                p.r = 0.25;
                p.gain *= 1.3;
            }
        }
        p
    }

    /// Bring a GM program in on a silent channel of the slot's pair.
    fn slot_gm_in(&mut self, i: usize) -> Option<(u8, bool)> {
        let now = self.now;
        let g = self.gm.as_ref()?;
        let s = &self.slots[i];
        let fast = now < self.fast_until;
        let ch = if g.silent(s.chans[s.cur], now) { s.cur } else { 1 - s.cur };
        if !g.silent(s.chans[ch], now) && !fast {
            return None;
        }
        let role = s.role;
        let (p, wild) = self.pick_program(role);
        let chn = self.slots[i].chans[ch];
        let fade = if fast { 0.1 } else { self.rng.range(4.0, 9.0) };
        if let Some(g) = self.gm.as_mut() {
            if fast {
                g.all_off(chn);
                g.snap(chn, 0.0);
            }
            let oc = self.slots[i].chans[1 - ch];
            if fast && g.expr(oc) > 0.0 {
                g.fade(oc, 0.0, 0.1);
            }
            g.program(chn, p, prog_gain(p));
            g.fade(chn, 1.0, fade);
        }
        let sl = &mut self.slots[i];
        sl.cur = ch;
        sl.program = Some(p);
        sl.changed_at = now;
        sl.synth = None;
        sl.desc = format!("GM {} ({})", GM_NAMES[p as usize], p);
        sl.recent.push_back(p);
        if sl.recent.len() > 8 {
            sl.recent.pop_front();
        }
        self.recent_global.push_back(p);
        if self.recent_global.len() > 16 {
            self.recent_global.pop_front();
        }
        self.gm_used[p as usize] += 1;
        Some((p, wild))
    }

    /// Fade the slot's current sound out (GM: CC11 fade, then release held pads; synth: release voices).
    fn slot_out(&mut self, i: usize) {
        let now = self.now;
        if self.slots[i].synth.is_some() {
            if now < self.fast_until {
                self.pool.cut_owner(i, now);
            } else {
                self.pool.release_owner(i, now);
            }
            self.slots[i].synth = None;
            return;
        }
        if self.gm.is_none() || self.slots[i].program.is_none() {
            return;
        }
        let s = &self.slots[i];
        let chn = s.chans[s.cur];
        if now < self.fast_until {
            self.cut_channel(chn);
            return;
        }
        let fade = self.rng.range(6.0, 12.0);
        if let Some(g) = self.gm.as_mut() {
            g.fade(chn, 0.0, fade);
        }
        let at = now + (fade * self.sr) as u64;
        let held: Vec<(usize, i32)> = self.pad_held.iter().cloned().filter(|h| h.0 == chn).collect();
        for (hc, k) in held {
            self.evs.push(Ev { at, ch: hc, key: self.midi_root + k, vel: 0 });
        }
        self.pad_held.retain(|h| h.0 != chn);
    }

    /// GM to GM crossfade onto the other channel of the pair (held pad notes move across).
    fn slot_gm_swap(&mut self, i: usize) -> Option<(u8, bool)> {
        if self.now < self.fast_until {
            return self.slot_gm_fast(i);
        }
        let now = self.now;
        let s = &self.slots[i];
        if s.synth.is_some() || s.program.is_none() {
            // synth -> GM: bring GM in, then let the synth voices ring out
            let r = self.slot_gm_in(i)?;
            self.pool.release_owner(i, now);
            return Some(r);
        }
        let other = 1 - s.cur;
        let och = s.chans[other];
        if !self.gm.as_ref()?.silent(och, now) {
            return None;
        }
        let old_ch = s.chans[s.cur];
        let role = s.role;
        let (p, wild) = self.pick_program(role);
        let fin = self.rng.range(5.0, 9.0);
        let fout = self.rng.range(7.0, 12.0);
        if let Some(g) = self.gm.as_mut() {
            g.program(och, p, prog_gain(p));
            g.fade(och, 1.0, fin);
            g.fade(old_ch, 0.0, fout);
        }
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
        sl.changed_at = now;
        sl.desc = format!("GM {} ({})", GM_NAMES[p as usize], p);
        sl.recent.push_back(p);
        if sl.recent.len() > 8 {
            sl.recent.pop_front();
        }
        self.recent_global.push_back(p);
        if self.recent_global.len() > 16 {
            self.recent_global.pop_front();
        }
        self.gm_used[p as usize] += 1;
        Some((p, wild))
    }

    fn slot_synth(&mut self, i: usize) -> String {
        let role = self.slots[i].role;
        let pl = Self::player_of(role);
        let p = self.design_patch(role, pl);
        let d = p.describe();
        self.slots[i].synth = Some(p);
        self.slots[i].desc = d.clone();
        self.slots[i].changed_at = self.now;
        self.synth_used += 1;
        d
    }

    /// (re)strike the keys player's synth pad on the current chord
    fn strike_synth_pad(&mut self, si: usize, bars_left: u32) {
        let Some(p) = self.slots[si].synth else { return };
        let c = self.chord();
        let nv = voice_lead(&self.pad_prev, &c.pcs(), 3, 7, 29);
        let now = self.now;
        self.pool.release_owner(si, now);
        let gate = (bars_left.max(1) as f32 * 16.0 + 2.0) * self.step_len;
        for (vi, &k) in nv.iter().enumerate() {
            let k = self.in_key(3, k, 45);
            let f = semi_to_freq(self.root, k as f32);
            let pan = [-0.5, 0.1, 0.55][vi % 3];
            let jitter = (self.rng.f() * 0.03 * self.sr) as u64;
            self.pool.note(si, &p, f, 0.5 + 0.15 * self.rng.f(), now + jitter, gate as u64, pan);
            self.heard_semi(3, k, 0.45);
        }
        self.pad_prev = nv;
    }

    fn gm_manage(&mut self) {
        let state = self.leader.state;
        // which layers the musicians want
        let lone = self.leader.lone;
        for i in 0..self.slots.len() {
            let want = match self.slots[i].role {
                Role::Pad => self.on[3] || self.meander(),
                Role::Comp => self.on[3] && (self.leader.invite(P::Keys) > 0.45 || lone == Some(P::Keys) || self.leader.soloist == Some(P::Keys)),
                Role::Lead => self.on[4],
                Role::Colour => self.on[5],
                Role::Bass => {
                    let sb = self.sty().gm_bass;
                    self.gm.is_some() && self.bass_gm_ok && (self.bass_mode == BassMode::Groove || (sb && self.bass_mode == BassMode::Pulse)) && (sb || self.persona[1].eager > 0.35)
                }
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
        for i in 0..self.slots.len() {
            let role = self.slots[i].role;
            let pl = Self::player_of(role);
            let ok = allowed.contains(&role);
            if ok && !self.slots[i].on {
                // bring a layer in: a GM program or a self-designed synth patch
                if role != Role::Bass && self.want_synth(pl) {
                    self.slot_synth(i);
                } else if self.slot_gm_in(i).is_none() {
                    continue;
                }
                self.slots[i].on = true;
                self.since[pl] = self.now;
                let d = self.slots[i].desc.clone();
                self.say(format!("+ {d} [{role:?}]"));
                if role == Role::Pad {
                    let left = self.cur_bpc.saturating_sub(self.bars_since_change);
                    self.strike_synth_pad(i, left);
                }
            } else if !ok && self.slots[i].on {
                let d = self.slots[i].desc.clone();
                self.slot_out(i);
                self.slots[i].on = false;
                self.say(format!("\u{2212} {d} [{role:?}]"));
            } else if ok && self.slots[i].on && self.ready(pl) {
                // inspired: pick up a GM instrument or design a synth patch
                let old = self.slots[i].desc.clone();
                let what = if role != Role::Bass && self.want_synth(pl) {
                    self.slot_out(i);
                    let d = self.slot_synth(i);
                    if role == Role::Pad {
                        let left = self.cur_bpc.saturating_sub(self.bars_since_change);
                        self.strike_synth_pad(i, left);
                    }
                    format!("designing {}", d.trim_start_matches("synth "))
                } else {
                    match self.slot_gm_swap(i) {
                        Some((p, wild)) => {
                            let nm = GM_NAMES[p as usize].to_lowercase();
                            let art = if nm.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
                            format!("picking up {art} {nm}{}", if wild { " (wildcard!)" } else { "" })
                        }
                        None => continue, // try again next bar
                    }
                };
                let d = self.slots[i].desc.clone();
                self.say(format!("+ {d}  \u{2212} {old} [{role:?}]"));
                self.changed_sound(pl, what);
            }
        }
    }

    // ------------------------------------------------------------ bed voices

    fn bell(&mut self, semi: f32, gain: f32, delay_s: f32) {
        self.bell_for(5, semi, gain, delay_s);
    }
    fn bell_for(&mut self, who: usize, semi: f32, gain: f32, delay_s: f32) {
        let _ = delay_s;
        let Some((v, _)) = self.gate(who, (gain * 100.0) as i32, 2.0, true) else { return };
        let gain = v as f32 / 100.0;
        let semi = self.in_key(who, semi.round() as i32, 40) as f32;
        self.heard_semi(who, semi.round() as i32, gain);
        self.mel_until[who] = self.mel_until[who].max(self.now + (self.step_len * 6.0) as u64);
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
        let (_, ratio, cents, dscale) = BELL_PATCHES[self.bell_patch];
        let dec = (0.6 + self.rng.f() * 1.2) * dscale;
        let a = gain * (0.6 + 0.4 * self.rng.f());
        let b = &mut self.bells[idx];
        b.on = true;
        b.inc1 = f * (cents / 1200.0).exp2() / sr;
        b.inc2 = f * ratio * (-cents / 1200.0).exp2() / sr;
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
        self.space_tick = self.space_tick.wrapping_add(1);
        if self.space_tick % 4 == 0 {
            self.apply_space();
        }
        // the drone bed is a background voice: a darker filter that closes further when ducked
        let tc = 1300.0 * (e * 1.0).exp2() * (0.85 + 0.3 * bri) * (1.0 + 0.15 * warm) * self.bed_fc;
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
        if self.acid_on {
            // 303: resonant lowpass, slow sweep over 32 bars + accent envelope
            let lfo = 0.18 * (std::f32::consts::TAU * (self.step as f32 / (32.0 * 16.0))).sin();
            let base = match self.tsec {
                1 => 0.1,
                2 => 0.2 + 0.7 * (self.tsec_ph as f32 / self.tsec_len.max(1) as f32),
                3 => 0.8,
                _ => 0.35 + 0.3 * self.leader.energy,
            };
            self.acid_cut_t = (base + lfo).clamp(0.05, 1.0);
            self.acid_cut += (self.acid_cut_t - self.acid_cut) * 0.01;
            self.acid_env *= (-(CR as f32) / (sr * 0.12)).exp();
            let fc = 110.0 * (5.0 * self.acid_cut).exp2() * (1.0 + 1.6 * self.acid_env);
            self.bass_f.set(fc.min(5000.0), 3.2 + 3.0 * self.acid_cut, sr);
        } else {
            self.bass_f.set(140.0 + 520.0 * self.bass_env * (0.4 + e), 0.8, sr);
        }
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
        // style keys act per audio block, not per step
        if crate::shared::KEY_STYLE.load(Relaxed) != 0 {
            self.steer();
        }
        let ui_on = self.ui.is_some();
        let sti = self.st_idx();
        self.st_time[sti] += frames as f64 / self.sr as f64;
        let sr = self.sr;
        let mut fi = 0;
        // bass patch changes glide over ~0.5 s (no timbre jumps)
        self.bass_saw += (self.bass_saw_t - self.bass_saw) * 0.08;
        self.bass_drive += (self.bass_drive_t - self.bass_drive) * 0.08;
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
                        if e.vel > 0 { g.note_on(e.ch, e.key, e.vel, now) } else if e.vel < 0 { g.bend(e.ch, -(e.vel + 1)) } else if e.key < 0 { g.all_off(e.ch) } else { g.note_off(e.ch, e.key, now) }
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
            // GM sits forward of the oscillator bed, a little more as the band heats up
            let gmg = 5.2 + 1.6 * self.leader.energy;
            let now0 = self.now;
            self.pool.render(now0, n, &self.tbl);
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
                let pl = self.pad_f[0].run(self.padl[ci] * self.pad_norm).0 * self.bed_g;
                let pr = self.pad_f[1].run(self.padr[ci] * self.pad_norm).0 * self.bed_g;
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
                    let mut raw = saw(self.bass_ph, self.bass_inc) * self.bass_saw + sine(tbl, self.bass_ph) * 0.9;
                    let w = ((self.bass_drive - 1.0) / 1.2).clamp(0.0, 1.0);
                    if w > 0.0 {
                        raw = raw * (1.0 - w) + (raw * self.bass_drive).tanh() / self.bass_drive.sqrt() * w;
                    }
                    adv(&mut self.bass_ph, self.bass_inc);
                    bass = self.bass_f.run(raw).0 * e * 0.3;
                }
                // kick
                let mut kick = 0.0;
                if self.kick_on {
                    let t = self.kick_t;
                    let kf = self.kick_b + 90.0 * (-t / 0.035).exp();
                    kick = sine(tbl, self.kick_ph) * self.kick_amp * (-t / self.kick_d).exp();
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
                    ((g.l[j] + g.dl[j]) * gmg, (g.r[j] + g.dr[j]) * gmg)
                } else {
                    (0.0, 0.0)
                };
                // fx
                let dly_in = (bl + br) * 0.5 + (gl + gr) * 0.3 + if self.acid_on { (self.pool.l[j] + self.pool.r[j]) * 0.18 } else { 0.0 };
                // echo time follows the tempo, gliding one sample at a time (tape-like, no clicks)
                if self.dlen != self.dlen_target && (self.now as usize + j) % 4 == 0 {
                    if self.dlen < self.dlen_target { self.dlen += 1 } else { self.dlen -= 1 }
                }
                let dn = self.dl.len();
                let rd = (self.di + dn - self.dlen) % dn;
                let dl = self.dl[rd];
                let drr = self.dr[rd];
                self.dl[self.di] = dly_in + drr * 0.4;
                self.dr[self.di] = dl * 0.85;
                self.di += 1;
                if self.di == dn {
                    self.di = 0;
                }
                let (yl, yr) = (self.pool.l[j], self.pool.r[j]);
                let send = (yl + yr) * 0.35 + (pl + pr) * 0.25 + (bl + br) * 0.6 + (dl + drr) * 0.5 + (ml + mr) * 0.3 + (gl + gr) * 0.3 + (cl + cr) * 0.3 + ris;
                let (rl, rr) = self.rev.run(send * 0.5);
                // throttle-rev swell: low-passed saw rumble whose pitch rises (or falls)
                let mut rev = 0.0;
                if self.rev_on {
                    let x = self.rev_t / 1.8;
                    if x >= 1.0 {
                        self.rev_on = false;
                    } else {
                        let f = if self.rev_up { 33.0 + 42.0 * x * x } else { 72.0 - 38.0 * x };
                        let inc = f / sr;
                        adv(&mut self.rev_ph[0], inc);
                        adv(&mut self.rev_ph[1], inc * 0.5);
                        let raw = saw(self.rev_ph[0], inc) + 0.8 * sine(tbl, self.rev_ph[1]);
                        self.rev_lp[0] += (raw - self.rev_lp[0]) * 0.03;
                        self.rev_lp[1] += (self.rev_lp[0] - self.rev_lp[1]) * 0.03;
                        let e = (std::f32::consts::PI * x).sin();
                        rev = self.rev_lp[1] * e * e * 0.32;
                        self.rev_t += 1.0 / sr;
                    }
                }
                let low = sub + bass + kick + rev; // mono low end, never high-passed
                let mut l = pl + low + bl + cl + ml + yl + gl + air + hat + ris + dl * 0.3 + rl;
                let mut r = pr + low + br + cr + mr + yr + gr - air * 0.7 + hat + ris + drr * 0.3 + rr;
                if self.fade < self.fade_target {
                    self.fade = (self.fade + self.fade_step).min(self.fade_target);
                } else if self.fade > self.fade_target {
                    self.fade = (self.fade - self.fade_step).max(self.fade_target);
                }
                let g = self.master * self.fade * self.fade * 1.9;
                l *= g;
                r *= g;
                if self.meter.is_some() {
                    let drums = self.last_drum > 0 && ((self.now + j as u64).saturating_sub(self.last_drum) as f32) < self.step_len * 16.0;
                    let (gdl, gdr) = match self.gm.as_ref() {
                        Some(gm) => (gm.dl[j] * gmg, gm.dr[j] * gmg),
                        None => (0.0, 0.0),
                    };
                    let src = [
                        (yl, yr),
                        (pl, pr),
                        (bl, br),
                        (sub, sub),
                        (bass, bass),
                        (kick, kick),
                        (hat, hat),
                        (ml - gdl, mr - gdr),
                        (gdl, gdr),
                        (gl, gr),
                        (l / g.max(1e-9), r / g.max(1e-9)),
                    ];
                    let m = self.meter.as_mut().unwrap();
                    let c = drums as usize;
                    m.n[c] += 1.0;
                    for (k, (a, b)) in src.iter().enumerate() {
                        let (a, b) = ((a * g) as f64, (b * g) as f64);
                        m.p[c][k] += 0.5 * (a * a + b * b);
                    }
                }
                let (ol, or) = self.lim.run(l, r);
                out[(fi + j) * 2] = ol;
                out[(fi + j) * 2 + 1] = or;
                // three one-pole splits: <60 Hz, <250 Hz, <3 kHz (UI meters + the conductor's ears)
                let m = 0.5 * (ol + or);
                self.lp[0] += (m - self.lp[0]) * 0.0085;
                self.lp[1] += (m - self.lp[1]) * 0.035;
                self.lp[2] += (m - self.lp[2]) * 0.35;
                let (a, b2, c) = (self.lp[0], self.lp[1], self.lp[2]);
                let e = [a * a, (b2 - a) * (b2 - a), (c - b2) * (c - b2), (m - c) * (m - c)];
                let cm = &mut self.cond.cm;
                cm[0] += e[0];
                cm[1] += e[1];
                cm[2] += e[2];
                cm[3] += e[3];
                cm[4] += 1.0;
                if ui_on {
                    self.lv[0] += e[0];
                    self.lv[1] += e[1];
                    self.lv[2] += e[2];
                    self.lv[3] += e[3];
                    self.lv_n += 1;
                }
            }
            self.now = end;
            fi += n;
        }
        if ui_on && (self.pub_now || self.now.saturating_sub(self.last_pub) as f32 >= self.sr * 0.1) {
            self.pub_now = false;
            let dt = (self.now - self.last_pub) as f32 / self.sr;
            self.last_pub = self.now;
            for m in self.minds.iter_mut() {
                m.activity *= (-dt / 0.8).exp();
            }
            self.publish();
        }
    }
}
