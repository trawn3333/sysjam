//! xdg-desktop-portal ScreenCast: ask once for a monitor, keep a restore token
//! (persist_mode = 2) so later runs start without a prompt, then hand the
//! PipeWire fd + node id to pwcap.

use crate::dbus::{Bus, A, DV, V};
use std::path::PathBuf;
use std::time::Duration;

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SC: &str = "org.freedesktop.portal.ScreenCast";
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// the user may need a while to answer the first-run dialog
const PROMPT_TIMEOUT: Duration = Duration::from_secs(90);

pub fn config_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|x| !x.is_empty()) {
        return Some(PathBuf::from(x).join("sysjam"));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/sysjam"))
}
fn token_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("portal-restore-token"))
}
pub fn declined_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("portal-declined"))
}
fn load_token() -> Option<String> {
    let t = std::fs::read_to_string(token_path()?).ok()?;
    let t = t.trim().to_string();
    (!t.is_empty() && t.len() < 512).then_some(t)
}
fn write_private(p: &PathBuf, s: &str) {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(p) {
        use std::io::Write;
        let _ = f.write_all(s.as_bytes());
    }
}

pub enum Fail {
    /// no portal / no ScreenCast backend: try the next method quietly
    Unavailable(String),
    /// the user said no
    Declined,
    /// anything else (timeouts, errors)
    Error(String),
}

pub struct Session {
    bus: Bus,
    handle: String,
    pub fd: i32,
    pub node: u32,
    pub prompted: bool,
}

impl Session {
    pub fn close(self) {
        let _ = self.bus.call(DEST, &self.handle, "org.freedesktop.portal.Session", "Close", &[], Duration::from_secs(2));
    }
}

struct Req<'a> {
    bus: &'a Bus,
    sender: String,
    n: u32,
}
impl<'a> Req<'a> {
    /// Call a portal method that answers through a Request object's Response signal.
    fn call(&mut self, method: &str, mut args: Vec<A>, timeout: Duration, cancel: &dyn Fn() -> bool) -> Result<(u64, V), Fail> {
        self.n += 1;
        let token = format!("sysjam{}_{}", std::process::id(), self.n);
        let path = format!("{PATH}/request/{}/{token}", self.sender);
        self.bus
            .add_match(&format!("type='signal',interface='org.freedesktop.portal.Request',member='Response',path='{path}'"))
            .map_err(Fail::Error)?;
        if let Some(A::Dict(d)) = args.last_mut() {
            d.push(("handle_token", DV::S(token)));
        }
        let r = self.bus.call(DEST, PATH, SC, method, &args, CALL_TIMEOUT).map_err(|e| {
            if e.contains("UnknownMethod") || e.contains("ServiceUnknown") || e.contains("UnknownInterface") || e.contains("NoReply") && method == "CreateSession" {
                Fail::Unavailable(e)
            } else {
                Fail::Error(format!("{method}: {e}"))
            }
        })?;
        let mut paths = vec![path];
        if let Some(p) = r.first().and_then(|v| v.s()) {
            if p != paths[0] {
                // pre-0.9 portals return a different request path
                let _ = self.bus.add_match(&format!("type='signal',interface='org.freedesktop.portal.Request',member='Response',path='{p}'"));
                paths.push(p.to_string());
            }
        }
        let a = self
            .bus
            .wait_signal("org.freedesktop.portal.Request", "Response", &paths, timeout, cancel)
            .map_err(|e| Fail::Error(format!("{method}: {e}")))?;
        let code = a.first().and_then(|v| v.u()).unwrap_or(2);
        Ok((code, a.get(1).cloned().unwrap_or(V::List(vec![]))))
    }
}

/// Run the portal handshake. `cancel` aborts promptly (shutdown).
pub fn open(cancel: &dyn Fn() -> bool, verbose: bool) -> Result<Session, Fail> {
    let bus = Bus::session().map_err(Fail::Unavailable)?;
    // version / source types (also tells us whether a backend exists)
    let prop = |name: &str| {
        bus.call(DEST, PATH, "org.freedesktop.DBus.Properties", "Get", &[A::S(SC), A::S(name)], CALL_TIMEOUT)
            .ok()
            .and_then(|v| v.first().and_then(|x| x.u()))
    };
    let version = match bus.call(DEST, PATH, "org.freedesktop.DBus.Properties", "Get", &[A::S(SC), A::S("version")], CALL_TIMEOUT) {
        Ok(v) => v.first().and_then(|x| x.u()).unwrap_or(1),
        Err(e) => return Err(Fail::Unavailable(e)),
    };
    let types = prop("AvailableSourceTypes").unwrap_or(1);
    if types & 1 == 0 {
        return Err(Fail::Unavailable("ScreenCast backend offers no monitor sources".into()));
    }
    let sender = bus.unique_name().trim_start_matches(':').replace('.', "_");
    let mut rq = Req { bus: &bus, sender, n: 0 };
    let mut token = if version >= 4 { load_token() } else { None };

    for attempt in 0..2 {
        let (code, res) = rq.call("CreateSession", vec![A::Dict(vec![("session_handle_token", DV::S(format!("sysjam{}", std::process::id())))])], Duration::from_secs(10), cancel)?;
        if code != 0 {
            return Err(Fail::Error(format!("CreateSession refused ({code})")));
        }
        let handle = res.get("session_handle").and_then(|v| v.s()).map(String::from).ok_or_else(|| Fail::Error("CreateSession: no session handle".into()))?;
        let mut opts = vec![("types", DV::U(1)), ("multiple", DV::B(false))];
        if version >= 4 {
            opts.push(("persist_mode", DV::U(2)));
            if let Some(t) = &token {
                opts.push(("restore_token", DV::S(t.clone())));
            }
        }
        if version >= 2 {
            // hidden cursor (1) if offered
            if prop("AvailableCursorModes").map(|m| m & 1 != 0).unwrap_or(false) {
                opts.push(("cursor_mode", DV::U(1)));
            }
        }
        let close = |h: &str| {
            let _ = bus.call(DEST, h, "org.freedesktop.portal.Session", "Close", &[], Duration::from_secs(2));
        };
        let (code, _) = rq.call("SelectSources", vec![A::O(&handle), A::Dict(opts)], Duration::from_secs(10), cancel).inspect_err(|_| close(&handle))?;
        if code != 0 {
            close(&handle);
            return Err(if code == 1 { Fail::Declined } else { Fail::Error(format!("SelectSources refused ({code})")) });
        }
        if verbose {
            say!("screen colour: portal {} (ScreenCast v{version})", if token.is_some() { "restoring previous permission" } else { "asking for permission; pick a screen in the dialog" });
        }
        let (code, res) = match rq.call("Start", vec![A::O(&handle), A::S(""), A::Dict(vec![])], PROMPT_TIMEOUT, cancel) {
            Ok(x) => x,
            Err(e) => {
                close(&handle);
                return Err(match e {
                    Fail::Error(m) if m.ends_with("timed out") => Fail::Error("no answer to the screen-share prompt".into()),
                    e => e,
                });
            }
        };
        if code == 1 {
            close(&handle);
            return Err(Fail::Declined);
        }
        if code != 0 {
            close(&handle);
            if token.is_some() && attempt == 0 {
                // stale token (monitor gone, portal reset): forget it and ask once
                token = None;
                if let Some(p) = token_path() {
                    let _ = std::fs::remove_file(p);
                }
                continue;
            }
            return Err(Fail::Error(format!("Start refused ({code})")));
        }
        if let Some(t) = res.get("restore_token").and_then(|v| v.s()) {
            if let Some(p) = token_path() {
                write_private(&p, t);
            }
        }
        // streams: a(ua{sv}) inside a variant
        let node = res.get("streams").and_then(|v| first_node(v)).ok_or_else(|| {
            close(&handle);
            Fail::Error("portal returned no stream".into())
        })?;
        let r = bus.call(DEST, PATH, SC, "OpenPipeWireRemote", &[A::O(&handle), A::Dict(vec![])], CALL_TIMEOUT).map_err(|e| {
            close(&handle);
            Fail::Error(format!("OpenPipeWireRemote: {e}"))
        })?;
        let fd = match r.first() {
            Some(V::Fd(fd)) if *fd >= 0 => *fd,
            _ => {
                close(&handle);
                return Err(Fail::Error("OpenPipeWireRemote returned no fd".into()));
            }
        };
        if let Some(p) = declined_path() {
            let _ = std::fs::remove_file(p);
        }
        return Ok(Session { bus, handle, fd, node, prompted: token.is_none() });
    }
    Err(Fail::Error("portal session failed".into()))
}

fn first_node(v: &V) -> Option<u32> {
    // descend until we find a list whose first element is the u32 node id
    match v {
        V::List(l) => {
            if l.len() == 2 {
                if let (V::U(n), V::List(_)) = (&l[0], &l[1]) {
                    return Some(*n as u32);
                }
            }
            l.iter().find_map(first_node)
        }
        _ => None,
    }
}

pub fn mark_declined() {
    if let Some(p) = declined_path() {
        write_private(&p, "declined\n");
    }
}
