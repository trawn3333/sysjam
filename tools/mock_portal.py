#!/usr/bin/env python3
"""Mock org.freedesktop.portal.ScreenCast for testing sysjam.
Behaves like xdg-desktop-portal: Request objects + Response signals,
persist_mode/restore_token, OpenPipeWireRemote returning a unix fd.
Env: MOCK_NODE (PipeWire node id), MOCK_DECLINE=1 (user presses Cancel),
MOCK_STATE (token file), MOCK_LOG (log file)."""
import os, socket, sys, uuid
import dbus, dbus.service, dbus.mainloop.glib
from gi.repository import GLib

dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
BUS = dbus.SessionBus()
NAME = dbus.service.BusName("org.freedesktop.portal.Desktop", BUS)
STATE = os.environ.get("MOCK_STATE", "/tmp/mock-portal-tokens")
LOGF = open(os.environ.get("MOCK_LOG", "/tmp/mock-portal.log"), "a", buffering=1)
def log(*a):
    print("[mock-portal]", *a, file=LOGF)

def tokens():
    try:
        return set(open(STATE).read().split())
    except OSError:
        return set()

class Request(dbus.service.Object):
    def __init__(self, path):
        super().__init__(BUS, path)
    @dbus.service.signal("org.freedesktop.portal.Request", signature="ua{sv}")
    def Response(self, code, results):
        pass
    @dbus.service.method("org.freedesktop.portal.Request")
    def Close(self):
        pass

class Session(dbus.service.Object):
    def __init__(self, path):
        super().__init__(BUS, path)
        self.path = path
        self.opts = {}
    @dbus.service.method("org.freedesktop.portal.Session")
    def Close(self):
        log("session closed", self.path)
        self.remove_from_connection()

SESSIONS = {}

def reply_later(sender, opts, code, results):
    tok = str(opts.get("handle_token", "t" + uuid.uuid4().hex[:6]))
    path = "/org/freedesktop/portal/desktop/request/%s/%s" % (sender[1:].replace(".", "_"), tok)
    req = Request(path)
    def fire():
        req.Response(dbus.UInt32(code), dbus.Dictionary(results, signature="sv"))
        req.remove_from_connection()
        return False
    GLib.timeout_add(int(opts.get("_delay", 50)), fire)
    return dbus.ObjectPath(path)

class Portal(dbus.service.Object):
    def __init__(self):
        super().__init__(BUS, "/org/freedesktop/portal/desktop")

    @dbus.service.method("org.freedesktop.DBus.Properties", in_signature="ss", out_signature="v")
    def Get(self, iface, prop):
        if iface != "org.freedesktop.portal.ScreenCast":
            raise dbus.exceptions.DBusException("no such interface", name="org.freedesktop.DBus.Error.UnknownInterface")
        return {"version": dbus.UInt32(5), "AvailableSourceTypes": dbus.UInt32(7), "AvailableCursorModes": dbus.UInt32(7)}[prop]

    @dbus.service.method("org.freedesktop.portal.ScreenCast", in_signature="a{sv}", out_signature="o", sender_keyword="sender")
    def CreateSession(self, opts, sender=None):
        sp = "/org/freedesktop/portal/desktop/session/%s/%s" % (sender[1:].replace(".", "_"), opts["session_handle_token"])
        SESSIONS[sp] = Session(sp)
        log("CreateSession", sp)
        return reply_later(sender, opts, 0, {"session_handle": dbus.String(sp)})

    @dbus.service.method("org.freedesktop.portal.ScreenCast", in_signature="oa{sv}", out_signature="o", sender_keyword="sender")
    def SelectSources(self, session, opts, sender=None):
        SESSIONS[session].opts = dict(opts)
        log("SelectSources types=%s persist_mode=%s restore_token=%s cursor_mode=%s" % (opts.get("types"), opts.get("persist_mode"), opts.get("restore_token"), opts.get("cursor_mode")))
        return reply_later(sender, opts, 0, {})

    @dbus.service.method("org.freedesktop.portal.ScreenCast", in_signature="osa{sv}", out_signature="o", sender_keyword="sender")
    def Start(self, session, parent, opts, sender=None):
        so = SESSIONS[session].opts
        tok = str(so.get("restore_token", ""))
        if tok and tok in tokens():
            log("Start: restore token valid -> no dialog")
            delay = 50
        else:
            log("Start: DIALOG SHOWN to user (no/invalid restore token)")
            delay = int(os.environ.get("MOCK_DIALOG_MS", "800"))
            if os.environ.get("MOCK_DECLINE") == "1":
                log("Start: user pressed Cancel")
                return reply_later(sender, dict(opts, _delay=delay), 1, {})
        res = {
            "streams": dbus.Array([dbus.Struct((dbus.UInt32(int(os.environ["MOCK_NODE"])),
                                                dbus.Dictionary({"source_type": dbus.UInt32(1), "size": dbus.Struct((dbus.Int32(320), dbus.Int32(180)))}, signature="sv")))],
                                  signature="(ua{sv})"),
        }
        if int(so.get("persist_mode", 0)) == 2:
            new = uuid.uuid4().hex
            with open(STATE, "a") as f:
                f.write(new + "\n")
            res["restore_token"] = dbus.String(new)
            log("Start: issued restore_token", new)
        return reply_later(sender, dict(opts, _delay=delay), 0, res)

    @dbus.service.method("org.freedesktop.portal.ScreenCast", in_signature="oa{sv}", out_signature="h")
    def OpenPipeWireRemote(self, session, opts):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(os.path.join(os.environ["XDG_RUNTIME_DIR"], "pipewire-0"))
        log("OpenPipeWireRemote -> fd")
        fd = dbus.types.UnixFd(s)
        s.close()
        return fd

Portal()
log("ready, node", os.environ.get("MOCK_NODE"))
GLib.MainLoop().run()
