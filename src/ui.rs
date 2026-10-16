//! Live terminal UI. The audio thread publishes a cheap snapshot (try_lock,
//! never blocks); the monitor thread publishes machine signals; this thread
//! redraws at ~5 fps. A slow terminal can only slow this thread down.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, terminal};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};
use crate::theme::th;

// ------------------------------------------------------------------ shared state

/// One band member's state of mind.
#[derive(Clone, Default)]
pub struct Mind {
    pub name: &'static str,
    pub status: &'static str,
    pub think: String,
    pub instrument: String,
    pub notes: Vec<String>,
    pub activity: f32,
    /// personality, e.g. "curious .82 · eager .31"
    pub traits: String,
    /// the player's own key, e.g. "E Dorian"
    pub key: String,
    /// true when it differs from the band key
    pub away: bool,
    /// the machine signal this player is tied to, with its level
    pub tied: String,
    /// the conductor is talking to this player right now
    pub addressed: bool,
}

/// What the conductor is doing (pose drives the little figure).
#[derive(Clone, Default)]
pub struct CondSnap {
    pub pose: crate::conductor::Pose,
    pub speech: String,
    pub target: Option<&'static str>,
    pub count: u32,
    pub clutter: f32,
}

#[derive(Clone, Default)]
pub struct BandSnap {
    pub key: String,
    /// (chord, roman, -1 past / 0 now / 1 planned)
    pub prog: Vec<(String, String, i8)>,
    pub bpm: f32,
    /// target while the tempo is ramping
    pub bpm_to: Option<f32>,
    pub bpm_locked: bool,
    /// "", "half-time", "double-time"
    pub feel: &'static str,
    /// home key, when the band is currently away from it
    pub home: Option<String>,
    pub bar: u64,
    pub beat: u32,
    pub state: &'static str,
    pub energy: f32,
    pub agitation: f32,
    pub cond: CondSnap,
    /// (player, full citation, variation, progress 0..1) while a theme is being quoted
    pub quote: Option<(String, String, String, f32)>,
    /// recent citations, oldest first ("mm:ss player: citation")
    pub cites: Vec<String>,
    pub minds: Vec<Mind>,
    /// sub, bass, mid, high in dBFS
    pub levels: [f32; 4],
    pub seed: u64,
    pub time: f32,
    pub version: u64,
    /// conductor style ("auto · House Band"), previous style while changing over
    pub style: String,
    pub style_from: Option<String>,
    /// user tempo bias in bpm (↑/↓, --tempo-bias)
    pub tempo_bias: f32,
    /// conductor costume for the style
    pub hat: &'static str,
    pub face: Option<&'static str>,
    pub style_id: &'static str,
    /// where the harmony is heading
    pub plan: String,
}

pub const HIST: usize = 120;

#[derive(Clone, Default)]
pub struct MonSnap {
    pub cpu: f32,
    pub cores: (usize, usize),
    pub net: f64,
    pub write: f64,
    pub fd: f64,
    /// combined keyboard + mouse activity
    pub input: f32,
    pub input_on: bool,
    /// short input source label ("evdev×3", "irq xhci≈", "none", "off")
    pub input_src: String,
    /// hue, saturation, brightness
    pub screen: Option<(f32, f32, f32)>,
    pub top: Vec<(String, f32)>,
    /// cpu, cores, net, disk, files, input — each 0..1
    pub hist: [VecDeque<f32>; 6],
}

pub static TUI_ON: AtomicBool = AtomicBool::new(false);
static LOG: Mutex<VecDeque<(Instant, String)>> = Mutex::new(VecDeque::new());

/// Event log sink used by `say!` while the TUI owns the terminal.
pub fn log(m: String) {
    let m = m.trim().to_string();
    // the header already shows every chord; keep key changes and band events
    if m.is_empty() || m.starts_with("Key:") || m.starts_with('[') {
        return;
    }
    if let Ok(mut l) = LOG.lock() {
        let now = Instant::now();
        // rate limit: at most ~4 lines per second
        let recent = l.iter().rev().take_while(|(t, _)| now.duration_since(*t) < Duration::from_secs(1)).count();
        if recent >= 4 {
            return;
        }
        l.push_back((now, m));
        while l.len() > 200 {
            l.pop_front();
        }
    }
}

// ------------------------------------------------------------------ terminal

fn restore() {
    let mut out = std::io::stdout();
    let _ = terminal::disable_raw_mode();
    let _ = execute!(out, terminal::LeaveAlternateScreen, cursor::Show);
    let _ = out.flush();
}

pub struct Ui {
    pub band: Arc<Mutex<BandSnap>>,
    pub mon: Arc<Mutex<MonSnap>>,
}

/// Run the TUI until `stop`. `quit` is called on q/Esc/Ctrl-C (first press fades,
/// the second exits at once).
pub fn run(ui: Ui, stop: Arc<AtomicBool>, quit: impl Fn() -> u32) {
    crate::shared::set_thread_nice(5);
    if terminal::enable_raw_mode().is_err() {
        return;
    }
    let mut out = std::io::stdout();
    if execute!(out, terminal::EnterAlternateScreen, cursor::Hide).is_err() {
        restore();
        return;
    }
    TUI_ON.store(true, Ordering::SeqCst);
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        prev(info);
    }));
    let mut term = match Terminal::new(CrosstermBackend::new(std::io::stdout())) {
        Ok(t) => t,
        Err(_) => {
            TUI_ON.store(false, Ordering::SeqCst);
            restore();
            return;
        }
    };
    let frame_dt = Duration::from_millis(200); // 5 fps
    let mut paused = false;
    let mut quitting = false;
    let mut last = Instant::now() - frame_dt;
    // held arrows auto-repeat fast: accept one tempo nudge per 160 ms, one style step per 300 ms
    let mut last_t = Instant::now() - Duration::from_secs(1);
    let mut last_s = Instant::now() - Duration::from_secs(1);
    let (mut band, mut mon) = (BandSnap::default(), MonSnap::default());
    while !stop.load(Ordering::Relaxed) {
        let wait = frame_dt.saturating_sub(last.elapsed()).min(Duration::from_millis(100));
        if event::poll(wait).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => match k.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                        quitting = true;
                        if quit() >= 1 {
                            break;
                        }
                    }
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        quitting = true;
                        if quit() >= 1 {
                            break;
                        }
                    }
                    KeyCode::Char(' ') => {
                        paused = !paused;
                        last = Instant::now() - frame_dt;
                    }
                    KeyCode::Up | KeyCode::Down if last_t.elapsed() >= Duration::from_millis(160) => {
                        last_t = Instant::now();
                        crate::shared::KEY_TEMPO.fetch_add(if k.code == KeyCode::Up { 1 } else { -1 }, Ordering::Relaxed);
                        last = Instant::now() - frame_dt;
                    }
                    KeyCode::Left | KeyCode::Right if last_s.elapsed() >= Duration::from_millis(300) => {
                        last_s = Instant::now();
                        crate::shared::KEY_STYLE.fetch_add(if k.code == KeyCode::Right { 1 } else { -1 }, Ordering::Relaxed);
                        crate::theme::fast_next();
                        last = Instant::now() - frame_dt;
                    }
                    _ => {}
                },
                Ok(Event::Resize(..)) => last = Instant::now() - frame_dt,
                _ => {}
            }
        }
        if last.elapsed() < frame_dt {
            continue;
        }
        last = Instant::now();
        if !paused {
            if let Ok(b) = ui.band.lock() {
                band = b.clone();
            }
            if let Ok(m) = ui.mon.lock() {
                mon = m.clone();
            }
        }
        let log: Vec<String> = LOG.lock().map(|l| l.iter().rev().take(40).map(|x| x.1.clone()).collect()).unwrap_or_default();
        let _ = term.draw(|f| draw(f, &band, &mon, &log, paused, quitting));
    }
    TUI_ON.store(false, Ordering::SeqCst);
    restore();
}

// ------------------------------------------------------------------ drawing

const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

fn meter(v: f32, w: usize) -> String {
    let (fc, ec) = crate::theme::meter_chars();
    let v = v.clamp(0.0, 1.0) * w as f32;
    let full = v.floor() as usize;
    if fc != '█' {
        // themed glyph meters: whole cells only
        let n = (v.round() as usize).min(w);
        return std::iter::repeat(fc).take(n).chain(std::iter::repeat(ec).take(w - n)).collect();
    }
    let mut s = "█".repeat(full.min(w));
    if full < w {
        let part = ((v - full as f32) * 8.0) as usize;
        s.push(if part == 0 { ' ' } else { [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'][part] });
        s.push_str(&" ".repeat(w - full - 1));
    }
    s
}

fn spark(h: &VecDeque<f32>, w: usize) -> String {
    let n = h.len();
    let mut s = String::new();
    for i in n.saturating_sub(w)..n {
        s.push(BARS[(h[i].clamp(0.0, 1.0) * 7.0).round() as usize]);
    }
    format!("{s:>w$}")
}

fn human(b: f64) -> String {
    if b >= 1e6 {
        format!("{:.1} MB/s", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} kB/s", b / 1e3)
    } else {
        format!("{:.0} B/s", b)
    }
}

fn hsv(h: f32, s: f32, v: f32) -> Color {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let f = |a: f32| ((a + m).clamp(0.0, 1.0) * 255.0) as u8;
    Color::Rgb(f(r), f(g), f(b))
}

fn state_color(s: &str) -> Color {
    match s {
        "chilling" => th(Color::Cyan),
        "awakening" => th(Color::Yellow),
        "alive" => th(Color::LightRed),
        "subsiding" => th(Color::Magenta),
        _ => th(Color::Gray),
    }
}

fn status_color(s: &str) -> Color {
    match s {
        "soloing" => th(Color::LightRed),
        "trading" => th(Color::LightYellow),
        "comping" => th(Color::LightGreen),
        "following" => th(Color::LightCyan),
        "laying out" => th(Color::DarkGray),
        "holding" => th(Color::Blue),
        "stretching" => th(Color::LightMagenta),
        "grooving" => th(Color::LightGreen),
        "quoting" => th(Color::LightYellow),
        _ => th(Color::Gray),
    }
}

fn draw(f: &mut Frame, b: &BandSnap, m: &MonSnap, log: &[String], paused: bool, quitting: bool) {
    let area = f.area();
    crate::theme::frame(b.style_id, b.beat);
    f.render_widget(Block::default().style(Style::default().bg(crate::theme::bg()).fg(th(Color::Gray))), area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(10), Constraint::Length(7)])
        .split(area);
    draw_header(f, rows[0], b, m, paused, quitting);
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(rows[1]);
    draw_band(f, mid[0], b);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Length(5), Constraint::Min(8), Constraint::Length(6)])
        .split(mid[1]);
    draw_conductor(f, right[0], b);
    draw_cites(f, right[1], b);
    draw_machine(f, right[2], m);
    draw_levels(f, right[3], b);
    draw_log(f, rows[2], log);
    if crate::theme::ascii() {
        // plain-ASCII terminals: transliterate every cell that is still non-ASCII
        for cell in f.buffer_mut().content.iter_mut() {
            if !cell.symbol().is_ascii() {
                let a = crate::theme::asciify(cell.symbol());
                cell.set_symbol(&a);
            }
        }
    }
}

fn draw_header(f: &mut Frame, r: Rect, b: &BandSnap, m: &MonSnap, paused: bool, quitting: bool) {
    let dim = Style::default().fg(th(Color::DarkGray));
    let mut prog = vec![Span::styled(format!(" {} ", b.key), Style::default().fg(th(Color::White)).add_modifier(Modifier::BOLD))];
    if let Some(h) = &b.home {
        prog.push(Span::styled(format!("(home {h}) "), dim));
    }
    prog.push(Span::styled("│ ", dim));
    for (i, (name, roman, when)) in b.prog.iter().enumerate() {
        if i > 0 {
            prog.push(Span::styled(" → ", dim));
        }
        let st = match when {
            0 => Style::default().fg(th(Color::Black)).bg(th(Color::LightYellow)).add_modifier(Modifier::BOLD),
            -1 => dim,
            _ => Style::default().fg(th(Color::Gray)),
        };
        prog.push(Span::styled(format!("{roman} {name}"), st));
    }
    if !b.plan.is_empty() {
        prog.push(Span::styled(crate::theme::asciify(&format!("   ⟶ {}", b.plan)), Style::default().fg(th(Color::LightCyan))));
    }
    let beat: String = (0..4).map(|i| if i == b.beat { '●' } else { '○' }).collect();
    let line2 = vec![
        match b.bpm_to {
            Some(t) => Span::styled(format!(" {:.0}→{:.0} bpm ", b.bpm, t), Style::default().fg(th(Color::LightYellow)).add_modifier(Modifier::BOLD)),
            None => Span::styled(format!(" {:>3.0} bpm{} ", b.bpm, if b.bpm_locked { " (locked)" } else { "" }), Style::default().fg(th(Color::White))),
        },
        Span::styled(
            if b.tempo_bias.abs() >= 0.5 { format!("tempo bias {:+.0} ", b.tempo_bias) } else { String::new() },
            Style::default().fg(th(Color::LightCyan)).add_modifier(Modifier::BOLD),
        ),
        Span::styled(if b.feel.is_empty() { String::new() } else { format!("{} ", b.feel) }, Style::default().fg(th(Color::LightMagenta))),
        Span::styled(format!("bar {:<4} ", b.bar + 1), dim),
        Span::styled(beat, Style::default().fg(th(Color::LightYellow))),
        Span::raw("   "),
        Span::styled(format!(" {} ", b.state), Style::default().fg(th(Color::Black)).bg(state_color(b.state)).add_modifier(Modifier::BOLD)),
        Span::raw("  energy "),
        Span::styled(meter(b.energy, 14), Style::default().fg(th(Color::LightGreen))),
        Span::styled(format!(" {:.2}", b.energy), dim),
        Span::raw("  agitation "),
        Span::styled(meter(b.agitation, 14), Style::default().fg(th(Color::LightRed))),
        Span::styled(format!(" {:.2}", b.agitation), dim),
    ];
    let mut tail = format!("  seed {}  {:>4.0}s", b.seed, b.time);
    if paused {
        tail.push_str("  [display paused]");
    }
    if quitting {
        tail.push_str("  [fading out…]");
    }
    let mut line2 = line2;
    line2.push(Span::styled(tail, dim));
    // input source + live typing / mouse meters, top-right on the border
    let src = if m.input_src.is_empty() { "…" } else { m.input_src.as_str() };
    let live = m.input_on && src != "none";
    let mut inp = vec![Span::styled(format!(" input {src} "), if live { Style::default().fg(th(Color::Gray)) } else { dim })];
    if live {
        inp.push(Span::styled("activity ", dim));
        inp.push(Span::styled(meter(m.input, 8), Style::default().fg(th(Color::LightGreen))));
        inp.push(Span::raw(" "));
    }
    // persistent "now quoting" line while a theme is in the air
    let line3 = match &b.quote {
        Some((who, cite, what, prog)) => Line::from(vec![
            Span::styled(" ♫ now quoting ", Style::default().fg(th(Color::Black)).bg(th(Color::LightYellow)).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {} ", meter(*prog, 6)), Style::default().fg(th(Color::LightYellow))),
            Span::styled(format!("{who}: "), Style::default().fg(th(Color::LightYellow))),
            Span::styled(cite.clone(), Style::default().fg(th(Color::White)).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" — {what}"), Style::default().fg(th(Color::Gray))),
        ]),
        None => Line::from(Span::styled(" ♫ no theme quoted right now", dim)),
    };
    let p = Paragraph::new(vec![Line::from(prog), Line::from(line2), line3]).block(
        Block::default()
            .borders(Borders::ALL).border_set(crate::theme::border_set()).border_style(Style::default().fg(crate::theme::border_color()))
            .title(Span::styled(crate::theme::title(" sysjam "), Style::default().fg(th(Color::LightYellow)).add_modifier(Modifier::BOLD)))
            .title(style_title(b))
            .title(Line::from(inp).right_aligned())
            .title_bottom(Span::styled(crate::theme::asciify(" ↑↓ tempo  ←→ style  space pause  q quit "), dim)),
    );
    f.render_widget(p, r);
}

fn style_title(b: &BandSnap) -> Line<'static> {
    let mut v = vec![Span::styled(format!(" {} style {} ", crate::theme::icon(), b.style), Style::default().fg(th(Color::Black)).bg(th(Color::LightCyan)).add_modifier(Modifier::BOLD))];
    if let Some(from) = &b.style_from {
        v.push(Span::styled(crate::theme::asciify(&format!(" ← {from}, changing over… ")), Style::default().fg(th(Color::LightCyan))));
    }
    Line::from(v)
}

/// scrolling history of theme citations (newest at the bottom)
fn draw_cites(f: &mut Frame, r: Rect, b: &BandSnap) {
    let block = Block::default().borders(Borders::ALL).border_set(crate::theme::border_set()).border_style(Style::default().fg(crate::theme::border_color())).title(crate::theme::title(" quoted themes · public domain "));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let h = inner.height as usize;
    let lines: Vec<Line> = if b.cites.is_empty() {
        vec![Line::from(Span::styled("none yet — themes come rarely", Style::default().fg(th(Color::DarkGray))))]
    } else {
        let n = b.cites.len();
        b.cites[n.saturating_sub(h)..]
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let last = n.saturating_sub(h) + i + 1 == n;
                Line::from(Span::styled(c.clone(), Style::default().fg(if last { th(Color::LightYellow) } else { th(Color::Gray) })))
            })
            .collect()
    };
    f.render_widget(Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: true }), inner);
}

fn draw_conductor(f: &mut Frame, r: Rect, b: &BandSnap) {
    use crate::conductor::{frame, Pose};
    let c = &b.cond;
    let sn = b.style.rsplit(" · ").next().unwrap_or("");
    let title = format!(" conductor · {sn} · {} cue{} ", c.count, if c.count == 1 { "" } else { "s" });
    let block = Block::default().borders(Borders::ALL).border_set(crate::theme::border_set()).border_style(Style::default().fg(crate::theme::border_color())).title(crate::theme::title(&title));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let col = match c.pose {
        Pose::Calm => th(Color::LightYellow),
        Pose::Shh => th(Color::LightMagenta),
        Pose::Point => th(Color::LightCyan),
        Pose::Surge => th(Color::LightRed),
        Pose::Rest => th(Color::DarkGray),
    };
    let art = crate::theme::conductor(frame(c.pose, b.beat), c.pose, b.beat, b.hat, b.face, b.style_id);
    let w = inner.width as usize;
    let tw = w.saturating_sub(13).max(8);
    // speech, word-wrapped beside the figure
    let mut said: Vec<String> = Vec::new();
    let mut cur = String::new();
    let (mo, mc) = crate::theme::marks();
    let text = if c.speech.is_empty() { String::new() } else { format!("{mo}{}{mc}", c.speech) };
    for word in text.split_whitespace() {
        if cur.chars().count() + word.chars().count() + 1 > tw && !cur.is_empty() {
            said.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        said.push(cur);
    }
    let mut lines = Vec::new();
    for (k, a) in art.iter().enumerate() {
        let mut sp = vec![Span::styled(format!("{a:<12} "), Style::default().fg(col).add_modifier(Modifier::BOLD))];
        let row = k.checked_sub(1).and_then(|j| said.get(j));
        if let Some(t) = row {
            sp.push(Span::styled(t.clone(), Style::default().fg(th(Color::White))));
        } else if k == 4 {
            let tgt = c.target.map(|t| format!("→ {t}  ")).unwrap_or_default();
            sp.push(Span::styled(format!("{tgt}clutter {}", meter(c.clutter.min(1.5) / 1.5, 8)), Style::default().fg(th(Color::DarkGray))));
        }
        lines.push(Line::from(sp));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_band(f: &mut Frame, r: Rect, b: &BandSnap) {
    let block = Block::default().borders(Borders::ALL).border_set(crate::theme::border_set()).border_style(Style::default().fg(crate::theme::border_color())).title(crate::theme::title(" the band "));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let n = b.minds.len().max(1);
    let per = (inner.height as usize / n).clamp(1, 3);
    let dim = Style::default().fg(th(Color::DarkGray));
    let w = inner.width as usize;
    let mut lines = Vec::new();
    for mi in &b.minds {
        let st = status_color(mi.status);
        let mut l1 = vec![
            Span::styled(
                format!("{:<9}", mi.name),
                if mi.addressed { Style::default().fg(th(Color::Black)).bg(th(Color::LightYellow)).add_modifier(Modifier::BOLD) } else { Style::default().fg(th(Color::White)).add_modifier(Modifier::BOLD) },
            ),
            Span::styled(format!("{:<11}", mi.status), Style::default().fg(st)),
            Span::styled(meter(mi.activity, 8), Style::default().fg(st)),
            Span::raw(" "),
            Span::styled(
                format!("{:<14}", mi.key),
                if mi.away { Style::default().fg(th(Color::Black)).bg(th(Color::LightMagenta)).add_modifier(Modifier::BOLD) } else { Style::default().fg(th(Color::Gray)) },
            ),
            Span::raw(" "),
        ];
        let used = 9 + 11 + 8 + 1 + 15;
        let inst: String = mi.instrument.chars().take(w.saturating_sub(used)).collect();
        l1.push(Span::styled(inst, Style::default().fg(th(Color::LightBlue))));
        lines.push(Line::from(l1));
        if per >= 2 {
            let t: String = format!("  ↳ {}", mi.think).chars().take(w).collect();
            let ts = if mi.addressed { Style::default().fg(th(Color::LightYellow)).add_modifier(Modifier::ITALIC) } else { Style::default().fg(th(Color::Gray)).add_modifier(Modifier::ITALIC) };
            lines.push(Line::from(Span::styled(t, ts)));
        }
        if per >= 3 {
            let notes = if mi.notes.is_empty() { "·".to_string() } else { mi.notes.join(" ") };
            // right side: tied signal and traits
            let right = match (mi.tied.is_empty(), mi.traits.is_empty()) {
                (true, true) => String::new(),
                (false, true) => format!(" ⇢ {} ", mi.tied),
                (true, false) => format!(" {} ", mi.traits),
                _ => format!(" ⇢ {} · {} ", mi.tied, mi.traits),
            };
            let rw = right.chars().count();
            let left_w = w.saturating_sub(rw).max(8);
            let mut left: String = format!("  ♪ {notes}").chars().take(left_w).collect();
            let lw = left.chars().count();
            if lw + rw <= w {
                left.push_str(&" ".repeat(w - lw - rw));
                lines.push(Line::from(vec![Span::styled(left, dim), Span::styled(right, Style::default().fg(th(Color::Cyan)).add_modifier(Modifier::DIM))]));
            } else {
                lines.push(Line::from(Span::styled(left, dim)));
            }
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_machine(f: &mut Frame, r: Rect, m: &MonSnap) {
    let block = Block::default().borders(Borders::ALL).border_set(crate::theme::border_set()).border_style(Style::default().fg(crate::theme::border_color())).title(crate::theme::title(" machine "));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let w = inner.width as usize;
    let sw = w.saturating_sub(23).clamp(4, HIST);
    let dim = Style::default().fg(th(Color::DarkGray));
    let row = |label: &str, val: String, h: &VecDeque<f32>, c: Color| {
        Line::from(vec![
            Span::styled(format!("{label:<7}"), Style::default().fg(th(Color::White))),
            Span::styled(format!("{val:>11} "), Style::default().fg(c)),
            Span::styled(spark(h, sw), Style::default().fg(c)),
        ])
    };
    let mut lines = vec![
        row("cpu", format!("{:.0}%", m.cpu * 100.0), &m.hist[0], th(Color::LightRed)),
        row("cores", format!("{}/{}", m.cores.0, m.cores.1), &m.hist[1], th(Color::Red)),
        row("net", human(m.net), &m.hist[2], th(Color::LightCyan)),
        row("disk", human(m.write), &m.hist[3], th(Color::LightBlue)),
        row("files", format!("{:.0}/s", m.fd), &m.hist[4], th(Color::LightMagenta)),
    ];
    if m.input_on {
        lines.push(row("input", format!("{:.2}", m.input), &m.hist[5], th(Color::LightGreen)));
    } else {
        lines.push(Line::from(Span::styled("input  activity off", dim)));
    }
    match m.screen {
        Some((h, s, v)) => lines.push(Line::from(vec![
            Span::styled(format!("{:<7}", "screen"), Style::default().fg(th(Color::White))),
            Span::styled("      ", Style::default().bg(hsv(h, s.max(0.15), v.max(0.25)))),
            Span::styled(format!(" hue {h:.0}° bri {v:.2}"), dim),
        ])),
        None => lines.push(Line::from(Span::styled("screen colour off", dim))),
    }
    lines.push(Line::from(Span::styled("top threads", Style::default().fg(th(Color::White)))));
    for (name, cpu) in m.top.iter().take(inner.height.saturating_sub(lines.len() as u16) as usize) {
        let n: String = name.chars().take(w.saturating_sub(16)).collect();
        lines.push(Line::from(vec![
            Span::styled(format!(" {:>4.0}% ", cpu * 100.0), Style::default().fg(th(Color::LightRed))),
            Span::styled(meter(*cpu, 6), Style::default().fg(th(Color::Red))),
            Span::raw(" "),
            Span::styled(n, dim),
        ]));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_levels(f: &mut Frame, r: Rect, b: &BandSnap) {
    let block = Block::default().borders(Borders::ALL).border_set(crate::theme::border_set()).border_style(Style::default().fg(crate::theme::border_color())).title(crate::theme::title(" levels "));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let w = (inner.width as usize).saturating_sub(16).max(4);
    let names = ["sub", "bass", "mid", "high"];
    let cols = [th(Color::Magenta), th(Color::LightMagenta), th(Color::LightYellow), th(Color::LightCyan)];
    let lines: Vec<Line> = (0..4)
        .map(|i| {
            let db = b.levels[i];
            let v = ((db + 60.0) / 54.0).clamp(0.0, 1.0);
            Line::from(vec![
                Span::styled(format!("{:<5}", names[i]), Style::default().fg(th(Color::White))),
                Span::styled(meter(v, w), Style::default().fg(cols[i])),
                Span::styled(format!(" {:>5.0} dB", db.max(-99.0)), Style::default().fg(th(Color::DarkGray))),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_log(f: &mut Frame, r: Rect, log: &[String]) {
    let block = Block::default().borders(Borders::ALL).border_set(crate::theme::border_set()).border_style(Style::default().fg(crate::theme::border_color())).title(crate::theme::title(" band events "));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let n = inner.height as usize;
    let lines: Vec<Line> = log
        .iter()
        .take(n)
        .rev()
        .map(|m| {
            let c = if m.starts_with('~') {
                th(Color::LightYellow)
            } else if m.contains("key change") || m.contains("modulation") {
                th(Color::LightMagenta)
            } else if m.contains("industrial") {
                th(Color::LightRed)
            } else if m.starts_with('+') || m.starts_with('\u{2212}') {
                th(Color::LightBlue)
            } else if m.contains("solo") || m.contains("joins") || m.contains("breakdown") {
                th(Color::LightGreen)
            } else {
                th(Color::Gray)
            };
            Line::from(Span::styled(m.clone(), Style::default().fg(c)))
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}
