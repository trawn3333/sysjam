//! Tiny PipeWire video consumer loaded with dlopen("libpipewire-0.3.so.0").
//! Negotiates raw RGB-ish shared-memory frames (no DMA-BUF), keeps the stream
//! paused between samples and grabs a single frame every few seconds.

use std::ffi::{c_char, c_int, c_void, CString};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

macro_rules! fns {
    ($($name:ident : fn($($a:ty),*) -> $r:ty;)*) => {
        #[allow(non_snake_case)]
        #[derive(Clone, Copy)]
        struct Fns { $($name: unsafe extern "C" fn($($a),*) -> $r,)* }
        impl Fns {
            unsafe fn load(h: *mut c_void) -> Result<Fns, String> {
                Ok(Fns { $($name: {
                    let s = CString::new(stringify!($name)).unwrap();
                    let p = libc::dlsym(h, s.as_ptr());
                    if p.is_null() { return Err(format!("libpipewire lacks {}", stringify!($name))); }
                    std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($a),*) -> $r>(p)
                },)* })
            }
        }
    };
}
type P = *mut c_void;
fns! {
    pw_init: fn(*mut c_int, *mut *mut *mut c_char) -> ();
    pw_thread_loop_new: fn(*const c_char, P) -> P;
    pw_thread_loop_get_loop: fn(P) -> P;
    pw_thread_loop_start: fn(P) -> c_int;
    pw_thread_loop_stop: fn(P) -> ();
    pw_thread_loop_destroy: fn(P) -> ();
    pw_thread_loop_lock: fn(P) -> ();
    pw_thread_loop_unlock: fn(P) -> ();
    pw_context_new: fn(P, P, usize) -> P;
    pw_context_destroy: fn(P) -> ();
    pw_context_connect_fd: fn(P, c_int, P, usize) -> P;
    pw_core_disconnect: fn(P) -> c_int;
    pw_properties_new_string: fn(*const c_char) -> P;
    pw_stream_new: fn(P, *const c_char, P) -> P;
    pw_stream_add_listener: fn(P, P, *const Events, P) -> ();
    pw_stream_connect: fn(P, u32, u32, u32, *const *const u8, u32) -> c_int;
    pw_stream_disconnect: fn(P) -> c_int;
    pw_stream_destroy: fn(P) -> ();
    pw_stream_set_active: fn(P, bool) -> c_int;
    pw_stream_update_params: fn(P, *const *const u8, u32) -> c_int;
    pw_stream_dequeue_buffer: fn(P) -> P;
    pw_stream_queue_buffer: fn(P, P) -> c_int;
}

#[repr(C)]
pub struct Events {
    version: u32,
    destroy: Option<unsafe extern "C" fn(P)>,
    state_changed: Option<unsafe extern "C" fn(P, c_int, c_int, *const c_char)>,
    control_info: Option<unsafe extern "C" fn(P, u32, P)>,
    io_changed: Option<unsafe extern "C" fn(P, u32, P, u32)>,
    param_changed: Option<unsafe extern "C" fn(P, u32, *const u8)>,
    add_buffer: Option<unsafe extern "C" fn(P, P)>,
    remove_buffer: Option<unsafe extern "C" fn(P, P)>,
    process: Option<unsafe extern "C" fn(P)>,
    drained: Option<unsafe extern "C" fn(P)>,
}

// ---- SPA pod builder (just enough for EnumFormat and Buffers) ----
struct Pod(Vec<u8>);
impl Pod {
    fn u32s(v: &[u32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }
    fn wrap(ty: u32, body: &[u8]) -> Vec<u8> {
        let mut o = Pod::u32s(&[body.len() as u32, ty]);
        o.extend_from_slice(body);
        while o.len() % 8 != 0 {
            o.push(0);
        }
        o
    }
    fn id(v: u32) -> Vec<u8> {
        Pod::wrap(3, &v.to_le_bytes())
    }
    fn choice(kind: u32, child_ty: u32, child_size: u32, vals: &[u32]) -> Vec<u8> {
        let mut b = Pod::u32s(&[kind, 0, child_size, child_ty]);
        b.extend(Pod::u32s(vals));
        Pod::wrap(19, &b)
    }
    fn object(oty: u32, id: u32, props: &[(u32, Vec<u8>)]) -> Pod {
        let mut b = Pod::u32s(&[oty, id]);
        for (k, v) in props {
            b.extend(Pod::u32s(&[*k, 0]));
            b.extend_from_slice(v);
        }
        Pod(Pod::wrap(15, &b))
    }
}

const FMT_BGRX: u32 = 8;
const FMT_RGBX: u32 = 7;
const FMT_XRGB: u32 = 9;
const FMT_XBGR: u32 = 10;
const FMT_RGBA: u32 = 11;
const FMT_BGRA: u32 = 12;
const FMT_ARGB: u32 = 13;
const FMT_ABGR: u32 = 14;
const FMT_RGB: u32 = 15;
const FMT_BGR: u32 = 16;

fn enum_format() -> Pod {
    let fmts = [FMT_BGRX, FMT_BGRX, FMT_RGBX, FMT_BGRA, FMT_RGBA, FMT_XRGB, FMT_XBGR, FMT_ARGB, FMT_ABGR, FMT_RGB, FMT_BGR];
    Pod::object(
        0x40003,
        3, // EnumFormat
        &[
            (1, Pod::id(2)), // mediaType video
            (2, Pod::id(1)), // mediaSubtype raw
            (0x20001, Pod::choice(3, 3, 4, &fmts)),
            // size: prefer tiny; compositors that can't scale just send native size
            (0x20003, Pod::choice(1, 10, 8, &[256, 144, 1, 1, 16384, 16384])),
            (0x20004, Pod::choice(1, 11, 8, &[1, 1, 0, 1, 360, 1])),
        ],
    )
}
fn buffers_param() -> Pod {
    Pod::object(
        0x40004,
        5, // Buffers
        &[
            (1, Pod::choice(1, 4, 4, &[2, 1, 4])), // buffers: 2 (1..4)
            (6, Pod::choice(4, 4, 4, &[(1 << 1) | (1 << 2)])), // dataType MemPtr | MemFd
        ],
    )
}

/// parse a Format object: (format, width, height)
unsafe fn parse_format(p: *const u8) -> Option<(u32, u32, u32)> {
    let rd = |o: usize| u32::from_le_bytes(std::slice::from_raw_parts(p.add(o), 4).try_into().unwrap());
    let size = rd(0) as usize;
    if rd(4) != 15 || size < 8 || size > 65536 {
        return None;
    }
    let (mut fmt, mut w, mut h) = (0, 0, 0);
    let mut o = 16;
    while o + 16 <= 8 + size {
        let key = rd(o);
        let vs = rd(o + 8) as usize;
        let vt = rd(o + 12);
        let body = o + 16;
        // value may itself be a Choice(None) wrapper
        let (vt2, body2) = if vt == 19 { (rd(body + 12), body + 16) } else { (vt, body) };
        match (key, vt2) {
            (0x20001, 3) => fmt = rd(body2),
            (0x20003, 10) => {
                w = rd(body2);
                h = rd(body2 + 4);
            }
            _ => {}
        }
        o = body + ((vs + 7) & !7);
    }
    Some((fmt, w, h))
}

struct Shared {
    f: Fns,
    stream: P,
    want: AtomicBool,
    state: AtomicI32,
    frames: std::sync::atomic::AtomicU32,
    fmt: Mutex<(u32, u32, u32)>,
    out: Mutex<Option<Vec<(u8, u8, u8)>>>,
    err: Mutex<Option<String>>,
}

unsafe extern "C" fn on_state(d: P, _old: c_int, st: c_int, err: *const c_char) {
    let s = &*(d as *const Shared);
    s.state.store(st, Ordering::Relaxed);
    if st < 0 {
        let m = if err.is_null() { "stream error".into() } else { std::ffi::CStr::from_ptr(err).to_string_lossy().into_owned() };
        *s.err.lock().unwrap() = Some(m);
    }
}
unsafe extern "C" fn on_param(d: P, id: u32, param: *const u8) {
    let s = &*(d as *const Shared);
    if id != 4 || param.is_null() {
        return;
    }
    if let Some(f) = parse_format(param) {
        *s.fmt.lock().unwrap() = f;
    }
    let b = buffers_param();
    let pp = [b.0.as_ptr()];
    (s.f.pw_stream_update_params)(s.stream, pp.as_ptr(), 1);
}
unsafe extern "C" fn on_process(d: P) {
    let s = &*(d as *const Shared);
    let b = (s.f.pw_stream_dequeue_buffer)(s.stream);
    if b.is_null() {
        return;
    }
    s.frames.fetch_add(1, Ordering::Relaxed);
    if s.want.load(Ordering::Relaxed) {
        match sample(s, b) {
            Ok(Some(px)) => {
                *s.out.lock().unwrap() = Some(px);
                s.want.store(false, Ordering::Relaxed);
            }
            Ok(None) => {}
            Err(e) => *s.err.lock().unwrap() = Some(e),
        }
    }
    (s.f.pw_stream_queue_buffer)(s.stream, b);
}

unsafe fn sample(s: &Shared, pwb: P) -> Result<Option<Vec<(u8, u8, u8)>>, String> {
    let sb = *(pwb as *const *const u8); // pw_buffer.buffer -> spa_buffer
    if sb.is_null() {
        return Ok(None);
    }
    let n_datas = *(sb.add(4) as *const u32);
    let datas = *(sb.add(16) as *const *const u8);
    if n_datas == 0 || datas.is_null() {
        return Ok(None);
    }
    let dtype = *(datas as *const u32);
    let data = *(datas.add(24) as *const *const u8);
    let chunk = *(datas.add(32) as *const *const u8);
    let maxsize = *(datas.add(20) as *const u32) as usize;
    if dtype == 3 || data.is_null() {
        return Err("compositor only offers DMA-BUF frames".into());
    }
    if chunk.is_null() {
        return Ok(None);
    }
    let c_off = *(chunk as *const u32) as usize;
    let c_size = *(chunk.add(4) as *const u32) as usize;
    let c_stride = *(chunk.add(8) as *const i32);
    let c_flags = *(chunk.add(12) as *const i32);
    if c_size == 0 || c_flags & 1 != 0 {
        return Ok(None); // empty (cursor-only) or corrupted frame
    }
    let (fmt, w, h) = *s.fmt.lock().unwrap();
    let (w, h) = (w as usize, h as usize);
    let bpp = if fmt == FMT_RGB || fmt == FMT_BGR { 3 } else { 4 };
    if w == 0 || h == 0 {
        return Ok(None);
    }
    let stride = if c_stride > 0 { c_stride as usize } else { w * bpp };
    let off = c_off % maxsize.max(1);
    let avail = c_size.min(maxsize.saturating_sub(off));
    let base = data.add(off);
    let (ri, gi, bi) = match fmt {
        FMT_BGRX | FMT_BGRA | FMT_BGR => (2, 1, 0),
        FMT_RGBX | FMT_RGBA | FMT_RGB => (0, 1, 2),
        FMT_XRGB | FMT_ARGB => (1, 2, 3),
        FMT_XBGR | FMT_ABGR => (3, 2, 1),
        _ => return Err(format!("unexpected pixel format {fmt}")),
    };
    let mut px = Vec::with_capacity(64 * 36);
    for yi in 0..36 {
        let y = (yi * 2 + 1) * h / 72;
        for xi in 0..64 {
            let x = (xi * 2 + 1) * w / 128;
            let o = y * stride + x * bpp;
            if o + bpp > avail {
                continue;
            }
            let p = base.add(o);
            px.push((*p.add(ri), *p.add(gi), *p.add(bi)));
        }
    }
    Ok(if px.is_empty() { None } else { Some(px) })
}

pub struct Capture {
    f: Fns,
    tl: P,
    ctx: P,
    core: P,
    stream: P,
    _hook: Box<[u64; 8]>,
    _ev: Box<Events>,
    sh: Box<Shared>,
}
unsafe impl Send for Capture {}

static EVENTS: Events = Events {
    version: 0,
    destroy: None,
    state_changed: Some(on_state),
    control_info: None,
    io_changed: None,
    param_changed: Some(on_param),
    add_buffer: None,
    remove_buffer: None,
    process: Some(on_process),
    drained: None,
};

impl Capture {
    /// Connect to the portal's PipeWire remote (takes ownership of `fd`).
    pub fn open(fd: i32, node: u32) -> Result<Capture, String> {
        unsafe {
            let h = libc::dlopen(c"libpipewire-0.3.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if h.is_null() {
                libc::close(fd);
                return Err("libpipewire-0.3.so.0 not found".into());
            }
            let f = Fns::load(h).inspect_err(|_| {
                libc::close(fd);
            })?;
            (f.pw_init)(std::ptr::null_mut(), std::ptr::null_mut());
            let tl = (f.pw_thread_loop_new)(c"sysjam-pw".as_ptr(), std::ptr::null_mut());
            if tl.is_null() {
                libc::close(fd);
                return Err("pw_thread_loop_new failed".into());
            }
            let ctx = (f.pw_context_new)((f.pw_thread_loop_get_loop)(tl), std::ptr::null_mut(), 0);
            if ctx.is_null() {
                (f.pw_thread_loop_destroy)(tl);
                libc::close(fd);
                return Err("pw_context_new failed".into());
            }
            if (f.pw_thread_loop_start)(tl) < 0 {
                (f.pw_context_destroy)(ctx);
                (f.pw_thread_loop_destroy)(tl);
                libc::close(fd);
                return Err("pw_thread_loop_start failed".into());
            }
            (f.pw_thread_loop_lock)(tl);
            let core = (f.pw_context_connect_fd)(ctx, fd, std::ptr::null_mut(), 0);
            if core.is_null() {
                (f.pw_thread_loop_unlock)(tl);
                (f.pw_thread_loop_stop)(tl);
                (f.pw_context_destroy)(ctx);
                (f.pw_thread_loop_destroy)(tl);
                return Err("cannot connect to the portal's PipeWire remote".into());
            }
            let props = (f.pw_properties_new_string)(c"media.type=Video media.category=Capture media.role=Screen node.name=sysjam-screen".as_ptr());
            let stream = (f.pw_stream_new)(core, c"sysjam screen colour".as_ptr(), props);
            let mut cap = Capture {
                f,
                tl,
                ctx,
                core,
                stream,
                _hook: Box::new([0; 8]),
                _ev: Box::new(Events { ..EVENTS }),
                sh: Box::new(Shared {
                    f,
                    stream,
                    want: AtomicBool::new(true),
                    state: AtomicI32::new(0),
                    frames: std::sync::atomic::AtomicU32::new(0),
                    fmt: Mutex::new((0, 0, 0)),
                    out: Mutex::new(None),
                    err: Mutex::new(None),
                }),
            };
            if stream.is_null() {
                (f.pw_thread_loop_unlock)(tl);
                cap.stream = std::ptr::null_mut();
                drop(cap);
                return Err("pw_stream_new failed".into());
            }
            (f.pw_stream_add_listener)(stream, cap._hook.as_mut_ptr() as P, &*cap._ev, &*cap.sh as *const Shared as P);
            let ef = enum_format();
            let params = [ef.0.as_ptr()];
            let r = (f.pw_stream_connect)(stream, 0, node, 1 | 4, params.as_ptr(), 1);
            (f.pw_thread_loop_unlock)(tl);
            if r < 0 {
                return Err(format!("pw_stream_connect failed ({r})"));
            }
            Ok(cap)
        }
    }

    fn set_active(&self, on: bool) {
        unsafe {
            (self.f.pw_thread_loop_lock)(self.tl);
            (self.f.pw_stream_set_active)(self.stream, on);
            (self.f.pw_thread_loop_unlock)(self.tl);
        }
    }

    /// Resume the stream, wait (bounded) for one frame, pause again.
    /// Ok(None) = no new frame arrived (static screen); Err = give up.
    pub fn grab(&self, timeout: Duration, stop: &dyn Fn() -> bool) -> Result<Option<Vec<(u8, u8, u8)>>, String> {
        self.sh.want.store(true, Ordering::Relaxed);
        self.set_active(true);
        let t0 = Instant::now();
        let r = loop {
            if let Some(e) = self.sh.err.lock().unwrap().take() {
                break Err(e);
            }
            if let Some(px) = self.sh.out.lock().unwrap().take() {
                break Ok(Some(px));
            }
            if t0.elapsed() > timeout || stop() {
                break Ok(None);
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        self.sh.want.store(false, Ordering::Relaxed);
        self.set_active(false);
        r
    }

    /// buffers delivered by the compositor so far
    pub fn frames(&self) -> u32 {
        self.sh.frames.load(Ordering::Relaxed)
    }
    pub fn state(&self) -> i32 {
        self.sh.state.load(Ordering::Relaxed)
    }
    pub fn format(&self) -> (u32, u32, u32) {
        *self.sh.fmt.lock().unwrap()
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            let f = self.f;
            (f.pw_thread_loop_lock)(self.tl);
            if !self.stream.is_null() {
                (f.pw_stream_disconnect)(self.stream);
                (f.pw_stream_destroy)(self.stream);
            }
            (f.pw_core_disconnect)(self.core);
            (f.pw_thread_loop_unlock)(self.tl);
            (f.pw_thread_loop_stop)(self.tl);
            (f.pw_context_destroy)(self.ctx);
            (f.pw_thread_loop_destroy)(self.tl);
        }
    }
}
