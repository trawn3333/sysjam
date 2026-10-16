#!/usr/bin/env python3
"""Record the TUI from tmux as timed frames, then make a GIF or an MP4 (with the run's audio).

  tools/record.py gif  docs/anim-country.gif --args "--seed 3 --style country" --warm 40 --secs 8
  tools/record.py mp4  demo.mp4 --args "--seed 3 --style country" --warm 10 --secs 45 \\
                       --keys "8:Up,14:Left,20:l,28:Right"

Frames are tmux captures (-p -e) rendered with ansi2png. For mp4 the run renders to a WAV in
real time (--wav) and writes the wall clock of its sample 0 (SYSJAM_T0FILE), so every frame is
placed at its true time against the audio. Needs tmux, PIL, fontTools, ffmpeg (mp4).
LOAD=N runs synthetic CPU/disk load for N seconds so the band wakes up."""
import argparse, os, subprocess, sys, time, tempfile
sys.path.insert(0, os.path.dirname(__file__))
from ansi2png import render

ap = argparse.ArgumentParser()
ap.add_argument("kind", choices=["gif", "mp4"])
ap.add_argument("out")
ap.add_argument("--args", default="")
ap.add_argument("--warm", type=float, default=30)
ap.add_argument("--secs", type=float, default=8)
ap.add_argument("--fps", type=float, default=12)
ap.add_argument("--keys", default="", help="t:key,... seconds after capture start, tmux key names")
ap.add_argument("--crop", default="", help="rows a:b of the terminal to keep (gif)")
ap.add_argument("--scale", type=float, default=1.0)
ap.add_argument("--env", default="")
a = ap.parse_args()

work = tempfile.mkdtemp(prefix="sysjam-rec-")
S = "rec"
subprocess.run(["tmux", "kill-session", "-t", S], stderr=subprocess.DEVNULL)
wav = os.path.join(work, "run.wav"); t0f = os.path.join(work, "t0")
total = a.warm + a.secs + 4
sink = f"--wav {wav} --duration {total:.0f}" if a.kind == "mp4" else "--null"
cmd = (f"env -u NO_COLOR TERM=xterm-256color COLORTERM=truecolor SYSJAM_T0FILE={t0f} {a.env} "
       f"./target/release/sysjam {sink} {a.args}; sleep 30")
load = []
if os.environ.get("LOAD"):
    n = os.environ["LOAD"]
    for _ in range(4):
        load.append(subprocess.Popen(["timeout", n, "sh", "-c", "while :; do :; done"], start_new_session=True))
subprocess.run(["tmux", "-f", "/dev/null", "new-session", "-d", "-s", S, "-x", "140", "-y", "46", cmd], check=True)
time.sleep(a.warm)
keys = sorted((float(t), k) for t, k in (e.split(":", 1) for e in a.keys.split(",") if e))
frames = []
start = time.time()
dt = 1.0 / a.fps
i = 0
while True:
    now = time.time() - start
    if now >= a.secs:
        break
    while keys and keys[0][0] <= now:
        subprocess.run(["tmux", "send-keys", "-t", S, keys.pop(0)[1]])
    t = time.time()
    txt = subprocess.run(["tmux", "capture-pane", "-t", S, "-p", "-e", "-N"], capture_output=True, text=True).stdout
    frames.append((t, txt))
    i += 1
    nxt = start + i * dt
    if nxt > time.time():
        time.sleep(nxt - time.time())
subprocess.run(["tmux", "send-keys", "-t", S, "q"]); time.sleep(0.4)
subprocess.run(["tmux", "send-keys", "-t", S, "q"]); time.sleep(1.5)
def wav_ok():
    try:
        with open(wav, "rb") as f:
            return f.read(4) == b"RIFF"
    except OSError:
        return False
for _ in range(40):  # let the WAV header be finalised before the session goes
    if a.kind != "mp4" or wav_ok():
        break
    time.sleep(0.25)
subprocess.run(["tmux", "kill-session", "-t", S], stderr=subprocess.DEVNULL)
import signal
for p in load:
    try:
        os.killpg(p.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
print(f"captured {len(frames)} frames in {a.secs:.1f}s; work dir {work}", file=sys.stderr)
with open(os.path.join(work, "frames.ans"), "w") as f:
    for t, txt in frames:
        f.write(f"@@FRAME {t - frames[0][0]:.3f}\n{txt}\n")

imgs = []
r0, r1 = (int(x) for x in a.crop.split(":")) if a.crop else (0, 46)
for t, txt in frames:
    lines = txt.split("\n")[r0:r1]
    im = render("\n".join(lines), 140, r1 - r0)
    if a.scale != 1.0:
        im = im.resize((int(im.width * a.scale) // 2 * 2, int(im.height * a.scale) // 2 * 2))
    imgs.append(im)

if a.kind == "gif":
    durs = [int(round((frames[k + 1][0] - frames[k][0]) * 1000)) for k in range(len(frames) - 1)] + [int(dt * 1000)]
    # one shared 48-colour palette: frames then differ only where something moved
    shared = imgs[len(imgs) // 2].quantize(colors=48)
    from PIL import Image
    pal = [im.quantize(palette=shared, dither=Image.Dither.NONE) for im in imgs]
    pal[0].save(a.out, save_all=True, append_images=pal[1:], duration=durs, loop=0, optimize=True, disposal=1)
    print(f"{a.out}: {len(imgs)} frames, {sum(durs)/1000:.1f}s, {os.path.getsize(a.out)/1024:.0f} KiB")
else:
    t0 = int(open(t0f).read()) / 1e9
    # resample the captured frames onto a constant 15 fps grid (each output frame shows the
    # latest capture at or before its time), then mux with the WAV from that same instant
    for k, im in enumerate(imgs):
        im.save(os.path.join(work, f"f{k:05d}.png"))
    off = frames[0][0] - t0  # audio second shown by the first frame
    dur = frames[-1][0] - frames[0][0] + dt
    ofps = 15
    k = 0
    for j in range(int(dur * ofps)):
        while k + 1 < len(frames) and frames[k + 1][0] - frames[0][0] <= j / ofps:
            k += 1
        dst = os.path.join(work, f"s{j:05d}.png")
        if os.path.lexists(dst):
            os.remove(dst)
        os.symlink(os.path.join(work, f"f{k:05d}.png"), dst)
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", str(ofps), "-i", os.path.join(work, "s%05d.png"),
                    *([] if wav_ok() else ["-f", "s16le", "-ar", "44100", "-ac", "2"]),
                    "-ss", f"{off:.3f}", "-t", f"{dur:.3f}", "-i", wav,
                    "-vf", "format=yuv420p", "-c:v", "libx264", "-preset", "slow", "-crf", "26", "-tune", "animation",
                    "-c:a", "aac", "-b:a", "160k", "-shortest", "-movflags", "+faststart", a.out], check=True)
    print(f"{a.out}: {dur:.1f}s, audio from {off:.2f}s of the run, {os.path.getsize(a.out)/1e6:.1f} MB")
