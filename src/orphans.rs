//! Orphan-note bookkeeping: a melodic/comp onset (keys, lead, colour) with no other
//! onset from the same player within ~2 beats either side. Notes that start within
//! 40 ms of each other are one event (a chord). Whitelisted programs (bird tweet,
//! seashore, bells, timpani, percussive/FX programs) and long swells (a bar or
//! more) are idiomatic lone hits and are not counted.

/// GM programs where a sparse single hit is idiomatic
pub fn lone_ok(program: Option<u8>, steps: f32) -> bool {
    if steps >= 16.0 {
        return true; // a swell, not a stray note
    }
    match program {
        Some(p) => matches!(p, 9 | 14 | 47 | 55 | 96..=103 | 112..=127) || ((88..=95).contains(&p) && steps >= 8.0),
        None => false,
    }
}

#[derive(Default)]
pub struct Orphans {
    /// per player: (time of the last event, it had a neighbour before, it was exempt)
    pub last: [Option<(u64, bool, bool)>; 7],
    pub events: u64,
    pub orphans: u64,
    pub exempt: u64,
    pub per: [u64; 7],
    /// debug: what the last event of each player was (SYSJAM_ORPH_LOG)
    pub what: [String; 7],
    pub log: bool,
}

impl Orphans {
    /// one onset at sample time `t` for player `i`; `win` = 2 beats in samples
    pub fn note(&mut self, i: usize, t: u64, exempt: bool, win: u64, chord: u64) {
        if !(3..=5).contains(&i) {
            return;
        }
        match self.last[i] {
            Some((l, _, ex)) if t.abs_diff(l) <= chord => {
                // same event (a chord or a doubled note)
                self.last[i] = Some((l.min(t), self.last[i].unwrap().1, ex && exempt));
                return;
            }
            Some((l, had, ex)) => {
                if t.saturating_sub(l) <= win {
                    self.last[i] = Some((l, true, ex)); // neighbour after: not an orphan
                    self.close(i);
                    self.last[i] = Some((t, true, exempt));
                    return;
                }
                if !had {
                    self.count_orphan(ex);
                    if !ex {
                        self.per[i] += 1;
                        if self.log {
                            eprintln!("orphan p{i} at {} {}", l, self.what[i]);
                        }
                    }
                }
                self.events += 1;
            }
            None => {}
        }
        self.last[i] = Some((t, false, exempt));
    }
    fn close(&mut self, _i: usize) {
        self.events += 1;
    }
    fn count_orphan(&mut self, ex: bool) {
        if ex {
            self.exempt += 1;
        } else {
            self.orphans += 1;
        }
    }
    pub fn report(&self, secs: f64) -> String {
        let per10 = |x: u64| x as f64 * 600.0 / secs.max(1.0);
        format!("orphans: {} orphan notes ({:.1} per 10 min) of {} melodic/comp events, plus {} whitelisted lone hits ({:.1} per 10 min)", self.orphans, per10(self.orphans), self.events, self.exempt, per10(self.exempt))
    }
}
