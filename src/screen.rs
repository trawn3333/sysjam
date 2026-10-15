//! Very low-rate screen colour sampling.
//! X11: a handful of 1-pixel-high rows via x11rb GetImage (no libX11 needed).
//! Wayland: xdg-desktop-portal ScreenCast + PipeWire (one paused stream, one
//! frame every few seconds), then grim, then XWayland; otherwise disabled.

use crate::shared::{set_thread_nice, ScreenColor};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat, ImageOrder};
use x11rb::rust_connection::RustConnection;

#[derive(Default)]
struct Acc {
    n: f32,
    r: f32,
    g: f32,
    b: f32,
    warm: f32,
    dark: usize,
    total: usize,
}
impl Acc {
    fn px(&mut self, r: u8, g: u8, b: u8) {
        let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        let d = mx - mn;
        let s = if mx > 0.0 { d / mx } else { 0.0 };
        if d > 1e-3 {
            let h = if mx == r {
                ((g - b) / d).rem_euclid(6.0)
            } else if mx == g {
                (b - r) / d + 2.0
            } else {
                (r - g) / d + 4.0
            } * 60.0;
            self.warm += s * mx * (h - 30.0).to_radians().cos();
        }
        if mx < 0.02 {
            self.dark += 1;
        }
        self.total += 1;
        self.n += 1.0;
        self.r += r;
        self.g += g;
        self.b += b;
    }
    fn result(&self) -> Option<ScreenColor> {
        if self.n < 1.0 {
            return None;
        }
        let (r, g, b) = (self.r / self.n, self.g / self.n, self.b / self.n);
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        let d = mx - mn;
        let hue = if d < 1e-4 {
            0.0
        } else if mx == r {
            ((g - b) / d).rem_euclid(6.0) * 60.0
        } else if mx == g {
            ((b - r) / d + 2.0) * 60.0
        } else {
            ((r - g) / d + 4.0) * 60.0
        };
        Some(ScreenColor {
            valid: true,
            warmth: (self.warm / self.n * 2.0).clamp(-1.0, 1.0),
            brightness: 0.2126 * r + 0.7152 * g + 0.0722 * b,
            saturation: if mx > 0.0 { d / mx } else { 0.0 },
            hue,
        })
    }
    fn all_black(&self) -> bool {
        self.total > 0 && self.dark * 100 >= self.total * 99
    }
}

struct X11Grab {
    conn: RustConnection,
    root: u32,
    w: u16,
    h: u16,
    bpp: u8,
    lsb: bool,
    masks: (u32, u32, u32),
}

impl X11Grab {
    fn open() -> Result<Self, String> {
        let (conn, sn) = x11rb::connect(None).map_err(|e| format!("cannot connect to X display: {e}"))?;
        let setup = conn.setup();
        let scr = &setup.roots[sn];
        let depth = scr.root_depth;
        let bpp = setup
            .pixmap_formats
            .iter()
            .find(|f| f.depth == depth)
            .map(|f| f.bits_per_pixel)
            .unwrap_or(0);
        let vis = scr
            .allowed_depths
            .iter()
            .flat_map(|d| d.visuals.iter())
            .find(|v| v.visual_id == scr.root_visual)
            .ok_or("no root visual")?;
        if bpp != 32 && bpp != 24 {
            return Err(format!("unsupported pixel format ({bpp} bpp)"));
        }
        Ok(X11Grab {
            root: scr.root,
            w: scr.width_in_pixels,
            h: scr.height_in_pixels,
            bpp,
            lsb: setup.image_byte_order == ImageOrder::LSB_FIRST,
            masks: (vis.red_mask, vis.green_mask, vis.blue_mask),
            conn,
        })
    }
    fn grab(&self) -> Result<Acc, String> {
        const ROWS: u16 = 12;
        let mut cookies = Vec::new();
        for i in 0..ROWS {
            let y = ((i as u32 * 2 + 1) * self.h as u32 / (ROWS as u32 * 2)) as i16;
            cookies.push(
                self.conn
                    .get_image(ImageFormat::Z_PIXMAP, self.root, 0, y, self.w, 1, !0)
                    .map_err(|e| e.to_string())?,
            );
        }
        let mut acc = Acc::default();
        let bytes = (self.bpp / 8) as usize;
        let shift = |m: u32| m.trailing_zeros();
        let (rm, gm, bm) = self.masks;
        for c in cookies {
            let rep = c.reply().map_err(|e| format!("GetImage failed: {e:?}"))?;
            let d = &rep.data;
            let step = (self.w as usize / 160).max(1);
            let mut x = 0;
            while (x + 1) * bytes <= d.len() {
                let o = x * bytes;
                let mut v: u32 = 0;
                for k in 0..bytes {
                    let byte = d[o + k] as u32;
                    if self.lsb {
                        v |= byte << (8 * k);
                    } else {
                        v = (v << 8) | byte;
                    }
                }
                let r = ((v & rm) >> shift(rm)) as u8;
                let g = ((v & gm) >> shift(gm)) as u8;
                let b = ((v & bm) >> shift(bm)) as u8;
                acc.px(r, g, b);
                x += step;
            }
        }
        Ok(acc)
    }
}

/// Run a command producing a binary PPM (P6) on stdout, with a timeout.
fn grab_cmd(cmd: &[String]) -> Result<Acc, String> {
    let mut child = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", cmd[0]))?;
    let out;
    let stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut v = Vec::new();
        if let Some(mut s) = stdout {
            let _ = s.read_to_end(&mut v);
        }
        v
    });
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                out = reader.join().unwrap_or_default();
                if !st.success() {
                    let mut e = String::new();
                    if let Some(mut se) = child.stderr.take() {
                        use std::io::Read;
                        let _ = se.read_to_string(&mut e);
                    }
                    return Err(format!("{} exited with {st}: {}", cmd[0], e.trim()));
                }
                break;
            }
            Ok(None) if t0.elapsed() > Duration::from_secs(4) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{} timed out", cmd[0]));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e.to_string()),
        }
    }
    parse_ppm(&out).ok_or_else(|| format!("{} did not output a P6 PPM image", cmd[0]))
}

fn parse_ppm(d: &[u8]) -> Option<Acc> {
    if d.len() < 2 || &d[0..2] != b"P6" {
        return None;
    }
    let mut pos = 2;
    let mut nums = [0usize; 3];
    for n in nums.iter_mut() {
        loop {
            while pos < d.len() && d[pos].is_ascii_whitespace() {
                pos += 1;
            }
            if pos < d.len() && d[pos] == b'#' {
                while pos < d.len() && d[pos] != b'\n' {
                    pos += 1;
                }
            } else {
                break;
            }
        }
        let st = pos;
        while pos < d.len() && d[pos].is_ascii_digit() {
            pos += 1;
        }
        *n = std::str::from_utf8(&d[st..pos]).ok()?.parse().ok()?;
    }
    pos += 1;
    let (w, h, maxv) = (nums[0], nums[1], nums[2]);
    let bps = if maxv < 256 { 1 } else { 2 };
    if maxv == 0 || d.len() < pos + w * h * 3 * bps {
        return None;
    }
    let px = &d[pos..];
    let total = w * h;
    let step = (total / 4000).max(1);
    let mut acc = Acc::default();
    let mut i = 0;
    let ch = |o: usize| -> u8 {
        let v = if bps == 1 { px[o] as u32 } else { (px[o] as u32) << 8 | px[o + 1] as u32 };
        (v * 255 / maxv as u32) as u8
    };
    while i < total {
        let o = i * 3 * bps;
        acc.px(ch(o), ch(o + bps), ch(o + 2 * bps));
        i += step;
    }
    Some(acc)
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

enum Source {
    X(X11Grab),
    Cmd(Vec<String>),
    Portal(crate::pwcap::Capture, crate::portal::Session),
}

/// X11 connect + first grab on a helper thread so a wedged or absent
/// server can never hold us up (fails fast and quietly).
fn try_x11(wayland: bool) -> Result<(X11Grab, Acc), String> {
    if std::env::var_os("DISPLAY").map(|d| d.is_empty()).unwrap_or(true) {
        return Err("no X display".into());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("sysjam-x11".into())
        .spawn(move || {
            let r = X11Grab::open().and_then(|x| x.grab().map(|a| (x, a)));
            let _ = tx.send(r);
        })
        .map_err(|e| e.to_string())?;
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok((x, a))) => {
            if wayland && a.all_black() {
                Err("XWayland root window is black (native Wayland content is not exposed)".into())
            } else {
                Ok((x, a))
            }
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err("X server did not answer".into()),
    }
}

fn grim_cmd() -> Vec<String> {
    vec!["grim".into(), "-s".into(), "0.05".into(), "-t".into(), "ppm".into(), "-".into()]
}

fn px_acc(px: &[(u8, u8, u8)]) -> Acc {
    let mut a = Acc::default();
    for &(r, g, b) in px {
        a.px(r, g, b);
    }
    a
}

pub fn run(method: String, screen_cmd: Option<String>, interval: f32, out: Arc<Mutex<ScreenColor>>, stop: Arc<AtomicBool>, verbose: bool) {
    set_thread_nice(15);
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").map(|s| s == "wayland").unwrap_or(false);
    let off = |why: &str| say!("screen colour: off ({why})");
    let stopped = || stop.load(Ordering::Relaxed);

    let order: Vec<&str> = match method.as_str() {
        "auto" if wayland => vec!["portal", "grim", "x11"],
        "auto" => vec!["x11", "portal"],
        m => vec![m],
    };
    let mut notes: Vec<String> = Vec::new();
    let mut chosen: Option<(Source, &str)> = None;
    let mut declined = false;
    for m in order {
        if stopped() {
            return;
        }
        match m {
            "cmd" => {
                let Some(c) = screen_cmd.clone() else {
                    off("--screen-method cmd needs --screen-cmd");
                    return;
                };
                let v = vec!["sh".into(), "-c".into(), c];
                match grab_cmd(&v) {
                    Ok(a) => {
                        if let Some(sc) = a.result() {
                            *out.lock().unwrap() = sc;
                        }
                        chosen = Some((Source::Cmd(v), "--screen-cmd"));
                    }
                    Err(e) => notes.push(format!("cmd: {e}")),
                }
            }
            "grim" => {
                if !which("grim") {
                    notes.push("grim: not installed".into());
                    continue;
                }
                match grab_cmd(&grim_cmd()) {
                    Ok(a) => {
                        if let Some(sc) = a.result() {
                            *out.lock().unwrap() = sc;
                        }
                        chosen = Some((Source::Cmd(grim_cmd()), "grim"));
                    }
                    Err(e) => notes.push(format!("grim: {e}")),
                }
            }
            "x11" => match try_x11(wayland) {
                Ok((x, a)) => {
                    if let Some(sc) = a.result() {
                        *out.lock().unwrap() = sc;
                    }
                    chosen = Some((Source::X(x), if wayland { "XWayland" } else { "X11" }));
                }
                Err(e) => notes.push(format!("x11: {e}")),
            },
            "portal" => {
                if method == "portal" {
                    if let Some(p) = crate::portal::declined_path() {
                        let _ = std::fs::remove_file(p);
                    }
                } else if crate::portal::declined_path().map(|p| p.exists()).unwrap_or(false) {
                    notes.push("portal: declined earlier (--screen-method portal asks again)".into());
                    declined = true;
                    continue;
                }
                match crate::portal::open(&stopped, verbose) {
                    Ok(sess) => match crate::pwcap::Capture::open(sess.fd, sess.node) {
                        Ok(cap) => {
                            // first frame (stream starts active)
                            match cap.grab(Duration::from_secs(5), &stopped) {
                                Ok(px) => {
                                    if let Some(sc) = px.and_then(|p| px_acc(&p).result()) {
                                        *out.lock().unwrap() = sc;
                                    }
                                    if verbose {
                                        let (f, w, h) = cap.format();
                                        say!("screen colour: portal stream node {} format {f} {w}x{h}{}", sess.node, if sess.prompted { " (permission saved)" } else { " (restored, no prompt)" });
                                    }
                                    chosen = Some((Source::Portal(cap, sess), "xdg-desktop-portal ScreenCast"));
                                }
                                Err(e) => {
                                    drop(cap);
                                    sess.close();
                                    notes.push(format!("portal: {e}"));
                                }
                            }
                        }
                        Err(e) => {
                            sess.close();
                            notes.push(format!("portal: {e}"));
                        }
                    },
                    Err(crate::portal::Fail::Declined) => {
                        crate::portal::mark_declined();
                        declined = true;
                        notes.push("portal: screen sharing declined".into());
                    }
                    Err(crate::portal::Fail::Unavailable(e)) => notes.push(format!("portal: unavailable ({e})")),
                    Err(crate::portal::Fail::Error(e)) => notes.push(format!("portal: {e}")),
                }
            }
            _ => {}
        }
        if chosen.is_some() {
            break;
        }
    }
    let Some((src, name)) = chosen else {
        if stopped() {
            return;
        }
        if verbose {
            for n in &notes {
                say!("screen colour: {n}");
            }
        }
        off(if declined { "screen sharing declined; --screen-method portal asks again" } else if wayland { "no portal/grim capture; --verbose for details" } else { "no capture method; --verbose for details" });
        return;
    };
    if verbose {
        for n in &notes {
            say!("screen colour: skipped {n}");
        }
    }
    say!("screen colour: {name}, one tiny sample every {interval:.0}s");
    let mut blacks = 0;
    let mut fails = 0;
    loop {
        // sleep in small slices so shutdown is prompt
        let t0 = Instant::now();
        while t0.elapsed().as_secs_f32() < interval {
            if stopped() {
                if let Source::Portal(cap, sess) = src {
                    if verbose {
                        say!("screen colour: {} portal frames received", cap.frames());
                    }
                    drop(cap);
                    sess.close();
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let r = match &src {
            Source::X(x) => x.grab().map(Some),
            Source::Cmd(v) => grab_cmd(v).map(Some),
            Source::Portal(cap, _) => {
                if cap.state() < 0 {
                    Err("portal stream ended".into())
                } else {
                    cap.grab(Duration::from_millis(2500), &stopped).map(|o| o.map(|p| px_acc(&p)))
                }
            }
        };
        match r {
            Ok(None) => {} // static screen: no new frame, keep the last colour
            Ok(Some(a)) => {
                fails = 0;
                if wayland && matches!(src, Source::X(_)) && a.all_black() {
                    blacks += 1;
                    if blacks >= 3 {
                        out.lock().unwrap().valid = false;
                        off("XWayland only returns black");
                        return;
                    }
                    continue;
                }
                blacks = 0;
                if let Some(sc) = a.result() {
                    *out.lock().unwrap() = sc;
                }
            }
            Err(e) => {
                fails += 1;
                if verbose {
                    say!("screen colour: capture error: {e}");
                }
                if fails >= 3 {
                    out.lock().unwrap().valid = false;
                    off(&e);
                    if let Source::Portal(cap, sess) = src {
                        drop(cap);
                        sess.close();
                    }
                    return;
                }
            }
        }
    }
}
