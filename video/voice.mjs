// Narration with ElevenLabs: one clip per scene, with word timings for captions.
//
//   node voice.mjs                list voices, then generate any clips that changed
//   VOICE="Name" node voice.mjs   use a specific voice from your library
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
const scenes = [...src.matchAll(/\{ id: "([\w-]+)", min: [\d.]+, vo: "([^"]+)" \}/g)].map((m) => ({ id: m[1], vo: m[2] }));

const voices = (await (await fetch(`${API}/voices`, { headers })).json()).voices ?? [];
if (!voices.length) throw new Error("couldn't list voices; check the API key");
const PREFERRED = [process.env.VOICE, "Brian", "Eric", "George", "Daniel", "Chris", "Adam"].filter(Boolean);
const voice = PREFERRED.map((n) => voices.find((v) => v.name.split(" ")[0].toLowerCase() === n.toLowerCase())).find(Boolean) ?? voices[0];
console.log(`voices: ${voices.map((v) => v.name.split(" ")[0]).join(", ")}`);
console.log(`using: ${voice.name} (${voice.voice_id})`);

const settings = { stability: 0.42, similarity_boost: 0.8, style: 0.28, use_speaker_boost: true, speed: 1.0 };
const timings = {};

for (const [i, s] of scenes.entries()) {
  const hash = createHash("sha1").update(JSON.stringify([s.vo, voice.voice_id, settings])).digest("hex").slice(0, 10);
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
        text: s.vo,
        model_id: "eleven_multilingual_v2",
        voice_settings: settings,
        // Neighbouring lines keep the delivery continuous across clips.
        previous_text: scenes[i - 1]?.vo,
        next_text: scenes[i + 1]?.vo,
      }),
    });
    if (!res.ok) throw new Error(`${s.id}: ${res.status} ${await res.text()}`);
    const out = await res.json();
    await writeFile(mp3, Buffer.from(out.audio_base64, "base64"));
    cached = { hash, words: words(out.alignment) };
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
