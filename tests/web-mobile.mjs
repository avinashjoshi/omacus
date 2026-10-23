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
const sw = readFileSync(join(site, "app/sw.js"), "utf8");

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
check(html.includes("data-settings"), "settings dialog is in the page");
check(html.includes('data-action="settings"'), "the menu opens settings");
check(js.includes("hideAnswers"), "the answers column can be hidden");
check(js.includes('"#f6f1e7"') && js.includes('"#1c2433"'), "paper and dusk match the desktop themes");
check(js.includes('"#9ece6a"') && js.includes('"#7dcfff"') && js.includes('"#1a7f37"'), "definition, use and keyword colours are in the themes");
check(js.includes("explicitCompletion") && js.includes('"Ctrl-Space"'), "Enter accepts only an explicitly opened suggestion");
check(js.includes("completionStatus(state)") && js.includes("if (update.docChanged) onInput()"), "a text change recalculates even if the completion check throws");
check(!js.includes("completionStatus(view)") && !js.includes("completionStatus(update.view)"), "completion status is read from the state, not the view");
check(js.includes('autocomplete: "off"') && js.includes("commitTypedText"), "phone composition is committed and autocomplete stays off");
check(js.includes("hoverValue") && js.includes("t-def"), "hover values and definition colours are wired up");
check(css.includes(".cm-editor .t-def") && css.includes(".hover-value"), "definition and hover styles exist");
check(js.includes('e.key === ","') && js.includes('e.key === "\\\\"'), "settings and the answers column have shortcuts");
check(js.includes("previewHtml"), "preview renders from the engine");
check(js.includes('e.key.toLowerCase() === "p"'), "Ctrl+P opens the preview");
check(html.includes("data-page-preview"), "the preview overlay is in the page");
check(sw.includes('const CACHE = "omacus-app-3"'), "the app cache name moved past the cache-first shell");
check(!sw.includes("omacus-app-2"), "the old app cache name is gone");
check(sw.includes("if (res.ok)") && sw.includes("cache.put"), "only a successful response is stored for offline");
check(sw.includes("e.waitUntil(fetched"), "the shell is refreshed in the background");
check(sw.includes('"/app/pkg/omacus_web_bg.wasm"'), "the engine is precached for offline");
check(!sw.includes("self.registration.unregister"), "the worker stays registered so an offline check can use it");
check(js.includes('has("sw")'), "a local preview can register the worker with ?sw=1");
check(html.includes("rates may be stale") && html.includes("data-rates-note"), "a failed rates refresh says rates may be stale");
check(js.includes('note.dataset.ratesState = state'), "the rates note records whether the refresh succeeded");
check(css.includes(".rates-note"), "the rates note is styled");
check(css.includes(".pill"), "answers in the preview are marked");
check(/\.hover-value \{[^}]*white-space:\s*nowrap/.test(css), "a hover value stays on one line");
check(/\.cm-tooltip\.cm-tooltip-hover \{[^}]*max-width:\s*none/.test(css), "the hover tooltip sizes to its text");
check(html.includes("rail-settings"), "the collapsed rail has a settings gear");
check(html.includes('class="side-tools"'), "the open sidebar has a settings gear");
check(html.includes('data-settings-tab="about"'), "settings and about share one window");
check(html.includes('data-action="about"'), "the menu opens About");
check(css.includes("grid-template-rows: minmax(0, 1fr)"), "the workspace row cannot grow past the window");
check(css.includes("overflow-y: auto !important"), "the editor scroller can scroll");
check(css.includes(".line.blank") || css.includes("p.line.blank"), "blank lines keep their height");
check(css.includes(".note"), "comments are styled in the preview");
check(js.includes("showSettingsTab"), "settings and about are tabs");
check(js.includes("afterprint"), "print restores the window title");
check(css.includes("#1a3a5c"), "printed answers use a dark ink");
check(/@page \{[^}]*margin:/.test(css), "printed pages have margins");
check(js.includes("pill.dataset.value"), "clicking an answer copies the value, not the brackets");
check(css.includes("body.answers-inline .res"), "hiding the column places answers inline");
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
let serverStopped = false;
function stopServer() {
  if (serverStopped) return;
  serverStopped = true;
  server.close();
  server.closeAllConnections?.();
}

const profile = join("/tmp", `omacus-chrome-${process.pid}`);
const browser = spawn(chrome, [
  "--headless=new",
  "--disable-gpu",
  "--no-sandbox",
  "--disable-dev-shm-usage",
  `--remote-debugging-port=0`,
  `--user-data-dir=${profile}`,
  "about:blank",
], { stdio: ["ignore", "ignore", "ignore"] });

// Chrome writes the port into the profile once DevTools is up. Reading stdout
// for the "DevTools listening" line deadlocks when that pipe fills.
const portFile = join(profile, "DevToolsActivePort");
const started = Date.now();
let version = null;
while (!version && Date.now() - started < 20000) {
  if (existsSync(portFile)) {
    const port = readFileSync(portFile, "utf8").split("\n")[0].trim();
    try {
      version = await fetch(`http://127.0.0.1:${port}/json/version`).then((r) => r.json());
    } catch {
      version = null;
    }
  }
  if (!version) await new Promise((r) => setTimeout(r, 50));
}
if (!version?.webSocketDebuggerUrl) {
  browser.kill();
  server.close();
  console.error("Chrome did not expose DevTools");
  process.exit(1);
}

const ws = new WebSocket(version.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = () => reject(new Error("DevTools websocket failed"));
});
let nextId = 0;
let failRates = false;
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
  } else if (msg.method === "Fetch.requestPaused") {
    const requestId = msg.params.requestId;
    const url = String(msg.params.request?.url || "");
    const fail = failRates && url.includes("er-api.com");
    const done = fail
      ? send("Fetch.failRequest", { requestId, errorReason: "Failed" }, msg.sessionId)
      : send("Fetch.continueRequest", { requestId }, msg.sessionId);
    done.catch(() => {});
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
  // app.js imports the wasm package, so the module (and Settings) only runs when it exists.
  await waitFor(`document.documentElement.style.getPropertyValue("--app-bg").trim() !== ""`);
  await evalJson(`document.querySelector("[data-loading]")?.remove()`);
  await waitFor(`document.querySelector(".res")`, 20000);
  await evalJson(`document.querySelector("[data-menu-button]").click()`);
  const menuOpen = await evalJson(`!document.querySelector("[data-menu]").hidden`);
  check(menuOpen === true, "the menu did not open");
  await evalJson(`document.querySelector('[data-action="settings"]').click()`);
  const settingsOpen = await evalJson(`!document.querySelector("[data-settings]").hidden`);
  check(settingsOpen === true, "settings did not open from the menu");
  const tabs = await evalJson(`(() => {
    const selected = document.querySelector('[data-settings-tab="settings"]')?.getAttribute("aria-selected");
    const aboutHidden = document.querySelector('[data-settings-panel="about"]')?.hidden;
    const settingsHidden = document.querySelector('[data-settings-panel="settings"]')?.hidden;
    return { selected, aboutHidden, settingsHidden };
  })()`);
  check(tabs.selected === "true" && tabs.aboutHidden === true && tabs.settingsHidden === false, `settings did not open on the Settings tab (${JSON.stringify(tabs)})`);
  async function pickTheme(name) {
    return evalJson(`(() => {
      const select = document.querySelector("[data-setting=theme]");
      select.value = ${JSON.stringify(name)};
      select.dispatchEvent(new Event("change"));
      return getComputedStyle(document.documentElement).getPropertyValue("--app-bg").trim().toLowerCase();
    })()`);
  }
  check(await pickTheme("paper") === "#f6f1e7", "paper theme did not apply");
  if (shots) await plainShot(join(shots, "settings-paper.png"));
  check(await pickTheme("dusk") === "#1c2433", "dusk theme did not apply");
  if (shots) await plainShot(join(shots, "settings-dusk.png"));
  check(await pickTheme("light") === "#ffffff", "light theme did not apply");
  if (shots) await plainShot(join(shots, "settings-light.png"));
  check(await pickTheme("dark") === "#1a1b26", "dark theme did not apply");
  if (shots) await plainShot(join(shots, "settings-dark.png"));
  const answers = await evalJson(`(() => {
    const hide = document.querySelector("[data-setting=hide-answers]");
    hide.click();
    const inline = document.body.classList.contains("answers-inline");
    hide.click();
    const column = !document.body.classList.contains("answers-inline");
    const note = document.querySelector("[data-theme-note]").textContent;
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    const closed = document.querySelector("[data-settings]").hidden;
    window.dispatchEvent(new KeyboardEvent("keydown", { key: ",", ctrlKey: true }));
    const reopened = !document.querySelector("[data-settings]").hidden;
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "\\\\", ctrlKey: true }));
    const toggled = document.body.classList.contains("answers-inline");
    document.querySelector("[data-settings]").hidden = true;
    localStorage.removeItem("omacus:settings");
    return { inline, column, note, closed, reopened, toggled };
  })()`);
  check(answers.inline === true, "hiding the answers column did not mark the page");
  check(answers.column === true, "showing the column again left answers inline");
  check(answers.note.includes("dark"), `theme note was ${answers.note}`);
  check(answers.closed === true, "Escape did not close settings");
  check(answers.reopened === true, "Ctrl+, did not open settings");
  check(answers.toggled === true, "Ctrl+\\ did not hide the answers column");
  const inlineFloat = await evalJson(`getComputedStyle(document.querySelector(".res")).float`);
  check(inlineFloat === "none", `an inline answer still floats ${inlineFloat}`);
  if (shots && answers.toggled) await plainShot(join(shots, "answers-inline.png"));

  // Put the answers column back, then open the rendered preview.
  await evalJson(`document.body.classList.remove("answers-inline")`);
  const preview = await evalJson(`(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "p", ctrlKey: true }));
    const root = document.querySelector("[data-page-preview]");
    const pill = root?.querySelector(".pill");
    const md = root?.querySelector(".md");
    return {
      open: !!root && !root.hidden,
      pill: pill?.textContent ?? "",
      value: pill?.dataset.value ?? "",
      color: pill ? getComputedStyle(pill).color : "",
      body: md ? getComputedStyle(md).color : "",
    };
  })()`);
  check(preview.open === true, "Ctrl+P did not open the preview");
  check(/^\[ .+ \]$/.test(preview.pill), `an answer is not in brackets (${preview.pill})`);
  check(preview.value.length > 0 && !preview.value.startsWith("["), `the copied value kept the brackets (${preview.value})`);
  check(preview.color && preview.color !== preview.body, `an answer is not its own colour (${preview.color} vs ${preview.body})`);
  if (shots) await plainShot(join(shots, "preview.png"));
  await page("Emulation.setEmulatedMedia", { media: "print" });
  const printed = await evalJson(`(() => {
    const pill = document.querySelector("[data-page-preview] .pill");
    const bar = getComputedStyle(document.querySelector(".page-preview-bar")).display;
    return { color: pill ? getComputedStyle(pill).color : "", bar };
  })()`);
  check(printed.bar === "none", `print still shows the preview bar (${printed.bar})`);
  check(printed.color && printed.color !== "rgb(0, 0, 0)", `printed answer lost its ink (${printed.color})`);
  if (shots) await plainShot(join(shots, "print-preview.png"));
  await page("Emulation.setEmulatedMedia", { media: "screen" });
  const closedPreview = await evalJson(`(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    return document.querySelector("[data-page-preview]").hidden;
  })()`);
  check(closedPreview === true, "Escape did not close the preview");
  const heading = await evalJson(`(() => {
    const head = document.querySelector(".t-h1");
    const line = document.querySelector(".cm-line");
    return {
      has: !!head,
      headSize: head ? parseFloat(getComputedStyle(head).fontSize) : 0,
      lineSize: line ? parseFloat(getComputedStyle(line).fontSize) : 0,
      weight: head ? getComputedStyle(head).fontWeight : "",
    };
  })()`);
  check(heading.has === true, "a heading line is not marked in the editor");
  check(heading.headSize > heading.lineSize + 2, `heading is ${heading.headSize}px, line is ${heading.lineSize}px`);
  check(Number(heading.weight) >= 600, `heading weight is ${heading.weight}`);

  // A long page has to scroll inside the editor, and the preview keeps each
  // source line, blank line, comment, and a shared right edge for answers.
  const mixed = ["# Northwind trip", "", "a = 5", "b = 6 # second", "", "", "// only a note", "- [x] packed", "", "| item | amount |", "| --- | --- |", "| flights | $4,720 |"];
  for (let i = 0; i < 200; i++) mixed.push(`row ${i} = ${i + 1}`);
  await evalJson(`(() => {
    localStorage.setItem("omacus:page:scroll", ${JSON.stringify(mixed.join("\n"))});
    localStorage.setItem("omacus:index", JSON.stringify({ order: ["scroll"], active: "scroll" }));
    localStorage.removeItem("omacus:settings");
    return true;
  })()`);
  await navigate(`http://127.0.0.1:${port}/app/`);
  await waitFor(`document.querySelector(".cm-scroller") && document.querySelector(".cm-content")?.innerText?.includes("Northwind") && document.querySelector(".cm-scroller").scrollHeight > 2500`, 20000);
  await waitFor(`[...document.querySelectorAll(".cm-line")].some((n) => n.textContent.includes("a = 5") && n.querySelector(".res"))`, 20000);
  await evalJson(`document.querySelector(".cm-content")?.focus()`);
  const beforeEnd = await evalJson(`document.querySelector(".cm-scroller").scrollTop`);
  await page("Input.dispatchKeyEvent", { type: "keyDown", key: "End", code: "End", windowsVirtualKeyCode: 35, modifiers: 2 });
  await page("Input.dispatchKeyEvent", { type: "keyUp", key: "End", code: "End", windowsVirtualKeyCode: 35, modifiers: 2 });
  await new Promise((r) => setTimeout(r, 80));
  const scrolled = await evalJson(`(() => {
    const scroller = document.querySelector(".cm-scroller");
    const sheet = document.querySelector(".sheet");
    const at = scroller.scrollTop;
    scroller.scrollTop = 0;
    const box = scroller.getBoundingClientRect();
    return {
      taller: scroller.scrollHeight > scroller.clientHeight + 80,
      caret: at > ${beforeEnd} + 40,
      sheetFits: sheet.scrollHeight <= sheet.clientHeight + 2,
      wheelX: box.x + Math.min(80, box.width / 2),
      wheelY: box.y + Math.min(80, box.height / 2),
    };
  })()`);
  check(scrolled.taller, "a long page does not overflow the editor scroller");
  check(scrolled.caret, "Ctrl+End did not follow the caret");
  check(scrolled.sheetFits, "the sheet grew with the page instead of scrolling inside it");
  const moved = await evalJson(`(async () => {
    const scroller = document.querySelector(".cm-scroller");
    const frame = () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const place = () => {
      const line = [...document.querySelectorAll(".cm-line")].find((n) => n.textContent.includes("a = 5"));
      const answer = line?.querySelector(".res");
      return answer ? answer.getBoundingClientRect().top : null;
    };
    scroller.scrollTop = 0;
    await frame();
    const before = place();
    scroller.scrollTop = 120;
    await frame();
    const after = place();
    scroller.scrollTop = 0;
    return { before, after, delta: before != null && after != null ? before - after : null };
  })()`);
  check(moved.delta > 40, `the answers column did not move with the editor (${JSON.stringify(moved)})`);
  const beforeWheel = await evalJson(`document.querySelector(".cm-scroller").scrollTop`);
  await page("Input.dispatchMouseEvent", {
    type: "mouseWheel",
    x: scrolled.wheelX,
    y: scrolled.wheelY,
    deltaX: 0,
    deltaY: 400,
  });
  await new Promise((r) => setTimeout(r, 100));
  const afterWheel = await evalJson(`document.querySelector(".cm-scroller").scrollTop`);
  check(afterWheel > beforeWheel + 20, `the mouse wheel did not scroll the editor (${beforeWheel} -> ${afterWheel})`);
  if (shots) {
    await evalJson(`(() => { const s = document.querySelector(".cm-scroller"); s.scrollTop = 400; return s.scrollTop; })()`);
    await plainShot(join(shots, "editor-scrolled.png"));
  }
  const previewLines = await evalJson(`(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "p", ctrlKey: true }));
    const root = document.querySelector("[data-page-preview-body]");
    root.scrollTop = 0;
    const html = root.innerHTML;
    const pills = [...root.querySelectorAll("p.line .pill")].slice(0, 2).map((p) => Math.round(p.getBoundingClientRect().right));
    const note = root.querySelector(".note");
    const before = root.scrollTop;
    root.scrollTop = root.scrollHeight;
    const canPreview = root.scrollHeight > root.clientHeight + 40 && root.scrollTop > before + 40;
    root.scrollTop = 0;
    document.body.classList.add("answers-inline");
    const inline = getComputedStyle(root.querySelector("p.line")).display;
    document.body.classList.remove("answers-inline");
    return {
      blanks: (html.match(/class="line blank"/g) || []).length,
      note: note?.textContent || "",
      noteBeforeAnswer: html.indexOf("second") < html.indexOf("[ 6 ]") && html.indexOf("second") > html.indexOf("b = 6"),
      heading: !!root.querySelector("h1"),
      rights: pills,
      canPreview,
      inline,
    };
  })()`);
  check(previewLines.blanks >= 2, `blank lines collapsed (${previewLines.blanks})`);
  check(previewLines.note.includes("second") || previewLines.noteBeforeAnswer, `comment was ${previewLines.note}`);
  check(previewLines.noteBeforeAnswer, "the comment is not between the expression and its answer");
  check(previewLines.heading, "the heading was not rendered");
  check(previewLines.rights.length === 2 && Math.abs(previewLines.rights[0] - previewLines.rights[1]) <= 1, `answers are not in one column (${previewLines.rights})`);
  check(previewLines.canPreview, "the preview does not scroll");
  check(previewLines.inline === "block", `inline answers stayed in a column (${previewLines.inline})`);
  if (shots) {
    await evalJson(`document.querySelector("[data-page-preview-body]").scrollTop = 0`);
    await plainShot(join(shots, "preview-column.png"));
    await evalJson(`document.body.classList.add("answers-inline")`);
    await plainShot(join(shots, "preview-inline.png"));
    await evalJson(`document.body.classList.remove("answers-inline")`);
  }
  await evalJson(`(() => {
    document.querySelector("[data-page-preview]").hidden = true;
    document.querySelector("[data-editor]").hidden = false;
    return true;
  })()`);
  await page("Emulation.setDeviceMetricsOverride", { width: 390, height: 700, deviceScaleFactor: 1, mobile: true });
  await new Promise((r) => setTimeout(r, 80));
  const narrow = await evalJson(`(() => {
    const scroller = document.querySelector(".cm-scroller");
    return { taller: scroller.scrollHeight > scroller.clientHeight + 80, w: innerWidth, h: scroller.scrollHeight, c: scroller.clientHeight };
  })()`);
  check(narrow.taller && narrow.w <= 400, `narrow editor does not scroll (${JSON.stringify(narrow)})`);
  if (shots) await plainShot(join(shots, "editor-narrow.png"));
  await page("Emulation.setDeviceMetricsOverride", { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
  await evalJson(`(() => {
    document.querySelector("[data-page-preview]").hidden = true;
    document.querySelector("[data-editor]").hidden = false;
    return true;
  })()`);

  // A page with a definition, a use, a keyword, a scaled number and a # comment.
  await evalJson(`(() => {
    localStorage.setItem("omacus:page:hover", "rent = $1,450 # monthly\\nrent * 2\\n1.5k\\ntotal\\nnum_companies = 10\\nrevenue = $4m * num_companies");
    localStorage.setItem("omacus:index", JSON.stringify({ order: ["hover"], active: "hover" }));
    return true;
  })()`);
  await navigate(`http://127.0.0.1:${port}/app/`);
  await waitFor(`document.querySelector(".t-def") && document.querySelector(".t-var") && document.querySelector(".t-kw") && document.querySelector(".t-com")`, 20000);
  const tones = await evalJson(`(() => {
    const color = (sel) => getComputedStyle(document.querySelector(sel)).color;
    return { def: color(".t-def"), use: color(".t-var"), kw: color(".t-kw"), com: color(".t-com"), num: color(".t-num") };
  })()`);
  check(tones.def !== tones.use && tones.use !== tones.kw && tones.def !== tones.kw, `colours did not split (${JSON.stringify(tones)})`);
  check(tones.com !== tones.def && tones.com !== tones.use, `a # comment is not its own colour (${tones.com})`);
  const spot = await evalJson(`(() => {
    const el = document.querySelector(".t-var");
    const r = el.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + Math.min(r.height / 2, 8), word: el.textContent };
  })()`);
  check(spot.word === "rent", `hovered word was ${spot.word}`);
  await page("Input.dispatchMouseEvent", { type: "mouseMoved", x: spot.x, y: spot.y });
  await waitFor(`document.querySelector(".hover-value")`, 4000);
  const tip = await evalJson(`(() => {
    const el = document.querySelector(".hover-value");
    const r = el.getBoundingClientRect();
    const host = el.closest(".cm-tooltip");
    const h = host.getBoundingClientRect();
    return {
      text: el.textContent,
      x: h.x, y: h.y, w: h.width, h: h.height,
      shown: getComputedStyle(host).visibility !== "hidden" && h.width > 0 && h.height > 0,
      word: r.width,
    };
  })()`);
  check(tip.text === "$1,450", `hover showed ${tip.text}`);
  check(tip.shown && tip.y > 0 && tip.y < 760 && tip.x > 0, `hover tooltip is off-screen (${JSON.stringify(tip)})`);
  check(tip.w > tip.h, `hover tooltip is taller than it is wide (${tip.w}x${tip.h})`);
  const nowrap = await evalJson(`getComputedStyle(document.querySelector(".hover-value")).whiteSpace`);
  check(nowrap === "nowrap", `hover value wraps (${nowrap})`);
  if (shots) await plainShot(join(shots, "hover.png"));
  const big = await evalJson(`(() => {
    const el = [...document.querySelectorAll(".t-def")].find((n) => n.textContent === "revenue");
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + Math.min(r.height / 2, 8) };
  })()`);
  check(big, "revenue is not marked as a definition");
  if (big) {
  await page("Input.dispatchMouseEvent", { type: "mouseMoved", x: 8, y: 8 });
  await page("Input.dispatchMouseEvent", { type: "mouseMoved", x: big.x, y: big.y });
  await waitFor(`document.querySelector(".hover-value")?.textContent === "$40,000,000"`, 4000);
  const longTip = await evalJson(`(() => {
    const el = document.querySelector(".hover-value");
    const host = el.closest(".cm-tooltip");
    const h = host.getBoundingClientRect();
    const inside = h.x >= 0 && h.y >= 0 && h.right <= innerWidth + 1 && h.bottom <= innerHeight + 1;
    return { text: el.textContent, w: h.width, h: h.height, x: h.x, y: h.y, inside, lines: el.innerText.split("\\n").length };
  })()`);
  check(longTip.text === "$40,000,000", `long hover showed ${longTip.text}`);
  check(longTip.w > longTip.h * 2, `long hover wrapped into a tall box (${longTip.w}x${longTip.h})`);
  check(longTip.lines === 1, `long hover split onto ${longTip.lines} lines`);
  check(longTip.inside, `long hover left the window (${JSON.stringify(longTip)})`);
  if (shots) await plainShot(join(shots, "hover-long.png"));
  }
  const gears = await evalJson(`(() => {
    document.querySelector("[data-settings]").hidden = true;
    const side = document.querySelector(".side-tools [data-action=settings]");
    side.click();
    const opened = !document.querySelector("[data-settings]").hidden;
    document.querySelector("[data-settings]").hidden = true;
    const workspace = document.querySelector(".workspace");
    workspace.classList.add("collapsed");
    const rail = document.querySelector(".rail-settings");
    const box = rail.getBoundingClientRect();
    return { opened, rail: getComputedStyle(rail).display, x: box.x, y: box.y, w: box.width, h: box.height };
  })()`);
  check(gears.opened === true, "the sidebar gear did not open settings");
  check(gears.rail !== "none" && gears.w > 0 && gears.h > 0, `the collapsed rail has no gear (${JSON.stringify(gears)})`);
  if (shots) await plainShot(join(shots, "settings-rail.png"));
  const railOpened = await evalJson(`(() => {
    document.querySelector(".rail-settings").click();
    const opened = !document.querySelector("[data-settings]").hidden;
    document.querySelector("[data-settings]").hidden = true;
    document.querySelector(".workspace").classList.remove("collapsed");
    return opened;
  })()`);
  check(railOpened === true, "the collapsed-rail gear did not open settings");
  const aboutTab = await evalJson(`(() => {
    document.querySelector('[data-action="about"]').click();
    const about = document.querySelector('[data-settings-tab="about"]')?.getAttribute("aria-selected");
    const panel = document.querySelector('[data-settings-panel="about"]');
    const settingsPanel = document.querySelector('[data-settings-panel="settings"]')?.hidden;
    return { about, open: panel && !panel.hidden, text: panel?.innerText || "", settingsPanel };
  })()`);
  check(aboutTab.about === "true" && aboutTab.open === true && aboutTab.settingsPanel === true, `About tab did not open (${JSON.stringify(aboutTab)})`);
  check(aboutTab.text.includes("omacus.com"), "About does not link to the site");
  if (shots) await plainShot(join(shots, "about-tab.png"));
  await evalJson(`document.querySelector('[data-settings-tab="settings"]').click()`);
  if (shots) await plainShot(join(shots, "settings-tab.png"));
  await evalJson(`document.querySelector("[data-settings]").hidden = true`);
  if (shots) await plainShot(join(shots, "settings-sidebar.png"));

  // A refresh that fails leaves the cached rates in use and says so. A cache younger
  // than 12 hours does not. `at: 0` is older than any 12-hour window.
  failRates = true;
  await page("Fetch.enable", {
    patterns: [{ urlPattern: "https://open.er-api.com/*", requestStage: "Request" }],
  });
  await evalJson(`localStorage.setItem("omacus:rates", JSON.stringify({ at: 0, rates: { EUR: 0.5, USD: 1 } }))`);
  await navigate(`http://127.0.0.1:${port}/app/`);
  await waitFor(`document.querySelector("[data-rates-note]")?.dataset.ratesState === "stale"`, 20000);
  const staleRates = await evalJson(`(() => {
    const note = document.querySelector("[data-rates-note]");
    return { hidden: note.hidden, text: note.textContent, state: note.dataset.ratesState };
  })()`);
  check(staleRates.state === "stale" && staleRates.hidden === false && staleRates.text.includes("rates may be stale"), `stale rates note was ${JSON.stringify(staleRates)}`);
  if (shots) await plainShot(join(shots, "rates-stale.png"));
  await evalJson(`localStorage.setItem("omacus:rates", JSON.stringify({ at: Date.now(), rates: { EUR: 0.5, USD: 1 } }))`);
  await navigate(`http://127.0.0.1:${port}/app/`);
  await waitFor(`document.querySelector("[data-rates-note]")?.dataset.ratesState === "fresh"`, 20000);
  const freshRates = await evalJson(`(() => {
    const note = document.querySelector("[data-rates-note]");
    return { hidden: note.hidden, state: note.dataset.ratesState };
  })()`);
  check(freshRates.state === "fresh" && freshRates.hidden === true, `fresh rates still showed the note (${JSON.stringify(freshRates)})`);
  failRates = false;
  await page("Fetch.disable");

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

  // Offline: register the worker on 127.0.0.1 (?sw=1), wait until the shell is
  // cached, then stop the server. The editor has to come back from that cache,
  // and the old rates have to say they may be stale.
  await page("Emulation.clearDeviceMetricsOverride");
  await page("Emulation.setEmulatedMedia", { media: "", features: [] });
  await page("Emulation.setDeviceMetricsOverride", {
    width: 1280,
    height: 800,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await evalJson(`localStorage.setItem("omacus:rates", JSON.stringify({ at: 0, rates: { EUR: 0.85, USD: 1 } }))`);
  await navigate(`http://127.0.0.1:${port}/app/?sw=1`);
  await waitFor(`document.querySelector(".cm-content")`, 20000);
  await waitForAppCache();
  await evalJson(`localStorage.setItem("omacus:rates", JSON.stringify({ at: 0, rates: { EUR: 0.85, USD: 1 } }))`);
  failRates = true;
  await page("Fetch.enable", {
    patterns: [{ urlPattern: "https://open.er-api.com/*", requestStage: "Request" }],
  });
  stopServer();
  await navigate(`http://127.0.0.1:${port}/app/?sw=1`);
  await waitFor(`document.querySelector(".cm-content") && document.querySelector("[data-rates-note]")?.dataset.ratesState === "stale"`, 20000);
  const offlineApp = await evalJson(`(() => {
    const note = document.querySelector("[data-rates-note]");
    return {
      controller: !!navigator.serviceWorker.controller,
      editor: !!document.querySelector(".cm-content"),
      hidden: note.hidden,
      text: note.textContent,
      state: note.dataset.ratesState,
    };
  })()`);
  check(offlineApp.controller === true, "the reloaded page was not served by the worker");
  check(offlineApp.editor === true, "the editor did not open offline");
  check(offlineApp.state === "stale" && offlineApp.hidden === false && offlineApp.text.includes("rates may be stale"), `offline rates note was ${JSON.stringify(offlineApp)}`);
  if (shots) await plainShot(join(shots, "offline.png"));
} else {
  console.log("editor checks skipped (wasm pkg not built)");
}

await page("Emulation.clearDeviceMetricsOverride").catch(() => {});
ws.close();
browser.kill();
stopServer();

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

async function waitForAppCache() {
  const start = Date.now();
  let last = "";
  while (Date.now() - start < 40000) {
    try {
      last = await evalJson(`(async () => {
        const keys = await caches.keys();
        if (!keys.includes("omacus-app-3")) return "no-cache";
        const urls = (await (await caches.open("omacus-app-3")).keys()).map((k) => new URL(k.url).pathname);
        const need = ["/app/", "/app/app.js", "/app/pkg/omacus_web_bg.wasm"];
        const missing = need.filter((path) => !urls.includes(path));
        return missing.length ? "missing " + missing.join(",") : "ready";
      })()`);
    } catch (e) {
      last = String(e);
    }
    if (last === "ready") return;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error("app cache not ready: " + last);
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

async function plainShot(path) {
  const { data } = await page("Page.captureScreenshot", { format: "png" });
  writeFileSync(path, Buffer.from(data, "base64"));
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
