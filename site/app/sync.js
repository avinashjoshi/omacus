// Sync for the private test. The published app does not mount this.
// A device file stays in this browser. The server receives a sealed record.

const LABEL = "omacus 1.0.0-beta.2";
const INVITE_MS = 5 * 60 * 1000;

export async function mountSync(api) {
  if (!api?.syncTest) return;
  let sync;
  try {
    const mod = await import("./pkg/omacus_sync_web.js");
    await mod.default();
    sync = mod;
  } catch {
    return;
  }

  const root = document.querySelector("[data-settings-panel='settings']")?.parentElement;
  const tabs = document.querySelector(".settings-tabs");
  if (!root || !tabs) return;

  const tab = button("Sync", { type: "button", role: "tab", "data-settings-tab": "sync", "aria-selected": "false" });
  tabs.append(tab);
  const panel = el("div", { "data-settings-panel": "sync", hidden: "" });
  panel.className = "sync-panel";
  root.append(panel);

  const chip = button("Sync off", { type: "button", class: "sync-chip", "data-sync-status": "", "data-state": "off" });
  chip.addEventListener("click", () => api.openSettings("sync"));
  const beta = el("span", { class: "sync-beta" }, [LABEL]);
  const bar = document.querySelector(".bar");
  const barActions = document.querySelector(".bar-actions");
  if (bar && barActions) {
    bar.insertBefore(beta, barActions);
    bar.insertBefore(chip, barActions);
  }
  const version = document.querySelector("[data-app-version]");
  if (version) version.textContent = LABEL;

  const state = {
    screen: "off",
    desk: null,
    words: [],
    qr: "",
    asks: [],
    picks: {},
    choices: {},
    wrong: 0,
    account: "",
    revision: 0,
    code: "",
    device: "",
    requestId: "",
    pending: null,
    phrase: "",
    phraseMsg: "",
    phraseOk: false,
    error: "",
    busy: false,
    dirty: false,
    member: true,
    devices: [],
    name: matchMedia("(pointer: coarse)").matches ? "This phone" : "This browser",
    video: false,
    invite: null,
    joinCode: "",
    accessCode: "",
    localPages: false,
  };
  let wordlist = [];
  let timer = null;
  let loop = null;
  let stream = null;
  let ticking = false;

  function setChip(text, mode) {
    chip.textContent = text;
    chip.dataset.state = mode || "off";
  }

  function onPhone() {
    return matchMedia("(max-width: 760px)").matches;
  }

  function here() {
    return onPhone() ? "this phone" : "this computer";
  }

  function hereCap() {
    return onPhone() ? "This phone" : "This computer";
  }

  function plainError(message) {
    let text = String(message || "");
    if (/no pages/i.test(text)) return "That code didn't work. Check it on the other phone or computer.";
    text = text.replaceAll("other device", "other phone or computer");
    text = text.replaceAll("That device", "That phone or computer");
    text = text.replaceAll("this device", "this phone or computer");
    text = text.replaceAll("a new device", "a phone or computer");
    text = text.replaceAll("new device", "phone or computer");
    if (text === "Tap the three words first.") return "Pick the three words first.";
    return text;
  }

  async function noteLocalPages() {
    try {
      const snap = await api.snapshot();
      state.localPages = Array.isArray(snap.pages) && snap.pages.length > 0;
    } catch {
      state.localPages = false;
    }
  }

  function mergeLine() {
    if (!state.localPages) return "";
    return `Pages already on ${here()} will be combined with those. Nothing is deleted.`;
  }

  async function loadWords() {
    if (wordlist.length) return;
    const response = await fetch("./words.txt");
    const text = await response.text();
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

  function setUpCheck() {
    state.asks = shuffle([...Array(12).keys()].map((i) => i + 1)).slice(0, 3).sort((a, b) => a - b);
    state.picks = {};
    state.wrong = 0;
    state.choices = {};
    const mine = new Set(state.words.map((word) => word.toLowerCase()));
    for (const n of state.asks) {
      const options = new Set([state.words[n - 1]]);
      let guard = 0;
      while (options.size < 6 && wordlist.length && guard < 1000) {
        guard += 1;
        const word = wordlist[randomInt(wordlist.length)];
        if (!mine.has(word.toLowerCase())) options.add(word);
      }
      state.choices[n] = shuffle([...options]);
    }
  }

  function picks() {
    return state.asks.filter((n) => state.picks[n]).map((n) => [n, state.picks[n]]);
  }

  async function callServer(method, path, body, origin) {
    const url = origin && origin !== location.origin ? `${origin.replace(/\/$/, "")}${path}` : path;
    let response;
    try {
      response = await fetch(url, {
        method,
        headers: body ? { "Content-Type": "application/json" } : undefined,
        body: body ? JSON.stringify(body) : undefined,
      });
    } catch {
      const error = new Error("Offline. Your pages are still here.");
      error.offline = true;
      throw error;
    }
    const data = await response.json().catch(() => ({}));
    if (!response.ok) {
      const error = new Error(data.message || "Couldn't sync.");
      error.status = response.status;
      throw error;
    }
    return data;
  }

  async function installMerged(merged) {
    const unpacked = JSON.parse(sync.unpackPages(merged.packed));
    if (!unpacked.ok) throw new Error(unpacked.message || "Those pages could not be opened.");
    const doc = unpacked.doc ? api.b64ToBytes(unpacked.doc) : new Uint8Array();
    await api.apply(doc, unpacked.pages || []);
  }

  async function pack(doc, pages) {
    const docB64 = doc && doc.length ? api.bytesToB64(doc) : "";
    const result = JSON.parse(sync.packPages(docB64, JSON.stringify(pages)));
    if (!result.ok) throw new Error(result.message || "That copy could not be read.");
    return result.packed;
  }

  async function asSet(packed) {
    const unpacked = JSON.parse(sync.unpackPages(packed));
    if (unpacked.ok) return { packed, doc: unpacked.doc, pages: unpacked.pages };
    const bytes = api.b64ToBytes(packed);
    let text = "";
    try {
      text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    } catch {
      text = "";
    }
    const pages = text ? [["trip", text]] : [];
    return { packed: await pack(new Uint8Array(), pages), doc: "", pages };
  }

  async function saveFile() {
    const file = JSON.parse(sync.exportLocal(state.desk));
    if (!file.ok) return;
    await api.storeSet("device-file", file.file);
    await api.storeSet("sync-account", state.account);
    await api.storeSet("sync-revision", String(state.revision || 0));
  }

  async function publish(record) {
    if (!state.revision) {
      const created = await callServer("POST", "/v1/mailboxes", {
        account: state.account,
        record,
        code: state.accessCode,
      });
      state.revision = created.revision;
    } else {
      const put = await callServer("PUT", `/v1/mailboxes/${state.account}`, { revision: state.revision, record });
      state.revision = put.revision;
    }
    await saveFile();
  }

  async function pull(mailbox) {
    const opened = JSON.parse(sync.absorbRecord(state.desk, mailbox.record));
    if (!opened.ok) throw new Error(opened.message || "Those pages could not be opened.");
    state.member = opened.member !== false;
    const remote = await asSet(opened.packed);
    const snap = await api.snapshot();
    const localPacked = await pack(snap.doc, snap.pages);
    const merged = JSON.parse(sync.mergePacked(localPacked, remote.packed));
    if (!merged.ok) throw new Error(merged.message || "Those pages could not be opened.");
    await installMerged(merged);
    state.revision = mailbox.revision;
    state.dirty = merged.packed !== remote.packed;
    await saveFile();
    await refreshDevices();
  }

  async function push() {
    if (!state.member) {
      setChip("Sync off", "off");
      return;
    }
    const snap = await api.snapshot();
    const packed = await pack(snap.doc, snap.pages);
    const sealed = JSON.parse(sync.sealPages(state.desk, packed));
    if (!sealed.ok) throw new Error(sealed.message || "Couldn't sync.");
    try {
      await publish(sealed.record);
      state.dirty = false;
      setChip("Saved", "on");
    } catch (error) {
      if (error.status !== 409) throw error;
      const mailbox = await callServer("GET", `/v1/mailboxes/${state.account}`);
      await pull(mailbox);
      state.dirty = true;
    }
  }

  async function refreshDevices() {
    const list = JSON.parse(sync.deviceList(state.desk));
    state.devices = list.ok ? list.devices : [];
  }

  async function tick() {
    if (ticking || !state.desk || !state.account) return;
    if (["off", "access", "save", "confirm", "recover", "recover-confirm", "join", "join-start", "join-type", "join-scan", "ready", "joined", "approved", "off-ask", "off-done"].includes(state.screen)) return;
    ticking = true;
    try {
      if (navigator.onLine === false) setChip("Offline", "off");
      else setChip("Saving…", "on");
      const mailbox = await callServer("GET", `/v1/mailboxes/${state.account}`);
      if (mailbox.revision !== state.revision) await pull(mailbox);
      if (state.dirty && state.member) await push();
      else if (state.member) setChip("Saved", "on");
      else setChip("Sync off", "off");
      if (state.screen === "linked" || state.screen === "invite") {
        const found = await findRequest();
        if (found) {
          state.screen = "approve";
          draw();
        }
      }
    } catch (error) {
      setChip(error.offline || navigator.onLine === false ? "Offline" : "Not saved", "off");
    } finally {
      ticking = false;
    }
  }

  function schedule() {
    clearTimeout(timer);
    timer = setTimeout(() => tick(), 800);
  }

  async function findRequest() {
    if (!state.invite || Date.now() >= state.invite.expires) {
      state.pending = null;
      return false;
    }
    const data = await callServer("GET", `/v1/mailboxes/${state.account}/requests`);
    for (const request of data.requests || []) {
      const check = JSON.parse(sync.checkProof(state.invite.code, request.device, request.proof || ""));
      if (!check.ok) continue;
      const coded = JSON.parse(sync.codeFor(request.device));
      state.pending = { id: request.id, name: request.name, device: request.device, code: coded.code || "" };
      return true;
    }
    state.pending = null;
    return false;
  }

  function stopScan() {
    if (loop) cancelAnimationFrame(loop);
    loop = null;
    if (stream) stream.getTracks().forEach((track) => track.stop());
    stream = null;
    state.video = false;
  }

  async function scan() {
    state.error = "";
    if ("BarcodeDetector" in window && navigator.mediaDevices) {
      try {
        stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: "environment" } });
        state.video = true;
        draw();
        const video = panel.querySelector("video");
        if (video) video.srcObject = stream;
        const detector = new BarcodeDetector({ formats: ["qr_code"] });
        const step = async () => {
          if (!state.video || !video) return;
          try {
            const codes = await detector.detect(video);
            if (codes[0]?.rawValue) {
              stopScan();
              fillPhrase(codes[0].rawValue);
              return;
            }
          } catch {
            // Keep looking. A still photo is the other way.
          }
          loop = requestAnimationFrame(() => step());
        };
        await video?.play?.();
        step();
        return;
      } catch {
        stopScan();
      }
    }
    panel.querySelector("[data-sync-file]")?.click();
  }

  async function scanFile(file) {
    if (!file || !("BarcodeDetector" in window)) {
      state.error = "This browser can't read that picture. Type the words.";
      draw();
      return;
    }
    const bitmap = await createImageBitmap(file);
    const codes = await new BarcodeDetector({ formats: ["qr_code"] }).detect(bitmap);
    bitmap.close?.();
    if (!codes[0]?.rawValue) {
      state.error = "That picture has no recovery key.";
      draw();
      return;
    }
    fillPhrase(codes[0].rawValue);
  }

  function fillPhrase(raw) {
    if (!String(raw).startsWith("omacus-recovery")) {
      state.error = "That picture is not a recovery key.";
      draw();
      return;
    }
    state.phrase = String(raw).replace(/^omacus-recovery\s*/, "");
    state.error = "";
    onPhrase();
    draw();
  }

  function onPhrase() {
    state.phrase = panel.querySelector("textarea")?.value ?? state.phrase;
    const check = JSON.parse(sync.checkPhrase(state.phrase));
    state.phraseOk = !!check.ok;
    state.phraseMsg = state.phrase.trim() ? check.message : "";
    const msg = panel.querySelector("[data-sync-phrase]");
    if (msg) {
      msg.textContent = state.phraseMsg;
      msg.dataset.ok = state.phraseOk ? "1" : "0";
    }
  }

  async function turnOn() {
    state.busy = true;
    state.error = "";
    draw();
    await frame();
    const started = JSON.parse(sync.startNew());
    state.busy = false;
    if (!started.ok) {
      state.error = plainError(started.message);
      state.screen = "off";
      draw();
      return;
    }
    state.desk = started.id;
    state.words = started.words;
    state.qr = started.qr;
    state.screen = "save";
    draw();
  }

  async function confirmWords(recovering) {
    const chosen = picks();
    if (chosen.length !== 3) {
      state.error = "Pick the three words first.";
      draw();
      return;
    }
    const miss = chosen.find(([n, word]) => word !== state.words[n - 1]);
    if (miss) {
      state.wrong = miss[0];
      state.error = "";
      draw();
      return;
    }
    state.busy = true;
    state.error = "";
    draw();
    await frame();
    if (recovering) {
      const account = JSON.parse(sync.recoveryAccount(state.desk, JSON.stringify(chosen)));
      if (!account.ok) {
        state.busy = false;
        state.error = "Those are not the three words.";
        draw();
        return;
      }
      state.account = account.account;
      try {
        const mailbox = await callServer("GET", `/v1/mailboxes/${state.account}`);
        const opened = JSON.parse(sync.recoveryOpenPages(state.desk, mailbox.record));
        if (!opened.ok) throw new Error(opened.message);
        const remote = await asSet(opened.packed);
        const snap = await api.snapshot();
        const merged = JSON.parse(sync.mergePacked(await pack(snap.doc, snap.pages), remote.packed));
        if (!merged.ok) throw new Error(merged.message || "Those pages could not be opened.");
        await installMerged(merged);
        state.revision = mailbox.revision;
        state.member = true;
        state.screen = "joined";
        await saveFile();
        await refreshDevices();
        setChip("Saved", "on");
      } catch (error) {
        state.error = plainError(error.message);
        state.screen = "recover";
      }
      state.busy = false;
      draw();
      return;
    }
    if (!state.account) {
      const saved = JSON.parse(sync.confirmSaved(state.desk, JSON.stringify(chosen)));
      if (!saved.ok) {
        state.busy = false;
        state.error = "Those are not the three words.";
        draw();
        return;
      }
      state.account = saved.account;
    }
    try {
      const snap = await api.snapshot();
      const sealed = JSON.parse(sync.sealPages(state.desk, await pack(snap.doc, snap.pages)));
      if (!sealed.ok) throw new Error(sealed.message);
      await publish(sealed.record);
      state.member = true;
      state.screen = "ready";
      await refreshDevices();
      setChip("Saved", "on");
    } catch (error) {
      state.error = plainError(error.message);
      state.screen = "confirm";
    }
    state.busy = false;
    draw();
  }

  async function checkAccess() {
    state.accessCode = panel.querySelector("[data-access-code]")?.value ?? state.accessCode;
    const code = String(state.accessCode || "").trim();
    if (!code) {
      state.error = "Type the invite code you were given.";
      draw();
      return;
    }
    state.accessCode = code;
    state.busy = true;
    state.error = "";
    draw();
    try {
      await callServer("POST", "/v1/access", { code });
    } catch (error) {
      state.busy = false;
      state.error = plainError(error.message);
      state.screen = "access";
      draw();
      return;
    }
    state.busy = false;
    await turnOn();
  }

  function leaveFresh() {
    state.desk = null;
    state.words = [];
    state.qr = "";
    state.error = "";
    state.screen = "off";
    draw();
  }

  async function addDevice() {
    stopScan();
    state.error = "";
    state.joinCode = "";
    await noteLocalPages();
    state.screen = "join-type";
    draw();
  }

  async function openRecover() {
    stopScan();
    state.error = "";
    state.phraseMsg = "";
    state.phraseOk = false;
    await noteLocalPages();
    state.screen = "recover";
    draw();
  }

  async function turnOff() {
    state.busy = true;
    state.error = "";
    draw();
    try {
      const gone = api.storeRemove || (() => {});
      await gone("device-file");
      await gone("sync-account");
      await gone("sync-revision");
    } catch {
      state.busy = false;
      state.error = "Sync is still on. Try again.";
      draw();
      return;
    }
    stopScan();
    state.desk = null;
    state.account = "";
    state.revision = 0;
    state.member = false;
    state.devices = [];
    state.invite = null;
    state.pending = null;
    state.words = [];
    state.qr = "";
    state.requestId = "";
    state.dirty = false;
    state.screen = "off-done";
    state.busy = false;
    setChip("Sync off", "off");
    draw();
  }

  async function startInvite() {
    state.busy = true;
    state.error = "";
    draw();
    const raw = crypto.getRandomValues(new Uint8Array(5));
    const secret = btoa(String.fromCharCode(...raw));
    const made = JSON.parse(sync.makeInvite(state.account, location.origin, secret));
    if (!made.ok) {
      state.busy = false;
      state.error = plainError(made.message);
      draw();
      return;
    }
    try {
      await callServer("POST", `/v1/mailboxes/${state.account}/invites`, { hash: made.hash });
    } catch (error) {
      state.busy = false;
      state.error = plainError(error.message);
      draw();
      return;
    }
    state.invite = { code: made.code, qr: made.qr, expires: Date.now() + INVITE_MS };
    state.screen = "invite";
    state.busy = false;
    draw();
  }

  async function submitJoin(code, origin) {
    stopScan();
    state.joinCode = code || "";
    state.screen = "join-type";
    state.busy = true;
    state.error = "";
    draw();
    const started = JSON.parse(sync.startJoin());
    if (!started.ok) {
      state.busy = false;
      state.error = plainError(started.message || "That code didn't work. Check it on the other phone or computer.");
      draw();
      return;
    }
    const proof = JSON.parse(sync.joinProof(started.device, code || ""));
    if (!proof.ok) {
      state.busy = false;
      state.error = plainError(proof.message || "That code didn't work. Check it on the other phone or computer.");
      draw();
      return;
    }
    try {
      const created = await callServer("POST", "/v1/join", {
        code,
        device: started.device,
        name: state.name || "This browser",
        proof: proof.proof,
      }, origin);
      state.desk = started.id;
      state.code = started.code;
      state.device = started.device;
      state.account = created.account;
      state.requestId = created.id;
    } catch (error) {
      state.busy = false;
      state.error = plainError(error.message || "That code didn't work. Check it on the other phone or computer.");
      draw();
      return;
    }
    state.screen = "join";
    state.busy = false;
    draw();
    waitForApproval();
  }

  function parseJoin(raw) {
    const lines = String(raw || "").split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
    if (lines[0] !== "omacus-join" || lines.length < 4) return null;
    const origin = lines[1];
    const account = lines[2];
    const code = lines[3];
    if (!/^[0-9a-f]{64}$/i.test(account) || !allowedOrigin(origin)) return null;
    return { origin, account, code };
  }

  function allowedOrigin(origin) {
    try {
      const url = new URL(origin);
      if (url.origin === location.origin) return true;
      if (url.protocol === "https:" && (url.hostname === "omacus.com" || url.hostname.endsWith(".omacus.com"))) return true;
      if (url.protocol === "http:" && (url.hostname === "127.0.0.1" || url.hostname === "localhost")) return true;
    } catch {
      return false;
    }
    return false;
  }

  async function scanJoin() {
    state.error = "";
    if (!navigator.mediaDevices?.getUserMedia) {
      state.screen = "join-type";
      state.error = "This browser can't use the camera. Type the code.";
      draw();
      return;
    }
    let reader;
    try {
      reader = (await import("./jsqr.js")).default;
    } catch {
      state.screen = "join-type";
      state.error = "This browser can't read that code. Type it.";
      draw();
      return;
    }
    try {
      stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: "environment" } });
    } catch {
      stopScan();
      state.screen = "join-type";
      state.error = "The camera didn't open. Type the code.";
      draw();
      return;
    }
    state.screen = "join-scan";
    state.video = true;
    draw();
    const video = panel.querySelector("video");
    if (!video) {
      stopScan();
      return;
    }
    video.srcObject = stream;
    video.playsInline = true;
    try {
      await video.play();
    } catch {
      // A tap on the picture starts it on some phones.
    }
    const canvas = document.createElement("canvas");
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    const step = () => {
      if (state.screen !== "join-scan" || !video) return;
      if (video.readyState >= 2 && ctx) {
        canvas.width = video.videoWidth || 0;
        canvas.height = video.videoHeight || 0;
        if (canvas.width && canvas.height) {
          ctx.drawImage(video, 0, 0);
          const image = ctx.getImageData(0, 0, canvas.width, canvas.height);
          const found = reader(image.data, image.width, image.height);
          if (found?.data) {
            const parsed = parseJoin(found.data);
            stopScan();
            if (!parsed) {
              state.screen = "join-start";
              state.error = "That code didn't work. Check it on the other phone or computer.";
              draw();
              return;
            }
            submitJoin(parsed.code, parsed.origin);
            return;
          }
        }
      }
      loop = requestAnimationFrame(step);
    };
    step();
  }

  async function waitForApproval() {
    if (state.screen !== "join" || !state.requestId) return;
    try {
      const decision = await callServer("GET", `/v1/mailboxes/${state.account}/requests/${state.requestId}`);
      if (decision.state === "pending") {
        setTimeout(waitForApproval, 1000);
        return;
      }
      if (decision.state !== "approved") {
        state.error = "That phone or computer said no.";
        draw();
        return;
      }
      state.busy = true;
      draw();
      const mailbox = await callServer("GET", `/v1/mailboxes/${state.account}`);
      const opened = JSON.parse(sync.openApprovalPages(state.desk, decision.chain, decision.wrap, mailbox.record));
      if (!opened.ok) throw new Error(opened.message);
      const remote = await asSet(opened.packed);
      const snap = await api.snapshot();
      const merged = JSON.parse(sync.mergePacked(await pack(snap.doc, snap.pages), remote.packed));
      if (!merged.ok) throw new Error(merged.message || "Those pages could not be opened.");
      await installMerged(merged);
      state.revision = mailbox.revision;
      state.member = true;
      state.screen = "joined";
      await saveFile();
      await refreshDevices();
      setChip("Saved", "on");
    } catch (error) {
      state.error = plainError(error.message);
    }
    state.busy = false;
    draw();
  }

  async function approve() {
    if (!state.pending) return;
    state.busy = true;
    state.error = "";
    draw();
    const approval = JSON.parse(sync.approveDevice(state.desk, state.pending.device));
    if (!approval.ok) {
      state.busy = false;
      state.error = approval.message;
      draw();
      return;
    }
    try {
      const snap = await api.snapshot();
      const sealed = JSON.parse(sync.sealPages(state.desk, await pack(snap.doc, snap.pages)));
      if (!sealed.ok) throw new Error(sealed.message);
      const mailbox = await callServer("GET", `/v1/mailboxes/${state.account}`);
      state.revision = mailbox.revision;
      await publish(sealed.record);
      await callServer("POST", `/v1/mailboxes/${state.account}/requests/${state.pending.id}`, {
        chain: approval.chain,
        wrap: approval.wrap,
      });
      state.pending = null;
      state.invite = null;
      state.screen = "approved";
      await refreshDevices();
      setChip("Saved", "on");
    } catch (error) {
      state.error = plainError(error.message);
    }
    state.busy = false;
    draw();
  }

  async function decline() {
    if (!state.pending) {
      state.screen = "linked";
      draw();
      return;
    }
    try {
      await callServer("POST", `/v1/mailboxes/${state.account}/requests/${state.pending.id}`, { reject: true });
    } catch {
      // The request can expire. This device still has the pages.
    }
    state.pending = null;
    state.invite = null;
    state.screen = "linked";
    draw();
  }

  async function removeDevice(id) {
    const removed = JSON.parse(sync.removeDevice(state.desk, id));
    if (!removed.ok) {
      state.error = removed.message;
      draw();
      return;
    }
    state.dirty = true;
    try {
      await push();
      await refreshDevices();
    } catch (error) {
      state.error = error.message;
    }
    draw();
  }

  async function startRecovery() {
    stopScan();
    const check = JSON.parse(sync.checkPhrase(state.phrase));
    if (!check.ok) {
      state.phraseMsg = check.message;
      state.phraseOk = false;
      state.error = check.message;
      draw();
      return;
    }
    state.busy = true;
    draw();
    await frame();
    const started = JSON.parse(sync.startRecovery(state.phrase));
    state.busy = false;
    if (!started.ok) {
      state.error = started.message;
      draw();
      return;
    }
    state.desk = started.id;
    state.words = started.words;
    await loadWords().catch(() => {});
    setUpCheck();
    state.screen = "recover-confirm";
    draw();
  }

  function draw() {
    panel.replaceChildren();
    panel.append(el("p", { class: "sync-label" }, [LABEL]));
    if (state.error) panel.append(el("p", { class: "sync-err", role: "alert" }, [plainError(state.error)]));
    if (state.screen === "off") drawOff();
    else if (state.screen === "access") drawAccess();
    else if (state.screen === "save") drawSave();
    else if (state.screen === "confirm" || state.screen === "recover-confirm") drawConfirm();
    else if (state.screen === "ready") drawReady();
    else if (state.screen === "invite") drawInvite();
    else if (state.screen === "join-start" || state.screen === "join-type") drawJoinType();
    else if (state.screen === "join-scan") drawJoinScan();
    else if (state.screen === "join") drawJoin();
    else if (state.screen === "joined") drawJoined();
    else if (state.screen === "approve") drawApprove();
    else if (state.screen === "approved") drawApproved();
    else if (state.screen === "recover") drawRecover();
    else if (state.screen === "off-ask") drawOffAsk();
    else if (state.screen === "off-done") drawOffDone();
    else drawLinked();
  }

  function drawOff() {
    panel.append(
      el("h2", {}, ["Turn on sync"]),
      el("p", { class: "setting-help" }, ["Your pages can open on your other phones and computers, and they stay there."]),
      el("p", { class: "setting-help" }, ["Start fresh if this is the first time. Join if your pages are already on another phone or computer."]),
      addActions([
        [state.busy ? "Working…" : "Start fresh", "primary", () => { state.error = ""; state.screen = "access"; draw(); }, state.busy],
        ["Join my existing pages", "", () => addDevice(), state.busy],
        ["Back", "", () => api.openSettings("settings"), state.busy],
      ]),
    );
  }

  function drawAccess() {
    const field = el("input", {
      class: "sync-name",
      "aria-label": "Invite code",
      autocapitalize: "characters",
      autocomplete: "off",
      spellcheck: "false",
      "data-access-code": "",
    });
    field.value = state.accessCode || "";
    field.addEventListener("input", () => {
      state.accessCode = field.value;
    });
    panel.append(
      el("h2", {}, ["Have an invite code?"]),
      el("p", { class: "setting-help" }, ["Type the code you were given. This starts a new set of pages here. It is not the code on your other phone."]),
      field,
      addActions([
        [state.busy ? "Working…" : "Start fresh", "primary", () => checkAccess(), state.busy],
        ["Back", "", () => { state.screen = "off"; state.error = ""; draw(); }, state.busy],
      ]),
    );
    field.focus();
  }

  function drawSave() {
    const words = el("div", { class: "sync-words" });
    state.words.forEach((word, index) => {
      const n = index + 1;
      words.append(el("div", { class: "sync-word", "data-n": String(n), "data-word": word }, [
        el("b", {}, [String(n)]),
        word,
      ]));
    });
    const qr = el("div", { class: "sync-qr" });
    const picture = qrNode(state.qr, "Recovery key");
    if (picture) qr.append(picture);
    qr.append(el("span", {}, ["The same words"]));
    const row = el("div", { class: "sync-key" }, [words, qr]);
    panel.append(
      el("h2", {}, ["Write down these 12 words"]),
      el("p", { class: "setting-help" }, ["They are the only way back in if you lose every phone and computer. Write them on paper. You will not see them again."]),
      row,
      addActions([
        ["I wrote them down", "primary", async () => {
          await loadWords().catch(() => {});
          setUpCheck();
          state.screen = "confirm";
          state.error = "";
          draw();
        }],
        ["Copy", "", () => copyWords()],
        ["Print", "", () => printWords()],
        ["Back", "", () => leaveFresh()],
      ]),
    );
  }

  function drawConfirm() {
    const recovering = state.screen === "recover-confirm";
    const [a, b, c] = state.asks;
    panel.append(
      el("h2", {}, ["Do these match your paper?"]),
      el("p", { class: "setting-help" }, [`Pick word ${a}, word ${b}, and word ${c}.`]),
    );
    if (state.wrong) {
      panel.append(el("p", { class: "sync-err", role: "alert" }, [`That's not word ${state.wrong}. Look at your paper and try again.`]));
    }
    for (const n of state.asks) {
      const choices = el("div", { class: "sync-choices" });
      for (const word of state.choices[n] || []) {
        const pressed = state.picks[n] === word;
        const choice = button(word, {
          type: "button",
          class: "sync-choice",
          "data-n": String(n),
          "data-word": word,
          "aria-pressed": pressed ? "true" : "false",
        });
        choice.addEventListener("click", () => {
          state.picks[n] = word;
          state.wrong = 0;
          state.error = "";
          draw();
        });
        choices.append(choice);
      }
      const field = el("fieldset", { class: "sync-ask" });
      field.append(el("legend", {}, [`Word ${n}`]), choices);
      panel.append(field);
    }
    panel.append(actionsNode([
      [state.busy ? "Checking the words…" : "These match", "primary", () => confirmWords(recovering), state.busy || picks().length !== 3],
      [recovering ? "Back" : "Back to the words", "", () => {
        state.screen = recovering ? "recover" : "save";
        state.wrong = 0;
        state.error = "";
        draw();
      }, state.busy],
    ]));
  }

  function drawReady() {
    panel.append(
      el("h2", {}, ["Sync is on"]),
      el("p", { class: "setting-help", "data-sync-linked": "" }, ["Your pages stay on your phones and computers. Add a phone or computer when you want them there too."]),
      addActions([
        ["Add a phone or computer", "primary", () => startInvite(), state.busy],
        ["Done", "", () => { state.screen = "linked"; state.error = ""; draw(); }, state.busy],
      ]),
    );
  }

  function drawInvite() {
    const qr = el("div", { class: "sync-qr sync-join" });
    const picture = qrNode(state.invite?.qr, "Join code");
    if (picture) qr.append(picture);
    panel.append(
      el("h2", {}, ["Add a phone or computer"]),
      el("p", { class: "setting-help" }, ["On the other phone or computer, open Sync and tap Join my existing pages. Type this code. It works for about 5 minutes, and only once."]),
      el("div", { class: "sync-invite-code" }, [state.invite?.code || ""]),
      el("p", { class: "setting-help" }, ["Or scan this picture."]),
      qr,
      addActions([["Back", "", () => { state.screen = "linked"; state.error = ""; draw(); }]]),
    );
  }

  function drawJoinType() {
    const field = el("input", {
      class: "sync-name",
      "aria-label": "Code",
      autocapitalize: "characters",
      autocomplete: "off",
      spellcheck: "false",
      "data-join-code": "",
    });
    field.value = state.joinCode || "";
    field.addEventListener("input", () => {
      state.joinCode = field.value;
    });
    const bits = [
      el("h2", {}, ["Join my existing pages"]),
      el("p", { class: "setting-help" }, ["On a phone or computer that already has your pages, open Sync and tap Add a phone or computer. Type the code you see there."]),
    ];
    const merged = mergeLine();
    if (merged) bits.push(el("p", { class: "setting-help" }, [merged]));
    bits.push(field, addActions([
      [state.busy ? "Joining…" : "Join", "primary", () => submitJoin(state.joinCode), state.busy],
      ["Scan the code instead", "", () => scanJoin(), state.busy],
      ["Use my recovery key instead", "", () => openRecover(), state.busy],
      ["Back", "", () => { stopScan(); state.screen = "off"; state.error = ""; draw(); }, state.busy],
    ]));
    panel.append(...bits);
    field.focus();
  }

  function drawJoinScan() {
    panel.append(
      el("h2", {}, ["Scan the code"]),
      el("p", { class: "setting-help" }, ["Point the camera at the picture on your other phone or computer."]),
      el("video", { class: "sync-video", autoplay: "", playsinline: "", muted: "" }),
      addActions([["Type the code instead", "", () => { stopScan(); state.screen = "join-type"; state.error = ""; draw(); }]]),
    );
  }

  function drawJoin() {
    panel.append(
      el("h2", {}, ["Does this code match?"]),
      el("p", { class: "setting-help" }, ["On your other phone or computer, you should see the same six numbers. Tap Yes, approve it there only if they match."]),
      el("div", { class: "sync-code" }, [state.code || ""]),
      el("p", { class: "setting-help" }, [state.busy ? "Opening your pages…" : "Waiting for that phone or computer"]),
      addActions([["Cancel", "", () => { state.screen = "off"; state.requestId = ""; state.error = ""; draw(); }, state.busy]]),
    );
  }

  function drawJoined() {
    const bits = [
      el("h2", {}, ["Your pages are here"]),
      el("p", { class: "setting-help", "data-sync-linked": "" }, ["They stay on your phones and computers."]),
    ];
    if (state.localPages) {
      bits.push(el("p", { class: "setting-help" }, ["Pages already here were combined. Nothing was deleted."]));
    }
    bits.push(addActions([["Done", "primary", () => { state.screen = "linked"; state.error = ""; draw(); }]]));
    panel.append(...bits);
  }

  function drawApprove() {
    panel.append(
      el("h2", {}, ["Is this your phone or computer?"]),
      el("p", { class: "setting-help" }, ["It wants to open your pages. The same six numbers should be on that screen. Tap Yes, approve it only if they match."]),
      el("div", { class: "sync-code" }, [state.pending?.code || ""]),
      addActions([
        [state.busy ? "Approving…" : "Yes, approve it", "primary", () => approve(), state.busy],
        ["No", "", () => decline(), state.busy],
      ]),
    );
  }

  function drawApproved() {
    panel.append(
      el("h2", {}, ["That phone or computer can open your pages"]),
      el("p", { class: "setting-help", "data-sync-linked": "" }, ["Your pages stay on your phones and computers."]),
      addActions([["Done", "primary", () => { state.screen = "linked"; state.error = ""; draw(); }]]),
    );
  }

  function drawRecover() {
    const phrase = el("textarea", {
      class: "sync-phrase",
      spellcheck: "false",
      "aria-label": "Recovery key",
    });
    phrase.value = state.phrase;
    phrase.addEventListener("input", onPhrase);
    const file = el("input", { type: "file", accept: "image/*", hidden: "", "data-sync-file": "" });
    file.addEventListener("change", () => scanFile(file.files?.[0]));
    const bits = [
      el("h2", {}, ["Use your recovery key"]),
      el("p", { class: "setting-help" }, ["Type or paste the 12 words you wrote down. Numbers and capitals are fine."]),
    ];
    const merged = mergeLine();
    if (merged) bits.push(el("p", { class: "setting-help" }, [merged]));
    bits.push(phrase, el("p", { class: "sync-msg", "data-sync-phrase": "", "data-ok": state.phraseOk ? "1" : "0" }, [state.phraseMsg]));
    if (state.video) bits.push(el("video", { class: "sync-video", autoplay: "", playsinline: "" }));
    bits.push(file, addActions([
      [state.busy ? "Checking the words…" : "Open my pages", "primary", () => startRecovery(), state.busy],
      ["Scan the paper instead", "", () => scan(), state.busy],
      ["Back", "", () => { stopScan(); state.screen = "join-type"; draw(); }, state.busy],
    ]));
    panel.append(...bits);
    phrase.focus();
  }

  function drawOffAsk() {
    panel.append(
      el("h2", {}, [`Turn sync off on ${here()}?`]),
      el("p", { class: "setting-help" }, [`Your pages stay here. ${hereCap()} will stop sharing them. The 12 words still open them.`]),
      addActions([
        [state.busy ? "Working…" : "Turn sync off", "primary", () => turnOff(), state.busy],
        ["Not now", "", () => { state.screen = "linked"; state.error = ""; draw(); }, state.busy],
      ]),
    );
  }

  function drawOffDone() {
    panel.append(
      el("h2", {}, ["Sync is off"]),
      el("p", { class: "setting-help" }, [`Your pages are still on ${here()}.`]),
      addActions([["Done", "primary", () => { state.screen = "off"; state.error = ""; draw(); }]]),
    );
  }

  function drawLinked() {
    if (!state.member) {
      panel.append(
        el("h2", {}, [`Sync is off on ${here()}`]),
        el("p", { class: "setting-help", "data-sync-linked": "" }, ["Your pages are still here. The 12 words can open them again."]),
        addActions([
          ["Use my recovery key", "primary", () => openRecover()],
          ["Back", "", () => api.openSettings("settings")],
        ]),
      );
      return;
    }
    const home = el("div", { "data-sync-set": "" });
    const others = state.devices.some((device) => !device.self);
    home.append(
      el("h2", {}, ["Sync is on"]),
      el("p", { class: "setting-help", "data-sync-linked": "" }, ["Your pages stay on your phones and computers."]),
    );
    if (!state.devices.length) home.append(el("p", { class: "setting-help" }, [hereCap()]));
    for (const device of state.devices) {
      const row = el("div", { class: "sync-device" });
      row.append(el("div", {}, [
        el("b", {}, [device.self ? hereCap() : "Phone or computer"]),
        el("span", {}, [device.code]),
      ]));
      if (!device.self) {
        const remove = button("Remove", { type: "button", class: "btn small" });
        remove.addEventListener("click", () => removeDevice(device.id));
        row.append(remove);
      }
      home.append(row);
    }
    if (others) {
      home.append(el("p", { class: "setting-help" }, ["Remove stops that phone or computer from getting new changes. Pages already there stay there."]));
    }
    home.append(addActions([
      [state.busy ? "Working…" : "Add a phone or computer", "primary", () => startInvite(), state.busy],
      ["Turn sync off", "", () => { state.screen = "off-ask"; state.error = ""; draw(); }, state.busy],
      ["Back", "", () => api.openSettings("settings"), state.busy],
    ]));
    panel.append(home);
  }

  function addActions(rows) {
    return actionsNode(rows);
  }

  function actionsNode(rows) {
    const wrap = el("div", { class: "sync-actions" });
    for (const [label, kind, run, disabled] of rows) {
      const node = button(label, { type: "button", class: kind ? `btn ${kind}` : "btn" });
      if (disabled) node.disabled = true;
      node.addEventListener("click", run);
      wrap.append(node);
    }
    return wrap;
  }

  async function copyWords() {
    const text = state.words.map((word, index) => `${index + 1}. ${word}`).join("\n");
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      state.error = "Copy didn't work. Write the words down.";
      draw();
    }
  }

  function printWords() {
    const popup = window.open("", "_blank");
    if (!popup) return;
    const doc = popup.document;
    doc.title = "Recovery key";
    const heading = doc.createElement("h1");
    heading.textContent = "Recovery key";
    const list = doc.createElement("ol");
    for (const word of state.words) {
      const item = doc.createElement("li");
      item.textContent = word;
      list.append(item);
    }
    doc.body.append(heading, list);
    popup.print();
  }

  function frame() {
    return new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  }

  api.onEdit(() => {
    if (!state.account || !state.member) return;
    state.dirty = true;
    schedule();
  });

  const stored = api.storeGet("device-file");
  if (stored) {
    const imported = JSON.parse(sync.importLocal(stored));
    if (imported.ok) {
      state.desk = imported.id;
      state.account = imported.account;
      state.revision = Number(api.storeGet("sync-revision") || 0);
      state.screen = "linked";
      state.member = true;
      await refreshDevices().catch(() => {});
      setChip("Saved", "on");
    }
  }
  draw();
  if (state.account) tick();
  setInterval(() => tick(), 4000);
  window.addEventListener("online", () => tick());
  window.addEventListener("offline", () => {
    if (state.account) setChip("Offline", "off");
  });
  document.addEventListener("click", (event) => {
    if (event.target.closest?.("[data-settings-tab='sync']")) draw();
  });
  window.addEventListener("resize", () => {
    if (panel.hidden || state.screen === "join-scan" || state.video) return;
    draw();
  });
}

function el(tag, attrs = {}, kids = []) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === false || value == null) continue;
    if (key === "class") node.className = value;
    else if (value === true || value === "") node.setAttribute(key, "");
    else node.setAttribute(key, String(value));
  }
  for (const kid of kids) node.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  return node;
}

function button(text, attrs) {
  return el("button", attrs, [text]);
}

function qrNode(svg, label) {
  if (!svg || svg.includes("<script") || !svg.startsWith("<svg")) return null;
  const view = /viewBox="0 0 (\d+) (\d+)"/.exec(svg);
  const path = / d="([^"]*)"/.exec(svg);
  if (!view || !path) return null;
  const ns = "http://www.w3.org/2000/svg";
  const node = document.createElementNS(ns, "svg");
  node.setAttribute("viewBox", `0 0 ${view[1]} ${view[2]}`);
  node.setAttribute("role", "img");
  node.setAttribute("aria-label", label || "Recovery key");
  const d = document.createElementNS(ns, "path");
  d.setAttribute("fill", "currentColor");
  d.setAttribute("d", path[1]);
  node.append(d);
  return node;
}
