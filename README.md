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

You can steer it from the terminal UI. **↑/↓** nudge the tempo; **←/→** change the conductor's
**style**: House Band, Ambient Monk, Jazz Cat, Blues, Country, Biker, Baroque, Celtic Session,
Industrial, Techno, Space Cadet, Tribal/Gamelan, or *auto* (the default: it wanders between them).
Each style changes the band's instruments, tempo, drum feel, harmony, bass line and theme quotes,
and it restyles the whole UI (palette, borders, icons, meters, and the conductor's outfit and voice).
See [Steering](#steering-tempo-and-conductor-styles). A switch you ask for happens at once: the
harmony on the next beat, everything else within about a second.

Everyone on stage is a little animated character. The conductor beats real conducting patterns
phase-locked to the audio you are hearing, and each musician moves with their own notes (sticks on
the drum hits, plucks, bowing, a horn that tilts up for high notes, a drone that breathes). The
conductor also has **feelings** (16 blended moods) worked out by a tiny on-device neural net, and
he learns what you like, but **only** when you press **`l`** (like). See
[the conductor's feelings](#the-conductors-feelings-and-his-tiny-brain) and [likes](#likes-the-only-thing-sysjam-learns-from).

| Country | Techno |
|---|---|
| ![Country: the conductor beats 4/4 in a cowboy hat while the band plays](docs/anim-country.gif) | ![Techno: the DJ conductor's platters spin, hands up on every kick](docs/anim-techno.gif) |

(Formerly developed under the working name *sysmusic*.) Listen: [`docs/sample.mp3`](docs/sample.mp3) is a 75 s demo at 192 kbps. It starts calm with no hiss, then hits chaos (CPU + network + disk load plus simulated fast typing), where a short burst of industrial noise comes in, then settles again. Its verbose log is `docs/sample-render.log`. [`docs/sample-tempo.mp3`](docs/sample-tempo.mp3) is a 90 s demo (seed 5, made with `tools/demo_tempo.sh`). It starts in A major at 84 bpm, and the tempo eases down to 73 while the band is chilling. Under load (from 18 s) the band comes alive and ramps toward 97 bpm. At about 60 s a **surge** opens the throttle from 85 to 133 bpm with the V-twin lope, while the band modulates to D minor. The surge winds down to 109 bpm at about 75 s, and the band then subsides. Its log is `docs/sample-tempo-render.log`.

![sysjam's live terminal UI mid-transition: two ↑ presses gave tempo bias +14, then ← switched Country to Blues; the players change over one by one ("conductor went Blues → picking up a fingered bass"), and the crowd of processes below holds up lighters](docs/tui.png)

One static-ish binary (`sysjam`, ~2.2 MB, x86_64, needs glibc ≥ 2.29). It
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
| `--style NAME` | conductor style: `auto` (default), `house`, `monk`, `jazz`, `blues`, `country`, `biker`, `baroque`, `celtic`, `industrial`, `techno`, `space`, `tribal` (names and unique prefixes work). ←/→ in the TUI |
| `--tempo-bias N` | sticky bpm offset on the living tempo (±70, the tempo stays within 50–160). ↑/↓ in the TUI nudge it by 7 |
| `--reset-likes` | forget everything you liked (deletes `~/.config/sysjam/likes.yaml`) and start fresh |
| `--ascii` | plain ASCII TUI: no emoji, box drawing or symbols. Automatic on `TERM=linux` |
| `--chord-bars N` | base bars per chord (default 2). Chilling stretches this, peaks tighten it |
| `--seed N` | repeat a run exactly (same input → same music) |
| `--soundfont PATH`, `--no-gm`, `--gm-layers N` | General MIDI layer (see below); N = 1–4 simultaneous layers (default 3, +1 at peaks) |
| `--clicks L` | file open/close click texture 0–1 (default 0.25) |
| `--no-leads` | never let a busy thread sit in as a guest soloist |
| `--no-input` | don't watch keyboard/mouse activity (see below) |
| `--no-audience` | no process crowd under the band, and no audience scan at all (see [the audience](#the-audience-your-processes-on-the-dance-floor)) |
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
  bandleader's energy and agitation meters, and `tempo bias +14` while you have nudged the tempo.
  The top border carries the style badge (`🐎 style Country`, `♪ style auto · Blues`, or
  `♪ style Blues ← Country, changing over…` during a switch). Top right: the input source
  (`evdev×2`, `evdev×2 setgid`, `usb-hid`, `irq i8042`, `irq xhci≈`, `none`) with one combined activity meter.
  The bottom border shows the key hint `↑↓ tempo  ←→ style  l like  space pause  q quit`.
  After you press `l` a `♥ liked (n)` badge flashes in the top border.
  The first line also shows where the harmony is heading, e.g.
  `⟶ heading to D minor: Dm (pivot) → Bbmaj7 → A7 → Dm7` or `⟶ callback to the phrase from 6 phrases ago: …`.
  The third line is the persistent **`♫ now quoting`** line: while a player quotes a
  theme it shows a progress meter, the player and the full citation, e.g.
  `♫ now quoting ██▍ lead: Wachet auf, ruft uns die Stimme (J.S. Bach, BWV 140.7) · music21 corpus (Bach chorales) · public domain — a straight quote`.
- **The band**: one panel per member: drone, bassist, drummer, keys, lead,
  colour/FX, plus the guest voice when a busy thread sits in. On the left of each panel is the
  member as a small animated character (10 columns × 4 rows, 3 on short terminals), driven by the
  notes the engine actually plays, at the moment you hear them:
  - **drummer**: the sticks come down on each hit (alternating hands), the snare flashes, the kick
    drum thumps, and the cymbal wobbles after a hat, ride or crash;
  - **bassist**: an upright that bobs the player's head and shows the pluck;
  - **keys**: piano (a key goes down), organ (two manuals, alternate hands) or electric piano poses,
    by the GM program;
  - **lead / colour**: guitar strums, a bowed fiddle (long down-bow, short up-bow), brass that
    tilts up on high notes (with a bend squiggle in Blues), a sax with notes floating up, a flute
    with moving fingers, mallets on bells, knobs turning on a synth; leads sway on the beat;
  - **drone**: eyes closed, breathing every two beats, humming `~ ~ ~`;
  - **guest**: walks on from the right when a busy thread sits in, and walks off again;
  - **resting** players put the instrument down: coffee (`c[_]`), dozing (`zZ`) or a phone (`[:]`);
    a **hushed** player looks sheepish (`(._.)` and `shh`); an **instrument swap** slides the old
    instrument out and the new one in; a player quoting a theme gets a `(♫)` bubble;
  - they glance at the conductor: a sweat drop when he's cross, a sparkle when he beams;
  - everyone wears the style's costume: cowboy hats in Country, sunglasses in Blues, headphones in
    Techno (glowsticks while resting), wigs in Baroque, hard hats in Industrial, helmets in Space Cadet.

  Each panel then shows:
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
- **Conductor** (top right): a figure that **conducts in time**. Its baton traces real
  conducting patterns, phase-locked to the beat you are hearing (see
  [In time with the audio](#in-time-with-the-audio)):
  - 4/4: down, left, right, up, with the downbeat (the lowest point) exactly on beat 1;
  - half time: a 2-beat down-up over the whole bar; double time: the 4-pattern twice per bar;
  - Blues' 12/8 shuffle: a little tick on the swung third triplet of every beat;
  - 3/4 (triangle) and 2/4 (down-up) patterns are built in too; all current styles play in 4/4;
  - tempo ramps and surges are followed continuously, because the baton position is computed from the
    engine's beat clock rather than from a frame counter;
  - the gesture grows and shrinks with his mood's energy.

  Other poses: sitting on a stool (`zZ`) while the band is chilling, `shh` with a finger to his
  lips when he hushes someone, pointing (`────>`) when he cues or invites a player, both arms
  up during a surge. In Techno he is a DJ: the platters spin with the beat and his hands go up on
  every kick. His face shows his **mood** (`(^_^)` content, `(^o^)*` delighted, `(>_<)#`
  frustrated, `(-.-) zZ` sleepy, `(O_O)'` anxious, `(;_;)` melancholy…), the panel title shows the
  mood name, and the bottom line shows what his little brain is thinking, e.g.
  `🧠 frustrated 0.62 → hush keys` (mood, its weight, the intervention or speech intent it leans to).

  Its speech line shows the current instruction, e.g. `“keys, bring it down”`, or how he feels,
  in the style's voice (`“Hot dang! y'all, that's the sound”`, `“that's not a wrong note, that's a choice”`).
  The addressed player's panel lights up yellow for a few seconds, and its thinking line shows the
  exchange: `conductor: keys, bring it down → “ok, quieter”`. The panel title counts the
  interventions so far.

  ![The conductor's 16 expressions in five costumes (Country, Blues, Techno, Baroque, Tribal)](docs/expressions.png)
- **Quoted themes** (right, under the conductor): a scrolling history of the recent
  citations with their time, newest highlighted.
- **Machine**: CPU, busy cores, network, disk writes, file churn and input
  activity (keyboard and mouse combined), each with a sparkline of the last two minutes. Below them sit a
  swatch of the sampled screen colour and the busiest threads.
- **Levels**: sub (< 60 Hz), bass, mid and high meters of the output.
- **Band events**: a rate-limited log of instrument changes, band moves,
  modulations, guests, and the conductor's instructions with the players' replies.

Keys:

| key | action |
|---|---|
| `↑` / `↓` | tempo bias ±7 bpm, ramped over one bar (hold to repeat, ~6 per second) |
| `←` / `→` | previous / next conductor style (auto → House Band → … → Tribal → auto) |
| `q`, `Esc`, `Ctrl-C` | fade out and quit (press again to quit immediately) |
| `n` or `Tab` | **next idea**: move on right now, within about a beat (see [next idea](#next-idea-and-never-dead-air)) |
| `l` or `Enter` | **like** what's playing now (see [likes](#likes-the-only-thing-sysjam-learns-from)) |
| `a` | force the crowd panel on / off (by default it shows by itself when the terminal is at least 160×55) |
| `space` | pause / resume the display (the music keeps playing) |

The footer shows them: `↑↓ tempo  ←→ style  n next  l like  a crowd  space pause  q quit`.
The arrow keys only work in the TUI; `--style` and `--tempo-bias` are the command-line equivalents.

The UI runs in its own thread at nice 5. The audio thread publishes a small snapshot about every
100 ms with a non-blocking `try_lock`, so a slow or stalled terminal can never hold up the audio.
The whole UI refreshes about 3 times a second; in between, only the little people (and the beat
dots) are redrawn, at about 16 fps, straight to the changed cells, plus one extra frame exactly on
every beat. The crowd moves at about 8 fps (and on every beat). Frames where nothing moved are
skipped, and only colour changes are sent to the terminal. That keeps the UI thread at about 0.40%
of one core with the crowd on (measured below).

All figures are drawn only from ASCII, light box drawing (`─│┌┐└┘├┤┬┴┼╭╮╯╰╱╲`) and `♪ ♫ • ·`:
no block or shade elements (hats are drawn with box lines, e.g. the cowboy hat `~╭^╮~`), no heavy/light mixing, no rare
IPA letters, no emoji, no combining characters or variation selectors. Every frame of every pose,
mood, style and instrument is checked by a unit test to be exactly 12 (conductor) or 10
(musician) cells wide by `unicode-width`, so nothing shears in any terminal font. (The old Tribal
figure's `╿╽` headdress, `ᴗ` mouth and `▒` robe are gone: it now wears `\|||/`.)

### In time with the audio

The audio thread stamps every 16th-note step into a lock-free ring of atomics (step number, the
engine sample it starts on, its length, and the feel: half/double time, surge, shuffle), and every
note a player plays into a per-player hit ring (sample, pitch, velocity). After each block it
writes to the sound card it records how far ahead of the speaker it is (ALSA `snd_pcm_delay`), so
the UI can tell which engine sample is being **heard** right now. Each frame asks "which sample is
in the air?" and draws the baton and the musicians for exactly that point in the music. The UI
also wakes up exactly on each beat, so the downbeat frame isn't left to a 66 ms frame grid.

Measured (Techno, seed 5, 75 s in tmux, 3 tempo nudges up and 5 down during the run): the engine's
154 synth kicks were found in the rendered audio a median 0.9 ms after their scheduled sample. The
first frame of each beat reached the terminal a median **−0.7 ms** from the heard kick (mean −1.1,
95% within ±11 ms, worst 43 ms), well within one frame (61 ms at 16.3 fps).
 The terminal is restored
on exit and also when sysjam panics. Colours follow `NO_COLOR` (crossterm drops all
colour codes). Theme colours are truecolor when `COLORTERM` is `truecolor`/`24bit`, otherwise
they are mapped to the nearest xterm-256 entry. `--ascii` (automatic on the Linux console)
transliterates every glyph to ASCII. The UI needs a terminal of about 100×30 or larger; 130×36 is comfortable.

## Steering: tempo and conductor styles

**Tempo (↑/↓).** Each press adds ±7 bpm of *user bias* on top of the living tempo, and the
band ramps there smoothly over one bar. The conductor acknowledges it in its style's voice
(`“picking it up!”`, `“giddy-up!”`, `“more BPM!”`) and the drummer's thinking line shows the new
target. The bandleader keeps moving the tempo with the arrangement; the bias rides on top, and the
result is clamped to 50–160 bpm. Key-press bias **holds for a minute after the last press, then
fades** with a ~90 s half-life (gone after about 5 minutes), so the band drifts back to its own
feel. `--tempo-bias N` is **sticky** (it never fades). With `--bpm` the locked tempo plus the bias
is used.

**Styles (←/→).** A style is a conductor personality. It changes the instruments (inspired swaps
favour the style's GM programs and synth patch kinds), the tempo range, density and strictness,
the drum kit and feel, the bass line, harmony templates, modes, synth vs GM preference, and which
theme collections get quoted. The core rules hold in every style. The deep sub and bass are always
there. There is no hiss except Industrial's deliberate gated texture. The synth sits back while the
drums play (Space Cadet and Techno loosen this). Restraint and CPU stay as before.

| style | one line |
|---|---|
| **auto** (default) | starts as House Band and wanders to another style every 5–9 minutes, at a phrase boundary |
| **House Band** | the balanced default: a bit of everything, nothing pushed |
| **Ambient Monk** | 50–84 bpm drones, sparse, strict hush, Lydian/major drone harmony, pads and choirs, almost no drums |
| **Jazz Cat** | swing, ii–V–I and turnarounds with sevenths everywhere, walking upright bass, brushes and ride, EP/vibes/sax, more quotes, a loose conductor |
| **Blues** | 12-bar form (standard, quick-change, VI7–ii–V and I–IV turnarounds), dominant sevenths, the blues scale with b3/b5/b7 on lead and colour (GM blue notes scooped up with pitch bend), triplet shuffle at 60–100 bpm, boogie bass 1-3-5-6-b7-6-5-3, harmonica/overdrive/organ/piano, call and response between lead and colour |
| **Country** | I–IV–V, I–vi–IV–V and mixolydian I–bVII–IV, 80–120 bpm, train beat (brushed sixteenths) or two-step, boom-chick bass with walk-ups, chick strums on 2 and 4, steel and nylon guitars, fiddle, banjo, harmonica |
| **Biker** | V-twin surges four times as likely, heavy low kick, distorted guitars and organ, Mixolydian rock changes, faster |
| **Baroque** | chorale cadences and Pachelbel-ish progressions, harpsichord, strings, church organ, contrabass, Bach/Handel/Mozart/Haydn quotes favoured, a steady tempo |
| **Celtic Session** | fiddle, whistle, pipes and harp, hand-drum figures, Dorian/Mixolydian modal changes, O'Neill's/Ryan's/Aird's tunes favoured |
| **Industrial** | metallic synth patches (FM-heavy), deliberate gated noise in alive moments, machine kits, minor/Phrygian |
| **Techno** | 120–135 bpm even while chilling (minimal/dub techno), four-on-the-floor deep synthesized sub kick (~50 Hz), off-beat open hats, claps on 2 and 4, an acid 303-style line (resonant filter, accents, slides, a slow sweep over 32 bars), dub chord stabs on the off-beat into the echo, and 16/32-bar grooves → breakdowns → builds (snare roll, filter opening) → drops driven by the machine's intensity. The drums stay in and the synth isn't pushed back, but there is a hard cap of 4–5 layers and no hiss |
| **Space Cadet** | synth-forward (wavetable/FM, +4 dB allowance), slow, evolving, Lydian and major-third colours |
| **Tribal / Gamelan** | hand percussion carries a 3+3+2 cycle, pelog-ish five-note melodies in minor and pentatonic in major, kalimba, steel drums, marimba, taiko |

**Switches you ask for are instant.** When you press ←/→ the switch happens at once and may be
jarring: the harmony restarts on the next beat in the new style, the tempo ramps there within one
beat, the drum feel and kit change right away, every instrument is swapped with 0.1–0.15 s fades
(held notes are cut and re-struck), and the UI theme snaps over in 150 ms. Cooldowns, dwell times
and phrase waits are skipped. Keypress to fully switched (tempo ramp done and in the new range, new
kit, every playing layer replaced, old channels silent, new ones at full level, harmony restarted),
30 switches over 3 seeds: **0.55–1.06 s, median about 0.9 s**, down from 25–110 s before (several
didn't finish within 90 s). The *auto* style's own wanderings stay smooth, as described next.

**Auto transitions are musical.** The conductor announces the switch (`“alright folks, jazz club
tonight — swing it”`, `“Howdy folks! grab a partner — country night”`, `“Lord have mercy… let's play
the blues”`, `“lights down… the system is the DJ tonight”`). Every player first thinks
`conductor went Jazz Cat → listening, changing over at the phrase`. Bass and drums change over at
the next phrase, and the others over one or two phrases, each with its reason, e.g.
`conductor went Blues → picking up a fingered bass`,
`conductor went Blues → switching to the GM Brush kit with triangle & tambourine`. The tempo
ramps to the new style's range (at most 24 bpm per ramp) and the harmony picks up the new
templates at the next phrase. The UI fades to the new theme over about a second.

**The UI restyles with the style.** Each style has its own palette, border style, panel title
decorations and icons, meter glyphs, conductor outfit and props with their own animation, and
speech marks. A few examples:

- Country: sandy tan, brown and dusty orange; rope borders (`@~~~@`, `┊`); 🌵 ★ 🐎; cowboy hats for everyone and a lasso on the baton.
- Blues: deep indigo and smoky blue; a pork-pie hat and sunglasses (off when he's upset); ♪.
- Techno: neon black with cyan and magenta; borders and meters pulse on the kick; headphones and a DJ deck with spinning platters instead of a baton.
- Jazz: warm amber; a beret; 🎷.
- Biker: black and chrome orange; a bandana; flame meters.
- Baroque: cream and gold; powdered wigs `@@@@@` with curls `{ }`; ornate double borders.
- Celtic: greens; knotwork-ish borders (`✠∞∞✠`, `§`); a flat cap.
- Industrial: rust grey; a hard hat; hazard-stripe borders.
- Ambient Monk: soft lavender; a hood and robe; minimal borders.
- Space Cadet: deep-space palette; a helmet; 🚀.
- Gamelan: batik browns and golds; a `\\|||/` headdress.

| | | |
|---|---|---|
| ![Country](docs/styles/country.png) Country | ![Techno](docs/styles/techno.png) Techno | ![Blues](docs/styles/blues.png) Blues |
| ![Jazz Cat](docs/styles/jazz.png) Jazz Cat | ![Biker](docs/styles/biker.png) Biker | ![Baroque](docs/styles/baroque.png) Baroque |
| ![Celtic Session](docs/styles/celtic.png) Celtic Session | ![Industrial](docs/styles/industrial.png) Industrial | ![Ambient Monk](docs/styles/monk.png) Ambient Monk |
| ![Space Cadet](docs/styles/space.png) Space Cadet | ![Tribal / Gamelan](docs/styles/tribal.png) Tribal / Gamelan | ![House Band](docs/styles/house.png) House Band |

**Scripted key presses** (tests): `SYSJAM_KEYS="20:right,45:up,46:up,150:left"` presses arrows at
the given seconds. In the offline bench (`SYSJAM_BENCH=1`) these are seconds of music time and the
bench prints a `bench steering:` summary (switches, nudges, techno sections, blue-note bends). In
a live run they are wall-clock seconds. The real keys were tested through tmux `send-keys Up/Left`
(crossterm raw mode).

**Harmony with memory and purpose.** The chord engine remembers the last 32 chords, keys,
cadences and tension levels. Each phrase gets a goal (authentic, half, deceptive or plagal cadence,
a climax, or free drift), chosen from what came before: a half cadence gets answered, the same
cadence twice in a row gets tired, tension builds and resolves. A beam search (width 16) then plans
the phrase towards that goal with functional harmony (tonic, predominant, dominant, secondary
dominants, borrowed chords; tritone substitutions and backdoor cadences in Jazz; pivot chords
for modulations), scored for voice leading, tension arc, the style's habits and novelty: it avoids
recent chords and bigrams and penalises A-B-A-B ping-pong. Now and then it calls back an earlier
phrase, transposed. Forms (the Blues 12-bar, Country changes, Techno loops) keep their shape.
The header shows the plan, for example:

- `heading to D minor: Dm (pivot) → Bbmaj7 → A7 → Dm7`
- `plagal landing Gm7 → Dm7 in D minor: Bbmaj7 → Fmaj7 → Gm7 → Dm7`
- `callback to the phrase from 6 phrases ago: F7 → Bbmaj7 → C7 → Fmaj7`
- `cadence home to Fmaj7 in F major: C7 → Am7 → Bbmaj7 → Dm7 → Bbmaj7 → Gm7 → C7 → Fmaj7`

Repetition over 12 runs of 600 s (6 styles × 2 seeds, free harmony only): A-B-A-B rate 3.72% → 2.12%,
repeated 4-chord runs within 32 chords 6.9% → 1.5%, distinct chord bigrams per 5 minutes 39.8 → 46.3.

## The audience: your processes on the dance floor

Under the band there is a strip of floor with a little crowd: up to 16 of **your own processes**
(as many as fit, one per 12 columns, about 7 on a 170-column terminal), each a 5-row ASCII figure
with a short name tag such as `firefox` or `cargo` (exe name only, no pid; long names are cut to fit
the 11-cell slot).
Now and then the tag shows instead the name of a file the process has open (`notes.md`,
`spawn.log`): the base name only, never a path. At the right end is the bar (`🍺 ☕`, or `U c` in
ASCII). The panel title counts how many of your processes there are, how many are at the bar and
how many kernel threads stay backstage.

- **Who they are:** every exe name is hashed (FNV-1a) into a *persona*, so `firefox` always looks
  like the same person, on every run and every machine: hair or hat (spiky, bun, cap, beanie,
  top hat, mohawk, curls, bald …), clothes (hoodie, suit, stripes, dress, vest, jacket …), an
  accessory (headphones, earring, scarf, bag, a flower), height (short people have no knee row),
  and a dance personality (bouncer, swayer, stepper, headbanger, shuffler, twirler, wallflower).
  The style's theme (cowboy hat, glowsticks, wig) goes over the top. Their **faces change with the
  music and what they are doing**: `(*o*)`/`(^O^)` when the band is hot, `(O)` mouths in a surge,
  `(^v^)` applauding a like, `(>o>)` when the process is CPU-hungry, `(@_@)` busy on disk, `('o')`
  chatting, `(-.-)` dozing, `(^_^)`/`(-_-)` swaying to a chill groove, `(^-^)` waving goodbye, plus blinks.
- **Legs and feet, on the beat clock:** multi-frame cycles synced to the same beat clock as the
  band. A 4-frame walk cycle when coming in, going to the bar and leaving; foot taps on the beat;
  knee bounces; kicks, stomps and jumps with both feet off the ground (the whole figure lifts into
  the headroom row) during surges and hype. Each style has its footwork: a Country line-dance
  **grapevine** (the figure actually travels sideways) and heel-toe, a Techno **shuffle** and
  **running man**, a Blues **swing step**, Baroque **minuet** steps with pointed toes, a Biker
  **stomp**, a Celtic **jig** with high kicks. The musicians tap too, by style (country heel-toe,
  biker stomp, celtic eighths, blues on 2 and 4, baroque on beat 1, techno every beat), the
  **drummer's right foot comes down on the actual kick hits**, and the conductor shifts his
  weight every two beats and taps his foot (and bounces and stomps in surges).
- **Only when there is room:** the crowd panel shows by itself when the terminal is at least
  **160 columns × 55 rows** and disappears below that, coming back automatically when you
  resize. `a` forces it on or off.
- **Who comes:** a rotating, CPU-weighted sample. Processes that start after sysjam ("fresh"
  ones: compilers, shell commands, a browser) get priority and **walk in** from the edge; an idle
  one goes home to make room. When a process exits its figure **waves and walks out**. Short
  spawns (a `cc1`, a `sleep`) pop in for a few seconds and leave again. Fresh spawns are let in
  at most one per 4 s, and not a second copy of a name already on the floor, so a shell that
  spawns all the time can't flood it. Kernel threads are only counted, never shown.
- **What they do, from real numbers:** CPU-hungry processes (over 50% of a core) dance hard; a
  little CPU makes them groove; idle ones sway, and after 40 s they sit down (`z`). Processes
  doing storage I/O (over 200 kB/s from `/proc/<pid>/io`, `read_bytes + write_bytes`) carry a box
  `[#]` over their heads; processes with a lot of other I/O (sockets, pipes, ttys and page-cache
  reads: `rchar + wchar` minus the storage part, which is the cheap stand-in for "network" a
  process can have) chat (`..`). Idle ones wander off to the bar now and then, have a beer `U` or a
  coffee `c` for 6–14 s and come back (at most two at the bar).
- **What they do with the music:** when the band is alive and hot, in a surge, or the crowd is
  roaring, everyone puts their hands up and jumps on the beat (from the same beat clock as the
  conductor). When it's chilling they mingle and nod. A like gets a round of applause. By style:
  cowboy hats and a grapevine in Country, glowsticks and the running man in Techno, a swing step
  with lighters in Blues, wigs, minuets and a polite golf clap in Baroque. After `n` they cheer briefly.
- **And back (lightly):** the crowd's excitement (how many there are × how busy × how alive the
  band is, smoothed over 6 s) adds at most 0.06 to the band's energy input. Above 0.55 the
  conductor may say so ("the crowd's into it!", at most every 2 minutes) and gets a small lift
  of delight. When a notable process walks in (a browser, an editor, a compiler, anything over 30%
  CPU) a musician nods: `~ firefox walked in; the keys nods`.

**Cost.** It reuses the monitor's existing `/proc/<pid>/stat` walk (every 1.5 s, already there
for the "top threads" panel), plus one `stat` of `/proc/<pid>` for the owner of each new pid,
`/proc/<pid>/io` for the ≤16 people in the crowd, and every 6 s the `fd` links of one of them
(at most 48). Measured: 0.13 ms per census on top of the walk (≈0.01% of a core); the monitor
thread stayed at 0.10% and the UI thread at 0.40% with the crowd on (0.42% without it before the
terminal-output tweak), see [Performance](#performance).

**Privacy.** Process names and file names are only drawn (no pids any more) on your own screen; nothing is
logged, sent or stored. Only processes owned by your user are named, and only their file
descriptors are looked at (the kernel wouldn't show other users' anyway); other users' processes
and kernel threads are only counted. File tags are base names with no path, skipping `/dev`,
`/proc`, `/sys`, `/run`, `/usr`, hidden files, libraries, fonts, caches and lock files.
`--no-audience` turns the whole thing off (no scan at all); `a` forces the panel on or off. By
default it appears at 160×55 or larger (9 rows: headroom for jumps, 5-row figures, name tags).

**Width-safe.** The crowd is pure ASCII; a unit test renders 12 exe names × every style × every
activity × every vibe × 24 frames of the beat clock (walks, jumps, grapevines included) and checks
that every row is exactly 7 cells wide, and that at least 10 of the 12 names get different looks.

## The conductor's feelings and his tiny brain

The conductor has a continuous emotional state: valence and arousal, plus a blend of 16 moods
with inertia: content, delighted, proud, amused, inspired, nostalgic (while a theme is quoted),
focused, impatient, frustrated, annoyed, bored, sleepy, anxious (chaos), surprised (sudden spikes),
tender and melancholy. Once a bar a feature vector of 32 numbers describes the moment (energy,
agitation, clutter, density, overlap, loudness, arrangement state, chaos, breakdowns, quotes,
minor/major, tempo, CPU/input/net/disk, spikes, stubborn players, bars since he last stepped in,
how many play, surge, build, a recent like, the style's strictness, a fresh cadence, harmonic
tension).

**What the model is, honestly:** a tiny multilayer perceptron, 32 → 32 → 16 → 30 (tanh), 8,404
bytes of weights compiled into the binary (`assets/conductor.bin`). It is not a language model
and does not understand music; it is a small learned function. It was trained offline
(`tools/train_conductor.py`, Python + numpy) on synthetic episodes, labelled by the hand-written
heuristic conductor plus a set of emotional rules (`src/mood.rs`), so it imitates and smooths those
rules (held-out agreement: mood 94%, intervention 97%, speech intent 98%). Its outputs are mood
logits, an intervention preference (none, hush, lay out, cue down, invite, build) and a speech
intent (praise, scold, encourage, joke, reminisce, hype, soothe). The mood he shows is 70% the
model and 30% the rules, sharpened and moved towards slowly (inertia), with jolts for surprises and
likes. One evaluation takes about 1–3 µs, once a bar: about 0.0001% of one core. Set
`SYSJAM_NO_BRAIN=1` to use the rules alone.

The model never acts directly. Its intervention preference only nudges the thresholds of the
existing conductor (for example "hush" lowers the clutter threshold for hushing from 0.45 to
0.36; "invite" makes inviting someone back more likely; "build" asks the harmony for tension and
lifts the band), and the hard rules always win: the deep sub and bass are never hushed, no hiss,
the drums are laid out only when the band isn't alive, the synth sits back while the drums play
(unless they rest), and restraint is kept.

His mood shows in his face and marks (sparkles, a sweat drop, an anger vein `#`, a tear, `zZ`),
in the size of his beat, in what he says (about 1,460 distinct lines, voiced per style), and in his
decisions: frustrated or annoyed he is stricter, delighted or inspired he invites players back
more. The players notice (`uh oh, he's frowning — playing it safe`, `he's beaming — let's give him more`).

Example trajectories (offline bench, seed 4, Jazz Cat; idle 40 s, busy 80 s, idle 80 s):

```
   0s focused 0.42 → soothe
  27s sleepy 0.87 → soothe          (chilling, nothing happening)
  54s anxious 0.40 → invite someone (the machine gets busy: chaos)
 109s anxious 0.51 → invite keys
 123s content 0.41 → praise         (the band settles into it)
 136s content 0.87 → praise
 151s nostalgic 0.74 → reminisce    (a theme is quoted)
 186s sleepy 0.87 → soothe          (back to idle)
```

## Likes: the only thing sysjam learns from

Press **`l`** (or Enter) when you like what you hear. sysjam takes a snapshot of the moment
(style, tempo, arrangement state, key and mode, which instrument families are playing, how
dense it is, who is playing, whether a theme is being quoted) and makes a small reward-weighted
update to a table of preferences. The conductor is delighted (`“Yeehaw! glad you like it!”`) and a
`♥` flashes in the header. From then on he steers towards similar moments: *auto* picks styles you
liked more often, tempo choices lean to tempos you liked, players pick up instrument families you
liked, modes you liked come up more, and liked quotes come more often. It is a gentle bias, not a lock.

**Nothing else is learned**, except a small negative from each **`n`** (next idea): the moment you
skipped is stored under `skips:` in the same file and nudges the style, state, tempo, mode and
density weights down by 0.06 (one like is worth about +0.3), spreading it over the alternatives.
Instruments, players and quotes are not touched by a skip. sysjam does not learn from switching
styles with ←/→, from how long you listen, from your typing, or from any other behaviour; only
presses of `l` and `n` count. Everything stays local:
the likes and the learned weights are written to `~/.config/sysjam/likes.yaml` (or
`$XDG_CONFIG_HOME/sysjam/likes.yaml`), plain human-readable YAML with a timestamp per like, loaded
at startup, never sent anywhere. Edit it or delete it any time, or run `sysjam --reset-likes` to
forget everything.

```yaml
learned_bias:
  style:
    "celtic": 1.800
    "jazz": -0.164
  tempo:
    "110-129": 0.675
    "70-89": 0.300
  mode:
    "major": 1.200
skips:
  - time: "2026-10-15T21:03:10-05:00"
    style: "House Band"
    state: "chilling"
    ...
likes:
  - time: "2026-10-15T20:42:33-05:00"
    style: "Celtic Session"
    tempo: 80
    state: "awakening"
    key: "F major"
    ...
```

A scripted test: six likes during a Celtic Session run, then four 20-minute *auto* runs (seeds 4,
7, 9, 13) with and without that file. Without likes, Celtic got 0% of the time in all four; with them
29%, 26%, 28% and 0% (21% on average). Time at 70–89 bpm went from 15% to 45% on average,
and guitar picks (a liked family) from 14 to 29.

## Next idea, and never dead air

**`n` / Tab: next idea, now.** When it's going nowhere, press `n` (or Tab). Within about a beat
(measured 0.33–0.42 s from key to new harmony on screen), the band:
- jumps to a **new key** away from the recent ones (`next idea: modulation straight to F# minor`);
- starts a **new progression** and writes a **new motif**, new comping and new bass, drum and acid patterns;
- picks **new sounds for at least half of the band**;
- lifts any lay-outs, brings two melodic players straight in with full phrases, and the drummer plays a fill if he's in.

The conductor says `“right, moving on!”` (then `“ok — next idea!”`, `“fresh page, here we go”`, …),
the crowd cheers lightly, and the moment you skipped is stored as a small negative in your
likes file (see [likes](#likes-the-only-thing-sysjam-learns-from)). It also works offline: `SYSJAM_KEYS="30:next"`.

**Why there was dead time.** Measured in 20-minute offline auto runs (6 seeds, `SYSJAM_BENCH=1
SYSJAM_BENCH_SCALE=6`: 4 min idle, 8 min busy, 8 min idle), 70–84% of the no-melody time was in
*chilling*. Keys and colour rested, the lead was rarely invited, chill chords changed only every 4–5
bars, the conductor had a habit of "dropping to the drone", and *hold* notes were often swallowed.

**What changed.**
- A **floor**: at least one melodic voice playing and at least two of keys/lead/colour in, always;
  the drone/pad is always evolving (pad voices drift each bar, deeper LFO when chilling).
- **Restraint means fewer notes, not silence**: a lay-out comes with an invite (`keys, back to the
  pad — lead, you take it`), the drone cue became `“everyone, softer — let the drone breathe”`.
- **Lush chilling**: chords every 1–2 bars, more colour swells, rolled-chord holds, the lead calls in every phrase.
- A **stagnation detector** (same chords, same players, same sounds, flat energy) shows its reason
  and makes a change: a new section, a key change, a featured soloist, call and response, or a build
  (`getting stale — new section`), so something contrasting happens every 1–3 minutes.
- **No lone "dumb notes"**: a keys/lead/colour player only enters at a phrase start or pickup, with a
  complete statement (a 3+ note phrase with contour, no gap over a beat and a half, ending on a held
  chord tone, or a full comp pattern of at least a bar), and finishes it unless a style switch or `n`
  cuts in; thinning shortens phrases instead of deleting notes from them. Single hits are only for
  programs that make sense alone (bird tweet, seashore, tubular bells, glockenspiel, timpani, gongs,
  FX, reverse cymbal, long pad/drone swells), placed on strong beats or chord changes.

**Before → after** (same 6 seeds, 20 min each; *dead* = fewer than 2 voices or nothing new for 4 s;
an *orphan* is a melodic/comp note with no other note from the same player within 2 beats either side):

| metric | before | after |
|---|---|---|
| dead time | 21.6% (13.4–26.7) | **4.2%** (3.1–5.0) |
| no melodic voice | ~52% | 29.5% (25.9–33.8) |
| fewer than 2 players | ~38% | 5.6% (0–9.3) |
| stagnant bars | ~12% (4–25) | **0.9%** (0.5–1.8) |
| longest stretch without a section change | 99–217 s | 86–88 s |
| median gap between section changes | ~25 s (15–42) | 34 s (28–39) |
| orphan notes per 10 min | 47 (36–63), 15% of melodic/comp notes | **8** (5–15), 1.6% |

The bench prints these lines itself (`bench deadtime:`, `bench sections:`, `bench orphans:`), plus
why there was no melody (`keys off/lay/rest/hold/gated …`). `SYSJAM_NO_DEVELOP=1` turns the
stagnation detector off for comparison; `SYSJAM_ORPH_LOG=1` lists every orphan note.

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
- the arrangement state (chilling leans towards fewer notes, but never towards silence);
- the player's tied signal, and whether it just changed;
- how dense the rest of the band already is;
- novelty: a freshly picked-up sound counts in favour, while playing many phrases in a row ("fatigue") and doubling another player count against.

The reason shows in the thinking line, e.g. `band is busy enough (9 notes/bar) → nothing useful to add, resting`,
`holding one long chord tone under the drone`, or `my signal is up (net 92%) → worth playing`.

In **chilling**, players thin out by playing *fewer, shorter phrases*, not by going silent: there is
always at least one melodic voice and at least two of keys/lead/colour in, the pad keeps drifting,
chords change every 1–2 bars, colour swells come more often and the lead keeps calling (see
[never dead air](#next-idea-and-never-dead-air)).

Patterns are sparser overall:
- keys: mostly sparse stabs; arpeggios in quarters, eighths only when alive;
- lead: a two-bar call per phrase, a connected statement of 3+ notes ending on a held chord tone;
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

Keyboard and mouse are one combined **input activity** signal and meter. On most machines both
counts come from the same source anyway, so two meters would only show the same thing twice. The
header shows which source feeds it.

Intense typing makes the band pick up (and can trigger a surge), and mouse
bursts nudge it more gently. These are improvisational cues, not one click per
keypress. sysjam only counts **how many** input events happen. It never reads
which keys, and key codes are never stored or logged.

At startup sysjam logs which source it uses, and the TUI header shows it with
the combined activity meter (top right: `input evdev×2 activity ███▌`):

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
Key presses and pointer motion are added into one rate, smoothed over about 3 s and mapped to 0–1; about 6 keystrokes/s saturates it. It can add at most 0.45 to the bandleader's energy input, so they push but never dominate. `--no-input` turns all of it off, and nothing is opened.

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
| idle, TimGM6mb, styles + themed UI, **UI on** vs `--no-tui` (auto, side by side, 60 s, truecolor) | 1.82% vs 1.35% (UI thread 0.30%) | |
| idle, themed UI per style, whole process (60 s, truecolor): Techno / Country / Baroque | 2.16% / 2.08% / 1.53% (UI thread 0.31% / 0.28% / 0.27%, fades included) | |
| offline bench with scripted style switches and tempo nudges (13 key presses through 10 styles, incl. Techno build/drop) | 1.06% / 1.66% / 1.63% (idle / busy / idle) | |
| Techno, live animated UI (~16 fps figures, beat-locked frames), WAV sink, 40 s of a 75 s run, per thread | audio/engine thread 1.47%, **UI thread 0.45%**, monitor 0.10% (≈2.0% total) | |
| same, **with the audience** (crowd of 7 on the floor, its own census, cheaper terminal output) vs `--no-audience` (previous output path) | engine 1.27%, **UI 0.40%**, monitor 0.10% (≈1.8% total; census 0.13 ms per 1.5 s) vs UI 0.42% | |
| Techno / Space Cadet, live UI at 170×56 with the 5-row crowd, legs, beat-synced feet, next-idea + never-dead-air engine, WAV sink, 40 s of a 75 s run, per thread | engine 1.55% / 1.37%, **UI 0.45% / 0.42%**, monitor 0.12% (≈2.1% / 1.9% total) | |
| the conductor's MLP (once a bar, offline bench) | 1–3 µs per call, ≈0.0001% | |
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
- `tools/shot.sh` screenshots the TUI in tmux (optionally sending arrow keys) and renders it with `tools/ansi2png.py`, which carries tmux's SGR state across lines, draws double-width emoji, and falls back to Noto Sans Symbols and a monochrome Noto Emoji font (`SYSJAM_EMOJI_FONT`, OFL, not shipped) for glyphs DejaVu lacks. `LOAD=75 tools/shot.sh country 55 "--seed 3 --style country"` made the gallery.
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
