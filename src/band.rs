//! The bandleader (arrangement/energy state machine), player personalities
//! and the motif engine. The musicians themselves live in synth.rs (Engine).

use crate::dsp::Rng;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Chill,
    Awaken,
    Alive,
    Subside,
}
impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Chill => "chilling",
            State::Awaken => "awakening",
            State::Alive => "alive",
            State::Subside => "subsiding",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum P {
    Drone,
    Bass,
    Drums,
    Keys,
    Lead,
    Colour,
}
impl P {
    pub fn name(self) -> &'static str {
        match self {
            P::Drone => "drone player",
            P::Bass => "bassist",
            P::Drums => "drummer",
            P::Keys => "keys",
            P::Lead => "lead",
            P::Colour => "colour player",
        }
    }
}

/// Normalised (0..1), lightly smoothed machine signals.
#[derive(Clone, Copy, Debug, Default)]
pub struct Signals {
    pub cpu: f32,
    pub cores: f32,
    pub net: f32,
    pub file: f32,
    pub fd: f32,
    pub screen_change: f32,
    /// combined keyboard + mouse activity 0..1 (counts only)
    pub input: f32,
    /// the audience's excitement (set by the engine, not the monitor), a light push
    pub crowd: f32,
}

#[derive(Default)]
pub struct Requests {
    pub tension: bool,
    pub modulate: bool,
    pub resolve: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Entry {
    Fill,
    Riser,
    Appear,
}

pub struct Leader {
    rng: Rng,
    pub state: State,
    pub phrases_in_state: u32,
    dwell: u32,    // phrases
    since_surprise: u32,
    idle_t: f32,
    sub_len: u32,  // bars of subsiding
    pub bars_in_state: u32,
    pub energy: f32,
    pub agitation: f32,
    /// sustained chaos: the colour player may bring in industrial noise
    pub chaos: bool,
    chaos_t: f32,
    /// one-shot deliberate industrial colour move (consumed by the engine)
    pub industrial_move: bool,
    pub n_hi: u32,
    cpu_hi: f32,
    burst: f32,
    tension_cd: f32,
    mod_cd: f32,
    resolve_cd: f32,
    peaked: bool,
    pub soloist: Option<P>,
    solo_left: u32,
    pub breakdown: u32,
    pub lone: Option<P>,
    pub trade: bool,
    pub entry: Option<Entry>,
    started: bool,
}

impl Leader {
    pub fn new(seed: u64) -> Self {
        let mut rng = Rng::seeded(seed ^ 0x4c454144);
        let dwell = 3 + rng.below(2) as u32;
        Leader {
            rng,
            state: State::Chill,
            phrases_in_state: 0,
            dwell,
            since_surprise: 2,
            idle_t: 0.0,
            sub_len: 12,
            bars_in_state: 0,
            energy: 0.0,
            agitation: 0.0,
            chaos: false,
            chaos_t: 0.0,
            industrial_move: false,
            n_hi: 0,
            cpu_hi: 0.0,
            burst: 0.0,
            tension_cd: 20.0,
            mod_cd: 30.0,
            resolve_cd: 0.0,
            peaked: false,
            soloist: None,
            solo_left: 0,
            breakdown: 0,
            lone: None,
            trade: false,
            entry: None,
            started: false,
        }
    }

    /// Fold the machine signals into energy/agitation; detect sustained events.
    pub fn listen(&mut self, s: &Signals, dt: f32, msgs: &mut Vec<String>) -> Requests {
        let mut req = Requests::default();
        let n_hi = [s.cpu > 0.5, s.net > 0.5, s.file > 0.5, s.cores > 0.5, s.input > 0.55].iter().filter(|&&b| b).count() as u32;
        self.n_hi = n_hi;
        // input activity tips energy; capped so compositor chatter can't dominate
        let input = (0.55 * s.input).min(0.45);
        let mut inst = 0.40 * s.cpu + 0.12 * s.cores + 0.18 * s.net + 0.18 * s.file + 0.04 * s.fd + 0.08 * s.screen_change + input + 0.06 * s.crowd.clamp(0.0, 1.0);
        if n_hi >= 2 {
            inst += 0.12 * (n_hi - 1) as f32;
        }
        let inst = inst.clamp(0.0, 1.0);
        if !self.started {
            self.energy = inst * 0.5;
            self.agitation = inst;
            self.started = true;
        }
        self.energy += (inst - self.energy) * (1.0 - (-dt / 15.0).exp());
        self.agitation += (inst - self.agitation) * (1.0 - (-dt / 3.0).exp());
        self.tension_cd -= dt;
        self.mod_cd -= dt;
        self.resolve_cd -= dt;
        if s.cpu > 0.65 { self.cpu_hi += dt } else { self.cpu_hi = 0.0 }
        if s.net > 0.7 || s.file > 0.7 { self.burst += dt } else { self.burst = 0.0 }
        if self.cpu_hi >= 5.0 && self.tension_cd <= 0.0 {
            req.tension = true;
            self.tension_cd = 40.0;
            msgs.push("~ sustained CPU spike -> tension".into());
        }
        if self.burst >= 4.0 && self.mod_cd <= 0.0 {
            req.modulate = true;
            self.mod_cd = 90.0;
            msgs.push("~ big I/O burst -> modulation".into());
        }
        if self.energy > 0.4 {
            self.peaked = true;
        }
        if self.agitation > 0.68 { self.chaos_t = (self.chaos_t + dt).min(10.0) } else { self.chaos_t = (self.chaos_t - 2.0 * dt).max(0.0) }
        self.chaos = self.chaos_t >= 6.0 || (self.state == State::Alive && n_hi >= 3 && self.agitation > 0.55);
        if self.agitation < 0.12 { self.idle_t += dt } else { self.idle_t = 0.0 }
        if self.peaked && self.idle_t >= 8.0 && self.resolve_cd <= 0.0 {
            self.peaked = false;
            self.resolve_cd = 60.0;
            req.resolve = true;
            msgs.push("~ back to idle -> resolve to tonic".into());
        }
        req
    }

    fn go(&mut self, to: State, msgs: &mut Vec<String>) {
        msgs.push(format!("~ {} -> {} (energy {:.2})", self.state.name(), to.name(), self.energy));
        self.state = to;
        self.phrases_in_state = 0;
        self.bars_in_state = 0;
        self.dwell = 3 + self.rng.below(2) as u32; // 3-4 phrases (~36-48 s)
        if to == State::Subside {
            self.sub_len = 8 + self.rng.below(12) as u32; // 8-19 bars (~24-57 s)
        }
        self.entry = Some(match self.rng.below(3) {
            0 => Entry::Fill,
            1 => Entry::Riser,
            _ => Entry::Appear,
        });
        self.soloist = None;
        self.breakdown = 0;
        self.lone = None;
    }

    pub fn on_bar(&mut self) {
        self.bars_in_state += 1;
    }

    /// Phrase boundary: maybe change state (stochastically, with dwell and
    /// hysteresis) and maybe call a surprise.
    pub fn on_phrase(&mut self, msgs: &mut Vec<String>) -> Requests {
        let mut req = Requests::default();
        self.phrases_in_state += 1;
        self.entry = None;
        let e = self.energy;
        let dwell_ok = self.phrases_in_state >= self.dwell;
        // the longer we've been past the dwell, the more decisive the leader gets
        let over = self.phrases_in_state.saturating_sub(self.dwell) as f32;
        let mut p = (0.5 + 0.2 * over).min(0.95);
        if (self.state == State::Alive && e < 0.12) || (self.state == State::Chill && e > 0.6) {
            p = 1.0; // unmistakable: no dithering
        }
        let r = self.rng.f();
        match self.state {
            State::Chill if dwell_ok && e > 0.22 => {
                if self.agitation > 0.55 && self.n_hi >= 2 && r < 0.35 {
                    self.go(State::Alive, msgs)
                } else if r < p {
                    self.go(State::Awaken, msgs)
                }
            }
            State::Awaken if (dwell_ok && e > 0.42 && r < p) || (e > 0.35 && self.agitation > 0.55) => self.go(State::Alive, msgs),
            State::Awaken if dwell_ok && e < 0.15 && r < p => {
                self.go(State::Subside, msgs);
                req.resolve = true;
            }
            State::Alive if dwell_ok && e < 0.3 && r < p => {
                self.go(State::Subside, msgs);
                req.resolve = true;
            }
            State::Subside if e > 0.45 && r < 0.4 => self.go(State::Awaken, msgs),
            State::Subside if self.bars_in_state >= self.sub_len => {
                self.go(State::Chill, msgs);
                req.resolve = true;
            }
            _ => {}
        }
        // surprises
        if self.solo_left > 0 {
            self.solo_left -= 1;
            if self.solo_left == 0 {
                if let Some(p) = self.soloist.take() {
                    msgs.push(format!("{} ends the solo", p.name()));
                }
            }
        }
        if self.breakdown > 0 {
            self.breakdown -= 1;
            if self.breakdown == 0 {
                msgs.push("band comes back in".into());
            }
        }
        if self.lone.is_some() && self.rng.chance(0.6) {
            self.lone = None;
        }
        self.since_surprise += 1;
        let n_before = msgs.len();
        if self.phrases_in_state > 0 && self.since_surprise >= 4 && self.soloist.is_none() && self.breakdown == 0 && self.lone.is_none() {
            let x = self.rng.f();
            match self.state {
                State::Alive => {
                    if x < 0.10 {
                        let p = [P::Keys, P::Lead, P::Colour][self.rng.below(3)];
                        self.soloist = Some(p);
                        self.solo_left = 2;
                        msgs.push(format!("{} takes a solo", p.name()));
                    } else if x < 0.16 {
                        self.breakdown = 1;
                        msgs.push("breakdown: drums and bass drop out".into());
                    } else if x < 0.19 {
                        req.modulate = true;
                        msgs.push("bandleader calls a key change".into());
                    } else if x < 0.22 {
                        self.industrial_move = true;
                    }
                    self.trade = self.rng.chance(0.4);
                }
                State::Awaken => {
                    if x < 0.06 {
                        let p = [P::Keys, P::Lead][self.rng.below(2)];
                        self.soloist = Some(p);
                        self.solo_left = 1;
                        msgs.push(format!("{} takes a solo", p.name()));
                    }
                    self.trade = self.rng.chance(0.2);
                }
                State::Chill => {
                    if x < 0.08 {
                        let p = [P::Colour, P::Keys, P::Lead][self.rng.below(3)];
                        self.lone = Some(p);
                        msgs.push(format!("{} plays alone for a while", p.name()));
                    }
                    self.trade = false;
                }
                State::Subside => self.trade = false,
            }
        }
        if msgs.len() > n_before {
            self.since_surprise = 0;
        }
        req
    }

    /// the conductor features a player for a few phrases (stagnation breaker)
    pub fn feature(&mut self, p: P, phrases: u32) {
        self.soloist = Some(p);
        self.solo_left = phrases;
        self.since_surprise = 0;
    }

    /// How much the leader invites a player in, 0..1.
    pub fn invite(&self, p: P) -> f32 {
        let base = match self.state {
            State::Chill => [1.0, 0.3, 0.05, 0.35, 0.18, 0.6],
            State::Awaken => [1.0, 0.65, 0.5, 0.55, 0.45, 0.45],
            State::Alive => [1.0, 0.95, 0.95, 0.75, 0.85, 0.5],
            State::Subside => {
                let f = (self.bars_in_state as f32 / self.sub_len as f32).min(1.0);
                // peel order: drums first, then melody resolves, then bass
                [1.0, 0.3 + 0.55 * (1.0 - f / 0.9).max(0.0), 0.75 * (1.0 - f / 0.35).max(0.0), 0.5, 0.6 * (1.0 - f / 0.6).max(0.0), 0.5]
            }
        };
        let i = match p {
            P::Drone => 0,
            P::Bass => 1,
            P::Drums => 2,
            P::Keys => 3,
            P::Lead => 4,
            P::Colour => 5,
        };
        let mut v = base[i];
        if self.breakdown > 0 && matches!(p, P::Drums | P::Bass) {
            v *= 0.15;
        }
        if let Some(s) = self.soloist {
            if s == p {
                v = 1.0;
            } else if matches!(p, P::Keys | P::Lead | P::Colour) {
                v *= 0.5;
            }
        }
        if let Some(l) = self.lone {
            if l == p {
                v = v.max(0.7);
            } else if p != P::Drone {
                v *= 0.2;
            }
        }
        v
    }
}

/// Each musician's personality, rolled per run (or per --seed).
#[derive(Clone, Copy, Debug)]
pub struct Persona {
    pub eager: f32,
    pub busy: f32,
    pub swing: f32,
    pub reply: f32,
    pub register: f32,
    /// how readily this player gets bored of a sound / picks up a new one
    pub curiosity: f32,
    /// 0 = always reaches for a GM sample, 1 = always designs its own synth patch
    pub synth: f32,
    /// how far from the band key this player dares to wander
    pub adventure: f32,
}
impl Persona {
    pub fn roll(rng: &mut Rng) -> Self {
        Persona { eager: rng.f(), busy: rng.range(0.2, 0.9), swing: rng.range(0.0, 0.35), reply: rng.f(), register: rng.bi(), curiosity: rng.f(), synth: rng.range(0.15, 0.85), adventure: rng.f() }
    }
    /// join/leave decision against an invitation (hysteresis built in)
    pub fn decide(&self, on: bool, invite: f32, rng: &mut Rng) -> bool {
        let want = invite + (self.eager - 0.5) * 0.2 + rng.bi() * 0.12;
        if !on && want > 0.42 && rng.chance(0.7) {
            return true;
        }
        if on && want < 0.22 && rng.chance(0.65) {
            return false;
        }
        on
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MNote {
    pub at: u8,
    pub deg: i32,
    pub len: u8,
    pub vel: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Motif {
    pub notes: Vec<MNote>,
    pub len: u8,
}

impl Motif {
    pub fn random(rng: &mut Rng, density: f32, len: u8) -> Motif {
        let mut notes = Vec::new();
        let mut deg = [0, 2, 4][rng.below(3)];
        let mut t = 0u8;
        while t < len {
            let strong = t % 4 == 0;
            let p = density * if strong { 1.0 } else { 0.45 };
            if rng.chance(p) {
                let l = [2u8, 2, 3, 4, 4, 6, 8][rng.below(7)].min(len - t);
                notes.push(MNote { at: t, deg, len: l, vel: 60 + rng.below(25) as i32 });
                let r = rng.f();
                deg += if r < 0.35 { 1 } else if r < 0.7 { -1 } else if r < 0.8 { 2 } else if r < 0.9 { -2 } else if r < 0.95 { 4 } else { -3 };
                deg = deg.clamp(-5, 9);
                t += l;
            } else {
                t += 2;
            }
        }
        if notes.is_empty() {
            notes.push(MNote { at: 0, deg: 0, len: 4, vel: 70 });
        }
        Motif { notes, len }
    }

    /// A complete statement: at least `min` notes with a contour, ending on a held
    /// chord tone (the cadence point). Short fragments get stepwise notes added.
    pub fn phrase(&self, rng: &mut Rng, min: usize) -> Motif {
        let mut m = self.clone();
        m.notes.sort_by_key(|n| n.at);
        while m.notes.len() < min {
            let last = *m.notes.last().unwrap();
            let at = last.at + last.len.max(2);
            if at + 2 > m.len {
                break;
            }
            let d = if rng.chance(0.5) { 1 } else { -1 };
            m.notes.push(MNote { at, deg: (last.deg + d).clamp(-6, 10), len: 2, vel: (last.vel - 4).max(40) });
        }
        // squeeze into the bar if it ran out of room
        if m.notes.len() < min {
            let n = min.max(1) as u8;
            let step = (m.len / n).max(2);
            let base = m.notes[0].deg;
            m.notes = (0..n).map(|k| MNote { at: k * step, deg: base + [0, 1, 2, 1, 0][k as usize % 5], len: step, vel: 64 }).collect();
        }
        // a connected statement: no gap longer than a beat and a half inside the phrase
        let mut t = m.notes[0].at.min(m.len.saturating_sub(2));
        let mut keep = Vec::new();
        for n in m.notes.iter() {
            if t + 1 >= m.len {
                break;
            }
            let gap = n.len.clamp(2, 6);
            keep.push(MNote { at: t, len: gap, ..*n });
            t += gap;
        }
        if !keep.is_empty() {
            m.notes = keep;
        }
        // cadence: the last note lands on a chord tone and rings
        let k = m.notes.len() - 1;
        let d = m.notes[k].deg;
        let tones = [-7, -5, -3, 0, 2, 4, 7, 9, 11];
        let near = *tones.iter().min_by_key(|t| (*t - d).abs()).unwrap();
        m.notes[k].deg = near;
        let room = m.len.saturating_sub(m.notes[k].at).max(2);
        m.notes[k].len = room.min(8).max(m.notes[k].len);
        m
    }

    /// Develop the idea: returns the new motif and what was done.
    pub fn develop(&self, rng: &mut Rng, density: f32) -> (Motif, &'static str) {
        let mut m = self.clone();
        let x = rng.f();
        let what = if x < 0.3 {
            "repeat"
        } else if x < 0.48 {
            let d = if rng.chance(0.5) { 1 } else { -1 };
            for n in m.notes.iter_mut() {
                n.deg += d;
            }
            "sequence"
        } else if x < 0.6 {
            let first = m.notes[0].deg;
            for n in m.notes.iter_mut() {
                n.deg = first - (n.deg - first);
            }
            "invert"
        } else if x < 0.74 {
            // rhythmic variation: nudge one note / drop one
            if m.notes.len() > 2 && rng.chance(0.5) {
                let i = rng.below(m.notes.len());
                m.notes.remove(i);
            } else {
                let i = rng.below(m.notes.len());
                m.notes[i].deg += if rng.chance(0.5) { 1 } else { -1 };
            }
            "vary"
        } else if x < 0.84 {
            let k = (m.notes.len() / 2).max(1);
            m.notes.truncate(k);
            "fragment"
        } else {
            m = Motif::random(rng, density, self.len);
            "new idea"
        };
        for n in m.notes.iter_mut() {
            n.deg = n.deg.clamp(-6, 10);
            n.vel = (n.vel + rng.below(11) as i32 - 5).clamp(40, 100);
        }
        (m, what)
    }
}
