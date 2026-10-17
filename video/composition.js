// The Omacus launch video, as a function of time: render(t) draws the frame at t
// seconds. render.mjs steps through it at 60fps and pipes the frames to ffmpeg.
//
// Scene lengths come from the narration (build/timings.json, written by voice.mjs)
// so cuts land on the words; without it, each scene uses its minimum length.

const FPS = 60;
const LEAD = 0.3; // narration starts this long into its scene (unless the scene sets `lead`)
const HOOK_IN = 1.0; // a quiet beat on the caret before the first line types

const SCENES = [
  { id: "hook", min: 3.6, lead: 1.3, hold: 0.9, vo: "You think in sentences. Your calculator doesn't." },
  { id: "intro", min: 8.2, vo: "Meet Omacus, a notepad calculator. Write math the way you'd write a note, and every answer lines up as you type." },
  { id: "currency", min: 4.8, vo: "Split a trip in yen, dollars and euros. Rates stay fresh, and work offline." },
  { id: "zones", min: 4.4, vo: "Time zones, with daylight saving, even on a date next month." },
  { id: "percent", min: 5.4, vo: "Percentages, the way you'd say them. And hundreds of units." },
  { id: "invoice", min: 5.8, vo: "Name a number once. Change it, and everything below updates." },
  { id: "switch", min: 4.8, vo: "Every sheet saves as you type. Jump to any of them in a keystroke." },
  { id: "share", min: 6.6, vo: "Share any sheet as a link. It opens live in any browser, even on your phone." },
  { id: "themes", min: 5.4, vo: "On Omarchy, it wears your theme, and switches when you do." },
  { id: "platforms", min: 5.0, vo: "Free and open source. Linux, macOS, Windows, and your browser." },
  { id: "end", min: 5.2, vo: "Omacus. Math, the way you write it." },
];

const THEMES = [
  ["matte-black", "Matte Black", "#121212", "#e68e0d"],
  ["tokyo-night", "Tokyo Night", "#1a1b26", "#7aa2f7"],
  ["catppuccin", "Catppuccin", "#1e1e2e", "#89b4fa"],
  ["gruvbox", "Gruvbox", "#282828", "#7daea3"],
  ["everforest", "Everforest", "#2d353b", "#7fbbb3"],
  ["nord", "Nord", "#2e3440", "#81a1c1"],
  ["rose-pine", "Rosé Pine", "#faf4ed", "#56949f"],
  ["catppuccin-latte", "Catppuccin Latte", "#eff1f5", "#1e66f5"],
];

// ------------------------------------------------------------------------------------
// Easing and helpers

const clamp = (x, a = 0, b = 1) => Math.min(b, Math.max(a, x));
const lerp = (a, b, p) => a + (b - a) * p;
const prog = (t, a, b) => clamp((t - a) / (b - a));
const easeOut = (p) => 1 - Math.pow(1 - p, 3);
const easeOutExpo = (p) => (p >= 1 ? 1 : 1 - Math.pow(2, -10 * p));
const easeInOut = (p) => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2);
const easeIn = (p) => p * p * p;
// A soft spring: overshoots a little, settles.
const spring = (p) => (p >= 1 ? 1 : 1 - Math.exp(-6.5 * p) * Math.cos(9 * p));
const $ = (s) => document.querySelector(s);
const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

let seed = 7;
const rand = () => ((seed = (seed * 16807) % 2147483647) - 1) / 2147483646;

// ------------------------------------------------------------------------------------
// Timeline: scene starts and the schedules everything else reads.

let DATA, TL;

async function load() {
  DATA = await (await fetch("/video/build/data.json")).json();
  let timings = {};
  try {
    timings = await (await fetch("/video/build/timings.json")).json();
  } catch {}
  TL = buildTimeline(timings);
  setupStatic();
  await document.fonts.ready;
  await Promise.all([...document.images].map((i) => (i.complete ? 0 : new Promise((r) => (i.onload = i.onerror = r)))));
}

function buildTimeline(timings) {
  let t = 0;
  const scenes = {};
  for (const s of SCENES) {
    const vo = timings[s.id];
    const lead = s.lead ?? LEAD;
    const dur = Math.max(s.min + (s.hold ?? 0), vo ? lead + vo.duration + 0.55 + (s.hold ?? 0) : 0);
    const words = vo?.words ?? fakeWords(s.vo, dur - lead - 0.6 - (s.hold ?? 0));
    scenes[s.id] = { ...s, start: t, dur, words: words.map((w) => ({ ...w, t0: w.t0 + t + lead, t1: w.t1 + t + lead })) };
    t += dur;
  }
  const tl = { scenes, total: t, typing: [], events: [] };
  scheduleTyping(tl);
  return tl;
}

// Even word timing, for previews before the narration exists.
function fakeWords(text, span) {
  const words = text.split(" ");
  const per = span / words.length;
  return words.map((w, i) => ({ w, t0: i * per, t1: (i + 1) * per }));
}

const S = (id) => TL.scenes[id];

// Type `sheet` line `line` between times a and b (absolute), with a human rhythm.
function type(tl, sheet, line, a, b) {
  const text = DATA[sheet].lines[line];
  const n = text.length;
  const times = [];
  let acc = 0;
  const gaps = Array.from({ length: n }, (_, i) => 0.7 + rand() * 0.6 + (text[i] === " " ? 0.5 : 0));
  const total = gaps.reduce((x, y) => x + y, 0);
  for (const g of gaps) {
    acc += g;
    times.push(a + ((b - a) * acc) / total);
  }
  tl.typing.push({ sheet, line, times, done: b });
  times.forEach((ct) => tl.events.push({ t: ct, kind: "key" }));
  if (DATA[sheet].results[line]) tl.events.push({ t: b + 0.06, kind: "pop" });
}

// Type a run of lines evenly across [a, b], with a short pause between lines.
function typeLines(tl, sheet, from, to, a, b) {
  const lines = [];
  for (let i = from; i <= to; i++) lines.push(i);
  const lens = lines.map((i) => Math.max(DATA[sheet].lines[i].length, 0));
  const pause = 0.12;
  const typingTime = b - a - pause * lines.length;
  const totalChars = lens.reduce((x, y) => x + y, 0) || 1;
  let t = a;
  lines.forEach((i, k) => {
    const d = (typingTime * lens[k]) / totalChars;
    if (lens[k] > 0) type(tl, sheet, i, t, t + d);
    else tl.typing.push({ sheet, line: i, times: [], done: t });
    t += d + pause;
  });
}

function scheduleTyping(tl) {
  const sc = tl.scenes;
  const at = (id, s) => sc[id].start + s;
  // Hook: one big line, typed by the hook layer itself (see renderHook).
  const hook = DATA.hook.lines[0];
  hookTimes = [];
  for (let i = 0; i < hook.length; i++) hookTimes.push(at("hook", HOOK_IN + 0.35 + i * 0.065 + (hook[i] === " " ? 0.03 : 0)));
  hookTimes.forEach((ct) => tl.events.push({ t: ct, kind: "key", big: true }));
  tl.events.push({ t: at("hook", HOOK_IN + 1.75), kind: "pop", big: true });

  tl.events.push({ t: at("intro", -0.12), kind: "whoosh" });
  logoBeads(0).forEach((b) => tl.events.push({ t: at("intro", b.land), kind: "bead" }));
  tl.events.push({ t: at("intro", 2.2), kind: "whoosh" });
  // Kyoto: header through `each = prev / 4` during the intro.
  typeLines(tl, "kyoto", 0, 7, at("intro", 3.25), at("intro", sc.intro.dur - 0.15));
  // The currency lines, then the times back home.
  typeLines(tl, "kyoto", 8, 10, at("currency", 0.2), at("currency", 2.2));
  typeLines(tl, "kyoto", 11, 13, at("currency", 2.35), at("currency", sc.currency.dur - 0.15));
  // Quick math: time zones, then percentages and units.
  tl.events.push({ t: at("zones", 0.0), kind: "tick" });
  typeLines(tl, "quick", 0, 2, at("zones", 0.25), at("zones", Math.min(3.6, sc.zones.dur - 0.3)));
  typeLines(tl, "quick", 3, 6, at("percent", 0.1), at("percent", 2.5));
  typeLines(tl, "quick", 7, 10, at("percent", 2.65), at("percent", sc.percent.dur - 0.2));
  // Invoice: select 150, type 175, everything ripples.
  tl.events.push({ t: at("invoice", 0.0), kind: "tick" });
  [1.45, 1.6, 1.75].forEach((s) => tl.events.push({ t: at("invoice", s), kind: "key" }));
  for (let i = 0; i < 10; i++) tl.events.push({ t: at("invoice", 2.0 + i * 0.07), kind: "roll" });
  // Switcher: Ctrl+P, "kyo", Enter.
  tl.events.push({ t: at("switch", 0.35), kind: "thock" });
  tl.events.push({ t: at("switch", 0.42), kind: "thock" });
  [1.0, 1.17, 1.32].forEach((s) => tl.events.push({ t: at("switch", s), kind: "key" }));
  tl.events.push({ t: at("switch", 2.15), kind: "thock" });
  // Share: click Share, the link is copied, a phone opens it and adds a line.
  tl.events.push({ t: at("share", SHARE.click), kind: "thock" });
  tl.events.push({ t: at("share", SHARE.click + 0.12), kind: "pop" });
  tl.events.push({ t: at("share", SHARE.phoneIn - 0.1), kind: "whoosh" });
  const extra = DATA.phone.lines.at(-1);
  phoneTimes = [];
  for (let i = 0; i < extra.length; i++) phoneTimes.push(at("share", SHARE.typeAt + i * 0.075));
  phoneTimes.forEach((ct) => tl.events.push({ t: ct, kind: "key" }));
  tl.events.push({ t: phoneTimes.at(-1) + 0.12, kind: "pop" });
  // Themes: a whoosh per theme.
  for (let i = 0; i < THEMES.length; i++) tl.events.push({ t: at("themes", themeTime(i)), kind: "swish" });
  // Install command.
  const cmd = "curl -fsSL https://omacus.com/install | bash";
  cmdTimes = [];
  for (let i = 0; i < cmd.length; i++) cmdTimes.push(at("platforms", 1.25 + i * 0.038));
  cmdTimes.forEach((ct) => tl.events.push({ t: ct, kind: "key" }));
  [0.15, 0.32, 0.49].forEach((s) => tl.events.push({ t: at("platforms", s), kind: "tick" }));
  [at("platforms", -0.15), at("end", -0.1)].forEach((w) => tl.events.push({ t: w, kind: "whoosh" }));
  logoBeads(0.25).forEach((b) => tl.events.push({ t: at("end", b.land), kind: "bead" }));
  tl.events.push({ t: at("end", 1.55), kind: "chime" });
}
let hookTimes = [];
let cmdTimes = [];
let phoneTimes = [];
// The share scene's beats, in seconds from its start.
const SHARE = { click: 1.0, phoneIn: 2.0, typeAt: 3.7 };
const themeTime = (i) => 0.55 + i * 0.58;

// ------------------------------------------------------------------------------------
// Syntax colors, as in the app (src/highlight.rs), simplified.

const KEYWORDS = new Set("in to as of on off what is prev sum total avg average mean today now tomorrow yesterday plus minus times mod".split(" "));

function highlight(line, vars) {
  if (/^\s*#/.test(line)) return `<span class="t-head">${esc(line)}</span>`;
  let code = line;
  let comment = "";
  const c = line.indexOf("//");
  if (c >= 0) {
    code = line.slice(0, c);
    comment = line.slice(c);
  }
  let out = "";
  const label = code.match(/^(\s*[A-Za-z][A-Za-z '\-_&/]*:)(?= )/);
  if (label) {
    out += `<span class="t-label">${esc(label[1])}</span>`;
    code = code.slice(label[1].length);
  }
  const re = /("[^"]*"?)|(\d[\w.,]*)|([A-Za-z_]\w*)|([$£€¥₹%])|(\s+)|(.)/gu;
  let m;
  while ((m = re.exec(code))) {
    const [tok, str, num, word, sym, ws] = m;
    if (str) out += `<span class="t-com">${esc(tok)}</span>`;
    else if (num || sym) out += `<span class="t-num">${esc(tok)}</span>`;
    else if (word) {
      const cls = vars.has(word) ? "t-var" : KEYWORDS.has(word.toLowerCase()) ? "t-kw" : "";
      out += cls ? `<span class="${cls}">${esc(tok)}</span>` : esc(tok);
    } else if (ws) out += tok;
    else out += `<span class="t-op">${esc(tok)}</span>`;
  }
  if (comment) out += `<span class="t-com">${esc(comment)}</span>`;
  return out;
}

const varsOf = (lines) => new Set(lines.map((l) => l.match(/^\s*([A-Za-z_]\w*)\s*=[^=]/)?.[1]).filter(Boolean));

// ------------------------------------------------------------------------------------
// The logo: an abacus that assembles itself. Same geometry as site/img/icon.svg.

const BEADS = [
  // [row y, final x, colour]
  [160, 140, "#e68e0d"], [160, 190, "#e68e0d"], [160, 328, "#e68e0d"],
  [232, 140, "#e68e0d"], [232, 278, "#e68e0d"], [232, 328, "#e68e0d"],
  [304, 140, "#f5f5f5"], [304, 190, "#f5f5f5"], [304, 240, "#f5f5f5"], [304, 328, "#f5f5f5"],
];

// When each bead starts and lands, relative to the logo's start (plus `delay`).
function logoBeads(delay) {
  const rows = [160, 232, 304];
  return BEADS.map(([y, x, color]) => {
    const row = rows.indexOf(y);
    const inRow = BEADS.filter((b) => b[0] === y);
    const k = inRow.findIndex((b) => b[1] === x);
    const n = inRow.length;
    // Beads wait stacked at the right end of their rod, then flick into place.
    const from = 350 - (n - 1 - k) * 46;
    const start = delay + 0.42 + row * 0.13 + k * 0.07;
    return { y, x, color, from, start, land: start + 0.24 };
  });
}

function logoSvg() {
  const beads = BEADS.map(([y, , c], i) => `<rect class="bead" data-i="${i}" y="${y}" width="44" height="48" rx="22" fill="${c}"/>`).join("");
  return `<svg viewBox="0 0 512 512">
    <g class="tile"><rect x="32" y="32" width="448" height="448" rx="104" fill="#161616"/>
    <rect x="33.5" y="33.5" width="445" height="445" rx="102.5" fill="none" stroke="#2c2c2c" stroke-width="3"/></g>
    <rect class="frame" x="104" y="112" width="304" height="288" rx="20" fill="none" stroke="#3a3a3a" stroke-width="14" pathLength="1" stroke-dasharray="1"/>
    <g stroke="#4a4a4a" stroke-width="8" stroke-linecap="round">
      <line class="rod" x1="118" y1="184" x2="394" y2="184" pathLength="1" stroke-dasharray="1"/>
      <line class="rod" x1="118" y1="256" x2="394" y2="256" pathLength="1" stroke-dasharray="1"/>
      <line class="rod" x1="118" y1="328" x2="394" y2="328" pathLength="1" stroke-dasharray="1"/>
    </g>${beads}</svg>`;
}

// Draw the logo `s` seconds into its animation.
function animateLogo(root, s, delay = 0) {
  const svg = root.querySelector("svg");
  const tile = svg.querySelector(".tile");
  const pt = prog(s, delay, delay + 0.7);
  tile.style.transformOrigin = "256px 256px";
  tile.style.transform = `scale(${lerp(0.6, 1, spring(pt))})`;
  tile.style.opacity = easeOut(prog(s, delay, delay + 0.25));
  svg.querySelector(".frame").style.strokeDashoffset = 1 - easeInOut(prog(s, delay + 0.12, delay + 0.6));
  svg.querySelectorAll(".rod").forEach((rod, i) => {
    rod.style.strokeDashoffset = 1 - easeOut(prog(s, delay + 0.25 + i * 0.08, delay + 0.6 + i * 0.08));
  });
  const beads = logoBeads(delay);
  svg.querySelectorAll(".bead").forEach((el, i) => {
    const b = beads[i];
    const p = prog(s, b.start, b.land + 0.2);
    el.setAttribute("x", lerp(b.from, b.x, spring(p)));
    el.style.opacity = easeOut(prog(s, b.start - 0.12, b.start + 0.05));
  });
}

// Letters of a word resolve one by one.
function animateWord(el, s, at) {
  [...el.children].forEach((ch, i) => {
    const p = prog(s, at + i * 0.045, at + i * 0.045 + 0.5);
    ch.style.opacity = easeOut(p);
    ch.style.transform = `translateY(${lerp(30, 0, easeOutExpo(p))}px)`;
    ch.style.filter = `blur(${lerp(10, 0, easeOut(p))}px)`;
  });
}

// ------------------------------------------------------------------------------------
// Static setup

function setupStatic() {
  document.querySelectorAll(".logo-svg").forEach((el) => (el.innerHTML = logoSvg()));
  document.querySelectorAll(".word").forEach((el) => (el.innerHTML = [...el.textContent].map((c) => `<span>${c}</span>`).join("")));
  $("#ver").textContent = `v${DATA.version}`;
  $("#sheets").innerHTML = DATA.sheets.map((s) => `<div class="sheet">${esc(s)}</div>`).join("");
  const sprite = (id) => `<svg aria-hidden="true"><use href="/site/img/sprite.svg#${id}"/></svg>`;
  $("#platforms").innerHTML = `
    <div class="icons">
      <div class="os">${sprite("linux")}<span>Linux</span></div>
      <div class="os">${sprite("apple")}<span>macOS</span></div>
      <div class="os">${sprite("windows")}<span>Windows</span></div>
      <div class="os"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3z"/></svg><span>Web</span></div>
    </div>
    <div class="cmd"><span class="p">$ </span><span class="c"></span></div>
    <div class="free">Free and open source · MIT</div>`;
  $("#themes").innerHTML =
    THEMES.map(([id]) => `<div class="card"><img src="/site/img/screens/${id}.webp" alt=""></div>`).join("") +
    `<div class="label"><i><b></b><b></b></i><span></span></div>`;
  $("#keys").innerHTML = `<div class="key k1">Ctrl</div><div class="plus">+</div><div class="key k2">P</div><div class="key k3" style="margin-left:60px">Enter ↵</div>`;
}

// ------------------------------------------------------------------------------------
// Rendering

function show(el, opacity, transform = "") {
  el.style.opacity = opacity;
  el.style.visibility = opacity <= 0.001 ? "hidden" : "visible";
  el.style.transform = transform;
}

function render(t) {
  const sc = TL.scenes;
  const local = (id) => t - sc[id].start;
  const inScene = (id) => t >= sc[id].start && t < sc[id].start + sc[id].dur;

  // Background glow drifts slowly.
  $("#glow").style.setProperty("--gx", `${50 + 12 * Math.sin(t * 0.21)}%`);
  $("#glow").style.setProperty("--gy", `${62 + 8 * Math.cos(t * 0.17)}%`);

  renderHook(t, local("hook"), sc.hook.dur);
  renderLogo(local("intro"));
  renderWindow(t);
  renderKeys(local("switch"), sc.switch.dur);
  renderPhone(t, local("share"), sc.share.dur);
  renderThemes(local("themes"), sc.themes.dur);
  renderPlatforms(t, local("platforms"), sc.platforms.dur);
  renderEnd(local("end"), sc.end.dur);
  renderWipe(t);
  renderCaptions(t);

  // Fade in from black, and out at the very end.
  const fadeIn = 1 - prog(t, 0, 0.8);
  const fadeOut = prog(t, TL.total - 0.7, TL.total);
  $("#fade").style.opacity = Math.max(fadeIn, easeIn(fadeOut));
  void inScene;
}

function renderHook(t, s, dur) {
  const el = $("#hook");
  if (s < 0 || s > dur + 0.1) return show(el, 0);
  const text = DATA.hook.lines[0];
  const n = hookTimes.filter((ct) => ct <= t).length;
  // Reserve the full width so the line stays centred while it types.
  el.querySelector(".line").style.width = `${text.length + 0.6}ch`;
  const typed = s - HOOK_IN;
  const caret = typed < 1.7 && Math.floor(s * 2.4) % 2 === 0 ? '<span class="caret"></span>' : "";
  el.querySelector(".line").innerHTML = esc(text.slice(0, n)) + (n < text.length || typed < 1.7 ? caret : "");
  const a = prog(typed, 1.75, 2.15);
  const ans = el.querySelector(".ans");
  ans.textContent = DATA.hook.results[0];
  ans.style.opacity = easeOut(a);
  ans.style.transform = `translateY(${lerp(26, 0, easeOutExpo(a))}px)`;
  ans.style.filter = `blur(${lerp(10, 0, easeOut(a))}px)`;
  ans.style.textShadow = `0 0 ${lerp(40, 0, prog(typed, 1.8, 2.8))}px rgba(240,161,42,.8)`;
  // Push in slowly, then zoom through into the next scene.
  const out = prog(s, dur - 0.45, dur);
  show(el, 1 - easeIn(out), `scale(${lerp(1, 1.04, s / dur) * lerp(1, 1.6, easeIn(out))})`);
  el.style.filter = `blur(${lerp(0, 14, easeIn(out))}px)`;
}

function renderLogo(s) {
  const el = $("#logo");
  if (s < -0.1 || s > 3.0) return show(el, 0);
  animateLogo(el.querySelector(".logo-svg"), s);
  animateWord(el.querySelector(".word"), s, 0.75);
  const tag = el.querySelector(".tag");
  const pTag = prog(s, 1.25, 1.85);
  tag.style.opacity = easeOut(pTag);
  tag.style.transform = `translateY(${lerp(24, 0, easeOutExpo(pTag))}px)`;
  tag.style.filter = `blur(${lerp(8, 0, easeOut(pTag))}px)`;
  // Rise away as the window comes in.
  const out = prog(s, 2.2, 2.9);
  show(el, 1 - easeIn(out), `translateY(${lerp(0, -240, easeInOut(out))}px) scale(${lerp(1, 0.84, easeInOut(out))})`);
}

// What the window shows: which sheet, how much of each line is typed.
function sheetView(t) {
  const sc = TL.scenes;
  const at = (id, s = 0) => sc[id].start + s;
  if (t >= at("switch", 2.3)) return { key: "kyoto", active: 0, prev: 2, since: at("switch", 2.3), full: true };
  if (t >= at("invoice")) return { key: "invoice", active: 2, prev: 1, since: at("invoice"), full: true };
  if (t >= at("zones")) return { key: "quick", active: 1, prev: 0, since: at("zones") };
  return { key: "kyoto", active: 0, prev: 0, since: -1 };
}

function typedState(sheet, line, t) {
  const item = TL.typing.find((x) => x.sheet === sheet && x.line === line);
  if (!item) return { chars: 0, done: Infinity };
  return { chars: item.times.filter((ct) => ct <= t).length, done: item.done, typing: t < item.done && t >= (item.times[0] ?? item.done) - 0.05 };
}

function renderWindow(t) {
  const sc = TL.scenes;
  const stage = $("#stage");
  const sIntro = t - sc.intro.start;
  const sThemes = t - sc.themes.start;
  if (sIntro < 2.15 || sThemes > 0.8) return show(stage, 0);

  // Fly in from below with a tilt; fly back as the themes take over.
  const pIn = prog(sIntro, 2.2, 3.3);
  const pOut = prog(sThemes, 0, 0.7);
  const rx = lerp(24, 0, easeOutExpo(pIn)) + lerp(0, -10, easeIn(pOut));
  const ty = lerp(420, 0, easeOutExpo(pIn)) + lerp(0, -60, easeIn(pOut));
  const sc0 = lerp(0.86, 1, easeOutExpo(pIn)) * lerp(1, 0.78, easeIn(pOut));
  // Step aside for the phone during the share scene.
  const sShare = t - sc.share.start;
  const aside = easeInOut(prog(sShare, SHARE.phoneIn - 0.3, SHARE.phoneIn + 0.6));
  show(
    stage,
    easeOut(prog(sIntro, 2.2, 2.65)) * (1 - easeIn(pOut)),
    `perspective(2400px) translateX(${lerp(0, -330, aside)}px) translateY(${ty}px) rotateX(${rx}deg) rotateY(${lerp(0, 8, aside)}deg) scale(${sc0 * lerp(1, 0.72, aside)})`,
  );
  stage.style.filter = `brightness(${lerp(1, 0.62, aside)})`;

  // Share, above the sheet: hovered, pressed, then "Link copied".
  const bar = $("#winbar .share");
  const hover = prog(sShare, SHARE.click - 0.45, SHARE.click - 0.3);
  const press = prog(sShare, SHARE.click, SHARE.click + 0.06) * (1 - prog(sShare, SHARE.click + 0.14, SHARE.click + 0.3));
  bar.style.background = `rgba(255,255,255,${0.07 * hover})`;
  bar.style.color = hover > 0.5 ? "var(--app-fg)" : "";
  bar.style.transform = `scale(${1 - 0.06 * press})`;
  const toast = $("#toast");
  const tIn = prog(sShare, SHARE.click + 0.1, SHARE.click + 0.4);
  const tOut = prog(sShare, SHARE.click + 2.4, SHARE.click + 2.7);
  show(toast, easeOut(tIn) * (1 - tOut), `translateY(${lerp(-10, 0, easeOutExpo(tIn))}px)`);
  stage.style.transformOrigin = "50% 60%";

  const view = sheetView(t);
  // Sidebar: the highlight glides to the new sheet; the content slides in.
  const rows = [...document.querySelectorAll("#side .sheet")];
  rows.forEach((el, i) => el.classList.toggle("on", i === view.active));
  const glide = easeOutExpo(prog(t, view.since, view.since + 0.45));
  $("#pill").style.top = `${lerp(rows[view.prev].offsetTop, rows[view.active].offsetTop, glide)}px`;
  const enter = prog(t, view.since, view.since + 0.35);
  $("#lines").style.opacity = easeOut(enter);
  $("#lines").style.transform = `translateY(${lerp(18, 0, easeOutExpo(enter))}px)`;

  // Lines.
  let lines, html;
  if (view.key === "invoice") {
    html = renderInvoice(t - sc.invoice.start);
  } else {
    const sheet = DATA[view.key];
    const vars = varsOf(sheet.lines);
    lines = sheet.lines.map((text, i) => {
      const st = view.full ? { chars: text.length, done: -Infinity } : typedState(view.key, i, t);
      const shown = text.slice(0, st.chars);
      const caret = st.typing ? '<span class="caret"></span>' : "";
      const res = sheet.results[i];
      let resHtml = "";
      if (res && t >= st.done) {
        const p = prog(t, st.done, st.done + 0.32);
        const glow = lerp(26, 0, prog(t, st.done, st.done + 0.9));
        resHtml = `<span style="opacity:${easeOut(p)};transform:translateX(${lerp(-18, 0, easeOutExpo(p))}px);text-shadow:0 0 ${glow}px rgba(240,161,42,.9)">${esc(res)}</span>`;
      }
      if (st.chars === 0 && !caret && !view.full) return `<li><span class="src"> </span><span class="res"></span></li>`;
      return `<li><span class="src">${highlight(shown, vars) || " "}${caret}</span><span class="res">${resHtml}</span></li>`;
    });
    html = lines.join("");
  }
  $("#lines").innerHTML = html;

  renderSwitcher(t - sc.switch.start);
  renderCamera(t);
}

function renderInvoice(s) {
  const a = DATA.invoice150;
  const b = DATA.invoice175;
  const vars = varsOf(a.lines);
  return a.lines
    .map((text, i) => {
      let src;
      if (i === 1) {
        // Select "150", then type "175" over it.
        const typed = s < 1.45 ? null : "175".slice(0, s < 1.6 ? 1 : s < 1.75 ? 2 : 3);
        if (typed === null) {
          const selected = s > 1.0;
          src = highlight("rate = $", vars) + (selected ? `<span class="sel">${highlight("150", vars)}</span>` : highlight("150", vars)) + highlight("/hour", vars);
        } else {
          src = highlight(`rate = $${typed}`, vars) + '<span class="caret"></span>' + highlight("/hour", vars);
        }
      } else {
        src = highlight(text, vars) || " ";
      }
      let res = "";
      if (a.results[i]) {
        const p = easeOutExpo(prog(s, 2.0 + i * 0.07, 2.0 + i * 0.07 + 0.45));
        const glow = lerp(30, 0, prog(s, 2.0 + i * 0.07, 2.0 + i * 0.07 + 1.0)) * (p > 0 ? 1 : 0);
        res = `<span style="position:relative;display:inline-block;height:46px;overflow:hidden;vertical-align:top">
          <span style="display:block;transform:translateY(${-46 * p}px);opacity:${1 - p}">${esc(a.results[i])}</span>
          <span style="display:block;position:absolute;right:0;top:0;transform:translateY(${46 * (1 - p)}px);opacity:${p};text-shadow:0 0 ${glow}px rgba(240,161,42,.95)">${esc(b.results[i])}</span>
        </span>`;
      }
      return `<li><span class="src">${src}</span><span class="res">${res}</span></li>`;
    })
    .join("");
}

function renderSwitcher(s) {
  const sw = $("#switcher");
  const scrim = $("#scrim");
  const open = prog(s, 0.5, 0.75);
  const close = prog(s, 2.25, 2.45);
  const vis = easeOut(open) * (1 - easeIn(close));
  show(sw, vis, `scale(${lerp(0.95, 1, easeOutExpo(open))}) translateY(${lerp(-12, 0, easeOutExpo(open))}px)`);
  scrim.style.opacity = vis;
  if (vis <= 0) return;
  const q = "kyo".slice(0, s < 1.0 ? 0 : s < 1.17 ? 1 : s < 1.32 ? 2 : 3);
  sw.querySelector(".q").innerHTML = q ? esc(q) + '<span class="caret" style="height:26px;width:3px;vertical-align:-4px"></span>' : '<span class="hint">Go to sheet…</span>';
  const titles = DATA.sheets;
  const rows = q ? titles.filter((x) => x.toLowerCase().includes(q) || [...q].every((c) => x.toLowerCase().includes(c))) : titles.slice(0, 5);
  sw.querySelector(".rows").innerHTML = rows
    .slice(0, 5)
    .map((x, i) => `<div class="row${i === 0 ? " on" : ""}"><span>${esc(x)}</span>${x === DATA.sheets[2] ? "<small>current</small>" : ""}</div>`)
    .join("");
}

// Camera: where it looks (in page pixels) and how close, per moment.
function cameraAt(t) {
  const sc = TL.scenes;
  const L = (i) => ({ x: 980, y: 96 + 34 + 46 * i + 23 }); // a line's centre
  const full = { x: 960, y: 526, z: 1 };
  const keys = [];
  const k = (id, s, pos, z) => keys.push({ t: sc[id].start + s, x: pos.x, y: pos.y, z });
  k("intro", 0, full, 1);
  k("intro", sc.intro.dur - 0.6, full, 1);
  k("currency", 0.5, { x: 1060, y: L(9.5).y }, 1.42);
  k("currency", 2.2, { x: 1060, y: L(9.5).y }, 1.42);
  k("currency", sc.currency.dur - 0.2, { x: 1040, y: L(11).y }, 1.18);
  k("zones", 0.45, { x: 1060, y: L(1.5).y }, 1.38);
  k("zones", sc.zones.dur - 0.2, { x: 1060, y: L(1.8).y }, 1.38);
  k("percent", 0.5, { x: 1040, y: L(5).y }, 1.3);
  k("percent", 2.4, { x: 1040, y: L(5.5).y }, 1.3);
  k("percent", sc.percent.dur - 0.3, { x: 1040, y: L(9).y }, 1.3);
  // Close enough to read, wide enough that the rate and every answer it changes are in shot.
  k("invoice", 0.7, { x: 1180, y: L(4).y }, 1.32);
  k("invoice", 2.0, { x: 1180, y: L(4).y }, 1.34);
  k("invoice", 3.4, { x: 1120, y: L(4.5).y }, 1.18);
  k("invoice", sc.invoice.dur, { x: 1100, y: L(4.5).y }, 1.14);
  k("switch", 0.4, full, 1);
  k("share", 0.15, full, 1);
  k("share", 0.8, { x: 1420, y: 230 }, 1.55);
  k("share", SHARE.click + 0.45, { x: 1420, y: 230 }, 1.55);
  k("share", SHARE.phoneIn - 0.1, full, 1);
  k("themes", 0, full, 1);
  if (t <= keys[0].t) return keys[0];
  for (let i = 1; i < keys.length; i++) {
    if (t <= keys[i].t) {
      const a = keys[i - 1];
      const b = keys[i];
      const p = easeInOut(prog(t, a.t, b.t));
      return { x: lerp(a.x, b.x, p), y: lerp(a.y, b.y, p), z: lerp(a.z, b.z, p) };
    }
  }
  return keys[keys.length - 1];
}

function renderCamera(t) {
  const c = cameraAt(t);
  const tx = 960 - c.x * c.z;
  const ty = 520 - c.y * c.z;
  $("#camera").style.transform = `translate(${tx}px, ${ty}px) scale(${c.z})`;
}

// The shared sheet on a phone, at omacus.com/app.
function renderPhone(t, s, dur) {
  const el = $("#phone");
  if (s < SHARE.phoneIn - 0.5 || s > dur + 0.1) return show(el, 0);
  const pIn = prog(s, SHARE.phoneIn - 0.2, SHARE.phoneIn + 0.9);
  const out = prog(s, dur - 0.5, dur);
  show(
    el,
    easeOut(prog(s, SHARE.phoneIn - 0.2, SHARE.phoneIn + 0.2)) * (1 - easeIn(out)),
    `translateX(${lerp(520, 0, easeOutExpo(pIn))}px) translateY(${lerp(0, -40, easeIn(out))}px) rotate(${lerp(6, -2, easeOutExpo(pIn))}deg)`,
  );
  const sheet = DATA.phone;
  const vars = varsOf(sheet.lines);
  const n = phoneTimes.filter((ct) => ct <= t).length;
  const last = sheet.lines.length - 1;
  const lineIn = (i) => prog(s, SHARE.phoneIn + 0.35 + i * 0.03, SHARE.phoneIn + 0.7 + i * 0.03);
  el.querySelector(".lines").innerHTML = sheet.lines
    .map((text, i) => {
      if (i === last) {
        if (s < SHARE.typeAt - 0.4) return "";
        const typing = n < text.length;
        const caret = typing || Math.floor(s * 2.4) % 2 === 0 ? '<span class="caret"></span>' : "";
        const done = phoneTimes.at(-1) + 0.1;
        const p = prog(t, done, done + 0.35);
        const res = t >= done ? `<span style="opacity:${easeOut(p)};text-shadow:0 0 ${lerp(22, 0, prog(t, done, done + 0.9))}px rgba(240,161,42,.9)">${esc(sheet.results[i])}</span>` : "";
        return `<li><span class="src">${highlight(text.slice(0, n), vars)}${caret}</span><span class="res">${res}</span></li>`;
      }
      const o = easeOut(lineIn(i));
      return `<li style="opacity:${o}"><span class="src">${highlight(text, vars) || " "}</span><span class="res">${esc(sheet.results[i] || "")}</span></li>`;
    })
    .join("");
}

function renderKeys(s, dur) {
  const el = $("#keys");
  if (s < 0.2 || s > 2.8) return show(el, 0);
  show(el, easeOut(prog(s, 0.2, 0.35)) * (1 - easeIn(prog(s, 2.45, 2.8))));
  const press = (key, at) => {
    const p = prog(s, at, at + 0.06) * (1 - prog(s, at + 0.16, at + 0.28));
    key.style.transform = `translateY(${4 * p}px)`;
    key.style.boxShadow = `0 ${6 - 4 * p}px 0 #0c0c0c, 0 20px 40px -10px rgba(0,0,0,.8)${p > 0.2 ? ", 0 0 0 2px rgba(240,161,42,.6)" : ""}`;
  };
  const k1 = el.querySelector(".k1");
  const k2 = el.querySelector(".k2");
  const k3 = el.querySelector(".k3");
  press(k1, 0.35);
  press(k2, 0.42);
  press(k3, 2.15);
  const enter = prog(s, 1.9, 2.05);
  k3.style.opacity = easeOut(enter);
  el.querySelector(".plus").style.opacity = 1 - easeOut(enter) * 0.6;
  k1.style.opacity = k2.style.opacity = 1 - easeOut(enter) * 0.6;
  void dur;
}

function renderThemes(s, dur) {
  const el = $("#themes");
  if (s < 0 || s > dur + 0.1) return show(el, 0);
  const out = prog(s, dur - 0.45, dur);
  show(el, easeOut(prog(s, 0.1, 0.5)) * (1 - easeIn(out)), `scale(${lerp(1, 1.06, easeIn(out))})`);
  // A carousel that steps to the next theme on each beat.
  let pos = 0;
  for (let i = 1; i < THEMES.length; i++) pos += easeInOut(prog(s, themeTime(i) - 0.2, themeTime(i) + 0.12));
  const cards = el.querySelectorAll(".card");
  cards.forEach((card, i) => {
    const o = i - pos;
    const ao = Math.abs(o);
    card.style.transform = `translateX(${o * 980}px) translateZ(${-ao * 420}px) rotateY(${-o * 28}deg) scale(${lerp(1, 0.82, clamp(ao))})`;
    card.style.opacity = clamp(1.6 - ao);
    card.style.zIndex = String(100 - Math.round(ao * 10));
    card.style.filter = `brightness(${lerp(1, 0.45, clamp(ao))})`;
  });
  const cur = Math.round(pos);
  const [, name, bg, fg] = THEMES[cur];
  const label = el.querySelector(".label");
  label.querySelector("span").textContent = name;
  const sw = label.querySelectorAll("b");
  sw[0].style.background = bg;
  sw[1].style.background = fg;
  const frac = Math.abs(pos - cur);
  label.style.opacity = 1 - clamp(frac * 3);
}

function renderPlatforms(t, s, dur) {
  const el = $("#platforms");
  if (s < 0 || s > dur + 0.1) return show(el, 0);
  const out = prog(s, dur - 0.4, dur);
  show(el, easeOut(prog(s, 0, 0.3)) * (1 - easeIn(out)), `scale(${lerp(1, 0.96, easeIn(out))})`);
  el.querySelectorAll(".os").forEach((os, i) => {
    const p = prog(s, 0.12 + i * 0.17, 0.12 + i * 0.17 + 0.8);
    os.style.opacity = easeOut(prog(s, 0.12 + i * 0.17, 0.12 + i * 0.17 + 0.25));
    os.style.transform = `translateY(${lerp(60, 0, spring(p))}px)`;
  });
  const cmd = "curl -fsSL https://omacus.com/install | bash";
  const n = cmdTimes.filter((ct) => ct <= t).length;
  const box = el.querySelector(".cmd");
  box.style.opacity = easeOut(prog(s, 0.9, 1.2));
  box.style.transform = `translateY(${lerp(20, 0, easeOutExpo(prog(s, 0.9, 1.4)))}px)`;
  const blink = Math.floor(s * 2.4) % 2 === 0 || n < cmd.length;
  box.querySelector(".c").innerHTML = esc(cmd.slice(0, n)) + (blink ? '<span class="caret"></span>' : "");
  el.querySelector(".free").style.opacity = easeOut(prog(s, 2.8, 3.3));
}

function renderEnd(s, dur) {
  const el = $("#end");
  if (s < 0) return show(el, 0);
  show(el, 1);
  animateLogo(el.querySelector(".logo-svg"), s, 0.25);
  animateWord(el.querySelector(".word"), s, 0.85);
  [el.querySelector(".tag"), el.querySelector(".url"), el.querySelector(".sub")].forEach((p, i) => {
    const at = 1.35 + i * 0.3;
    const q = prog(s, at, at + 0.9);
    p.style.opacity = easeOut(prog(s, at, at + 0.35));
    p.style.transform = `translateY(${lerp(26, 0, easeOutExpo(q))}px)`;
    p.style.filter = `blur(${lerp(12, 0, easeOut(prog(s, at, at + 0.4)))}px)`;
  });
  // The URL gets a slow shine.
  const url = el.querySelector(".url");
  const shine = prog(s, 2.6, 3.6);
  url.style.backgroundImage = `linear-gradient(100deg, #ffc061 ${lerp(-30, 110, shine) - 15}%, #fff3dc ${lerp(-30, 110, shine)}%, #ffc061 ${lerp(-30, 110, shine) + 15}%)`;
  url.style.webkitBackgroundClip = "text";
  url.style.color = "transparent";
  void dur;
}

// A streak of light across the frame, covering a cut between sections.
function renderWipe(t) {
  const sc = TL.scenes;
  const cuts = [sc.intro.start - 0.05, sc.platforms.start - 0.1, sc.end.start - 0.05];
  const bar = $("#wipe .bar");
  const cut = cuts.find((c) => t >= c - 0.35 && t <= c + 0.35);
  if (cut === undefined) return (bar.style.opacity = 0);
  const p = prog(t, cut - 0.35, cut + 0.35);
  bar.style.opacity = Math.sin(Math.PI * p);
  bar.style.left = `${lerp(-500, 2200, easeInOut(p))}px`;
}

function renderCaptions(t) {
  const box = $("#captions .box");
  const scene = SCENES.map((s) => TL.scenes[s.id]).find((s) => t >= s.words[0].t0 - 0.15 && t <= s.words.at(-1).t1 + 0.35);
  if (!scene) {
    box.parentElement.style.opacity = 0;
    return;
  }
  const a = prog(t, scene.words[0].t0 - 0.15, scene.words[0].t0 + 0.05);
  const b = 1 - prog(t, scene.words.at(-1).t1 + 0.15, scene.words.at(-1).t1 + 0.35);
  box.parentElement.style.opacity = Math.min(a, b);
  box.innerHTML = scene.words.map((w) => `<span class="w${t >= w.t0 ? " said" : ""}">${esc(w.w)}</span>`).join(" ");
}

// ------------------------------------------------------------------------------------
// API for render.mjs

window.video = {
  ready: load(),
  fps: FPS,
  duration: () => TL.total,
  render,
  events: () => TL.events.sort((a, b) => a.t - b.t),
  scenes: () => SCENES.map((s) => ({ id: s.id, start: TL.scenes[s.id].start, dur: TL.scenes[s.id].dur, lead: s.lead ?? LEAD, vo: s.vo })),
};
