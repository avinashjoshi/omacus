// Render the composition to video frames.
//
//   node render.mjs --stills 1.2,5,12   PNG stills at those times (build/still-<t>.png)
//   node render.mjs                     the full video, silent (build/video.mp4), plus
//                                       build/events.json and build/scenes.json for the audio
//
// Serves the repo root over HTTP so the page can load the site's fonts and images.
import { createServer } from "node:http";
import { readFile, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { extname, join, resolve, sep } from "node:path";
import puppeteer from "puppeteer-core";

const ROOT = new URL("..", import.meta.url).pathname;
const BUILD = new URL("build/", import.meta.url).pathname;
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".json": "application/json", ".css": "text/css",
  ".svg": "image/svg+xml", ".png": "image/png", ".webp": "image/webp", ".woff2": "font/woff2" };

// Only this machine can connect, and only files inside the repo are served.
const server = createServer(async (req, res) => {
  try {
    const file = resolve(join(ROOT, decodeURIComponent(new URL(req.url, "http://x").pathname)));
    if (!file.startsWith(resolve(ROOT) + sep)) throw new Error("outside the repo");
    const body = await readFile(file);
    res.writeHead(200, { "content-type": TYPES[extname(file)] ?? "application/octet-stream" });
    res.end(body);
  } catch {
    res.writeHead(404).end();
  }
});
await new Promise((ok) => server.listen(0, "127.0.0.1", ok));
const port = server.address().port;

// Chrome or Chromium: set CHROME to override.
const CANDIDATES = [process.env.CHROME, "/usr/bin/chromium", "/usr/bin/chromium-browser", "/usr/bin/google-chrome",
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe"];
const { existsSync } = await import("node:fs");
const executablePath = CANDIDATES.find((p) => p && existsSync(p));
if (!executablePath) throw new Error("No Chrome or Chromium found; set CHROME=/path/to/chrome");

const browser = await puppeteer.launch({
  executablePath,
  headless: true,
  args: ["--hide-scrollbars", "--force-color-profile=srgb", "--disable-lcd-text", "--font-render-hinting=none"],
});
const page = await browser.newPage();
await page.setViewport({ width: 1920, height: 1080, deviceScaleFactor: 1 });
page.on("pageerror", (e) => console.error("page error:", e.message));
await page.goto(`http://localhost:${port}/video/composition.html`, { waitUntil: "networkidle0" });
await page.evaluate(() => window.video.ready);

const duration = await page.evaluate(() => window.video.duration());
const frame = async (t) => {
  await page.evaluate((t) => window.video.render(t), t);
  return page.screenshot({ type: "png", optimizeForSpeed: true });
};

const stills = process.argv.indexOf("--stills");
if (stills > 0) {
  for (const t of process.argv[stills + 1].split(",").map(Number)) {
    await writeFile(join(BUILD, `still-${t}.png`), await frame(t));
    console.log(`still-${t}.png`);
  }
} else {
  await writeFile(join(BUILD, "events.json"), JSON.stringify(await page.evaluate(() => window.video.events())));
  await writeFile(join(BUILD, "scenes.json"), JSON.stringify(await page.evaluate(() => window.video.scenes()), null, 1));
  const fps = await page.evaluate(() => window.video.fps);
  const frames = Math.ceil(duration * fps);
  const ffmpeg = spawn("ffmpeg", ["-y", "-loglevel", "error", "-f", "image2pipe", "-framerate", String(fps), "-i", "-",
    "-c:v", "libx264", "-preset", "slow", "-crf", "14", "-pix_fmt", "yuv420p", "-tune", "animation",
    join(BUILD, "video.mp4")], { stdio: ["pipe", "inherit", "inherit"] });
  const started = Date.now();
  for (let i = 0; i < frames; i++) {
    const png = await frame(i / fps);
    if (!ffmpeg.stdin.write(png)) await new Promise((r) => ffmpeg.stdin.once("drain", r));
    if (i % 120 === 0) {
      const rate = (i + 1) / ((Date.now() - started) / 1000);
      process.stdout.write(`frame ${i}/${frames}  ${rate.toFixed(1)} fps  ~${Math.round((frames - i) / rate)}s left\n`);
    }
  }
  ffmpeg.stdin.end();
  await new Promise((r) => ffmpeg.on("close", r));
  console.log(`build/video.mp4  ${duration.toFixed(2)}s, ${frames} frames`);
}

console.log(`duration ${duration.toFixed(2)}s`);
await browser.close();
server.close();
