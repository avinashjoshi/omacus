// The Omacus launch video, as a function of time: render(t) draws the frame at t
// seconds. render.mjs steps through it at 60fps and pipes the frames to ffmpeg.
//
// Scene lengths come from the narration (build/timings.json, written by voice.mjs)
// so cuts land on the words; without it, each scene uses its minimum length.

const FPS = 60;
const LEAD = 0.3; // narration starts this long into its scene (unless the scene sets `lead`)
const HOOK_IN = 0.3; // a short beat on the caret before the first line types
// The music's tempo (audio.py uses the same). Every scene is a whole number of beats,
// so cuts land on the beat, and answers pop on half-beats.
const BPM = 116;
const BEAT = 60 / BPM;
const onBeat = (t, step = BEAT) => Math.ceil(t / step - 1e-6) * step;
// A calm lead-in before the hook: fade up from black on a blinking caret, as the music swells.
const PRE = 4 * BEAT;

// `say` is what's spoken when it differs from the caption (audio tags), `stab` its stability.
const SCENES = [
  { id: "hook", min: 3.6, lead: 0.5, tail: 0.3, vo: "You think in sentences. Your calculator doesn't." },
  { id: "intro", min: 5.6, vo: "Meet Omacus. Write math like a note, and every answer appears as you type." },
  { id: "currency", min: 4.8, vo: "Split a trip in yen, dollars and euros. Live rates, even offline." },
  { id: "montage", min: 6.4, lead: 0.3, tail: 0.2, vo: "Time zones. Tips. Splits. Runway. Countdowns. Hex. Say it the way you'd say it." },
  { id: "invoice", min: 5.4, vo: "Name a number once. Change it, and everything below updates." },
  { id: "library", min: 6.2, vo: "A fresh library opens with examples. A trip, the groceries, and a monthly total that reads another page." },
  { id: "keys", min: 5.6, vo: "On Omarchy, one hotkey opens it. Click any answer to copy. Every page saves itself." },
  { id: "cli", min: 5.0, vo: "Live in the terminal? Pipe a whole page straight in." },
  { id: "share", min: 6.0, vo: "Share any page as a link. It opens live, even on your phone." },
  { id: "sync", min: 7.2, vo: "Sync keeps named vaults on every device you link. A recovery phrase restores them. The pages stay encrypted, even from us." },
  { id: "themes", min: 5.0, vo: "It wears your Omarchy theme, and switches when you do." },
  { id: "platforms", min: 5.2, vo: "Free, open source, no account. Linux, macOS, Windows, and the web, even offline." },
  { id: "end", min: 5.6, hold: 2.8, stab: 0.3, say: "[confident] Omacus. [warm] Math... the way YOU write it.", vo: "Omacus. Math, the way you write it." },
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
  let t = PRE;
  const scenes = {};
  for (const s of SCENES) {
    const vo = timings[s.id];
    const lead = s.lead ?? LEAD;
    const dur = onBeat(Math.max(s.min + (s.hold ?? 0), vo ? lead + vo.duration + (s.tail ?? 0.55) + (s.hold ?? 0) : 0));
    const words = vo?.words ?? fakeWords(s.vo, dur - lead - 0.6 - (s.hold ?? 0));
    scenes[s.id] = { ...s, start: t, dur, words: words.map((w) => ({ ...w, t0: w.t0 + t + lead, t1: w.t1 + t + lead })) };
    t += dur;
  }
  const tl = { scenes, total: onBeat(t), typing: [], events: [] };
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
  const done = onBeat(b, BEAT / 2);
  tl.typing.push({ sheet, line, times, done });
  times.forEach((ct) => tl.events.push({ t: ct, kind: "key" }));
  if (DATA[sheet].results[line]) tl.events.push({ t: done, kind: "pop" });
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

// The hook types this fast, and its answer lands this long after HOOK_IN.
const HOOK_KEY = 0.045;
let HOOK_ANS = 1.2;

function scheduleTyping(tl) {
  const sc = tl.scenes;
  const at = (id, s) => sc[id].start + s;
  // Hook: one big line, typed by the hook layer itself (see renderHook).
  const hook = DATA.hook.lines[0];
  hookTimes = [];
  for (let i = 0; i < hook.length; i++) hookTimes.push(at("hook", HOOK_IN + 0.2 + i * HOOK_KEY + (hook[i] === " " ? 0.02 : 0)));
  HOOK_ANS = onBeat(hookTimes.at(-1) + 0.08, BEAT / 2) - HOOK_IN;
  hookTimes.forEach((ct) => tl.events.push({ t: ct, kind: "key", big: true }));
  tl.events.push({ t: at("hook", HOOK_IN + HOOK_ANS), kind: "pop", big: true });

  // The window flies straight in; Kyoto types, fast.
  tl.events.push({ t: at("intro", -0.12), kind: "whoosh" });
  typeLines(tl, "kyoto", 0, 7, at("intro", 0.75), at("intro", Math.min(3.9, sc.intro.dur - 0.3)));
  // The currency lines, then the times back home.
  typeLines(tl, "kyoto", 8, 10, at("currency", 0.15), at("currency", 1.3));
  typeLines(tl, "kyoto", 11, 13, at("currency", 1.45), at("currency", 2.7));

  // Montage: each one-liner lands on its word in the narration ("Time zones. Tips. ... Runway. ...").
  const words = sc.montage.words;
  const keyWords = [0, 2, 3, 4, 5, 6].map((i) => words[Math.min(i, words.length - 1)].t0);
  const restFrom = words[Math.min(7, words.length - 1)].t0;
  const restTo = sc.montage.start + sc.montage.dur - 0.35;
  const n = DATA.montage.lines.length;
  const extra = n - keyWords.length;
  const starts = [...keyWords.map((t) => t - 0.4), ...Array.from({ length: extra }, (_, k) => restFrom - 0.2 + (k * (restTo - restFrom)) / extra)];
  montage = DATA.montage.lines.map((line, i) => {
    const t0 = Math.max(sc.montage.start + 0.05, starts[i]);
    const typeEnd = t0 + Math.min(0.32, line.length * 0.014);
    const ans = onBeat(typeEnd + 0.04, BEAT / 2);
    return { line, res: DATA.montage.results[i], t0, typeEnd, ans };
  });
  montage.forEach((m, i) => {
    m.t1 = montage[i + 1]?.t0 ?? sc.montage.start + sc.montage.dur;
    tl.events.push({ t: m.t0, kind: "swish" });
    tl.events.push({ t: m.ans, kind: "pop", big: true });
  });

  // Invoice: select 150, type 175, everything ripples.
  tl.events.push({ t: at("invoice", 0.0), kind: "whoosh" });
  [1.45, 1.6, 1.75].forEach((s) => tl.events.push({ t: at("invoice", s), kind: "key" }));
  for (let i = 0; i < 10; i++) tl.events.push({ t: at("invoice", 2.0 + i * 0.07), kind: "roll" });

  // Examples: Trip Budget types, then Monthly budget is already there, reading @trip.total.
  tl.events.push({ t: at("library", -0.05), kind: "whoosh" });
  typeLines(tl, "trip", 0, DATA.trip.lines.length - 1, at("library", 0.25), at("library", 2.5));
  tl.events.push({ t: at("library", 2.7), kind: "swish" });
  tl.events.push({ t: at("library", 3.15), kind: "pop", big: true });

  // Keys: Super+Ctrl+Alt+= and the window pops up; click an answer to copy; hop between sheets.
  // The click and the hops follow the narration ("... hotkey opens it. Click ... Every sheet ...").
  const kw = (w, fallback) => {
    const hit = sc.keys.words.find((x) => x.w.replace(/\W/g, "").toLowerCase() === w);
    return hit ? hit.t0 - sc.keys.start : fallback;
  };
  const hot = kw("hotkey", 0.8);
  KEYS.press = [0, 0.07, 0.14, 0.21].map((d) => hot - 0.25 + d);
  KEYS.pop = kw("opens", 1.3) + 0.05;
  KEYS.click = kw("click", 2.3);
  KEYS.cursorIn = KEYS.click - 0.7;
  KEYS.jumps = [kw("every", 4.3) + 0.05, kw("every", 4.3) + 0.7];
  KEYS.press.forEach((s) => tl.events.push({ t: at("keys", s), kind: "thock" }));
  tl.events.push({ t: at("keys", KEYS.pop - 0.1), kind: "whoosh" });
  tl.events.push({ t: at("keys", KEYS.click), kind: "thock" });
  tl.events.push({ t: at("keys", KEYS.click + 0.08), kind: "pop" });
  KEYS.jumps.forEach((s) => tl.events.push({ t: at("keys", s), kind: "swish" }));

  // Terminal: type a command, the answer prints; then a whole sheet, piped.
  cliTimes = [];
  const cmdKeys = (cmd, a, per) => Array.from(cmd, (_, i) => a + i * per);
  cliTimes.push(cmdKeys(DATA.cli.one.cmd, at("cli", CLI.one), 0.028));
  cliTimes.push(cmdKeys(DATA.cli.pipe.cmd, at("cli", CLI.pipe), 0.019));
  CLI.outAt = cliTimes.map((ts) => onBeat(ts.at(-1) + 0.12, BEAT / 2));
  cliTimes.flat().forEach((ct) => tl.events.push({ t: ct, kind: "key" }));
  CLI.outAt.forEach((ct) => tl.events.push({ t: ct, kind: "pop" }));
  tl.events.push({ t: at("cli", -0.1), kind: "whoosh" });

  // Share: click Share, the link is copied, a phone opens it and adds a line.
  tl.events.push({ t: at("share", -0.05), kind: "whoosh" });
  tl.events.push({ t: at("share", SHARE.click), kind: "thock" });
  tl.events.push({ t: at("share", SHARE.click + 0.12), kind: "pop" });
  tl.events.push({ t: at("share", SHARE.phoneIn - 0.1), kind: "whoosh" });
  const extraLine = DATA.phone.lines.at(-1);
  phoneTimes = [];
  for (let i = 0; i < extraLine.length; i++) phoneTimes.push(at("share", SHARE.typeAt + i * 0.05));
  phoneTimes.forEach((ct) => tl.events.push({ t: ct, kind: "key" }));
  tl.events.push({ t: phoneTimes.at(-1) + 0.12, kind: "pop" });
  // Sync: vaults, then the recovery phrase, then the two devices.
  tl.events.push({ t: at("sync", -0.08), kind: "whoosh" });
  [0.35, 0.55, 0.75].forEach((s) => tl.events.push({ t: at("sync", s), kind: "tick" }));
  tl.events.push({ t: at("sync", 2.4), kind: "pop" });
  tl.events.push({ t: at("sync", 4.6), kind: "swish" });
  // Themes: a whoosh per theme.
  for (let i = 0; i < THEMES.length; i++) tl.events.push({ t: at("themes", themeTime(i)), kind: "swish" });
  // Install command.
  cmdTimes = [];
  for (let i = 0; i < INSTALL.length; i++) cmdTimes.push(at("platforms", 1.0 + i * 0.026));
  cmdTimes.forEach((ct) => tl.events.push({ t: ct, kind: "key" }));
  [0.15, 0.32, 0.49].forEach((s) => tl.events.push({ t: at("platforms", s), kind: "tick" }));
  [at("platforms", -0.15), at("end", -0.1)].forEach((w) => tl.events.push({ t: w, kind: "whoosh" }));
  logoBeads(0.25).forEach((b) => tl.events.push({ t: at("end", b.land), kind: "bead" }));
  tl.events.push({ t: at("end", 1.55), kind: "chime" });
}
const INSTALL = "curl -fsSL https://omacus.com/install | bash";
let hookTimes = [];
let cmdTimes = [];
let phoneTimes = [];
let cliTimes = [];
let montage = [];
// The share scene's beats, in seconds from its start.
const SHARE = { click: 0.9, phoneIn: 1.8, typeAt: 3.3 };
// The keys scene: the hotkey, the window popping up, a click on an answer, two sheet hops.
const KEYS = { press: [0.35, 0.42, 0.49, 0.56], pop: 1.05, cursorIn: 1.6, click: 2.3, jumps: [4.3, 4.95] }; // set from the narration
// The terminal scene: when each command starts typing.
const CLI = { one: 0.45, pipe: 1.75, outAt: [] };
const themeTime = (i) => 0.45 + i * BEAT;

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
  $("#sync .phrase").innerHTML = PHRASE.map((w, i) => `<span>${i + 1} ${esc(w)}</span>`).join("");
  const sprite = (id) => `<svg aria-hidden="true"><use href="/site/img/sprite.svg#${id}"/></svg>`;
  $("#platforms").innerHTML = `
    <div class="icons">
      <div class="os">${sprite("linux")}<span>Linux</span></div>
      <div class="os">${sprite("apple")}<span>macOS</span></div>
      <div class="os">${sprite("windows")}<span>Windows</span></div>
      <div class="os"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3z"/></svg><span>Web</span></div>
    </div>
    <div class="cmd"><span class="p">$ </span><span class="c"></span></div>
    <div class="free">Free · open source · no account</div>`;
  $("#themes").innerHTML =
    THEMES.map(([id]) => `<div class="card"><img src="/site/img/screens/${id}.webp" alt=""></div>`).join("") +
    `<div class="label"><i><b></b><b></b></i><span></span></div>`;
  $("#keys").innerHTML =
    ["Super", "Ctrl", "Alt", "="].map((k, i) => `${i ? '<div class="plus">+</div>' : ""}<div class="key">${k}</div>`).join("") +
    `<div class="omarchy">On Omarchy</div>`;
  $("#term .body").innerHTML = "";
  $("#end .cmd").innerHTML = `<span class="p">$ </span>${esc(INSTALL)}`;
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
  renderLogo(-1);
  renderWindow(t);
  renderKeys(local("keys"));
  renderMontage(t, local("montage"), sc.montage.dur);
  renderTerm(t, local("cli"), sc.cli.dur);
  renderPhone(t, local("share"), sc.share.dur);
  renderSync(local("sync"), sc.sync.dur);
  renderThemes(local("themes"), sc.themes.dur);
  renderPlatforms(t, local("platforms"), sc.platforms.dur);
  renderEnd(local("end"), sc.end.dur);
  renderWipe(t);
  renderCaptions(t);

  // Fade in from black, and out at the very end.
  const fadeIn = 1 - easeInOut(prog(t, 0, PRE * 0.8));
  const fadeOut = prog(t, TL.total - 1.4, TL.total);
  $("#fade").style.opacity = Math.max(fadeIn, easeIn(fadeOut));
  void inScene;
}

function renderHook(t, s, dur) {
  const el = $("#hook");
  if (s < -PRE || s > dur + 0.1) return show(el, 0);
  const text = DATA.hook.lines[0];
  const n = hookTimes.filter((ct) => ct <= t).length;
  // Reserve the full width so the line stays centred while it types.
  el.querySelector(".line").style.width = `${text.length + 0.6}ch`;
  const typed = s - HOOK_IN;
  if (s < 0) {
    // The lead-in: just the caret, blinking, as the frame fades up.
    el.querySelector(".line").innerHTML = Math.floor((s + PRE) * 1.9) % 2 === 0 ? '<span class="caret"></span>' : "";
    el.querySelector(".ans").style.opacity = 0;
    el.style.filter = "";
    return show(el, 1, `scale(${lerp(0.985, 1, easeOut(prog(s, -PRE, 0)))})`);
  }
  const caret = typed < HOOK_ANS && (n < text.length || Math.floor(s * 2.4) % 2 === 0) ? '<span class="caret"></span>' : "";
  el.querySelector(".line").innerHTML = esc(text.slice(0, n)) + caret;
  const a = prog(typed, HOOK_ANS, HOOK_ANS + 0.25);
  const ans = el.querySelector(".ans");
  ans.textContent = DATA.hook.results[0];
  ans.style.opacity = easeOut(a);
  ans.style.transform = `translateY(${lerp(26, 0, easeOutExpo(a))}px)`;
  ans.style.filter = `blur(${lerp(10, 0, easeOut(a))}px)`;
  ans.style.textShadow = `0 0 ${lerp(40, 0, prog(typed, HOOK_ANS, HOOK_ANS + 1))}px rgba(240,161,42,.8)`;
  // A punch on the answer.
  const punch = Math.sin(Math.PI * prog(typed, HOOK_ANS, HOOK_ANS + 0.22)) * 0.035;
  // Push in slowly, then zoom through into the next scene.
  const out = prog(s, dur - 0.45, dur);
  show(el, 1 - easeIn(out), `scale(${(lerp(1, 1.04, s / dur) + punch) * lerp(1, 1.6, easeIn(out))})`);
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
  if (t >= at("share")) return { key: "kyoto", active: 0, prev: 0, since: -1, full: true };
  if (t >= at("keys", KEYS.jumps[1])) return { key: "kyoto", active: 0, prev: 1, since: at("keys", KEYS.jumps[1]), full: true };
  if (t >= at("keys", KEYS.jumps[0])) return { key: "quick", active: 1, prev: 2, since: at("keys", KEYS.jumps[0]), full: true };
  if (t >= at("library", 2.7)) return { key: "monthly", active: 2, prev: 0, since: at("library", 2.7), full: true, library: true };
  if (t >= at("library")) return { key: "trip", active: 0, prev: 0, since: at("library"), library: true };
  if (t >= at("invoice")) return { key: "invoice", active: 2, prev: 2, since: -1, full: true };
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
  if (sIntro < -0.05 || sThemes > 0.8) return show(stage, 0);

  // Fly in from below with a tilt; fly back as the themes take over.
  const pIn = prog(sIntro, 0, 0.75);
  const pOut = prog(sThemes, 0, 0.7);
  // Away for the montage, the hotkey and the terminal; back with a spring.
  const gaps = [
    [sc.montage.start - 0.05, sc.invoice.start],
    [sc.keys.start, sc.keys.start + KEYS.pop],
    [sc.cli.start - 0.05, sc.share.start],
    [sc.sync.start - 0.05, sc.themes.start],
  ];
  let gone = 0;
  let back = 1;
  for (const [a, b] of gaps) {
    if (t >= a - 0.01 && t < b + 0.6) {
      gone = t < b ? easeIn(prog(t, a, a + 0.22)) : 0;
      back = t < b ? 1 : spring(prog(t, b, b + 0.55));
    }
  }
  if (gone >= 0.999) return show(stage, 0);
  const rx = lerp(24, 0, easeOutExpo(pIn)) + lerp(0, -10, easeIn(pOut));
  const ty = lerp(420, 0, easeOutExpo(pIn)) + lerp(0, -60, easeIn(pOut));
  const sc0 = lerp(0.86, 1, easeOutExpo(pIn)) * lerp(1, 0.78, easeIn(pOut));
  // Step aside for the phone during the share scene.
  const sShare = t - sc.share.start;
  const aside = easeInOut(prog(sShare, SHARE.phoneIn - 0.3, SHARE.phoneIn + 0.6));
  const popIn = clamp(back, 0, 2);
  show(
    stage,
    easeOut(prog(sIntro, 0, 0.35)) * (1 - easeIn(pOut)) * (1 - gone) * clamp(popIn * 2),
    `perspective(2400px) translateX(${lerp(0, -330, aside)}px) translateY(${ty}px) rotateX(${rx}deg) rotateY(${lerp(0, 8, aside)}deg) scale(${sc0 * lerp(1, 0.72, aside) * lerp(1, 0.9, gone) * lerp(0.86, 1, popIn)})`,
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
  const sKeys = t - sc.keys.start;
  const copying = sKeys >= 0 && sKeys < KEYS.jumps[0];
  const toastAt = copying ? KEYS.click : SHARE.click;
  const sToast = copying ? sKeys : sShare;
  toast.innerHTML = copying ? `Copied <b>${esc(DATA.invoice175.results[8])}</b>` : "Link copied. <span>Anyone with it can open this page.</span>";
  const tIn = prog(sToast, toastAt + 0.1, toastAt + 0.4);
  const tOut = prog(sToast, toastAt + (copying ? 1.0 : 2.4), toastAt + (copying ? 1.25 : 2.7));
  show(toast, easeOut(tIn) * (1 - tOut), `translateY(${lerp(-10, 0, easeOutExpo(tIn))}px)`);
  renderCursor(sKeys);
  stage.style.transformOrigin = "50% 60%";

  const view = sheetView(t);
  // The example library replaces the sidebar for that one scene, then the earlier list returns.
  const sideMode = view.library ? "library" : "pages";
  if (sideMode !== renderWindow.sideMode) {
    $("#sheets").innerHTML = view.library
      ? `<div class="folder">Examples</div>${["Trip Budget", "Groceries", "Monthly budget", "Cooking", "Time zones"]
          .map((s) => `<div class="sheet in">${esc(s)}</div>`)
          .join("")}<div class="sheet">Welcome to Omacus</div>`
      : DATA.sheets.map((s) => `<div class="sheet">${esc(s)}</div>`).join("");
    renderWindow.sideMode = sideMode;
  }
  // Sidebar: the highlight glides to the new page; the content slides in.
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

  show($("#switcher"), 0);
  $("#scrim").style.opacity = 0;
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
  sw.querySelector(".q").innerHTML = q ? esc(q) + '<span class="caret" style="height:26px;width:3px;vertical-align:-4px"></span>' : '<span class="hint">Go to page…</span>';
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
  // `e` is the easing into that key: punches use easeOutExpo.
  const k = (id, s, pos, z, e = easeInOut) => keys.push({ t: sc[id].start + s, x: pos.x, y: pos.y, z, e });
  const punch = (p) => easeOutExpo(p) + Math.sin(Math.PI * clamp(p * 1.4)) * 0.04;
  k("intro", 0, full, 1);
  k("intro", sc.intro.dur - 0.2, full, 1);
  // Currency: punch in on the totals, then drift to the times back home.
  k("currency", 0.15, { x: 1080, y: L(9).y }, 1.5, punch);
  k("currency", 1.4, { x: 1080, y: L(9.5).y }, 1.5);
  k("currency", 1.7, { x: 1040, y: L(12).y }, 1.3, punch);
  k("currency", sc.currency.dur, { x: 1040, y: L(12).y }, 1.34);
  k("montage", 0, full, 1);
  // Invoice: wide enough for the rate and every answer it changes; a punch on the ripple.
  k("invoice", 0.5, { x: 1180, y: L(4).y }, 1.3);
  k("invoice", 1.95, { x: 1180, y: L(4).y }, 1.3);
  k("invoice", 2.25, { x: 1200, y: L(5).y }, 1.48, punch);
  k("invoice", 3.3, { x: 1200, y: L(5).y }, 1.48);
  k("invoice", sc.invoice.dur, { x: 1120, y: L(4.5).y }, 1.14);
  k("library", 0.2, full, 1);
  k("library", 2.2, { x: 1180, y: L(5).y }, 1.35, punch);
  k("library", 2.65, full, 1);
  k("library", 3.3, { x: 1200, y: L(4).y }, 1.42, punch);
  k("library", sc.library.dur, { x: 1140, y: L(4).y }, 1.2);
  k("keys", 0.01, full, 1, () => 1);
  // Click to copy: lean in on the answer.
  k("keys", KEYS.cursorIn, full, 1);
  k("keys", KEYS.click - 0.15, { x: 1380, y: L(8).y }, 1.35, punch);
  k("keys", KEYS.click + 0.9, { x: 1380, y: L(8).y }, 1.35);
  k("keys", KEYS.jumps[0] - 0.1, full, 1);
  k("share", 0.15, full, 1);
  k("share", 0.6, { x: 1420, y: 230 }, 1.55, punch);
  k("share", SHARE.click + 0.4, { x: 1420, y: 230 }, 1.55);
  k("share", SHARE.phoneIn - 0.1, full, 1);
  k("themes", 0, full, 1);
  if (t <= keys[0].t) return keys[0];
  for (let i = 1; i < keys.length; i++) {
    if (t <= keys[i].t) {
      const a = keys[i - 1];
      const b = keys[i];
      const p = b.e(prog(t, a.t, b.t));
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

// The hotkey: Super + Ctrl + Alt + =, pressed and held, then the window pops up.
function renderKeys(s) {
  const el = $("#keys");
  if (s < 0 || s > KEYS.pop + 0.35) return show(el, 0);
  const vis = easeOut(prog(s, 0.0, 0.12)) * (1 - easeIn(prog(s, KEYS.pop - 0.05, KEYS.pop + 0.3)));
  show(el, vis, `scale(${lerp(1, 1.25, easeIn(prog(s, KEYS.pop - 0.05, KEYS.pop + 0.3)))})`);
  const badge = el.querySelector(".omarchy");
  const pb = prog(s, 0.0, 0.35);
  badge.style.opacity = easeOut(pb);
  badge.style.transform = `translate(-50%, ${lerp(16, 0, easeOutExpo(pb))}px)`;
  el.querySelectorAll(".key").forEach((key, i) => {
    const at = KEYS.press[i];
    const p = prog(s, at, at + 0.05) * (1 - prog(s, KEYS.pop - 0.1, KEYS.pop));
    key.style.transform = `translateY(${5 * p}px)`;
    key.style.boxShadow = `0 ${6 - 5 * p}px 0 #0c0c0c, 0 20px 40px -10px rgba(0,0,0,.8)${p > 0.2 ? ", 0 0 0 2px rgba(240,161,42,.7)" : ""}`;
  });
}

// A pointer that glides to the total and clicks it.
function renderCursor(s) {
  const cur = $("#cursor");
  const hl = $("#copyhl");
  const on = s >= KEYS.cursorIn - 0.1 && s < KEYS.click + 1.2;
  show(cur, on ? easeOut(prog(s, KEYS.cursorIn - 0.1, KEYS.cursorIn + 0.15)) * (1 - prog(s, KEYS.click + 0.9, KEYS.click + 1.2)) : 0);
  const p = easeInOut(prog(s, KEYS.cursorIn, KEYS.click - 0.1));
  const press = prog(s, KEYS.click, KEYS.click + 0.05) * (1 - prog(s, KEYS.click + 0.12, KEYS.click + 0.22));
  cur.style.left = `${lerp(760, 1080, p)}px`;
  cur.style.top = `${lerp(620, 34 + 46 * 8 + 14, p)}px`;
  cur.style.transform = `scale(${1 - 0.15 * press})`;
  const flash = s >= KEYS.click ? Math.exp(-(s - KEYS.click) * 3) : 0;
  hl.style.opacity = flash;
}

// The montage: one big line at a time, the answer slams in under it.
function renderMontage(t, s, dur) {
  const el = $("#montage");
  if (s < 0 || s > dur) return show(el, 0);
  const m = montage.find((x) => t >= x.t0 && t < x.t1) ?? (t < montage[0].t0 ? null : montage.at(-1));
  if (!m) return show(el, 0);
  const sIn = t - m.t0;
  const exit = prog(t, m.t1 - 0.12, m.t1);
  const last = m === montage.at(-1);
  const out = last ? prog(s, dur - 0.3, dur) : 0;
  const n = Math.round(m.line.length * clamp(sIn / Math.max(0.01, m.typeEnd - m.t0)));
  const typing = t < m.typeEnd;
  el.querySelector(".line").innerHTML = esc(m.line.slice(0, n)) + (typing || t < m.ans ? '<span class="caret"></span>' : "");
  el.querySelector(".line").style.width = `${m.line.length + 0.6}ch`;
  const a = prog(t, m.ans, m.ans + 0.18);
  const ans = el.querySelector(".ans");
  ans.textContent = m.res;
  ans.style.opacity = easeOut(a);
  ans.style.transform = `translateY(${lerp(30, 0, easeOutExpo(a))}px) scale(${lerp(1.25, 1, easeOutExpo(a))})`;
  ans.style.textShadow = `0 0 ${lerp(46, 0, prog(t, m.ans, m.ans + 0.8))}px rgba(240,161,42,.85)`;
  const punch = Math.sin(Math.PI * prog(t, m.ans, m.ans + 0.2)) * 0.04;
  show(el, (1 - exit) * (1 - out) * easeOut(prog(sIn, 0, 0.08)), `translateY(${lerp(0, -40, easeIn(exit))}px) scale(${lerp(0.94, 1, easeOutExpo(prog(sIn, 0, 0.25))) + punch})`);
  el.style.filter = `blur(${lerp(0, 10, easeIn(exit))}px)`;
  // A row of dots: which one-liner this is.
  el.querySelector(".dots").innerHTML = montage.map((x) => `<i class="${x === m ? "on" : ""}"></i>`).join("");
}

// A terminal: `omacus '$100 in EUR'`, then a sheet piped through `omacus -`.
function renderTerm(t, s, dur) {
  const el = $("#term");
  if (s < -0.1 || s > dur) return show(el, 0);
  const pIn = prog(s, -0.05, 0.4);
  const out = prog(s, dur - 0.3, dur);
  show(el, easeOut(prog(s, -0.05, 0.15)) * (1 - easeIn(out)), `translateY(${lerp(80, 0, easeOutExpo(pIn))}px) scale(${lerp(0.94, 1, easeOutExpo(pIn)) * lerp(1, 0.96, easeIn(out))})`);
  const cmds = [DATA.cli.one, DATA.cli.pipe];
  let html = "";
  cmds.forEach((c, i) => {
    const ts = cliTimes[i];
    if (t < ts[0] - 0.4) return;
    const n = ts.filter((ct) => ct <= t).length;
    const typing = n < c.cmd.length;
    const showCaret = t < CLI.outAt[i] && (typing || Math.floor(s * 2.4) % 2 === 0);
    html += `<div><span class="p">~ $ </span>${esc(c.cmd.slice(0, n))}${showCaret ? '<span class="caret"></span>' : ""}</div>`;
    if (t >= CLI.outAt[i]) {
      c.out.forEach((o, j) => {
        const at = CLI.outAt[i] + j * 0.07;
        if (t < at) return;
        const p = prog(t, at, at + 0.2);
        html += `<div class="o" style="opacity:${easeOut(p)};text-shadow:0 0 ${lerp(24, 0, prog(t, at, at + 0.8))}px rgba(240,161,42,.8)">${esc(o)}</div>`;
      });
    }
  });
  const last = cliTimes.at(-1);
  if (t >= CLI.outAt[1] + 0.3) html += `<div><span class="p">~ $ </span>${Math.floor(s * 2.4) % 2 === 0 ? '<span class="caret"></span>' : ""}</div>`;
  void last;
  el.querySelector(".body").innerHTML = html;
}

const PHRASE = "amber birch cedar drift ember fern grove harbor iris juniper kettle lantern meadow north orchard pebble quartz river spruce timber umber valley willow yarrow".split(" ");

function renderSync(s, dur) {
  const el = $("#sync");
  if (s < -0.2 || s > dur + 0.1) return show(el, 0);
  const pIn = prog(s, 0, 0.55);
  const out = prog(s, dur - 0.45, dur);
  show(el, easeOut(prog(s, 0, 0.25)) * (1 - easeIn(out)), `translateY(${lerp(40, 0, easeOutExpo(pIn))}px) scale(${lerp(0.96, 1, easeOutExpo(pIn))})`);
  el.querySelectorAll(".vault").forEach((card, i) => {
    const p = prog(s, 0.2 + i * 0.16, 0.55 + i * 0.16);
    card.style.opacity = easeOut(p);
    card.style.transform = `translateY(${lerp(18, 0, easeOutExpo(p))}px)`;
  });
  const words = el.querySelectorAll(".phrase span");
  words.forEach((word, i) => {
    const p = prog(s, 1.7 + i * 0.06, 2.05 + i * 0.06);
    word.style.opacity = easeOut(p);
  });
  el.querySelectorAll(".device").forEach((device, i) => {
    const p = prog(s, 4.15 + i * 0.2, 4.7 + i * 0.2);
    device.style.opacity = easeOut(p);
    device.style.transform = `translateY(${lerp(16, 0, easeOutExpo(p))}px)`;
  });
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
  const cmd = INSTALL;
  const n = cmdTimes.filter((ct) => ct <= t).length;
  const box = el.querySelector(".cmd");
  box.style.opacity = easeOut(prog(s, 0.7, 0.95));
  box.style.transform = `translateY(${lerp(20, 0, easeOutExpo(prog(s, 0.7, 1.15)))}px)`;
  const blink = Math.floor(s * 2.4) % 2 === 0 || n < cmd.length;
  box.querySelector(".c").innerHTML = esc(cmd.slice(0, n)) + (blink ? '<span class="caret"></span>' : "");
  el.querySelector(".free").style.opacity = easeOut(prog(s, 2.3, 2.7));
}

function renderEnd(s, dur) {
  const el = $("#end");
  if (s < 0) return show(el, 0);
  show(el, 1);
  animateLogo(el.querySelector(".logo-svg"), s, 0.25);
  animateWord(el.querySelector(".word"), s, 0.85);
  [el.querySelector(".tag"), el.querySelector(".url"), el.querySelector(".cmd"), el.querySelector(".sub")].forEach((p, i) => {
    const at = 1.2 + i * 0.25;
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
  const cuts = [sc.intro.start - 0.05, sc.montage.start - 0.05, sc.cli.start - 0.05, sc.platforms.start - 0.1, sc.end.start - 0.05];
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
