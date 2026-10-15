#!/bin/bash
# Render dist/sample.wav (3:00) while load goes idle -> spike -> busy -> idle.
set -u
cd "$(dirname "$0")/.."
B=${B:-dist/sysjam}
OUT=${OUT:-dist/sample.wav}
SEED=${SEED:-2026}
LEN=180
export DISPLAY=${DISPLAY:-:11}
TMP=$(mktemp -d)
pids=()
bg() { "$@" >/dev/null 2>&1 & pids+=($!); }
stopall() { kill "${pids[@]}" 2>/dev/null; pids=(); pkill -f "speed.cloudflare" 2>/dev/null; pkill -f "of=$TMP/w" 2>/dev/null; pkill -x stress-ng 2>/dev/null; }
cleanup() { stopall; pkill -f "display -size" 2>/dev/null; rm -rf "$TMP"; }
trap cleanup EXIT
log() { printf '[demo %5.1fs] %s\n' "$(echo "$(date +%s.%N) - $T0" | bc)" "$*"; }

display -size 1920x1200 xc:'#183070' -geometry 1920x1200+0+0 >/dev/null 2>&1 &   # cool blue screen
sleep 1.5
T0=$(date +%s.%N)
$B --wav "$OUT" --duration $LEN --verbose --seed $SEED --soundfont assets/TimGM6mb.sf2 --screen-interval 4 > "$TMP/run.log" 2>&1 &
SM=$!
log "idle (chilling)"
sleep 32; log "SPIKE: stress-ng 8 CPU workers for 16 s"
bg stress-ng --cpu 8 --timeout 16
sleep 16; log "sustained busy: 4 CPU workers + 'mixer' thread, downloads, direct writes, fd churn"
cat > "$TMP/threads.py" <<'PY'
import threading, ctypes, time
libc = ctypes.CDLL(None)
def work(name, duty):
    libc.prctl(15, name.encode(), 0, 0, 0)
    while True:
        t = time.time()
        while time.time() - t < duty * 0.05: pass
        time.sleep((1 - duty) * 0.05)
threading.Thread(target=work, args=("mixer", 0.9), daemon=True).start()
time.sleep(300)
PY
bg python3 "$TMP/threads.py"
bg stress-ng --cpu 4 --timeout 70
bg bash -c 'while :; do curl -s -o /dev/null "https://speed.cloudflare.com/__down?bytes=50000000"; done'
bg bash -c "while :; do dd if=/dev/zero of=$TMP/w bs=1M count=48 oflag=direct status=none; done"
bg python3 -c "
import time
while True:
    fs=[open('/etc/hostname') for _ in range(400)]; time.sleep(0.15)
    for f in fs: f.close()
    time.sleep(0.15)"
sleep 22; log "screen turns warm orange"
display -size 1920x1200 xc:'#ff8a20' -geometry 1920x1200+0+0 >/dev/null 2>&1 &
sleep 40; log "all load stopped -> subsiding -> chilling"
stopall
sleep 65
ns=0; for f in /proc/$SM/task/*/schedstat; do ns=$((ns + $(cut -d' ' -f1 "$f"))); done
el=$(echo "$(date +%s.%N) - $T0" | bc)
echo "sysjam CPU during demo: $(echo "scale=2; $ns / 10000000 / $el" | bc)% of one core (over ${el}s)"
wait $SM
cp "$TMP/run.log" dist/sample-render.log
