# sysjam

A jam band of machine musicians playing your Linux system. Six autonomous
players (drone, bassist, drummer, keys, lead, colour/FX) listen to CPU,
cores, network, disk writes, file activity and screen colour, and to each other.
A loose bandleader moves the arrangement through **chilling → awakening →
alive → subsiding**. Chords keep moving even on an idle machine. Every run
starts in a random key (any of the 12 roots, major or minor) at a random tempo
(60–100 bpm). The tempo then breathes with the arrangement and the key wanders
around the circle of fifths. Players pick up a new General MIDI instrument or
design a new synth patch whenever something inspires them. Under everything sits
a continuous deep sub drone and bass. No two runs are the same unless you pass `--seed`.

(Formerly developed under the working name *sysmusic*.) Listen: [`docs/sample.mp3`](docs/sample.mp3) is a 75 s demo at 192 kbps. It starts calm with no hiss, then hits chaos (CPU + network + disk load plus simulated fast typing), where a short burst of industrial noise comes in, then settles again. Its verbose log is `docs/sample-render.log`. [`docs/sample-tempo.mp3`](docs/sample-tempo.mp3) is a 90 s demo (seed 5, made with `tools/demo_tempo.sh`). It starts in A major at 84 bpm, and the tempo eases down to 73 while the band is chilling. Under load (from 18 s) the band comes alive and ramps toward 97 bpm. At about 60 s a **surge** opens the throttle from 85 to 133 bpm with the V-twin lope, while the band modulates to D minor. The surge winds down to 109 bpm at about 75 s, and the band then subsides. Its log is `docs/sample-tempo-render.log`.

![sysjam's live terminal UI: the colour player quotes "The New Highland Laddie" (Aird's Airs) while the keys duck under it](docs/tui.png)

One static-ish binary (`sysjam`, ~1.7 MB, x86_64, needs glibc ≥ 2.29). It
links only libc, libm, libpthread and libdl. ALSA, D-Bus and PipeWire are
loaded at run time with `dlopen`, so it starts even when one of them is missing.

## Run

```bash
./sysjam                         # play with the live terminal UI (q fades out; twice = quit now)
./sysjam --no-tui                # plain log output instead of the UI
./sysjam --no-tui --verbose      # plain log of everything: chords, players, instruments, metrics
./sysjam --volume 0.4 --sub 1.5  # quieter overall, more sub
./sysjam --seed 42               # reproducible run
./sysjam --wav out.wav --duration 120   # render to a file (real time)
```

| Option | Meaning |
|---|---|
| `--volume V` | master 0–1 (default 0.6) |
| `--sub L` | sub-drone level 0–2 (default 1.2). The sub is never high-passed; a gentle limiter guards the output |
| `--scale minor\|major`, `--root NOTE` | home key. Default: random each run (all 12 roots, major or minor, root in D#2–D3). Given flags are used exactly. Older scale names still work: dorian etc. map to minor, lydian etc. to major |
| `--bpm N` | lock the tempo at N. Default: a random start of 60–100 bpm, after which the bandleader moves it |
| `--chord-bars N` | base bars per chord (default 2). Chilling stretches this, peaks tighten it |
| `--seed N` | repeat a run exactly (same input → same music) |
| `--soundfont PATH`, `--no-gm`, `--gm-layers N` | General MIDI layer (see below); N = 1–4 simultaneous layers (default 3, +1 at peaks) |
| `--clicks L` | file open/close click texture 0–1 (default 0.25) |
| `--no-leads` | never let a busy thread sit in as a guest soloist |
| `--no-input` | don't watch keyboard/mouse activity (see below) |
| `--no-screen`, `--screen-method auto\|portal\|grim\|x11\|cmd`, `--screen-cmd CMD`, `--screen-interval S` | screen colour (see below) |
| `--device NAME` | ALSA device (`default`, `pipewire`, `pulse`, `hw:0`) |
| `--no-tui` | plain log lines instead of the live terminal UI (the UI is the default when stdout is a terminal) |
| `--null`, `--duration S`, `--verbose` | testing / logging (`--verbose` adds metrics lines with `--no-tui`) |

## Live terminal UI

When stdout is a terminal, sysjam opens a full-screen terminal UI (ratatui +
crossterm) that shows how the band is thinking. Piped or redirected output, or
`--no-tui`, gives the plain log instead.

- **Header**: key, the chord progression with roman numerals (past chords dim,
  the current chord highlighted, the planned ones after it), the home key when the
  band is away from it, e.g. `D minor (home A major)`, the tempo with an arrow while it is
  ramping (`105→98 bpm`; `(locked)` with `--bpm`), any half-time, double-time or `surge — V-twin lope` feel, bar and beat,
  the arrangement state (chilling / awakening / alive / subsiding) and the
  bandleader's energy and agitation meters. Top right: the input source
  (`evdev×2`, `evdev×2 setgid`, `usb-hid`, `irq i8042`, `irq xhci≈`, `none`) with live keys and mouse meters.
  The third line is the persistent **`♫ now quoting`** line: while a player quotes a
  theme it shows a progress meter, the player and the full citation, e.g.
  `♫ now quoting ██▍ lead: Wachet auf, ruft uns die Stimme (J.S. Bach, BWV 140.7) · music21 corpus (Bach chorales) · public domain — a straight quote`.
- **The band**: one panel per member: drone, bassist, drummer, keys, lead,
  colour/FX, plus the guest voice when a busy thread sits in. Each panel shows:
  - the member's status: laying out, holding, comping, following, soloing, trading, grooving (drums),
    stretching (a synth voice coming forward while the drums are out) or quoting;
  - an activity meter and the current GM instrument or synth patch;
  - a "thinking" line explaining the latest decision, e.g. `heard a network burst (98%) → adding shimmer`,
    `resolving the motif to the root (A4), then resting`,
    `kicks follow the bassist's syncopations`, or
    `thread make at 100% cpu sits in → long E5`, or why it picked up a new sound,
    e.g. `inspired by the modulation to D minor → picking up a fifths lead`;
  - the notes it played in the last few seconds, plus the machine signal it is tied
    to (`⇢ net 98%`) and its personality (`curious 0.4 eager 0.5 busy 0.6`).
- **Conductor** (top right): a little figure that conducts in time with the beat.
  Its pose changes with what it's doing:
  - tracing a 4/4 pattern with the baton while the band plays;
  - sitting on a stool (`z Z`) while the band is chilling and the drone carries it;
  - `shh…` with a finger to its lips when it hushes someone;
  - pointing when it cues a player or invites one back;
  - both arms up during a surge.

  Its speech line shows the current instruction, e.g. `“keys, bring it down”`, with
  the addressed player and a clutter meter beneath. The addressed player's panel
  lights up yellow for a few seconds, and its thinking line shows the exchange:
  `conductor: keys, bring it down → “ok, quieter”`. The panel title counts the
  interventions so far.
- **Quoted themes** (right, under the conductor): a scrolling history of the recent
  citations with their time, newest highlighted.
- **Machine**: CPU, busy cores, network, disk writes, file churn, keyboard and
  mouse rates, each with a sparkline of the last two minutes. Below them sit a
  swatch of the sampled screen colour and the busiest threads.
- **Levels**: sub (< 60 Hz), bass, mid and high meters of the output.
- **Band events**: a rate-limited log of instrument changes, band moves,
  modulations, guests, and the conductor's instructions with the players' replies.

Keys:

| key | action |
|---|---|
| `q`, `Esc`, `Ctrl-C` | fade out and quit (press again to quit immediately) |
| `space` | pause / resume the display (the music keeps playing) |

The UI runs in its own thread at nice 5 and redraws at 5 fps. The audio thread
publishes a small snapshot about every 100 ms with a non-blocking `try_lock`, so
a slow or stalled terminal can never hold up the audio. The terminal is restored
on exit and also when sysjam panics. Colours follow `NO_COLOR`. The UI needs a
terminal of about 100×30 or larger; 130×36 is comfortable.

## The band

Each player has a personality, drawn from the seed: eagerness, busyness,
swing, how often it replies, and register. On every bar each player decides
whether to **lay out, comp, follow, solo or trade**. The decision depends on
the bandleader's invitation, the shared energy and what the others are doing.

| Player | Plays | Listens to |
|---|---|---|
| **Drone / landscape** | Continuous sub drone (detuned sine pair at 30–60 Hz plus 2nd/3rd harmonics, gliding to each chord root), and a voice-led detuned pad (low-mids). Never lays out. There is no constant hiss/air bed | energy, screen brightness |
| **Bassist** | Roots and fifths at 40–120 Hz, in three styles: long notes, pulsing roots, or a syncopated groove. Plays approach notes into the next chord. Heavy file/disk activity makes it busier | file + disk writes, the drummer |
| **Drummer** | Synth kick, plus the GM kit (channel 10) when a SoundFont is loaded. **Minimal by default**: decides every phrase to rest, play a minimal part (soft downbeat and a tick or shaker pair) or groove — groove mostly when alive, always in surges. Locks onto the bassist's syncopations, occasional fills before section changes, crashes on entries | CPU, cores, the bassist |
| **Keys** | GM pads hold chord tones (common tones are kept between chords). Keys/plucks play arps, broken chords or offbeat comping, and take solos when invited | energy, the lead (trading) |
| **Lead / melody** | Sparse two-bar call phrases built from a motif it develops (repeat, sequence, invert, vary, fragment, new idea). Rests between phrases, and resolves the melody before laying out | agitation, colour player, keys |
| **Colour / FX** | Bells, shimmer clouds driven by network traffic, swells, odd GM colours (FX Atmosphere, Seashore, …). Answers the lead's phrases. **Industrial texture:** only when the band hears real chaos, i.e. agitation above 0.68 for 6 s, or alive with 3+ hot signals at once, or a rare deliberate colour move while alive. It is 1–3 bars of gated, rhythmic band-passed noise with a metallic resonance (peak ≈ 0.018, about −12 dB under the old continuous air bed and gated, so it averages far lower). It fades out as soon as things calm down, then waits 25 s before it can come back | network, the lead, chaos |
| *Guest* | A thread using >40% of a core for >10 s may sit in as a single quiet guest voice. It holds for ≥25 s, is released 10 s after it drops below 25%, then a 20 s cooldown applies. Compositor, audio-server and sysjam threads are ignored | that thread |

**Restraint: play only when there's something to add.** Players are calm by
default. At every phrase, keys, lead and colour each decide to **play**, **hold**
a single sustained tone, or **rest**. The decision weighs:
- the arrangement state (chilling leans hard towards resting);
- the player's tied signal, and whether it just changed;
- how dense the rest of the band already is;
- novelty: a freshly picked-up sound counts in favour, while playing many phrases in a row ("fatigue") and doubling another player count against.

The reason shows in the thinking line, e.g. `band is busy enough (9 notes/bar) → nothing useful to add, resting`,
`holding one long chord tone under the drone`, or `my signal is up (net 92%) → worth playing`.

In **chilling**, most players rest or hold one long tone, and the drone, pad and deep bass carry it.

Patterns are sparser overall:
- keys: mostly sparse stabs; arpeggios in quarters, eighths only when alive;
- lead: one bar of call per phrase, two when alive;
- colour: a few shimmer bells per bar at most;
- drone: rarer landscape bells;
- drums: see below — rest or minimal unless alive; hats in quarters, eighths only when really alive, no sixteenths outside surges.

Surges and alive moments stay energetic, mostly through the bass and drums.

**The synth stays in the background, and trades space with the drums.** The
self-designed synth patches and the drone's detuned pad bed are background voices:
- they sit **10 dB under GM** (`SYN_TRIM`), with slow attacks, long releases, lower
  cutoffs, gentler resonance and FM index;
- at control rate every synth voice (per player) and the pad bed pick a *space*:
  - **duck** while a GM instrument, the lead or a theme quote is sounding: a further
    −8 dB and the filter closes (≈900 Hz on the patches, the pad's cutoff ×0.45);
    short notes are dropped, only long tones remain;
  - **back** while the drummer is grooving: −2 dB, filter at ≈2.6 kHz, notes become
    pad-like (short ones mostly dropped, the rest doubled in length);
  - **stretch** when the drums are resting or minimal and nobody else is up front:
    +4 dB, filter open, and more notes get through — `“drums are out, my turn to stretch”`.
- The drummer rests far more: chilling never, subsiding mostly out, awakening rest or
  minimal, alive groove about half the time (more with a hot CPU signal); after three
  phrases of playing it gets tired and drops out to “let the synth have a go”.

The thinking lines say what is happening, e.g. `drums are back → sinking into a soft pad`,
`lead is up front → ducking under it, filter closed`, `drums are out → letting the drone bloom a little`,
`minimal: a soft downbeat and a tick, nothing more`.

**Mix before vs after** (offline 200 s bench with `SYSJAM_RMS=1`, seeds 1–4, RMS in dBFS
per source over the time the drums were playing or resting; ranges across the seeds):

| source | drums playing, before → after | drums resting, before → after |
|---|---|---|
| **synth bus** (patches + pad bed + bells) | −21.4…−22.0 → **−33.5…−34.8** | −20.1…−20.3 → **−27.9…−29.3** |
| drone pad bed | −22.0…−22.3 → −34.3…−37.0 | −20.4…−20.6 → −28.0…−29.9 |
| synth patches | −30.0…−35.6 → −36.4…−42.9 | −32.0…−35.2 → −38.1…−48.5 |
| GM melodic voices | −17.8…−26.7 → −17.5…−26.0 | −40.9…−54.2 → −23.1…−31.1 |
| GM drums | −20.4…−24.0 → −23.1…−27.5 ¹ | — |
| synth bus relative to GM melodic | −4…+5 dB → **−9…−16 dB** | |

¹ −13 dB in one run where a surge played a TR-808 kit; surges stay loud on purpose.
With the drums playing, the synth now sits 9–16 dB under the GM instruments, where it
used to be level with them. When the drums rest it comes forward by about 6 dB, still
8 dB under its old level. Drums rest for 104–145 s of the 200 s, up from 97–105 s.

**Drum density** (notes/min, same benches): alive 321–560 (mean 405) → 160–234 (mean 190, −53%);
awakening 120–201 → 0 (rest or minimal); subsiding 49–63 → 0–17.

**The conductor.** Every bar the conductor listens to the whole band:
- note density of keys + lead + colour against a state target (about 1.5 notes/bar chilling, 4 awakening, 8 alive, 2.5 subsiding, 5 in a surge);
- players crowding the same register within two beats, and players stepping on the lead;
- the mid/high share of the output spectrum, and the loudness.

When the mix gets out of control it steps in, at most every 8 s and never nagging one player more than every 20 s:
- **hush** the worst offender: velocity down, and short notes thinned out by half, for 2 phrases;
- tell it to **lay out** for 1–2 phrases (repeat offenders, or bad clutter);
- **cue** the band when things are badly out of hand: `everyone drop to the drone` (chilling/subsiding; the bassist goes to long notes) or `bass and drums only`;
- **invite** a resting player back (`keys, come back in — softly`) when space opens up.

It also enforces the synth/drums trade:
- the **synth is the first to be hushed** when others play: `keys (synth), back to the pad — let the lead through`,
  or `colour (synth), sit back under the drums` (synth players score extra as offenders whenever a GM voice, the lead or the drums are going);
- outside alive moments and surges, a grooving drummer is told `drums, give it some air — synth, take the space` (lay out 2 phrases);
- when the drums lay out it **invites the synth in**: `drums are out — keys (synth), stretch a little`;
- during a theme quote it backs the quoting player: `nice — lead has Wachet auf, ruft uns die Stimme · everyone support`, and afterwards `lovely. back to it`.

Players respect it, but stubborn ones (high eagerness/busyness) grumble first and comply a bar later:
`conductor: keys (synth), lay out 2 phrases → “one more lick? …ok, ok”`.
Soloists are left alone. `SYSJAM_COND_LOG=1` prints the conductor's measurements every phrase on stderr.

**Note density, before vs after** (offline 200 s bench, `SYSJAM_BENCH=1`, time-weighted over seeds 1, 5, 11 and 23, notes per minute):

| player | chilling before → after | alive before → after |
|---|---|---|
| drone (pad voices that move + sub glides) | 21 → 22 | 69 → 80 |
| bass | 5 → 5 | 155 → 179 |
| drums | 0 → 0 | 480 → 303 |
| keys | 15 → 6 | 216 → 89 |
| lead | 4 → 5 | 27 → 21 |
| colour | 37 → 9 | 72 → 28 |
| **total** | **83 → 48 (−42%)** | **1018 → 700 (−31%)** |

Across the four runs the conductor stepped in 11 times: 7 hushes, 2 lay-outs and 2 invitations back, with 3 grumbles. Restraint alone keeps most phrases tidy, so the conductor only acts when a player really overdoes it.

**Bandleader / arrangement.**
- **Energy** is a 15 s average of the system signals; **agitation** is a 3 s average.
- State changes only happen on phrase boundaries (4 bars). Each state must be held for 3–4 phrases (~36–48 s) before the next change; the thresholds have hysteresis:
  - chilling → awakening at energy > 0.22, or straight to alive on a big multi-signal spike;
  - awakening → alive at > 0.42;
  - alive → subsiding at < 0.30;
  - subsiding lasts 8–19 bars and peels players off: drums first, then the lead resolves, then the bass. It goes back to awakening if energy returns.
- Entries come with drum fills or noise risers.
- Surprises happen at most every 4 phrases: a solo (keys, lead or colour), a breakdown (drums and bass drop out), a bandleader key change, or a lone instrument in chilling.

**Harmony** is independent of the OS. Every 1–2 bars (`--chord-bars`) the
progression moves using 4- and 8-chord phrases drawn from weighted templates:
- major: I–V–vi–IV, vi–IV–I–V, ii–V–I, I–vi–IV–V, I–IV–vi–V, IV–I–V–vi, I–iii–IV–V, IV–V–iii–vi, and two 8-bar forms;
- minor: i–VI–III–VII, i–iv–v–i, i–VII–VI–VII, Andalusian i–VII–VI–V, i–iv–VII–III, i–III–VII–iv, VI–VII–i–i, i–VI–iv–V, and two 8-bar forms.

In chilling the band "meanders" through a Markov graph built from the same templates (every template step is a weighted edge).

About 13% of phrases take a **detour**:
- a secondary dominant (V/x), a borrowed chord (bVI, bVII, iv in major; IV or V in minor), or a colour phrase (sus2/sus4/add9/7ths);
- an occasional **modulation**: to the relative key via a pivot chord, to the parallel major or minor, or a fifth up or down around the circle of fifths via pivot → V7 → new tonic. Screen colour biases the new key's mode: warm/bright screens favour major, cool/dark favour minor.

**The home key isn't sacred.** Once a new key has lasted about 3 phrases, it can
become the new home (40% chance per phrase). The band then confirms it with a
V7 → I cadence and logs `~ new home key: D major (was A major)`. The pull back to
the old home is weak and builds only after about 6 phrases away. Over a long session
the key wanders.

**The tempo breathes.** At phrase boundaries the bandleader may set a new
target tempo, depending on the arrangement state and its agitation:

| state | tempo range |
|---|---|
| chilling | 58–76 bpm |
| awakening | 72–96 bpm |
| alive | 90–128 bpm |
| subsiding | drifts 4–14 bpm lower per move |

The tempo never jumps. It ramps smoothly over 2–8 bars (at most 24 bpm per ramp), one tiny step-length change per sixteenth, so the step clock stays glitch-free. The echo time follows the tempo, gliding one sample at a time. Sometimes the drummer plays a phrase or two in **half-time** or **double-time** feel. `--bpm N` locks the tempo; feel moves still happen.

**Surges.** Every so often the band opens the throttle. A surge is rare: the
first one can come after about 40 s, later ones at least 2.5 minutes apart. It only
starts while the band is awakening or alive (or from any state when the typing is
racing), and it is much more likely when the band is alive or the typing is intense. When it fires:
- the tempo ramps to 120–150 bpm over 2 bars, holds for 2–4 phrases, then winds back down over 6 bars to the middle of the state's tempo range;
- the drummer drops into a lopsided Harley-Davidson V-twin **"potato-potato" lope**: long-short pairs of deep kicks (38–47 Hz bodies) and floor toms on sixteenths 0, 3, 8 and 11, with a soft rim;
- the bassist thuds the root on the same long-short pulse with a growl patch;
- a low-passed **throttle-rev swell** (a rising 33→75 Hz rumble) opens the surge and some phrases inside it, and a falling one closes it;
- the TUI shows the feel as `surge — V-twin lope`, and the drummer's thinking line explains it, e.g. `the typing is racing → opening the throttle to 138 bpm: V-twin lope, long-short low kicks + floor toms`.

Nothing in a surge is high-passed or noisy: it is all low end. `--bpm` keeps the tempo locked, but the lope still happens.

**Every player has its own key.** The bassist and drone anchor the band key. Keys,
lead and colour each carry their own key and mode, related to the band's, and
they listen to the others:
- At phrase boundaries a player may wander, guided by its *adventure* trait, to the relative or parallel key, a fifth up or down, or a brighter or darker mode (Lydian, Mixolydian, Dorian, Phrygian…). The TUI shows e.g. `drifting to E Dorian over the band's A minor`.
- Adventurous players can go **bitonal** (a tritone or a major third away), but only in chaos.
- Notes outside the player's own scale snap to the nearest scale tone. Every note is scored for roughness against the bass and pad, and a player that hears too much steers back (`heard roughness against the bass/pad … → steering back`).
- At cadences, phrase ends and key changes everyone converges on the band key again.
- In minor keys the raised 7th is spelt as a sharp (G# in A minor, C# in D minor), and keys are spelt the conventional way (E♭ minor, not D# minor).

The voicing keeps common tones and moves the other voices by the smallest
interval. The bass follows chord roots; shimmer, arps and the lead use chord
tones.

**System events push the band; they never drive notes directly.**
- CPU > 65% for 5 s → tension: the next chord becomes a V7, a secondary dominant or a borrowed chord (40 s cooldown).
- A net or disk burst (> 0.7 for 4 s) → modulation (90 s cooldown).
- Going back to idle after a busy stretch → a cadence resolving to the tonic (60 s cooldown).

`--verbose` shows all of this, rate-limited. For example:
```
Key: A minor | Am → E → Em7 → Asus2
~ sustained CPU spike -> tension
Key: A minor | E → Em7 → Asus2 → A7   (tension: A7 (V/Dm7))
~ chilling -> awakening (energy 0.48)
bassist joins
drummer joins
+ Warm Pad (89)  − Sweep Pad (95) [Pad]
Key: E major | A7 → Dm7 → B7 → E   (modulation: A minor -> E major)
keys takes a solo
```

## Themes: public-domain quotes, always cited

Every now and then a player gets inspired and quotes a well-known tune, woven into
whatever the band is playing. It happens rarely: on average a couple of times in 10–15
minutes, more likely when the band is awakening or alive, with a 100–200 s pause after
each episode. Only one theme is in the air at a time.

- **Transposed and adapted**: the theme is stored as scale degrees, so the player maps it
  into its *own* key and mode. A major tune turns minor in a minor key, Dorian in a Dorian
  key, and so on. Chromatic notes are kept only when the modes match. Durations are in
  beats, so it follows the band's tempo.
- **Variations**: a straight quote; a fragment (the first half); augmented, as a slow
  pad melody (keys/colour); diminution, twice as fast (lead, only when alive); inversion;
  call and response (the lead plays the first phrase, someone else answers with the
  second); or the bassist plays it low.
- **Passed around**: when a quote ends, another player may pick up a fragment, an
  inversion or the bass line of the same theme (at most two passes).
- **Woven in**: quotes are played softly (velocity ≈45–55). The quoting player is
  featured for the duration. The others are hushed to support it, and the synth voices
  duck under it.
- **Cited, every time**, in the terminal:
  - the quoting player's thinking line: `♫ quoting Wachet auf, ruft uns die Stimme (J.S. Bach, BWV 140.7) · music21 corpus (Bach chorales) · public domain — augmented, as a slow pad melody, in Eb major`;
  - the persistent `♫ now quoting` header line, for as long as the quote lasts;
  - the *quoted themes* panel (a scrolling history);
  - the band-event log. Citation lines start with `♫` and are **also printed with `--no-tui`**, even without `--verbose`:
    ```
    ♫ [04:13] lead (synth) quotes Kathleen O'Moore (trad., O'Neill no. 110) · O'Neill's Music of Ireland, 1903 · music21 corpus · public domain — a straight quote, in G# minor
    ♫ [04:21] bass picks up Kathleen O'Moore (trad., O'Neill no. 110) · O'Neill's Music of Ireland, 1903 · music21 corpus · public domain — low in the bass (passed on), in G# minor
    ```

**Where the themes come from.** `tools/extract_themes.py` reads a curated selection
from the corpus bundled with [music21](https://github.com/cuthbertLab/music21) 10.5.0. For each tune it
keeps only a 2–8 bar incipit (Bach chorales: up to the first fermata) as melodic facts:
scale degrees relative to the tonic, durations, mode, meter, pickup, title and citation.
It writes them to `assets/themes/themes.txt` (371 themes, 78 KB), which is embedded in the
binary with `include_str!`. **The binary does not need Python or music21.**

**Licensing: public domain only.** A source is used only if the composition is public
domain *and* the corpus files carry no restrictive notice. No encoding text is copied.
Details, transcriber credits and the reasons for every exclusion are in
[`assets/themes/LICENSE`](assets/themes/LICENSE).

| source | themes | status |
|---|---|---|
| J.S. Bach chorales (BWV numbers from music21's chorale lists) | 120 | composition PD; files have no rights notice (credit for Margaret Greentree's PDF edition noted) |
| Beethoven string quartets (Op. 18, 59, 74, 132, 133) | 15 | PD; no rights notice |
| Haydn string quartets (Op. 1 No. 1, Op. 74 No. 1) | 9 | PD; no rights notice |
| Mozart (K. 80, 155, 156, 458 “Hunt”, 545) | 16 | PD; no rights notice |
| Handel, “Lascia ch'io pianga” (Rinaldo, HWV 7) | 1 | PD; no rights notice |
| Ryan's Mammoth Collection (Boston, 1883) | 110 | traditional, PD; ABC transcriber credits only |
| O'Neill's Music of Ireland (Chicago, 1903) | 60 | traditional, PD; ABC transcriber credits only |
| Aird's Selection of Airs (Glasgow, 1782–97) | 40 | traditional, PD; ABC transcriptions credited to Jack Campin |
| **total** | **371** | |

**Excluded:**
- Essen folksongs: non-commercial permission only;
- Ciconia and Luca: CC-BY-SA 3.0 editorial transcriptions, not GPL-3 compatible;
- Trecento: editorial transcriptions with no stated licence;
- Palestrina: John Miller copyright;
- Monteverdi: Ut Orpheus ©;
- Beach: CC-BY-SA engraving;
- Schubert and Clara Schumann: bare © notices;
- the lead sheets: one is “All Rights Reserved”;
- Joplin: an unverifiable musescore.com upload.

Regenerate with:
```bash
pip install --user --break-system-packages music21
python3 tools/extract_themes.py            # → assets/themes/themes.txt
```
`SYSJAM_QUOTE_TEST=1` makes quotes come every ~25 s, for demos and screenshots.

## General MIDI instruments (SoundFont)

Instruments come from a GM `.sf2` file played by rustysynth (polyphony
capped at 28, reverb/chorus off). The file is found in this order:
1. `--soundfont PATH`
2. system paths: `/usr/share/sounds/sf2/FluidR3_GM.sf2`, `/usr/share/soundfonts/FluidR3_GM.sf2`, `default-GM.sf2`, `TimGM6mb.sf2`, `GeneralUser_GS.sf2`, any `/usr/share/soundfonts/*.sf2`
3. `~/.local/share/sysjam/*.sf2`
4. `TimGM6mb.sf2` (or any `.sf2`) next to the binary or in an `assets/` directory beside it or above it. **This file is shipped in this repo as `assets/TimGM6mb.sf2`**, so a source checkout finds it automatically.

If none is found, the band plays its oscillator instruments only and prints
one hint line: `sudo apt install fluid-soundfont-gm` (FluidR3, 140 MB, sounds
better than TimGM6mb).

Picks are weighted across **all 128 programs**: every program has a real
weight in at least two roles (pad, comp, lead, colour, bass). On top of that,
each player draws a **signature palette** from its personality (ethnic plucks;
brass, reeds & organs; synth leads & FX; mallets & bells; strings & voices; keys
& guitars; winds & pipes; oddballs & sound FX). It favours that palette, with an
occasional wildcard from anywhere. Per-program gain is kept narrow (0.75–0.95), so
oddballs are audible, and the GM layer rises with the band's energy so it sits
in front of the oscillator bed.

The **drummer** switches between GM kits (Standard, Room, Power, Electronic,
TR-808, Jazz, Brush, Orchestra) and auxiliary percussion sets (shakers,
congas & bongos, agogo/claves/woodblocks, triangle & tambourine, timbales &
cowbell).

**Every player can play GM or synthesize.** Each has a *synth* trait (sample vs
synth preference). When inspired, it picks either a GM program or a patch it
designs itself, with parameters drawn from its personality and its tied signal.
There are four engines:
- **subtractive**: detuned saw / square / PWM into a resonant filter with its own envelope;
- **FM**: 2–4 operators with a decaying index (bells, e-pianos, metallic leads);
- **wavetable**: morphing between sine, soft saw, hollow square, vocal and glass tables;
- **Karplus-Strong pluck**, with a brightness-controlled loop filter.

The voices share a 10-voice pool. The quietest or released voice is stolen,
with a de-click tail, so CPU stays capped. The sub drone is separate and always
on. The TUI shows the source, e.g. `GM Koto (107)` or `synth FM 3-op ratio 3.5 bell`.

- 2–4 layers are active (pad, comp/keys, lead, colour, and sometimes a GM bass).
- There is no fixed rotation. A player picks up a new instrument when something **inspires** it:
  - a state change (e.g. awakening → alive);
  - a key change, or the band settling into a new home key;
  - its tied signal spiking (bassist: disk/files, drummer: CPU, keys: keyboard/mouse, lead: busy cores, colour: network);
  - hearing another player do something interesting: the lead's new idea, the colour player's answer, industrial noise, or a guest thread sitting in;
  - boredom, which grows with time on the same sound and comes faster for players with a high *curiosity* trait.
- Each sound stays at least 8–10 s, and a player isn't open to new inspiration for 15 s after a change.
- Swaps are crossfaded (CC11, 5–12 s) to the other channel of the player's pair. Held pad notes are re-struck on the new channel and released on the old one after its fade, so notes are never cut off. The reason shows in the TUI, e.g. `inspired by the move to subsiding → picking up a banjo`.
- Oscillator players design new **synth patches** the same way:
  - bassist: round sine / warm saw / growl / soft pluck, gliding over about 0.5 s;
  - drummer: deep / tight / boomy kick, latched per hit;
  - colour/lead bells: glass, octave chimes, metal, detuned soft, applied to new notes only.
- Program changes only happen on a channel that has been silent for 4 s.
- CC7 sets per-program gain, so odd programs (FX, helicopter, gunshot…) play quieter and rarer.
- Velocities are humanised.
- `--verbose` prints e.g. `+ Vibraphone (11)  − Sitar (104) [Comp]`.

**SoundFont licence (separate from the code).** `assets/TimGM6mb.sf2` is
© 2004 Tim Brechbill and © 2010 David Bolton. It is licensed under the **GNU GPL
version 2 only**. Debian's copyright file says "the terms of the GNU General
Public License version 2" and does not include an "or any later version"
clause. Provenance and the full GPL-2 text are in
`assets/TimGM6mb-LICENSE.txt`.

GPL-2-only is not compatible with GPL-3, so the SoundFont is **not** part of
the GPL-3.0-or-later program. It is a separately licensed data asset that is
distributed alongside the code, unmodified, as a "mere aggregation". sysjam
does not link or embed it; it only reads it as an input file at run time, just
like any other `.sf2` it finds. You can delete `assets/`, use `--soundfont`
with another file, or pass `--no-gm`. sysjam works fine without it.

## Sound / spectrum

The bands are filled deliberately:
- **sub** 20–60 Hz: drone and kick, mono, never high-passed;
- **bass** 60–250 Hz: bass voice and GM bass;
- **low-mids**: pad;
- **mids**: GM instruments, arps and lead;
- **highs**: bells, hats, cymbals and risers. The upper band stays quiet when calm; brief industrial noise only appears in chaos.

A gentle look-ahead-free limiter with a soft clip sits at the end of the chain.

Measured on the earlier 3-minute demo render (`tools/bands.py`, energy per band relative to the 800 Hz–3 kHz mids). That render still had the old air bed; in the new 75 s demo the 6–16 kHz band in the calm part is −34.9 dB (it was −27.6 dB), and sub/bass are +13.2/+7.2 dB calm and +8.5/−0.2 dB in chaos:

| band | idle 2–32 s | busy 50–110 s | winding down 140–178 s |
|---|---|---|---|
| sub 20–60 Hz | +14.1 dB | +6.3 dB | +10.1 dB |
| bass 60–250 Hz | +10.1 dB | +3.5 dB | +2.5 dB |
| low-mid 250–800 Hz | +9.7 dB | +3.4 dB | +2.8 dB |
| mid 800 Hz–3 kHz | 0 | 0 | 0 |
| high-mid 3–6 kHz | −13.9 dB | −13.9 dB | −16.3 dB |
| high 6–16 kHz | −27.6 dB | −25.4 dB | −25.6 dB |

Level is about −15 dBFS RMS with peaks around −2 to −5 dBFS. On small
speakers, raise `--sub` (up to 2) or lower `--volume`.

## Keyboard / mouse activity

Intense typing makes the band pick up (and can trigger a surge), and mouse
bursts nudge it more gently. These are improvisational cues, not one click per
keypress. sysjam only counts **how many** input events happen. It never reads
which keys, and key codes are never stored or logged.

At startup sysjam logs which source it uses, and the TUI header shows it with
live keys and mouse meters (top right: `input evdev×2 keys ███▌ mouse █`):

```
input: evdev (2 devices: AT Translated Set 2 keyboard, ELAN0712:00 04F3:30FD Touchpad)
input: evdev via setgid input (privileges dropped) — 2 devices: …
input: evdev unavailable: permission denied on 3 keyboard/pointer nodes (/dev/input/event* is root:input 660, and you're not in that group)
input: to opt in: `sudo usermod -aG input $USER` and log in again (or the setgid install below); sysjam counts events only, never key codes
input: irq fallback: usb-hid Logitech USB Receiver (kbd+pointer), irq 1 i8042 (kbd), irq 51 elan0712:00 (pointer)
input: irq fallback: xhci (irq 127) — shared controller line(s), idle baseline calibrated, USB disk/net traffic ignored; approximate
input: none — no readable keyboard/mouse counters …
```

Sources, best first:
1. **evdev** (`/dev/input/event*`), read-only and non-blocking. Only keyboard and pointer nodes are opened. They are classified from sysfs capabilities (EV_KEY+EV_REP = keyboard, EV_REL or a touchpad/mouse name = pointer); power buttons, lid switches, jacks and the like are skipped. It counts key *presses* and motion events. This needs the `input` group or the setgid install below. Devices are opened once at startup, so plug in new ones before starting.
2. **No-permission fallbacks**, combined when several exist:
   - **USB HID report counters**: `/sys/bus/usb/devices/*/urbnum` (world-readable) of each USB device that carries a keyboard or pointer, identified via `/sys/class/input`. This covers USB keyboards and mice, which otherwise hide behind the shared USB controller interrupt. Devices that also stream audio (headsets) are skipped.
   - **specific `/proc/interrupts` lines**: `i8042` (irq 1 = keyboard, irq 12 = PS/2 pointer) and i2c-HID touchpads/keyboards (`i2c_hid`, `ELAN…`, `SYNA…`, `MSFT0001`, `PNP0C50`, …).
3. **Aggregate controller lines** (`xhci_hcd`, `ehci_hcd`, `i2c_designware`) when nothing more specific exists. These are shared with other devices, so sysjam keeps an **idle baseline** per line (it drops at once, then creeps up over about 5 minutes) and counts only the excess. Intervals in which a USB-attached disk or network adapter moved data are ignored. Constant streams like a USB DAC sink into the baseline. It is approximate: a trickle is read as typing, a stream as mouse motion.
4. Otherwise **none**. The music carries on.

Dedicated fallback counters get the same baseline, but with a smaller margin and a ~30-minute creep, so long typing sessions aren't absorbed into it.
Rates are smoothed over about 3 s and mapped to 0–1; about 6 keystrokes/s saturates the keyboard meter. The two together can add at most 0.45 to the bandleader's energy input, so they push but never dominate. `--no-input` turns all of it off, and nothing is opened.

### Giving sysjam evdev access, and the security tradeoff

Access to `/dev/input/event*` is access to **every keystroke**, including
passwords, for any program that has it. sysjam reads only the event type and
whether it was a press, and drops the rest, but whatever you grant this way
applies to the binary, so pick the narrowest option:

- **Do nothing.** The fallbacks above need no permission, and on most machines they are good enough to drive the band.
- **`sudo usermod -aG input $USER`**, then log out and back in. Simple, but *every* program you run can then read every keyboard. This is the broadest option.
- **setgid `input` on the sysjam binary only.** Only this binary gets the group, and only for an instant:

  ```bash
  install -m 0755 dist/sysjam ~/.local/bin/sysjam   # a dir only you can write to
  sudo chgrp input ~/.local/bin/sysjam && sudo chmod g+s ~/.local/bin/sysjam
  ```

  At the very start of `main`, before any thread, library or argument parsing,
  sysjam opens the keyboard/pointer event nodes it needs. It then permanently
  drops the elevated gid with `setresgid(rgid, rgid, rgid)` and verifies that the
  real, effective and saved gid are all your own and that the old gid can't be
  regained; otherwise it refuses to run. It logs
  `input: evdev via setgid input (privileges dropped)`. The kernel keeps a setgid
  process non-dumpable, so other programs running as you can't ptrace it and take
  over the open input fds.

  Caveats:
  - **Redo the `chgrp`/`chmod` after every binary update.** Replacing or rebuilding the file drops the setgid bit (that's intended).
  - **Keep it in a path only you (or root) can write**, such as `~/.local/bin` (check with `ls -ld ~/.local/bin`) or `/usr/local/bin`. If another user could replace the file, they could swap in a binary that keeps the `input` gid.
  - In setgid mode, glibc runs sysjam in *secure mode* (AT_SECURE). `LD_LIBRARY_PATH` and similar variables are ignored, and so are sysjam's own test hooks (`SYSJAM_INPUT_DIR`, `SYSJAM_INTERRUPTS`). ALSA, PipeWire and D-Bus are loaded from the standard system library paths: by soname, then by absolute path in `/usr/lib/x86_64-linux-gnu`, `/usr/lib64` and `/usr/lib`. libdbus ignores the environment in secure mode, so sysjam passes it the session-bus address itself (`$DBUS_SESSION_BUS_ADDRESS`, or `$XDG_RUNTIME_DIR/bus`). Screen capture through the portal keeps working.
  - To undo: `sudo chmod g-s ~/.local/bin/sysjam`.

## Screen colour

Screen colour is a slow input. Warmth and brightness tint the pad, and they bias
whether a modulation lands in major or minor. Screen *changes* add a little energy.
It uses one heavily downsampled frame every `--screen-interval` seconds
(default 4).

`--screen-method auto` tries:
- **Wayland session** (COSMIC, GNOME): portal → grim → XWayland → off
- **X11 session**: X11 → portal → off

1. **xdg-desktop-portal ScreenCast** (D-Bus via `libdbus-1.so.3`, frames via
   `libpipewire-0.3.so.0`, both loaded at run time).
   - It asks for one monitor with `persist_mode=2`.
   - The restore token is saved to `~/.config/sysjam/portal-restore-token` (mode 0600, or under `$XDG_CONFIG_HOME`), so the share dialog appears **once**. Later runs restore silently. A stale token is deleted and the dialog is asked once more.
   - The PipeWire stream negotiates shared-memory RGB frames (no DMA-BUF). It is kept **paused** and resumed for a single frame every ~4 s; about 64×36 pixels are read.
   - If you press **Cancel**, sysjam remembers that in `~/.config/sysjam/portal-declined` and won't ask again. `--screen-method portal` asks again.
2. **grim** (wlroots / ext-image-copy compositors), if installed.
3. **X11 GetImage** (12 one-pixel rows). On Wayland it only sees XWayland
   windows; an all-black result is rejected. Connect and grab run on a helper thread with a 2 s limit, so a missing server fails fast and quietly.
4. `--screen-method cmd --screen-cmd "…"`: any command that prints a P6 PPM.

If nothing works you get one short line, such as
`screen colour: off (no portal/grim capture; --verbose for details)`, and
the music carries on. `--verbose` lists why each method was skipped.

All D-Bus calls use 5 s timeouts. The share dialog waits up to 90 s, and Ctrl-C
interrupts it. Capture runs on its own nice-15 thread, so it can never block audio.

## What you see on first run

```
instruments: /usr/share/sounds/sf2/FluidR3_GM.sf2      (or ./TimGM6mb.sf2)
audio: ALSA 'default' 44100 Hz stereo
sysjam playing: root F2 major, 87 bpm, seed 734912055  (Ctrl-C to fade out)
```
On a Wayland desktop the system **screen-share dialog** opens. Pick the monitor
and allow it. The music is already playing while the dialog is open. Then:
```
screen colour: xdg-desktop-portal ScreenCast, one tiny sample every 4s
```
The desktop may show its "screen is being shared" indicator while sysjam
runs. From the second run on there is no dialog.

## Performance

- Audio is rendered in 2048-frame blocks (~120 ms ALSA buffer). The oscillator bed updates at control rate; rustysynth renders in 64-sample blocks with polyphony capped at 28 and reverb/chorus off.
- Denormals are flushed to zero.
- The monitor thread runs at nice 10, the screen thread at nice 15. The PipeWire stream is paused between samples.

Measured exactly (schedstat, all threads) on an 8-vCPU Xeon VM:

| case | CPU (one core) | RSS |
|---|---|---|
| idle, FluidR3 GM (140 MB SoundFont) | 1.97% | 146 MB |
| idle, shipped TimGM6mb | 1.77% | 10 MB |
| idle, `--no-gm` | 1.47% | 3.7 MB |
| idle, TimGM6mb, input tracking on (/proc/interrupts) vs `--no-input` | 1.87% vs 1.63% (difference within run-to-run noise) | 10 MB |
| 75 s chaos demo (`tools/demo_short.sh`) | 2.15% | |
| 90 s tempo/key demo (`tools/demo_tempo.sh`: random key + tempo, 40 s of stress-ng/downloads/direct writes/typing, one surge at ~60–75 s), incl. WAV writing | 2.84% | |
| 3-min demo under heavy load (stress-ng, ~50 MB/s downloads, ~500 MB/s direct writes), incl. WAV writing | 2.50% | |
| offline engine only, alive state | ~1.7% | |
| idle, FluidR3, **live terminal UI on** vs `--no-tui` (same seed, side by side, 120 s, /proc utime+stime) | 2.22% vs 1.89% (the UI thread itself: 0.27%) | |
| idle, TimGM6mb, synth voice pool + per-player keys, **UI on** vs `--no-tui` (side by side, 60 s) | 2.05% vs 1.71% | |
| idle, TimGM6mb, restraint + conductor, **UI on with the conductor panel** vs `--no-tui` (side by side, 60 s) | 1.81% vs 1.61% (UI thread 0.28%, audio thread 1.28%) | |
| idle, TimGM6mb, synth ducking + drum restraint + 371 embedded themes, **UI on** (now-quoting line + citation panel) vs `--no-tui` (side by side, 60 s) | 2.10% vs 1.72% (UI thread 0.31%) | |
| offline bench, seeds 1–4, busy phase: previous release vs this one (same box, sequential) | 1.31–1.86% → 1.57–2.07% (longer pad-like synth notes keep more pool voices sounding) | |
| alive under heavy load (busy thread as guest, ~400 MB/s writes, ~40 MB/s downloads, file churn), UI on, X11 screen sampling, 60 s | 3.05% | |

## Troubleshooting

**No sound / "cannot open ALSA device".** Pop!_OS routes ALSA `default` to
PipeWire through `pipewire-alsa`. Try `--device pipewire` or `--device pulse`,
and check `aplay -L`. You can always use `--wav test.wav --duration 30`.

**Instruments sound thin.** Install the better SoundFont with
`sudo apt install fluid-soundfont-gm`. It is found automatically, before the
shipped TimGM6mb.

**No share dialog / colour stays off on Wayland.**
- Run with `--verbose` to see why each method was skipped.
- The portal needs `xdg-desktop-portal` plus a backend with ScreenCast (`xdg-desktop-portal-cosmic` or `-gnome`) and PipeWire.
- If the compositor only offers DMA-BUF frames, sysjam says so and turns colour off.
- If you cancelled once, use `--screen-method portal` to be asked again. To forget the saved permission, delete `~/.config/sysjam/portal-restore-token`.

**The file-open clicks feel sparse.** `/proc/sys/fs/file-nr` is a cheap,
batched system-wide counter, so small bursts cancel out.

## Building

```bash
cargo build --release           # native build
# portable build used for dist/ (glibc 2.29 symbol baseline) via zig:
pip install ziglang && cargo install cargo-zigbuild --version 0.21.8
cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.29
mkdir -p dist && cp target/x86_64-unknown-linux-gnu/release/sysjam dist/
```
Requires rustc ≥ 1.85 (rustysynth is pinned to 1.3.5).

Tools:
- `tools/demo_short.sh` renders the 75 s calm → chaos → calm demo. Typing is simulated through a fake `/proc/interrupts` (`SYSJAM_INTERRUPTS`).
- `tools/demo.sh` renders a 3-minute idle → spike → busy → idle demo.
- `tools/bands.py` prints band energies.
- `tools/measure.py SECS ./sysjam --null` measures sysjam's own CPU.
- `tools/portal_test.sh BIN COLOR ARGS…` runs the end-to-end portal test. It uses a private D-Bus, pipewire, wireplumber, a GStreamer test source and `tools/mock_portal.py`.
- `SYSJAM_BENCH=1 SYSJAM_BENCH_LOG=1 ./sysjam --seed 1` gives an offline 200 s timeline with CPU figures, per-player note density per state, the conductor's intervention count and the list of theme quotes. Add `SYSJAM_RMS=1` for RMS per source (synth patches, pad bed, bells, sub, synth bass/kick, GM melodic, GM drums) split by drums playing/resting, and `SYSJAM_BENCH_SCALE=5` for a 5× longer timeline.
- `tools/extract_themes.py` regenerates `assets/themes/themes.txt` from the music21 corpus (see *Themes*).

## Licence

sysjam is free software: you can redistribute it and/or modify it under the
terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version (**GPL-3.0-or-later**). The full text is in `LICENSE`.

`assets/TimGM6mb.sf2` is a separately licensed asset (GPL-2.0-only); see above.

`assets/themes/themes.txt` holds short melodic incipits of public-domain compositions taken from the
music21 corpus. Sources, transcriber credits and exclusions are listed in `assets/themes/LICENSE`.
