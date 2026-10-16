//! TUI look and feel per conductor style: palette, borders, title decorations,
//! icons, meter glyphs, conductor outfit and props, speech marks. The whole UI
//! fades from one theme to the next over about a second.
//! Respects NO_COLOR (crossterm drops colours), falls back to the 256-colour cube
//! unless COLORTERM says truecolor, and `--ascii` (or TERM=linux) swaps every
//! glyph for plain ASCII.

use ratatui::style::Color;
use ratatui::symbols::border;
use std::cell::RefCell;
use std::time::Instant;

type Rgb = (u8, u8, u8);

#[derive(Clone, Copy)]
pub struct Pal {
    pub bg: Rgb,
    pub fg: Rgb,
    pub text: Rgb,
    pub dim: Rgb,
    pub acc: Rgb,
    pub acc2: Rgb,
    pub hot: Rgb,
    pub good: Rgb,
    pub mag: Rgb,
    pub cool: Rgb,
    pub border: Rgb,
    /// text drawn on an accent background
    pub ink: Rgb,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Brd {
    Plain,
    Rounded,
    Double,
    Thick,
    Rope,
    Knot,
    Hazard,
    Minimal,
}

pub struct Theme {
    pub id: &'static str,
    /// None = the terminal's own colours (House Band)
    pub pal: Option<Pal>,
    pub brd: Brd,
    /// title decorations (unicode, ascii)
    pub deco: (&'static str, &'static str),
    pub deco_ascii: (&'static str, &'static str),
    /// meter glyphs: full, empty
    pub meter: (char, char),
    /// meters pulse with the kick
    pub pulse: bool,
    /// outfit sides around the conductor's face
    pub sides: (&'static str, &'static str),
    /// speech marks
    pub marks: (&'static str, &'static str),
    /// header badge icon (unicode, ascii)
    pub icon: (&'static str, &'static str),
}

const HOUSE_PAL: Pal = Pal {
    bg: (24, 24, 30),
    fg: (240, 240, 240),
    text: (204, 204, 204),
    dim: (102, 102, 102),
    acc: (245, 245, 67),
    acc2: (41, 184, 219),
    hot: (241, 76, 76),
    good: (35, 209, 139),
    mag: (214, 112, 214),
    cool: (59, 142, 234),
    border: (204, 204, 204),
    ink: (0, 0, 0),
};

pub const THEMES: &[Theme] = &[
    Theme { id: "house", pal: None, brd: Brd::Plain, deco: ("", ""), deco_ascii: ("", ""), meter: ('█', ' '), pulse: false, sides: ("", ""), marks: ("“", "”"), icon: ("♫", "#") },
    Theme {
        id: "monk",
        pal: Some(Pal { bg: (30, 26, 40), fg: (234, 226, 248), text: (196, 186, 216), dim: (112, 102, 134), acc: (204, 176, 244), acc2: (176, 194, 242), hot: (232, 164, 204), good: (184, 222, 204), mag: (212, 172, 232), cool: (164, 174, 232), border: (92, 82, 118), ink: (30, 26, 40) }),
        brd: Brd::Minimal,
        deco: ("~ ", ""),
        deco_ascii: ("o ", ""),
        meter: ('━', '┄'),
        pulse: false,
        sides: ("", ""),
        marks: ("〜 ", " 〜"),
        icon: ("~", "o"),
    },
    Theme {
        id: "jazz",
        pal: Some(Pal { bg: (32, 21, 10), fg: (255, 238, 204), text: (228, 198, 152), dim: (132, 102, 66), acc: (255, 178, 60), acc2: (242, 142, 62), hot: (222, 92, 52), good: (232, 202, 92), mag: (204, 124, 84), cool: (172, 142, 102), border: (164, 112, 52), ink: (32, 21, 10) }),
        brd: Brd::Rounded,
        deco: ("🎷 ", ""),
        deco_ascii: ("~ ", ""),
        meter: ('█', '░'),
        pulse: false,
        sides: ("", ""),
        marks: ("“", "” ♪"),
        icon: ("🎷", "~"),
    },
    Theme {
        id: "blues",
        pal: Some(Pal { bg: (14, 16, 42), fg: (212, 222, 255), text: (162, 177, 222), dim: (82, 92, 142), acc: (112, 152, 255), acc2: (142, 192, 232), hot: (202, 112, 172), good: (122, 172, 222), mag: (152, 122, 232), cool: (92, 132, 212), border: (62, 77, 152), ink: (14, 16, 42) }),
        brd: Brd::Rounded,
        deco: ("♪ ", " ♪"),
        deco_ascii: ("~ ", " ~"),
        meter: ('▓', '░'),
        pulse: false,
        sides: ("", ""),
        marks: ("“", "…”"),
        icon: ("♪", "~"),
    },
    Theme {
        id: "country",
        pal: Some(Pal { bg: (56, 40, 24), fg: (247, 230, 198), text: (224, 198, 152), dim: (146, 116, 80), acc: (234, 152, 72), acc2: (216, 188, 132), hot: (198, 92, 52), good: (176, 166, 84), mag: (204, 124, 92), cool: (164, 134, 98), border: (176, 124, 70), ink: (44, 30, 16) }),
        brd: Brd::Rope,
        deco: ("🌵 ", " ★"),
        deco_ascii: ("* ", " *"),
        meter: ('▬', '·'),
        pulse: false,
        sides: ("", ""),
        marks: ("★ “", "”"),
        icon: ("🐎", "*"),
    },
    Theme {
        id: "biker",
        pal: Some(Pal { bg: (12, 12, 12), fg: (226, 226, 232), text: (182, 182, 192), dim: (102, 102, 108), acc: (255, 122, 22), acc2: (202, 202, 212), hot: (255, 62, 22), good: (255, 172, 42), mag: (232, 92, 42), cool: (152, 152, 162), border: (172, 172, 182), ink: (12, 12, 12) }),
        brd: Brd::Thick,
        deco: ("🔥 ", ""),
        deco_ascii: ("^ ", ""),
        meter: ('▲', '_'),
        pulse: false,
        sides: ("", ""),
        marks: ("“", "!”"),
        icon: ("🔥", "^"),
    },
    Theme {
        id: "metal",
        // black, blood crimson and cold steel
        pal: Some(Pal { bg: (8, 6, 8), fg: (222, 220, 228), text: (170, 166, 178), dim: (88, 82, 94), acc: (204, 22, 36), acc2: (168, 176, 192), hot: (255, 40, 40), good: (190, 60, 70), mag: (150, 30, 60), cool: (120, 126, 142), border: (150, 18, 30), ink: (8, 6, 8) }),
        brd: Brd::Thick,
        deco: ("\\m/ ", " \\m/"),
        deco_ascii: ("\\m/ ", " \\m/"),
        meter: ('█', '·'),
        pulse: false,
        sides: ("", ""),
        marks: ("“", "!”"),
        icon: ("╳", "X"),
    },
    Theme {
        id: "baroque",
        pal: Some(Pal { bg: (250, 243, 224), fg: (62, 42, 20), text: (102, 76, 42), dim: (168, 142, 102), acc: (192, 142, 30), acc2: (152, 102, 42), hot: (162, 52, 42), good: (112, 122, 42), mag: (142, 72, 92), cool: (92, 92, 132), border: (192, 152, 62), ink: (250, 243, 224) }),
        brd: Brd::Double,
        deco: ("♪ ", " ♪"),
        deco_ascii: ("% ", " %"),
        meter: ('█', '·'),
        pulse: false,
        sides: ("§", "§"),
        marks: ("« ", " »"),
        icon: ("♫", "%"),
    },
    Theme {
        id: "celtic",
        pal: Some(Pal { bg: (10, 32, 18), fg: (226, 246, 226), text: (172, 212, 176), dim: (82, 122, 92), acc: (122, 222, 122), acc2: (202, 222, 122), hot: (232, 152, 62), good: (92, 202, 112), mag: (162, 202, 142), cool: (112, 182, 162), border: (62, 152, 82), ink: (10, 32, 18) }),
        brd: Brd::Knot,
        deco: ("* ", " *"),
        deco_ascii: ("& ", " &"),
        meter: ('■', '·'),
        pulse: false,
        sides: ("", ""),
        marks: ("“", "”"),
        icon: ("*", "&"),
    },
    Theme {
        id: "industrial",
        // rust, blood red and black; hazard yellow for the stripes
        pal: Some(Pal { bg: (14, 7, 5), fg: (236, 214, 200), text: (190, 150, 130), dim: (104, 64, 52), acc: (206, 84, 30), acc2: (230, 190, 30), hot: (236, 32, 20), good: (196, 120, 60), mag: (184, 60, 50), cool: (140, 110, 100), border: (200, 80, 30), ink: (14, 7, 5) }),
        brd: Brd::Hazard,
        deco: ("╱╱ ", " ╱╱"),
        deco_ascii: ("// ", " //"),
        meter: ('█', '╱'),
        pulse: false,
        sides: ("[", "]"),
        marks: ("> ", ""),
        icon: ("#", "#"),
    },
    Theme {
        id: "techno",
        pal: Some(Pal { bg: (5, 5, 12), fg: (232, 242, 255), text: (152, 172, 202), dim: (72, 72, 102), acc: (0, 255, 242), acc2: (255, 0, 202), hot: (255, 42, 142), good: (0, 255, 162), mag: (222, 62, 255), cool: (62, 142, 255), border: (0, 202, 222), ink: (5, 5, 12) }),
        brd: Brd::Thick,
        deco: ("♫ ", " ♫"),
        deco_ascii: ("o ", " o"),
        meter: ('▮', '▯'),
        pulse: true,
        sides: ("◖", "◗"),
        marks: ("» ", " «"),
        icon: ("♫", "o"),
    },
    Theme {
        id: "space",
        pal: Some(Pal { bg: (4, 6, 22), fg: (222, 232, 255), text: (162, 172, 222), dim: (72, 82, 132), acc: (182, 142, 255), acc2: (112, 202, 255), hot: (255, 122, 182), good: (142, 255, 212), mag: (202, 122, 255), cool: (102, 142, 255), border: (82, 92, 172), ink: (4, 6, 22) }),
        brd: Brd::Rounded,
        deco: ("* ", " *"),
        deco_ascii: ("+ ", " +"),
        meter: ('•', '·'),
        pulse: false,
        sides: ("│", "│"),
        marks: ("< ", " >"),
        icon: ("🚀", "+"),
    },
    Theme {
        id: "tribal",
        pal: Some(Pal { bg: (42, 25, 12), fg: (247, 227, 182), text: (217, 182, 122), dim: (132, 97, 62), acc: (227, 172, 52), acc2: (192, 112, 52), hot: (202, 82, 42), good: (182, 162, 72), mag: (172, 92, 62), cool: (152, 122, 82), border: (172, 122, 52), ink: (42, 25, 12) }),
        brd: Brd::Double,
        deco: ("<> ", " <>"),
        deco_ascii: ("<> ", " <>"),
        meter: ('◆', '◇'),
        pulse: false,
        sides: ("", ""),
        marks: ("“", "”"),
        icon: ("<>", "<>"),
    },
];

pub fn by_id(id: &str) -> usize {
    THEMES.iter().position(|t| t.id == id).unwrap_or(0)
}

struct State {
    from: usize,
    to: usize,
    t0: Instant,
    /// fade length in s (short right after a user key press)
    dur: f32,
    beat: u32,
    truecolor: bool,
}

static ASCII: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// set by ←/→: the next theme change snaps over ~150 ms instead of fading over 1 s
static FAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn fast_next() {
    FAST.store(epoch_ms() + 3000, std::sync::atomic::Ordering::Relaxed);
}
fn epoch_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

thread_local! {
    static ST: RefCell<State> = RefCell::new(State {
        from: 0,
        to: 0,
        t0: Instant::now(),
        dur: 1.0,
        beat: 0,
        truecolor: matches!(std::env::var("COLORTERM").as_deref(), Ok("truecolor") | Ok("24bit")),
    });
}

pub fn set_ascii(a: bool) {
    ASCII.store(a, std::sync::atomic::Ordering::Relaxed);
}

pub fn ascii() -> bool {
    ASCII.load(std::sync::atomic::Ordering::Relaxed)
}

/// once per frame: the style to show and the beat (for pulsing)
pub fn frame(style_id: &str, beat: u32) {
    let k = by_id(style_id);
    ST.with(|s| {
        let mut s = s.borrow_mut();
        s.beat = beat;
        if k != s.to {
            s.from = s.to;
            s.to = k;
            s.t0 = Instant::now();
            s.dur = if FAST.swap(0, std::sync::atomic::Ordering::Relaxed) > epoch_ms() { 0.15 } else { 1.0 };
        }
    });
}

/// 0..1 progress of the theme fade (1 s)
pub fn fading() -> bool {
    ST.with(|s| {
        let s = s.borrow();
        s.from != s.to && s.t0.elapsed().as_secs_f32() < s.dur
    })
}

pub fn cur() -> &'static Theme {
    ST.with(|s| &THEMES[s.borrow().to])
}

fn pal_of(k: usize) -> Pal {
    THEMES[k].pal.unwrap_or(HOUSE_PAL)
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// nearest xterm-256 cube / grey entry
fn to256(c: Rgb) -> u8 {
    let q = |v: u8| if v < 48 { 0 } else if v < 115 { 1 } else { ((v as u16 - 35) / 40) as u8 };
    let lv = [0u8, 95, 135, 175, 215, 255];
    let (r, g, b) = (q(c.0), q(c.1), q(c.2));
    let cube = (lv[r as usize], lv[g as usize], lv[b as usize]);
    let avg = (c.0 as u16 + c.1 as u16 + c.2 as u16) / 3;
    let gi = if avg < 8 { 0 } else { ((avg - 8) / 10).min(23) as u8 };
    let gv = 8 + 10 * gi as u16;
    let d = |x: Rgb| (x.0 as i32 - c.0 as i32).pow(2) + (x.1 as i32 - c.1 as i32).pow(2) + (x.2 as i32 - c.2 as i32).pow(2);
    if d((gv as u8, gv as u8, gv as u8)) < d(cube) { 232 + gi } else { 16 + 36 * r + 6 * g + b }
}

fn out(c: Rgb) -> Color {
    if ST.with(|s| s.borrow().truecolor) {
        Color::Rgb(c.0, c.1, c.2)
    } else {
        Color::Indexed(to256(c))
    }
}

/// the colour role for a named colour in the current (fading) theme
fn role(p: &Pal, c: Color) -> Option<Rgb> {
    Some(match c {
        Color::White => p.fg,
        Color::Gray => p.text,
        Color::DarkGray => p.dim,
        Color::LightYellow | Color::Yellow => p.acc,
        Color::LightCyan | Color::Cyan => p.acc2,
        Color::LightRed | Color::Red => p.hot,
        Color::LightGreen | Color::Green => p.good,
        Color::LightMagenta | Color::Magenta => p.mag,
        Color::LightBlue | Color::Blue => p.cool,
        Color::Black => p.ink,
        _ => return None,
    })
}

/// map a named colour through the theme (House Band keeps the terminal's colours)
pub fn th(c: Color) -> Color {
    let (from, to, t) = ST.with(|s| {
        let s = s.borrow();
        (s.from, s.to, (s.t0.elapsed().as_secs_f32() / s.dur).min(1.0))
    });
    if THEMES[to].pal.is_none() && (t >= 1.0 || from == to) {
        return c;
    }
    let (a, b) = (pal_of(from), pal_of(to));
    match (role(&a, c), role(&b, c)) {
        (Some(x), Some(mut y)) => {
            // techno: meters flash brighter on the kick
            if THEMES[to].pulse && matches!(c, Color::LightGreen | Color::Green | Color::LightRed) && ST.with(|s| s.borrow().beat % 2 == 0) {
                y = mix(y, (255, 255, 255), 0.45);
            }
            out(if from == to { y } else { mix(x, y, t) })
        }
        _ => c,
    }
}

/// whole-screen background
pub fn bg() -> Color {
    let (from, to, t) = ST.with(|s| {
        let s = s.borrow();
        (s.from, s.to, (s.t0.elapsed().as_secs_f32() / s.dur).min(1.0))
    });
    if THEMES[to].pal.is_none() && (t >= 1.0 || from == to) {
        return Color::Reset;
    }
    out(mix(pal_of(from).bg, pal_of(to).bg, if from == to { 1.0 } else { t }))
}

/// Industrial: a cheap glitch on the hits of the loud sections (set once per frame)
static GLITCH: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
pub fn set_glitch(level: u8) {
    GLITCH.store(level, std::sync::atomic::Ordering::Relaxed);
}
pub fn glitch() -> u8 {
    GLITCH.load(std::sync::atomic::Ordering::Relaxed)
}

thread_local! {
    /// dim / hazed colours, precomputed per theme: (theme, colour role, level) → colour
    static DIMC: RefCell<(usize, bool, [[Option<Color>; 4]; 12])> = RefCell::new((usize::MAX, false, [[None; 4]; 12]));
}

/// a themed colour blended toward the background: level 0 = as is, 1 / 2 / 3 = further
/// back (haze, a fake translucency). Precomputed per theme, so it costs nothing per cell.
pub fn dim(c: Color, level: u8) -> Color {
    if level == 0 {
        return th(c);
    }
    let level = level.min(3) as usize;
    let to = ST.with(|s| s.borrow().to);
    let truecolor = ST.with(|s| s.borrow().truecolor);
    let idx = match c {
        Color::White => 0,
        Color::Gray => 1,
        Color::DarkGray => 2,
        Color::LightYellow | Color::Yellow => 3,
        Color::LightCyan | Color::Cyan => 4,
        Color::LightRed | Color::Red => 5,
        Color::LightGreen | Color::Green => 6,
        Color::LightMagenta | Color::Magenta => 7,
        Color::LightBlue | Color::Blue => 8,
        _ => 9,
    };
    DIMC.with(|d| {
        let mut d = d.borrow_mut();
        if d.0 != to || d.1 != truecolor {
            *d = (to, truecolor, [[None; 4]; 12]);
        }
        if let Some(x) = d.2[idx][level] {
            return x;
        }
        let p = pal_of(to);
        let x = match role(&p, c) {
            Some(rgb) => {
                let k = [0.0, 0.32, 0.55, 0.72][level];
                // haze: toward the background, a touch toward grey
                let hz = mix(p.bg, p.dim, 0.35);
                out(mix(rgb, hz, k))
            }
            None => c,
        };
        // House Band (terminal colours): back rows just go grey
        let x = if THEMES[to].pal.is_none() { if level >= 2 { Color::DarkGray } else { Color::Gray } } else { x };
        d.2[idx][level] = Some(x);
        x
    })
}

/// the floor / haze background for crowd row `k` of `n` (front rows a little lighter)
pub fn floor_bg(k: usize, n: usize) -> Color {
    let to = ST.with(|s| s.borrow().to);
    if THEMES[to].pal.is_none() {
        return Color::Reset;
    }
    let p = pal_of(to);
    let x = k as f32 / n.max(1) as f32;
    // back rows: haze (a little grey in the dark), front rows: the floor catching stage light
    let haze = mix(p.bg, p.dim, 0.18 * (1.0 - x));
    out(mix(haze, p.acc, 0.07 * x * x))
}

/// stage-light colour for light `i` (theme accents), `on` = on the beat
pub fn light(i: usize, on: bool, section: u8) -> Color {
    let to = ST.with(|s| s.borrow().to);
    let p = pal_of(to);
    let cols = [p.acc, p.acc2, p.hot, p.mag, p.cool, p.good];
    let c = cols[(i + section as usize) % cols.len()];
    out(if on { mix(c, (255, 255, 255), 0.2) } else { mix(c, p.bg, 0.45) })
}

/// a light's colour washed into the background (a translucent beam): `amt` 0..1
pub fn light_bg(c: (u8, u8, u8), amt: f32) -> Color {
    let to = ST.with(|s| s.borrow().to);
    if THEMES[to].pal.is_none() {
        return Color::Reset;
    }
    let p = pal_of(to);
    out(mix(p.bg, c, amt))
}

/// the raw rgb of light `i` (for beams)
pub fn light_rgb(i: usize, section: u8, warm: bool, strobe: bool) -> (u8, u8, u8) {
    let to = ST.with(|s| s.borrow().to);
    let p = pal_of(to);
    if strobe {
        return p.hot;
    }
    if warm {
        return [p.acc, p.acc2][(i + section as usize) % 2];
    }
    let cols = [p.acc, p.acc2, p.hot, p.mag, p.cool, p.good];
    cols[(i + section as usize) % cols.len()]
}

pub fn border_color() -> Color {
    let (from, to, t, beat) = ST.with(|s| {
        let s = s.borrow();
        (s.from, s.to, (s.t0.elapsed().as_secs_f32() / s.dur).min(1.0), s.beat)
    });
    if THEMES[to].pal.is_none() && (t >= 1.0 || from == to) {
        return Color::Reset;
    }
    let (a, b) = (pal_of(from), pal_of(to));
    let mut y = b.border;
    match glitch() {
        1 => y = b.acc2,
        2 => y = b.hot,
        _ => {}
    }
    if THEMES[to].pulse {
        // neon borders swap cyan / magenta on the kick
        y = if beat % 2 == 0 { b.acc } else { b.acc2 };
    }
    out(if from == to { y } else { mix(a.border, y, t) })
}

pub fn border_set() -> border::Set {
    if ascii() {
        return border::Set { top_left: "+", top_right: "+", bottom_left: "+", bottom_right: "+", vertical_left: "|", vertical_right: "|", horizontal_top: "-", horizontal_bottom: "-" };
    }
    match cur().brd {
        Brd::Plain => border::PLAIN,
        Brd::Rounded => border::ROUNDED,
        Brd::Double => border::DOUBLE,
        Brd::Thick => border::THICK,
        Brd::Rope => border::Set { top_left: "@", top_right: "@", bottom_left: "@", bottom_right: "@", vertical_left: "┊", vertical_right: "┊", horizontal_top: "~", horizontal_bottom: "~" },
        Brd::Knot => border::Set { top_left: "✠", top_right: "✠", bottom_left: "✠", bottom_right: "✠", vertical_left: "§", vertical_right: "§", horizontal_top: "∞", horizontal_bottom: "∞" },
        // hazard stripes from light diagonals (width-safe everywhere)
        Brd::Hazard => border::Set { top_left: "┌", top_right: "┐", bottom_left: "└", bottom_right: "┘", vertical_left: "│", vertical_right: "│", horizontal_top: "╱", horizontal_bottom: "╱" },
        Brd::Minimal => border::Set { top_left: " ", top_right: " ", bottom_left: " ", bottom_right: " ", vertical_left: " ", vertical_right: " ", horizontal_top: "─", horizontal_bottom: " " },
    }
}

/// decorate a panel title with the theme's icons
pub fn title(t: &str) -> String {
    let th = cur();
    let (l, r) = if ascii() { th.deco_ascii } else { th.deco };
    if l.is_empty() && r.is_empty() {
        return asciify(t);
    }
    let core = t.trim();
    asciify(&format!(" {l}{core}{r} "))
}

/// meter glyphs (pulsing themes brighten on the beat via colour; glyphs stay put)
pub fn meter_chars() -> (char, char) {
    if ascii() {
        return ('#', '.');
    }
    cur().meter
}

pub fn pulse_on() -> bool {
    cur().pulse && ST.with(|s| s.borrow().beat % 2 == 0)
}

pub fn marks() -> (&'static str, &'static str) {
    if ascii() {
        return ("\"", "\"");
    }
    cur().marks
}

/// best-effort ASCII for the box-drawing / symbol glyphs the UI uses
pub fn asciify(s: &str) -> String {
    if !ascii() {
        return s.to_string();
    }
    s.chars()
        .map(|c| match c {
            c if c.is_ascii() => c,
            '─' | '━' | '═' | '┄' | '╌' | '▔' | '‾' | '~' | '∞' => '-',
            '│' | '┃' | '║' | '┊' | '╿' | '╽' | '╻' | '╹' | '▐' | '▌' => '|',
            '╭' | '┌' | '╔' | '╮' | '┐' | '╗' | '╰' | '└' | '╚' | '╯' | '┘' | '╝' | '┼' | '┬' | '┴' | '┤' | '├' => '+',
            '╱' => '/',
            '╲' => '\\',
            '▒' | '░' | '▓' | '█' | '▄' | '▀' | '■' | '▬' | '▮' | '▲' => '#',
            '▁' | '▂' | '▃' => '_',
            '▅' | '▆' | '▇' => '#',
            '•' | '●' | '◉' | '○' | '◯' | '◌' | '˚' | '◐' | '◑' | '◒' | '◓' | '◎' => 'o',
            '▶' | '→' | '»' => '>',
            '←' | '«' => '<',
            '↑' => '^',
            '↓' => 'v',
            '♪' | '♫' | '♬' | '♩' => '~',
            '✦' | '✧' | '✶' | '★' | '✲' | '✠' => '*',
            '◖' | '⟨' => '(',
            '◗' | '⟩' => ')',
            '“' | '”' => '"',
            '…' | '·' => '.',
            '≈' => '~',
            '§' => 'S',
            '╨' | '┆' | '¦' => '|',
            '↳' | '⇢' | '⟶' => '>',
            '—' | '–' | '−' => '-',
            '♯' => '#',
            '♭' => 'b',
            '⚙' | '☸' | '◈' | '◆' | '◇' => '*',
            '▏' | '▎' | '▍' | '▋' | '▊' | '▉' => '|',
            '▯' | '▱' => '.',
            _ => '.',
        })
        .collect()
}

pub fn icon() -> &'static str {
    let t = cur();
    if ascii() { t.icon.1 } else { t.icon.0 }
}

/// an rgb triple as a terminal colour (truecolor or the nearest 256-colour step)
pub fn rgb(c: (u8, u8, u8), toward_white: f32) -> Color {
    out(mix(c, (255, 255, 255), toward_white))
}

/// one crowd member's own colour (skin, hair, clothes...) kept readable on this theme's
/// background (lifted on dark themes, deepened on light ones), then hazed `level` rows back
pub fn person(c: (u8, u8, u8), level: u8) -> Color {
    let to = ST.with(|s| s.borrow().to);
    let p = pal_of(to);
    let lum = |c: (u8, u8, u8)| (0.299 * c.0 as f32 + 0.587 * c.1 as f32 + 0.114 * c.2 as f32) / 255.0;
    let dark = THEMES[to].pal.is_none() || lum(p.bg) < 0.5;
    let l = lum(c);
    let c = if dark && l < 0.42 {
        mix(c, (255, 255, 255), ((0.42 - l) / (1.0 - l)).min(0.8))
    } else if !dark && l > 0.5 {
        mix(c, (0, 0, 0), ((l - 0.5) / l).min(0.8))
    } else {
        c
    };
    if level == 0 {
        return out(c);
    }
    let k = [0.0, 0.32, 0.55, 0.72][level.min(3) as usize];
    let hz = if THEMES[to].pal.is_none() { (24, 24, 30) } else { mix(p.bg, p.dim, 0.35) };
    out(mix(c, hz, k))
}
