//! Minimal libdbus-1 binding, loaded at runtime with dlopen (no build-time
//! dependency, no extra glibc symbols). Only what the ScreenCast portal needs.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::time::{Duration, Instant};

type Conn = c_void;
type Msg = c_void;
#[repr(C, align(8))]
struct Iter([u8; 128]);
#[repr(C, align(8))]
struct DErr([u8; 64]);
impl DErr {
    fn new() -> Box<DErr> {
        Box::new(DErr([0; 64]))
    }
    fn message(&self) -> Option<String> {
        unsafe {
            let p = self as *const DErr as *const *const c_char;
            let name = *p;
            let msg = *p.add(1);
            if name.is_null() {
                return None;
            }
            let n = CStr::from_ptr(name).to_string_lossy().into_owned();
            let m = if msg.is_null() { String::new() } else { CStr::from_ptr(msg).to_string_lossy().into_owned() };
            Some(if m.is_empty() { n } else { format!("{n}: {m}") })
        }
    }
}

macro_rules! fns {
    ($($name:ident : fn($($a:ty),*) -> $r:ty;)*) => {
        #[allow(non_snake_case)]
        struct Fns { $($name: unsafe extern "C" fn($($a),*) -> $r,)* }
        impl Fns {
            unsafe fn load(h: *mut c_void) -> Result<Fns, String> {
                Ok(Fns { $($name: {
                    let s = CString::new(stringify!($name)).unwrap();
                    let p = libc::dlsym(h, s.as_ptr());
                    if p.is_null() { return Err(format!("libdbus-1 lacks {}", stringify!($name))); }
                    std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($a),*) -> $r>(p)
                },)* })
            }
        }
    };
}
fns! {
    dbus_error_init: fn(*mut DErr) -> ();
    dbus_error_free: fn(*mut DErr) -> ();
    dbus_bus_get_private: fn(c_int, *mut DErr) -> *mut Conn;
    dbus_connection_set_exit_on_disconnect: fn(*mut Conn, u32) -> ();
    dbus_bus_get_unique_name: fn(*mut Conn) -> *const c_char;
    dbus_bus_add_match: fn(*mut Conn, *const c_char, *mut DErr) -> ();
    dbus_message_new_method_call: fn(*const c_char, *const c_char, *const c_char, *const c_char) -> *mut Msg;
    dbus_message_iter_init_append: fn(*mut Msg, *mut Iter) -> ();
    dbus_message_iter_append_basic: fn(*mut Iter, c_int, *const c_void) -> u32;
    dbus_message_iter_open_container: fn(*mut Iter, c_int, *const c_char, *mut Iter) -> u32;
    dbus_message_iter_close_container: fn(*mut Iter, *mut Iter) -> u32;
    dbus_connection_send_with_reply_and_block: fn(*mut Conn, *mut Msg, c_int, *mut DErr) -> *mut Msg;
    dbus_connection_read_write: fn(*mut Conn, c_int) -> u32;
    dbus_connection_pop_message: fn(*mut Conn) -> *mut Msg;
    dbus_message_is_signal: fn(*mut Msg, *const c_char, *const c_char) -> u32;
    dbus_message_get_path: fn(*mut Msg) -> *const c_char;
    dbus_message_iter_init: fn(*mut Msg, *mut Iter) -> u32;
    dbus_message_iter_get_arg_type: fn(*mut Iter) -> c_int;
    dbus_message_iter_get_basic: fn(*mut Iter, *mut c_void) -> ();
    dbus_message_iter_recurse: fn(*mut Iter, *mut Iter) -> ();
    dbus_message_iter_next: fn(*mut Iter) -> u32;
    dbus_message_unref: fn(*mut Msg) -> ();
    dbus_connection_close: fn(*mut Conn) -> ();
    dbus_connection_unref: fn(*mut Conn) -> ();
}

/// A decoded D-Bus value (only the shapes the portal uses).
#[derive(Debug, Clone)]
pub enum V {
    S(String),
    U(u64),
    I(i64),
    B(bool),
    Fd(i32),
    D(f64),
    List(Vec<V>),
}
impl V {
    pub fn u(&self) -> Option<u64> {
        match self {
            V::U(x) => Some(*x),
            V::I(x) if *x >= 0 => Some(*x as u64),
            V::List(l) if l.len() == 1 => l[0].u(),
            _ => None,
        }
    }
    pub fn s(&self) -> Option<&str> {
        match self {
            V::S(s) => Some(s),
            V::List(l) if l.len() == 1 => l[0].s(),
            _ => None,
        }
    }
    /// look up key in an a{sv} (List of List[S(key), value])
    pub fn get(&self, key: &str) -> Option<&V> {
        if let V::List(l) = self {
            for e in l {
                if let V::List(kv) = e {
                    if kv.len() == 2 && kv[0].s() == Some(key) {
                        return Some(&kv[1]);
                    }
                }
            }
        }
        None
    }
}

/// Argument for method calls.
pub enum A<'a> {
    S(&'a str),
    O(&'a str),
    /// a{sv} with string / u32 / bool values
    Dict(Vec<(&'a str, DV)>),
}
pub enum DV {
    S(String),
    U(u32),
    B(bool),
}

pub struct Bus {
    f: Fns,
    c: *mut Conn,
}
unsafe impl Send for Bus {}

impl Bus {
    pub fn session() -> Result<Bus, String> {
        unsafe {
            let h = libc::dlopen(c"libdbus-1.so.3".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if h.is_null() {
                return Err("libdbus-1.so.3 not found".into());
            }
            let f = Fns::load(h)?;
            let mut e = DErr::new();
            (f.dbus_error_init)(&mut *e);
            let c = (f.dbus_bus_get_private)(0, &mut *e);
            if c.is_null() {
                let m = e.message().unwrap_or_else(|| "no session bus".into());
                (f.dbus_error_free)(&mut *e);
                return Err(m);
            }
            (f.dbus_connection_set_exit_on_disconnect)(c, 0);
            Ok(Bus { f, c })
        }
    }

    pub fn unique_name(&self) -> String {
        unsafe {
            let p = (self.f.dbus_bus_get_unique_name)(self.c);
            if p.is_null() {
                String::new()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }

    pub fn add_match(&self, rule: &str) -> Result<(), String> {
        unsafe {
            let r = CString::new(rule).unwrap();
            let mut e = DErr::new();
            (self.f.dbus_error_init)(&mut *e);
            (self.f.dbus_bus_add_match)(self.c, r.as_ptr(), &mut *e);
            if let Some(m) = e.message() {
                (self.f.dbus_error_free)(&mut *e);
                return Err(m);
            }
            Ok(())
        }
    }

    unsafe fn append(&self, it: &mut Iter, a: &A) {
        let f = &self.f;
        let s = |it: &mut Iter, t: u8, v: &str| {
            let cs = CString::new(v).unwrap();
            let p = cs.as_ptr();
            (f.dbus_message_iter_append_basic)(it, t as c_int, &p as *const _ as *const c_void);
        };
        match a {
            A::S(v) => s(it, b's', v),
            A::O(v) => s(it, b'o', v),
            A::Dict(entries) => {
                let mut arr = Iter([0; 128]);
                (f.dbus_message_iter_open_container)(it, b'a' as c_int, c"{sv}".as_ptr(), &mut arr);
                for (k, v) in entries {
                    let mut ent = Iter([0; 128]);
                    (f.dbus_message_iter_open_container)(&mut arr, b'e' as c_int, std::ptr::null(), &mut ent);
                    s(&mut ent, b's', k);
                    let mut var = Iter([0; 128]);
                    match v {
                        DV::S(x) => {
                            (f.dbus_message_iter_open_container)(&mut ent, b'v' as c_int, c"s".as_ptr(), &mut var);
                            s(&mut var, b's', x);
                        }
                        DV::U(x) => {
                            (f.dbus_message_iter_open_container)(&mut ent, b'v' as c_int, c"u".as_ptr(), &mut var);
                            (f.dbus_message_iter_append_basic)(&mut var, b'u' as c_int, x as *const u32 as *const c_void);
                        }
                        DV::B(x) => {
                            (f.dbus_message_iter_open_container)(&mut ent, b'v' as c_int, c"b".as_ptr(), &mut var);
                            let b: u32 = *x as u32;
                            (f.dbus_message_iter_append_basic)(&mut var, b'b' as c_int, &b as *const u32 as *const c_void);
                        }
                    }
                    (f.dbus_message_iter_close_container)(&mut ent, &mut var);
                    (f.dbus_message_iter_close_container)(&mut arr, &mut ent);
                }
                (f.dbus_message_iter_close_container)(it, &mut arr);
            }
        }
    }

    unsafe fn read(&self, it: &mut Iter) -> Option<V> {
        let f = &self.f;
        let t = (f.dbus_message_iter_get_arg_type)(it);
        let mut raw: u64 = 0;
        let basic = |it: &mut Iter, raw: &mut u64| (f.dbus_message_iter_get_basic)(it, raw as *mut u64 as *mut c_void);
        Some(match t as u8 {
            0 => return None,
            b's' | b'o' | b'g' => {
                basic(it, &mut raw);
                let p = raw as usize as *const c_char;
                V::S(if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() })
            }
            b'y' => {
                basic(it, &mut raw);
                V::U(raw & 0xff)
            }
            b'q' => {
                basic(it, &mut raw);
                V::U(raw & 0xffff)
            }
            b'u' => {
                basic(it, &mut raw);
                V::U(raw & 0xffff_ffff)
            }
            b't' => {
                basic(it, &mut raw);
                V::U(raw)
            }
            b'n' => {
                basic(it, &mut raw);
                V::I(raw as u16 as i16 as i64)
            }
            b'i' => {
                basic(it, &mut raw);
                V::I(raw as u32 as i32 as i64)
            }
            b'x' => {
                basic(it, &mut raw);
                V::I(raw as i64)
            }
            b'b' => {
                basic(it, &mut raw);
                V::B(raw as u32 != 0)
            }
            b'h' => {
                basic(it, &mut raw);
                V::Fd(raw as u32 as i32)
            }
            b'd' => {
                basic(it, &mut raw);
                V::D(f64::from_bits(raw))
            }
            b'a' | b'e' | b'r' | b'v' => {
                let mut sub = Iter([0; 128]);
                (f.dbus_message_iter_recurse)(it, &mut sub);
                let mut v = Vec::new();
                while let Some(x) = self.read(&mut sub) {
                    v.push(x);
                    if (f.dbus_message_iter_next)(&mut sub) == 0 {
                        break;
                    }
                }
                V::List(v)
            }
            _ => V::List(vec![]),
        })
    }

    unsafe fn args(&self, m: *mut Msg) -> Vec<V> {
        let mut it = Iter([0; 128]);
        let mut out = Vec::new();
        if (self.f.dbus_message_iter_init)(m, &mut it) == 0 {
            return out;
        }
        while let Some(v) = self.read(&mut it) {
            out.push(v);
            if (self.f.dbus_message_iter_next)(&mut it) == 0 {
                break;
            }
        }
        out
    }

    /// Blocking method call with a timeout.
    pub fn call(&self, dest: &str, path: &str, iface: &str, method: &str, args: &[A], timeout: Duration) -> Result<Vec<V>, String> {
        unsafe {
            let (d, p, i, m) = (CString::new(dest).unwrap(), CString::new(path).unwrap(), CString::new(iface).unwrap(), CString::new(method).unwrap());
            let msg = (self.f.dbus_message_new_method_call)(d.as_ptr(), p.as_ptr(), i.as_ptr(), m.as_ptr());
            if msg.is_null() {
                return Err("out of memory".into());
            }
            let mut it = Iter([0; 128]);
            (self.f.dbus_message_iter_init_append)(msg, &mut it);
            for a in args {
                self.append(&mut it, a);
            }
            let mut e = DErr::new();
            (self.f.dbus_error_init)(&mut *e);
            let rep = (self.f.dbus_connection_send_with_reply_and_block)(self.c, msg, timeout.as_millis() as c_int, &mut *e);
            (self.f.dbus_message_unref)(msg);
            if rep.is_null() {
                let m = e.message().unwrap_or_else(|| "no reply".into());
                (self.f.dbus_error_free)(&mut *e);
                return Err(m);
            }
            let v = self.args(rep);
            (self.f.dbus_message_unref)(rep);
            Ok(v)
        }
    }

    /// Wait for a signal iface.member on one of `paths`; returns its args.
    /// Gives up at `timeout` or when `cancel()` returns true.
    pub fn wait_signal(&self, iface: &str, member: &str, paths: &[String], timeout: Duration, cancel: &dyn Fn() -> bool) -> Result<Vec<V>, String> {
        let (i, m) = (CString::new(iface).unwrap(), CString::new(member).unwrap());
        let t0 = Instant::now();
        loop {
            unsafe {
                loop {
                    let msg = (self.f.dbus_connection_pop_message)(self.c);
                    if msg.is_null() {
                        break;
                    }
                    let mut hit = None;
                    if (self.f.dbus_message_is_signal)(msg, i.as_ptr(), m.as_ptr()) != 0 {
                        let p = (self.f.dbus_message_get_path)(msg);
                        let p = if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() };
                        if paths.iter().any(|x| *x == p) {
                            hit = Some(self.args(msg));
                        }
                    }
                    (self.f.dbus_message_unref)(msg);
                    if let Some(a) = hit {
                        return Ok(a);
                    }
                }
                if cancel() {
                    return Err("cancelled".into());
                }
                if t0.elapsed() > timeout {
                    return Err("timed out".into());
                }
                if (self.f.dbus_connection_read_write)(self.c, 200) == 0 {
                    return Err("session bus disconnected".into());
                }
            }
        }
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        unsafe {
            (self.f.dbus_connection_close)(self.c);
            (self.f.dbus_connection_unref)(self.c);
        }
    }
}
