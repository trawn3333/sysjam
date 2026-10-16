//! Learning from the like button, and only from it. Pressing `l` (or Enter) snapshots
//! the moment (style, tempo, state, key and mode, instruments, density, who is
//! playing, the quote in the air) and nudges a small table of preference weights
//! toward it. The conductor then leans toward similar moments: which style auto mode
//! wanders to, tempos, instruments, who he invites, quotes, major vs minor, density.
//! The one other explicit button is `n` (next idea): it stores a small negative
//! "skip" for the moment you moved on from (a fifth of a like, the other way).
//! Nothing else is ever learned: not history, not usage.
//!
//! Stored as human-readable YAML in $XDG_CONFIG_HOME/sysjam/likes.yaml
//! (default ~/.config/sysjam/likes.yaml). Delete it or run --reset-likes to forget.

use std::collections::BTreeMap;
use std::path::PathBuf;

pub const GROUPS: [&str; 8] = ["style", "state", "tempo", "mode", "density", "instruments", "players", "quote"];

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub time: String,
    pub style: String,
    pub style_id: String,
    pub tempo: f32,
    pub state: String,
    pub key: String,
    pub minor: bool,
    pub density: f32,
    pub instruments: Vec<String>,
    pub players: Vec<String>,
    pub quote: Option<String>,
}

impl Snapshot {
    /// the (group, value) features this moment lights up
    pub fn features(&self) -> Vec<(&'static str, String)> {
        let mut v = vec![
            ("style", self.style_id.clone()),
            ("state", self.state.clone()),
            ("tempo", tempo_bucket(self.tempo).to_string()),
            ("mode", if self.minor { "minor" } else { "major" }.to_string()),
            ("density", density_bucket(self.density).to_string()),
        ];
        for i in &self.instruments {
            v.push(("instruments", i.clone()));
        }
        for p in &self.players {
            v.push(("players", p.clone()));
        }
        // only a liked quote counts (liking a moment without one says nothing about quotes)
        if self.quote.is_some() {
            v.push(("quote", "yes".to_string()));
        }
        v
    }
}

pub fn tempo_bucket(bpm: f32) -> &'static str {
    match bpm as i32 {
        ..=69 => "under 70",
        70..=89 => "70-89",
        90..=109 => "90-109",
        110..=129 => "110-129",
        _ => "130 up",
    }
}
pub fn density_bucket(d: f32) -> &'static str {
    if d < 0.33 {
        "sparse"
    } else if d < 0.66 {
        "medium"
    } else {
        "busy"
    }
}

/// one-hot groups: a like raises the liked value and lowers its siblings a little
fn one_hot(g: &str) -> Option<&'static [&'static str]> {
    Some(match g {
        "state" => &["chilling", "awakening", "alive", "subsiding"],
        "tempo" => &["under 70", "70-89", "90-109", "110-129", "130 up"],
        "mode" => &["major", "minor"],
        "density" => &["sparse", "medium", "busy"],
        _ => return None,
    })
}

#[derive(Default)]
pub struct Likes {
    pub path: Option<PathBuf>,
    pub bias: BTreeMap<String, BTreeMap<String, f32>>,
    pub likes: Vec<Snapshot>,
    /// moments skipped with `n` (style, state, key, tempo; a small negative each)
    pub skips: Vec<Snapshot>,
    /// liked something this session
    pub dirty: bool,
}

pub fn config_path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(x) => PathBuf::from(x),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("sysjam").join("likes.yaml"))
}

const LR: f32 = 0.3;
/// a skip weighs a fifth of a like, the other way
const SKIP: f32 = 0.06;

impl Likes {
    pub fn load() -> Likes {
        let path = config_path();
        let mut l = Likes { path: path.clone(), ..Default::default() };
        if let Some(text) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            l.parse(&text);
        }
        l
    }

    pub fn reset() -> Result<Option<PathBuf>, String> {
        let Some(p) = config_path() else { return Ok(None) };
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            return Ok(Some(p));
        }
        Ok(None)
    }

    /// weight of a feature value (0 = neutral)
    pub fn b(&self, group: &str, value: &str) -> f32 {
        self.bias.get(group).and_then(|m| m.get(value)).copied().unwrap_or(0.0)
    }

    /// the reward-weighted update for one like
    pub fn like(&mut self, s: Snapshot) {
        for (g, v) in s.features() {
            let m = self.bias.entry(g.to_string()).or_default();
            if let Some(all) = one_hot(g) {
                let k = (all.len() - 1).max(1) as f32;
                for o in all {
                    let e = m.entry(o.to_string()).or_insert(0.0);
                    *e = if *o == v { *e + LR } else { *e - LR / k };
                }
            } else if g == "style" {
                *m.entry(v).or_insert(0.0) += LR;
                // a like for one style is a gentle vote against the others
                let n = crate::style::STYLES.len() as f32 - 1.0;
                for st in crate::style::STYLES {
                    if st.id != s.style_id {
                        *m.entry(st.id.to_string()).or_insert(0.0) -= LR / n;
                    }
                }
            } else {
                *m.entry(v).or_insert(0.0) += LR * 0.6;
            }
            for x in m.values_mut() {
                *x = x.clamp(-2.0, 2.0);
            }
        }
        self.likes.push(s);
        self.dirty = true;
    }

    /// the small negative for one `n` press: only the coarse context (style, state,
    /// tempo, mode, density) is pushed down; instruments and players are left alone
    pub fn skip(&mut self, s: Snapshot) {
        for (g, v) in s.features() {
            if matches!(g, "instruments" | "players" | "quote") {
                continue;
            }
            let m = self.bias.entry(g.to_string()).or_default();
            if let Some(all) = one_hot(g) {
                let k = (all.len() - 1).max(1) as f32;
                for o in all {
                    let e = m.entry(o.to_string()).or_insert(0.0);
                    *e = if *o == v { *e - SKIP } else { *e + SKIP / k };
                }
            } else {
                *m.entry(v).or_insert(0.0) -= SKIP;
            }
            for x in m.values_mut() {
                *x = x.clamp(-2.0, 2.0);
            }
        }
        self.skips.push(s);
        while self.skips.len() > 200 {
            self.skips.remove(0);
        }
        self.dirty = true;
    }

    pub fn to_yaml(&self) -> String {
        let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
        let mut o = String::new();
        o.push_str("# sysjam likes: written when you press `l` in the TUI. Plain YAML, safe to edit.\n");
        o.push_str("# Only these likes (and the small negative of each `n` skip) are learned from; delete this file or run `sysjam --reset-likes` to forget.\n");
        o.push_str("version: 1\n");
        o.push_str("# preference weights the conductor leans on (0 = neutral, clamped to ±2)\n");
        o.push_str("learned_bias:\n");
        for g in GROUPS {
            let Some(m) = self.bias.get(g) else { continue };
            let nz: Vec<(&String, &f32)> = m.iter().filter(|(_, v)| v.abs() >= 0.005).collect();
            if nz.is_empty() {
                continue;
            }
            o.push_str(&format!("  {g}:\n"));
            for (k, v) in nz {
                o.push_str(&format!("    {}: {:.3}\n", q(k), v));
            }
        }
        o.push_str(&format!("likes:{}\n", if self.likes.is_empty() { " []" } else { "" }));
        for s in &self.likes {
            o.push_str(&format!("  - time: {}\n", q(&s.time)));
            o.push_str(&format!("    style: {}\n", q(&s.style)));
            o.push_str(&format!("    style_id: {}\n", q(&s.style_id)));
            o.push_str(&format!("    tempo: {:.0}\n", s.tempo));
            o.push_str(&format!("    state: {}\n", q(&s.state)));
            o.push_str(&format!("    key: {}\n", q(&s.key)));
            o.push_str(&format!("    mode: {}\n", if s.minor { "minor" } else { "major" }));
            o.push_str(&format!("    density: {:.2}\n", s.density));
            o.push_str(&format!("    instruments: [{}]\n", s.instruments.iter().map(|x| q(x)).collect::<Vec<_>>().join(", ")));
            o.push_str(&format!("    players: [{}]\n", s.players.iter().map(|x| q(x)).collect::<Vec<_>>().join(", ")));
            o.push_str(&format!("    quote: {}\n", s.quote.as_deref().map(q).unwrap_or_else(|| "null".into())));
        }
        o.push_str("# moments you moved on from with `n` (each a small negative)\n");
        o.push_str(&format!("skips:{}\n", if self.skips.is_empty() { " []" } else { "" }));
        for s in &self.skips {
            o.push_str(&format!("  - time: {}\n", q(&s.time)));
            o.push_str(&format!("    style: {}\n", q(&s.style)));
            o.push_str(&format!("    style_id: {}\n", q(&s.style_id)));
            o.push_str(&format!("    tempo: {:.0}\n", s.tempo));
            o.push_str(&format!("    state: {}\n", q(&s.state)));
            o.push_str(&format!("    key: {}\n", q(&s.key)));
            o.push_str(&format!("    mode: {}\n", if s.minor { "minor" } else { "major" }));
            o.push_str(&format!("    density: {:.2}\n", s.density));
        }
        o
    }

    /// reads what to_yaml writes (and tolerates hand edits of the values)
    fn parse(&mut self, text: &str) {
        let unq = |s: &str| -> String {
            let s = s.trim();
            if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
                s[1..s.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
            } else {
                s.to_string()
            }
        };
        let list = |s: &str| -> Vec<String> {
            let s = s.trim().trim_start_matches('[').trim_end_matches(']');
            let mut out = Vec::new();
            let mut cur = String::new();
            let mut inq = false;
            let mut esc = false;
            for ch in s.chars() {
                if esc {
                    cur.push(ch);
                    esc = false;
                    continue;
                }
                match ch {
                    '\\' if inq => esc = true,
                    '"' => inq = !inq,
                    ',' if !inq => {
                        if !cur.trim().is_empty() {
                            out.push(cur.trim().to_string());
                        }
                        cur.clear();
                    }
                    _ => cur.push(ch),
                }
            }
            if !cur.trim().is_empty() {
                out.push(cur.trim().to_string());
            }
            out
        };
        let mut section = "";
        let mut group = String::new();
        let mut cur: Option<Snapshot> = None;
        for line in text.lines() {
            let t = line.trim_end();
            if t.trim_start().starts_with('#') || t.trim().is_empty() {
                continue;
            }
            let indent = t.len() - t.trim_start().len();
            let body = t.trim_start();
            if indent == 0 {
                if let Some(s) = cur.take() {
                    if section == "skips" { self.skips.push(s) } else { self.likes.push(s) }
                }
                section = if body.starts_with("learned_bias:") {
                    "bias"
                } else if body.starts_with("likes:") {
                    "likes"
                } else if body.starts_with("skips:") {
                    "skips"
                } else {
                    ""
                };
                continue;
            }
            if section == "bias" {
                if indent == 2 {
                    group = body.trim_end_matches(':').trim().to_string();
                } else if let Some((k, v)) = body.rsplit_once(':') {
                    if let Ok(x) = v.trim().parse::<f32>() {
                        self.bias.entry(group.clone()).or_default().insert(unq(k), x.clamp(-2.0, 2.0));
                    }
                }
            } else if section == "likes" || section == "skips" {
                let (body, start) = match body.strip_prefix("- ") {
                    Some(b) => (b, true),
                    None => (body, false),
                };
                if start {
                    if let Some(s) = cur.take() {
                        if section == "skips" { self.skips.push(s) } else { self.likes.push(s) }
                    }
                    cur = Some(Snapshot::default());
                }
                let Some(s) = cur.as_mut() else { continue };
                let Some((k, v)) = body.split_once(':') else { continue };
                let v = v.trim();
                match k.trim() {
                    "time" => s.time = unq(v),
                    "style" => s.style = unq(v),
                    "style_id" => s.style_id = unq(v),
                    "tempo" => s.tempo = v.parse().unwrap_or(0.0),
                    "state" => s.state = unq(v),
                    "key" => s.key = unq(v),
                    "mode" => s.minor = v == "minor",
                    "density" => s.density = v.parse().unwrap_or(0.0),
                    "instruments" => s.instruments = list(v).iter().map(|x| unq(x)).collect(),
                    "players" => s.players = list(v).iter().map(|x| unq(x)).collect(),
                    "quote" => s.quote = if v == "null" || v.is_empty() { None } else { Some(unq(v)) },
                    _ => {}
                }
            }
        }
        if let Some(s) = cur.take() {
            if section == "skips" { self.skips.push(s) } else { self.likes.push(s) }
        }
    }

    /// write the file off the audio thread
    pub fn save_async(&self) {
        let Some(p) = self.path.clone() else { return };
        let text = self.to_yaml();
        let _ = std::thread::Builder::new().name("sysjam-likes".into()).spawn(move || write(&p, &text));
    }

    /// write the file now (at exit, if anything was liked this session)
    pub fn save(&self) {
        if let Some(p) = self.path.as_ref() {
            write(p, &self.to_yaml());
        }
    }
}

fn write(p: &std::path::Path, text: &str) {
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = p.with_extension("yaml.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, p);
    }
}

/// local time as an ISO 8601 string with offset (no extra crates)
pub fn now_iso() -> String {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        let off = tm.tm_gmtoff / 60;
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
            if off < 0 { '-' } else { '+' },
            off.abs() / 60,
            off.abs() % 60
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn yaml_round_trip() {
        let mut l = Likes::default();
        l.like(Snapshot {
            time: "2026-10-15T20:00:00-05:00".into(),
            style: "Country".into(),
            style_id: "country".into(),
            tempo: 96.0,
            state: "awakening".into(),
            key: "G major".into(),
            minor: false,
            density: 0.4,
            instruments: vec!["fiddle".into(), "banjo \"5-string\"".into()],
            players: vec!["bass".into(), "drums".into()],
            quote: Some("Turkey in the Straw".into()),
        });
        let y = l.to_yaml();
        let mut m = Likes::default();
        m.parse(&y);
        assert_eq!(m.likes.len(), 1);
        assert_eq!(m.likes[0].instruments[1], "banjo \"5-string\"");
        assert_eq!(m.likes[0].quote.as_deref(), Some("Turkey in the Straw"));
        assert!((m.b("style", "country") - l.b("style", "country")).abs() < 1e-3);
        assert!(m.b("tempo", "90-109") > 0.29);
        assert_eq!(m.to_yaml(), y);
        // a skip is a small negative, stored separately, and round-trips too
        let before = m.b("style", "country");
        m.skip(Snapshot { time: "t".into(), style: "Country".into(), style_id: "country".into(), tempo: 96.0, state: "chilling".into(), key: "G major".into(), ..Default::default() });
        assert!(m.b("style", "country") < before && before - m.b("style", "country") < 0.1);
        let y2 = m.to_yaml();
        let mut k = Likes::default();
        k.parse(&y2);
        assert_eq!(k.likes.len(), 1);
        assert_eq!(k.skips.len(), 1);
        assert_eq!(k.to_yaml(), y2);
    }
}
