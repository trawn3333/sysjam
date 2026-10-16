//! The conductor: watches the whole band every bar (note density, players crowding
//! the same register, stepping on the lead, mid/high clutter, loudness) and steps in
//! when it gets out of hand: hush a player, lay one out for a few phrases, cue the
//! band down to the drone or to bass and drums, or invite a resting player back.
//! State lives here; the decisions run inside the engine (synth.rs).

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Pose {
    /// sitting on the stool while the drone carries it (chilling)
    #[default]
    Rest,
    /// keeping time, baton tracing the beat
    Calm,
    /// finger to lips: hushing a player
    Shh,
    /// pointing at a player: cue / lay out / come back in
    Point,
    /// both arms up for a surge
    Surge,
}

/// An order a stubborn player hasn't complied with yet.
#[derive(Clone, Copy, Debug)]
pub enum Order {
    Hush(u32),
    LayOut(u32),
}

#[derive(Default)]
pub struct Conductor {
    pub pose: Pose,
    pub pose_until: u64,
    pub speech: String,
    pub target: Option<usize>,
    pub target_t: u64,
    /// earliest time for the next intervention
    pub cool: u64,
    /// per-player: phrases to lay out, hush level (1 = none) and phrases left on it
    pub lay: [u32; 7],
    pub hushv: [f32; 7],
    pub hush_ph: [u32; 7],
    /// per-player cooldown so one player isn't nagged every bar
    pub nag_cool: [u64; 7],
    /// how many times each player was told off recently (decays per phrase)
    pub offenses: [f32; 7],
    /// stubborn players comply one bar later: (player, order, due bar)
    pub pending: Vec<(usize, Order, u64)>,
    /// cue: bassist holds long notes for this many phrases
    pub drone_cue: u32,
    // measurements
    pub bar_notes: [u32; 7],
    pub dens: [f32; 7],
    pub last_semi: [(i32, u64); 7],
    pub ov_hits: u32,
    pub overlap: f32,
    pub step_hits: [u32; 7],
    pub step_on: [f32; 7],
    /// output band energy accumulators since the last bar: sub, bass, mid, high, n
    pub cm: [f32; 5],
    pub mh: f32,
    pub loud: f32,
    pub clutter: f32,
    // stats
    pub n_hush: u32,
    pub n_lay: u32,
    pub n_cue: u32,
    pub n_invite: u32,
    pub n_grumble: u32,
}

impl Conductor {
    pub fn new() -> Conductor {
        Conductor { hushv: [1.0; 7], pose: Pose::Calm, speech: "taking the podium…".into(), ..Default::default() }
    }

    pub fn total(&self) -> u32 {
        self.n_hush + self.n_lay + self.n_cue + self.n_invite
    }
}
