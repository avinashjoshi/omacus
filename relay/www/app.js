import init, * as sync from "./pkg/omacus_sync_web.js";

const stage = document.getElementById("stage");
const title = document.getElementById("page-title");
const params = new URLSearchParams(location.search);

const state = {
  server: params.get("server") || location.origin,
  screen: "start",
  desk: null,
  words: [],
  qr: "",
  asks: [3, 7, 11],
  picks: {},
  choices: {},
  wrong: 0,
  account: "",
  record: "",
  revision: 0,
  code: "",
  device: "",
  requestId: "",
  pending: null,
  phrase: "",
  phraseMsg: "",
  phraseOk: false,
  page: "Trip Budget\nferry = 40\nmilk $3",
  error: "",
  busy: false,
  note: "",
  poll: 0,
  invite: null,
};

function esc(value) {
  return String(value).replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  }[ch]));
}

function serverBase() {
  const field = document.querySelector("#server");
  const raw = (field && field.value.trim()) || state.server;
  return raw.replace(/\/$/, "");
}

async function api(method, path, body) {
  const response = await fetch(`${serverBase()}${path}`, {
    method,
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.message || "Try again in a moment.");
  return data;
}

function stopPoll() {
  if (state.poll) {
    clearInterval(state.poll);
    state.poll = 0;
  }
}

function show(screen) {
  stopPoll();
  state.screen = screen;
  state.error = "";
  render();
}

function editor(text) {
  const lines = String(text).split("\n");
  const body = lines
    .map((line, index) => (index === 0 ? `<div class="h">${esc(line)}</div>` : esc(line)))
    .join("<br>");
  return `<div class="editor">${body}</div>`;
}

function sheet(inner, withPage) {
  return `${withPage ? editor(state.page) + '<div class="scrim"></div>' : ""}<div class="sheet" data-screen="${state.screen}">${inner}</div>`;
}

function actions(buttons) {
  return `<div class="actions">${buttons
    .map(
      ([act, label, kind]) =>
        `<button class="btn ${kind || ""}" type="button" data-act="${act}" ${state.busy ? "disabled" : ""}>${esc(label)}</button>`,
    )
    .join("")}</div>`;
}

function wordsGrid(tappable) {
  const wide = window.matchMedia("(min-width: 801px)").matches ? " three" : "";
  return `<div class="words${wide}">${state.words
    .map((word, index) => {
      const n = index + 1;
      const picked = state.picks[n] ? " picked" : "";
      const tag = tappable ? "button" : "div";
      const act = tappable ? ` data-act="tap" type="button"` : "";
      return `<${tag} class="word${picked}" data-n="${n}" data-word="${esc(word)}"${act}><b>${n}</b>${esc(word)}</${tag}>`;
    })
    .join("")}</div>`;
}

function asks() {
  return `<div class="checks">${state.asks
    .map(
      (n) => `<fieldset class="check" data-ask="${n}"><legend>Word ${n}</legend><div class="choices">${(state.choices[n] || [])
        .map((word) => {
          const on = state.picks[n] === word;
          return `<button class="choice${on ? " picked" : ""}" type="button" data-act="choose" data-n="${n}" data-word="${esc(word)}" aria-pressed="${on}">${esc(word)}</button>`;
        })
        .join("")}</div></fieldset>`,
    )
    .join("")}</div>`;
}

let wordlist = [];
async function loadWordlist() {
  const text = await (await fetch("./words.txt")).text();
  wordlist = text.split("\n").map((line) => line.trim()).filter((line) => line && !line.startsWith("#"));
}

function randomInt(max) {
  const buf = new Uint32Array(1);
  crypto.getRandomValues(buf);
  return buf[0] % max;
}

function shuffle(list) {
  for (let i = list.length - 1; i > 0; i -= 1) {
    const j = randomInt(i + 1);
    [list[i], list[j]] = [list[j], list[i]];
  }
  return list;
}

// Three random word numbers, each with its word and five decoys from the full
// list. Decoys never come from the other words on this sheet.
function setUpCheck() {
  state.asks = shuffle([...Array(12).keys()].map((i) => i + 1)).slice(0, 3).sort((a, b) => a - b);
  state.picks = {};
  state.wrong = 0;
  state.error = "";
  const mine = new Set(state.words.map((word) => word.toLowerCase()));
  state.choices = {};
  for (const n of state.asks) {
    const options = new Set([state.words[n - 1]]);
    while (options.size < 6 && wordlist.length) {
      const word = wordlist[randomInt(wordlist.length)];
      if (!mine.has(word.toLowerCase())) options.add(word);
    }
    state.choices[n] = shuffle([...options]);
  }
}

function qrBlock() {
  const svg = state.qr.startsWith("<svg") && !state.qr.includes("<script") ? state.qr : "";
  return `<div class="qr">${svg}<span>On the printed sheet</span></div>`;
}

function render() {
  title.textContent = state.screen === "start" ? "Sync test" : "Trip Budget";
  const err = state.error ? `<p class="err">${esc(state.error)}</p>` : "";
  let html = "";
  if (state.screen === "start") {
    html = sheet(
      `<p class="kicker">Private test</p>
      <h1>Sync test</h1>
      <p class="lead">A private copy, labeled 1.0.0-beta.1. Pages stay sealed. This server cannot open them.</p>
      <label class="lead" for="server">Test server</label>
      <input class="field" id="server" value="${esc(state.server)}" autocomplete="off" spellcheck="false">
      ${actions([
        ["turn-on", "Turn on sync", "primary block"],
        ["add", "Add this device", "block"],
        ["recover", "Use your recovery key", "block"],
      ])}
      ${err}`,
      false,
    );
  } else if (state.screen === "turn-on") {
    html = sheet(
      `<p class="kicker">Step 1 of 3</p>
      <h1>Turn on sync</h1>
      <p class="lead">Your pages can open on your other devices. They stay private. We never see them.</p>
      <p class="lead">Next you will get 12 words. Save them. They are the way back in if you lose every device.</p>
      ${actions([
        ["make-key", "Turn on sync", "primary block"],
        ["start", "Not now", "block"],
      ])}
      ${err}`,
      true,
    );
  } else if (state.screen === "save") {
    html = sheet(
      `<p class="kicker">Step 2 of 3</p>
      <h1>Save your recovery key</h1>
      <p class="lead">These 12 words are shown once. Write them down, or print the sheet. The code is the same words.</p>
      <div class="key">${wordsGrid(false)}${qrBlock()}</div>
      <p class="warn">If you lose these words and every device, nobody can open the pages. Not us either.</p>
      ${actions([
        ["print", "Print", "grow"],
        ["copy", "Copy", "grow"],
        ["to-confirm", "I saved them", "primary block"],
      ])}`,
      true,
    );
  } else if (state.screen === "confirm" || state.screen === "recover-confirm") {
    const step = state.screen === "confirm" ? `<p class="kicker">Step 3 of 3</p>` : "";
    html = sheet(
      `${step}
      <h1>Pick the three words</h1>
      <p class="lead">${esc(leadAsks())}</p>
      ${asks()}
      ${state.wrong ? `<p class="err" role="alert">That's not word ${state.wrong}. Go back to check your words.</p>` : err}
      <div class="actions">
        <button class="btn primary block" type="button" data-act="confirm" ${state.busy || chosenPicks().length !== 3 ? "disabled" : ""}>${state.busy ? "Checking the words…" : "Confirm"}</button>
        <button class="btn block" type="button" data-act="back" ${state.busy ? "disabled" : ""}>${state.wrong ? "Back to the words" : "Back"}</button>
      </div>`,
      true,
    );
  } else if (state.screen === "ready" || state.screen === "opened") {
    const note = state.note ? `<p class="ok banner">${esc(state.note)}</p>` : "";
    const add = state.screen === "ready"
      ? `<div class="actions"><button class="btn block" type="button" data-act="invite">Add a device</button></div>`
      : "";
    html = `<div data-screen="${state.screen}">${editor(state.page)}${note}${add}</div>`;
  } else if (state.screen === "invite") {
    const svg = state.qr.startsWith("<svg") && !state.qr.includes("<script") ? state.qr : "";
    html = sheet(
      `<h1>Add a device</h1>
      <p class="lead">On the new device, scan this code or type it. It works for about 5 minutes, and only once.</p>
      <div class="qr">${svg}</div>
      <div class="invite-code">${esc(state.invite ? state.invite.code : "")}</div>
      ${err}
      ${actions([["ready", "Back", "block"]])}`,
      true,
    );
  } else if (state.screen === "join-start") {
    html = sheet(
      `<h1>Add this device</h1>
      <p class="lead">On a device that already has your pages, open Sync and tap Add a device.</p>
      ${err}
      ${actions([
        ["join-type", "Type code", "primary block"],
        ["start", "Back", "block"],
      ])}`,
      true,
    );
  } else if (state.screen === "join-type") {
    html = sheet(
      `<h1>Type the code</h1>
      <p class="lead">Type the code shown on your other device. It looks like K7QM-4R2P.</p>
      <input class="field" id="join-code" autocomplete="off" spellcheck="false" autocapitalize="characters" aria-label="Device code">
      ${err}
      ${actions([
        ["join-go", state.busy ? "Checking…" : "Continue", "primary block"],
        ["add", "Back", "block"],
      ])}`,
      true,
    );
  } else if (state.screen === "join") {
    html = sheet(
      `<h1>Add this device</h1>
      <p class="lead">On a device that already has your pages, open Omacus. The same code is there. Tap Approve.</p>
      <div class="code">${esc(state.code)}</div>
      <p class="waiting">${esc(state.busy ? "Opening your pages…" : "Waiting for that device")}</p>
      ${err}
      ${actions([["start", "Cancel", "block"]])}`,
      true,
    );
  } else if (state.screen === "approve") {
    const name = state.pending ? state.pending.name : "New device";
    html = sheet(
      `<h1>Approve a new device?</h1>
      <p class="lead">${esc(name)} wants to open your pages. Tap Approve only if that device is yours and the code matches.</p>
      <div class="code">${esc(state.pending ? state.pending.code : "")}</div>
      <p class="hint">Your pages stay private. Only your devices can open them.</p>
      ${err}
      ${actions([
        ["approve", state.busy ? "Approving…" : "Approve", "primary block"],
        ["decline", "Not now", "block"],
      ])}`,
      true,
    );
  } else if (state.screen === "recover") {
    html = sheet(
      `<h1>Use your recovery key</h1>
      <p class="lead">Paste or type the whole phrase. Numbers, commas, and capitals are fine.</p>
      <textarea class="phrase" id="phrase" spellcheck="false">${esc(state.phrase)}</textarea>
      <p class="${state.phraseOk ? "ok" : "hint"}" id="phrase-msg">${esc(state.phraseMsg)}</p>
      ${state.video ? `<video class="video" id="scan" autoplay playsinline></video>` : ""}
      ${err}
      ${actions([
        ["scan", "Scan recovery sheet", "block"],
        ["open-pages", state.busy ? "Checking the words…" : "Open my pages", "primary block"],
      ])}`,
      true,
    );
  }
  stage.innerHTML = html;
  const phrase = document.getElementById("phrase");
  if (phrase) {
    phrase.addEventListener("input", onPhrase);
    phrase.focus();
    phrase.setSelectionRange(phrase.value.length, phrase.value.length);
  }
}

function leadAsks() {
  const [a, b, c] = state.asks;
  return `From your sheet, pick word ${a}, word ${b}, and word ${c}.`;
}

async function onPhrase() {
  state.phrase = document.getElementById("phrase").value;
  const check = JSON.parse(sync.checkPhrase(state.phrase));
  state.phraseOk = check.ok;
  state.phraseMsg = state.phrase.trim() ? check.message : "";
  const msg = document.getElementById("phrase-msg");
  if (msg) {
    msg.textContent = state.phraseMsg;
    msg.className = state.phraseOk ? "ok" : "hint";
  }
}

function numberedWords() {
  return state.words.map((word, index) => `${index + 1}. ${word}`).join("\n");
}

function chosenPicks() {
  return state.asks.filter((n) => state.picks[n]).map((n) => [n, state.picks[n]]);
}

async function makeKey() {
  state.busy = true;
  state.error = "";
  render();
  await waitFrame();
  const result = JSON.parse(sync.startNew());
  state.busy = false;
  if (!result.ok) {
    state.error = result.message;
    render();
    return;
  }
  state.desk = result.id;
  state.words = result.words;
  state.qr = result.qr;
  state.asks = result.asks;
  state.picks = {};
  show("save");
}

async function confirmWords() {
  const picks = chosenPicks();
  if (picks.length !== 3) {
    state.error = "Pick the three words first.";
    render();
    return;
  }
  const miss = picks.find(([n, word]) => word !== state.words[n - 1]);
  if (miss) {
    state.wrong = miss[0];
    render();
    return;
  }
  state.busy = true;
  state.error = "";
  render();
  await waitFrame();
  if (state.screen === "confirm") {
    if (!state.record) {
      const result = JSON.parse(sync.confirmSaved(state.desk, JSON.stringify(picks)));
      if (!result.ok) {
        state.busy = false;
        state.error = "Those are not the three words.";
        render();
        return;
      }
      state.account = result.account;
      state.record = result.record;
    }
    try {
      const created = await api("POST", "/v1/mailboxes", {
        account: state.account,
        record: state.record,
      });
      state.revision = created.revision;
      state.note = "This device is on. A new device will ask before it can open these pages.";
      state.busy = false;
      show("ready");
      pollApproval();
    } catch (error) {
      state.busy = false;
      state.error = error.message;
      render();
    }
    return;
  }
  if (!state.account) {
    const result = JSON.parse(sync.recoveryAccount(state.desk, JSON.stringify(picks)));
    if (!result.ok) {
      state.busy = false;
      state.error = "Those are not the three words.";
      render();
      return;
    }
    state.account = result.account;
  }
  try {
    const mailbox = await api("GET", `/v1/mailboxes/${state.account}`);
    const opened = JSON.parse(sync.recoveryOpen(state.desk, mailbox.record));
    state.busy = false;
    if (!opened.ok) {
      state.error = opened.message;
      render();
      return;
    }
    state.page = opened.page;
    state.note = "Opened with your recovery key.";
    show("opened");
  } catch (error) {
    state.busy = false;
    state.error = error.message;
    render();
  }
}

function pollApproval() {
  stopPoll();
  state.poll = setInterval(lookForDevice, 1000);
  lookForDevice();
}

async function lookForDevice() {
  if (!state.account || state.screen === "approve") return;
  if (!state.invite || Date.now() >= state.invite.expires) return;
  try {
    const data = await api("GET", `/v1/mailboxes/${state.account}/requests`);
    for (const request of data.requests || []) {
      const check = JSON.parse(sync.checkProof(state.invite.code, request.device, request.proof || ""));
      if (!check.ok) continue;
      const coded = JSON.parse(sync.codeFor(request.device));
      state.pending = { id: request.id, name: request.name, device: request.device, code: coded.code };
      show("approve");
      return;
    }
  } catch (error) {
    state.error = error.message;
  }
}

function beginJoin() {
  state.server = serverBase();
  state.error = "";
  show("join-start");
}

async function startInvite() {
  state.busy = true;
  state.error = "";
  render();
  const raw = crypto.getRandomValues(new Uint8Array(5));
  const secret = btoa(String.fromCharCode(...raw));
  const made = JSON.parse(sync.makeInvite(state.account, location.origin, secret));
  if (!made.ok) {
    state.busy = false;
    state.error = made.message;
    render();
    return;
  }
  try {
    await api("POST", `/v1/mailboxes/${state.account}/invites`, { hash: made.hash });
  } catch (error) {
    state.busy = false;
    state.error = error.message;
    render();
    return;
  }
  state.invite = { code: made.code, qr: made.qr, expires: Date.now() + 5 * 60 * 1000 };
  state.qr = made.qr;
  state.busy = false;
  show("invite");
}

async function sendJoin() {
  const code = document.getElementById("join-code")?.value || "";
  state.busy = true;
  state.error = "";
  render();
  const started = JSON.parse(sync.startJoin());
  if (!started.ok) {
    state.busy = false;
    state.error = started.message || "That code didn't work. Check it on the other device.";
    show("join-type");
    state.error = state.error;
    render();
    return;
  }
  const proof = JSON.parse(sync.joinProof(started.device, code));
  if (!proof.ok) {
    state.busy = false;
    state.error = proof.message || "That code didn't work. Check it on the other device.";
    show("join-type");
    state.error = state.error;
    render();
    return;
  }
  try {
    const created = await api("POST", "/v1/join", {
      code,
      device: started.device,
      name: "This phone",
      proof: proof.proof,
    });
    state.desk = started.id;
    state.code = started.code;
    state.device = started.device;
    state.account = created.account;
    state.requestId = created.id;
  } catch (error) {
    state.busy = false;
    state.error = error.message;
    show("join-type");
    state.error = error.message;
    render();
    return;
  }
  state.busy = false;
  show("join");
  state.poll = setInterval(waitForApproval, 1000);
}

async function waitForApproval() {
  try {
    const decision = await api("GET", `/v1/mailboxes/${state.account}/requests/${state.requestId}`);
    if (decision.state === "pending") return;
    stopPoll();
    if (decision.state !== "approved") {
      state.error = "That device did not approve.";
      show("join");
      state.error = "That device did not approve.";
      render();
      return;
    }
    state.busy = true;
    render();
    const mailbox = await api("GET", `/v1/mailboxes/${state.account}`);
    const opened = JSON.parse(
      sync.openApproval(state.desk, decision.chain, decision.wrap, mailbox.record),
    );
    state.busy = false;
    if (!opened.ok) {
      state.error = opened.message;
      render();
      return;
    }
    state.page = opened.page;
    state.note = "This device can open your pages.";
    show("opened");
  } catch (error) {
    state.error = error.message;
  }
}

async function approve() {
  state.busy = true;
  render();
  await waitFrame();
  const result = JSON.parse(sync.approveDevice(state.desk, state.pending.device));
  if (!result.ok) {
    state.busy = false;
    state.error = result.message;
    render();
    return;
  }
  try {
    const put = await api("PUT", `/v1/mailboxes/${state.account}`, {
      revision: state.revision,
      record: result.record,
    });
    state.revision = put.revision;
    await api("POST", `/v1/mailboxes/${state.account}/requests/${state.pending.id}`, {
      chain: result.chain,
      wrap: result.wrap,
    });
    state.pending = null;
    state.busy = false;
    state.note = "That device can open your pages.";
    show("ready");
    pollApproval();
  } catch (error) {
    state.busy = false;
    state.error = error.message;
    render();
  }
}

async function decline() {
  if (!state.pending) return;
  try {
    await api("POST", `/v1/mailboxes/${state.account}/requests/${state.pending.id}`, { reject: true });
  } catch (error) {
    state.error = error.message;
  }
  state.pending = null;
  show("ready");
  pollApproval();
}

async function openRecovery() {
  state.phrase = document.getElementById("phrase")?.value ?? state.phrase;
  const started = JSON.parse(sync.startRecovery(state.phrase));
  if (!started.ok) {
    state.phraseOk = false;
    state.phraseMsg = started.message;
    state.error = "";
    render();
    return;
  }
  state.desk = started.id;
  state.words = started.words;
  setUpCheck();
  show("recover-confirm");
}

async function scan() {
  const input = document.createElement("input");
  input.type = "file";
  input.accept = "image/*";
  input.capture = "environment";
  if ("BarcodeDetector" in window && navigator.mediaDevices) {
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: "environment" } });
      state.video = true;
      render();
      const video = document.getElementById("scan");
      video.srcObject = stream;
      const detector = new BarcodeDetector({ formats: ["qr_code"] });
      const tick = async () => {
        if (!state.video) {
          stream.getTracks().forEach((track) => track.stop());
          return;
        }
        const codes = await detector.detect(video).catch(() => []);
        const raw = codes[0] && codes[0].rawValue;
        if (raw) {
          state.video = false;
          stream.getTracks().forEach((track) => track.stop());
          fillFromScan(raw);
          return;
        }
        requestAnimationFrame(tick);
      };
      video.onloadeddata = tick;
      return;
    } catch {
      /* Fall through to a picture of the sheet. */
    }
  }
  input.addEventListener("change", async () => {
    const file = input.files && input.files[0];
    if (!file || !("BarcodeDetector" in window)) {
      state.error = "This browser cannot scan. Paste the words instead.";
      render();
      return;
    }
    const bitmap = await createImageBitmap(file);
    const codes = await new BarcodeDetector({ formats: ["qr_code"] }).detect(bitmap);
    if (!codes[0]) {
      state.error = "That is not a recovery sheet.";
      render();
      return;
    }
    fillFromScan(codes[0].rawValue);
  });
  input.click();
}

function fillFromScan(raw) {
  if (!raw.startsWith("omacus-recovery")) {
    state.error = "That is not a recovery sheet.";
    state.screen = "recover";
    render();
    return;
  }
  state.phrase = raw.replace(/^omacus-recovery\s*/, "");
  state.screen = "recover";
  state.video = false;
  render();
  onPhrase();
}

function waitFrame() {
  return new Promise((resolve) => setTimeout(resolve, 40));
}

const acts = {
  home() {
    show("start");
  },
  start() {
    show("start");
  },
  "turn-on"() {
    state.server = serverBase();
    show("turn-on");
  },
  "make-key": makeKey,
  "to-confirm"() {
    setUpCheck();
    show("confirm");
  },
  back() {
    state.wrong = 0;
    state.error = "";
    show(state.screen === "recover-confirm" ? "recover" : "save");
  },
  tap(button) {
    const n = Number(button.dataset.n);
    if (state.picks[n]) {
      delete state.picks[n];
    } else if (Object.keys(state.picks).length < 3) {
      state.picks[n] = button.dataset.word;
    }
    render();
  },
  choose(button) {
    const n = Number(button.dataset.n);
    state.picks[n] = button.dataset.word;
    state.wrong = 0;
    state.error = "";
    render();
  },
  confirm: confirmWords,
  copy() {
    navigator.clipboard.writeText(numberedWords()).catch(() => {});
  },
  print() {
    window.print();
  },
  add: beginJoin,
  invite: startInvite,
  "join-type"() {
    state.error = "";
    show("join-type");
  },
  "join-go": sendJoin,
  ready() {
    show("ready");
  },
  approve,
  decline,
  recover() {
    state.server = serverBase();
    state.phraseMsg = "";
    state.phraseOk = false;
    show("recover");
  },
  scan,
  "open-pages": openRecovery,
  theme() {
    const next = document.documentElement.dataset.appTheme === "dark" ? "light" : "dark";
    document.documentElement.dataset.appTheme = next;
  },
};

document.addEventListener("click", (event) => {
  const button = event.target.closest("[data-act]");
  if (!button || state.busy) return;
  const act = acts[button.dataset.act];
  if (act) act(button);
});

stage.innerHTML = sheet("<h1>Loading the test…</h1>", false);

Promise.all([init(), loadWordlist()])
  .then(() => {
    show("start");
  })
  .catch(() => {
    stage.innerHTML = sheet(
      "<h1>The test page could not load</h1><p class=\"lead\">From the repository, run ./relay/run and open this address again.</p>",
      false,
    );
  });
