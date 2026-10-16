//! State shared between the (niced) monitor thread and the audio thread.

#[derive(Clone, Debug)]
pub struct LeadInfo {
    pub tid: i32,
    pub name: String,
    pub cpu: f32, // fraction of one core
    pub hash: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Control {
    pub version: u64,
    pub sig: crate::band::Signals,
    pub warmth: f32,     // smoothed screen warmth/brightness score -1..1
    pub brightness: f32, // 0..1
    pub screen_valid: bool,
    pub click_rate: f32,
    pub leads: [Option<LeadInfo>; 3],
    /// the busiest of the user's programs right now (named when a guest sits in), and
    /// whether guests may sit in at all (--no-leads turns them off)
    pub sitin: Option<String>,
    pub leads_ok: bool,
    /// the audience: size and liveliness 0..1, and the last notable process to walk in
    pub crowd_n: f32,
    pub crowd_act: f32,
    pub walkin: Option<String>,
    pub walkin_ver: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ScreenColor {
    pub valid: bool,
    pub warmth: f32,     // -1 (cool/blue) .. +1 (warm/orange)
    pub brightness: f32, // 0..1
    #[allow(dead_code)]
    pub saturation: f32,
    pub hue: f32, // degrees, of average colour
}

pub fn set_thread_nice(n: i32) {
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, n);
    }
}

/// True when the kernel started us in secure mode (setgid/setuid install). Stays true
/// after privileges are dropped; glibc then ignores LD_LIBRARY_PATH and friends.
pub fn secure() -> bool {
    unsafe { libc::getauxval(libc::AT_SECURE) != 0 }
}

/// dlopen a system library by soname, then by absolute path in the standard
/// multiarch dirs (works the same in secure mode, which ignores LD_LIBRARY_PATH).
pub fn dlopen_sys(soname: &str) -> *mut std::ffi::c_void {
    const DIRS: [&str; 6] = ["/usr/lib/x86_64-linux-gnu", "/lib/x86_64-linux-gnu", "/usr/lib64", "/lib64", "/usr/lib", "/lib"];
    let flags = libc::RTLD_NOW | libc::RTLD_LOCAL;
    let c = std::ffi::CString::new(soname).unwrap();
    let h = unsafe { libc::dlopen(c.as_ptr(), flags) };
    if !h.is_null() {
        return h;
    }
    for d in DIRS {
        let p = format!("{d}/{soname}");
        if std::path::Path::new(&p).exists() {
            let c = std::ffi::CString::new(p).unwrap();
            let h = unsafe { libc::dlopen(c.as_ptr(), flags) };
            if !h.is_null() {
                return h;
            }
        }
    }
    std::ptr::null_mut()
}

/// Steering from the keyboard (TUI arrows / SYSJAM_KEYS): net presses, drained by the engine.
pub static KEY_TEMPO: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
pub static KEY_STYLE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
pub static KEY_LIKE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// `n` / Tab: next idea, now (net presses, drained by the engine)
pub static KEY_NEXT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
