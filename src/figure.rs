//! The little people: the conductor (baton phase-locked to the heard beat) and an
//! animated character per musician, dressed for the style and moved by their notes.
//!
//! Every figure is a fixed grid of single-width cells drawn only from ASCII, light
//! box drawing, a few block elements for hats and ♪ ♫ • ·, so nothing shears in any
//! terminal font (see the width audit test at the bottom).

use crate::clock::{Feel, Pos};
use crate::conductor::Pose;
use crate::mood::Mood;

pub const CW: usize = 12;
pub const PW: usize = 10;

/// what a style dresses everyone in
pub struct Gear {
    pub hat: &'static str,
    /// replaces the face's parentheses (headphones, goggles, wig curls, helmet)
    pub sides: (char, char),
    /// sunglasses
    pub shades: bool,
    pub robe: char,
    pub props: &'static [char],
    /// what idle hands wave (techno glowsticks)
    pub wave: Option<char>,
}

pub fn gear(style: &str) -> Gear {
    let g = |hat, sides, shades, robe, props, wave| Gear { hat, sides, shades, robe, props, wave };
    match style {
        "monk" => g("╭───╮", ('(', ')'), false, ':', &['.', 'o', '.', ' '], None),
        "jazz" => g(" _=_ ", ('(', ')'), false, '=', &['♪', '♫', '♪', '♫'], None),
        "blues" => g("_┌#┐_", ('(', ')'), true, '#', &['♪', ' ', '♪', '♪'], None),
        "country" => g("~╭^╮~", ('(', ')'), false, 'x', &['o', 'O', 'o', '*'], None),
        "biker" => g("╭───╮", ('(', ')'), false, '#', &['•', '*', '•', '*'], None),
        "baroque" => g("@@@@@", ('{', '}'), false, '+', &['♪', '•', '♪', '•'], None),
        "celtic" => g("_╭──┐", ('(', ')'), false, '%', &['*', '•', '*', '•'], None),
        "industrial" => g("┌┬─┬┐", ('[', ']'), false, '#', &['*', 'x', '*', 'x'], None),
        "techno" => g("╭───╮", ('d', 'b'), true, '=', &['|', '/', '-', '\\'], Some('!')),
        "space" => g("╭─*─╮", ('|', '|'), false, 'o', &['*', '+', '*', '.'], None),
        "tribal" => g("\\|||/", ('(', ')'), false, 'v', &['o', '*', 'o', '+'], None),
        _ => g("     ", ('(', ')'), false, '=', &['•'], None),
    }
}

/// eyes, mouth and an extra mark for a mood
fn face(m: Mood) -> (char, char, char, Option<&'static str>) {
    use Mood::*;
    match m {
        Content => ('^', '_', '^', None),
        Delighted => ('^', 'o', '^', Some("*")),
        Proud => ('^', 'v', '^', None),
        Amused => ('^', 'w', '^', None),
        Inspired => ('*', 'o', '*', Some("*")),
        Nostalgic => ('-', '_', '-', Some("♪")),
        Focused => ('o', '_', 'o', None),
        Impatient => ('>', '_', '>', None),
        Frustrated => ('>', '_', '<', Some("#")),
        Annoyed => ('=', '_', '=', Some("#")),
        Bored => ('.', '_', '.', Some("...")),
        Sleepy => ('-', '.', '-', Some("zZ")),
        Anxious => ('O', '_', 'O', Some("'")),
        Surprised => ('O', 'o', 'O', Some("!")),
        Tender => ('^', '.', '^', None),
        Melancholy => (';', '_', ';', None),
    }
}

struct Grid {
    w: usize,
    c: Vec<Vec<char>>,
}
impl Grid {
    fn new(w: usize, h: usize) -> Grid {
        Grid { w, c: vec![vec![' '; w]; h] }
    }
    fn put(&mut self, r: i32, col: i32, s: &str) {
        if r < 0 || r as usize >= self.c.len() {
            return;
        }
        for (k, ch) in s.chars().enumerate() {
            let x = col + k as i32;
            if x >= 0 && (x as usize) < self.w && ch != '\u{0}' {
                self.c[r as usize][x as usize] = ch;
            }
        }
    }
    fn set(&mut self, r: i32, col: i32, ch: char) {
        if r >= 0 && (r as usize) < self.c.len() && col >= 0 && (col as usize) < self.w {
            self.c[r as usize][col as usize] = ch;
        }
    }
    fn rows(self) -> Vec<String> {
        self.c.into_iter().map(|r| r.into_iter().collect()).collect()
    }
}

fn face_str(g: &Gear, m: Mood) -> String {
    let (l, mo, r, _) = face(m);
    // sunglasses stay on while he's cool; strong feelings show his eyes
    let cool = matches!(m, Mood::Content | Mood::Proud | Mood::Focused | Mood::Impatient | Mood::Bored | Mood::Tender | Mood::Amused);
    let (l, r) = if g.shades && cool { ('=', '=') } else { (l, r) };
    format!("{}{l}{mo}{r}{}", g.sides.0, g.sides.1)
}

// ------------------------------------------------------------------ conductor

/// baton tip ictus points per pattern, in grid cells (x 7..11, y 0..4)
const P4: [(f32, f32); 4] = [(9.0, 4.0), (7.0, 2.6), (11.0, 2.6), (9.5, 0.3)];
const P3: [(f32, f32); 3] = [(9.0, 4.0), (11.0, 2.6), (9.5, 0.3)];
const P2: [(f32, f32); 2] = [(9.0, 4.0), (10.5, 2.4)];

/// which conducting pattern, and where in it: (points, beat index with fraction)
pub fn pattern(p: &Pos) -> (&'static [(f32, f32)], f64) {
    let inbar = p.in_bar();
    match p.meta.feel {
        // half time: beat it in 2 over the bar
        Feel::Half => (&P2, inbar / 2.0),
        // double time: the 4-pattern twice per bar
        Feel::Double => (&P4, (inbar * 2.0) % 4.0),
        _ if p.meta.meter == 3 => (&P3, p.beats().rem_euclid(3.0)),
        _ if p.meta.meter == 2 => (&P2, p.beats().rem_euclid(2.0)),
        _ => (&P4, inbar),
    }
}

/// continuous baton tip position for a beat position (energy 0.4..1 scales the gesture)
pub fn baton(p: &Pos, energy: f32) -> (f32, f32) {
    let (pts, idx) = pattern(p);
    let n = pts.len();
    let k = (idx.floor() as usize) % n;
    let f = idx.fract() as f32;
    let (a, b) = (pts[k], pts[(k + 1) % n]);
    let ease = f * f * (3.0 - 2.0 * f);
    let x = a.0 + (b.0 - a.0) * ease;
    // rebound: up out of each ictus, falling into the next (a straight drop into 1)
    let h = if (k + 1) % n == 0 { 0.5 } else { 1.4 };
    let mut y = a.1 + (b.1 - a.1) * f - h * 4.0 * f * (1.0 - f);
    if p.meta.shuffle {
        // 12/8 shuffle: a little tick on the swung third triplet of every beat
        let d = (f - 0.667) / 0.08;
        y += 0.8 * (-d * d).exp();
    }
    let e = if p.meta.feel == Feel::Surge { 1.0 } else { energy.clamp(0.4, 1.0) };
    let x = (9.0 + (x - 9.0) * e).clamp(7.0, 11.0);
    let y = (2.0 + (y - 2.0) * e).clamp(0.0, 4.0);
    (x, y)
}

/// the 5 × 12 conductor for this instant
pub fn conductor(style: &str, pose: Pose, mood: Mood, energy: f32, pos: Option<Pos>, t: f32) -> Vec<String> {
    let g = gear(style);
    let mut gr = Grid::new(CW, 5);
    let beats = pos.map(|p| p.beats()).unwrap_or(t as f64 * 1.5);
    let frac = beats.fract() as f32;
    let beat_i = beats.floor() as i64;
    let prop = g.props[(beat_i.rem_euclid(g.props.len() as i64)) as usize];
    let (_, _, _, extra) = face(mood);
    let fs = face_str(&g, mood);
    if style == "techno" && pose != Pose::Rest {
        // DJ: platters spin with the beat, hands up on every kick
        gr.put(0, 2, g.hat);
        gr.put(1, 2, &fs);
        if frac < 0.25 {
            gr.put(2, 1, " ╲│=│╱");
        } else {
            gr.put(2, 1, " ╭┤=├╮");
        }
        let sp = ['|', '/', '-', '\\'];
        let a = sp[((frac * 4.0) as usize) % 4];
        let b = sp[((frac * 4.0) as usize + 2) % 4];
        gr.put(3, 0, &format!(" [{a}]──[{b}] "));
        gr.put(4, 0, " └──────┘  ");
        marks(&mut gr, extra, beat_i, frac);
        return gr.rows();
    }
    gr.put(0, 2, g.hat);
    gr.put(1, 2, &fs);
    let robe = g.robe;
    match pose {
        Pose::Rest => {
            gr.put(2, 2, &format!("╭┤{robe}├╮"));
            gr.put(3, 1, "╰┴───┴╯");
            gr.put(4, 2, " │   │");
            let z = if frac < 0.5 { "z" } else { "zZ" };
            gr.put(0, 8, z);
        }
        Pose::Shh => {
            gr.put(2, 2, &format!("╭┤{robe}├╯"));
            gr.set(1, 7, '│');
            gr.put(3, 3, &format!("│{robe}│"));
            gr.put(4, 3, "╯ ╰");
            gr.put(0, 8, if frac < 0.5 { "shh" } else { "sh." });
        }
        Pose::Point => {
            gr.put(2, 2, &format!("╭┤{robe}├"));
            gr.put(3, 3, &format!("│{robe}│"));
            gr.put(4, 3, "╯ ╰");
            let len = if frac < 0.5 { 4 } else { 5 };
            for k in 0..len {
                gr.set(2, 6 + k, '─');
            }
            gr.set(2, 6 + len, '>');
        }
        Pose::Surge => {
            gr.put(3, 3, &format!("│{robe}│"));
            gr.put(4, 3, "╯ ╰");
            if frac < 0.5 {
                gr.put(2, 3, &format!("┤{robe}├"));
                gr.set(1, 1, '╲');
                gr.set(1, 7, '╱');
                gr.set(0, 0, prop);
                gr.set(0, 8, prop);
            } else {
                gr.put(2, 1, &format!("─┤{robe}├─"));
                gr.set(2, 0, prop);
                gr.set(2, 8, prop);
            }
        }
        Pose::Calm => {
            gr.put(2, 2, &format!("╭┤{robe}├"));
            gr.put(3, 3, &format!("│{robe}│"));
            gr.put(4, 3, "╯ ╰");
            marks(&mut gr, extra, beat_i, frac);
            let (tx, ty) = match pos {
                Some(p) => baton(&p, energy),
                None => (9.0, 2.0),
            };
            arm(&mut gr, (6, 2), (tx.round() as i32, ty.round() as i32), prop);
            conductor_feet(&mut gr, pose, beat_i, frac);
            return gr.rows();
        }
    }
    conductor_feet(&mut gr, pose, beat_i, frac);
    marks(&mut gr, extra, beat_i, frac);
    gr.rows()
}

/// the conductor shifts his weight every two beats and taps the free foot on the beat;
/// in a surge his knees bounce and he stamps
fn conductor_feet(gr: &mut Grid, pose: Pose, beat: i64, frac: f32) {
    let on = frac < 0.2;
    let f = match pose {
        Pose::Rest => return,
        Pose::Surge => {
            if frac >= 0.5 {
                "╲ ╱"
            } else if beat % 2 == 0 {
                "┴ ╰"
            } else {
                "╯ ┴"
            }
        }
        _ => match (beat.rem_euclid(4) < 2, on) {
            (true, true) => "│ ─",
            (true, false) => "│ ╰",
            (false, true) => "─ │",
            (false, false) => "╯ │",
        },
    };
    gr.put(4, 3, f);
}

/// mood marks around the head (sparkles blink on the beat, Zzz drifts)
fn marks(gr: &mut Grid, extra: Option<&str>, beat: i64, frac: f32) {
    match extra {
        Some("*") => {
            if beat % 2 == 0 {
                gr.set(0, 0, '*');
            } else {
                gr.set(1, 8, '*');
            }
        }
        Some("zZ") => gr.put(0, 8, if frac < 0.5 { "z" } else { " zZ" }),
        Some("'") => gr.set(1, 8, '\''),
        Some(";") => gr.set(1, 1, ','),
        Some(m) => gr.put(0, 8, m),
        None => {}
    }
}

/// right arm from the shoulder to the baton tip
fn arm(gr: &mut Grid, from: (i32, i32), to: (i32, i32), tip: char) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let n = dx.abs().max(dy.abs());
    for k in 1..n {
        let x = from.0 + (dx as f32 * k as f32 / n as f32).round() as i32;
        let y = from.1 + (dy as f32 * k as f32 / n as f32).round() as i32;
        let (px, py) = (from.0 + (dx as f32 * (k - 1) as f32 / n as f32).round() as i32, from.1 + (dy as f32 * (k - 1) as f32 / n as f32).round() as i32);
        let (sx, sy) = (x - px, y - py);
        let ch = match (sx.signum(), sy.signum()) {
            (_, 0) => '─',
            (0, _) => '│',
            (a, b) if a * b < 0 => '╱',
            _ => '╲',
        };
        gr.set(y, x, ch);
    }
    gr.set(to.1, to.0, tip);
}

// ------------------------------------------------------------------ musicians

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Fam {
    #[default]
    Drone,
    Bass,
    Drums,
    Piano,
    Organ,
    Ep,
    Guitar,
    Strings,
    Horn,
    Reed,
    Flute,
    Voice,
    Synth,
    Bells,
    Guest,
}

pub const FAMS: [Fam; 15] = [Fam::Drone, Fam::Bass, Fam::Drums, Fam::Piano, Fam::Organ, Fam::Ep, Fam::Guitar, Fam::Strings, Fam::Horn, Fam::Reed, Fam::Flute, Fam::Voice, Fam::Synth, Fam::Bells, Fam::Guest];

/// GM program → instrument family
pub fn fam_of_gm(p: u8) -> Fam {
    match p {
        0..=3 | 6 | 7 => Fam::Piano,
        4 | 5 => Fam::Ep,
        8..=15 | 112..=119 => Fam::Bells,
        16..=20 => Fam::Organ,
        21..=23 | 64..=71 | 109 | 111 => Fam::Reed,
        24..=31 | 104..=108 => Fam::Guitar,
        32..=39 => Fam::Bass,
        40..=51 | 110 => Fam::Strings,
        52..=54 => Fam::Voice,
        56..=63 => Fam::Horn,
        72..=79 => Fam::Flute,
        _ => Fam::Synth,
    }
}

impl Fam {
    pub fn name(self) -> &'static str {
        ["drone", "bass", "drums", "piano", "organ", "electric piano", "guitar", "strings", "brass", "reeds", "flute", "voice", "synth", "bells", "guest"][self as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PState {
    Play,
    /// resting: 0 coffee, 1 dozing, 2 phone
    Rest(u8),
    /// hushed by the conductor (seconds since)
    Hushed(f32),
}

pub struct PlayerIn<'a> {
    pub style: &'a str,
    pub fam: Fam,
    pub state: PState,
    /// seconds since the last heard note, its key and velocity, and a running hit count
    pub hit: Option<(f32, i32, i32, u64)>,
    pub beats: f64,
    /// instrument change in progress: (old family, seconds since)
    pub swap: Option<(Fam, f32)>,
    /// guest walking: 0 on stage, 1 fully off to the right
    pub walk: f32,
    pub quoting: bool,
    pub cond_mood: Mood,
    pub t: f32,
    /// the band is surging: knee bounces and stomps
    pub surge: bool,
}

const SWAP_S: f32 = 1.0;

/// one musician: `rows` (3 or 4) × 10 cells
pub fn player(p: &PlayerIn, rows: usize) -> Vec<String> {
    let g = gear(p.style);
    let mut gr = Grid::new(PW, 4);
    let (age, key, vel, n) = p.hit.unwrap_or((99.0, 0, 0, 0));
    let fresh = age < 0.14;
    let beat_f = p.beats.fract() as f32;
    let robe = g.robe;
    // face: sheepish when hushed, squint on hard hits, dozing when resting
    let mut mood = match p.state {
        PState::Hushed(_) => Mood::Melancholy,
        PState::Rest(1) => Mood::Sleepy,
        PState::Rest(_) => Mood::Content,
        PState::Play if fresh && vel > 105 => Mood::Frustrated,
        PState::Play if p.fam == Fam::Drone => Mood::Sleepy,
        PState::Play => Mood::Focused,
    };
    if p.swap.is_some() {
        mood = Mood::Surprised;
    }
    let mut fs: Vec<char> = face_str(&g, mood).chars().collect();
    if let PState::Hushed(_) = p.state {
        fs[1] = '.';
        fs[3] = '.';
    }
    let fs: String = fs.into_iter().collect();
    // sway (leads), bob (bass) as a one-cell shift
    let mut dx = 0;
    if matches!(p.fam, Fam::Horn | Fam::Reed | Fam::Flute | Fam::Voice | Fam::Guitar) && p.state == PState::Play && (p.beats / 2.0).fract() < 0.5 {
        dx = 1;
    }
    let bob = p.fam == Fam::Bass && age < 0.15;
    gr.put(0, dx, g.hat);
    gr.put(1, dx + bob as i32, &fs);
    gr.put(3, dx, " ╯ ╰ ");
    let (mut la, mut ra) = ('╱', '╲');
    match p.state {
        PState::Rest(kind) => {
            gr.put(2, dx, &format!("╭|{robe}|╮"));
            gr.put(3, dx, " └─┘ ");
            match kind {
                0 => gr.put(2, dx + 5, "c[_]"),
                1 => gr.put(0, dx + 5, if beat_f < 0.5 { " z" } else { " zZ" }),
                _ => gr.put(2, dx + 5, if (p.t * 0.7).fract() < 0.5 { "[:]" } else { "[.]" }),
            }
            if let Some(w) = g.wave {
                // techno: glowstick swings with the beat
                gr.set(1, dx + 5, if beat_f < 0.5 { '/' } else { '\\' });
                gr.set(0, dx + 6, w);
            }
            react(&mut gr, p.cond_mood, p.beats);
            return finish(gr, rows);
        }
        PState::Hushed(s) if s < 2.5 => gr.put(0, 5, "shh"),
        _ => {}
    }
    // instrument, possibly sliding out (old) or in (new) during a swap
    let (fam, shift) = match p.swap {
        Some((old, s)) if s < SWAP_S / 2.0 => (old, (s / (SWAP_S / 2.0) * 5.0).round() as i32),
        Some((_, s)) if s < SWAP_S => (p.fam, ((1.0 - (s - SWAP_S / 2.0) / (SWAP_S / 2.0)) * 5.0).round() as i32),
        _ => (p.fam, 0),
    };
    let ix = dx + 5 + shift;
    let wob = ["─┬─", "╱┬╲", "╲┬╱"];
    match fam {
        Fam::Drums => {
            let cym = matches!(key, 42 | 44 | 46 | 49 | 51 | 52 | 53 | 55 | 57 | 59) && age < 0.45;
            gr.put(1, ix + 2, if cym { wob[1 + ((age * 12.0) as usize % 2)] } else { wob[0] });
            gr.put(2, ix + 1, if fresh && matches!(key, 37 | 38 | 39 | 40) { "(*)" } else { "(_)" });
            gr.put(3, ix, if fresh && matches!(key, 35 | 36) { "[@]" } else { "[O]" });
            if fresh {
                if n % 2 == 0 {
                    ra = '_';
                } else {
                    la = '_';
                }
            }
        }
        Fam::Bass => {
            gr.put(1, ix + 2, "│");
            gr.put(2, ix + 1, if age < 0.15 { "(~)" } else { "(│)" });
            gr.put(3, ix + 1, "(_)");
            ra = if age < 0.15 { '─' } else { '╲' };
        }
        Fam::Piano => {
            gr.put(2, ix, "┌───┐");
            let mut k: Vec<char> = "│|||│".chars().collect();
            if fresh {
                k[1 + (key.rem_euclid(3)) as usize] = '_';
                ra = '_';
            }
            gr.put(3, ix, &k.iter().collect::<String>());
            gr.put(3, dx, " ╯ ╰");
        }
        Fam::Organ => {
            let mut up: Vec<char> = "[===]".chars().collect();
            let mut lo: Vec<char> = "[===]".chars().collect();
            if fresh {
                let i = 1 + (key.rem_euclid(3)) as usize;
                if n % 2 == 0 {
                    up[i] = '-';
                } else {
                    lo[i] = '-';
                }
            }
            gr.put(1, ix, " ┌┴┐ ");
            gr.put(2, ix, &up.iter().collect::<String>());
            gr.put(3, ix, &lo.iter().collect::<String>());
            ra = '─';
            la = if fresh { '─' } else { '╱' };
        }
        Fam::Ep => {
            gr.put(2, ix, if fresh { "_-_-_" } else { "_____" });
            gr.put(3, ix, " │ │ ");
            ra = if fresh { '_' } else { '─' };
        }
        Fam::Guitar => {
            gr.put(2, ix, if fresh { "*──┤" } else { "o──┤" });
            ra = if n % 2 == 0 { '╱' } else { '╲' };
        }
        Fam::Strings => {
            gr.set(1, ix, '<');
            // bow: down-bow long, up-bow short, alternating per note
            let bow = if n % 2 == 0 { "────" } else { "──  " };
            let bow = if age < 0.6 { bow } else { "─── " };
            gr.put(2, ix, bow);
            ra = '─';
        }
        Fam::Horn => {
            let hi = key >= 72;
            if hi && age < 1.2 {
                gr.set(1, ix, '╱');
                gr.put(0, ix + 1, "─<");
            } else {
                gr.put(1, ix, "──<");
            }
            if p.style == "blues" && age < 0.35 {
                gr.set(1, ix + 3, '~');
            }
            la = '╲';
            ra = '─';
        }
        Fam::Reed => {
            let up = (key >= 72 && age < 1.2) as i32;
            gr.set(1 - up, ix, '╲');
            gr.set(2 - up, ix + 1, '│');
            gr.put(3 - up, ix + 1, "╰)");
            if age < 0.5 {
                gr.set(0, ix + 2 + (age * 6.0) as i32, '♪');
            }
            ra = '─';
        }
        Fam::Flute => {
            let mut fl: Vec<char> = "────".chars().collect();
            if fresh || age < 0.3 {
                fl[(key.rem_euclid(4)) as usize] = 'o';
            }
            gr.put(1, ix, &fl.iter().collect::<String>());
            la = '╲';
            ra = '─';
        }
        Fam::Voice | Fam::Guest => {
            if age < 0.8 {
                let r = if age < 0.4 { 1 } else { 0 };
                gr.set(r, ix + (age * 4.0) as i32, '♪');
            }
            let mut fc: Vec<char> = fs.chars().collect();
            if age < 0.3 {
                fc[2] = 'o';
            }
            gr.put(1, dx, &fc.iter().collect::<String>());
            la = '╲';
            ra = '╱';
        }
        Fam::Synth => {
            let sp = ['|', '/', '-', '\\'];
            let a = sp[(n % 4) as usize];
            let b = sp[((n + 2) % 4) as usize];
            gr.put(2, ix, &format!("[{a} {b}]"));
            gr.put(3, ix, " ╰─╯ ");
            ra = if fresh { '_' } else { '─' };
        }
        Fam::Bells => {
            let mut bars: Vec<char> = "=====".chars().collect();
            if age < 0.25 {
                bars[(key.rem_euclid(5)) as usize] = '*';
            }
            gr.put(2, ix, " o o ");
            gr.put(3, ix, &bars.iter().collect::<String>());
            ra = if fresh { '_' } else { '╲' };
        }
        Fam::Drone => {
            // a sleepy hum; breathing every two beats
            let breathe = (p.beats / 4.0).fract() < 0.5;
            let body = if breathe { format!("╱({robe})╲") } else { format!("╱|{robe}|╲") };
            gr.put(2, dx, &body);
            let hum = (p.t * 0.8) as i32 % 4;
            gr.put(1, dx + 5, &" ~ ~ ~".chars().skip(hum as usize % 2).take(5).collect::<String>());
            react(&mut gr, p.cond_mood, p.beats);
            return finish(gr, rows);
        }
    }
    gr.put(2, dx, &format!("{la}|{robe}|{ra}"));
    if p.state == PState::Play && p.walk <= 0.0 {
        let kick = fam == Fam::Drums && age < 0.16 && matches!(key, 35 | 36);
        gr.put(3, dx, feet(p.style, p.beats, p.surge, kick));
    }
    if p.quoting {
        gr.put(0, 6, "(♫)");
    }
    react(&mut gr, p.cond_mood, p.beats);
    // a guest walks on and off from the right
    if p.walk > 0.0 {
        let off = (p.walk * PW as f32).round() as usize;
        for row in gr.c.iter_mut() {
            let mut r = vec![' '; off.min(PW)];
            r.extend(row.iter().take(PW - off.min(PW)));
            *row = r;
        }
        // legs scissor while walking
        let legs = if (p.t * 6.0) as i32 % 2 == 0 { " ╯ ╰ " } else { "  │  " };
        if off < PW {
            gr.put(3, off as i32, legs);
        }
    }
    finish(gr, rows)
}

/// a musician's feet (4 cells): taps on the beat in the style's own way; the
/// drummer's right foot comes down on the kick pedal on real kick hits
fn feet(style: &str, beats: f64, surge: bool, kick: bool) -> &'static str {
    if kick {
        return " ╯ ─";
    }
    let beat = beats.floor() as i64;
    let frac = beats.fract() as f32;
    let on = frac < 0.2;
    if surge {
        return if frac >= 0.5 { " ╲ ╱" } else if beat % 2 == 0 { " ┴ ╰" } else { " ╯ ┴" };
    }
    match style {
        "techno" | "house" if on => " ╯ ─",
        "blues" | "jazz" if on && beat % 2 == 1 => " ╯ ─",
        // heel-toe
        "country" if on => if beat % 2 == 0 { " ╯ └" } else { " ╯ ─" },
        "baroque" if on && beat.rem_euclid(4) == 0 => " ╯ ─",
        "biker" | "industrial" if on => " ╯ ┴",
        // jig: both feet on the eighths
        "celtic" => match ((beats * 2.0).floor() as i64).rem_euclid(4) {
            0 => " ╯ ─",
            2 => " ─ ╰",
            _ => " ╯ ╰",
        },
        _ if on && beat % 2 == 0 => " ╯ ─",
        _ => " ╯ ╰",
    }
}

/// players glance at the conductor: a sweat drop when he's cross, a sparkle when he beams
fn react(gr: &mut Grid, m: Mood, beats: f64) {
    match m {
        Mood::Frustrated | Mood::Annoyed => gr.set(0, 9, '\''),
        Mood::Delighted | Mood::Inspired if (beats.floor() as i64) % 2 == 0 => gr.set(0, 9, '*'),
        _ => {}
    }
}

fn finish(gr: Grid, rows: usize) -> Vec<String> {
    let mut r = gr.rows();
    if rows < 4 {
        r.remove(0);
    }
    r
}

// ------------------------------------------------------------------ audit

/// glyphs the figures may use: ASCII, light box drawing, a few block elements for hats
pub fn safe_glyph(c: char) -> bool {
    (' '..='~').contains(&c) || "─│┌┐└┘├┤┬┴┼╭╮╯╰╱╲♪♫•·".contains(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Meta;
    use unicode_width::UnicodeWidthStr;

    fn check(lines: &[String], w: usize, what: &str) {
        for l in lines {
            assert_eq!(l.width(), w, "{what}: width of {l:?}");
            assert_eq!(l.chars().count(), w, "{what}: cells of {l:?}");
            for c in l.chars() {
                assert!(safe_glyph(c), "{what}: unsafe glyph {c:?} (U+{:04X}) in {l:?}", c as u32);
            }
        }
    }

    #[test]
    fn every_figure_is_clean_and_fixed_width() {
        let poses = [Pose::Rest, Pose::Calm, Pose::Shh, Pose::Point, Pose::Surge];
        for st in crate::style::STYLES {
            for &pose in &poses {
                for &m in crate::mood::ALL.iter() {
                    for feel in [Feel::Normal, Feel::Half, Feel::Double, Feel::Surge] {
                        for k in 0..32 {
                            let pos = Pos { steps: k as f64 * 0.53, len: 5000.0, meta: Meta { feel, meter: 4, shuffle: st.id == "blues" } };
                            let c = conductor(st.id, pose, m, 0.4 + (k % 7) as f32 * 0.1, Some(pos), k as f32 * 0.1);
                            assert_eq!(c.len(), 5);
                            check(&c, CW, &format!("conductor {} {pose:?} {m:?}", st.id));
                        }
                    }
                }
            }
            let states = [PState::Play, PState::Rest(0), PState::Rest(1), PState::Rest(2), PState::Hushed(1.0), PState::Hushed(5.0)];
            for fam in FAMS {
                for &state in &states {
                    for k in 0..24 {
                        let age = k as f32 * 0.07;
                        let pin = PlayerIn {
                            style: st.id,
                            fam,
                            state,
                            hit: if k % 5 == 0 { None } else { Some((age, 30 + k * 3, 60 + k * 3, k as u64)) },
                            beats: k as f64 * 0.37,
                            swap: if k % 3 == 0 { Some((FAMS[k as usize % FAMS.len()], age)) } else { None },
                            walk: if fam == Fam::Guest { (k as f32 / 23.0).min(1.0) } else { 0.0 },
                            quoting: k % 4 == 0,
                            cond_mood: crate::mood::ALL[k as usize % 16],
                            t: k as f32 * 0.21,
                            surge: k % 4 == 1,
                        };
                        for rows in [3, 4] {
                            let p = player(&pin, rows);
                            assert_eq!(p.len(), rows);
                            check(&p, PW, &format!("player {} {fam:?} {state:?}", st.id));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn downbeat_is_the_bottom_of_the_pattern() {
        for feel in [Feel::Normal, Feel::Half, Feel::Double] {
            let pos = |s: f64| Pos { steps: s, len: 5000.0, meta: Meta { feel, meter: 4, shuffle: false } };
            let (_, y0) = baton(&pos(16.0), 1.0);
            assert!((y0 - 4.0).abs() < 1e-4, "{feel:?}: downbeat at {y0}");
            for k in 1..64 {
                let (_, y) = baton(&pos(16.0 + k as f64 * 0.25), 1.0);
                assert!(y <= y0 + 1e-4);
            }
        }
    }
}

#[cfg(test)]
mod show {
    use super::*;
    use crate::clock::Meta;
    /// `cargo test --release expressions -- --ignored` writes the expression sheet text
    #[test]
    #[ignore]
    fn expressions() {
        use Mood::*;
        let moods = [Content, Delighted, Proud, Amused, Inspired, Nostalgic, Focused, Impatient, Frustrated, Annoyed, Bored, Sleepy, Anxious, Surprised, Tender, Melancholy];
        let mut out = String::new();
        for (st, col) in [("country", "38;2;234;152;72"), ("blues", "38;2;112;152;255"), ("techno", "38;2;0;230;230"), ("baroque", "38;2;230;200;120"), ("tribal", "38;2;220;170;90")] {
            for half in moods.chunks(8) {
                let mut rows = vec![String::new(); 6];
                for (k, m) in half.iter().enumerate() {
                    let pos = Pos { steps: 2.0 + k as f64 * 1.7, len: 5000.0, meta: Meta { feel: Feel::Normal, meter: 4, shuffle: st == "blues" } };
                    let c = conductor(st, Pose::Calm, *m, 0.9, Some(pos), 0.0);
                    for (r, l) in c.iter().enumerate() {
                        rows[r].push_str(&format!("\x1b[1;{col}m{l}\x1b[0m  "));
                    }
                    rows[5].push_str(&format!("\x1b[2;37m{:<14}\x1b[0m", format!("{}", m.name())));
                }
                for r in rows {
                    out.push_str(&r);
                    out.push('\n');
                }
            }
            out.push('\n');
        }
        std::fs::write(std::env::var("SYSJAM_SHEET").unwrap_or("/tmp/expressions.ans".into()), out).unwrap();
    }

    #[test]
    #[ignore]
    fn print_some() {
        for st in ["tribal", "country", "techno", "blues", "baroque"] {
            let mut out = vec![String::new(); 5];
            for k in 0..8 {
                let pos = Pos { steps: k as f64 * 2.0, len: 5000.0, meta: Meta { feel: Feel::Normal, meter: 4, shuffle: false } };
                let c = conductor(st, Pose::Calm, crate::mood::ALL[k * 2], 1.0, Some(pos), 0.0);
                for (o, l) in out.iter_mut().zip(c) { o.push_str(&l); o.push('|'); }
            }
            println!("{st}"); for o in &out { println!("{o}"); }
        }
        for st in ["country", "techno"] {
            let mut out = vec![String::new(); 4];
            for (i, fam) in FAMS.iter().enumerate() {
                let pin = PlayerIn { style: st, fam: *fam, state: if i == 14 { PState::Rest(0) } else { PState::Play }, hit: Some((0.05, 40 + i as i32 * 3, 110, i as u64)), beats: 0.2, swap: None, walk: 0.0, quoting: i == 3, cond_mood: Mood::Content, t: 0.0, surge: false };
                for (o, l) in out.iter_mut().zip(player(&pin, 4)) { o.push_str(&l); o.push('|'); }
            }
            println!("{st}"); for o in &out { println!("{o}"); }
        }
    }
}
