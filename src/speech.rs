//! What the conductor says: speech intent (from the brain) × mood × the style's voice.
//! Generic lines carry {pal} / {cheer} / {sigh} / {band} slots that each style fills
//! in its own way, plus a few lines that only one style would say.

use crate::dsp::Rng;

struct Voice {
    id: &'static str,
    pal: &'static [&'static str],
    cheer: &'static [&'static str],
    sigh: &'static [&'static str],
    band: &'static str,
    own: &'static [(usize, &'static str)],
}

const VOICES: &[Voice] = &[
    Voice { id: "house", pal: &["folks", "team", "friends"], cheer: &["nice!", "lovely", "yes!"], sigh: &["hmm", "well"], band: "band", own: &[] },
    Voice { id: "monk", pal: &["friends", "dear ones"], cheer: &["peace", "beautiful", "mm"], sigh: &["breathe", "mm…"], band: "circle", own: &[(7, "let the silence hold us"), (1, "the stillness sings"), (5, "this melody has walked a long road")] },
    Voice { id: "jazz", pal: &["cats", "baby", "daddy-o"], cheer: &["dig it!", "smokin'", "cookin' now"], sigh: &["man…", "oh brother"], band: "combo", own: &[(4, "that's not a wrong note, that's a choice"), (6, "take it to the bridge, cats!"), (2, "cool it, cat — less is more")] },
    Voice { id: "blues", pal: &["children", "brother", "sister"], cheer: &["Lord have mercy!", "oh yeah", "sing it"], sigh: &["mm-mm", "Lord…"], band: "band", own: &[(7, "we all got the blues sometimes"), (5, "that one takes me back down to the Delta"), (1, "now that's the real thing")] },
    Voice { id: "country", pal: &["y'all", "partner", "folks"], cheer: &["yeehaw!", "hot dang!", "well alright"], sigh: &["shucks", "aw heck"], band: "outfit", own: &[(4, "slicker'n a greased pig, that lick"), (2, "whoa there, rein it in, partner"), (6, "grab your partner, here we go!")] },
    Voice { id: "biker", pal: &["riders", "brothers", "crew"], cheer: &["hell yeah!", "full throttle!", "ride on"], sigh: &["ugh", "c'mon"], band: "pack", own: &[(6, "open the throttle!"), (2, "ease off the gas, rider"), (4, "that riff's got chrome on it")] },
    Voice { id: "baroque", pal: &["gentlemen", "signori", "my dears"], cheer: &["bravo!", "magnifico", "splendid"], sigh: &["mon dieu", "alas"], band: "ensemble", own: &[(2, "decorum, if you please — fewer ornaments"), (1, "a cadence worthy of the Elector"), (5, "ah, a melody from simpler times")] },
    Voice { id: "celtic", pal: &["lads", "lassies", "friends"], cheer: &["sláinte!", "grand stuff", "lovely"], sigh: &["ach", "och"], band: "session", own: &[(4, "that reel's a wee bit drunk"), (6, "another round of that tune!"), (5, "me granny hummed that one")] },
    Voice { id: "industrial", pal: &["maggots", "machines", "you lot"], cheer: &["LOUDER.", "again!", "YES. HARDER."], sigh: &["no.", "break it."], band: "machine", own: &[(2, "too clean — break it."), (1, "that's the wall. hold it."), (6, "LOUDER. LOUDER."), (3, "again! harder!"), (7, "quiet… let it seethe")] },
    Voice { id: "techno", pal: &["ravers", "people", "fam"], cheer: &["let's go!", "hands up!", "bang"], sigh: &["ugh", "oof"], band: "floor", own: &[(6, "wait for it… wait for it…"), (2, "too many layers — strip it back"), (4, "that hi-hat just winked at me")] },
    Voice { id: "space", pal: &["crew", "cadets", "star-folk"], cheer: &["stellar!", "out of this world", "nominal and gorgeous"], sigh: &["houston…", "static"], band: "crew", own: &[(7, "drifting in zero-g"), (6, "engage the boosters!"), (5, "a signal from an old star")] },
    Voice { id: "tribal", pal: &["friends", "drummers", "family"], cheer: &["yes!", "the spirits dance", "beautiful"], sigh: &["hmm", "slowly"], band: "circle", own: &[(1, "the circle is strong today"), (7, "let the gongs breathe"), (6, "faster, the fire is rising!")] },
];

/// generic lines per intent (index = mood::INTENTS), each with mood colouring
const LINES: [&[&str]; 8] = [
    &[],
    &["{cheer} {pal}, that's the sound", "{cheer} — keep it right there", "that's it, {pal}, beautiful", "now we're a {band}", "{cheer} I could listen to this all night", "lovely blend, {pal}", "that cadence landed just right", "{cheer} every part in its place"],
    &["{sigh}, too much, {pal}", "{pal}, listen to each other", "{sigh} — who's stepping on whom?", "not everybody at once, {pal}", "less, {pal}, less", "I can't hear the melody, {pal}", "{sigh}… the {band} is crowding up", "space, {pal}, give it space"],
    &["come on, {pal}, wake up a little", "give me something, {pal}", "let's move it along", "{pal}, find a new idea", "same old, same old… surprise me", "a little more, {pal}", "let's go somewhere, {pal}"],
    &["{cheer} who taught you that?", "ha — cheeky, {pal}", "that note had a sense of humour", "the computer is dancing, {pal}", "I see what you did there", "now that's a {band} with jokes"],
    &["ah… I remember this one", "an old friend of a tune, {pal}", "takes me back, {pal}", "they played this long before us", "the old songs still have it", "listen — history"],
    &["{cheer} here it comes!", "bigger, {pal}, BIGGER", "now we fly!", "everybody up, {pal}!", "{cheer} feel that lift?", "build it, {pal}, build it"],
    &["easy, {pal}… easy", "shh, let it settle", "breathe with me, {pal}", "nice and soft now", "slow down, it's alright", "rest a moment, {pal}", "gently, {pal}"],
];

pub fn line(rng: &mut Rng, intent: usize, style_id: &str) -> String {
    let v = VOICES.iter().find(|v| v.id == style_id).unwrap_or(&VOICES[0]);
    let own: Vec<&str> = v.own.iter().filter(|o| o.0 == intent).map(|o| o.1).collect();
    if !own.is_empty() && rng.chance(0.3) {
        return own[rng.below(own.len())].to_string();
    }
    let pool = LINES[intent.min(7)];
    if pool.is_empty() {
        return String::new();
    }
    let t = pool[rng.below(pool.len())];
    let pick = |rng: &mut Rng, a: &[&str]| a[rng.below(a.len())].to_string();
    let pal = pick(rng, v.pal);
    let cheer = pick(rng, v.cheer);
    let sigh = pick(rng, v.sigh);
    let s = t.replace("{pal}", &pal).replace("{cheer}", &cheer).replace("{sigh}", &sigh).replace("{band}", v.band);
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s,
    }
}

/// reacting to a like
pub fn liked(rng: &mut Rng, style_id: &str) -> String {
    let v = VOICES.iter().find(|v| v.id == style_id).unwrap_or(&VOICES[0]);
    let c = v.cheer[rng.below(v.cheer.len())];
    let opts = ["glad you like it!", "you like this one? we'll remember", "thank you — more of this, then", "noted, with love"];
    format!("{c} {}", opts[rng.below(opts.len())])
}

/// how many distinct lines the bank can produce for a style (README / report)
pub fn variety() -> usize {
    let mut n = 0;
    for v in VOICES {
        for l in LINES.iter() {
            for t in l.iter() {
                let mut k = 1;
                if t.contains("{pal}") {
                    k *= v.pal.len();
                }
                if t.contains("{cheer}") {
                    k *= v.cheer.len();
                }
                if t.contains("{sigh}") {
                    k *= v.sigh.len();
                }
                n += k;
            }
        }
        n += v.own.len();
    }
    n
}
