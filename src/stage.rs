//! The stage: each player drawn with their instrument as a prop (a grand piano with its lid
//! up, a drum kit, a tuba, a harp, a synth rig with a patch wall ...), picked from the GM
//! program or synth behind them. All art is static text (precomputed, width-safe glyphs
//! only); `@` marks the face. Two frames per prop: frame 1 is the "hit" (keys down,
//! cymbals shaking, the bow across, the harp shimmering, the tuba puffing).

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Prop {
    Grand,
    Upright,
    Keys,
    Organ,
    Kit,
    MetalRig,
    Taiko,
    Timpani,
    UprightBass,
    Cello,
    BassGtr,
    Harp,
    Tuba,
    Guitar,
    Banjo,
    Fiddle,
    Sax,
    Trumpet,
    Flute,
    Harmonica,
    Accordion,
    Bagpipe,
    Sitar,
    Koto,
    Kalimba,
    SteelDrum,
    Marimba,
    Tubular,
    Gong,
    SynthRig,
    Modular,
    Dj,
    Mic,
}

/// which prop for a GM program
pub fn prop_of_gm(p: u8) -> Prop {
    match p {
        0..=2 => Prop::Grand,
        3 | 6 => Prop::Upright,
        4 | 5 | 7 => Prop::Keys,
        8..=10 | 108 => Prop::Kalimba,
        11..=13 => Prop::Marimba,
        14 | 112 => Prop::Tubular,
        15 | 106 | 107 => Prop::Koto,
        16..=20 => Prop::Organ,
        21 | 23 => Prop::Accordion,
        22 => Prop::Harmonica,
        24..=31 => Prop::Guitar,
        32 | 43 => Prop::UprightBass,
        33..=39 => Prop::BassGtr,
        42 => Prop::Cello,
        40 | 41 | 44 | 45 | 48..=51 | 110 => Prop::Fiddle,
        46 => Prop::Harp,
        47 => Prop::Timpani,
        52..=55 => Prop::Mic,
        56 | 57 | 59 | 61..=63 => Prop::Trumpet,
        58 | 60 => Prop::Tuba,
        64..=67 => Prop::Sax,
        68..=79 | 111 => Prop::Flute,
        104 => Prop::Sitar,
        105 => Prop::Banjo,
        109 => Prop::Bagpipe,
        113 | 115 => Prop::Marimba,
        114 => Prop::SteelDrum,
        116..=118 => Prop::Taiko,
        119 => Prop::Gong,
        _ => Prop::SynthRig,
    }
}

/// the prop for stage player `i` (0 drone, 1 bass, 2 drums, 3 keys, 4 lead, 5 colour)
pub fn prop_for(style: &str, i: usize, prog: Option<u8>, synth: bool) -> Option<Prop> {
    Some(match (i, prog) {
        (0, _) => if style == "techno" || style == "house" { Prop::Dj } else { Prop::Modular },
        (2, _) => match style {
            "industrial" => Prop::MetalRig,
            "monk" | "zen" => Prop::Taiko,
            "baroque" => Prop::Timpani,
            _ => Prop::Kit,
        },
        (1, Some(p)) => prop_of_gm(p),
        (1, None) => if matches!(style, "jazz" | "country" | "blues") { Prop::UprightBass } else { Prop::BassGtr },
        (_, Some(p)) => prop_of_gm(p),
        (_, None) if synth => Prop::SynthRig,
        _ => return None,
    })
}

impl Prop {
    /// short instrument word for the caption
    pub fn word(self) -> &'static str {
        use Prop::*;
        match self {
            Grand => "grand",
            Upright => "upright",
            Keys => "keys",
            Organ => "organ",
            Kit => "kit",
            MetalRig => "metal",
            Taiko => "taiko",
            Timpani => "timpani",
            UprightBass => "dbl bass",
            Cello => "cello",
            BassGtr => "bass",
            Harp => "harp",
            Tuba => "tuba",
            Guitar => "guitar",
            Banjo => "banjo",
            Fiddle => "fiddle",
            Sax => "sax",
            Trumpet => "horn",
            Flute => "flute",
            Harmonica => "harp.",
            Accordion => "accord.",
            Bagpipe => "pipes",
            Sitar => "sitar",
            Koto => "koto",
            Kalimba => "kalimba",
            SteelDrum => "steel",
            Marimba => "mallets",
            Tubular => "bells",
            Gong => "gong",
            SynthRig => "synth",
            Modular => "modular",
            Dj => "decks",
            Mic => "vocals",
        }
    }
    /// big instruments keep their big art longest when the stage is tight
    pub fn big(self) -> bool {
        use Prop::*;
        matches!(self, Grand | Organ | Kit | MetalRig | Timpani | Harp | Tuba | Marimba | Tubular | SynthRig | Modular | Dj | UprightBass | Koto | Gong)
    }
    /// bowed / continuously moving: frame 1 alternates on the beat while playing
    pub fn moving(self) -> bool {
        use Prop::*;
        matches!(self, Fiddle | Cello | UprightBass | Harp | Accordion | Dj | Modular | Bagpipe)
    }
}

/// all art rows for (prop, frame), padded to one width
pub fn art(p: Prop, frame: usize) -> &'static [String] {
    use std::sync::OnceLock;
    static ART: OnceLock<Vec<[Vec<String>; 2]>> = OnceLock::new();
    let all = ART.get_or_init(|| ALL.iter().map(|&q| build(q)).collect());
    let k = ALL.iter().position(|&q| q == p).unwrap_or(0);
    &all[k][frame.min(1)]
}

pub fn width(p: Prop) -> usize {
    art(p, 0).first().map(|r| r.chars().count()).unwrap_or(0)
}

pub const ROWS: usize = 6;

pub const ALL: [Prop; 33] = [
    Prop::Grand, Prop::Upright, Prop::Keys, Prop::Organ, Prop::Kit, Prop::MetalRig, Prop::Taiko, Prop::Timpani,
    Prop::UprightBass, Prop::Cello, Prop::BassGtr, Prop::Harp, Prop::Tuba, Prop::Guitar, Prop::Banjo, Prop::Fiddle,
    Prop::Sax, Prop::Trumpet, Prop::Flute, Prop::Harmonica, Prop::Accordion, Prop::Bagpipe, Prop::Sitar, Prop::Koto,
    Prop::Kalimba, Prop::SteelDrum, Prop::Marimba, Prop::Tubular, Prop::Gong, Prop::SynthRig, Prop::Modular, Prop::Dj, Prop::Mic,
];

fn build(p: Prop) -> [Vec<String>; 2] {
    let (a, b) = raw(p);
    let pad = |v: &[&str]| -> Vec<String> {
        let mut rows: Vec<String> = v.iter().map(|s| s.to_string()).collect();
        while rows.len() < ROWS {
            rows.insert(0, String::new());
        }
        rows
    };
    let mut fa = pad(a);
    let mut fb = pad(if b.is_empty() { a } else { b });
    let w = fa.iter().chain(fb.iter()).map(|r| r.chars().count()).max().unwrap_or(0) + 1;
    for r in fa.iter_mut().chain(fb.iter_mut()) {
        let n = r.chars().count();
        r.push_str(&" ".repeat(w - n));
    }
    [fa, fb]
}

/// (frame 0, frame 1) — frame 1 empty = same as frame 0
fn raw(p: Prop) -> (&'static [&'static str], &'static [&'static str]) {
    use Prop::*;
    match p {
        Grand => (
            &["        ╱╲            ", " (@)   ╱  ╲________    ", " /|\\─┬┴──────────╲─╮  ", "  │  │┬┬┬┬┬┬┬┬┬┬┬┬┬│  ", " ╭┴╮ └┬────────────┬╯  ", " ─┴─  │            │   "],
            &["        ╱╲            ", " (@)   ╱  ╲________ ♪  ", " \\|/─┬┴──────────╲─╮  ", "  │  │┬┴┬┬┴┬┬┬┴┬┬┴┬┬│  ", " ╭┴╮ └┬────────────┬╯  ", " ─┴─  │            │   "],
        ),
        Upright => (
            &[" ┌──────────┐", " │ ♪     ♫  │", "(@)─────────┤", "/|┬┬┬┬┬┬┬┬┬┬│", "╱│└─────────┤", "─┴─        ┘"],
            &[" ┌──────────┐", " │  ♫    ♪  │", "(@)─────────┤", "\\|┬┴┬┬┴┬┬┴┬┬│", "╱│└─────────┤", "─┴─        ┘"],
        ),
        Keys => (
            &["   (@)      ", "   /|\\      ", " ┌┬┬┬┬┬┬┬┬┬┐", " └┬───────┬┘", "  ╱╲     ╱╲ "],
            &["   (@)  ♪   ", "   \\|/      ", " ┌┬┴┬┬┴┬┬┴┬┐", " └┬───────┬┘", "  ╱╲     ╱╲ "],
        ),
        Organ => (
            &[" ││ │││ │││ ││      ", " ┴┴─┴┴┴─┴┴┴─┴┴      ", " │• • • • • •│  (@) ", " │┬┬┬┬┬┬┬┬┬┬┬│ ─/|  ", " │┬┬┬┬┬┬┬┬┬┬┬│   │  ", " └┴─────────┴┘  ╱ ╲ "],
            &[" ││ │││ │││ ││  ♪   ", " ┴┴─┴┴┴─┴┴┴─┴┴      ", " │• · • • · •│  (@) ", " │┬┴┬┬┬┴┬┬┬┴┬│ ─/|  ", " │┬┬┴┬┬┬┬┴┬┬┬│   │  ", " └┴─────────┴┘  ╱ ╲ "],
        ),
        Kit => (
            &[" ──     (@)     ── ", " ╲╱     /|\\     ╲╱ ", "  │ ╭─╮ ─┴─ ╭─╮  │  ", "  │ ╰┬╯╭───╮╰┬╯  │  ", "  │  │ │ • │ │   │  ", " ╱ ╲   ╰───╯    ╱ ╲ "],
            &[" ~~     (@)     ~~ ", " ╲╱     \\|/     ╲╱ ", "  │ ╭─╮ ─┴─ ╭─╮  │  ", "  │ ╰┬╯╭───╮╰┬╯  │  ", "  │  │ │ o │ │   │  ", " ╱ ╲   ╰───╯    ╱ ╲ "],
        ),
        MetalRig => (
            &[" ┬─┬─┬  ──(@)──  ┌──┐ ", " │ │ │  ╲╱/|\\╲╱  │##│ ", " ' │ '  ╭─┴─┴─╮  │##│ ", "   '    │ ╭─╮ │  └┬─┘ ", "  ==    │ ╰─╯ │   │   ", " ─┴─    ╰─────╯  ─┴─  "],
            &[" ┬─┬─┬  ~~(@)~~  ┌──┐ ", " ╱ │ ╲  ╲╱\\|/╲╱  │//│ ", "'  │  ' ╭─┴─┴─╮  │//│ ", "   '    │ ╭─╮ │  └┬─┘ ", "  ==    │ ╰─╯ │   │   ", " ─┴─    ╰─────╯  ─┴─  "],
        ),
        Taiko => (
            &["   (@)    ", "  \\/|\\/   ", " ╭─────╮  ", " │  ·  │  ", " ╰─────╯  ", " ╱╲   ╱╲  "],
            &["   (@)    ", "  ─/|\\─   ", " ╭─────╮  ", " │ ·♪· │  ", " ╰─────╯  ", " ╱╲   ╱╲  "],
        ),
        Timpani => (
            &["    (@)      ", "   ─/|\\─     ", " ╭───╮ ╭───╮ ", " ╰─┬─╯ ╰─┬─╯ ", "  ╱│╲   ╱│╲  "],
            &["    (@)  ♪   ", "   \\/|\\/     ", " ╭───╮ ╭───╮ ", " ╰─┬─╯ ╰─┬─╯ ", "  ╱│╲   ╱│╲  "],
        ),
        UprightBass => (
            &["  ╭╮      ", "  ││  (@) ", " ╭┴┴╮ /|  ", " │::│──┘  ", " │::│  │  ", " ╰┬┬╯ ╱ ╲ "],
            &["  ╭╮      ", "  ││  (@) ", " ╭┴┴╮ /|  ", " │::│─┘   ", " │::│  │  ", " ╰┬┬╯ ╱ ╲ "],
        ),
        Cello => (
            &["  ╭╮      ", "  ││  (@) ", " ╭┴┴╮ /|  ", "─┼::┼─┘   ", " ╰┬┬╯ ╱ ╲ "],
            &["  ╭╮      ", "  ││  (@) ", " ╭┴┴╮ /|  ", " │::┼──┘  ", " ╰┬┬╯ ╱ ╲ "],
        ),
        BassGtr => (
            &["   (@)     ", "   /|\\     ", " ──────[o] ", "    │      ", "   ╱ ╲     "],
            &["   (@)     ", "   /|─     ", " ──────[o] ", "    │      ", "   ╱ ╲     "],
        ),
        Harp => (
            &[" ╭────╮     ", " │╲│││ (@)  ", " │ ╲││ /|─  ", " │  ╲│ ╱│   ", " │   ╲ ╱ ╲  ", " └────┘     "],
            &[" ╭────╮  ♪  ", " │╲:│: (@)  ", " │ ╲:│ /|─  ", " │  ╲: ╱│   ", " │   ╲ ╱ ╲  ", " └────┘     "],
        ),
        Tuba => (
            &["  ╭────╮  ", "  │ ·· │  ", "  ╰╮ ┌─╯  ", " (@)│ │   ", " /|╰─╯    ", " ╱ ╲      "],
            &[" ♪╭────╮  ", "  │ ·· │  ", "  ╰╮ ┌─╯  ", " (@)│ │   ", " /|╰─╯    ", " ╱ ╲      "],
        ),
        Guitar => (
            &["   (@)    ", "   /|\\    ", " ─────(o) ", "    │     ", "   ╱ ╲    "],
            &["   (@)    ", "   /|─    ", " ─────(o)♪", "    │     ", "   ╱ ╲    "],
        ),
        Banjo => (
            &["   (@)    ", "   /|\\    ", " ─────(#) ", "    │     ", "   ╱ ╲    "],
            &["   (@)    ", "   /|─    ", " ─────(#)♪", "    │     ", "   ╱ ╲    "],
        ),
        Fiddle => (
            &["   (@)<=  ", "  ─/|──── ", "    │     ", "   ╱ ╲    "],
            &["   (@)<=  ", "────/|─   ", "    │     ", "   ╱ ╲    "],
        ),
        Sax => (
            &["   (@)   ", "   /|╮   ", "    │││  ", "    │╰┘  ", "   ╱ ╲   "],
            &["   (@) ♪ ", "   /|╮   ", "    │││  ", "    │╰┘  ", "   ╱ ╲   "],
        ),
        Trumpet => (
            &["   (@)──<  ", "   /|╯     ", "    │      ", "   ╱ ╲     "],
            &["   (@)──< ♪", "   /|╯     ", "    │      ", "   ╱ ╲     "],
        ),
        Flute => (
            &["   (@)──── ", "   /|\\     ", "    │      ", "   ╱ ╲     "],
            &["   (@)────♪", "   /|\\     ", "    │      ", "   ╱ ╲     "],
        ),
        Harmonica => (
            &["   (@)    ", "  ╭[=]╮   ", "    │     ", "   ╱ ╲    "],
            &["   (@) ♪  ", "  ╭[=]╮   ", "    │     ", "   ╱ ╲    "],
        ),
        Accordion => (
            &["   (@)    ", "  ┌┼┼┼┐   ", "  │┼┼┼│   ", "  └┼┼┼┘   ", "   ╱ ╲    "],
            &["   (@)    ", " ┌┼─┼─┼┐  ", " │┼─┼─┼│  ", " └┼─┼─┼┘  ", "   ╱ ╲    "],
        ),
        Bagpipe => (
            &["  │││    ", "  (@)─   ", "  ╭──╮/  ", "  ╰──╯   ", "  ╱ ╲    "],
            &[" ♪│││    ", "  (@)─   ", " ╭───╮/  ", " ╰───╯   ", "  ╱ ╲    "],
        ),
        Sitar => (
            &["     ┬   ", "  (@)│   ", "  /|╲│   ", "  ╭──┴╮  ", "  ╰───╯  ", "  ─┴─    "],
            &["     ┬ ♪ ", "  (@)│   ", "  /|─│   ", "  ╭──┴╮  ", "  ╰───╯  ", "  ─┴─    "],
        ),
        Koto => (
            &["    (@)       ", "    /|\\       ", " ┌──────────┐ ", " │╱╲╱╲╱╲╱╲╱╲│ ", " └┴────────┴┘ "],
            &["    (@)  ♪    ", "    \\|/       ", " ┌──────────┐ ", " │╲╱╲╱╲╱╲╱╲╱│ ", " └┴────────┴┘ "],
        ),
        Kalimba => (
            &["   (@)   ", "  ╭┬┬┬╮  ", "  ╰───╯  ", "   ╱ ╲   "],
            &["   (@) ♪ ", "  ╭┴┬┴╮  ", "  ╰───╯  ", "   ╱ ╲   "],
        ),
        SteelDrum => (
            &["   (@)    ", "  \\/|\\/   ", " ╭─────╮  ", " ╰┬───┬╯  ", "  │   │   "],
            &["   (@) ♪  ", "  ─/|\\─   ", " ╭─────╮  ", " ╰┬───┬╯  ", "  │   │   "],
        ),
        Marimba => (
            &["     (@)       ", "    ─/|\\─      ", " ┬┬┬┬┬┬┬┬┬┬┬┬  ", " ┴┴┴┴┴┴┴┴┴┴┴┴  ", "  │        │   ", "  ┴        ┴   "],
            &["     (@)   ♪   ", "    \\/|\\/      ", " ┬┬┴┬┬┬┬┴┬┬┬┬  ", " ┴┴┴┴┴┴┴┴┴┴┴┴  ", "  │        │   ", "  ┴        ┴   "],
        ),
        Tubular => (
            &[" ┌───────┐    ", " │ │ │ │ │    ", " │ │ │ │ │(@) ", " │ │ │ │ │/|─ ", "   │   │  ╱ ╲ ", " ──┴───┴──    "],
            &[" ┌───────┐ ♪  ", " │ │ │ │ │    ", " │ │ │ │ │(@) ", " │ │ │ │ │\\|─ ", "   │   │  ╱ ╲ ", " ──┴───┴──    "],
        ),
        Gong => (
            &[" ┌─────┐      ", " │ ╭─╮ │ (@)  ", " │ │·│ │─/|   ", " │ ╰─╯ │ ╱ ╲  ", " ┴     ┴      "],
            &[" ┌─────┐  ♪   ", " │╭───╮│ (@)  ", " ││ · ││─\\|   ", " │╰───╯│ ╱ ╲  ", " ┴     ┴      "],
        ),
        SynthRig => (
            &[" ┌───────────┐     ", " │•╮•╭•╮•╭• │ (@) ", " │•╯•╰•╯•╰• │ /|\\ ", " └───────────┘  │  ", " ┌┬┬┬┬┬┬┬┬┬┬┐ ╱ ╲ "],
            &[" ┌───────────┐     ", " │·╮•╭·╮•╭· │ (@) ", " │•╯·╰•╯·╰• │ \\|/ ", " └───────────┘  │  ", " ┌┬┴┬┬┴┬┬┴┬┬┐ ╱ ╲ "],
        ),
        Modular => (
            &[" ┌─────┐ (@) ", " │~~~~~│ /|\\ ", " │•╮•╭•│  │  ", " └┴───┴┘ ╱ ╲ "],
            &[" ┌─────┐ (@) ", " │ ~~~~│ /|─ ", " │·╮•╭·│  │  ", " └┴───┴┘ ╱ ╲ "],
        ),
        Dj => (
            &["      (@)       ", "     ─/|\\─      ", " ┌────────────┐ ", " │ (─)  ┼┼ (─) │", " └┬──────────┬┘ ", "  │          │  "],
            &["      (@)       ", "     \\/|\\/      ", " ┌────────────┐ ", " │ (│)  ┼┼ (│) │", " └┬──────────┬┘ ", "  │          │  "],
        ),
        Mic => (
            &["   (@)    ", "   /|─o   ", "    │ │   ", "   ╱ ╲┴   "],
            &["   (@)  ♪ ", "   \\|─o   ", "    │ │   ", "   ╱ ╲┴   "],
        ),
    }
}

/// a wedge monitor and an amp stack
pub const WEDGE: &str = "╱──╲";
pub fn amp(industrial: bool) -> [&'static str; 6] {
    if industrial {
        ["┌──┐", "│##│", "│##│", "├──┤", "│##│", "└──┘"]
    } else {
        ["┌──┐", "│::│", "│::│", "├──┤", "│::│", "└──┘"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::figure::safe_glyph;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn live_props_are_clean_and_fixed_width() {
        let mut cases = Vec::new();
        for age in [0.0f32, 0.05, 0.2, 9.0] {
            for n in 0..3u64 {
                for key in [36, 38, 42, 44, 46, 49, 51, 45, 41, 60, 84] {
                    let mut l = Live { age, n, key, frac: (key as f32 - 30.0) / 60.0, chord: n == 1, lo: 0.1, hi: 0.9, chug: n == 2, ..Default::default() };
                    l.drums.add(key, age);
                    l.drums.add(38, age);
                    cases.push(l);
                }
            }
        }
        for p in ALL {
            let w = width(p);
            for l in &cases {
                for metal in [false, true] {
                    let (rows, _) = live(p, l, metal);
                    assert_eq!(rows.len(), ROWS, "{p:?}");
                    for r in &rows {
                        assert_eq!(r.width(), w, "{p:?} {l:?}: {r:?}");
                        assert_eq!(r.chars().count(), w, "{p:?}: {r:?}");
                        assert!(r.chars().all(safe_glyph), "{p:?}: unsafe glyph in {r:?}");
                    }
                    assert_eq!(rows.iter().map(|r| r.matches('@').count()).sum::<usize>(), 1, "{p:?} {l:?} lost its face");
                }
            }
        }
    }

    #[test]
    fn props_are_clean_and_fixed_width() {
        for p in ALL {
            let w = width(p);
            assert!(w > 0 && w <= 24, "{p:?} width {w}");
            for f in 0..2 {
                let a = art(p, f);
                assert_eq!(a.len(), ROWS, "{p:?}");
                for r in a {
                    assert_eq!(r.width(), w, "{p:?} frame {f}: {r:?}");
                    assert_eq!(r.chars().count(), w, "{p:?} frame {f}: {r:?}");
                    assert!(r.chars().all(safe_glyph), "{p:?}: unsafe glyph in {r:?}");
                }
                assert_eq!(a.iter().map(|r| r.matches('@').count()).sum::<usize>(), 1, "{p:?} needs exactly one face");
            }
        }
        for r in amp(true).iter().chain(amp(false).iter()) {
            assert_eq!(r.width(), 4);
        }
        for g in 0..128u8 {
            let _ = prop_of_gm(g);
        }
        assert_eq!(prop_of_gm(0), Prop::Grand);
        assert_eq!(prop_of_gm(58), Prop::Tuba);
        assert_eq!(prop_of_gm(30), Prop::Guitar);
    }
}

// ---------------------------------------------------------------- note-driven props

/// what a player's notes are doing right now (from the heard note events)
#[derive(Clone, Copy, Debug, Default)]
pub struct Live {
    /// seconds since the last note heard (large when none)
    pub age: f32,
    /// notes so far (parity alternates bows, strums and sticks)
    pub n: u64,
    /// the last note's pitch (MIDI) and where it sits in the player's range, 0 low .. 1 high
    pub key: i32,
    pub frac: f32,
    /// several notes at once (a chord), spanning low and high
    pub chord: bool,
    pub lo: f32,
    pub hi: f32,
    /// the last few notes were the same low pitch (palm-muted chugs)
    pub chug: bool,
    /// drums: seconds since each voice was heard
    pub drums: Drums,
}

#[derive(Clone, Copy, Debug)]
pub struct Drums {
    pub hat: f32,
    pub open: f32,
    pub pedal: f32,
    pub snare: f32,
    pub kick: f32,
    pub tom: f32,
    pub floor: f32,
    pub crash: f32,
    pub ride: f32,
    pub perc: f32,
}
impl Default for Drums {
    fn default() -> Self {
        Drums { hat: 9.0, open: 9.0, pedal: 9.0, snare: 9.0, kick: 9.0, tom: 9.0, floor: 9.0, crash: 9.0, ride: 9.0, perc: 9.0 }
    }
}
impl Drums {
    /// account a heard drum hit (GM key) `age` seconds ago
    pub fn add(&mut self, key: i32, age: f32) {
        let slot = match key {
            42 => &mut self.hat,
            46 => &mut self.open,
            44 => &mut self.pedal,
            37..=40 => &mut self.snare,
            35 | 36 => &mut self.kick,
            41 | 43 => &mut self.floor,
            45 | 47 | 48 | 50 => &mut self.tom,
            49 | 52 | 55 | 57 => &mut self.crash,
            51 | 53 | 59 => &mut self.ride,
            _ => &mut self.perc,
        };
        *slot = slot.min(age);
    }
}

/// how long a hit shows
pub const FLASH: f32 = 0.11;

fn set(rows: &mut [Vec<char>], r: usize, c: usize, ch: char) {
    if let Some(row) = rows.get_mut(r) {
        if let Some(x) = row.get_mut(c) {
            *x = ch;
        }
    }
}
fn put(rows: &mut [Vec<char>], r: usize, c: usize, s: &str) {
    for (k, ch) in s.chars().enumerate() {
        set(rows, r, c + k, ch);
    }
}

/// a prop drawn from the notes as they're heard: the drummer's limbs hit exactly the drums
/// that sound, strings strum or bow per note with the fret hand following the pitch, keys go
/// down where the note is (left hand low, right hand high, both for chords), horn players'
/// mouths open while they play and close for a breath, mallets strike; silence is relaxed.
/// Returns the rows and whether it is a "hit" frame (for the face).
pub fn live(p: Prop, l: &Live, metal: bool) -> (Vec<String>, bool) {
    use Prop::*;
    let on = l.age < FLASH;
    // the bowed and squeezed ones alternate per note; everybody else shows frame 1 on a hit
    let frame = match p {
        Fiddle | Cello | UprightBass | Bagpipe => (l.n % 2) as usize,
        Accordion => ((l.n / 4) % 2) as usize,
        Kit => 0,
        _ => on as usize,
    };
    let base = art(p, frame);
    let mut rows: Vec<Vec<char>> = base.iter().map(|r| r.chars().collect()).collect();
    let o = ROWS - raw(p).0.len(); // rows padded on top
    match p {
        Kit => {
            let d = &l.drums;
            let f = |a: f32| a < FLASH;
            // right stick on the hat (closed / open), the left foot on the hat pedal
            if f(d.hat) || f(d.open) || f(d.perc) {
                put(&mut rows, o, 1, if f(d.open) { "~~" } else if f(d.hat) { "==" } else { "──" });
                put(&mut rows, o + 1, 4, "────");
                set(&mut rows, o + 1, 8, '─');
                if f(d.perc) && !f(d.hat) && !f(d.open) {
                    set(&mut rows, o, 3, '•');
                }
            }
            if f(d.pedal) {
                put(&mut rows, o + 5, 1, "╱_╲");
                put(&mut rows, o, 1, "==");
            }
            // left stick on the snare (the drum on the right as we see it)
            if f(d.snare) {
                set(&mut rows, o + 1, 10, '─');
                set(&mut rows, o + 1, 11, '╲');
                set(&mut rows, o + 2, 13, '*');
            }
            // toms: the sticks sweep across, high toms on the left, the floor tom on the right
            if f(d.tom) {
                set(&mut rows, o + 1, 8, '─');
                set(&mut rows, o + 1, 7, '╱');
                set(&mut rows, o + 2, 5, '*');
            }
            if f(d.floor) {
                set(&mut rows, o + 1, 10, '╲');
                set(&mut rows, o + 1, 11, '╲');
                set(&mut rows, o + 3, 13, '*');
            }
            // crash / ride: the arm goes up to the cymbal
            if f(d.crash) || f(d.ride) {
                set(&mut rows, o, 11, '╱');
                set(&mut rows, o + 1, 10, '│');
                put(&mut rows, o, 16, if f(d.crash) { "**" } else { "~•" });
            }
            // the right foot on the kick pedal: the beater hits the head
            if f(d.kick) {
                set(&mut rows, o + 4, 9, 'O');
                set(&mut rows, o + 5, 9, '┴');
            }
        }
        MetalRig => {
            let d = &l.drums;
            // double kick: the beaters alternate per hit
            if d.kick < FLASH {
                set(&mut rows, o + 4, if l.n % 2 == 0 { 10 } else { 12 }, '*');
            }
        }
        Guitar | Banjo | BassGtr => {
            // the strumming hand per note (down, up), the fret hand along the neck by pitch
            let neck = 1 + (l.frac.clamp(0.0, 1.0) * 4.0).round() as usize;
            for c in 1..6 {
                if rows[o + 2][c] == '┼' {
                    rows[o + 2][c] = '─';
                }
            }
            if l.age < 2.0 {
                set(&mut rows, o + 2, neck, '┼');
            }
            if on {
                set(&mut rows, o + 1, 5, if l.n % 2 == 0 { '─' } else { '╲' });
                if metal && l.chug {
                    // palm-muted chugs: the hand stays on the bridge
                    set(&mut rows, o + 1, 5, '_');
                    set(&mut rows, o + 2, 7, '=');
                }
            } else {
                set(&mut rows, o + 1, 5, '\\');
            }
        }
        Keys | Grand | Upright | Organ | SynthRig => {
            // (arms row, left-hand col, right-hand col, keyboard row, first key col, last key col)
            let (ar, lc, rc, kr, k0, k1): (usize, Option<usize>, Option<usize>, usize, usize, usize) = match p {
                Keys => (1, Some(3), Some(5), 2, 2, 10),
                Grand => (2, Some(1), Some(3), 3, 6, 18),
                Upright => (3, Some(0), None, 3, 2, 11),
                Organ => (3, Some(15), Some(16), 3, 2, 12),
                _ => (2, Some(15), Some(17), 4, 2, 12),
            };
            let (ar, kr) = (ar + o, kr + o);
            // start from the resting frame, then press what sounds
            rows = art(p, 0).iter().map(|r| r.chars().collect()).collect();
            if on {
                let lo = l.chord || l.frac < 0.5;
                let hi = l.chord || l.frac >= 0.5 || rc.is_none();
                if let (true, Some(c)) = (lo || rc.is_none(), lc) {
                    set(&mut rows, ar, c, '\\');
                }
                if let (true, Some(c)) = (hi, rc) {
                    set(&mut rows, ar, c, '/');
                }
                let span = (k1 - k0) as f32;
                let mut press = |f: f32| {
                    let c = k0 + (f.clamp(0.0, 1.0) * span).round() as usize;
                    if rows.get(kr).and_then(|r| r.get(c)).is_some_and(|&ch| ch == '┬') {
                        set(&mut rows, kr, c, '┴');
                    }
                };
                if l.chord {
                    press(l.lo);
                    press(l.hi);
                } else {
                    press(l.frac);
                }
            }
        }
        Sax | Trumpet | Tuba | Bagpipe => {
            // cheeks puff while the note sounds
            if l.age < 0.35 {
                for r in rows.iter_mut() {
                    if let Some(i) = r.iter().position(|&c| c == '@') {
                        if i > 0 && i + 1 < r.len() && r[i - 1] == '(' && r[i + 1] == ')' {
                            r[i - 1] = '{';
                            r[i + 1] = '}';
                        }
                    }
                }
            }
        }
        _ => {}
    }
    let hit = on || (matches!(p, Kit | MetalRig) && l.drums.kick.min(l.drums.snare).min(l.drums.hat).min(l.drums.crash) < FLASH);
    (rows.into_iter().map(|r| r.into_iter().collect()).collect(), hit)
}

/// the face for a horn or a singer, from the note timing (the '@' cell)
pub fn blow_face(l: &Live) -> char {
    if l.age < 0.35 {
        'O'
    } else if l.age < 1.5 {
        'o'
    } else {
        '-'
    }
}
