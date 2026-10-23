// Narration with ElevenLabs: one clip per scene, with word timings for captions.
//
//   cd video && node voice.mjs
//
// The API key is read from ~/.config/elevenlabs/api_key and only sent to ElevenLabs.
// Voice: Jessica Anne Bogart, id lxYfHSkYm1EzQzGhdbfc. Model: eleven_v4 (override with MODEL).
// Spoken spellings stay in SAY below: Omacus is "O-mah-cus", Omarchy is "Omahchee".
// Captions keep the spelling in each scene's `vo`.
//
// Writes, for every scene id in composition.js:
//   video/build/vo/<id>.mp3
//   video/build/vo/<id>.json
//   video/build/timings.json
//
// Clips are cached by text and voice, so re-running costs nothing unless a line changed.
//   VOICE="Name" node voice.mjs              a voice from your own library
//   VOICE_ID=... MODEL=... node voice.mjs    another voice id or model
import { readFile, writeFile, mkdir, access } from "node:fs/promises";
import { createHash } from "node:crypto";
import { homedir } from "node:os";

const API = "https://api.elevenlabs.io/v1";
const key = (await readFile(`${homedir()}/.config/elevenlabs/api_key`, "utf8")).trim();
const headers = { "xi-api-key": key, "content-type": "application/json" };
const BUILD = new URL("build/", import.meta.url).pathname;
await mkdir(`${BUILD}vo`, { recursive: true });

// The script lives in composition.js; read the scene list from it.
const src = await readFile(new URL("composition.js", import.meta.url), "utf8");
// A scene can set `say` (what's spoken, e.g. with audio tags) and `stab` (stability); captions keep `vo`.
const scenes = [...src.matchAll(/\{ id: "([\w-]+)",([^}]*?)vo: "([^"]+)" \}/g)].map((m) => ({
  id: m[1],
  vo: m[3],
  say: m[2].match(/say: "([^"]+)"/)?.[1],
  stab: Number(m[2].match(/stab: ([\d.]+)/)?.[1] ?? NaN),
}));

// Names the voice gets wrong, spelled the way they're said. Captions keep the real spelling.
const SAY = { Omarchy: "Omahchee", Omacus: "O-mah-cus" };
const spoken = (text) => Object.entries(SAY).reduce((t, [w, say]) => t.replaceAll(w, say), text);
const unspoken = (word) => Object.entries(SAY).reduce((w, [real, say]) => w.replaceAll(say, real), word);

const voices = (await (await fetch(`${API}/voices`, { headers })).json()).voices ?? [];
if (!voices.length) throw new Error("couldn't list voices; check the API key");
// The narrator: Jessica Anne Bogart from the shared Voice Library, on eleven_v4.
// VOICE="Name" picks one from your own voices instead; VOICE_ID any voice by id.
const NARRATOR = { name: "Jessica Anne Bogart", voice_id: "lxYfHSkYm1EzQzGhdbfc" };
const named = process.env.VOICE && voices.find((v) => v.name.split(" ")[0].toLowerCase() === process.env.VOICE.toLowerCase());
if (process.env.VOICE && !named) throw new Error(`no voice named ${process.env.VOICE}`);
const voice = named ?? (process.env.VOICE_ID ? { name: process.env.VOICE_ID, voice_id: process.env.VOICE_ID } : NARRATOR);
const MODEL = process.env.MODEL ?? "eleven_v4";
console.log(`voices: ${voices.map((v) => v.name.split(" ")[0]).join(", ")}`);
console.log(`using: ${voice.name} (${voice.voice_id})`);

// eleven_v3 takes only stability (0 creative, 0.5 natural, 1 robust).
const V3 = /^eleven_v[34]/.test(MODEL); // v3 and v4 take only stability, no context
const base = V3
  ? { stability: 0.5 }
  : { stability: 0.42, similarity_boost: 0.8, style: 0.28, use_speaker_boost: true, speed: 1.0 };
const timings = {};

for (const [i, s] of scenes.entries()) {
  const settings = Number.isNaN(s.stab) ? base : { ...base, stability: s.stab };
  const text = spoken(s.say ?? s.vo);
  const hash = createHash("sha1").update(JSON.stringify([text, voice.voice_id, MODEL, settings])).digest("hex").slice(0, 10);
  const mp3 = `${BUILD}vo/${s.id}.mp3`;
  const meta = `${BUILD}vo/${s.id}.json`;
  let cached = null;
  try {
    await access(mp3);
    cached = JSON.parse(await readFile(meta, "utf8"));
  } catch {}
  if (cached?.hash !== hash) {
    const res = await fetch(`${API}/text-to-speech/${voice.voice_id}/with-timestamps?output_format=mp3_44100_128`, {
      method: "POST",
      headers,
      body: JSON.stringify({
        text,
        model_id: MODEL,
        voice_settings: settings,
        // Neighbouring lines keep the delivery continuous across clips (not on eleven_v3).
        ...(!V3 && {
          previous_text: scenes[i - 1] && spoken(scenes[i - 1].vo),
          next_text: scenes[i + 1] && spoken(scenes[i + 1].vo),
        }),
      }),
    });
    if (!res.ok) throw new Error(`${s.id}: ${res.status} ${await res.text()}`);
    const out = await res.json();
    await writeFile(mp3, Buffer.from(out.audio_base64, "base64"));
    let ws = words(out.alignment).map((w) => ({ ...w, w: unspoken(w.w) }));
    // With `say`, show the caption's own words when they line up one to one.
    const shown = s.vo.split(" ");
    if (s.say && shown.length === ws.length) ws = ws.map((w, k) => ({ ...w, w: shown[k] }));
    cached = { hash, words: ws };
    await writeFile(meta, JSON.stringify(cached));
    console.log(`generated ${s.id}`);
  } else {
    console.log(`cached    ${s.id}`);
  }
  const last = cached.words.at(-1);
  timings[s.id] = { duration: last.t1, words: cached.words };
}
await writeFile(`${BUILD}timings.json`, JSON.stringify(timings, null, 1));
console.log("wrote build/timings.json");

// Character timings to word timings. Audio tags like [warm] aren't words, so they're dropped.
function words(a) {
  const out = [];
  let cur = null;
  let tag = false;
  a.characters.forEach((ch, i) => {
    if (ch === "[") tag = true;
    if (tag) {
      if (ch === "]") tag = false;
      return;
    }
    if (/\s/.test(ch)) {
      if (cur) out.push(cur);
      cur = null;
      return;
    }
    if (!cur) cur = { w: "", t0: a.character_start_times_seconds[i], t1: 0 };
    cur.w += ch;
    cur.t1 = a.character_end_times_seconds[i];
  });
  if (cur) out.push(cur);
  return out;
}
