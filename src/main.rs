/// println! that never panics (e.g. stdout piped into `head` and closed).
macro_rules! say {
    ($($t:tt)*) => {{
        use std::io::Write;
        if crate::ui::TUI_ON.load(std::sync::atomic::Ordering::Relaxed) {
            crate::ui::log(format!($($t)*));
        } else {
            let _ = writeln!(std::io::stdout(), $($t)*);
        }
    }};
}

mod audio;
mod band;
mod dbus;
mod dsp;
mod gm;
mod harmony;
mod input;
mod monitor;
mod music;
mod portal;
mod pwcap;
mod screen;
mod shared;
mod synth;
mod ui;

use shared::{Control, ScreenColor};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

static SIGS: AtomicU32 = AtomicU32::new(0);
extern "C" fn on_sig(_: libc::c_int) {
    if SIGS.fetch_add(1, Ordering::SeqCst) >= 1 {
        unsafe { libc::_exit(130) };
    }
}

const HELP: &str = "sysjam - a jam band of machine musicians playing your Linux system

USAGE: sysjam [OPTIONS]

  --volume V          master volume 0.0-1.0 (default 0.6)
  --sub L             sub drone level 0-2 (default 1.2)
  --scale NAME        starting key flavour: minor (default) or major
                      (minor-pentatonic, dorian, ... map to minor; major-pentatonic, lydian, ... to major)
  --root NOTE         root note or Hz, e.g. A2 (default), C3, 98
  --bpm N             tempo (default 80)
  --chord-bars N      base bars per chord (default 2; chilling plays slower, peaks faster)
  --seed N            reproducible run (instrument picks, motifs, voicings, personalities)
  --soundfont PATH    General MIDI .sf2 to use (default: search system paths, then next to the binary)
  --no-gm             oscillator band only, no SoundFont instruments
  --gm-layers N       simultaneous GM layers 1-4 (default 3; +1 at peaks)
  --clicks L          file open/close click texture 0-1 (default 0.25, 0 = off)
  --no-leads          never let a busy thread sit in as a guest musician
  --verbose           band events, chords, instruments, and metrics every 5 s (with --no-tui)
  --no-tui            plain log output instead of the live terminal UI
                      (the UI is the default when stdout is a terminal; keys: q quit, space pause display)
  --device NAME       ALSA device (default \"default\"; e.g. pipewire, pulse, hw:0)
  --wav FILE          render to a WAV file instead of the sound card (real time)
  --null              run without any audio output (testing)
  --duration S        stop after S seconds (default: forever; 30 with --wav)
  --no-screen         don't sample screen colour
  --no-input          don't watch keyboard/mouse activity
  --screen-method M   auto (default), portal, grim, x11, cmd
  --screen-cmd CMD    shell command that prints a P6 PPM screenshot (implies --screen-method cmd)
  --screen-interval S seconds between screen samples (default 4)
  -h, --help          this help

Ctrl-C fades out and exits (press twice to quit immediately).";

fn die(msg: &str) -> ! {
    eprintln!("sysjam: {msg}\n(see --help)");
    std::process::exit(2)
}

fn enable_ftz() {
    // flush denormals to zero: decaying filter/reverb tails otherwise get very slow
    #[cfg(target_arch = "x86_64")]
    #[allow(deprecated)]
    unsafe {
        use std::arch::x86_64::{_mm_getcsr, _mm_setcsr};
        _mm_setcsr(_mm_getcsr() | 0x8040);
    }
}

fn main() {
    enable_ftz();
    let mut args: Vec<String> = Vec::new();
    for a in std::env::args().skip(1) {
        if let Some((k, v)) = a.split_once('=').filter(|_| a.starts_with("--")) {
            args.push(k.to_string());
            args.push(v.to_string());
        } else {
            args.push(a);
        }
    }
    let mut volume = 0.6f32;
    let mut no_screen = false;
    let mut no_input = false;
    let mut scale = music::find_scale("minor-pentatonic").unwrap();
    let mut root = 110.0f32;
    let mut bpm = 80.0f32;
    let mut clicks = 0.25f32;
    let mut sub = 1.2f32;
    let mut chord_bars = 2u32;
    let mut seed: Option<u64> = None;
    let mut soundfont: Option<String> = None;
    let mut no_gm = false;
    let mut gm_layers = 3usize;
    let mut screen_method = "auto".to_string();
    let mut leads = true;
    let mut verbose = false;
    let mut no_tui = false;
    let mut device = "default".to_string();
    let mut wav: Option<String> = None;
    let mut null = false;
    let mut duration: Option<f32> = None;
    let mut screen_cmd: Option<String> = None;
    let mut screen_iv = 4.0f32;
    let mut i = 0;
    let val = |i: &mut usize| -> String {
        *i += 1;
        args.get(*i).cloned().unwrap_or_else(|| die(&format!("{} needs a value", args[*i - 1])))
    };
    let num = |s: String, what: &str| -> f32 { s.parse().unwrap_or_else(|_| die(&format!("bad number for {what}: {s}"))) };
    while i < args.len() {
        match args[i].as_str() {
            "--volume" => volume = num(val(&mut i), "--volume").clamp(0.0, 1.0),
            "--no-screen" => no_screen = true,
            "--no-input" => no_input = true,
            "--scale" => {
                let s = val(&mut i);
                scale = music::find_scale(&s).unwrap_or_else(|| die(&format!("unknown scale {s}")))
            }
            "--root" => {
                let s = val(&mut i);
                root = music::parse_root(&s).unwrap_or_else(|| die(&format!("bad root {s}")))
            }
            "--bpm" => bpm = num(val(&mut i), "--bpm").clamp(30.0, 200.0),
            "--clicks" => clicks = num(val(&mut i), "--clicks").clamp(0.0, 1.0),
            "--verbose" | "-v" => verbose = true,
            "--no-tui" => no_tui = true,
            "--no-leads" => leads = false,
            "--sub" => sub = num(val(&mut i), "--sub").clamp(0.0, 2.0),
            "--chord-bars" => chord_bars = num(val(&mut i), "--chord-bars").clamp(1.0, 16.0) as u32,
            "--seed" => {
                let v = val(&mut i);
                seed = Some(v.parse::<u64>().unwrap_or_else(|_| music::fnv1a(&v)))
            }
            "--soundfont" => soundfont = Some(val(&mut i)),
            "--no-gm" => no_gm = true,
            "--gm-layers" => gm_layers = num(val(&mut i), "--gm-layers").clamp(1.0, 4.0) as usize,
            "--screen-method" => {
                let v = val(&mut i);
                if !["auto", "portal", "grim", "x11", "cmd"].contains(&v.as_str()) {
                    die(&format!("--screen-method must be auto, portal, grim, x11 or cmd (got {v})"));
                }
                screen_method = v
            }
            "--device" => device = val(&mut i),
            "--wav" => wav = Some(val(&mut i)),
            "--null" => null = true,
            "--duration" => duration = Some(num(val(&mut i), "--duration").max(1.0)),
            "--screen-cmd" => {
                screen_cmd = Some(val(&mut i));
                if screen_method == "auto" {
                    screen_method = "cmd".into();
                }
            }
            "--screen-interval" => screen_iv = num(val(&mut i), "--screen-interval").clamp(1.0, 60.0),
            "-h" | "--help" => {
                say!("{HELP}");
                return;
            }
            o => die(&format!("unknown option {o}")),
        }
        i += 1;
    }
    if wav.is_some() && duration.is_none() {
        duration = Some(30.0);
    }

    unsafe {
        libc::signal(libc::SIGINT, on_sig as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_sig as libc::sighandler_t);
    }

    const RATE: u32 = 44100;
    const BLOCK: usize = 2048;
    let seed = seed.unwrap_or_else(|| {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        (t ^ ((std::process::id() as u64) << 32)) % 1_000_000_000
    });
    // snap the root to the nearest equal-tempered note so GM instruments stay in tune
    let root = 440.0 * (((69.0 + 12.0 * (root / 440.0).log2()).round() - 69.0) / 12.0).exp2();
    let minor = !scale.default_bright;
    // SoundFont (General MIDI) lookup
    let sf = if no_gm {
        None
    } else {
        match gm::find_soundfont(soundfont.as_deref()) {
            Some(p) => match gm::load(&p) {
                Ok(sf) => {
                    say!("instruments: {}", p.display());
                    Some(sf)
                }
                Err(e) => {
                    say!("instruments: oscillators only ({e})");
                    None
                }
            },
            None => {
                say!("instruments: oscillators only (no GM SoundFont found; `sudo apt install fluid-soundfont-gm` or put TimGM6mb.sf2 next to the binary)");
                None
            }
        }
    };
    let bench = std::env::var_os("SYSJAM_BENCH").is_some();
    let tui = !no_tui && !bench && unsafe { libc::isatty(1) == 1 };
    let band_ui: Option<Arc<Mutex<ui::BandSnap>>> = tui.then(|| Arc::new(Mutex::new(ui::BandSnap::default())));
    let mon_ui: Option<Arc<Mutex<ui::MonSnap>>> = tui.then(|| Arc::new(Mutex::new(ui::MonSnap::default())));
    let make_engine = |tx, ui: Option<Arc<Mutex<ui::BandSnap>>>| {
        synth::Engine::new(synth::EngineCfg {
            sr: RATE as f32,
            root,
            minor,
            bpm,
            master: volume,
            clicks,
            chord_bars,
            seed,
            soundfont: sf.clone(),
            gm_layers,
            sub,
            tx,
            ui,
        })
    };
    if bench {
        // offline benchmark / timeline: idle 40 s, busy 80 s, idle 80 s
        let (tx, rx) = std::sync::mpsc::channel();
        let mut en = make_engine(Some(tx), None);
        let mut b = vec![0f32; BLOCK * 2];
        let mut raw = std::env::var("SYSJAM_BENCH_RAW").ok().and_then(|p| std::fs::File::create(p).ok());
        let show = std::env::var_os("SYSJAM_BENCH_LOG").is_some();
        let mut ver = 0;
        for (phase, secs, busy) in [("idle", 40usize, false), ("busy", 80, true), ("idle again", 80, false)] {
            let t = std::time::Instant::now();
            let blocks = RATE as usize * secs / BLOCK;
            let t_base = en.time();
            for i in 0..blocks {
                if i % (RATE as usize / BLOCK) == 0 {
                    let mut c = Control::default();
                    ver += 1;
                    c.version = ver;
                    if busy {
                        c.sig = band::Signals { cpu: 0.85, cores: 0.8, net: 0.75, file: 0.75, fd: 0.5, screen_change: 0.0, keyboard: 0.0, mouse: 0.0 };
                    }
                    en.set_control(&c);
                    if show {
                        while let Ok(m) = rx.try_recv() {
                            say!("  [{:5.1}s] {m}", en.time());
                        }
                    }
                }
                en.render(&mut b);
                if let Some(f) = raw.as_mut() {
                    use std::io::Write;
                    let bytes: Vec<u8> = b.iter().flat_map(|x| x.to_le_bytes()).collect();
                    let _ = f.write_all(&bytes);
                }
            }
            let e = t.elapsed().as_secs_f64();
            say!("bench {phase} ({:.0}-{:.0}s): {secs} s audio in {e:.3} s = {:.2}% of one core", t_base, en.time(), e / secs as f64 * 100.0);
        }
        return;
    }
    let mut sink = if let Some(p) = &wav {
        say!("rendering to {p} (real time, {:.0}s)", duration.unwrap());
        audio::Sink::wav(p, RATE).unwrap_or_else(|e| die(&e))
    } else if null {
        say!("audio: null sink (no sound)");
        audio::Sink::null(RATE)
    } else {
        match audio::Alsa::open(&device, RATE, 120_000) {
            Ok(a) => {
                say!("audio: ALSA '{device}' {RATE} Hz stereo");
                audio::Sink::Alsa(a)
            }
            Err(e) => {
                eprintln!("sysjam: {e}\nTry --device pipewire / --device pulse, or --wav out.wav to render to a file.");
                std::process::exit(1);
            }
        }
    };

    let stop = Arc::new(AtomicBool::new(false));
    let ctl = Arc::new(Mutex::new(Control::default()));
    let scr = Arc::new(Mutex::new(ScreenColor::default()));
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let mcfg = monitor::MonitorCfg {
        verbose,
        leads,
        clicks,
        scale_dark: scale.dark,
        scale_bright: scale.bright,
        default_bright: scale.default_bright,
        root,
        input: !no_input,
        ui: mon_ui.clone(),
    };
    if tui {
        // from here on, messages go to the UI's event log
        ui::TUI_ON.store(true, Ordering::Relaxed);
    }
    let mon = {
        let (c, s, st) = (ctl.clone(), scr.clone(), stop.clone());
        std::thread::Builder::new().name("sysjam-mon".into()).spawn(move || monitor::run(mcfg, c, s, st, Some(rx))).unwrap()
    };
    let scr_t = if no_screen {
        say!("screen colour: off (--no-screen)");
        None
    } else {
        let (s, st) = (scr.clone(), stop.clone());
        Some(std::thread::Builder::new().name("sysjam-scr".into()).spawn(move || screen::run(screen_method, screen_cmd, screen_iv, s, st, verbose)).unwrap())
    };

    let mut synth = make_engine(Some(tx), band_ui.clone());
    let tui_t = match (band_ui, mon_ui) {
        (Some(band), Some(mon)) => {
            let st = stop.clone();
            Some(
                std::thread::Builder::new()
                    .name("sysjam-tui".into())
                    .spawn(move || ui::run(ui::Ui { band, mon }, st, || SIGS.fetch_add(1, Ordering::SeqCst)))
                    .unwrap(),
            )
        }
        _ => None,
    };
    say!(
        "sysjam playing: root {} {}, seed {}{}  (Ctrl-C to fade out)",
        music::note_name(root),
        if minor { "minor" } else { "major" },
        seed,
        if synth.has_gm() { "" } else { ", oscillators only" }
    );
    let mut fbuf = vec![0f32; BLOCK * 2];
    let mut ibuf = vec![0i16; BLOCK * 2];
    let mut local = Control::default();
    let mut rendered = 0u64;
    let mut fading = false;
    loop {
        if let Ok(c) = ctl.try_lock() {
            if c.version != local.version {
                local = c.clone();
            }
        }
        synth.set_control(&local);
        synth.render(&mut fbuf);
        for (o, s) in ibuf.iter_mut().zip(&fbuf) {
            *o = (s * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
        }
        if let Err(e) = sink.write(&ibuf) {
            eprintln!("sysjam: {e}");
            break;
        }
        rendered += BLOCK as u64;
        let t = rendered as f32 / RATE as f32;
        if !fading {
            let timeup = duration.map(|d| t >= d - 2.0).unwrap_or(false);
            if SIGS.load(Ordering::Relaxed) > 0 || timeup {
                if !timeup {
                    say!("{}fading out...", if tui { "" } else { "\n" });
                }
                synth.fade_out(2.0);
                fading = true;
            }
        } else if synth.faded_out() {
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    if let Some(t) = tui_t {
        let _ = t.join(); // restores the terminal
    }
    sink.finish();
    let _ = mon.join();
    if let Some(t) = scr_t {
        // the screen thread tears down portal/PipeWire state; don't wait forever
        let t0 = std::time::Instant::now();
        while !t.is_finished() && t0.elapsed() < std::time::Duration::from_millis(1500) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if t.is_finished() {
            let _ = t.join();
        }
    }
    say!("bye.");
}
