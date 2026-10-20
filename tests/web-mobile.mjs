// Mobile web app: no zoom-on-focus, the sheet drawer reaches the screen edge
// above the home indicator, and a tap outside it closes it without editing.
// Layout checks run in Chrome when it is installed. Anything that needs the
// running editor (the embed class, a tap on the sheet) waits for the built
// wasm package, which CI does not produce.

import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { readFileSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const site = join(root, "site");
const css = readFileSync(join(site, "app/app.css"), "utf8");
const siteCss = readFileSync(join(site, "css/site.css"), "utf8");
const js = readFileSync(join(site, "app/app.js"), "utf8");
const html = readFileSync(join(site, "app/index.html"), "utf8");

const failures = [];
function check(cond, message) {
  if (!cond) failures.push(message);
}

const viewport = html.match(/<meta name="viewport" content="([^"]+)"/)?.[1] ?? "";
check(/width=device-width/.test(viewport), "viewport sets width=device-width");
check(/viewport-fit=cover/.test(viewport), "viewport-fit=cover keeps the installed app edge to edge");
check(!/user-scalable\s*=\s*no/i.test(viewport), "pinch-zoom stays available (no user-scalable=no)");
check(!/maximum-scale\s*=\s*1\b/.test(viewport), "pinch-zoom stays available (no maximum-scale=1)");
check(html.includes('data-scrim'), "a scrim catches taps outside the drawer");

check(css.includes("--font-size: 15px;"), "desktop editor font stays 15px");
check(!css.includes("--font-size: 14px"), "the phone editor is not under 16px");
check(/@media \(max-width: 760px\)[\s\S]*?--font-size: 16px;/.test(css), "phones use a 16px editor");
check(
  /@media \(pointer: coarse\), \(max-width: 760px\)[\s\S]*?\.cm-content[\s\S]*?font-size: max\(16px, 1em\)/.test(css),
  "the focused contenteditable stays at least 16px",
);
check(css.includes("--safe-bottom: env(safe-area-inset-bottom, 0px);"), "bottom safe area is read from the device");
check(/\.side \{[^}]*padding-bottom: var\(--safe-bottom\)/.test(css), "the sidebar clears the home indicator");
check(/position: fixed;\s*inset: 0;/.test(css), "the shell fills the visual viewport, not 100dvh");
check(!/\.app-page \{[^}]*height:\s*100dvh/.test(css), "the shell is not sized with 100dvh");
check(/\.app-page\.drawer \.drawer-scrim \{\s*display: block/.test(css), "the scrim shows only while the drawer is open");

check(js.includes("function setDrawer(open)"), "opening and closing the drawer share one path");
check(/closest\?\.\("\[data-scrim\]"\)\) setDrawer\(false\)/.test(js), "a scrim click closes the drawer");
check(/pointerdown[\s\S]*setDrawer\(false\)[\s\S]*preventDefault\(\)/.test(js), "the outside tap is cancelled before it edits");
check(/closest\("\.side, \.sheets-toggle, \.bar"\)/.test(js), "taps on the drawer itself do not dismiss it");
check(js.includes('setAttribute("aria-expanded", String(open))'), "the sheets button reflects the drawer");
check(js.includes("Recently deleted"), "deleted sheets can be restored from Recently deleted");
check(js.includes("r.hint"), "name hints are marked so they can wait out typing");
check(
  /@media \(max-width: 620px\)[\s\S]*?\.features \{[\s\S]*?overflow-x:\s*auto/.test(siteCss),
  "example cards scroll sideways on a narrow screen",
);

if (failures.length) {
  console.error(failures.map((f) => "  - " + f).join("\n"));
  process.exit(1);
}
console.log("static checks ok");

const chrome = ["/usr/bin/google-chrome", "/usr/bin/google-chrome-stable", "/usr/bin/chromium"].find((p) => existsSync(p));
if (!chrome || process.env.OMACUS_SKIP_BROWSER === "1") {
  console.log("browser checks skipped (no Chrome)");
  process.exit(0);
}

const types = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css",
  ".js": "text/javascript",
  ".wasm": "application/wasm",
  ".woff2": "font/woff2",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".webp": "image/webp",
  ".webmanifest": "application/manifest+json",
  ".json": "application/json",
};

const server = createServer((req, res) => {
  const url = new URL(req.url, "http://127.0.0.1");
  let path = decodeURIComponent(url.pathname);
  if (path.endsWith("/")) path += "index.html";
  const file = normalize(join(site, path));
  if (!file.startsWith(site) || !existsSync(file)) {
    res.writeHead(404);
    res.end("not found");
    return;
  }
  res.writeHead(200, { "content-type": types[extname(file)] || "application/octet-stream" });
  res.end(readFileSync(file));
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const port = server.address().port;

const profile = join("/tmp", `omacus-chrome-${process.pid}`);
const browser = spawn(chrome, [
  "--headless=new",
  "--disable-gpu",
  "--no-sandbox",
  "--disable-dev-shm-usage",
  `--remote-debugging-port=0`,
  `--user-data-dir=${profile}`,
  "about:blank",
], { stdio: ["ignore", "pipe", "pipe"] });

let devtoolsUrl = "";
browser.stderr.setEncoding("utf8");
browser.stderr.on("data", (chunk) => {
  const found = chunk.match(/DevTools listening on (ws:\/\/\S+)/);
  if (found) devtoolsUrl = found[1];
});
const started = Date.now();
while (!devtoolsUrl && Date.now() - started < 20000) await new Promise((r) => setTimeout(r, 50));
if (!devtoolsUrl) {
  browser.kill();
  server.close();
  console.error("Chrome did not expose DevTools");
  process.exit(1);
}

const version = await fetch(devtoolsUrl.replace("ws://", "http://").replace(/\/devtools\/.*/, "/json/version")).then((r) => r.json());
const ws = new WebSocket(version.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = () => reject(new Error("DevTools websocket failed"));
});
let nextId = 0;
const pending = new Map();
const sessions = new Map();
ws.onmessage = (ev) => {
  const msg = JSON.parse(ev.data);
  if (msg.id && pending.has(msg.id)) {
    const { resolve, reject } = pending.get(msg.id);
    pending.delete(msg.id);
    if (msg.error) reject(new Error(`${msg.error.message || JSON.stringify(msg.error)}`));
    else resolve(msg.result);
  } else if (msg.method === "Target.attachedToTarget") {
    sessions.set(msg.params.sessionId, msg.params);
  }
};
function send(method, params = {}, sessionId) {
  const id = ++nextId;
  const payload = { id, method, params };
  if (sessionId) payload.sessionId = sessionId;
  ws.send(JSON.stringify(payload));
  return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
}

await send("Target.setDiscoverTargets", { discover: true });
const { targetId } = await send("Target.createTarget", { url: "about:blank" });
const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
const page = (method, params) => send(method, params, sessionId);

await page("Page.enable");
await page("Runtime.enable");
await openMobile();
await waitFor(`getComputedStyle(document.body).getPropertyValue("--bar").trim() === "56px" && matchMedia("(max-width: 760px)").matches`);

const mobile = await evalJson(`(() => {
  document.body.style.setProperty("--safe-top", "47px");
  document.body.style.setProperty("--safe-bottom", "34px");
  document.body.classList.add("drawer");
  const side = document.querySelector(".side");
  const foot = document.querySelector(".side-foot");
  const scrim = document.querySelector("[data-scrim]");
  const editor = document.querySelector("[data-editor]");
  const sr = side.getBoundingClientRect();
  const fr = foot.getBoundingClientRect();
  return {
    font: parseFloat(getComputedStyle(editor).fontSize),
    sideBottom: sr.bottom,
    sideTop: sr.top,
    innerHeight: window.innerHeight,
    paddingBottom: parseFloat(getComputedStyle(side).paddingBottom),
    footBottom: fr.bottom,
    scrim: getComputedStyle(scrim).display,
    position: getComputedStyle(side).position,
    viewport: document.querySelector('meta[name=viewport]').content,
  };
})()`);

check(mobile.font >= 16, `phone editor font is ${mobile.font}px, want >= 16`);
check(Math.abs(mobile.sideBottom - mobile.innerHeight) <= 1, `drawer bottom ${mobile.sideBottom} != screen ${mobile.innerHeight}`);
check(mobile.paddingBottom === 34, `drawer padding-bottom is ${mobile.paddingBottom}, want 34`);
check(mobile.footBottom <= mobile.innerHeight - 34 + 1, `footer is cut by the home indicator (${mobile.footBottom} > ${mobile.innerHeight - 34})`);
check(mobile.scrim === "block", `scrim display is ${mobile.scrim} while the drawer is open`);
check(mobile.position === "fixed", `phone drawer position is ${mobile.position}`);
check(mobile.sideTop >= 47, "drawer starts below the status bar");

const shots = process.env.OMACUS_SHOTS;
if (shots) {
  mkdirSync(shots, { recursive: true });
  await shot(join(shots, "mobile-drawer.png"));
}

await evalJson(`(() => {
  document.body.style.removeProperty("--safe-top");
  document.body.style.removeProperty("--safe-bottom");
  document.body.classList.remove("drawer");
})()`);

// Desktop layout: a wide window and a mouse. Clear the phone emulation first;
// otherwise Chrome keeps the coarse pointer and the tablet font rule applies.
await page("Emulation.clearDeviceMetricsOverride");
await page("Emulation.setEmulatedMedia", { media: "", features: [] });
await page("Emulation.setDeviceMetricsOverride", {
  width: 1280,
  height: 800,
  deviceScaleFactor: 1,
  mobile: false,
});
await navigate(`http://127.0.0.1:${port}/app/`);
await waitFor(`getComputedStyle(document.body).getPropertyValue("--bar").trim() === "56px" && matchMedia("(min-width: 761px)").matches`);
const media = await evalJson(`({
  w: innerWidth,
  h: innerHeight,
  wide: matchMedia("(min-width: 761px)").matches,
  fine: matchMedia("(pointer: fine)").matches,
  coarse: matchMedia("(pointer: coarse)").matches,
})`);
check(media.wide && !media.coarse, `desktop media did not apply (${JSON.stringify(media)})`);
const desktop = await evalJson(`(() => {
  const side = document.querySelector(".side");
  const editor = document.querySelector("[data-editor]");
  const scrim = document.querySelector("[data-scrim]");
  return {
    font: parseFloat(getComputedStyle(editor).fontSize),
    position: getComputedStyle(side).position,
    scrim: getComputedStyle(scrim).display,
    columns: getComputedStyle(document.querySelector(".workspace")).gridTemplateColumns,
    paddingBottom: parseFloat(getComputedStyle(side).paddingBottom),
    rail: getComputedStyle(document.querySelector(".rail")).display,
    resize: getComputedStyle(document.querySelector("[data-resize]")).display,
  };
})()`);
check(desktop.font === 15, `desktop editor font is ${desktop.font}px, want 15`);
// Relative, so the resize handle anchors to the sidebar. Fixed is the phone drawer.
check(desktop.position === "relative", `desktop sidebar position is ${desktop.position}`);
check(desktop.scrim === "none", `desktop scrim display is ${desktop.scrim}`);
check(desktop.paddingBottom === 0, `desktop sidebar padding-bottom is ${desktop.paddingBottom}`);
check(/240px/.test(desktop.columns), `desktop columns are ${desktop.columns}`);
check(desktop.rail === "none", `desktop rail display is ${desktop.rail}`);
check(desktop.resize !== "none", `desktop resize handle display is ${desktop.resize}`);
const collapsed = await evalJson(`(() => {
  const workspace = document.querySelector(".workspace");
  workspace.classList.add("collapsed");
  const cols = getComputedStyle(workspace).gridTemplateColumns;
  const rail = getComputedStyle(document.querySelector(".rail")).display;
  const body = getComputedStyle(document.querySelector(".side-body")).display;
  workspace.classList.remove("collapsed");
  return { cols, rail, body };
})()`);
check(/^52px/.test(collapsed.cols), `collapsed columns are ${collapsed.cols}`);
check(collapsed.rail === "flex", `collapsed rail display is ${collapsed.rail}`);
check(collapsed.body === "none", `collapsed sheet list display is ${collapsed.body}`);

if (shots) await shot(join(shots, "desktop.png"));

const pkg = existsSync(join(site, "app/pkg/omacus_web.js"));
if (pkg) {
  // The homepage embeds the editor in an iframe. The shell still fills that frame.
  // The embed class is added by app.js, which only loads once the wasm package exists.
  await page("Emulation.clearDeviceMetricsOverride");
  await page("Emulation.setEmulatedMedia", { media: "", features: [] });
  await page("Emulation.setDeviceMetricsOverride", {
    width: 800,
    height: 520,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await navigate(`http://127.0.0.1:${port}/app/?embed`);
  await waitFor(`document.body.classList.contains("embed") && getComputedStyle(document.body).getPropertyValue("--bar").trim() === "0px"`);
  const embed = await evalJson(`(() => {
    const page = document.body.getBoundingClientRect();
    const editor = document.querySelector("[data-editor]").getBoundingClientRect();
    return {
      pageH: Math.round(page.height),
      pageW: Math.round(page.width),
      editorH: Math.round(editor.height),
      side: getComputedStyle(document.querySelector(".side")).display,
      scrim: getComputedStyle(document.querySelector("[data-scrim]")).display,
    };
  })()`);
  check(Math.abs(embed.pageH - 520) <= 1 && Math.abs(embed.pageW - 800) <= 1, `embed shell is ${embed.pageW}x${embed.pageH}, want 800x520`);
  check(embed.editorH > 400, `embed editor height is ${embed.editorH}`);
  check(embed.side === "none", `embed sidebar display is ${embed.side}`);
  check(embed.scrim === "none", `embed scrim display is ${embed.scrim}`);

  await openMobile();
  await waitFor(`document.querySelector(".cm-content")`, 15000);
  const contentFont = await evalJson(`parseFloat(getComputedStyle(document.querySelector(".cm-content")).fontSize)`);
  check(contentFont >= 16, `contenteditable font is ${contentFont}px, want >= 16`);

  // The editor is not focused. Opening the drawer and tapping the sheet closes
  // it, and must not focus the editor or place a cursor.
  await evalJson(`document.querySelector(".cm-content").blur()`);
  await evalJson(`document.querySelector(".sheets-toggle").click()`);
  await waitFor(`document.body.classList.contains("drawer") && document.querySelector(".side").getBoundingClientRect().left >= -1`);
  const point = await evalJson(`(() => {
    const scrim = document.querySelector("[data-scrim]");
    const r = scrim.getBoundingClientRect();
    const side = document.querySelector(".side").getBoundingClientRect();
    const x = Math.min(r.right - 8, Math.max(side.right + 8, r.left + 8));
    const y = r.top + r.height / 2;
    const hit = document.elementFromPoint(x, y);
    return { x, y, sideRight: side.right, hit: hit?.className || hit?.tagName || "" };
  })()`);
  check(point.x > point.sideRight, `tap x ${point.x} is not outside the drawer`);
  check(String(point.hit).includes("drawer-scrim"), `tap hit ${point.hit}, not the scrim`);
  await tap(point.x, point.y);
  await waitFor(`!document.body.classList.contains("drawer")`);
  const after = await evalJson(`({
    open: document.body.classList.contains("drawer"),
    focused: !!document.querySelector(".cm-editor.cm-focused"),
    active: document.activeElement?.className || "",
    expanded: document.querySelector(".sheets-toggle").getAttribute("aria-expanded"),
    scrim: getComputedStyle(document.querySelector("[data-scrim]")).display,
  })`);
  check(after.open === false, "tapping the sheet left the drawer open");
  check(after.focused === false, `tap focused the editor (${after.active})`);
  check(after.expanded === "false", "sheets button still says expanded");
  check(after.scrim === "none", `scrim stayed up (${after.scrim}) and would block typing`);

  // A tap on the drawer's own heading keeps it open. Wait out the slide-in so
  // the hit test isn't still the sheet behind it.
  await evalJson(`document.querySelector(".sheets-toggle").click()`);
  await waitFor(`document.querySelector(".side").getBoundingClientRect().left >= -1`);
  const head = await evalJson(`(() => {
    const label = document.querySelector(".side-head span");
    const r = label.getBoundingClientRect();
    const x = r.left + r.width / 2;
    const y = r.top + r.height / 2;
    const hit = document.elementFromPoint(x, y);
    return { x, y, inSide: !!hit?.closest(".side"), hit: hit?.className || hit?.tagName || "" };
  })()`);
  check(head.inSide === true, `drawer tap hit ${head.hit} at ${head.x},${head.y}`);
  await tap(head.x, head.y);
  await new Promise((r) => setTimeout(r, 50));
  const still = await evalJson(`document.body.classList.contains("drawer")`);
  check(still === true, "tapping the drawer closed it");

  if (shots) {
    await evalJson(`document.body.style.setProperty("--safe-top", "59px"); document.body.style.setProperty("--safe-bottom", "34px");`);
    await shot(join(shots, "after-drawer-open.png"));
    await page("Input.dispatchMouseEvent", { type: "mousePressed", x: point.x, y: point.y, button: "left", clickCount: 1 });
    await page("Input.dispatchMouseEvent", { type: "mouseReleased", x: point.x, y: point.y, button: "left", clickCount: 1 });
    await waitFor(`!document.body.classList.contains("drawer")`);
    await shot(join(shots, "after-drawer-closed.png"));
  }
} else {
  console.log("editor checks skipped (wasm pkg not built)");
}

await page("Emulation.clearDeviceMetricsOverride").catch(() => {});
ws.close();
browser.kill();
server.close();

if (failures.length) {
  console.error(failures.map((f) => "  - " + f).join("\n"));
  process.exit(1);
}
console.log("browser checks ok");

async function evalJson(expression) {
  const { result, exceptionDetails } = await page("Runtime.evaluate", {
    expression,
    returnByValue: true,
    awaitPromise: true,
  });
  if (exceptionDetails) throw new Error(exceptionDetails.text + " " + (exceptionDetails.exception?.description || ""));
  return result.value;
}

async function waitFor(expression, timeout = 5000) {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    try {
      if (await evalJson(`!!(${expression})`)) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 50));
  }
  throw new Error("timed out waiting for " + expression);
}

// Page.navigate returns before the new page loads, and the old one can already
// pass the next waitFor: mark the old page, then wait for a loaded one without
// the mark. (Checks once ran against a half-parsed page in CI.)
async function navigate(url) {
  await evalJson("window.__leaving = true").catch(() => {});
  await page("Page.navigate", { url });
  await waitFor(`!window.__leaving && document.readyState === "complete"`, 10000);
}

async function openMobile() {
  await page("Emulation.setDeviceMetricsOverride", {
    width: 390,
    height: 844,
    deviceScaleFactor: 1,
    mobile: true,
  });
  await page("Emulation.setEmulatedMedia", {
    features: [
      { name: "display-mode", value: "standalone" },
      { name: "pointer", value: "coarse" },
      { name: "hover", value: "none" },
    ],
  });
  await navigate(`http://127.0.0.1:${port}/app/`);
}

async function tap(x, y) {
  await page("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
  await page("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
}

async function shot(path) {
  // A home-indicator pill, only in the picture, so the inset is visible.
  await evalJson(`(() => {
    let pill = document.getElementById("shot-pill");
    if (!pill) {
      pill = document.createElement("div");
      pill.id = "shot-pill";
      pill.style.cssText = "position:fixed;left:50%;bottom:8px;transform:translateX(-50%);width:120px;height:5px;border-radius:999px;background:rgba(255,255,255,.85);z-index:80;pointer-events:none";
      document.body.append(pill);
    }
  })()`);
  const { data } = await page("Page.captureScreenshot", { format: "png" });
  writeFileSync(path, Buffer.from(data, "base64"));
  await evalJson(`document.getElementById("shot-pill")?.remove()`);
}
