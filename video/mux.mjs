// Mix narration, music and sound effects onto the rendered video.
//
//   node mux.mjs    -> out/omacus-launch.mp4 (1920x1080) and out/omacus-launch-vertical.mp4 (1080x1920)
//
// Narration clips (build/vo/<scene>.mp3) start `lead` seconds into their scene; the music ducks
// under them. Without narration it makes a preview with music and effects only.
import { readFile, mkdir, access } from "node:fs/promises";
import { spawnSync } from "node:child_process";

const BUILD = new URL("build/", import.meta.url).pathname;
const OUT = new URL("out/", import.meta.url).pathname;
await mkdir(OUT, { recursive: true });

const scenes = JSON.parse(await readFile(`${BUILD}scenes.json`, "utf8"));
const clips = [];
for (const s of scenes) {
  try {
    await access(`${BUILD}vo/${s.id}.mp3`);
    clips.push({ path: `${BUILD}vo/${s.id}.mp3`, at: s.start + s.lead });
  } catch {}
}
const voiced = clips.length === scenes.length;
if (!voiced) console.log(clips.length ? "some narration clips are missing; mixing without narration" : "no narration yet; mixing a preview");

const inputs = ["-i", `${BUILD}video.mp4`, "-i", `${BUILD}music.wav`, "-i", `${BUILD}sfx.wav`];
let graph = "[1:a]volume=1.0[music];[2:a]volume=1.0[sfx];";
if (voiced) {
  clips.forEach((c) => inputs.push("-i", c.path));
  const delayed = clips.map((c, i) => {
    const ms = Math.round(c.at * 1000);
    graph += `[${i + 3}:a]aresample=48000,adelay=${ms}|${ms},volume=1.0[v${i}];`;
    return `[v${i}]`;
  });
  graph += `${delayed.join("")}amix=inputs=${clips.length}:normalize=0[vo];`;
  // Pad the narration to the full length, or the ducked music would stop when the last line does.
  const total = Math.max(...scenes.map((s) => s.start + s.dur));
  graph += `[vo]apad=whole_dur=${total.toFixed(3)},asplit=2[vo1][vo2];`;
  // Duck the music under the voice.
  graph += "[music][vo1]sidechaincompress=threshold=0.05:ratio=3:attack=30:release=300[ducked];";
  graph += "[ducked][sfx][vo2]amix=inputs=3:normalize=0:weights=1.0 0.85 1.2[mix];";
} else {
  graph += "[music][sfx]amix=inputs=2:normalize=0[mix];";
}
graph += "[mix]loudnorm=I=-14:TP=-1.5:LRA=9[aout]";

const run = (args) => {
  const r = spawnSync("ffmpeg", ["-y", "-loglevel", "error", ...args], { stdio: "inherit" });
  if (r.status !== 0) throw new Error("ffmpeg failed");
};

const name = voiced ? "omacus-launch" : "omacus-launch-preview";
run([...inputs, "-filter_complex", graph, "-map", "0:v", "-map", "[aout]", "-c:v", "copy",
  "-c:a", "aac", "-b:a", "256k", "-ar", "48000", "-movflags", "+faststart", "-shortest", `${OUT}${name}.mp4`]);
console.log(`out/${name}.mp4`);

// A 9:16 cut for Reels, Shorts and TikTok: the frame centred over a blurred fill.
run(["-i", `${OUT}${name}.mp4`, "-filter_complex",
  "[0:v]split=2[a][b];[a]scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,boxblur=40:2,eq=brightness=-0.25[bg];" +
  "[b]scale=1080:-2[fg];[bg][fg]overlay=(W-w)/2:(H-h)/2[v]",
  "-map", "[v]", "-map", "0:a", "-c:v", "libx264", "-preset", "slow", "-crf", "17", "-pix_fmt", "yuv420p",
  "-c:a", "copy", "-movflags", "+faststart", `${OUT}${name}-vertical.mp4`]);
console.log(`out/${name}-vertical.mp4`);
