//! Output sinks. ALSA is loaded at runtime with dlopen("libasound.so.2"),
//! so the binary has no link-time audio dependency and can fall back cleanly.

use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CStr, CString};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::time::{Duration, Instant};

type PcmOpen = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int;
type SetParams = unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
type Writei = unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long;
type Recover = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int;
type Simple = unsafe extern "C" fn(*mut c_void) -> c_int;
type StrErr = unsafe extern "C" fn(c_int) -> *const c_char;
type Delay = unsafe extern "C" fn(*mut c_void, *mut c_long) -> c_int;

pub struct Alsa {
    pcm: *mut c_void,
    writei: Writei,
    recover: Recover,
    drain: Simple,
    close: Simple,
    delay: Option<Delay>,
    rate: u32,
}

unsafe fn sym<T: Copy>(h: *mut c_void, name: &str) -> Result<T, String> {
    let c = CString::new(name).unwrap();
    let p = libc::dlsym(h, c.as_ptr());
    if p.is_null() {
        return Err(format!("libasound missing symbol {name}"));
    }
    Ok(std::mem::transmute_copy(&p))
}

impl Alsa {
    pub fn open(device: &str, rate: u32, latency_us: u32) -> Result<Alsa, String> {
        unsafe {
            let h = crate::shared::dlopen_sys("libasound.so.2");
            if h.is_null() {
                return Err("libasound.so.2 not found (install libasound2 / alsa-lib)".into());
            }
            let open: PcmOpen = sym(h, "snd_pcm_open")?;
            let set: SetParams = sym(h, "snd_pcm_set_params")?;
            let strerr: StrErr = sym(h, "snd_strerror")?;
            let mut pcm: *mut c_void = std::ptr::null_mut();
            let dev = CString::new(device).unwrap();
            let e = open(&mut pcm, dev.as_ptr(), 0 /* PLAYBACK */, 0);
            let err = |e: c_int| CStr::from_ptr(strerr(e)).to_string_lossy().into_owned();
            if e < 0 {
                return Err(format!("cannot open ALSA device '{device}': {}", err(e)));
            }
            // S16_LE = 2, RW_INTERLEAVED = 3, 2 ch, soft resample on
            let e = set(pcm, 2, 3, 2, rate, 1, latency_us);
            let close: Simple = sym(h, "snd_pcm_close")?;
            if e < 0 {
                close(pcm);
                return Err(format!("cannot configure ALSA device '{device}': {}", err(e)));
            }
            Ok(Alsa { pcm, writei: sym(h, "snd_pcm_writei")?, recover: sym(h, "snd_pcm_recover")?, drain: sym(h, "snd_pcm_drain")?, close, delay: sym(h, "snd_pcm_delay").ok(), rate })
        }
    }
}

pub enum Sink {
    Alsa(Alsa),
    /// Paced to real time, discards audio (for testing / measuring).
    Null { start: Instant, frames: u64, rate: u32 },
    /// Paced to real time, writes 16-bit stereo WAV.
    Wav { f: File, start: Instant, frames: u64, rate: u32 },
}

impl Sink {
    pub fn null(rate: u32) -> Sink {
        Sink::Null { start: Instant::now(), frames: 0, rate }
    }
    pub fn wav(path: &str, rate: u32) -> Result<Sink, String> {
        let mut f = File::create(path).map_err(|e| format!("{path}: {e}"))?;
        f.write_all(&[0u8; 44]).map_err(|e| e.to_string())?;
        Ok(Sink::Wav { f, start: Instant::now(), frames: 0, rate })
    }

    /// seconds until the last written frame is heard (output latency right now)
    pub fn delay_s(&self) -> f64 {
        match self {
            Sink::Alsa(a) => {
                let mut d: c_long = 0;
                match a.delay {
                    Some(f) if unsafe { f(a.pcm, &mut d) } == 0 => d.max(0) as f64 / a.rate as f64,
                    _ => 0.12,
                }
            }
            Sink::Null { start, frames, rate } | Sink::Wav { start, frames, rate, .. } => *frames as f64 / *rate as f64 - start.elapsed().as_secs_f64(),
        }
    }

    pub fn write(&mut self, pcm: &[i16]) -> Result<(), String> {
        let n = (pcm.len() / 2) as u64;
        match self {
            Sink::Alsa(a) => {
                let mut off = 0usize;
                while off < pcm.len() {
                    let left = ((pcm.len() - off) / 2) as c_ulong;
                    let r = unsafe { (a.writei)(a.pcm, pcm[off..].as_ptr() as *const c_void, left) };
                    if r < 0 {
                        let rr = unsafe { (a.recover)(a.pcm, r as c_int, 1) };
                        if rr < 0 {
                            return Err(format!("ALSA write error {r}"));
                        }
                    } else {
                        off += r as usize * 2;
                    }
                }
                Ok(())
            }
            Sink::Null { start, frames, rate } => {
                *frames += n;
                pace(*start, *frames, *rate);
                Ok(())
            }
            Sink::Wav { f, start, frames, rate } => {
                let mut b = Vec::with_capacity(pcm.len() * 2);
                for s in pcm {
                    b.extend_from_slice(&s.to_le_bytes());
                }
                f.write_all(&b).map_err(|e| e.to_string())?;
                *frames += n;
                pace(*start, *frames, *rate);
                Ok(())
            }
        }
    }

    pub fn finish(self) {
        match self {
            Sink::Alsa(a) => unsafe {
                (a.drain)(a.pcm);
                (a.close)(a.pcm);
            },
            Sink::Null { .. } => {}
            Sink::Wav { mut f, frames, rate, .. } => {
                let data = (frames * 4) as u32;
                let mut h = Vec::with_capacity(44);
                h.extend_from_slice(b"RIFF");
                h.extend_from_slice(&(36 + data).to_le_bytes());
                h.extend_from_slice(b"WAVEfmt ");
                h.extend_from_slice(&16u32.to_le_bytes());
                h.extend_from_slice(&1u16.to_le_bytes());
                h.extend_from_slice(&2u16.to_le_bytes());
                h.extend_from_slice(&rate.to_le_bytes());
                h.extend_from_slice(&(rate * 4).to_le_bytes());
                h.extend_from_slice(&4u16.to_le_bytes());
                h.extend_from_slice(&16u16.to_le_bytes());
                h.extend_from_slice(b"data");
                h.extend_from_slice(&data.to_le_bytes());
                let _ = f.seek(SeekFrom::Start(0));
                let _ = f.write_all(&h);
            }
        }
    }
}

fn pace(start: Instant, frames: u64, rate: u32) {
    let due = Duration::from_secs_f64(frames as f64 / rate as f64);
    let el = start.elapsed();
    // keep ~50 ms ahead like a real device buffer would
    if due > el + Duration::from_millis(80) {
        std::thread::sleep(due - el - Duration::from_millis(80));
    }
}
