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
