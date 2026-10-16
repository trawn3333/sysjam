//! Dead-time / stagnation / section-change bookkeeping. Cheap enough to run always
//! (a few adds per 16th step); the bench prints the report.

#[derive(Default)]
pub struct DeadStats {
    pub steps: u64,
    /// fewer than 2 harmonic/melodic voices sounding (pad, bass, keys, lead, colour, guest)
    pub thin: u64,
    /// no melodic voice (keys/lead/colour/guest) sounding at all
    pub no_mel: u64,
    /// nothing new (no harmonic/melodic onset, no chord/pad move) for 4 s
    pub static_: u64,
    /// thin or static
    pub dead: u64,
    /// fewer than 2 players active (bass/drums/keys/lead/colour), outside deliberate breakdowns
    pub few: u64,
    pub act_sum: u64,
    /// why melody was silent: per player 3..5 [off, conductor lay-out, rest, hold, play-but-silent]
    pub why: [[u64; 5]; 3],
    /// silent-melody steps by arrangement state
    pub why_state: [u64; 4],
    pub last_onset: u64,
    pub rms_acc: f64,
    pub rms_n: u64,
    pub rms_win: u64,
    pub rms_gap: u64,
    pub bars: u64,
    pub stale_bars: u64,
    pub marks: Vec<(f64, &'static str)>,
    pub nexts: u64,
    pub develops: Vec<&'static str>,
}

impl DeadStats {
    pub fn mark(&mut self, t: f64, why: &'static str) {
        if let Some(&(lt, _)) = self.marks.last() {
            if t - lt < 4.0 {
                return;
            }
        }
        self.marks.push((t, why));
    }
    pub fn report(&self, total_s: f64) -> Vec<String> {
        let pc = |x: u64| if self.steps == 0 { 0.0 } else { x as f64 * 100.0 / self.steps as f64 };
        let mut gaps: Vec<f64> = self.marks.windows(2).map(|w| w[1].0 - w[0].0).collect();
        if let Some(&(f, _)) = self.marks.first() {
            gaps.push(f);
        }
        gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med = if gaps.is_empty() { total_s } else { gaps[gaps.len() / 2] };
        let maxg = gaps.last().copied().unwrap_or(total_s);
        // from a random moment, how long until the next section change (time-weighted)
        let mut all: Vec<f64> = gaps.clone();
        if let Some(&(l, _)) = self.marks.last() {
            all.push(total_s - l);
        }
        let s1: f64 = all.iter().sum();
        let wait = if s1 > 0.0 { all.iter().map(|g| g * g).sum::<f64>() / s1 / 2.0 } else { total_s };
        let mut kinds: Vec<(&str, usize)> = Vec::new();
        for (_, w) in &self.marks {
            match kinds.iter_mut().find(|k| k.0 == *w) {
                Some(k) => k.1 += 1,
                None => kinds.push((w, 1)),
            }
        }
        let kinds: Vec<String> = kinds.iter().map(|(w, n)| format!("{w} {n}")).collect();
        let mut out = vec![
            format!(
                "deadtime: dead {:.1}% (thin <2 voices {:.1}%, static 4s {:.1}%), no melody {:.1}%, <2 players {:.1}%, avg players {:.2}, rms gaps {:.1}% of 0.5s windows",
                pc(self.dead),
                pc(self.thin),
                pc(self.static_),
                pc(self.no_mel),
                pc(self.few),
                if self.steps == 0 { 0.0 } else { self.act_sum as f64 / self.steps as f64 },
                if self.rms_win == 0 { 0.0 } else { self.rms_gap as f64 * 100.0 / self.rms_win as f64 }
            ),
            format!(
                "sections: {} changes, median gap {:.0} s, longest {:.0} s, mean wait {:.0} s | stagnant {:.1}% of bars | next {} | develops {} | {}",
                self.marks.len(),
                med,
                maxg,
                wait,
                if self.bars == 0 { 0.0 } else { self.stale_bars as f64 * 100.0 / self.bars as f64 },
                self.nexts,
                self.develops.len(),
                kinds.join(", ")
            ),
        ];
        let nm = ["keys", "lead", "colour"];
        let w: Vec<String> = (0..3)
            .map(|i| {
                let r = self.why[i];
                let t = r.iter().sum::<u64>().max(1) as f64;
                format!("{} off {:.0}% lay {:.0}% rest {:.0}% hold {:.0}% gated {:.0}%", nm[i], r[0] as f64 * 100.0 / t, r[1] as f64 * 100.0 / t, r[2] as f64 * 100.0 / t, r[3] as f64 * 100.0 / t, r[4] as f64 * 100.0 / t)
            })
            .collect();
        let s = self.why_state;
        let ts = s.iter().sum::<u64>().max(1) as f64;
        out.push(format!(
            "no-melody why: {} | states chill {:.0}% awaken {:.0}% alive {:.0}% subside {:.0}%",
            w.join("; "),
            s[0] as f64 * 100.0 / ts,
            s[1] as f64 * 100.0 / ts,
            s[2] as f64 * 100.0 / ts,
            s[3] as f64 * 100.0 / ts
        ));
        out
    }
}

/// Stagnation detector state: how long since anything meaningful changed.
#[derive(Default)]
pub struct Stag {
    /// chord name per bar, last 16 bars
    pub chords: std::collections::VecDeque<String>,
    /// leader energy per bar, last 16 bars
    pub en: std::collections::VecDeque<f32>,
    /// sample time of the last player change (joins, lay-outs, new sounds)
    pub player_t: u64,
    /// sample time of the last section change
    pub mark_t: u64,
    pub score: f32,
    pub flagged: bool,
    /// earliest sample time the conductor may develop again
    pub cool: u64,
}
