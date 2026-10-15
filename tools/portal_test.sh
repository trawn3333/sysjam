#!/bin/bash
# End-to-end test of the portal + PipeWire path with a mock portal.
# Runs a private session bus, pipewire, wireplumber and a GStreamer test
# source; no real desktop needed.  Usage: tools/portal_test.sh BIN COLOR
set -u
BIN=${1:-dist/sysjam}; COLOR=${2:-0xffff8000}
export XDG_RUNTIME_DIR=/tmp/pwtest-rt XDG_CONFIG_HOME=${XDG_CONFIG_HOME:-/tmp/pwtest-cfg}
mkdir -p $XDG_RUNTIME_DIR $XDG_CONFIG_HOME; chmod 700 $XDG_RUNTIME_DIR
exec dbus-run-session -- bash -c '
  pipewire >/tmp/pwtest-pw.log 2>&1 & sleep 1
  wireplumber >/tmp/pwtest-wp.log 2>&1 & sleep 2
  gst-launch-1.0 -q videotestsrc pattern=solid-color foreground-color='$COLOR' is-live=true ! video/x-raw,format=BGRx,width=320,height=180,framerate=5/1 ! pipewiresink mode=provide stream-properties="props,node.name=mockscreen,media.class=Video/Source" >/tmp/pwtest-gst.log 2>&1 &
  sleep 2
  NODE=$(pw-dump 2>/dev/null | python3 -c "import json,sys; print([o[\"id\"] for o in json.load(sys.stdin) if o.get(\"type\",\"\").endswith(\"Node\") and o.get(\"info\",{}).get(\"props\",{}).get(\"node.name\")==\"mockscreen\"][0])")
  echo "mock screen node: $NODE"
  MOCK_NODE=$NODE python3 tools/mock_portal.py & sleep 1
  shift; WAYLAND_DISPLAY=wayland-mock DISPLAY= "$@"
  echo "exit: $?"
  kill %1 %2 %3 %4 2>/dev/null; wait 2>/dev/null
' _ "$BIN" "${@:3}"
