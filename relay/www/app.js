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
  joinWords: "",
  approval: null,
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
  joinCode: "",
  accessCode: "",
};

function quiet(message) {
  const text = String(message || "");
  if (/no pages/i.test(text)) return "That code didn't work. Check it on the other phone or computer.";
  return text.replaceAll("other device", "other phone or computer").replaceAll("That device", "That phone or computer");
}

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
  const headers = {};
  if (body) headers["Content-Type"] = "application/json";
  if (body && Number.isInteger(body.revision)) headers["If-Match"] = `"${body.revision}"`;
  const response = await fetch(`${serverBase()}${path}`, {
    method,
    headers: Object.keys(headers).length ? headers : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(quiet(data.message || "Try again in a moment."));
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
  return `<div class="qr">${svg}<span>The same words</span></div>`;
}

function render() {
  title.textContent = state.screen === "start" ? "Sync test" : "Trip Budget";
  const err = state.error ? `<p class="err">${esc(state.error)}</p>` : "";
  let html = "";
  if (state.screen === "start") {
    html = sheet(
      `<p class="kicker">Private test</p>
      <h1>Turn on sync</h1>
      <p class="lead">Your pages can open on your other phones and computers, and they stay there.</p>
      <p class="lead">Start fresh if this is the first time. Join if your pages are already on another phone or computer.</p>
      <label class="lead" for="server">Test server</label>
      <input class="field" id="server" value="${esc(state.server)}" autocomplete="off" spellcheck="false">
      ${actions([
        ["fresh", "Start fresh", "primary block"],
        ["add", "Join my existing pages", "block"],
      ])}
      ${err}`,
      false,
    );
  } else if (state.screen === "access") {
    html = sheet(
      `<h1>Have an invite code?</h1>
      <p class="lead">Type the code you were given. This starts a new set of pages here. It is not the code on your other phone.</p>
      <input class="field" id="access-code" autocomplete="off" spellcheck="false" autocapitalize="characters" aria-label="Invite code" value="${esc(state.accessCode || "")}">
      ${err}
      ${actions([
        ["use-access", state.busy ? "Working…" : "Start fresh", "primary block"],
        ["start", "Back", "block"],
      ])}`,
      false,
    );
  } else if (state.screen === "save") {
    html = sheet(
      `<h1>Write down these 12 words</h1>
      <p class="lead">They are the only way back in if you lose every phone and computer. Write them on paper. You will not see them again.</p>
      <div class="key">${wordsGrid(false)}${qrBlock()}</div>
      ${actions([
        ["to-confirm", "I wrote them down", "primary block"],
        ["copy", "Copy", "grow"],
        ["print", "Print", "grow"],
        ["start", "Back", "block"],
      ])}`,
      true,
    );
  } else if (state.screen === "confirm" || state.screen === "recover-confirm") {
    html = sheet(
      `<h1>Do these match your paper?</h1>
      <p class="lead">${esc(leadAsks())}</p>
      ${asks()}
      ${state.wrong ? `<p class="err" role="alert">That's not word ${state.wrong}. Look at your paper and try again.</p>` : err}
      <div class="actions">
        <button class="btn primary block" type="button" data-act="confirm" ${state.busy || chosenPicks().length !== 3 ? "disabled" : ""}>${state.busy ? "Checking the words…" : "These match"}</button>
        <button class="btn block" type="button" data-act="back" ${state.busy ? "disabled" : ""}>${state.screen === "recover-confirm" ? "Back" : "Back to the words"}</button>
      </div>`,
      true,
    );
  } else if (state.screen === "ready") {
    html = sheet(
      `<h1>Sync is on</h1>
      <p class="lead">Your pages stay on your phones and computers. Add a phone or computer when you want them there too.</p>
      ${err}
      ${actions([
        ["invite", "Add a phone or computer", "primary block"],
        ["opened", "Done", "block"],
      ])}`,
      true,
    );
  } else if (state.screen === "opened") {
    const note = state.note ? `<p class="ok banner">${esc(state.note)}</p>` : "";
    html = `<div data-screen="opened">${editor(state.page)}${note}</div>`;
  } else if (state.screen === "invite") {
    const svg = state.qr.startsWith("<svg") && !state.qr.includes("<script") ? state.qr : "";
    html = sheet(
      `<h1>Add a phone or computer</h1>
      <p class="lead">On the other phone or computer, open Sync and tap Join my existing pages. Type this code. It works for about 5 minutes, and only once.</p>
      <div class="invite-code">${esc(state.invite ? state.invite.code : "")}</div>
      <p class="lead">Or scan this picture.</p>
      <div class="qr">${svg}</div>
      ${err}
      ${actions([["ready", "Back", "block"]])}`,
      true,
    );
  } else if (state.screen === "join-type") {
    html = sheet(
      `<h1>Join my existing pages</h1>
      <p class="lead">On a phone or computer that already has your pages, open Sync and tap Add a phone or computer. Type the code you see there.</p>
      <p class="lead">Pages already on this computer will be combined with those. Nothing is deleted.</p>
      <input class="field" id="join-code" autocomplete="off" spellcheck="false" autocapitalize="characters" aria-label="Code" value="${esc(state.joinCode || "")}">
      ${err}
      ${actions([
        ["join-go", state.busy ? "Joining…" : "Join", "primary block"],
        ["recover", "Use my recovery key instead", "block"],
        ["start", "Back", "block"],
      ])}`,
      true,
    );
  } else if (state.screen === "join") {
    html = sheet(
      `<h1>Does this code match?</h1>
      <p class="lead">On your other phone or computer, you should see the same six numbers. Tap Yes, approve it there only if they match.</p>
      <div class="code">${esc(state.code)}</div>
      <p class="waiting">${esc(state.busy ? "Opening your pages…" : "Waiting for that phone or computer")}</p>
      ${err}
      ${actions([["start", "Cancel", "block"]])}`,
      true,
    );
  } else if (state.screen === "join-confirm") {
    html = sheet(
      `<h1>Do these match?</h1>
      <p class="lead">The same six numbers and the same four words should be on your other phone or computer.</p>
      <div class="code">${esc(state.code)}</div>
      <p class="words">${esc(state.joinWords || "")}</p>
      ${err}
      ${actions([
        ["accept-join", state.busy ? "Opening your pages…" : "These match", "primary block"],
        ["start", "Cancel", "block"],
      ])}`,
      true,
    );
  } else if (state.screen === "approve") {
    const named = state.desk ? JSON.parse(sync.accountWords(state.desk)) : { ok: false, words: "" };
    html = sheet(
      `<h1>Is this your phone or computer?</h1>
      <p class="lead">It wants to open your pages. The same six numbers and the same four words should be on that screen. Tap Yes, approve it only if they match.</p>
      <div class="code">${esc(state.pending ? state.pending.code : "")}</div>
      <p class="words">${esc(named.ok ? named.words : "")}</p>
      ${err}
      ${actions([
        ["approve", state.busy ? "Approving…" : "Yes, approve it", "primary block"],
        ["decline", "No", "block"],
      ])}`,
      true,
    );
  } else if (state.screen === "recover") {
    html = sheet(
      `<h1>Use your recovery key</h1>
      <p class="lead">Type or paste the 12 words you wrote down. Numbers and capitals are fine.</p>
      <p class="lead">Pages already on this computer will be combined with those. Nothing is deleted.</p>
      <textarea class="phrase" id="phrase" spellcheck="false">${esc(state.phrase)}</textarea>
      <p class="${state.phraseOk ? "ok" : "hint"}" id="phrase-msg">${esc(state.phraseMsg)}</p>
      ${state.video ? `<video class="video" id="scan" autoplay playsinline></video>` : ""}
      ${err}
      ${actions([
        ["open-pages", state.busy ? "Checking the words…" : "Open my pages", "primary block"],
        ["scan", "Scan the paper instead", "block"],
        ["add", "Back", "block"],
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
  const joinCode = document.getElementById("join-code");
  if (joinCode) {
    joinCode.addEventListener("input", () => {
      state.joinCode = joinCode.value;
    });
    joinCode.focus();
  }
}

function leadAsks() {
  const [a, b, c] = state.asks;
  return `Pick word ${a}, word ${b}, and word ${c}.`;
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
        code: state.accessCode,
      });
      state.revision = created.revision;
      state.note = "";
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
    state.note = "Your pages are here.";
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
  show("join-type");
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
  const code = document.getElementById("join-code")?.value || state.joinCode || "";
  state.joinCode = code;
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
    state.approval = { chain: decision.chain, wrap: decision.wrap };
    const mailbox = await api("GET", `/v1/mailboxes/${state.account}`);
    const named = JSON.parse(sync.recordWords(mailbox.record));
    state.joinWords = named.ok ? named.words : "";
    state.busy = false;
    state.screen = "join-confirm";
    render();
  } catch (error) {
    state.error = error.message;
    state.busy = false;
    render();
  }
}

async function acceptJoin() {
  if (!state.approval) return;
  state.busy = true;
  render();
  try {
    const confirmed = JSON.parse(sync.confirmLink(state.desk));
    if (!confirmed.ok) {
      state.busy = false;
      state.error = confirmed.message;
      render();
      return;
    }
    const mailbox = await api("GET", `/v1/mailboxes/${state.account}`);
    const opened = JSON.parse(
      sync.openApproval(state.desk, state.approval.chain, state.approval.wrap, mailbox.record),
    );
    state.busy = false;
    if (!opened.ok) {
      state.error = opened.message;
      render();
      return;
    }
    state.page = opened.page;
    state.note = "Your pages are here.";
    show("opened");
  } catch (error) {
    state.busy = false;
    state.error = error.message;
    render();
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
    state.note = "That phone or computer can open your pages.";
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
  fresh() {
    state.server = serverBase();
    state.error = "";
    show("access");
  },
  async "use-access"() {
    state.server = serverBase();
    state.accessCode = document.getElementById("access-code")?.value.trim() || "";
    const code = state.accessCode;
    if (!code) {
      state.error = "Type the invite code you were given.";
      render();
      return;
    }
    state.busy = true;
    state.error = "";
    render();
    try {
      await api("POST", "/v1/access", { code });
      state.busy = false;
      await makeKey();
    } catch (error) {
      state.busy = false;
      state.error = error.message;
      state.screen = "access";
      render();
    }
  },
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
  "accept-join": acceptJoin,
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
