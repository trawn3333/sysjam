#!/usr/bin/env python3
"""Spawns named test processes on a schedule for the audience demo video:
  python3 tools/audience_demo.py 14.2 & tools/record.py mp4 demo.mp4 --args "--seed 11 --style techno" \
      --warm 14 --secs 44 --keys "39:l" --env SYSJAM_SURGE_TEST=1
firefox/cc1/ld are symlinks to sleep, rustc to yes (CPU), rsync to dd (4 KiB synced writes, ~45 MB/s)."""
import os, subprocess, sys, time
d = "/tmp/sjdemo"; os.makedirs(d, exist_ok=True)
for name, target in [("firefox", "/bin/sleep"), ("cc1", "/bin/sleep"), ("ld", "/bin/sleep"), ("rustc", "/usr/bin/yes"), ("rsync", "/bin/dd")]:
    p = os.path.join(d, name)
    if not os.path.lexists(p):
        os.symlink(target, p)
off = float(sys.argv[1])
plan = [  # (t, name, args, lifetime)
    (3.0, "firefox", ["45"], 45),
    (6.0, "cc1", ["5"], 5),
    (8.5, "cc1", ["5"], 5),
    (10.5, "ld", ["4"], 4),
    (14.0, "rustc", [], 22),
    (15.0, "rsync", ["if=/dev/zero", "of=/tmp/sysjam-demo-big", "bs=4k", "count=400000", "oflag=dsync"], 20),
]
t0 = time.time()
procs = []
for t, name, args, life in plan:
    w = t0 + off + t - time.time()
    if w > 0:
        time.sleep(w)
    out = subprocess.DEVNULL
    p = subprocess.Popen([os.path.join(d, name)] + args, stdout=out, stderr=out)
    procs.append((p, time.time() + life))
    print(f"{time.time()-t0-off:6.2f} spawned {name}:{p.pid}", flush=True)
    # reap any that are due
while procs:
    now = time.time()
    for p, until in list(procs):
        if p.poll() is not None or now >= until:
            if p.poll() is None:
                p.kill(); p.wait()
            print(f"{time.time()-t0-off:6.2f} ended {os.path.basename(p.args[0])}:{p.pid}", flush=True)
            procs.remove((p, until))
    time.sleep(0.1)
try:
    os.remove("/tmp/sysjam-demo-big")
except OSError:
    pass
