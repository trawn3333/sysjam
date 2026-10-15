#!/usr/bin/env python3
"""Extract short public-domain melodic motifs from the music21 corpus for sysjam.

Usage:  python3 tools/extract_themes.py [out=assets/themes/themes.txt]

Needs music21 (pip install --user music21). The Rust binary does NOT need Python:
the output is a small text file embedded with include_str!.

Only sources whose *compositions* are public domain and whose corpus files carry
no restrictive notice are read (see assets/themes/LICENSE). From each tune we keep
only melodic facts of an incipit (2-8 bars): scale degrees relative to the tonic,
durations, mode, meter, title and citation. No encoding text, layout, fingering or
annotation is copied.

Line format (one theme per line, '#' = comment):
  title|composer|catalogue|collection|corpus path|mode|meter|pickup|notes
  pickup = position of the first note inside its bar, in 1/12 quarter notes
  notes  = space separated  DEG[#|b|x|bb]:LEN  or  r:LEN  (rest)
           DEG = diatonic degree from the tonic (0 = tonic, 7 = octave up, -1 = below)
           LEN = duration in 1/12 quarter notes
"""
import re
import sys
import warnings
from pathlib import Path

warnings.filterwarnings("ignore")
from music21 import corpus, converter, expressions, key as m21key, meter, note, chord, pitch  # noqa: E402
from music21.corpus import chorales as m21chorales  # noqa: E402

CORPUS = Path(corpus.__file__).parent
STEPS = {
    "major": [0, 2, 4, 5, 7, 9, 11],
    "minor": [0, 2, 3, 5, 7, 8, 10],
    "dorian": [0, 2, 3, 5, 7, 9, 10],
    "mixolydian": [0, 2, 4, 5, 7, 9, 10],
    "phrygian": [0, 1, 3, 5, 7, 8, 10],
    "lydian": [0, 2, 4, 6, 7, 9, 11],
}
MAX_NOTES = 28
MIN_NOTES = 5

# ----------------------------------------------------------------- curation
BACH_FAMOUS = ["Wachet auf", "Ein feste Burg", "O Haupt voll Blut", "Herzlich tut mich verlangen", "Jesu, meine Freude",
               "Nun danket alle Gott", "Wie schön leuchtet", "Befiehl du deine Wege", "Wer nur den lieben Gott",
               "Christ lag in Todesbanden", "Vom Himmel hoch", "Valet will ich", "Werde munter", "Allein Gott in der Höh",
               "Nun komm, der Heiden", "Ach Gott, vom Himmel", "Jesu, der du meine Seele", "O Welt, ich muss",
               "Was Gott tut", "Schmücke dich", "Liebster Jesu", "Freu dich sehr", "Vater unser", "Aus meines Herzens",
               "Christus, der ist mein Leben", "Komm, Gott Schöpfer", "Es ist genug", "Jesu, meines Lebens", "Ermuntre dich",
               "Wo soll ich fliehen", "Gelobet seist du", "In dich hab' ich gehoffet", "Meinen Jesum lass' ich nicht",
               "Von Gott will ich nicht lassen", "Durch Adams Fall", "Erhalt uns, Herr", "Herr Christ, der ein'ge"]
RYAN_FAMOUS = ["soldier's joy", "devil's dream", "arkansas", "fisher's hornpipe", "sailor's hornpipe", "college hornpipe",
               "rickett", "haste to the wedding", "irish washerwoman", "garry owen", "garryowen", "rakes of mallow",
               "mcleod", "flowers of edinburgh", "speed the plough", "durang", "money musk", "chorus jig", "petronella",
               "lamplighter", "bonaparte", "st. patrick", "off she goes", "fairy dance", "mountain hornpipe",
               "liverpool", "harvest home", "zip coon", "turkey in the straw", "pop goes the weasel", "campbells are coming",
               "lady of the lake", "opera reel", "mason's apron", "drowsy maggie", "rose tree", "girl i left behind",
               "yankee doodle", "hull's victory", "paddy whack", "tullochgorum", "village hornpipe", "wake up susan",
               "larry o'gaff", "jenny lind", "highland fling", "rory o'more", "kitty o'neil", "haymakers", "swallow",
               "st. anne", "wind that shakes", "morning star", "ten penny", "chicken reel", "rocky road", "blackbird",
               "white cockade", "marquis of huntly", "mrs. mcleod", "cuckoo's nest", "boys of blue hill", "staten island",
               "democratic rage", "president garfield", "golden slipper", "steamboat", "whiskey before breakfast",
               "fireman", "temperance reel", "miss johnson", "lord mcdonald", "duke of gordon", "bird in the bush",
               "galway", "kerry", "limerick", "dublin", "connaught", "cork", "londonderry", "belfast", "hornpipe"]
ONEILL_FAMOUS = ["londonderry", "coolin", "eileen aroon", "last rose", "minstrel boy", "believe me", "kathleen",
                 "snowy-breasted", "wearing of the green", "rakes of kildare", "kesh", "morrison", "banish misfortune",
                 "blackbird", "garden of daisies", "rocky road", "drops of brandy", "fox hunter", "silver spear",
                 "bucks of oranmore", "sligo maid", "mason's apron", "star of munster", "lark in the morning",
                 "kid on the mountain", "swallow's tail", "tarbolton", "harvest home", "dark rosaleen", "roisin dubh",
                 "brian boru", "o'donnell abu", "let erin remember", "planxty", "carolan", "lord inchiquin", "sheebeg",
                 "fanny power", "the coolun", "lament", "humours of", "the rose", "maid of", "boys of", "wind that shakes",
                 "irish washerwoman", "garryowen", "haste to the wedding", "frost is all over", "connaughtman"]
AIRD_FAMOUS = ["yankee doodle", "white cockade", "lochaber", "roslin castle", "flowers of the forest", "gilderoy",
               "corn rig", "maggie lauder", "dundee", "highland laddie", "duncan gray", "lang syne", "tullochgorum",
               "invermay", "patie's mill", "caledonian hunt", "hey tutti", "logan water", "waly", "blue bell",
               "gowrie", "soldier's joy", "rule britannia", "over the water", "within a mile", "jockey", "steer her up",
               "banks of", "bonny", "bonnie", "lass of", "mill mill o", "the birks", "auld rob", "the boatman",
               "my ain kind", "johnny cope", "cock o' the north", "fy let us", "corn riggs", "gala water"]
QUOTA = {"bach": 120, "ryansMammoth": 110, "oneills1850": 60, "airdsAirs": 40}

# hand-curated classical movements: (path, composer, title, catalogue, melody part index)
CLASSICAL = []
for n in range(1, 5):
    CLASSICAL.append((f"beethoven/opus18no1/movement{n}.mxl", "L. van Beethoven", f"String Quartet in F, mvt. {'I II III IV'.split()[n-1]}", "Op. 18 No. 1", 0))
for no in (3, 4, 5):
    CLASSICAL.append((f"beethoven/opus18no{no}.mxl", "L. van Beethoven", "String Quartet (opening)", f"Op. 18 No. {no}", 0))
for no in (1, 2, 3):
    for n in range(1, 5):
        CLASSICAL.append((f"beethoven/opus59no{no}/movement{n}.mxl", "L. van Beethoven", f"'Razumovsky' Quartet No. {no}, mvt. {'I II III IV'.split()[n-1]}", f"Op. 59 No. {no}", 0))
CLASSICAL += [("beethoven/opus74.mxl", "L. van Beethoven", "'Harp' Quartet (opening)", "Op. 74", 0),
              ("beethoven/opus132.mxl", "L. van Beethoven", "String Quartet in A minor (opening)", "Op. 132", 0),
              ("beethoven/opus133.mxl", "L. van Beethoven", "Große Fuge", "Op. 133", 0)]
for n in range(1, 6):
    CLASSICAL.append((f"haydn/opus1no1/movement{n}.mxl", "J. Haydn", f"String Quartet in B-flat, mvt. {'I II III IV V'.split()[n-1]}", "Op. 1 No. 1, Hob. III:1", 0))
for n in range(1, 5):
    CLASSICAL.append((f"haydn/opus74no1/movement{n}.mxl", "J. Haydn", f"String Quartet in C, mvt. {'I II III IV'.split()[n-1]}", "Op. 74 No. 1, Hob. III:72", 0))
for k, nm, cnt in [("k80", "String Quartet in G 'Lodi'", 4), ("k155", "String Quartet in D", 3), ("k156", "String Quartet in G", 4), ("k458", "'Hunt' Quartet in B-flat", 4)]:
    for n in range(1, cnt + 1):
        CLASSICAL.append((f"mozart/{k}/movement{n}.mxl", "W.A. Mozart", f"{nm}, mvt. {'I II III IV'.split()[n-1]}", f"K. {k[1:]}", 0))
CLASSICAL += [("mozart/k545/movement1_exposition.mxl", "W.A. Mozart", "Piano Sonata in C 'facile', mvt. I", "K. 545", 0),
              ("handel/rinaldo/Lascia_chio_pianga.mxl", "G.F. Handel", "Lascia ch'io pianga (Rinaldo)", "HWV 7", None),
              ("weber/concertino_clarinet.mxl", "C.M. von Weber", "Clarinet Concertino", "Op. 26, J. 109", 0)]

COLL = {
    "bach": "music21 corpus (Bach chorales)",
    "ryansMammoth": "Ryan's Mammoth Collection, 1883 · music21 corpus",
    "oneills1850": "O'Neill's Music of Ireland, 1903 · music21 corpus",
    "airdsAirs": "Aird's Selection of Airs, 1782-97 · music21 corpus",
    "classical": "music21 corpus",
}


# ------------------------------------------------------------------ helpers
def clean(s):
    return re.sub(r"\s+", " ", (s or "").replace("|", "/")).strip()


def titlecase(t):
    t = clean(t).rstrip(".")
    t = re.split(r"\.\s+(?=[A-Z][a-z])", t)[0]  # "THE COOLIN. With Variations" -> "THE COOLIN"
    words = []
    for w in t.split():
        if len(w) > 1 and w.isupper():
            w = w.capitalize() if not w.startswith("O'") else "O'" + w[2:].capitalize()
            if w in ("Of", "The", "And", "In", "On", "To", "A") and words:
                w = w.lower()
        words.append(w)
    t = " ".join(words).rstrip(".,")
    return t[:1].upper() + t[1:]


def melody_events(part):
    """(offset, Pitch or None, ql, fermata) for the top line, ties merged, graces dropped."""
    out = []
    for n in part.stripTies().flatten().notesAndRests:
        ql = float(n.duration.quarterLength)
        if ql <= 0:
            continue
        ferm = any(isinstance(e, expressions.Fermata) for e in getattr(n, "expressions", []))
        if isinstance(n, chord.Chord):
            p = max(n.pitches, key=lambda x: x.ps)
        elif isinstance(n, note.Note):
            p = n.pitch
        else:
            p = None
        out.append((float(n.offset), p, ql, ferm))
    # merge overlapping events (voices): keep the highest note at each offset
    out.sort(key=lambda e: (e[0], -(e[1].ps if e[1] else -1)))
    merged = []
    for e in out:
        if merged and abs(merged[-1][0] - e[0]) < 1e-6:
            continue
        if merged and merged[-1][0] + merged[-1][2] > e[0] + 1e-6:
            o, p, ql, f = merged[-1]
            merged[-1] = (o, p, e[0] - o, f)
        merged.append(e)
    return merged


def bar_info(part):
    ts = part.recurse().getElementsByClass(meter.TimeSignature).first()
    if ts is None:
        ts = meter.TimeSignature("4/4")
    bar = float(ts.barDuration.quarterLength)
    ms = list(part.getElementsByClass("Measure"))
    bar1 = 0.0
    if len(ms) > 1 and float(ms[0].duration.quarterLength) < bar - 0.01:
        bar1 = float(ms[1].offset)
    return ts.ratioString, bar, bar1


def incipit(part, stop_at_fermata=False):
    ev = melody_events(part)
    meter_s, bar, bar1 = bar_info(part)
    while ev and ev[0][1] is None:
        ev.pop(0)
    if not ev:
        return None
    bars = 8 if bar <= 2 else (4 if bar <= 4.01 else 2)
    end = bar1 + bars * bar
    sel = []
    for e in ev:
        if e[0] >= end - 1e-6:
            break
        o, p, ql, f = e
        ql = min(ql, end - o)
        sel.append((o, p, ql, f))
        if stop_at_fermata and f and len(sel) >= MIN_NOTES:
            break
        if len([x for x in sel if x[1] is not None]) >= MAX_NOTES:
            break
    while sel and sel[-1][1] is None:
        sel.pop()
    if len([x for x in sel if x[1] is not None]) < MIN_NOTES:
        return None
    pickup = (sel[0][0] - bar1) % bar
    return meter_s, pickup, sel


def degrees(sel, tonic_name, mode):
    steps = STEPS[mode]
    first = next(p for _, p, _, _ in sel if p is not None)
    t = pitch.Pitch(tonic_name)
    t.octave = first.octave
    while t.diatonicNoteNum > first.diatonicNoteNum:
        t.octave -= 1
    while t.diatonicNoteNum + 7 <= first.diatonicNoteNum:
        t.octave += 1
    toks = []
    for _, p, ql, _ in sel:
        ln = max(1, round(ql * 12))
        if p is None:
            toks.append(f"r:{ln}")
            continue
        d = p.diatonicNoteNum - t.diatonicNoteNum
        exp = t.ps + 12 * (d // 7) + steps[d % 7]
        alt = int(round(p.ps - exp))
        if abs(alt) > 2:
            return None
        a = {0: "", 1: "#", -1: "b", 2: "x", -2: "bb"}[alt]
        toks.append(f"{d}{a}:{ln}")
    return " ".join(toks)


ABC_MODES = [("mix", "mixolydian"), ("dor", "dorian"), ("phr", "phrygian"), ("lyd", "lydian"), ("aeo", "minor"),
             ("min", "minor"), ("ion", "major"), ("maj", "major"), ("m", "minor")]


def abc_key(k):
    k = k.strip().split("%")[0].strip()
    m = re.match(r"([A-Ga-g])([#b]?)\s*([A-Za-z]*)", k)
    if not m:
        return None
    ton = m.group(1).upper() + m.group(2).replace("b", "-")
    suf = m.group(3).lower()
    if suf.startswith("hp") or suf == "none":
        return None
    mode = "major"
    for pre, md in ABC_MODES:
        if suf.startswith(pre):
            mode = md
            break
    return ton, mode


def abc_tunes(path):
    """split an ABC file into (header, X, title, K, text) blocks"""
    txt = Path(path).read_text(errors="replace")
    parts = re.split(r"(?m)^(?=X:)", txt)
    header = parts[0] if not parts[0].startswith("X:") else ""
    for blk in parts:
        if not blk.startswith("X:"):
            continue
        x = re.search(r"^X:\s*(\d+)", blk, re.M)
        t = re.search(r"^T:\s*(.*)$", blk, re.M)
        kk = re.search(r"^K:\s*(.*)$", blk, re.M)
        if not (x and t and kk):
            continue
        yield header, int(x.group(1)), t.group(1), kk.group(1), blk


def line(title, comp, cat, coll, src, mode, meter_s, pickup, notes):
    return "|".join([clean(title), clean(comp), clean(cat), clean(coll), src, mode, meter_s, str(round(pickup * 12)), notes])


def famous_rank(title, famous):
    tl = title.lower()
    for i, f in enumerate(famous):
        if f.lower() in tl:
            return i
    return None


# ------------------------------------------------------------------ sources
def do_bach(out, seen):
    rk = m21chorales.ChoraleListRKBWV().byBWV
    cl = m21chorales.ChoraleList().byBWV
    cands = []
    for f in sorted((CORPUS / "bach").glob("bwv*.mxl")):
        stem = f.stem[3:]
        if "-" in stem:
            continue  # alternative versions
        info = rk.get(stem) or (cl.get(int(stem)) if stem.isdigit() else None)
        if not info:
            continue
        title = info["title"]
        r = famous_rank(title, BACH_FAMOUS)
        cands.append((r if r is not None else 999, info.get("riemenschneider") or 999, stem, title, f))
    cands.sort()
    n = 0
    for _, _, stem, title, f in cands:
        if n >= QUOTA["bach"]:
            break
        try:
            s = converter.parse(str(f))
            k = s.analyze("key")
            sop = s.parts[0]
            inc = incipit(sop, stop_at_fermata=True)
            if not inc:
                continue
            mode = "minor" if k.mode == "minor" else "major"
            notes = degrees(inc[2], k.tonic.name, mode)
        except Exception as e:  # noqa: BLE001
            print("skip", f.name, e, file=sys.stderr)
            continue
        if not notes:
            continue
        sig = " ".join(t.split(":")[0] for t in notes.split()[:8])
        if sig in seen:
            continue
        seen.add(sig)
        out.append(line(title, "J.S. Bach", f"BWV {stem}", COLL["bach"], f"bach/{f.name}", mode, inc[0], inc[1], notes))
        n += 1
    return n


def do_abc(dirname, famous, out, seen, comp="trad."):
    cands = []
    for f in sorted((CORPUS / dirname).glob("*.abc")):
        for header, x, t, kk, blk in abc_tunes(f):
            t0 = re.split(r"\s+--\s+", t)[0]
            title = titlecase(t0)
            kind = ""
            m = re.search(r"--\s*(.*)$", t)
            if m:
                kind = m.group(1).strip().lower().rstrip(".")
            r = famous_rank(title, famous)
            cands.append((r if r is not None else 999, f, x, title, kind, kk, header, blk))
    # famous first; then the rest in book order
    cands.sort(key=lambda c: (c[0], str(c[1]), c[2]))
    n = 0
    for r, f, x, title, kind, kk, header, blk in cands:
        if n >= QUOTA[dirname]:
            break
        km = abc_key(kk)
        if not km:
            continue
        try:
            s = converter.parse(header + "\n" + blk, format="abc")
            part = s.parts[0] if hasattr(s, "parts") and len(s.parts) else s
            inc = incipit(part)
            if not inc:
                continue
            notes = degrees(inc[2], km[0], km[1])
        except Exception as e:  # noqa: BLE001
            print("skip", f.name, x, e, file=sys.stderr)
            continue
        if not notes:
            continue
        sig = " ".join(t.split(":")[0] for t in notes.split()[:8])
        if sig in seen:
            continue
        seen.add(sig)
        if dirname == "oneills1850":
            cat = f"O'Neill no. {x}"
        elif dirname == "airdsAirs":
            cat = f"Aird vol. {f.stem[-1]} no. {x}"
        else:
            cat = kind
        out.append(line(title, comp, cat, COLL[dirname], f"{dirname}/{f.name}#X{x}", km[1], inc[0], inc[1], notes))
        n += 1
    return n


def do_classical(out, seen):
    n = 0
    for path, comp, title, cat, pi in CLASSICAL:
        f = CORPUS / path
        if not f.exists():
            print("missing", path, file=sys.stderr)
            continue
        try:
            s = converter.parse(str(f))
            parts = list(s.parts)
            if pi is None:
                # the voice: the part with the most lyrics, else the first
                pi = max(range(len(parts)), key=lambda i: sum(1 for x in parts[i].recurse().notes if x.lyrics))
            part = parts[pi]
            head = s.measures(0, 12)
            k = head.analyze("key")
            mode = "minor" if k.mode == "minor" else "major"
            inc = incipit(part)
            if not inc:
                continue
            notes = degrees(inc[2], k.tonic.name, mode)
        except Exception as e:  # noqa: BLE001
            print("skip", path, e, file=sys.stderr)
            continue
        if not notes:
            continue
        sig = " ".join(t.split(":")[0] for t in notes.split()[:8])
        if sig in seen:
            continue
        seen.add(sig)
        out.append(line(title, comp, cat, COLL["classical"], path, mode, inc[0], inc[1], notes))
        n += 1
    return n


def main():
    dest = Path(sys.argv[1] if len(sys.argv) > 1 else "assets/themes/themes.txt")
    out, seen, counts = [], set(), {}
    counts["Bach chorales"] = do_bach(out, seen)
    counts["classical (Beethoven, Haydn, Mozart, Handel, Weber)"] = do_classical(out, seen)
    counts["Ryan's Mammoth Collection"] = do_abc("ryansMammoth", RYAN_FAMOUS, out, seen)
    counts["O'Neill's Music of Ireland"] = do_abc("oneills1850", ONEILL_FAMOUS, out, seen)
    counts["Aird's Airs"] = do_abc("airdsAirs", AIRD_FAMOUS, out, seen)
    hdr = ["# sysjam themes — generated by tools/extract_themes.py from the music21 corpus (music21 "
           + __import__("music21").__version__ + ")",
           "# public-domain compositions only; melodic facts of short incipits. See assets/themes/LICENSE.",
           "# title|composer|catalogue|collection|corpus path|mode|meter|pickup|notes"]
    hdr += [f"# {k}: {v}" for k, v in counts.items()]
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text("\n".join(hdr + out) + "\n")
    print(f"{len(out)} themes, {dest.stat().st_size} bytes → {dest}")
    for k, v in counts.items():
        print(f"  {k}: {v}")


if __name__ == "__main__":
    main()
