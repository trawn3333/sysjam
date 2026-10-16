//! The big conductor for roomy terminals: a 9 × 22 figure with brows, eyes and a
//! mouth that talks, two expressive arms (the baton beating the real conducting
//! pattern, the other hand pointing, shushing, fist-pumping, face-palming, waving,
//! hand on heart…) and footwork (weight shifts, toe taps, kicks, stomps, tiptoes),
//! all chosen from his mood and locked to the beat. Small terminals keep the
//! 5 × 12 figure in figure.rs. Same costumes per style (hats, sides, shades, props),
//! and the same width-safe glyph set.

use crate::clock::{Feel, Pos};
use crate::conductor::Pose;
use crate::figure::gear;
use crate::mood::Mood;

pub const BW: usize = 22;
pub const BH: usize = 9;

/// what drives one frame
pub struct In<'a> {
    pub style: &'a str,
    pub pose: Pose,
    pub mood: Mood,
    pub energy: f32,
    pub pos: Option<Pos>,
    pub t: f32,
    /// he just said something: the mouth moves for a moment
    pub talking: bool,
    /// his runner-up feeling (the brain's second strongest output): its gestures show
    /// on alternate bars
    pub mood2: Option<Mood>,
    /// rows to fill, BH..=BH_MAX: taller panels get longer legs
    pub h: usize,
}

/// the tallest he gets (longer legs)
pub const BH_MAX: usize = BH + 3;

struct G {
    c: Vec<Vec<char>>,
}
impl G {
    fn new() -> G {
        G { c: vec![vec![' '; BW]; BH] }
    }
    fn set(&mut self, x: i32, y: i32, ch: char) {
        if x >= 0 && y >= 0 && (x as usize) < BW && (y as usize) < BH {
            self.c[y as usize][x as usize] = ch;
        }
    }
    /// spaces are transparent
    fn put(&mut self, x: i32, y: i32, s: &str) {
        for (k, ch) in s.chars().enumerate() {
            if ch != ' ' {
                self.set(x + k as i32, y, ch);
            }
        }
    }
    /// spaces overwrite
    fn put_solid(&mut self, x: i32, y: i32, s: &str) {
        for (k, ch) in s.chars().enumerate() {
            self.set(x + k as i32, y, ch);
        }
    }
    /// a limb from `a` to `b`, the end cell drawn as `tip`
    fn limb(&mut self, a: (i32, i32), b: (i32, i32), tip: char) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let n = dx.abs().max(dy.abs());
        let at = |k: i32| -> (i32, i32) {
            (a.0 + (dx as f32 * k as f32 / n.max(1) as f32).round() as i32, a.1 + (dy as f32 * k as f32 / n.max(1) as f32).round() as i32)
        };
        for k in 0..n {
            let (p, q) = (at((k - 1).max(0)), at(k));
            let (nx, ny) = at(k + 1);
            // the slope through this cell, from its neighbours
            let (sx, sy) = ((nx - p.0).signum(), (ny - p.1).signum());
            let ch = match (sx, sy) {
                (_, 0) => '─',
                (0, _) => '│',
                (a, b) if a * b < 0 => '╱',
                _ => '╲',
            };
            self.set(q.0, q.1, ch);
        }
        self.set(b.0, b.1, tip);
    }
    fn rows(self) -> Vec<String> {
        self.c.into_iter().map(|r| r.into_iter().collect()).collect()
    }
}

/// head: hat row 0 over columns 6..14, the face (rows 1-3) in columns 7..13
const HX: i32 = 7;
/// shoulder joints
const LS: (i32, i32) = (7, 4);
const RS: (i32, i32) = (13, 4);

/// the face for a mood: brows, eyes (l, r), mouth (5 cells each, the inside of the head)
struct Face {
    brows: &'static str,
    eyes: (char, char),
    mouth: &'static str,
}

fn face(m: Mood) -> Face {
    use Mood::*;
    let f = |brows, l, r, mouth| Face { brows, eyes: (l, r), mouth };
    match m {
        Content => f("     ", '^', '^', " \\_/ "),
        Delighted => f(" ^ ^ ", '^', '^', "\\___/"),
        Proud => f(" - - ", '-', '-', "  \\_/"),
        Amused => f(" ^ - ", '^', '^', " \\w/ "),
        Inspired => f(" ^ ^ ", '*', '*', " (O) "),
        Nostalgic => f(" ~ ~ ", '-', '-', "  ~  "),
        Focused => f(" _ _ ", 'o', 'o', "  -  "),
        Impatient => f(" \\ / ", '-', '-', " --- "),
        Frustrated => f("\\_ _/", '>', '<', " /^\\ "),
        Annoyed => f(" \\ / ", '=', '=', "  _  "),
        Bored => f(" - - ", '.', '.', " ___ "),
        Sleepy => f("     ", '-', '-', "  o  "),
        Anxious => f("_/ \\_", 'o', 'o', " ~~~ "),
        Surprised => f(" ^ ^ ", 'O', 'O', "  O  "),
        Tender => f(" ~ ~ ", '^', '^', "· u ·"),
        Melancholy => f("_/ \\_", 'T', 'T', " /-\\ "),
    }
}

/// what the free (left) hand does
#[derive(Clone, Copy, PartialEq, Debug)]
enum Hand {
    Down,
    Palm,
    Reach,
    Fist,
    Facepalm,
    Shh,
    Point,
    Hip,
    Heart,
    Chin,
    Scratch,
    Watch,
    Wave,
    Bite,
    Slap,
    Wipe,
    Cup,
}

/// how the feet go
#[derive(Clone, Copy, PartialEq, Debug)]
enum Feet {
    Sway,
    Kick,
    Bounce,
    Stomp,
    Tiptoe,
    Cross,
    Jitter,
    Tap,
    Still,
    Sit,
}

/// the baton arm
#[derive(Clone, Copy, PartialEq, Debug)]
enum Baton {
    Beat,
    Up,
    Droop,
    Lap,
}

fn hat(style: &str) -> &'static str {
    match style {
        "monk" => " ╭─────╮ ",
        "jazz" => "  _===_  ",
        "blues" => " _┌─#─┐_ ",
        "country" => "~~╭─^─╮~~",
        "biker" => " ~~~~~~~ ",
        "metal" => " ╭~~~~~╮ ",
        "baroque" => " @@@@@@@ ",
        "celtic" => " _╭────┐ ",
        "industrial" => " ┌┬─┬─┬┐ ",
        "techno" => " ╭─────╮ ",
        "space" => " ╭──*──╮ ",
        "tribal" => " \\\\|||// ",
        _ => "         ",
    }
}

/// eyes stay behind sunglasses while he's cool; strong feelings take them off
fn cool(m: Mood) -> bool {
    matches!(m, Mood::Content | Mood::Proud | Mood::Focused | Mood::Impatient | Mood::Bored | Mood::Tender | Mood::Amused)
}

fn draw_head(g: &mut G, style: &str, m: Mood, beat: i64, frac: f32, talking: bool, shh: bool) {
    let ge = gear(style);
    g.put(HX - 1, 0, hat(style));
    let f = face(m);
    let (l, r) = ge.sides;
    let (el, er) = f.eyes;
    // a blink now and then (not while delighted or surprised)
    let blink = beat.rem_euclid(7) == 3 && frac > 0.8 && !matches!(m, Mood::Surprised | Mood::Sleepy | Mood::Delighted);
    let (el, er) = if blink { ('-', '-') } else { (el, er) };
    let brows = f.brows;
    let eyes = if ge.shades && cool(m) { "[=-=]".to_string() } else { format!(" {el} {er} ") };
    let mut mouth = f.mouth.to_string();
    if talking && frac.rem_euclid(0.5) < 0.25 {
        mouth = if matches!(m, Mood::Frustrated | Mood::Annoyed | Mood::Delighted | Mood::Inspired | Mood::Surprised) { "  O  ".into() } else { "  o  ".into() };
    }
    if shh {
        mouth = "  │  ".into();
    }
    if m == Mood::Sleepy && frac < 0.5 {
        mouth = "  .  ".into();
    }
    g.put_solid(HX, 1, &format!("{l}{brows}{r}"));
    g.put_solid(HX, 2, &format!("{l}{eyes}{r}"));
    g.put_solid(HX, 3, &format!("{l}{mouth}{r}"));
    if style == "metal" {
        // long hair down past the shoulders
        g.set(HX - 1, 2, '│');
        g.set(HX + 7, 2, '│');
        g.set(HX - 1, 3, '│');
        g.set(HX + 7, 3, '│');
    }
    if style == "baroque" {
        // wig curls
        g.set(HX - 1, 1, '@');
        g.set(HX + 7, 1, '@');
        g.set(HX - 1, 2, '@');
        g.set(HX + 7, 2, '@');
    }
}

fn draw_body(g: &mut G, robe: char, sitting: bool) {
    g.put_solid(8, 4, &format!("╭┤{robe}├╮"));
    g.put_solid(8, 5, &format!("│{robe}{robe}{robe}│"));
    g.put_solid(8, 6, if sitting { "└┬─┬┘" } else { "└┬─┬┘" });
}

fn draw_feet(g: &mut G, feet: Feet, beat: i64, frac: f32) {
    let on = frac < 0.25;
    let two = beat.rem_euclid(4) < 2;
    let (a, b) = match feet {
        Feet::Still => ("  │ │  ", " ─┘ └─ "),
        Feet::Sway => match (two, on) {
            (true, true) => ("  │ ╲  ", " ─┘  ╰─"),
            (true, false) => ("  │ │  ", " ─┘ └─ "),
            (false, true) => ("  ╱ │  ", "─╯  └─ "),
            (false, false) => ("  │ │  ", " ─┘ └─ "),
        },
        Feet::Tap => {
            if on {
                ("  │ │  ", " ─┘ └╯ ")
            } else {
                ("  │ │  ", " ─┘ └─ ")
            }
        }
        Feet::Kick => match beat.rem_euclid(4) {
            1 if on => ("  │ ╲__", " ─┘    "),
            3 if on => ("__╱ │  ", "    └─ "),
            _ if frac < 0.5 => ("  │ │  ", " ─┘ └─ "),
            _ => ("  ╱ ╲  ", " ─┘ └─ "),
        },
        Feet::Bounce => {
            if frac < 0.5 {
                (" ╱   ╲ ", "─┘   └─")
            } else {
                ("  │ │  ", " ─┘ └─ ")
            }
        }
        Feet::Stomp => match (beat.rem_euclid(2) == 0, on) {
            (true, true) => ("  │ │  ", "*─┴ └─ "),
            (false, true) => ("  │ │  ", " ─┘ ┴─*"),
            (true, false) => ("  │ ╱  ", " ─┘ ╯  "),
            (false, false) => ("  ╲ │  ", "  ╰ └─ "),
        },
        Feet::Tiptoe => {
            if frac < 0.5 {
                ("  │ │  ", "  ┴ ┴  ")
            } else {
                ("  │ │  ", " ─┘ └─ ")
            }
        }
        Feet::Cross => {
            if two {
                ("   ╲╱  ", "  _╱╲_ ")
            } else {
                ("   ╱╲  ", "  _╱╲_ ")
            }
        }
        Feet::Jitter => {
            if frac.rem_euclid(0.5) < 0.25 {
                ("  ╲ ╱  ", " ─┘ └─ ")
            } else {
                ("  ╱ ╲  ", " ─┘ └─ ")
            }
        }
        Feet::Sit => {
            // on a stool, one foot swinging
            if two {
                ("┌─┴─┴─┐", "│ ╯ │ │")
            } else {
                ("┌─┴─┴─┐", "│ │ ╰ │")
            }
        }
    };
    g.put_solid(7, 7, a);
    g.put_solid(7, 8, b);
}

/// the free hand: a limb from the left shoulder plus its glyph, animated on the beat
fn draw_hand(g: &mut G, h: Hand, beat: i64, frac: f32, prop: char) {
    let on = frac < 0.3;
    let even = beat.rem_euclid(2) == 0;
    match h {
        Hand::Down => g.limb(LS, (5, 6), 'o'),
        Hand::Palm => {
            // open palm out, pressing the band gently on 2 and 4
            let y = if on && !even { 5 } else { 4 };
            g.limb(LS, (4, y), 'o');
        }
        Hand::Reach => g.limb(LS, if even { (3, 0) } else { (2, 1) }, 'w'),
        Hand::Fist => {
            // pumped on every beat
            if on {
                g.limb(LS, (4, 0), '@');
            } else {
                g.limb(LS, (3, 2), '@');
            }
        }
        Hand::Facepalm => {
            g.set(6, 4, '╲');
            g.set(6, 3, '│');
            g.put(HX + 1, 2, "@@");
        }
        Hand::Shh => {
            g.set(6, 4, '╲');
            g.set(6, 3, '╲');
            g.set(HX + 3, 4, 'o');
            g.put(16, 1, if frac < 0.5 { "shh" } else { "sh." });
        }
        Hand::Point => {
            let len = if frac < 0.5 { 6 } else { 5 };
            g.limb(LS, (LS.0 - len, 4), '<');
        }
        Hand::Hip => {
            g.set(6, 5, '<');
            g.set(7, 6, '╯');
        }
        Hand::Heart => {
            g.set(7, 5, '╰');
            g.put(9, 5, if frac < 0.5 { "<3" } else { "<3" });
            if even && on {
                g.set(5, 4, '·');
            }
        }
        Hand::Chin => {
            g.set(6, 5, '╰');
            g.set(7, 4, '╮');
            g.set(HX + 2, 4, '@');
        }
        Hand::Scratch => {
            g.set(6, 3, '│');
            g.set(6, 2, '│');
            g.set(if frac < 0.5 { 6 } else { 7 }, 0, '@');
            g.set(6, 1, '╱');
        }
        Hand::Watch => {
            // a look at the wrist on the downbeat
            if beat.rem_euclid(4) == 0 {
                g.set(6, 4, '─');
                g.set(5, 3, '╱');
                g.put(3, 2, "[o]");
            } else {
                g.set(6, 5, '<');
                g.set(7, 6, '╯');
            }
        }
        Hand::Wave => {
            let x = if even { 2 } else { 4 };
            g.limb(LS, (x, 1), 'w');
            if on {
                g.set(x - 1, 0, prop);
            }
        }
        Hand::Bite => {
            g.set(6, 4, '╲');
            g.set(6, 3, '│');
            g.set(HX + 1, 3, if frac.rem_euclid(0.5) < 0.25 { '@' } else { 'o' });
        }
        Hand::Slap => {
            // slapping the knee on the beat
            if on {
                g.limb(LS, (6, 6), 'w');
            } else {
                g.limb(LS, (4, 5), 'o');
            }
        }
        Hand::Wipe => {
            if beat.rem_euclid(4) == 2 {
                g.set(6, 4, '╲');
                g.set(6, 3, '│');
                g.set(HX + 1, 2, 'o');
            } else {
                g.limb(LS, (5, 6), 'o');
                g.set(HX + 1, 3, if frac < 0.5 { ',' } else { ' ' });
            }
        }
        Hand::Cup => {
            g.limb(LS, (5, 5), 'c');
            g.put(2, 5, "[_]");
            if frac < 0.5 {
                g.put(2, 4, " ~ ");
            } else {
                g.put(2, 4, "~  ");
            }
        }
    }
}

/// big pattern box: the baton tip from figure::baton's 7..11 × 0..4 grid scaled to
/// columns 14..21 and rows 0..7
fn baton_tip(p: &Pos, energy: f32) -> (i32, i32) {
    let (x, y) = crate::figure::baton(p, energy);
    let bx = 14.0 + (x - 7.0) * 1.75;
    let by = y * 1.75;
    (bx.round() as i32, by.round() as i32)
}

fn draw_baton(g: &mut G, b: Baton, pos: Option<Pos>, energy: f32, frac: f32, tip: char) {
    match b {
        Baton::Beat => {
            let t = pos.map(|p| baton_tip(&p, energy)).unwrap_or((17, 3));
            g.limb(RS, t, tip);
        }
        Baton::Up => {
            if frac < 0.5 {
                g.limb(RS, (18, 0), tip);
            } else {
                g.limb(RS, (20, 2), tip);
            }
        }
        Baton::Droop => g.limb(RS, (16, 6), tip),
        Baton::Lap => g.limb(RS, (15, 6), '─'),
    }
}

/// mood marks around the head
fn marks(g: &mut G, m: Mood, beat: i64, frac: f32) {
    use Mood::*;
    match m {
        Delighted | Inspired => {
            if beat % 2 == 0 {
                g.set(5, 0, '*');
            } else {
                g.set(15, 1, '*');
            }
        }
        Proud => {
            if frac < 0.3 {
                g.set(15, 0, '*');
            }
        }
        Amused => g.put(15, 1, if beat % 2 == 0 { "ha" } else { " ha" }),
        Nostalgic => g.set(15, if beat % 2 == 0 { 0 } else { 1 }, '♪'),
        Frustrated => g.put(15, 0, if frac < 0.5 { "#" } else { "#!" }),
        Annoyed => g.set(15, 0, '#'),
        Bored => g.put(15, 1, if frac < 0.5 { ".." } else { "..." }),
        Sleepy => g.put(15, 0, if frac < 0.5 { "z" } else { " zZ" }),
        Anxious => g.set(14, 1, '\''),
        Surprised => g.set(15, 0, '!'),
        Melancholy => {}
        Impatient => {
            if frac < 0.25 {
                g.set(15, 1, '-');
            }
        }
        _ => {}
    }
}

/// the gestures his mood leads to
fn lean(m: Mood, bar: i64) -> (Hand, Feet, Baton) {
    use Mood::*;
    let alt = bar.rem_euclid(2) == 0;
    match m {
        Content => (Hand::Palm, Feet::Sway, Baton::Beat),
        Delighted => (if alt { Hand::Wave } else { Hand::Fist }, Feet::Kick, Baton::Beat),
        Proud => (Hand::Hip, Feet::Tap, Baton::Beat),
        Amused => (Hand::Slap, Feet::Bounce, Baton::Beat),
        Inspired => (Hand::Reach, Feet::Tiptoe, Baton::Beat),
        Nostalgic => (Hand::Heart, Feet::Sway, Baton::Beat),
        Focused => (Hand::Palm, Feet::Still, Baton::Beat),
        Impatient => (Hand::Watch, Feet::Tap, Baton::Beat),
        Frustrated => (if alt { Hand::Fist } else { Hand::Facepalm }, Feet::Stomp, Baton::Beat),
        Annoyed => (if bar.rem_euclid(4) == 0 { Hand::Facepalm } else { Hand::Hip }, Feet::Tap, Baton::Beat),
        Bored => (if alt { Hand::Chin } else { Hand::Scratch }, Feet::Cross, Baton::Beat),
        Sleepy => (Hand::Down, Feet::Still, Baton::Droop),
        Anxious => (Hand::Bite, Feet::Jitter, Baton::Beat),
        Surprised => (Hand::Reach, Feet::Tiptoe, Baton::Beat),
        Tender => (Hand::Heart, Feet::Sway, Baton::Beat),
        Melancholy => (Hand::Wipe, Feet::Still, Baton::Beat),
    }
}

/// the 9 × 22 conductor for this instant
pub fn conductor(i: &In) -> Vec<String> {
    stretch(conductor9(i), i.h)
}

/// longer legs for a taller panel: thigh rows go in between the hips and the knees
fn stretch(rows: Vec<String>, h: usize) -> Vec<String> {
    let extra = h.clamp(BH, BH_MAX) - BH;
    if extra == 0 {
        return rows;
    }
    let mut out = rows;
    let thigh: String = format!("{}│ │{}", " ".repeat(9), " ".repeat(BW - 12));
    for _ in 0..extra {
        out.insert(7, thigh.clone());
    }
    out
}

fn conductor9(i: &In) -> Vec<String> {
    let ge = gear(i.style);
    let beats = i.pos.map(|p| p.beats()).unwrap_or(i.t as f64 * 1.5);
    let frac = beats.fract() as f32;
    let beat = beats.floor() as i64;
    let meter = i.pos.map(|p| p.meta.meter.max(1) as i64).unwrap_or(4);
    let bar = beat.div_euclid(meter);
    let prop = ge.props[(beat.rem_euclid(ge.props.len() as i64)) as usize];
    let tip = '•';
    let surge = i.pose == Pose::Surge || i.pos.map_or(false, |p| p.meta.feel == Feel::Surge);
    let mut g = G::new();
    let (mut hand, mut feet, mut bat) = lean(i.mood, bar);
    if let (Some(m2), true) = (i.mood2, bar.rem_euclid(2) == 1) {
        // the runner-up feeling shows in the free hand and the feet every other bar
        let (h2, f2, _) = lean(m2, bar / 2);
        hand = h2;
        feet = f2;
    }
    match i.pose {
        Pose::Rest => {
            hand = if i.mood == Mood::Sleepy { Hand::Down } else { Hand::Cup };
            feet = Feet::Sit;
            bat = Baton::Lap;
        }
        Pose::Shh => {
            hand = Hand::Shh;
            feet = Feet::Tiptoe;
        }
        Pose::Point => hand = Hand::Point,
        Pose::Surge => {
            bat = Baton::Up;
            if !matches!(hand, Hand::Fist | Hand::Wave | Hand::Reach) {
                hand = Hand::Wave;
            }
            if !matches!(feet, Feet::Stomp | Feet::Kick) {
                feet = Feet::Bounce;
            }
        }
        Pose::Calm => {}
    }
    let calm_or_surge = matches!(i.pose, Pose::Calm | Pose::Surge);
    // the three styles with their own act
    if i.style == "metal" && calm_or_surge {
        return metal(i, beat, frac);
    }
    if i.style == "techno" && calm_or_surge {
        return techno(i, beat, frac, prop);
    }
    if i.style == "industrial" && calm_or_surge {
        return industrial(i, beat, frac);
    }
    let _ = surge;
    draw_head(&mut g, i.style, i.mood, beat, frac, i.talking && i.pose != Pose::Rest, i.pose == Pose::Shh);
    draw_body(&mut g, ge.robe, i.pose == Pose::Rest);
    draw_feet(&mut g, feet, beat, frac);
    draw_hand(&mut g, hand, beat, frac, prop);
    draw_baton(&mut g, bat, i.pos, i.energy, frac, tip);
    if i.pose != Pose::Shh {
        marks(&mut g, i.mood, beat, frac);
    }
    g.rows()
}

/// the metalhead: headbanging on the beat (the hair falls over his face), horns up with
/// both hands, wide stance stomping
fn metal(i: &In, beat: i64, frac: f32) -> Vec<String> {
    let mut g = G::new();
    let down = frac < 0.45;
    if down {
        g.put_solid(HX, 1, "╭~~~~~╮");
        g.put_solid(HX - 1, 2, "╱│││││││╲");
        g.put_solid(HX - 1, 3, "│││││││││");
    } else {
        draw_head(&mut g, "metal", i.mood, beat, frac, i.talking, false);
    }
    draw_body(&mut g, '+', false);
    draw_feet(&mut g, Feet::Stomp, beat, frac);
    if down {
        g.limb(LS, (3, 2), 'm');
        g.put(2, 2, "\\m/");
        g.limb(RS, (17, 2), 'm');
        g.put(16, 2, "\\m/");
    } else {
        g.limb(LS, (2, 4), 'm');
        g.limb(RS, (18, 4), 'm');
    }
    marks(&mut g, i.mood, beat, frac);
    g.rows()
}

/// the DJ: platters spinning on the deck, one hand scratching, the other up on every kick
fn techno(i: &In, beat: i64, frac: f32, prop: char) -> Vec<String> {
    let mut g = G::new();
    draw_head(&mut g, "techno", i.mood, beat, frac, i.talking, false);
    draw_body(&mut g, '=', false);
    draw_feet(&mut g, Feet::Bounce, beat, frac);
    let sp = ['|', '/', '-', '\\'];
    let a = sp[((frac * 4.0) as usize) % 4];
    let b = sp[((frac * 4.0) as usize + 2) % 4];
    g.put_solid(2, 6, &format!("┌({a})──────────({b})┐"));
    g.put_solid(2, 7, "└──────────────────┘");
    // scratching hand on the left platter
    g.limb(LS, (if frac.rem_euclid(0.5) < 0.25 { 4 } else { 3 }, 5), 'o');
    if frac < 0.25 {
        g.limb(RS, (18, 0), 'w');
        g.set(19, 0, prop);
    } else {
        g.limb(RS, (17, 5), 'o');
    }
    marks(&mut g, i.mood, beat, frac);
    g.rows()
}

/// the angry one: a sledgehammer smashed down on the beat, the other fist pumping
fn industrial(i: &In, beat: i64, frac: f32) -> Vec<String> {
    let mut g = G::new();
    draw_head(&mut g, "industrial", i.mood, beat, frac, i.talking, false);
    draw_body(&mut g, '#', false);
    draw_feet(&mut g, Feet::Stomp, beat, frac);
    if frac < 0.3 {
        g.limb(RS, (18, 6), '┐');
        g.put_solid(17, 7, "[##]");
        g.put(16, 8, "*  *");
        g.limb(LS, (3, 4), '@');
    } else {
        g.limb(RS, (17, 1), '┘');
        g.put_solid(17, 0, "[##]");
        g.limb(LS, (4, 0), '@');
    }
    marks(&mut g, i.mood, beat, frac);
    g.rows()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Meta;
    use crate::figure::safe_glyph;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn big_conductor_is_clean_and_fixed_width() {
        let poses = [Pose::Rest, Pose::Calm, Pose::Shh, Pose::Point, Pose::Surge];
        for st in crate::style::STYLES {
            for &pose in &poses {
                for &m in crate::mood::ALL.iter() {
                    for feel in [Feel::Normal, Feel::Half, Feel::Double, Feel::Surge] {
                        for k in 0..24 {
                            let pos = Pos { steps: k as f64 * 0.53, len: 5000.0, meta: Meta { feel, meter: 4, shuffle: st.id == "blues" } };
                            let c = conductor(&In { style: st.id, pose, mood: m, energy: 0.4 + (k % 7) as f32 * 0.1, pos: Some(pos), t: k as f32 * 0.1, talking: k % 2 == 0, mood2: if k % 3 == 0 { Some(crate::mood::ALL[k % 16]) } else { None }, h: BH + k % 4 });
                            assert_eq!(c.len(), BH + k % 4);
                            for l in xl(&c) {
                                assert_eq!(l.width(), 2 * BW);
                                assert!(l.chars().all(safe_glyph), "xl {l:?}");
                            }
                            for l in &c {
                                assert_eq!(l.width(), BW, "{} {pose:?} {m:?}: {l:?}", st.id);
                                assert_eq!(l.chars().count(), BW);
                                assert!(l.chars().all(safe_glyph), "{} {pose:?} {m:?}: {l:?}", st.id);
                            }
                        }
                    }
                }
            }
        }
    }

    /// `cargo test --release big_sheet -- --ignored --nocapture` prints a contact sheet
    #[test]
    #[ignore]
    fn big_sheet() {
        let st = std::env::var("SYSJAM_STYLE").unwrap_or("country".into());
        let pose = match std::env::var("SYSJAM_POSE").as_deref() {
            Ok("rest") => Pose::Rest,
            Ok("shh") => Pose::Shh,
            Ok("point") => Pose::Point,
            Ok("surge") => Pose::Surge,
            _ => Pose::Calm,
        };
        for half in crate::mood::ALL.chunks(6) {
            let mut rows = vec![String::new(); BH + 1];
            for (k, m) in half.iter().enumerate() {
                let pos = Pos { steps: 2.0 + k as f64 * 1.7, len: 5000.0, meta: Meta { feel: Feel::Normal, meter: 4, shuffle: false } };
                let c = conductor(&In { style: &st, pose, mood: *m, energy: 0.9, pos: Some(pos), t: 0.0, talking: false, mood2: None, h: BH });
                for (r, l) in c.iter().enumerate() {
                    rows[r].push_str(l);
                    rows[r].push('|');
                }
                rows[BH].push_str(&format!("{:<23}", m.name()));
            }
            for r in rows {
                println!("{r}");
            }
        }
    }
}

/// the extra-large conductor for a very roomy panel: every cell of the big figure drawn as
/// 2 × 2 cells (lines stay joined, eyes and mouths get proper shapes)
pub fn xl(rows: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(rows.len() * 2);
    for r in rows {
        let (mut a, mut b) = (String::new(), String::new());
        for ch in r.chars() {
            let q: [char; 4] = match ch {
                '─' => ['─', '─', ' ', ' '],
                '│' => ['│', ' ', '│', ' '],
                '╭' => ['╭', '─', '│', ' '],
                '┌' => ['┌', '─', '│', ' '],
                '╮' => ['╮', ' ', '│', ' '],
                '┐' => ['┐', ' ', '│', ' '],
                '╰' => ['╰', '─', ' ', ' '],
                '└' => ['└', '─', ' ', ' '],
                '╯' => ['╯', ' ', ' ', ' '],
                '┘' => ['┘', ' ', ' ', ' '],
                '├' => ['├', '─', '│', ' '],
                '┤' => ['┤', ' ', '│', ' '],
                '┬' => ['┬', '─', '│', ' '],
                '┴' => ['┴', '─', ' ', ' '],
                '┼' => ['┼', '─', '│', ' '],
                '╱' => [' ', '╱', '╱', ' '],
                '╲' => ['╲', ' ', ' ', '╲'],
                '/' => [' ', '/', '/', ' '],
                '\\' => ['\\', ' ', ' ', '\\'],
                '(' => ['(', ' ', '(', ' '],
                ')' => [' ', ')', ' ', ')'],
                '[' => ['[', ' ', '[', ' '],
                ']' => [' ', ']', ' ', ']'],
                '{' => ['{', ' ', '{', ' '],
                '}' => [' ', '}', ' ', '}'],
                '|' => ['|', ' ', '|', ' '],
                '~' => ['~', '~', ' ', ' '],
                '_' => [' ', ' ', '_', '_'],
                '-' => ['-', '-', ' ', ' '],
                '=' => ['=', '=', ' ', ' '],
                '^' => ['/', '\\', ' ', ' '],
                'o' => ['(', ')', ' ', ' '],
                'O' => ['/', '\\', '\\', '/'],
                '>' => ['\\', ' ', '/', ' '],
                '<' => [' ', '/', ' ', '\\'],
                '.' => [' ', ' ', '.', ' '],
                ',' => [' ', ' ', ',', ' '],
                '\'' => ['\'', ' ', ' ', ' '],
                'w' => ['\\', '/', '\\', '/'],
                'v' => ['\\', '/', ' ', ' '],
                'T' => ['-', '-', ':', ' '],
                '@' => ['@', '@', '@', '@'],
                'm' => ['m', 'm', ' ', ' '],
                ' ' => [' ', ' ', ' ', ' '],
                '#' | 'x' | '+' | '%' | ':' => [ch, ch, ch, ch],
                c => [c, ' ', ' ', ' '],
            };
            a.push(q[0]);
            a.push(q[1]);
            b.push(q[2]);
            b.push(q[3]);
        }
        out.push(a);
        out.push(b);
    }
    out
}

#[cfg(test)]
mod xl_tests {
    use super::*;
    use crate::clock::Meta;
    #[test]
    #[ignore]
    fn xl_sheet() {
        let st = std::env::var("SYSJAM_STYLE").unwrap_or("country".into());
        for m in [Mood::Content, Mood::Frustrated, Mood::Surprised, Mood::Melancholy] {
            let pos = Pos { steps: 2.0, len: 5000.0, meta: Meta { feel: Feel::Normal, meter: 4, shuffle: false } };
            let c = conductor(&In { style: &st, pose: Pose::Calm, mood: m, energy: 0.9, pos: Some(pos), t: 0.0, talking: false, mood2: None, h: BH });
            for l in xl(&c) {
                println!("{l}|");
            }
        }
    }
}
