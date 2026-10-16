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
    /// instrument family (drives the little character)
    pub fam: crate::figure::Fam,
    pub playing: bool,
    /// conductor's hush level (1 = none)
    pub hushed: f32,
    pub quoting: bool,
    /// on stage (the guest comes and goes)
    pub present: bool,
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
    /// conductor mood (index into mood::ALL), its weight, baton energy, brain line
    pub mood: usize,
    pub mood_w: f32,
    pub mood_e: f32,
    pub brain: String,
    /// seconds since the last ♥, and how many likes are stored
    pub liked: f32,
    pub likes_n: usize,
    /// the audience's excitement 0..1
    pub crowd_hype: f32,
    /// seconds since the last `n` (next idea)
    pub skipped: f32,
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
    pub crowd: crate::audience::CrowdSnap,
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
    // ~15 fps while the little people are moving, 5 fps when the display is paused;
    // every beat also gets a frame exactly on it, so the baton's ictus lands on time
    let fast = Duration::from_millis(66);
    let slow = Duration::from_millis(200);
    let mut frame_dt = fast;
    let mut paused = false;
    let mut quitting = false;
    let mut last = Instant::now() - slow;
    let mut last_beat = i64::MIN;
    // held arrows auto-repeat fast: accept one tempo nudge per 160 ms, one style step per 300 ms
    let mut last_t = Instant::now() - Duration::from_secs(1);
    let mut last_s = Instant::now() - Duration::from_secs(1);
    let mut last_l = Instant::now() - Duration::from_secs(1);
    let mut last_n = Instant::now() - Duration::from_secs(1);
    let (mut band, mut mon) = (BandSnap::default(), MonSnap::default());
    let mut anim = Anim::new();
    let mut cache: Option<ratatui::buffer::Buffer> = None;
    let mut last_full = Instant::now() - slow;
    let mut force_full = true;
    let mut dirty = false;
    let mut prof = (Duration::ZERO, Duration::ZERO, 0u64, Instant::now(), 0u64);
    let mut last_cells: Vec<(u16, u16, String, usize, Style)> = Vec::new();
    let mut sync_log = std::env::var_os("SYSJAM_SYNCLOG").and_then(|p| std::fs::File::create(p).ok()).map(std::io::BufWriter::new);
    while !stop.load(Ordering::Relaxed) {
        // time to the next beat, as heard
        let heard = crate::clock::heard_at(crate::clock::ns());
        let pos = heard.and_then(crate::clock::pos_at);
        let to_beat = pos.map(|p| {
            let beat_s = p.len * 4.0 / crate::clock::rate().max(1.0);
            Duration::from_secs_f64(((1.0 - p.beats().fract()) * beat_s).max(0.0))
        });
        let mut wait = frame_dt.saturating_sub(last.elapsed());
        if let Some(tb) = to_beat {
            wait = wait.min(tb + Duration::from_micros(300));
        }
        if event::poll(wait.min(Duration::from_millis(100))).unwrap_or(false) {
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
                        last = Instant::now() - slow;
                        force_full = true;
                    }
                    KeyCode::Char('l') | KeyCode::Char('L') | KeyCode::Enter if last_l.elapsed() >= Duration::from_millis(400) => {
                        last_l = Instant::now();
                        crate::shared::KEY_LIKE.fetch_add(1, Ordering::Relaxed);
                        last = Instant::now() - slow;
                        force_full = true;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Tab if last_n.elapsed() >= Duration::from_millis(350) => {
                        last_n = Instant::now();
                        crate::shared::KEY_NEXT.fetch_add(1, Ordering::Relaxed);
                        last = Instant::now() - slow;
                        force_full = true;
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') if crate::audience::ENABLED.load(Ordering::Relaxed) => {
                        // force-toggle (otherwise it follows the terminal size)
                        let on = crate::audience::SHOWN.load(Ordering::Relaxed);
                        crate::audience::FORCE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
                        last = Instant::now() - slow;
                        force_full = true;
                    }
                    KeyCode::Up | KeyCode::Down if last_t.elapsed() >= Duration::from_millis(160) => {
                        last_t = Instant::now();
                        crate::shared::KEY_TEMPO.fetch_add(if k.code == KeyCode::Up { 1 } else { -1 }, Ordering::Relaxed);
                        last = Instant::now() - slow;
                        force_full = true;
                    }
                    KeyCode::Left | KeyCode::Right if last_s.elapsed() >= Duration::from_millis(300) => {
                        last_s = Instant::now();
                        crate::shared::KEY_STYLE.fetch_add(if k.code == KeyCode::Right { 1 } else { -1 }, Ordering::Relaxed);
                        crate::theme::fast_next();
                        last = Instant::now() - slow;
                        force_full = true;
                    }
                    _ => {}
                },
                Ok(Event::Resize(..)) => {
                    last = Instant::now() - slow;
                    force_full = true;
                }
                _ => {}
            }
        }
        let now_ns = crate::clock::ns();
        let heard = crate::clock::heard_at(now_ns);
        let pos = heard.and_then(crate::clock::pos_at);
        let beat_i = pos.map(|p| p.beats().floor() as i64).unwrap_or(last_beat);
        let on_beat = beat_i != last_beat;
        if last.elapsed() < frame_dt && !(on_beat && !paused) {
            continue;
        }
        last = Instant::now();
        last_beat = beat_i;
        // the full UI refreshes at 5 fps (or at once after a key); in between only the
        // little people are redrawn over the cached frame
        let full = force_full || last_full.elapsed() >= Duration::from_millis(300) || cache.is_none();
        if full && !paused {
            if let Ok(b) = ui.band.lock() {
                if b.version != band.version {
                    band = b.clone();
                }
            }
            if let Ok(m) = ui.mon.lock() {
                mon = m.clone();
            }
        }
        // idle (display paused, or no audio yet): slow down
        frame_dt = if paused || pos.is_none() { slow } else { fast };
        let ctx = Ctx { heard: if paused { None } else { heard }, pos: if paused { None } else { pos }, t: anim.t0.elapsed().as_secs_f32() };
        let tp = Instant::now();
        let mut tw = Duration::ZERO;
        if full {
            if dirty {
                if let Some(c) = cache.as_ref() {
                    let mut out = std::io::stdout().lock();
                    for cell in &last_cells {
                        emit(&mut out, c, cell.0, cell.1, None, cell.3);
                    }
                    let _ = crossterm::queue!(out, crossterm::style::SetAttribute(crossterm::style::Attribute::Reset), crossterm::style::ResetColor);
                    let _ = out.flush();
                }
                dirty = false;
            }
            let log: Vec<String> = LOG.lock().map(|l| l.iter().rev().take(40).map(|x| x.1.clone()).collect()).unwrap_or_default();
            let _ = term.draw(|f| {
                let t = Instant::now();
                draw(f, &band, &mon, &log, paused, quitting, &mut anim, &ctx);
                cache = Some(f.buffer_mut().clone());
                tw = t.elapsed();
            });
            last_full = Instant::now();
            force_full = false;
            last_cells.clear();
        } else {
            let t = Instant::now();
            let cells = anim_cells(&band, &mut anim, &ctx);
            let same = cells.len() == last_cells.len() && cells.iter().zip(&last_cells).all(|(a, b)| a.2 == b.2 && a.4 == b.4);
            tw = t.elapsed();
            if !same {
                // straight to the terminal, only the lines that changed; ratatui's own idea of
                // the screen (the cached full frame) is restored before its next full draw
                if let Some(c) = cache.as_ref() {
                    let mut out = std::io::stdout().lock();
                    for (i, cell) in cells.iter().enumerate() {
                        if last_cells.get(i).map_or(true, |l| l.2 != cell.2 || l.4 != cell.4 || l.0 != cell.0 || l.1 != cell.1) {
                            emit(&mut out, c, cell.0, cell.1, Some((&cell.2, cell.4)), cell.3);
                        }
                    }
                    let _ = crossterm::queue!(out, crossterm::style::SetAttribute(crossterm::style::Attribute::Reset), crossterm::style::ResetColor);
                    let _ = out.flush();
                    dirty = true;
                }
                last_cells = cells;
            } else {
                prof.4 += 1;
            }
        }
        prof.0 += tw;
        prof.1 += tp.elapsed() - tw;
        prof.2 += 1;
        if let (Some(w), Some(p), Some(h)) = (sync_log.as_mut(), ctx.pos, ctx.heard) {
            // frame shown at (now): wall ns, heard sample, beat position, and when the drawn frame hit the terminal
            let _ = writeln!(w, "{now_ns} {:.1} {:.4} {} {}", h, p.beats(), crate::clock::ns(), crate::clock::SINK_T0.load(Ordering::Relaxed));
        }
    }
    if let Some(p) = std::env::var_os("SYSJAM_UIPROF") {
        let el = prof.3.elapsed().as_secs_f64();
        let _ = std::fs::write(p, format!("frames {} ({} unchanged, skipped) in {el:.1}s; widgets {:.3}% core, terminal diff+write {:.3}% core\n", prof.2, prof.4, prof.0.as_secs_f64() / el * 100.0, prof.1.as_secs_f64() / el * 100.0));
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

fn draw(f: &mut Frame, b: &BandSnap, m: &MonSnap, log: &[String], paused: bool, quitting: bool, anim: &mut Anim, ctx: &Ctx) {
    let area = f.area();
    crate::theme::frame(b.style_id, b.beat);
    f.render_widget(Block::default().style(Style::default().bg(crate::theme::bg()).fg(th(Color::Gray))), area);
    // the crowd strip under the band needs 5 more rows (it shares the band's bottom border);
    // on a short terminal the event log gives up to 2 lines for it, below that it hides
    let force = crate::audience::FORCE.load(Ordering::Relaxed);
    let roomy = area.width >= CROWD_MIN_W && area.height >= CROWD_MIN_H;
    let crowd_on = crate::audience::ENABLED.load(Ordering::Relaxed) && match force {
        1 => true,
        2 => false,
        _ => roomy,
    };
    crate::audience::SHOWN.store(crowd_on, Ordering::Relaxed);
    let need = 30 + CROWD_H - 1;
    let mut log_h = 7u16;
    let mut show_crowd = false;
    if crowd_on && area.height >= 5 + 5 + need {
        show_crowd = true;
        log_h = (area.height - 5 - need).min(7);
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(10), Constraint::Length(log_h)])
        .split(area);
    draw_header(f, rows[0], b, m, paused, quitting, anim, ctx);
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(rows[1]);
    anim.crowd = m.crowd.clone();
    if show_crowd {
        let l = mid[0];
        let band = Rect { height: l.height - (CROWD_H - 1), ..l };
        let cr = Rect { y: l.y + band.height - 1, height: CROWD_H, ..l };
        draw_band(f, band, b, anim, ctx);
        draw_crowd(f, cr, b, anim, ctx);
    } else {
        anim.aud = None;
        draw_band(f, mid[0], b, anim, ctx);
    }
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Length(5), Constraint::Min(8), Constraint::Length(6)])
        .split(mid[1]);
    draw_conductor(f, right[0], b, anim, ctx);
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

fn draw_header(f: &mut Frame, r: Rect, b: &BandSnap, m: &MonSnap, paused: bool, quitting: bool, anim: &mut Anim, ctx: &Ctx) {
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
    let beat = beat_dots(ctx.pos.map(|p| p.in_bar() as u32).unwrap_or(b.beat));
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
    // the beat dots follow the heard beat between full refreshes too
    let off: usize = line2[..4].iter().map(|s| s.width()).sum();
    anim.dots = Some((r.x + 1 + off as u16, r.y + 2, Style::default().fg(th(Color::LightYellow))));
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
            .title(heart(b))
            .title(Line::from(inp).right_aligned())
            .title_bottom(Span::styled(crate::theme::asciify(if crate::audience::ENABLED.load(Ordering::Relaxed) { " ↑↓ tempo  ←→ style  n next  l like  a crowd  space pause  q quit " } else { " ↑↓ tempo  ←→ style  n next  l like  space pause  q quit " }), dim)),
    );
    f.render_widget(p, r);
}

/// write one run of cells straight to the terminal: `text` over the cached frame's cells
/// (keeping their background), or the cached cells themselves when `text` is None
fn emit(out: &mut impl Write, cache: &ratatui::buffer::Buffer, x: u16, y: u16, text: Option<(&String, Style)>, w: usize) {
    use crossterm::style::{Attribute, Color as CC, Print, SetAttribute, SetBackgroundColor, SetForegroundColor};
    let a = cache.area;
    if y >= a.y + a.height {
        return;
    }
    let _ = crossterm::queue!(out, cursor::MoveTo(x, y));
    let mut chars = text.map(|t| t.0.chars());
    // only send attribute/colour changes (most runs are one colour on one background)
    let mut cur: Option<(Color, Color, bool)> = None;
    for k in 0..w as u16 {
        if x + k >= a.x + a.width {
            break;
        }
        let base = &cache[(x + k, y)];
        let (sym, fg, bold): (String, Color, bool) = match (&mut chars, text) {
            (Some(it), Some((_, st))) => (it.next().unwrap_or(' ').to_string(), st.fg.unwrap_or(base.fg), st.add_modifier.contains(Modifier::BOLD)),
            _ => (base.symbol().to_string(), base.fg, base.modifier.contains(Modifier::BOLD)),
        };
        let st = (fg, base.bg, bold);
        if cur != Some(st) {
            let _ = crossterm::queue!(
                out,
                SetAttribute(if bold { Attribute::Bold } else { Attribute::NormalIntensity }),
                SetForegroundColor(CC::from(fg)),
                SetBackgroundColor(CC::from(base.bg))
            );
            cur = Some(st);
        }
        let _ = crossterm::queue!(out, Print(sym));
    }
}

fn beat_dots(beat: u32) -> String {
    (0..4).map(|i| if i == beat % 4 { '●' } else { '○' }).collect()
}

/// ♥ flashes for a couple of seconds after a like
fn heart(b: &BandSnap) -> Line<'static> {
    if b.liked >= 0.0 && b.liked < 2.5 {
        let on = ((b.liked * 4.0) as i32) % 2 == 0;
        let h = if crate::theme::ascii() { "<3" } else { "♥" };
        let st = if on { Style::default().fg(th(Color::Black)).bg(th(Color::LightRed)).add_modifier(Modifier::BOLD) } else { Style::default().fg(th(Color::LightRed)).add_modifier(Modifier::BOLD) };
        Line::from(Span::styled(format!(" {h} liked ({}) ", b.likes_n), st))
    } else if b.likes_n > 0 {
        let h = if crate::theme::ascii() { "<3" } else { "♥" };
        Line::from(Span::styled(format!(" {h} {} ", b.likes_n), Style::default().fg(th(Color::DarkGray))))
    } else {
        Line::from("")
    }
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

/// per-frame animation context: where the music is, as heard right now
pub struct Ctx {
    pub heard: Option<f64>,
    pub pos: Option<crate::clock::Pos>,
    pub t: f32,
}

/// UI-side memory for the little people (instrument swaps, the guest's walk, hush timing)
pub struct Anim {
    t0: Instant,
    fam: [Option<crate::figure::Fam>; 7],
    swap: [Option<(crate::figure::Fam, Instant)>; 7],
    present: [bool; 7],
    walk: [Option<(Instant, bool)>; 7],
    hush: [Option<Instant>; 7],
    col: [Color; 7],
    cond: Option<(Rect, Color)>,
    band: Option<(Rect, usize, bool)>,
    dots: Option<(u16, u16, Style)>,
    /// the crowd: who is on the floor (from the monitor), their UI-side walks, the floor rect
    crowd: crate::audience::CrowdSnap,
    floor: crate::audience::Floor,
    aud: Option<(Rect, usize)>,
    crowd_rows: Vec<Vec<(char, Color)>>,
    crowd_t: Instant,
    crowd_beat: i64,
}

impl Anim {
    pub fn new() -> Anim {
        Anim { t0: Instant::now(), fam: [None; 7], swap: [None; 7], present: [false; 7], walk: [None; 7], hush: [None; 7], col: [Color::Gray; 7], cond: None, band: None, dots: None, crowd: Default::default(), floor: crate::audience::Floor::new(), aud: None, crowd_rows: Vec::new(), crowd_t: Instant::now(), crowd_beat: 0 }
    }
}

fn draw_conductor(f: &mut Frame, r: Rect, b: &BandSnap, anim: &mut Anim, ctx: &Ctx) {
    use crate::conductor::Pose;
    let c = &b.cond;
    let mood = crate::mood::ALL[b.mood.min(crate::mood::N - 1)];
    let sn = b.style.rsplit(" · ").next().unwrap_or("");
    let title = format!(" conductor · {sn} · {} · {} cue{} ", mood.name(), c.count, if c.count == 1 { "" } else { "s" });
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
    let art = crate::figure::conductor(b.style_id, c.pose, mood, b.mood_e.max(0.4), ctx.pos, ctx.t);
    anim.cond = Some((inner, col));
    let w = inner.width as usize;
    let tw = w.saturating_sub(crate::figure::CW + 1).max(8);
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
        let mut sp = vec![Span::styled(format!("{} ", crate::theme::asciify(a)), Style::default().fg(col).add_modifier(Modifier::BOLD))];
        let row = k.checked_sub(1).and_then(|j| said.get(j));
        if let Some(t) = row {
            sp.push(Span::styled(t.clone(), Style::default().fg(th(Color::White))));
        } else if k == 4 && said.len() < 4 {
            let tgt = c.target.map(|t| format!("→ {t}  ")).unwrap_or_default();
            sp.push(Span::styled(format!("{tgt}clutter {}", meter(c.clutter.min(1.5) / 1.5, 8)), Style::default().fg(th(Color::DarkGray))));
        }
        lines.push(Line::from(sp));
    }
    // what the little brain thinks
    if !b.brain.is_empty() {
        let br = if crate::theme::ascii() { format!("brain: {}", b.brain) } else { format!("🧠 {}", b.brain) };
        lines.push(Line::from(Span::styled(br, Style::default().fg(th(Color::LightMagenta)))));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_band(f: &mut Frame, r: Rect, b: &BandSnap, anim: &mut Anim, ctx: &Ctx) {
    use crate::figure::PW;
    let block = Block::default().borders(Borders::ALL).border_set(crate::theme::border_set()).border_style(Style::default().fg(crate::theme::border_color())).title(crate::theme::title(" the band "));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let n = b.minds.len().max(1);
    let per = (inner.height as usize / n).clamp(1, 4);
    let dim = Style::default().fg(th(Color::DarkGray));
    let sprites = per >= 3 && inner.width as usize >= 60;
    let sw = if sprites { PW + 1 } else { 0 };
    let w = (inner.width as usize).saturating_sub(sw);
    let mut lines = Vec::new();
    for (pi, mi) in b.minds.iter().enumerate() {
        let st = status_color(mi.status);
        let mut block_lines: Vec<Vec<Span>> = Vec::new();
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
        block_lines.push(l1);
        if per >= 2 {
            let t: String = format!("  ↳ {}", mi.think).chars().take(w).collect();
            let ts = if mi.addressed { Style::default().fg(th(Color::LightYellow)).add_modifier(Modifier::ITALIC) } else { Style::default().fg(th(Color::Gray)).add_modifier(Modifier::ITALIC) };
            block_lines.push(vec![Span::styled(t, ts)]);
        }
        if per >= 3 {
            let notes = if mi.notes.is_empty() { "·".to_string() } else { mi.notes.join(" ") };
            let left: String = format!("  ♪ {notes}").chars().take(w).collect();
            block_lines.push(vec![Span::styled(left, dim)]);
        }
        if per >= 4 {
            let right = match (mi.tied.is_empty(), mi.traits.is_empty()) {
                (true, true) => String::new(),
                (false, true) => format!("  ⇢ {}", mi.tied),
                (true, false) => format!("  {}", mi.traits),
                _ => format!("  ⇢ {} · {}", mi.tied, mi.traits),
            };
            block_lines.push(vec![Span::styled(right.chars().take(w).collect::<String>(), Style::default().fg(th(Color::Cyan)).add_modifier(Modifier::DIM))]);
        }
        // the little character on the left
        let sprite: Vec<String> = if sprites { sprite_for(pi, mi, b, anim, ctx, per) } else { Vec::new() };
        let scol = if mi.playing { st } else { th(Color::DarkGray) };
        anim.col[pi] = scol;
        for k in 0..per {
            let mut sp = Vec::new();
            if sprites {
                let s = sprite.get(k).cloned().unwrap_or_else(|| " ".repeat(PW));
                sp.push(Span::styled(format!("{} ", crate::theme::asciify(&s)), Style::default().fg(scol)));
            }
            if let Some(l) = block_lines.get(k) {
                sp.extend(l.iter().cloned());
            }
            lines.push(Line::from(sp));
        }
    }
    anim.band = Some((inner, per, sprites));
    f.render_widget(Paragraph::new(lines), inner);
}

/// one musician's frame (also used for the in-between animation frames)
fn sprite_for(pi: usize, mi: &Mind, b: &BandSnap, anim: &mut Anim, ctx: &Ctx, per: usize) -> Vec<String> {
    use crate::figure::{player, Fam, PState, PlayerIn, PW};
    let now = Instant::now();
    let rate = crate::clock::rate().max(1.0);
    let cond_mood = crate::mood::ALL[b.mood.min(crate::mood::N - 1)];
    // remember instrument swaps, the guest's comings and goings, hush onsets
    if anim.fam[pi].is_some() && anim.fam[pi] != Some(mi.fam) && mi.playing {
        anim.swap[pi] = Some((anim.fam[pi].unwrap(), now));
    }
    anim.fam[pi] = Some(mi.fam);
    if mi.present != anim.present[pi] {
        anim.walk[pi] = Some((now, mi.present));
        anim.present[pi] = mi.present;
    }
    if mi.hushed < 0.99 {
        anim.hush[pi].get_or_insert(now);
    } else {
        anim.hush[pi] = None;
    }
    let walk = match anim.walk[pi] {
        Some((t, entering)) => {
            let k = (now.duration_since(t).as_secs_f32() / 1.5).min(1.0);
            if entering { 1.0 - k } else { k }
        }
        None => if mi.present { 0.0 } else { 1.0 },
    };
    let swap = anim.swap[pi].and_then(|(old, t)| {
        let s = now.duration_since(t).as_secs_f32();
        (s < 1.0).then_some((old, s))
    });
    let beats = ctx.pos.map(|p| p.beats()).unwrap_or(ctx.t as f64 * 1.5);
    let hit = ctx.heard.and_then(|h| crate::clock::last_hit(pi, h)).map(|(age, key, vel, n)| ((age / rate) as f32, key, vel, n));
    let busy = mi.playing || hit.map_or(false, |h| h.0 < 2.0);
    let state = if !busy && mi.present {
        PState::Rest(((pi as u64 + b.bar / 16) % 3) as u8)
    } else if let Some(t) = anim.hush[pi] {
        PState::Hushed(now.duration_since(t).as_secs_f32())
    } else {
        PState::Play
    };
    if walk >= 1.0 {
        vec![" ".repeat(PW); per.min(4)]
    } else {
        let pin = PlayerIn { style: b.style_id, fam: if pi == 6 { Fam::Guest } else { mi.fam }, state, hit, beats, swap, walk, quoting: mi.quoting, cond_mood, t: ctx.t, surge: b.feel.starts_with("surge") };
        let mut v = player(&pin, per.min(4));
        v.truncate(per);
        v
    }
}

/// in-between frames: only the conductor and the musicians move, on top of the last full
/// frame. Returns the cells to write; an unchanged frame is skipped altogether.
fn anim_cells(b: &BandSnap, anim: &mut Anim, ctx: &Ctx) -> Vec<(u16, u16, String, usize, Style)> {
    use crate::figure::PW;
    let mut out = Vec::new();
    if let (Some((x, y, st)), Some(p)) = (anim.dots, ctx.pos) {
        out.push((x, y, crate::theme::asciify(&beat_dots(p.in_bar() as u32)), 4, st));
    }
    if let Some((r, col)) = anim.cond {
        let c = &b.cond;
        let mood = crate::mood::ALL[b.mood.min(crate::mood::N - 1)];
        let art = crate::figure::conductor(b.style_id, c.pose, mood, b.mood_e.max(0.4), ctx.pos, ctx.t);
        let st = Style::default().fg(col).add_modifier(Modifier::BOLD);
        for (k, a) in art.iter().enumerate() {
            if (k as u16) < r.height {
                out.push((r.x, r.y + k as u16, crate::theme::asciify(a), crate::figure::CW.min(r.width as usize), st));
            }
        }
    }
    if let Some((r, per, true)) = anim.band {
        for (pi, mi) in b.minds.iter().enumerate() {
            let y0 = r.y + (pi * per) as u16;
            let sp = sprite_for(pi, mi, b, anim, ctx, per);
            let st = Style::default().fg(anim.col[pi]);
            for k in 0..per {
                let y = y0 + k as u16;
                if y >= r.y + r.height {
                    break;
                }
                let line = sp.get(k).cloned().unwrap_or_else(|| " ".repeat(PW));
                out.push((r.x, y, crate::theme::asciify(&line), PW, st));
            }
        }
    }
    // the crowd moves at ~8 fps (and on every beat); its cells go last so a change in the
    // number of colour runs doesn't shift the musicians' cells
    if let Some((r, fw)) = anim.aud {
        let beat = ctx.pos.map(|p| p.beats().floor() as i64).unwrap_or(0);
        if anim.crowd_rows.is_empty() || anim.crowd_t.elapsed() >= Duration::from_millis(120) || beat != anim.crowd_beat || anim.crowd_rows[0].len() != fw {
            anim.crowd_rows = crowd_grid(b, anim, ctx, fw);
            anim.crowd_t = Instant::now();
            anim.crowd_beat = beat;
        }
        for (k, row) in anim.crowd_rows.iter().enumerate() {
            let mut x = r.x;
            for sp in grid_spans(row) {
                let n = sp.content.chars().count();
                out.push((x, r.y + k as u16, sp.content.to_string(), n, sp.style));
                x += n as u16;
            }
        }
    }
    out
}

/// crowd panel rows incl. borders (headroom + 5 figure rows + name tags)
const CROWD_H: u16 = 9;
/// the crowd only shows on a terminal big enough to draw it well (`a` overrides)
const CROWD_MIN_W: u16 = 160;
const CROWD_MIN_H: u16 = 55;

/// how the crowd feels: applause after a like, cheering when the band is on fire
fn vibe(b: &BandSnap) -> crate::audience::Vibe {
    use crate::audience::Vibe;
    let delighted = crate::mood::ALL[b.mood.min(crate::mood::N - 1)] == crate::mood::Mood::Delighted;
    if b.liked >= 0.0 && b.liked < 3.5 {
        Vibe::Applause
    } else if b.skipped >= 0.0 && b.skipped < 1.6 {
        Vibe::Hype // "next idea": a quick little cheer
    } else if b.cond.pose == crate::conductor::Pose::Surge || b.feel.starts_with("surge") || b.crowd_hype > 0.6 || (b.state == "alive" && (b.energy > 0.55 || delighted)) {
        Vibe::Hype
    } else if b.state == "chilling" {
        Vibe::Chill
    } else {
        Vibe::Normal
    }
}

/// the floor as rows of (char, colour), `fw` cells wide (the bar is drawn separately)
fn crowd_grid(b: &BandSnap, anim: &mut Anim, ctx: &Ctx, fw: usize) -> Vec<Vec<(char, Color)>> {
    use crate::audience::{fig, tag, Act, Look, Vibe, FH, FW, SLOT};
    let v = vibe(b);
    anim.floor.step(&anim.crowd, fw, v);
    // one row of headroom (jumps), the figures, the name tags
    let tag_row = FH + 1;
    let mut g = vec![vec![(' ', Color::Reset); fw]; FH + 2];
    let surge = b.feel.starts_with("surge");
    let beats = ctx.pos.map(|p| p.beats()).unwrap_or(ctx.t as f64 * 1.5);
    let t = anim.floor.t;
    let odd = beats.floor() as i64 % 2 == 1;
    // people at home first, walkers on top
    let mut tags: Vec<(bool, i32, String, Color)> = Vec::new();
    let mut order: Vec<usize> = (0..anim.floor.people.len()).collect();
    order.sort_by_key(|&i| (anim.floor.people[i].act != Act::Here) as u8);
    for i in order {
        let p = &anim.floor.people[i];
        let m = &p.m;
        let look = Look { style: b.style_id, act: p.act, vibe: v, beats, t: t + (p.seed % 1000) as f32 * 0.013, cpu: m.cpu, disk: m.disk > 2e5, chat: m.chat > 2e4, idle: m.idle, step: p.step, beer: p.beer, id: p.seed, surge };
        let (rows, lift, dx) = fig(&look);
        let x0 = p.x.round() as i32 + if p.act == Act::Here { dx - 1 } else { 0 };
        let _ = odd;
        let col = if matches!(p.act, Act::Wave | Act::Leave) {
            th(Color::Gray)
        } else if p.act == Act::Bar || p.act == Act::ToBar {
            th(Color::LightYellow)
        } else if m.cpu > 0.5 {
            th(Color::LightRed)
        } else if m.disk > 2e5 {
            th(Color::LightBlue)
        } else if m.chat > 2e4 {
            th(Color::LightCyan)
        } else if m.cpu > 0.08 {
            th(Color::LightMagenta)
        } else if v == Vibe::Hype || v == Vibe::Applause {
            th(Color::White)
        } else {
            th(Color::Gray)
        };
        for (r, row) in rows.iter().enumerate() {
            let y = r as i32 + 1 - lift as i32;
            if y < 0 {
                continue;
            }
            // opaque between the outermost glyphs, so someone walking past hides who's behind
            let lo = row.chars().position(|c| c != ' ').unwrap_or(usize::MAX);
            let hi = row.chars().count() - row.chars().rev().position(|c| c != ' ').unwrap_or(0);
            for (k, ch) in row.chars().enumerate() {
                let x = x0 + k as i32;
                if (ch != ' ' || (k > lo && k < hi)) && x >= 0 && (x as usize) < fw {
                    g[y as usize][x as usize] = (ch, col);
                }
            }
        }
        // name tag (now and then the name of a file it has open)
        let show_file = m.file.is_some() && ((t / 4.0) as u32 + p.seed % 7) % 3 == 0;
        let label = if show_file { m.file.clone().unwrap().chars().take(SLOT - 1).collect::<String>() } else { tag(&m.name, SLOT - 1) };
        let tcol = if show_file { th(Color::Cyan) } else if m.fresh && m.cpu > 0.02 { th(Color::LightYellow) } else { th(Color::DarkGray) };
        let lx = x0 + FW as i32 / 2 - label.chars().count() as i32 / 2;
        tags.push((p.act == Act::Here || p.act == Act::Bar, lx, label, tcol));
    }
    // name tags: the ones standing still first; walkers' tags only where they fit
    tags.sort_by_key(|t| !t.0);
    let mut taken = vec![false; fw];
    for (still, lx, label, tcol) in tags {
        let n = label.chars().count() as i32;
        let cells: Vec<usize> = (lx..lx + n).filter(|&x| x >= 0 && (x as usize) < fw).map(|x| x as usize).collect();
        let clash = cells.iter().any(|&x| taken[x] || (x > 0 && taken[x - 1]) || (x + 1 < fw && taken[x + 1]));
        if !still && (clash || cells.len() < n as usize) {
            continue;
        }
        if still && cells.iter().any(|&x| taken[x]) {
            continue;
        }
        for (k, ch) in label.chars().enumerate() {
            let x = lx + k as i32;
            if x >= 0 && (x as usize) < fw {
                g[tag_row][x as usize] = (ch, tcol);
                taken[x as usize] = true;
            }
        }
    }
    g
}

fn grid_spans(row: &[(char, Color)]) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cc = None;
    for &(ch, c) in row {
        if Some(c) != cc && !cur.is_empty() {
            out.push(Span::styled(std::mem::take(&mut cur), Style::default().fg(cc.unwrap()).add_modifier(Modifier::BOLD)));
        }
        cc = Some(c);
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(Span::styled(cur, Style::default().fg(cc.unwrap()).add_modifier(Modifier::BOLD)));
    }
    out
}

fn draw_crowd(f: &mut Frame, r: Rect, b: &BandSnap, anim: &mut Anim, ctx: &Ctx) {
    use crate::audience::BARW;
    let c = &anim.crowd;
    let here = anim.floor.people.iter().filter(|p| !matches!(p.act, crate::audience::Act::Leave | crate::audience::Act::Wave)).count();
    let title = if c.on {
        let full = format!(" the crowd · {here} of your {} processes · {} at the bar · {} kernel threads backstage ", c.own, anim.floor.at_bar(), c.kthreads);
        let short = format!(" the crowd · {here} of your {} · {} at the bar · {} kthreads ", c.own, anim.floor.at_bar(), c.kthreads);
        if full.chars().count() + 6 <= r.width as usize { full } else { short }
    } else {
        " the crowd · doors opening… ".to_string()
    };
    let bs = crate::theme::border_set();
    let block = Block::default().borders(Borders::ALL).border_set(bs).border_style(Style::default().fg(crate::theme::border_color())).title(crate::theme::title(&title));
    let inner = block.inner(r);
    f.render_widget(block, r);
    // join the band panel above: ├ ┤ instead of corners
    let (l, rr) = match bs.vertical_left {
        "│" => ("├", "┤"),
        "║" => ("╠", "╣"),
        "┃" => ("┣", "┫"),
        _ => (bs.top_left, bs.top_right),
    };
    let buf = f.buffer_mut();
    buf[(r.x, r.y)].set_symbol(l);
    buf[(r.x + r.width - 1, r.y)].set_symbol(rr);
    if (inner.width as usize) < BARW + 12 || inner.height < 4 {
        anim.aud = None;
        return;
    }
    let fw = inner.width as usize - BARW;
    anim.aud = Some((inner, fw));
    let g = crowd_grid(b, anim, ctx, fw);
    anim.crowd_rows = g.clone();
    anim.crowd_t = Instant::now();
    let lines: Vec<Line> = g.iter().map(|row| Line::from(grid_spans(row))).collect();
    f.render_widget(Paragraph::new(lines), Rect { width: fw as u16, ..inner });
    let bar = crate::audience::bar_rows(crate::theme::ascii());
    let bl: Vec<Line> = bar.iter().enumerate().map(|(i, s)| Line::from(Span::styled(s.clone(), Style::default().fg(if i == 1 { th(Color::LightYellow) } else { th(Color::Yellow) })))).collect();
    f.render_widget(Paragraph::new(bl), Rect { x: inner.x + fw as u16 + 1, width: BARW as u16 - 1, ..inner });
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
