// Narration with ElevenLabs: one clip per scene, with word timings for captions.
//
//   node voice.mjs                list voices, then generate any clips that changed
//   VOICE="Name" node voice.mjs   use a specific voice from your library
//   VOICE_ID=... MODEL=... node voice.mjs   any voice id, and another model (e.g. eleven_v3)
//
// The API key is read from ~/.config/elevenlabs/api_key and only sent to ElevenLabs.
// Clips are cached by text and voice, so re-running costs nothing unless a line changed.
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
const scenes = [...src.matchAll(/\{ id: "([\w-]+)",[^}]*?vo: "([^"]+)" \}/g)].map((m) => ({ id: m[1], vo: m[2] }));

// Names the voice gets wrong, spelled the way they're said. Captions keep the real spelling.
const SAY = { Omarchy: "Oh-mah-chee", Omacus: "Oh-mah-kus" };
const spoken = (text) => Object.entries(SAY).reduce((t, [w, say]) => t.replaceAll(w, say), text);
const unspoken = (word) => Object.entries(SAY).reduce((w, [real, say]) => w.replaceAll(say, real), word);

const voices = (await (await fetch(`${API}/voices`, { headers })).json()).voices ?? [];
if (!voices.length) throw new Error("couldn't list voices; check the API key");
// The narrator: Jessica Anne Bogart from the shared Voice Library, on eleven_v3.
// VOICE="Name" picks one from your own voices instead; VOICE_ID any voice by id.
const NARRATOR = { name: "Jessica Anne Bogart", voice_id: "lxYfHSkYm1EzQzGhdbfc" };
const named = process.env.VOICE && voices.find((v) => v.name.split(" ")[0].toLowerCase() === process.env.VOICE.toLowerCase());
if (process.env.VOICE && !named) throw new Error(`no voice named ${process.env.VOICE}`);
const voice = named ?? (process.env.VOICE_ID ? { name: process.env.VOICE_ID, voice_id: process.env.VOICE_ID } : NARRATOR);
const MODEL = process.env.MODEL ?? "eleven_v3";
console.log(`voices: ${voices.map((v) => v.name.split(" ")[0]).join(", ")}`);
console.log(`using: ${voice.name} (${voice.voice_id})`);

// eleven_v3 takes only stability (0 creative, 0.5 natural, 1 robust).
const settings = MODEL === "eleven_v3"
  ? { stability: 0.5 }
  : { stability: 0.42, similarity_boost: 0.8, style: 0.28, use_speaker_boost: true, speed: 1.0 };
const timings = {};

for (const [i, s] of scenes.entries()) {
  const hash = createHash("sha1").update(JSON.stringify([spoken(s.vo), voice.voice_id, MODEL, settings])).digest("hex").slice(0, 10);
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
        text: spoken(s.vo),
        model_id: MODEL,
        voice_settings: settings,
        // Neighbouring lines keep the delivery continuous across clips (not on eleven_v3).
        ...(MODEL !== "eleven_v3" && {
          previous_text: scenes[i - 1] && spoken(scenes[i - 1].vo),
          next_text: scenes[i + 1] && spoken(scenes[i + 1].vo),
        }),
      }),
    });
    if (!res.ok) throw new Error(`${s.id}: ${res.status} ${await res.text()}`);
    const out = await res.json();
    await writeFile(mp3, Buffer.from(out.audio_base64, "base64"));
    cached = { hash, words: words(out.alignment).map((w) => ({ ...w, w: unspoken(w.w) })) };
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

// Character timings to word timings.
function words(a) {
  const out = [];
  let cur = null;
  a.characters.forEach((ch, i) => {
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
