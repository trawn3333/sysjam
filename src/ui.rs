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
    /// the GM program behind the instrument (stage props), or a synth patch
    pub prog: Option<u8>,
    pub synth: bool,
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
    /// biker: the frontman's last yell and how long ago (s)
    pub front: Option<(&'static str, f32)>,
    /// the machine's section name (Industrial / Biker), "" otherwise
    pub section: &'static str,
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
    let mut prof_full = Duration::ZERO;
    let mut last_cells: Vec<(u16, u16, String, usize, Style)> = Vec::new();
    // stage/crowd runs written since the last full frame
    let mut touched: Vec<(u16, u16, String, usize, Style)> = Vec::new();
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
        // the full UI refreshes at 2 fps (or at once after a key); in between only the
        // little people are redrawn over the cached frame
        let full = force_full || last_full.elapsed() >= Duration::from_millis(500) || cache.is_none();
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
            let log: Vec<String> = LOG.lock().map(|l| l.iter().rev().take(40).map(|x| x.1.clone()).collect()).unwrap_or_default();
            let _ = term.draw(|f| {
                let t = Instant::now();
                draw(f, &band, &mon, &log, paused, quitting, &mut anim, &ctx);
                match cache.as_mut() {
                    Some(c) => c.clone_from(f.buffer_mut()),
                    None => cache = Some(f.buffer_mut().clone()),
                }
                tw = t.elapsed();
                prof_full += tw;
            });
            // ratatui only rewrote what changed against its own idea of the screen, which
            // doesn't know about the in-between frames: put back the new frame wherever the
            // last in-between frame left something different on screen
            if dirty {
                if let Some(c) = cache.as_ref() {
                    let mut out = std::io::stdout().lock();
                    for cell in last_cells.iter().chain(touched.iter()) {
                        if !run_matches(c, cell.0, cell.1, &cell.2, cell.4, cell.3) {
                            emit(&mut out, c, cell.0, cell.1, None, cell.3);
                        }
                    }
                    let _ = crossterm::queue!(out, crossterm::style::SetAttribute(crossterm::style::Attribute::Reset), crossterm::style::ResetColor);
                    let _ = out.flush();
                }
                dirty = false;
            }
            last_full = Instant::now();
            force_full = false;
            last_cells.clear();
            touched.clear();
        } else {
            let t = Instant::now();
            let (cells, delta) = anim_cells(&band, &mut anim, &ctx);
            let td = Instant::now();
            // compare by position: a run that splits or merges doesn't resend the rest
            let prev: std::collections::HashMap<(u16, u16), (&String, Style, usize)> = last_cells.iter().map(|c| ((c.0, c.1), (&c.2, c.4, c.3))).collect();
            let fresh: Vec<bool> = cells.iter().map(|c| prev.get(&(c.0, c.1)).map_or(true, |p| *p.0 != c.2 || p.1 != c.4 || p.2 != c.3)).collect();
            let same = !fresh.iter().any(|&f| f) && cells.len() == last_cells.len() && delta.is_empty();
            PROF_DIFF.fetch_add(td.elapsed().as_nanos() as u64, Ordering::Relaxed);
            tw = t.elapsed();
            if !same {
                // straight to the terminal, only the lines that changed; ratatui's own idea of
                // the screen (the cached full frame) is restored before its next full draw
                if let Some(c) = cache.as_ref() {
                    let mut out = std::io::stdout().lock();
                    for (i, cell) in cells.iter().enumerate() {
                        if fresh[i] {
                            PROF_EMIT.fetch_add(cell.3 as u64, Ordering::Relaxed);
                            emit(&mut out, c, cell.0, cell.1, Some((&cell.2, cell.4)), cell.3);
                        }
                    }
                    for cell in &delta {
                        PROF_EMIT.fetch_add(cell.3 as u64, Ordering::Relaxed);
                        emit(&mut out, c, cell.0, cell.1, Some((&cell.2, cell.4)), cell.3);
                    }
                    let _ = crossterm::queue!(out, crossterm::style::SetAttribute(crossterm::style::Attribute::Reset), crossterm::style::ResetColor);
                    let _ = out.flush();
                    dirty = true;
                }
                last_cells = cells;
                touched.extend(delta);
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
        let _ = std::fs::write(p, format!("frames {} ({} unchanged, skipped) in {el:.1}s; widgets {:.3}% core (full frames {:.3}%, stage {:.3}%, crowd {:.3}%, diff {:.3}%, emitted cells/s {:.0}), terminal diff+write {:.3}% core\n", prof.2, prof.4, prof.0.as_secs_f64() / el * 100.0, prof_full.as_secs_f64() / el * 100.0, PROF_STAGE.load(Ordering::Relaxed) as f64 / 1e9 / el * 100.0, PROF_CROWD.load(Ordering::Relaxed) as f64 / 1e9 / el * 100.0, PROF_DIFF.load(Ordering::Relaxed) as f64 / 1e9 / el * 100.0, PROF_EMIT.load(Ordering::Relaxed) as f64 / el, prof.1.as_secs_f64() / el * 100.0));
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
    // Industrial wall of sound: the borders glitch now and then (flicker to amber / red)
    let g = if b.style_id == "industrial" && b.feel.starts_with("RAGE") {
        let k = (ctx.t * 9.0) as u32;
        let h = k.wrapping_mul(2654435761) >> 28;
        if h < 3 { 1 + (h % 2) as u8 } else { 0 }
    } else {
        0
    };
    crate::theme::set_glitch(g);
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
    let mut k: u16 = 0;
    while (k as usize) < w {
        if x + k >= a.x + a.width {
            break;
        }
        let base = &cache[(x + k, y)];
        let (sym, fg, bold): (String, Color, bool) = match (&mut chars, text) {
            (Some(it), Some((_, st))) => (it.next().unwrap_or(' ').to_string(), st.fg.unwrap_or(base.fg), st.add_modifier.contains(Modifier::BOLD)),
            _ => (base.symbol().to_string(), base.fg, base.modifier.contains(Modifier::BOLD)),
        };
        let bg = text.and_then(|t| t.1.bg).unwrap_or(base.bg);
        let st = (fg, bg, bold);
        if cur != Some(st) {
            let _ = crossterm::queue!(
                out,
                SetAttribute(if bold { Attribute::Bold } else { Attribute::NormalIntensity }),
                SetForegroundColor(CC::from(fg)),
                SetBackgroundColor(CC::from(bg))
            );
            cur = Some(st);
        }
        // a wide glyph (an emoji held up in the crowd) covers the next cell too
        let wide = unicode_width::UnicodeWidthStr::width(sym.as_str()) == 2;
        let _ = crossterm::queue!(out, Print(sym));
        k += if wide { 2 } else { 1 };
    }
}

/// does the cached frame already show this run (same glyphs, colours and weight)?
fn run_matches(cache: &ratatui::buffer::Buffer, x: u16, y: u16, text: &str, st: Style, w: usize) -> bool {
    let a = cache.area;
    if y >= a.y + a.height {
        return true;
    }
    let mut chars = text.chars();
    let mut k: u16 = 0;
    let bold = st.add_modifier.contains(Modifier::BOLD);
    let mut buf = [0u8; 4];
    while (k as usize) < w && x + k < a.x + a.width {
        let base = &cache[(x + k, y)];
        let ch = chars.next().unwrap_or(' ');
        let sym: &str = ch.encode_utf8(&mut buf);
        if base.symbol() != sym || st.fg.unwrap_or(base.fg) != base.fg || st.bg.unwrap_or(base.bg) != base.bg || base.modifier.contains(Modifier::BOLD) != bold {
            return false;
        }
        k += if unicode_width::UnicodeWidthStr::width(sym) == 2 { 2 } else { 1 };
    }
    true
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
    crowd_rows: Vec<Vec<Cell>>,
    crowd_t: Instant,
    crowd_beat: i64,
    stage_rows: Vec<Vec<Cell>>,
    stage_t: Instant,
    stage_beat: i64,
    /// the stage: each player's prop, swaps in progress, the stage rect
    prop: [Option<crate::stage::Prop>; 7],
    pswap: [Option<(Option<crate::stage::Prop>, Instant)>; 7],
    stage: Option<Rect>,
    /// a soloist stepping forward (player, since)
    solo: Option<(usize, Instant)>,
}

impl Anim {
    pub fn new() -> Anim {
        Anim { t0: Instant::now(), fam: [None; 7], swap: [None; 7], present: [false; 7], walk: [None; 7], hush: [None; 7], col: [Color::Gray; 7], cond: None, band: None, dots: None, crowd: Default::default(), floor: crate::audience::Floor::new(), aud: None, crowd_rows: Vec::new(), crowd_t: Instant::now(), crowd_beat: 0, stage_rows: Vec::new(), stage_t: Instant::now(), stage_beat: 0, prop: [None; 7], pswap: [None; 7], stage: None, solo: None }
    }
}

fn draw_conductor(f: &mut Frame, r: Rect, b: &BandSnap, anim: &mut Anim, ctx: &Ctx) {
    use crate::conductor::Pose;
    let c = &b.cond;
    let mood = crate::mood::ALL[b.mood.min(crate::mood::N - 1)];
    let sn = b.style.rsplit(" · ").next().unwrap_or("");
    let mname = if b.style_id == "industrial" { crate::mood::rage_name(mood) } else { mood.name() };
    let title = format!(" conductor · {sn} · {} · {} cue{} ", mname, c.count, if c.count == 1 { "" } else { "s" });
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
    // roomy: the band on a stage, with their instruments; small terminals keep the list
    if std::env::var("SYSJAM_STAGE").map_or(true, |v| v != "0") {
        if let Some(g) = stage_grid(b, anim, ctx, inner.width as usize, inner.height as usize) {
            anim.stage = Some(inner);
            anim.band = None;
            let lines: Vec<Line> = g.iter().map(|row| Line::from(grid_spans(row))).collect();
            f.render_widget(Paragraph::new(lines), inner);
            anim.stage_rows = g;
            anim.stage_t = Instant::now();
            anim.stage_beat = ctx.pos.map(|p| p.beats().floor() as i64).unwrap_or(0);
            return;
        }
    }
    anim.stage = None;
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
        let pin = PlayerIn { style: b.style_id, fam: if pi == 6 { Fam::Guest } else { mi.fam }, state, hit, beats, swap, walk, quoting: mi.quoting, cond_mood, t: ctx.t, surge: b.feel.starts_with("surge") || b.feel.starts_with("RAGE") };
        let mut v = player(&pin, per.min(4));
        v.truncate(per);
        v
    }
}

/// in-between frames: only the conductor and the musicians move, on top of the last full
/// frame. Returns the cells to write; an unchanged frame is skipped altogether.
fn anim_cells(b: &BandSnap, anim: &mut Anim, ctx: &Ctx) -> (Vec<(u16, u16, String, usize, Style)>, Vec<(u16, u16, String, usize, Style)>) {
    use crate::figure::PW;
    let mut out = Vec::new();
    let mut delta = Vec::new();
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
    // the stage moves at ~7 fps (the crowd ~8) plus a frame on every beat (the lights
    // and the baton change on the beat)
    if let Some(r) = anim.stage {
        let beat = ctx.pos.map(|p| p.beats().floor() as i64).unwrap_or(0);
        let stale = anim.stage_rows.len() != r.height as usize || anim.stage_rows.first().map_or(true, |x| x.len() != r.width as usize);
        if stale || anim.stage_t.elapsed() >= Duration::from_millis(150) || beat != anim.stage_beat {
            let g = stage_grid(b, anim, ctx, r.width as usize, r.height as usize).unwrap_or_default();
            // only the cells that changed since the grid on screen go out
            grid_delta(&anim.stage_rows, &g, r.x, r.y, crate::theme::ascii(), &mut delta);
            anim.stage_rows = g;
            anim.stage_t = Instant::now();
            anim.stage_beat = beat;
        }
    }
    // the crowd moves at ~8 fps (and on every beat); its cells go last so a change in the
    // number of colour runs doesn't shift the musicians' cells
    if let Some((r, fw)) = anim.aud {
        let beat = ctx.pos.map(|p| p.beats().floor() as i64).unwrap_or(0);
        if anim.crowd_rows.is_empty() || anim.crowd_t.elapsed() >= Duration::from_millis(120) || beat != anim.crowd_beat || anim.crowd_rows[0].len() != fw {
            let g = crowd_grid(b, anim, ctx, fw);
            grid_delta(&anim.crowd_rows, &g, r.x, r.y, false, &mut delta);
            anim.crowd_rows = g;
            anim.crowd_t = Instant::now();
            anim.crowd_beat = beat;
        }
    }
    (out, delta)
}

/// the runs of `new` placed at (x0, y0) that differ from `old` (the grid on screen), cell
/// by cell; everything when the sizes differ
fn grid_delta(old: &[Vec<Cell>], new: &[Vec<Cell>], x0: u16, y0: u16, ascii: bool, out: &mut Vec<(u16, u16, String, usize, Style)>) {
    let same_size = old.len() == new.len() && old.iter().zip(new).all(|(a, b)| a.len() == b.len());
    for (k, row) in new.iter().enumerate() {
        let w = row.len();
        let mut i = 0;
        while i < w {
            if same_size && old[k][i] == row[i] {
                i += 1;
                continue;
            }
            // a changed segment; small unchanged gaps are bridged (fewer cursor moves)
            let mut a = i;
            let mut e = i + 1;
            let mut gap = 0;
            let mut j = i + 1;
            while j < w {
                if !same_size || old[k][j] != row[j] {
                    e = j + 1;
                    gap = 0;
                } else {
                    gap += 1;
                    if gap > 4 {
                        break;
                    }
                }
                j += 1;
            }
            // never split a wide glyph from its covered half
            if a > 0 && (row[a].0 == '\0' || (same_size && old[k][a].0 == '\0')) {
                a -= 1;
            }
            while e < w && (row[e].0 == '\0' || (same_size && old[k][e].0 == '\0')) {
                e += 1;
            }
            let mut x = x0 + a as u16;
            for sp in grid_spans(&row[a..e]) {
                let n = unicode_width::UnicodeWidthStr::width(sp.content.as_ref());
                let txt = if ascii { crate::theme::asciify(&sp.content) } else { sp.content.to_string() };
                out.push((x, y0 + k as u16, txt, n, sp.style));
                x += n as u16;
            }
            i = e;
        }
    }
}

/// the stage scene, `w` x `h` cells, or None when it doesn't fit (list mode then):
/// a lighting truss with beams pulsing on the beat (colour by section, red strobe in
/// Industrial, warm in Jazz/Blues), amps, the players in a row with their instruments,
/// one-line captions, the stage lip, the apron (the conductor from behind, the frontman,
/// wedge monitors) and one condensed "thinking" line per player underneath.
#[allow(non_snake_case)]
static PROF_STAGE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn stage_grid(b: &BandSnap, anim: &mut Anim, ctx: &Ctx, w: usize, h: usize) -> Option<Vec<Vec<Cell>>> {
    let t = Instant::now();
    let r = stage_grid_(b, anim, ctx, w, h);
    PROF_STAGE.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
    r
}
#[allow(non_snake_case)]
fn stage_grid_(b: &BandSnap, anim: &mut Anim, ctx: &Ctx, w: usize, h: usize) -> Option<Vec<Vec<Cell>>> {
    use crate::stage::{self, Prop};
    use crate::figure::PW;
    const APRON: usize = 5;
    if h < 4 + stage::ROWS + 1 + APRON + 3 || w < 64 {
        return None;
    }
    // spare rows go to taller beams (up to 4 more)
    let spare = h.saturating_sub(4 + stage::ROWS + 1 + APRON + 1 + 7);
    let TOP: usize = 4 + spare.min(4); // truss + beam rows
    let CAP: usize = TOP + stage::ROWS; // caption row
    let LIP: usize = CAP + 1;
    let now = Instant::now();
    let beats = ctx.pos.map(|p| p.beats()).unwrap_or(ctx.t as f64 * 1.5);
    let in_bar = ctx.pos.map(|p| p.in_bar().floor() as i64).unwrap_or(0);
    let rate = crate::clock::rate().max(1.0);
    let st = b.style_id;
    let bgc = crate::theme::bg();
    let mut g: Vec<Vec<Cell>> = vec![vec![(' ', Color::Reset, bgc); w]; h];
    let put = |g: &mut Vec<Vec<Cell>>, x: i32, y: usize, s: &str, fg: Color| {
        for (k, ch) in s.chars().enumerate() {
            let xx = x + k as i32;
            if xx >= 0 && (xx as usize) < w && y < h && ch != ' ' {
                g[y][xx as usize].0 = ch;
                g[y][xx as usize].1 = fg;
            }
        }
    };
    // who is on stage, left to right (drums in the middle)
    let order = [3usize, 0, 1, 2, 4, 5, 6];
    struct Slot {
        pi: usize,
        prop: Option<Prop>,
        big: bool,
        w: usize,
    }
    let mut slots: Vec<Slot> = Vec::new();
    for &pi in &order {
        let Some(mi) = b.minds.get(pi) else { continue };
        let walking = anim.walk[pi].map_or(false, |(t, _)| now.duration_since(t).as_secs_f32() < 1.5);
        if !mi.present && !walking {
            continue;
        }
        let mut prop = if pi == 6 { None } else { stage::prop_for(st, pi, mi.prog, mi.synth) };
        // the biker frontman sings: the colour seat holds the mic
        if pi == 5 && b.front.is_some() {
            prop = Some(Prop::Mic);
        }
        if anim.prop[pi] != prop {
            if anim.prop[pi].is_some() && mi.playing {
                anim.pswap[pi] = Some((anim.prop[pi], now));
            }
            anim.prop[pi] = prop;
        }
        let pw = prop.map_or(PW, stage::width);
        slots.push(Slot { pi, prop, big: true, w: pw });
    }
    // fit: amps go first, then handheld props shrink to the small figures, then big ones
    let amp_w = 5;
    let mut amps = true;
    let total = |s: &[Slot]| s.iter().map(|x| x.w).sum::<usize>() + s.len() + 1;
    if total(&slots) + 2 * amp_w > w {
        amps = false;
    }
    // keep the drums big longest, then pianos/organ/harp/tuba..., synth rigs shrink first
    let rank = |p: Prop| -> u8 {
        use Prop::*;
        match p {
            Kit | MetalRig | Taiko | Timpani => 3,
            Grand | Organ | Harp | Tuba | UprightBass | Marimba | Dj | Upright => 2,
            _ => 1,
        }
    };
    while total(&slots) > w {
        let pick = slots.iter().enumerate().filter(|(_, s)| s.big && s.w > PW + 1 && s.prop.is_some()).min_by_key(|(_, s)| (rank(s.prop.unwrap()), std::cmp::Reverse(s.w))).map(|(i, _)| i);
        match pick {
            Some(i) => {
                slots[i].big = false;
                slots[i].w = PW;
            }
            None => return None,
        }
    }
    let avail = w - if amps { 2 * amp_w } else { 0 };
    let used: usize = slots.iter().map(|s| s.w).sum();
    let gap = ((avail - used.min(avail)) / (slots.len() + 1)).max(1);
    // lights: section colour, the beat, the mood
    let sec = b.section.bytes().fold(0u8, |a, c| a.wrapping_mul(31).wrapping_add(c)) % 6;
    let loud = b.feel.starts_with("RAGE") || b.feel.starts_with("build");
    let strobe = st == "industrial" && loud;
    let warm = matches!(st, "jazz" | "blues" | "country" | "baroque" | "monk");
    let beat = beats.floor() as i64;
    let lamp_n = (w / 16).max(2);
    let lamp_dx = w / lamp_n;
    let border = crate::theme::border_color();
    for x in 0..w {
        g[0][x].0 = '─';
        g[0][x].1 = border;
    }
    for li in 0..lamp_n {
        let lx = (lamp_dx / 2 + li * lamp_dx) as i32;
        let on = if strobe {
            ((beats * 4.0).floor() as i64) % 2 == 0
        } else if warm {
            (beat + li as i64) % 4 != 3
        } else {
            in_bar == 0 || (beat + li as i64) % 2 == 0
        };
        let rgb = crate::theme::light_rgb(li, sec, warm, strobe);
        let lc = if on { crate::theme::rgb(rgb, 0.25) } else { crate::theme::light_bg(rgb, 0.45) };
        put(&mut g, lx, 0, "•", lc);
        // the lamp and the beam's edges pulse on the beat (or strobe); the wash inside, a
        // big area of background colour, swells bar by bar so the terminal isn't repainted
        // every beat
        let bar = (beats / 4.0).floor() as i64;
        let wash_on = strobe || (bar + li as i64) % 2 == 0;
        // the beam: edges in the light's colour, the inside a translucent wash
        for row in 1..CAP {
            let k = row as i32;
            let amt = if wash_on { 0.20 - 0.012 * row as f32 } else { 0.05 };
            let wash = crate::theme::light_bg(rgb, amt);
            for x in (lx - k + 1)..(lx + k) {
                if x >= 0 && (x as usize) < w && wash != Color::Reset {
                    g[row][x as usize].2 = wash;
                }
            }
            if row < TOP {
                put(&mut g, lx - k, row, "╱", lc);
                put(&mut g, lx + k, row, "╲", lc);
            }
        }
    }
    if amps {
        let ind = st == "industrial";
        let ac = crate::theme::dim(Color::Gray, 1);
        for (k, r) in stage::amp(ind).iter().enumerate() {
            put(&mut g, 0, TOP + k, r, ac);
            put(&mut g, (w - 4) as i32, TOP + k, r, ac);
        }
    }
    // the soloist steps forward
    let soloist = b.minds.iter().position(|m| m.status == "soloing");
    match (soloist, anim.solo) {
        (Some(p), Some((q, _))) if p == q => {}
        (Some(p), _) => anim.solo = Some((p, now)),
        (None, _) => anim.solo = None,
    }
    let mut x = if amps { amp_w } else { 0 } + gap;
    let mut capx: Vec<(usize, usize, usize)> = Vec::new();
    for s in &slots {
        let mi = &b.minds[s.pi];
        let pi = s.pi;
        let colr = if mi.playing { status_color(mi.status) } else { th(Color::DarkGray) };
        anim.col[pi] = colr;
        let hit = ctx.heard.and_then(|hd| crate::clock::last_hit(pi, hd)).map(|(age, ..)| (age / rate) as f32);
        let hitting = hit.map_or(false, |a| a < 0.16);
        let fwd = matches!(anim.solo, Some((p, _)) if p == pi);
        let y0 = TOP + fwd as usize;
        match (s.prop, s.big) {
            (Some(p), true) => {
                let moving = p.moving() && mi.playing && ((beats * 2.0).floor() as i64) % 2 == 0;
                let frame = (hitting || moving) as usize;
                let face = if !mi.playing {
                    '-'
                } else if mi.hushed < 0.99 {
                    '.'
                } else {
                    match (st, hitting) {
                        ("industrial", _) => '>',
                        ("biker", true) => 'O',
                        (_, true) => '^',
                        _ => 'o',
                    }
                };
                // a swap: the old prop is carried off to the right, the new one wheeled on
                let mut draw_p = Some(p);
                let mut off: i32 = 0;
                if let Some((old, t)) = anim.pswap[pi] {
                    let k = now.duration_since(t).as_secs_f32();
                    if k < 0.5 {
                        draw_p = old;
                        off = (k / 0.5 * s.w as f32) as i32;
                    } else if k < 1.0 {
                        off = -(((1.0 - k) / 0.5) * s.w as f32) as i32;
                    } else {
                        anim.pswap[pi] = None;
                    }
                }
                if let Some(dp) = draw_p {
                    for (r, row) in stage::art(dp, frame).iter().enumerate() {
                        let y = y0 + r;
                        if y >= LIP {
                            continue;
                        }
                        for (k, ch) in row.chars().enumerate() {
                            let kx = k as i32 + off;
                            if ch == ' ' || kx < 0 || kx as usize >= s.w {
                                continue;
                            }
                            let xx = x + kx as usize;
                            if xx < w {
                                g[y][xx].0 = if ch == '@' { face } else { ch };
                                g[y][xx].1 = colr;
                            }
                        }
                    }
                }
            }
            _ => {
                // compact: the little figure from the list view, bottom-aligned
                let sp = sprite_for(pi, mi, b, anim, ctx, 4);
                for (r, row) in sp.iter().enumerate() {
                    put(&mut g, x as i32, y0 + 2 + r, row, colr);
                }
            }
        }
        capx.push((pi, x, s.w));
        x += s.w + gap;
    }
    // captions: name · instrument · key, one line, clipped to the slot (+ gap)
    for (i, &(pi, sx, sw)) in capx.iter().enumerate() {
        let mi = &b.minds[pi];
        let room = sw + gap - 1;
        let word = anim.prop[pi].map_or(mi.fam.name(), |p| p.word());
        let key = mi.key.split_whitespace().next().unwrap_or("");
        let mut c = format!("{}·{}", mi.name, word);
        if c.chars().count() + 1 + key.chars().count() <= room && key.chars().next().map_or(false, |k| k.is_ascii_alphabetic()) {
            c = format!("{c}·{key}");
        }
        let c: String = c.chars().take(room).collect();
        let cx = sx + sw.saturating_sub(c.chars().count()) / 2;
        let (fg, bg) = if mi.addressed { (th(Color::Black), th(Color::LightYellow)) } else if mi.away { (th(Color::LightMagenta), bgc) } else { (th(Color::White), bgc) };
        for (k, ch) in c.chars().enumerate() {
            if cx + k < w {
                g[CAP][cx + k] = (ch, fg, bg);
            }
        }
        let _ = i;
    }
    // the lip, the apron and its wedges
    for x in 0..w {
        g[LIP][x].0 = '─';
        g[LIP][x].1 = border;
        for y in LIP + 1..(LIP + APRON + 1).min(h) {
            g[y][x].2 = crate::theme::floor_bg(1, 6);
        }
    }
    let wedge_c = crate::theme::dim(Color::Gray, 2);
    for &(_, sx, sw) in &capx {
        put(&mut g, (sx + sw / 2) as i32 - 2, LIP + APRON, stage::WEDGE, wedge_c);
    }
    // the conductor at the front, facing the band; up on the beat when cueing
    let pose = b.cond.pose;
    let up = matches!(pose, crate::conductor::Pose::Surge | crate::conductor::Pose::Point) && in_bar % 2 == 0;
    let ccol = match pose {
        crate::conductor::Pose::Surge => th(Color::LightRed),
        crate::conductor::Pose::Rest => th(Color::DarkGray),
        _ => th(Color::LightYellow),
    };
    let cx = (w / 2) as i32 - 4;
    for (k, r) in stage::conductor_back(st, up).iter().enumerate() {
        put(&mut g, cx, LIP + 1 + k, r, ccol);
    }
    // the biker frontman's yell / the conductor's call, in a bubble on the apron
    let yell = match b.front {
        Some((t, age)) if age < 2.5 => Some((t.to_string(), th(Color::LightRed))),
        _ => None,
    };
    if let Some((t, c)) = yell {
        put(&mut g, cx + 12, LIP + 1, &format!("< {t}"), c);
    }
    // condensed thinking: one line per player
    let tl = h.saturating_sub(LIP + APRON + 1);
    let mut y = LIP + APRON + 1;
    let mut shown = 0;
    for &(pi, ..) in &capx {
        if shown >= tl {
            break;
        }
        let mi = &b.minds[pi];
        if mi.think.is_empty() {
            continue;
        }
        let line: String = format!("{:<8} ↳ {}", mi.name, mi.think).chars().take(w).collect();
        let fg = if mi.addressed { th(Color::LightYellow) } else { th(Color::Gray) };
        put(&mut g, 0, y, &line, fg);
        y += 1;
        shown += 1;
    }
    Some(g)
}

/// crowd rows inside the panel: headroom, three depth layers, name tags
const CROWD_ROWS: usize = 11;
/// crowd panel rows incl. borders
const CROWD_H: u16 = CROWD_ROWS as u16 + 2;
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
    } else if b.cond.pose == crate::conductor::Pose::Surge || b.feel.starts_with("surge") || b.feel.starts_with("RAGE") || b.feel.starts_with("CHORUS") || b.feel.starts_with("SOLO") || b.crowd_hype > 0.6 || (b.state == "alive" && (b.energy > 0.55 || delighted)) {
        Vibe::Hype
    } else if b.state == "chilling" {
        Vibe::Chill
    } else {
        Vibe::Normal
    }
}

/// one crowd cell: glyph, colour, background ('\0' = the right half of a wide glyph)
type Cell = (char, Color, Color);

/// the floor as rows of cells, `fw` cells wide (the bar is drawn separately). Three depth
/// layers, drawn back to front (painter's algorithm): small hazy figures at the back, then
/// the middle, then the front row at the stage; a floor gradient and a little haze behind.
static PROF_CROWD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static PROF_DIFF: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static PROF_EMIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn crowd_grid(b: &BandSnap, anim: &mut Anim, ctx: &Ctx, fw: usize) -> Vec<Vec<Cell>> {
    let t = Instant::now();
    let r = crowd_grid_(b, anim, ctx, fw);
    PROF_CROWD.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
    r
}
fn crowd_grid_(b: &BandSnap, anim: &mut Anim, ctx: &Ctx, fw: usize) -> Vec<Vec<Cell>> {
    use crate::audience::{fig, fig_small, item, tag, Act, Look, Vibe, FH, FW, SLOT};
    let v = vibe(b);
    anim.floor.step(&anim.crowd, fw, v);
    let ascii = crate::theme::ascii();
    let emoji = !ascii && std::env::var("SYSJAM_EMOJI").map_or(true, |e| e != "0");
    // rows: headroom, back layer (rows 1-3), middle (2-6), front (5-9), name tags (10)
    let nrows = CROWD_ROWS;
    let tag_row = nrows - 1;
    let bgs: Vec<Color> = (0..nrows).map(|k| crate::theme::floor_bg(k, nrows)).collect();
    let mut g: Vec<Vec<Cell>> = (0..nrows).map(|k| vec![(' ', Color::Reset, bgs[k]); fw]).collect();
    // a light haze in the back rows: sparse dim dots drifting
    if !ascii {
        let hz = crate::theme::dim(Color::DarkGray, 2);
        let t = anim.floor.t;
        for k in 1..3 {
            for x in (0..fw).step_by(23) {
                let xx = (x as f32 + t * (0.6 + k as f32 * 0.2) + k as f32 * 4.0) as usize % fw;
                g[k][xx] = ('·', hz, bgs[k]);
            }
        }
    }
    let surge = b.feel.starts_with("surge") || b.feel.starts_with("RAGE");
    let chorus = b.feel.starts_with("CHORUS");
    let beats = ctx.pos.map(|p| p.beats()).unwrap_or(ctx.t as f64 * 1.5);
    let t = anim.floor.t;
    let mut tags: Vec<(bool, i32, String, Color)> = Vec::new();
    // back to front; walkers on top within a layer
    let mut order: Vec<usize> = (0..anim.floor.people.len()).collect();
    let layer = |z: f32| -> usize { (z + 0.5).floor().clamp(0.0, 2.0) as usize };
    order.sort_by_key(|&i| {
        let p = &anim.floor.people[i];
        (layer(p.z), (p.act != Act::Here) as u8)
    });
    for i in order {
        let p = &anim.floor.people[i];
        let m = &p.m;
        let z = layer(p.z);
        let look = Look { style: b.style_id, act: p.act, vibe: v, beats, t: t + (p.seed % 1000) as f32 * 0.013, cpu: m.cpu, disk: m.disk > 2e5, chat: m.chat > 2e4, idle: m.idle, step: p.step, beer: p.beer, id: p.seed, surge, chorus };
        let base = if matches!(p.act, Act::Wave | Act::Leave) {
            Color::Gray
        } else if p.act == Act::Bar || p.act == Act::ToBar {
            Color::LightYellow
        } else if m.cpu > 0.5 {
            Color::LightRed
        } else if m.disk > 2e5 {
            Color::LightBlue
        } else if m.chat > 2e4 {
            Color::LightCyan
        } else if m.cpu > 0.08 {
            Color::LightMagenta
        } else if v == Vibe::Hype || v == Vibe::Applause {
            Color::White
        } else {
            Color::Gray
        };
        // depth = haze: the back row is dimmer (blended toward the floor), the middle a bit
        let col = crate::theme::dim(base, [2u8, 1, 0][z]);
        let (rows, lift, dx): (Vec<String>, bool, i32) = if z == 0 {
            (fig_small(&look).to_vec(), false, 0)
        } else {
            let (r, l, d) = fig(&look);
            (r.to_vec(), l, d)
        };
        let y0: i32 = [1, 2, 5][z] - lift as i32;
        let x0 = p.x.round() as i32 + if p.act == Act::Here { dx - 1 } else { 0 };
        let top_row = rows.iter().position(|r| r.trim() != "").unwrap_or(0) as i32;
        for (r, row) in rows.iter().enumerate() {
            let y = y0 + r as i32;
            if y < 0 || y as usize >= tag_row {
                continue;
            }
            // opaque between the outermost glyphs: whoever is in front hides who's behind,
            // only the edges of the one behind show around them
            let lo = row.chars().position(|c| c != ' ').unwrap_or(usize::MAX);
            let hi = row.chars().count() - row.chars().rev().position(|c| c != ' ').unwrap_or(0);
            for (k, ch) in row.chars().enumerate() {
                let x = x0 + k as i32;
                if (ch != ' ' || (k > lo && k < hi)) && x >= 0 && (x as usize) < fw {
                    let bg = bgs[y as usize];
                    g[y as usize][x as usize] = (ch, col, bg);
                }
            }
        }
        // something held up overhead, swaying on the beat
        if z > 0 {
            if let Some((it, sway)) = item(&look) {
                let (gl, w) = it.glyph(ascii, emoji);
                let y = y0 + top_row - 1;
                let x = x0 + 4 + sway;
                if y >= 0 && x >= 0 && (x as usize + w) < fw {
                    let ic = match it {
                        crate::audience::Item::Phone => th(Color::LightCyan),
                        crate::audience::Item::Lighter => th(Color::LightYellow),
                        crate::audience::Item::Glow => if (beats.floor() as i64 + p.seed as i64) % 2 == 0 { th(Color::LightGreen) } else { th(Color::LightMagenta) },
                        _ => col,
                    };
                    let yy = y as usize;
                    let mut cs = gl.chars();
                    let first = cs.next().unwrap_or(' ');
                    if w == 2 && gl.chars().count() == 1 {
                        g[yy][x as usize] = (first, ic, bgs[yy]);
                        g[yy][x as usize + 1] = ('\0', ic, bgs[yy]);
                    } else {
                        g[yy][x as usize] = (first, ic, bgs[yy]);
                        if let Some(c2) = cs.next() {
                            g[yy][x as usize + 1] = (c2, ic, bgs[yy]);
                        }
                    }
                }
            }
        }
        if z == 0 {
            continue; // the back row is too far away for name tags
        }
        let show_file = m.file.is_some() && ((t / 4.0) as u32 + p.seed % 7) % 3 == 0;
        let label = if show_file { m.file.clone().unwrap().chars().take(SLOT - 1).collect::<String>() } else { tag(&m.name, SLOT - 1) };
        let tcol = if show_file { th(Color::Cyan) } else if m.fresh && m.cpu > 0.02 { th(Color::LightYellow) } else { crate::theme::dim(Color::DarkGray, if z == 1 { 1 } else { 0 }) };
        let lx = x0 + FW as i32 / 2 - label.chars().count() as i32 / 2;
        tags.push((p.act == Act::Here || p.act == Act::Bar, lx, label, tcol));
    }
    let _ = FH;
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
                g[tag_row][x as usize] = (ch, tcol, bgs[tag_row]);
                taken[x as usize] = true;
            }
        }
    }
    g
}

fn grid_spans(row: &[Cell]) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cc: Option<(Color, Color)> = None;
    let style = |c: (Color, Color)| {
        let mut st = Style::default().fg(c.0).add_modifier(Modifier::BOLD);
        if c.1 != Color::Reset {
            st = st.bg(c.1);
        }
        st
    };
    for &(ch, c, bg) in row {
        if ch == '\0' {
            continue; // covered by the wide glyph before it
        }
        if Some((c, bg)) != cc && !cur.is_empty() {
            out.push(Span::styled(std::mem::take(&mut cur), style(cc.unwrap())));
        }
        cc = Some((c, bg));
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(Span::styled(cur, style(cc.unwrap())));
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
