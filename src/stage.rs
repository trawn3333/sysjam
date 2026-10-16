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

