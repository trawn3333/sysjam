//! Scales, pitch helpers.

pub struct ScalePair {
    pub names: &'static [&'static str],
    pub dark: &'static [i32],
    pub bright: &'static [i32],
    pub default_bright: bool,
}

pub const SCALES: &[ScalePair] = &[
    ScalePair { names: &["minor-pentatonic", "minor-pent", "pentatonic"], dark: &[0, 3, 5, 7, 10], bright: &[0, 2, 4, 7, 9], default_bright: false },
    ScalePair { names: &["major-pentatonic", "major-pent"], dark: &[0, 3, 5, 7, 10], bright: &[0, 2, 4, 7, 9], default_bright: true },
    ScalePair { names: &["minor", "aeolian"], dark: &[0, 2, 3, 5, 7, 8, 10], bright: &[0, 2, 4, 5, 7, 9, 11], default_bright: false },
    ScalePair { names: &["major", "ionian"], dark: &[0, 2, 3, 5, 7, 8, 10], bright: &[0, 2, 4, 5, 7, 9, 11], default_bright: true },
    ScalePair { names: &["dorian"], dark: &[0, 2, 3, 5, 7, 9, 10], bright: &[0, 2, 4, 5, 7, 9, 10], default_bright: false },
    ScalePair { names: &["mixolydian"], dark: &[0, 2, 3, 5, 7, 9, 10], bright: &[0, 2, 4, 5, 7, 9, 10], default_bright: true },
    ScalePair { names: &["phrygian"], dark: &[0, 1, 3, 5, 7, 8, 10], bright: &[0, 2, 4, 6, 7, 9, 11], default_bright: false },
    ScalePair { names: &["lydian"], dark: &[0, 1, 3, 5, 7, 8, 10], bright: &[0, 2, 4, 6, 7, 9, 11], default_bright: true },
    ScalePair { names: &["hirajoshi"], dark: &[0, 2, 3, 7, 8], bright: &[0, 2, 4, 7, 9], default_bright: false },
];

pub fn find_scale(name: &str) -> Option<&'static ScalePair> {
    let n = name.to_ascii_lowercase();
    SCALES.iter().find(|s| s.names.iter().any(|x| *x == n))
}

pub fn deg_to_semi(scale: &[i32], deg: i32) -> i32 {
    let n = scale.len() as i32;
    deg.div_euclid(n) * 12 + scale[deg.rem_euclid(n) as usize]
}

/// Nearest scale degree to a semitone offset (ties resolve upward).
pub fn quantize_deg(scale: &[i32], semi: i32) -> i32 {
    let n = scale.len() as i32;
    let oct = semi.div_euclid(12);
    let mut best = oct * n;
    let mut bd = i32::MAX;
    for d in (oct - 1) * n..(oct + 2) * n {
        let dist = (deg_to_semi(scale, d) - semi).abs();
        if dist < bd || (dist == bd && d > best) {
            bd = dist;
            best = d;
        }
    }
    best
}

pub fn semi_to_freq(root: f32, semi: f32) -> f32 {
    root * (semi / 12.0).exp2()
}

pub fn note_name(freq: f32) -> String {
    const N: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let midi = (69.0 + 12.0 * (freq / 440.0).log2()).round() as i32;
    format!("{}{}", N[midi.rem_euclid(12) as usize], midi.div_euclid(12) - 1)
}

/// Parse "A2", "C#3", "Bb1" or a frequency like "110".
pub fn parse_root(s: &str) -> Option<f32> {
    if let Ok(f) = s.parse::<f32>() {
        return if (20.0..2000.0).contains(&f) { Some(f) } else { None };
    }
    let b = s.as_bytes();
    if b.is_empty() {
        return None;
    }
    let base = match b[0].to_ascii_uppercase() {
        b'C' => 0, b'D' => 2, b'E' => 4, b'F' => 5, b'G' => 7, b'A' => 9, b'B' => 11,
        _ => return None,
    };
    let mut i = 1;
    let mut acc = 0;
    while i < b.len() && (b[i] == b'#' || b[i] == b'b') {
        acc += if b[i] == b'#' { 1 } else { -1 };
        i += 1;
    }
    let oct: i32 = s[i..].parse().ok()?;
    let midi = (oct + 1) * 12 + base + acc;
    Some(440.0 * ((midi - 69) as f32 / 12.0).exp2())
}

pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    // final avalanche so similar names spread out
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51afd7ed558ccd);
    h ^= h >> 33;
    h
}

/// Lead voice: base degree + 4-step motif from a name hash.
pub fn lead_pattern(hash: u64, n: i32) -> (i32, [i32; 4]) {
    let base = n + (hash % (2 * n as u64)) as i32; // between root+1oct and root+3oct
    let mut motif = [0i32; 4];
    for (i, m) in motif.iter_mut().enumerate().skip(1) {
        *m = ((hash >> (8 + 5 * i)) % 5) as i32 - 2;
    }
    (base, motif)
}
