//! The conductor's tiny brain: a 32 → 32 → 16 MLP with three heads (16 mood logits,
//! 6 interventions, 8 speech intents), ~2.1k weights, 8 KB, embedded in the binary.
//! Trained offline by tools/train_conductor.py on synthetic episodes labelled by the
//! heuristic conductor + emotional rules (src/mood.rs). Runs once per bar, a few µs.
//! It never learns at run time; your likes steer through likes.rs instead.

use crate::mood::{NF, N as NM};

static BIN: &[u8] = include_bytes!("../assets/conductor.bin");
const H1: usize = 32;
const H2: usize = 16;
pub const NA: usize = 6;
pub const NI: usize = 8;
const NO: usize = NM + NA + NI;

pub struct Brain {
    w1: Vec<f32>,
    b1: Vec<f32>,
    w2: Vec<f32>,
    b2: Vec<f32>,
    w3: Vec<f32>,
    b3: Vec<f32>,
}

pub struct Out {
    pub mood: [f32; NM],
    pub action: [f32; NA],
    pub intent: [f32; NI],
}

impl Brain {
    pub fn load() -> Option<Brain> {
        if BIN.len() < 28 || &BIN[..4] != b"SJB1" {
            return None;
        }
        let dims: Vec<usize> = (0..6).map(|i| u32::from_le_bytes(BIN[4 + 4 * i..8 + 4 * i].try_into().unwrap()) as usize).collect();
        if dims != [NF, H1, H2, NM, NA, NI] {
            return None;
        }
        let mut off = 28;
        let mut take = |n: usize| -> Option<Vec<f32>> {
            let b = BIN.get(off..off + 4 * n)?;
            off += 4 * n;
            Some(b.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect())
        };
        Some(Brain { w1: take(NF * H1)?, b1: take(H1)?, w2: take(H1 * H2)?, b2: take(H2)?, w3: take(H2 * NO)?, b3: take(NO)? })
    }

    pub fn size_bytes() -> usize {
        BIN.len()
    }

    pub fn forward(&self, x: &[f32; NF]) -> Out {
        let mut h1 = [0f32; H1];
        for j in 0..H1 {
            let mut s = self.b1[j];
            for i in 0..NF {
                s += x[i] * self.w1[i * H1 + j];
            }
            h1[j] = s.tanh();
        }
        let mut h2 = [0f32; H2];
        for j in 0..H2 {
            let mut s = self.b2[j];
            for i in 0..H1 {
                s += h1[i] * self.w2[i * H2 + j];
            }
            h2[j] = s.tanh();
        }
        let mut o = [0f32; NO];
        for j in 0..NO {
            let mut s = self.b3[j];
            for i in 0..H2 {
                s += h2[i] * self.w3[i * NO + j];
            }
            o[j] = s;
        }
        let mut out = Out { mood: [0.0; NM], action: [0.0; NA], intent: [0.0; NI] };
        out.mood.copy_from_slice(&o[..NM]);
        out.action.copy_from_slice(&o[NM..NM + NA]);
        out.intent.copy_from_slice(&o[NM + NA..]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mood::*;

    #[test]
    fn brain_agrees_with_its_teacher() {
        let b = Brain::load().expect("weights");
        let mut rng = crate::dsp::Rng::seeded(3);
        let (mut agree_m, mut agree_a, n) = (0, 0, 2000);
        let t0 = std::time::Instant::now();
        for _ in 0..n {
            let mut x = [0f32; NF];
            for v in x.iter_mut() {
                *v = rng.f();
            }
            for k in 7..11 {
                x[k] = 0.0;
            }
            x[7 + rng.below(4)] = 1.0;
            for k in [f::CHAOS, f::BREAK, f::QUOTE, f::MINOR, f::SURGE, f::BUILD, f::RELAX, f::CADENCE] {
                x[k] = (x[k] > 0.8) as i32 as f32;
            }
            x[f::SPIKE] = if rng.f() < 0.1 { rng.f() } else { 0.0 };
            x[f::LIKE] = if rng.f() < 0.05 { rng.f() } else { 0.0 };
            x[f::STRICT] = rng.f() * 0.9 - 0.3;
            x[f::ONE] = 1.0;
            let o = b.forward(&x);
            let am = |v: &[f32]| v.iter().enumerate().fold((0, f32::MIN), |b, (i, &x)| if x > b.1 { (i, x) } else { b }).0;
            let rs = rule_scores(&x);
            agree_m += (am(&o.mood) == am(&rs)) as i32;
            agree_a += (am(&o.action) == rule_action(&x)) as i32;
        }
        let per = t0.elapsed().as_nanos() as f64 / n as f64;
        eprintln!("brain: {} bytes, mood agreement {:.2}, action {:.2}, {per:.0} ns per call (incl. rules)", Brain::size_bytes(), agree_m as f32 / n as f32, agree_a as f32 / n as f32);
        assert!(agree_m as f32 / n as f32 > 0.6);
        assert!(agree_a as f32 / n as f32 > 0.6);
    }
}
