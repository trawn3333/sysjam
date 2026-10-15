#!/bin/bash
# ~75 s demo: calm (no hiss) -> chaos (load + simulated fast typing) -> calm.
# Typing is simulated through a fake /proc/interrupts (SYSJAM_INTERRUPTS) because
# a headless box has no keyboard; on a desktop the real counters are used.
set -u
cd "$(dirname "$0")/.."
B=${B:-dist/sysjam}; OUT=${OUT:-dist/sample-short.wav}; LEN=75
TMP=$(mktemp -d); IRQ=$TMP/interrupts
pids=()
bg() { "$@" >/dev/null 2>&1 & pids+=($!); }
trap 'kill "${pids[@]}" 2>/dev/null; pkill -f speed.cloudflare; pkill -x stress-ng; rm -rf "$TMP"' EXIT
cat > $TMP/irq.py <<PY
import time, os
k, m, t0 = 0, 0, time.time()
while True:
    t = time.time() - t0
    if 28 <= t < 52: k += 6          # ~6 keystrokes/s = 12 irq/s at 0.5 s ticks
    if 30 <= t < 50: m += 60
    open("$IRQ.tmp", "w").write(f"   CPU0\n  1: {k}  IO-APIC 1-edge i8042\n 12: {m}  IO-APIC 12-edge i8042\n")
    os.replace("$IRQ.tmp", "$IRQ"); time.sleep(0.5)
PY
bg python3 $TMP/irq.py; sleep 0.7
T0=$(date +%s.%N)
SYSJAM_INTERRUPTS=$IRQ $B --wav "$OUT" --duration $LEN --verbose --seed 77 --soundfont assets/TimGM6mb.sf2 --no-screen > $TMP/run.log 2>&1 &
SM=$!
sleep 28; echo "[28s] chaos: stress-ng 8 + downloads + direct writes + fast typing"
bg stress-ng --cpu 8 --timeout 24
bg bash -c 'while :; do curl -s -o /dev/null "https://speed.cloudflare.com/__down?bytes=50000000"; done'
bg bash -c "while :; do dd if=/dev/zero of=$TMP/w bs=1M count=48 oflag=direct status=none; done"
sleep 24; echo "[52s] calm again"
kill "${pids[@]:1}" 2>/dev/null; pkill -f speed.cloudflare; pkill -x stress-ng; pkill -f "of=$TMP/w"
sleep 18
ns=0; for f in /proc/$SM/task/*/schedstat; do ns=$((ns + $(cut -d' ' -f1 "$f"))); done
el=$(echo "$(date +%s.%N) - $T0" | bc)
echo "sysjam CPU during demo: $(echo "scale=2; $ns / 10000000 / $el" | bc)% of one core"
wait $SM; cp $TMP/run.log dist/sample-short-render.log
