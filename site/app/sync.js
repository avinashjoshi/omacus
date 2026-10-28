// Sync. A device file stays in this browser. The server receives a sealed record.
// The engine needs those key bytes, so the file cannot be a non-exportable
// WebCrypto key. The page's strict policy is what keeps other scripts out.
import { SYNC_URL } from "./sync-config.js";
import { APP_VERSION } from "./version.js";
import { approvalCard, beginSave, dedupeWelcome, FETCH_TIMEOUT_MS, finishSave, keepUnlessTombstone, noteLocalEdit, noteTombstone, pageHeading, pagesDiffer, pollWait, PUSH_DEBOUNCE_MS, removalSentence, setMark, showTurnOnSync } from "./sync-save.mjs";
import { ifTabLeader, tabLockName, withTabLock } from "./sync-leader.mjs";
import { guessDeviceName as nameFromAgent } from "./device-name.js";
import { deviceListNote, pageOrigin, parseJoinPayload, takeSyncLink } from "./sync-link.mjs";
import { RECOVERY_WORDS } from "./recovery-words.js";
import { WORD_CHECK_ERROR, buildChoices, usableList } from "./word-check.js";
import {
  activeVault,
  bookJson,
  legacyMirror,
  loadVaultBook,
  normalizeVault,
  upsertVault,
} from "./sync-vaults.mjs";

const LABEL = `omacus v${APP_VERSION}`;
const FIELD_ERROR_ID = "sync-field-error";
const PARTIAL_NOTE = "Some changes made after a phone or computer was removed may be missing. They were locked to phones and computers that are no longer linked. Pages already here were combined with the ones these words could open. Nothing was deleted.";
const INVITE_MS = 5 * 60 * 1000;
const APPROVAL_POLL_MS = 4000;
const INVITE_POLL_MS = 2000;
const INVITE_POLL_HIDDEN_MS = 3000;
const JOIN_WAIT_MS = 2000;
const MAILBOX_TRIES = 4;
const JOIN_FINISH_MS = 30 * 1000;

function syncOrigin() {
  const configured = String(SYNC_URL || "").trim().replace(/\/$/, "");
  return configured || location.origin;
}

export async function mountSync(api) {
  if (!api) return;
  let sync;
  try {
    const mod = await import("./pkg/omacus_sync_web.js");
    await mod.default();
    sync = mod;
    if (typeof sync.appVersion === "function") {
      const reported = sync.appVersion();
      if (reported) document.documentElement.dataset.syncVersion = reported;
    }
  } catch {
    return;
  }
  if (typeof api.settle === "function") api.settle(() => settleBeforeUpdate());

  const root = document.querySelector("[data-settings-panel='settings']")?.parentElement;
  const tabs = document.querySelector(".settings-tabs");
  if (!root || !tabs) return;

  const tab = button("Sync", { type: "button", role: "tab", "data-settings-tab": "sync", "aria-selected": "false" });
  const about = tabs.querySelector("[data-settings-tab='about']");
  if (about) tabs.insertBefore(tab, about);
  else tabs.append(tab);
  const panel = el("div", { "data-settings-panel": "sync", hidden: "" });
  panel.className = "sync-panel";
  root.append(panel);

  const chip = el("button", {
    type: "button",
    class: "sync-chip",
    "data-sync-status": "",
    "data-state": "none",
    "data-ready": "0",
    hidden: "",
    "aria-label": "Set up sync",
  });
  chip.addEventListener("click", () => {
    if (deviceWaiting()) openJoin();
    else {
      if (state.removed || pausedFlag()) state.screen = "removed";
      else if (state.listNote && state.member) state.screen = "linked";
      api.openSettings("sync");
      draw();
    }
  });
  const beta = el("span", { class: "sync-beta" }, [LABEL]);
  const bar = document.querySelector(".bar");
  const barActions = document.querySelector(".bar-actions");
  if (bar && barActions) {
    bar.insertBefore(beta, barActions);
    bar.insertBefore(chip, barActions);
  }
  const state = {
    screen: "off",
    desk: null,
    words: [],
    asks: [],
    picks: {},
    choices: {},
    wrong: 0,
    account: "",
    revision: 0,
    slots: "[]",
    code: "",
    device: "",
    requestId: "",
    joinProof: "",
    joinWords: "",
    approval: null,
    pending: null,
    phrase: "",
    phraseMsg: "",
    phraseOk: false,
    error: "",
    busy: false,
    dirty: false,
    editGen: 0,
    basePages: null,
    tombstones: [],
    saveError: "",
    member: true,
    devices: [],
    name: matchMedia("(pointer: coarse)").matches ? "This phone" : "This browser",
    video: false,
    invite: null,
    joinCode: "",
    accessCode: "",
    wroteWords: false,
    localPages: false,
    removing: null,
    removed: false,
    notice: "",
    listNote: "",
    removalAsk: [],
    removalTitles: [],
    removalDeclined: [],
    seenDeviceIds: "",
    selfId: "",
    selfName: "",
    selfRev: 1,
    vaultName: "My pages",
    vaultRev: 1,
    deviceNames: [],
    membership: [],
  };
  const wordlist = usableList(RECOVERY_WORDS) || [];
  let timer = null;
  let approveWatch = 0;
  let inviteWatch = 0;
  let inviteBackoff = 0;
  let inviteLooking = false;
  let requestsChecked = 0;
  let inviteChecked = 0;
  let inviteChannel = null;
  try {
    if (typeof BroadcastChannel === "function") inviteChannel = new BroadcastChannel("omacus-sync-invite");
  } catch {
    inviteChannel = null;
  }
  let hurry = false;
  let loop = null;
  let stream = null;
  let ticking = false;
  let leading = false;

  function lockName() {
    return tabLockName(location.hostname, document.documentElement?.dataset?.syncTest);
  }

  function asLeader(fn) {
    if (leading) return fn();
    return withTabLock(navigator.locks, lockName(), fn);
  }

  let saveWanted = false;
  let saveTimer = null;
  let naming = false;

  let vaultBook = { active: "", vaults: [] };

  function currentVault() {
    return activeVault(vaultBook);
  }

  function storeText(key) {
    const value = api.storeGet(key);
    return value == null ? "" : String(value);
  }

  async function persistVaultBook() {
    await api.storeSet("sync-vaults", bookJson(vaultBook));
    const mirror = legacyMirror(currentVault());
    for (const [key, value] of Object.entries(mirror.set)) await api.storeSet(key, value);
    for (const key of mirror.remove) if (api.storeRemove) await api.storeRemove(key);
  }

  function snapshotVault(extra) {
    const prev = currentVault();
    const base = Array.isArray(state.basePages) ? JSON.stringify(state.basePages) : (prev?.base || "[]");
    return {
      account: state.account || prev?.account || "",
      deviceFile: prev?.deviceFile || "",
      revision: String(state.revision || prev?.revision || 0),
      slots: state.slots || prev?.slots || "[]",
      base,
      paused: prev?.paused === true,
      name: state.vaultName || prev?.name || "My pages",
      deviceName: state.selfName || prev?.deviceName || "",
      deviceRev: String(state.selfRev || prev?.deviceRev || 1),
      vaultRev: String(state.vaultRev || prev?.vaultRev || 1),
      selfId: state.selfId || prev?.selfId || "",
      names: JSON.stringify(state.deviceNames || []),
      membership: JSON.stringify(state.membership || []),
      tombstones: JSON.stringify(state.tombstones || []),
      ...extra,
    };
  }

  async function writeVault(patch) {
    const prev = currentVault() || normalizeVault({ account: state.account || "" });
    vaultBook = upsertVault(vaultBook, { ...prev, ...patch });
    await persistVaultBook();
  }

  function pausedFlag() {
    const vault = currentVault();
    if (vault) return vault.paused === true;
    return api.storeGet("sync-paused") === "1";
  }

  function markPaused() {
    if (currentVault() || state.account) return writeVault(snapshotVault({ paused: true }));
    if (api.storeSet) return api.storeSet("sync-paused", "1");
  }

  function clearPaused() {
    if (currentVault()) return writeVault(snapshotVault({ paused: false }));
    if (api.storeRemove) api.storeRemove("sync-paused");
  }

  function deviceWaiting() {
    return state.screen === "approve" && !!state.pending;
  }

  function freshSetup() {
    return ["intro", "access", "save", "confirm"].includes(state.screen);
  }

  function setChip(text, mode) {
    if (deviceWaiting() && mode !== "ask") {
      text = "Device waiting";
      mode = "ask";
    } else if (state.listNote && mode === "on") {
      text = "List changed";
      mode = "list";
    }
    if (text === "Sync paused" && freshSetup()) {
      hideChip();
      return;
    }
    const next = mode || "off";
    const word = text || "Offline";
    const label = next === "offline" ? "Offline. Edits saved here." : word;
    chip.hidden = false;
    if (chip.dataset.state !== next) {
      chip.replaceChildren(cloudIcon(next), el("span", { class: "sync-chip-word" }, [word]));
    } else {
      const span = chip.querySelector(".sync-chip-word");
      if (span && span.textContent !== word) span.textContent = word;
    }
    chip.dataset.state = next;
    chip.setAttribute("aria-label", label);
    if (next === "offline") chip.title = "Edits saved here.";
    else if (next === "list") chip.title = state.listNote;
    else chip.removeAttribute("title");
    paintEntry();
  }

  function hideChip() {
    chip.hidden = true;
    chip.dataset.state = "none";
    chip.replaceChildren();
    chip.setAttribute("aria-label", "Set up sync");
    chip.removeAttribute("title");
    paintEntry();
  }

  function paintEntry() {
    const show = showTurnOnSync({
      account: state.account,
      removed: state.removed,
      paused: pausedFlag(),
    });
    const linked = !show && !!(state.account || state.removed || pausedFlag());
    const label = linked ? "Sync" : "Turn on sync";
    for (const node of document.querySelectorAll("[data-turn-on-sync]")) {
      node.hidden = !(show || linked);
      node.setAttribute("aria-label", label);
      if (linked && (state.removed || pausedFlag())) node.title = "Sync is paused. Open it to join again.";
      else node.title = label;
      const span = node.querySelector("span");
      if (span) span.textContent = label;
      else if (node.classList.contains("sync-setup")) node.textContent = label;
    }
    const ask = deviceWaiting();
    for (const node of document.querySelectorAll("[data-device-waiting]")) node.hidden = !ask;
  }

  function refreshCue() {
    paintEntry();
    if (deviceWaiting()) {
      if (chip.dataset.state !== "ask") setChip("Device waiting", "ask");
      return;
    }
    const settle = chip.dataset.state === "ask" || chip.dataset.state === "list"
      || (state.listNote && chip.dataset.state === "on");
    if (!settle) return;
    if (navigator.onLine === false && state.account) setChip("Offline", "offline");
    else if (state.removed || pausedFlag()) setChip("Sync paused", "off");
    else if (state.saveError) setChip("Not saved", "error");
    else if (state.listNote && state.member && state.account) setChip("List changed", "list");
    else if (state.member && state.account) paintConnection();
    else hideChip();
  }

  function openJoin() {
    api.openSettings("sync");
    draw();
  }
  globalThis.omacusShowJoin = openJoin;

  function openIntro() {
    if (state.removed || pausedFlag()) {
      state.screen = "removed";
      state.error = "";
    } else if (!state.account) {
      state.screen = "intro";
      state.error = "";
    }
    api.openSettings("sync");
    draw();
  }
  globalThis.omacusTurnOnSync = openIntro;

  function paintConnection() {
    if (!state.account || !state.member) {
      if (state.removed || pausedFlag()) setChip("Sync paused", "off");
      else hideChip();
      return;
    }
    if (deviceWaiting()) {
      setChip("Device waiting", "ask");
      return;
    }
    if (state.saveError) {
      setChip("Not saved", "error");
      return;
    }
    if (state.listNote) {
      setChip("List changed", "list");
      return;
    }
    if (navigator.onLine === false) {
      setChip("Offline", "offline");
      return;
    }
    // Syncing is a save that has not landed, or the quick connection is down
    // while ordinary requests still reach the pages. The approve button's own
    // wait is not a save, or Yes would leave the status on Syncing.
    if (saveWanted) {
      setChip("Syncing", "busy");
      return;
    }
    if (state.live) setChip("Live", "on");
    else setChip("Syncing", "busy");
  }

  function calm() {
    saveWanted = false;
    clearTimeout(saveTimer);
    saveTimer = null;
    if (state.member && state.account) {
      clearPaused();
      state.saveError = "";
      paintConnection();
    } else if (state.removed || pausedFlag()) {
      setChip("Sync paused", "off");
    } else {
      hideChip();
    }
  }

  function dropSaving() {
    saveWanted = false;
    clearTimeout(saveTimer);
    saveTimer = null;
  }

  function armSaving() {
    if (!state.account || !state.member || saveWanted) return;
    if (navigator.onLine === false) {
      setChip("Offline", "offline");
      return;
    }
    saveWanted = true;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      if (!saveWanted || !state.account || !state.member) return;
      if (navigator.onLine === false) {
        saveWanted = false;
        setChip("Offline", "offline");
        return;
      }
      setChip("Syncing", "wait");
    }, 1000);
  }

  function guessDeviceName() {
    return nameFromAgent(navigator.userAgent || "");
  }

  function cleanName(value) {
    return String(value || "").replace(/\s+/g, " ").trim().slice(0, 40);
  }

  function loadNames() {
    const vault = currentVault();
    state.selfName = cleanName(vault?.deviceName || api.storeGet("sync-device-name")) || guessDeviceName();
    state.selfRev = Number(vault?.deviceRev || api.storeGet("sync-device-rev") || 1) || 1;
    state.vaultName = cleanName(vault?.name || api.storeGet("sync-vault-name")) || "My pages";
    state.vaultRev = Number(vault?.vaultRev || api.storeGet("sync-vault-rev") || 1) || 1;
    state.selfId = vault?.selfId || api.storeGet("sync-self-id") || "";
    state.deviceNames = readJson(vault?.names || api.storeGet("sync-names"), []);
    state.membership = readJson(vault?.membership || api.storeGet("sync-membership"), []);
    state.tombstones = readJson(vault?.tombstones || api.storeGet("sync-tombstones"), []);
    state.removalDeclined = readJson(api.storeGet("sync-removal-declined"), []);
    state.seenDeviceIds = String(api.storeGet("sync-device-ids") || "");
  }

  function rememberDevices(ids) {
    const next = [...new Set(ids.map((id) => String(id || "").trim()).filter(Boolean))].sort().join(",");
    state.seenDeviceIds = next;
    state.listNote = "";
    api.storeSet("sync-device-ids", next);
  }

  // This phone just wrote the list, so it already knows. The other phones
  // still see the change the next time they read it.
  function acceptListed() {
    rememberDevices(state.devices.map((device) => device.id));
    refreshCue();
  }

  function readJson(text, fallback) {
    if (!text) return fallback;
    try {
      const value = JSON.parse(text);
      return Array.isArray(value) ? value : fallback;
    } catch {
      return fallback;
    }
  }

  function saveNames() {
    if (api.storeSet) api.storeSet("sync-removal-declined", JSON.stringify(state.removalDeclined || []));
    if (currentVault() || state.account) {
      return writeVault(snapshotVault({
        deviceName: state.selfName,
        deviceRev: String(state.selfRev || 1),
        name: state.vaultName || "My pages",
        vaultRev: String(state.vaultRev || 1),
        selfId: state.selfId || "",
        names: JSON.stringify(state.deviceNames || []),
        membership: JSON.stringify(state.membership || []),
      }));
    }
    api.storeSet("sync-device-name", state.selfName);
    api.storeSet("sync-device-rev", String(state.selfRev || 1));
    api.storeSet("sync-vault-name", state.vaultName);
    api.storeSet("sync-vault-rev", String(state.vaultRev || 1));
    api.storeSet("sync-self-id", state.selfId || "");
    api.storeSet("sync-names", JSON.stringify(state.deviceNames));
    api.storeSet("sync-membership", JSON.stringify(state.membership));
    api.storeSet("sync-tombstones", JSON.stringify(state.tombstones || []));
    api.storeSet("sync-removal-declined", JSON.stringify(state.removalDeclined || []));
  }

  function rememberSelf() {
    if (!state.desk) return;
    const list = JSON.parse(sync.deviceList(state.desk));
    if (!list.ok) return;
    const mine = (list.devices || []).find((device) => device.self);
    if (mine?.id) state.selfId = mine.id;
  }

  function metaForPack() {
    rememberSelf();
    const devices = state.deviceNames.filter((row) => row.id && row.id !== state.selfId && row.name);
    if (state.selfId && state.selfName) {
      devices.push({ id: state.selfId, rev: state.selfRev || 1, name: state.selfName });
    }
    return {
      vaultName: state.vaultName || "My pages",
      nameRev: state.vaultRev || 1,
      devices,
      membership: state.membership.filter((row) => row.id),
      tombstones: (state.tombstones || []).filter((row) => row.id),
    };
  }

  function localIds() {
    return state.membership.filter((row) => row && row.inVault === false).map((row) => row.id);
  }

  function showPlaces() {
    const on = !!(state.account && state.member && !state.removed);
    api.setSyncPlaces?.(on ? {
      vaultName: state.vaultName || "My pages",
      localIds: localIds(),
      here: onPhone() ? "On this phone" : "On this computer",
      move: (ids, inVault) => movePages(ids, inVault),
    } : null);
  }

  function movePages(ids, inVault) {
    const list = [...new Set(Array.isArray(ids) ? ids : [ids])].filter(Boolean);
    if (!list.length) return;
    const next = !!inVault;
    let changed = false;
    for (const id of list) {
      const prior = state.membership.find((row) => row.id === id);
      if (prior && prior.inVault === next) continue;
      const rev = (prior?.rev || 0) + 1;
      state.membership = state.membership.filter((row) => row.id !== id);
      state.membership.push({ id, rev, inVault: next });
      changed = true;
    }
    if (!changed) return;
    saveNames();
    showPlaces();
    if (state.member && state.account) {
      noteLocalEdit(state);
      armSaving();
      schedule();
    }
  }

  function renameDevice(id, name) {
    const next = cleanName(name);
    if (!id || !next) return;
    if (id === state.selfId) {
      if (next === state.selfName) return;
      state.selfName = next;
      state.selfRev += 1;
    } else {
      const row = state.deviceNames.find((item) => item.id === id);
      if (row && row.name === next) return;
      const rev = (row?.rev || 0) + 1;
      if (!row) state.deviceNames.push({ id, rev, name: next });
      else {
        row.rev = rev;
        row.name = next;
      }
      const shown = state.devices.find((item) => item.id === id);
      if (shown) shown.name = next;
    }
    saveNames();
    if (state.member && state.account) {
      noteLocalEdit(state);
      armSaving();
      schedule();
    }
    draw();
  }

  function adoptMeta(meta) {
    if (!meta || typeof meta !== "object") return;
    const remoteName = cleanName(meta.vaultName) || "";
    const remoteRev = Number(meta.nameRev || 0);
    if (remoteName && (remoteRev > state.vaultRev || (remoteRev === state.vaultRev && remoteName > state.vaultName))) {
      state.vaultName = remoteName;
      state.vaultRev = remoteRev;
    }
    for (const device of meta.devices || []) {
      const name = cleanName(device.name);
      const rev = Number(device.rev || 0);
      if (!device.id || !name) continue;
      if (device.id === state.selfId) {
        if (rev > state.selfRev) {
          state.selfName = name;
          state.selfRev = rev;
        }
        continue;
      }
      const row = state.deviceNames.find((item) => item.id === device.id);
      if (!row) state.deviceNames.push({ id: device.id, rev, name });
      else if (rev > row.rev || (rev === row.rev && name > row.name)) {
        row.rev = rev;
        row.name = name;
      }
    }
    for (const member of meta.membership || []) {
      if (!member.id) continue;
      const rev = Number(member.rev || 0);
      const inVault = member.inVault !== false;
      const row = state.membership.find((item) => item.id === member.id);
      if (!row) state.membership.push({ id: member.id, rev, inVault });
      else if (rev > row.rev || (rev === row.rev && inVault && !row.inVault)) {
        row.rev = rev;
        row.inVault = inVault;
      }
    }
    saveNames();
    showPlaces();
  }

  function deviceTitle(device) {
    if (device.self) return state.selfName || hereCap();
    const named = state.deviceNames.find((row) => row.id === device.id)?.name;
    return named || device.name || "Phone or computer";
  }

  function beginRename(row, device) {
    const current = deviceTitle(device);
    naming = true;
    const field = el("input", {
      class: "sync-name",
      "aria-label": "Name",
      maxlength: "40",
      autocomplete: "off",
      spellcheck: "false",
    });
    field.value = current;
    const save = button("Save", { type: "button", class: "btn small primary" });
    const cancel = button("Cancel", { type: "button", class: "btn small" });
    const finish = (commit) => {
      naming = false;
      const name = cleanName(field.value);
      if (commit && name && name !== current) renameDevice(device.id, name);
      draw();
    };
    save.addEventListener("click", () => finish(true));
    cancel.addEventListener("click", () => finish(false));
    field.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        finish(true);
      } else if (event.key === "Escape") {
        event.preventDefault();
        finish(false);
      }
    });
    row.replaceChildren(el("div", { class: "sync-rename" }, [
      el("label", { class: "sync-field" }, ["Name"]),
      field,
      el("div", { class: "sync-rename-actions" }, [save, cancel]),
    ]));
    field.focus();
    field.select();
  }

  function nameField(label, value, onValue) {
    const field = el("input", {
      class: "sync-name",
      "aria-label": label,
      maxlength: "40",
      autocomplete: "off",
      spellcheck: "false",
    });
    field.value = value || "";
    const commit = () => onValue(cleanName(field.value));
    field.addEventListener("focus", () => {
      naming = true;
    });
    field.addEventListener("blur", () => {
      naming = false;
      commit();
    });
    field.addEventListener("change", commit);
    field.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        field.blur();
      }
    });
    return field;
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
    text = text.replaceAll("other device", "other phone or computer");
    text = text.replaceAll("That device", "That phone or computer");
    text = text.replaceAll("This device", "This phone or computer");
    text = text.replaceAll("this device", "this phone or computer");
    text = text.replaceAll("a new device", "a phone or computer");
    text = text.replaceAll("new device", "phone or computer");
    if (text === "Tap the three words first.") return "Pick the three words first.";
    return text;
  }

  function joinError(message) {
    const text = String(message || "");
    if (/no pages/i.test(text)) return "That code didn't work. Check it on the other phone or computer.";
    return plainError(text);
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
    const built = buildChoices(state.words, state.asks, wordlist, randomInt);
    if (!built) return false;
    for (const n of state.asks) state.choices[n] = shuffle(built[n]);
    return true;
  }

  function picks() {
    return state.asks.filter((n) => state.picks[n]).map((n) => [n, state.picks[n]]);
  }

  async function callServer(method, path, body, origin, authMode, extra) {
    const base = (origin || syncOrigin()).replace(/\/$/, "");
    const url = `${base}${path}`;
    const text = body ? JSON.stringify(body) : "";
    const headers = {};
    if (body) headers["Content-Type"] = "application/json";
    if (authMode === "open") {
      // A new set of pages, and a phone that is still joining, use their own checks.
    } else if (authMode && authMode.proof) {
      headers["X-Omacus-Proof"] = authMode.proof;
    } else if (state.desk) {
      const signed = JSON.parse(sync.signRequest(state.desk, method, path, text));
      if (!signed.ok) throw new Error(signed.message || "Couldn't sync.");
      headers["X-Omacus-Device"] = signed.device;
      headers["X-Omacus-Time"] = signed.time;
      headers["X-Omacus-Nonce"] = signed.nonce;
      headers["X-Omacus-Signature"] = signed.signature;
    }
    if (extra) Object.assign(headers, extra);
    let response;
    try {
      response = await fetch(url, {
        method,
        headers,
        body: body ? text : undefined,
        cache: "no-store",
        signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
      });
    } catch (error) {
      const timed = error && (error.name === "TimeoutError" || error.name === "AbortError");
      const wrapped = new Error(timed
        ? "That took too long. Your pages are still here."
        : "Offline. Your pages are still here.");
      wrapped.offline = !timed;
      throw wrapped;
    }
    if (response.status === 304) return { unchanged: true };
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
    if (unpacked.ok) adoptMeta(unpacked.meta);
    if (!unpacked.ok) throw new Error(unpacked.message || "Those pages could not be opened.");
    const doc = unpacked.doc ? api.b64ToBytes(unpacked.doc) : new Uint8Array();
    await api.apply(doc, unpacked.pages || []);
  }

  async function pack(doc, pages) {
    const docB64 = doc && doc.length ? api.bytesToB64(doc) : "";
    const result = JSON.parse(sync.packPages(docB64, JSON.stringify(pages), JSON.stringify(metaForPack())));
    if (!result.ok) throw new Error(result.message || "That copy could not be read.");
    return result.packed;
  }

  async function asSet(packed) {
    const unpacked = JSON.parse(sync.unpackPages(packed));
    if (unpacked.ok) {
      adoptMeta(unpacked.meta);
      return { packed, doc: unpacked.doc, pages: unpacked.pages };
    }
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

  async function rememberBase(pages) {
    state.basePages = (pages || []).map((row) => [row[0], String(row[1] ?? "")]);
    await writeVault(snapshotVault({ base: JSON.stringify(state.basePages) }));
  }

  async function saveFile() {
    const file = JSON.parse(sync.exportLocal(state.desk));
    if (!file.ok) return;
    await writeVault(snapshotVault({
      account: state.account,
      deviceFile: file.file,
      revision: String(state.revision || 0),
      slots: state.slots || "[]",
    }));
  }

  async function publish(record, objects, dropIds) {
    const extra = {};
    if (objects && objects.length) extra.objects = objects;
    if (dropIds && dropIds.length) extra.drop = dropIds;
    const result = !state.revision
      ? await callServer("POST", "/v1/mailboxes", {
        account: state.account,
        record,
        code: state.accessCode,
        ...extra,
      }, undefined, "open")
      : await callServer("PUT", `/v1/mailboxes/${state.account}`, {
        revision: state.revision,
        record,
        ...extra,
      }, undefined, undefined, { "If-Match": `"${state.revision}"` });
    state.revision = result.revision;
    await saveFile();
    return result;
  }

  async function fetchObjects(ids) {
    if (!ids || !ids.length) return [];
    const data = await callServer("POST", `/v1/mailboxes/${state.account}/objects`, { ids });
    return data.objects || [];
  }

  function notedGeneration() {
    try {
      const prior = JSON.parse(sync.generation(state.desk));
      return prior.ok ? Number(prior.generation) || 0 : 0;
    } catch {
      return 0;
    }
  }

  function rewindSeal() {
    if (state.sealGen == null) return;
    try {
      sync.rewindGeneration(state.desk, state.sealGen);
    } catch {
      /* the next open still has the pages saved here */
    }
    state.sealGen = null;
  }

  async function sealPacked(packed) {
    state.sealGen = notedGeneration();
    const sealed = JSON.parse(sync.sealLibrary(state.desk, packed, state.slots || "[]"));
    if (!sealed.ok) {
      rewindSeal();
      throw new Error(sealed.message || "Couldn't sync.");
    }
    return sealed;
  }

  async function commitSeal(sealed) {
    let result;
    try {
      result = await publish(sealed.record, sealed.objects, sealed.drop);
    } catch (error) {
      rewindSeal();
      throw error;
    }
    state.sealGen = null;
    const slots = JSON.parse(sealed.slots || "[]");
    for (const row of result.objects || []) {
      const slot = slots.find((item) => item.objectId === row.id);
      if (slot) slot.serverRev = row.revision;
    }
    state.slots = JSON.stringify(slots);
    await writeVault(snapshotVault({ slots: state.slots, revision: String(state.revision || 0) }));
    return result;
  }

  function pageInVault(id) {
    const row = (state.membership || []).find((item) => item.id === id);
    if (!row) return true;
    return row.inVault !== false;
  }

  function adoptTombstone(tomb) {
    const id = String(tomb?.id || "");
    if (!id) return;
    const rev = Number(tomb.rev) || 0;
    state.tombstones = state.tombstones || [];
    const row = state.tombstones.find((item) => item.id === id);
    if (row) {
      if (rev > (Number(row.rev) || 0)) row.rev = rev;
      return;
    }
    state.tombstones.push({ id, rev: rev || 1 });
  }

  function titlesFor(asked, pages) {
    const out = [];
    for (const tomb of asked || []) {
      const row = (pages || []).find((page) => page && page[0] === tomb.id);
      const title = pageHeading(row ? row[1] : "");
      if (title && title !== "Untitled") out.push(title);
    }
    return out;
  }

  async function combineRemote(snap, remote) {
    const localPacked = await pack(snap.doc, snap.pages);
    const merged = JSON.parse(sync.mergePacked(localPacked, remote.packed));
    if (!merged.ok) throw new Error(merged.message || "Those pages could not be opened.");
    const openedPacked = JSON.parse(sync.unpackPages(merged.packed));
    if (!openedPacked.ok) throw new Error(openedPacked.message || "Those pages could not be opened.");
    const decision = keepUnlessTombstone(
      snap.pages,
      openedPacked.pages || [],
      openedPacked.meta?.tombstones || [],
      state.removalDeclined,
    );
    const wasHeld = decision.held === true;
    let pages = decision.pages;
    let welcomeRemoved = [];
    const folded = dedupeWelcome(pages, openedPacked.meta?.tombstones || []);
    if (folded.removed.length > 2) {
      decision.held = true;
      for (const id of folded.removed) {
        const tomb = folded.tombstones.find((row) => row.id === id);
        if (!decision.asked.some((row) => row.id === id)) {
          decision.asked.push({ id, rev: Number(tomb?.rev) || 0 });
        }
      }
    } else if (folded.removed.length) {
      pages = folded.pages;
      welcomeRemoved = folded.removed;
    }
    if (decision.held) {
      state.removalAsk = decision.asked;
      state.removalTitles = titlesFor(decision.asked, (snap.pages || []).concat(pages));
      for (const tomb of decision.asked) adoptTombstone(tomb);
    } else {
      state.removalAsk = [];
      state.removalTitles = [];
    }
    if (!wasHeld) {
      for (const id of decision.removed) {
        if (typeof api.forget === "function") await api.forget(id);
        state.tombstones = noteTombstone(state.tombstones, id);
      }
    }
    for (const id of welcomeRemoved) {
      if (decision.removed.includes(id)) continue;
      if (typeof api.forget === "function") await api.forget(id);
      state.tombstones = noteTombstone(state.tombstones, id);
    }
    saveNames();
    const doc = openedPacked.doc ? api.b64ToBytes(openedPacked.doc) : new Uint8Array();
    const packed = await pack(doc, pages);
    const remoteIds = new Set((remote.pages || []).map((row) => row && row[0]).filter(Boolean));
    return { packed, pages, welcomeRemoved, remoteIds };
  }

  function confirmRemoval() {
    const asked = state.removalAsk || [];
    state.removalAsk = [];
    state.removalTitles = [];
    for (const tomb of asked) {
      if (typeof api.forget === "function") api.forget(tomb.id);
      state.tombstones = noteTombstone(state.tombstones, tomb.id);
    }
    saveNames();
    state.dirty = true;
    armSaving();
    schedule();
    paintRemoval();
  }

  function declineRemoval() {
    for (const tomb of state.removalAsk || []) {
      const have = (state.removalDeclined || []).find((row) => row.id === tomb.id);
      if (have) have.rev = Math.max(Number(have.rev) || 0, Number(tomb.rev) || 0);
      else state.removalDeclined.push({ id: tomb.id, rev: Number(tomb.rev) || 0 });
      state.tombstones = (state.tombstones || []).filter((row) => row.id !== tomb.id);
    }
    state.removalAsk = [];
    state.removalTitles = [];
    saveNames();
    state.dirty = true;
    armSaving();
    schedule();
    paintRemoval();
  }

  function paintRemoval() {
    const existing = document.querySelector("[data-removal-ask]");
    const asked = state.removalAsk || [];
    if (asked.length <= 2) {
      existing?.remove();
      return;
    }
    const key = `${asked.length}:${(state.removalTitles || []).join("\n")}`;
    if (existing && existing.dataset.key === key) return;
    existing?.remove();
    const card = el("div", { class: "removal-card" }, [
      el("h2", { id: "removal-ask-title" }, ["Remove these pages?"]),
      el("p", {}, [removalSentence(asked.length)]),
    ]);
    if (state.removalTitles?.length) {
      const list = el("ul", { class: "removal-pages" });
      for (const title of state.removalTitles) list.append(el("li", {}, [title]));
      card.append(list);
    }
    const keep = button("Keep them", { type: "button", class: "btn primary" });
    const remove = button("Remove them", { type: "button", class: "btn danger" });
    keep.addEventListener("click", () => declineRemoval());
    remove.addEventListener("click", () => confirmRemoval());
    const actions = el("div", { class: "removal-actions" }, [keep, remove]);
    card.append(actions);
    const scrim = button("", { type: "button", class: "removal-scrim", "aria-label": "Keep them" });
    scrim.addEventListener("click", () => declineRemoval());
    const dialog = el("div", {
      class: "removal-ask",
      "data-removal-ask": "",
      role: "dialog",
      "aria-modal": "true",
      "aria-labelledby": "removal-ask-title",
    }, [scrim, card]);
    dialog.dataset.key = key;
    dialog.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        event.preventDefault();
        declineRemoval();
      }
    });
    document.body.append(dialog);
    keep.focus();
  }

  async function pull(mailbox, depth = 0) {
    const wanted = JSON.parse(sync.wantedObjects(state.desk, mailbox.record, state.slots || "[]"));
    if (!wanted.ok) throw new Error(wanted.message || "Those pages could not be opened.");
    const fetched = await fetchObjects(wanted.want || []);
    let docB64 = "";
    let snap = { doc: new Uint8Array(), pages: [] };
    let snapOk = false;
    try {
      const taken = await api.snapshot();
      if (taken) {
        snap = taken;
        snapOk = true;
        if (snap.doc && snap.doc.length) docB64 = api.bytesToB64(snap.doc);
      }
    } catch {
      docB64 = "";
    }
    const opened = JSON.parse(sync.openLibrary(state.desk, mailbox.record, JSON.stringify(fetched), state.slots || "[]", docB64));
    if (!opened.ok) {
      // Publishing the list this phone already has would replace a longer
      // recovery-signed list, and the server refuses that copy.
      throw new Error(opened.message || "Those pages could not be opened.");
    }
    const dropped = opened.member === false && opened.replace === false;
    if (opened.member === true) state.member = true;
    else if (dropped) state.member = false;
    state.revision = mailbox.revision;
    if (opened.replace === false) {
      await saveFile();
      await refreshDevices();
      if (dropped || state.removed) {
        showRemoved();
        return;
      }
      if (opened.republish && state.member && depth < 1) await push();
      return;
    }
    const remote = await asSet(opened.packed);
    // The pages on this phone have to be visible. Replacing them with the
    // remote list is how a page disappeared.
    if (!snapOk) {
      await saveFile();
      await refreshDevices();
      return;
    }
    const localChanged = pagesDiffer(state.basePages, snap.pages) || opened.combine === true;
    const joined = await combineRemote(snap, remote);
    await installMerged({ packed: joined.packed });
    const upload = (snap.pages || []).some((row) => {
      const id = row && row[0];
      return id && !joined.remoteIds.has(id) && joined.pages.some((page) => page[0] === id) && pageInVault(id);
    });
    state.dirty = upload || joined.welcomeRemoved.length > 0 || localChanged || opened.republish === true;
    if (!state.dirty) await rememberBase(joined.pages);
    paintRemoval();
    await saveFile();
    await refreshDevices();
    if (dropped || state.removed) {
      showRemoved();
      return;
    }
    if (opened.slots) state.slots = opened.slots;
    if ((opened.republish || opened.legacy) && opened.replace !== false && state.member && depth < 1) await push();
  }

  async function settleBeforeUpdate() {
    const deadline = Date.now() + 8000;
    while (ticking && Date.now() < deadline) await sleep(40);
    if (!state.account || !state.member || !state.dirty) return;
    if (navigator.onLine === false) return;
    try {
      await push();
    } catch {
      // Pages stay in this browser. Reloading does not remove them.
    }
  }

  async function push() {
    if (!state.member) {
      if (state.removed || pausedFlag()) setChip("Sync paused", "off");
      else hideChip();
      return;
    }
    const started = beginSave(state);
    const snap = await api.snapshot();
    const packed = await pack(snap.doc, snap.pages);
    const sealed = await sealPacked(packed);
    try {
      await commitSeal(sealed);
      await rememberBase(snap.pages);
      finishSave(state, started);
      state.saveError = "";
      if (!state.dirty) calm();
    } catch (error) {
      if (error.status !== 409) throw error;
      const mailbox = await callServer("GET", `/v1/mailboxes/${state.account}`);
      await pull(mailbox, 1);
      state.dirty = true;
    }
  }

  async function publishListedName() {
    rememberSelf();
    if (!state.selfId) return;
    const started = beginSave(state);
    const snap = await api.snapshot();
    const packed = await pack(snap.doc, snap.pages);
    const sealed = await sealPacked(packed);
    await commitSeal(sealed);
    await rememberBase(snap.pages || []);
    finishSave(state, started);
    if (!state.dirty) calm();
  }

  async function refreshDevices() {
    rememberSelf();
    const list = JSON.parse(sync.deviceList(state.desk));
    const rows = list.ok ? list.devices : [];
    state.devices = rows.map((device) => ({
      ...device,
      name: device.self
        ? state.selfName
        : (state.deviceNames.find((row) => row.id === device.id)?.name || ""),
    }));
    const listed = JSON.stringify(state.devices.map((device) => [device.id, device.name, device.code, device.self]))
      + JSON.stringify(state.membership);
    const changed = listed !== state.listed;
    state.listed = listed;
    if (list.ok) {
      const ids = state.devices.map((device) => device.id).filter(Boolean);
      if (!state.seenDeviceIds) {
        if (ids.length) rememberDevices(ids);
      } else {
        state.listNote = deviceListNote(state.seenDeviceIds, ids);
      }
    }
    if (!list.ok) {
      showPlaces();
      return;
    }
    const onList = state.devices.some((device) => device.self);
    if (onList) {
      state.member = true;
      state.removed = false;
    } else if (list.former) {
      state.member = false;
      state.removed = true;
    } else {
      state.member = false;
      state.removed = false;
    }
    showPlaces();
    refreshCue();
    if (changed && state.screen === "linked" && !panel.hidden) draw();
  }

  function showRemoved() {
    state.member = false;
    state.removed = true;
    showPlaces();
    const stay = ["off", "access", "save", "confirm", "recover", "join", "join-start", "join-type", "join-scan", "approve", "remove-ask", "off-ask", "removed"];
    if (!stay.includes(state.screen)) state.screen = "removed";
    markPaused();
    setChip("Sync paused", "off");
    if (state.screen === "removed") draw();
  }

  async function tick() {
    if (ticking || !state.desk || !state.account) return;
    if (state.screen === "approve") {
      ticking = true;
      try { await watchApproval(); }
      finally { ticking = false; }
      return;
    }
    if (["off", "access", "save", "confirm", "recover", "join", "join-start", "join-type", "join-scan", "remove-ask", "off-ask", "removed"].includes(state.screen)) return;
    ticking = true;
    try {
      await ifTabLeader(navigator.locks, lockName(), () => follow());
    } finally {
      ticking = false;
    }
  }

  async function noticeJoin() {
    if (state.screen !== "linked" && state.screen !== "invite" && state.screen !== "ready") return;
    const found = await findRequest();
    if (found) {
      state.screen = "approve";
      draw();
    }
  }

  async function follow() {
    leading = true;
    try {
      if (navigator.onLine === false) {
        saveWanted = false;
        clearTimeout(saveTimer);
        setChip("Offline", "offline");
      }
      const mailbox = await readMailbox();
      if (mailbox && mailbox.revision !== state.revision) {
        armSaving();
        await pull(mailbox);
      }
      if (state.member && navigator.onLine !== false) {
        const snap = await api.snapshot().catch(() => null);
        if (snap && pagesDiffer(state.basePages, snap.pages)) state.dirty = true;
      }
      if (state.dirty && state.member) {
        armSaving();
        await push();
      } else if (state.member && navigator.onLine !== false) calm();
      else if (state.removed || pausedFlag()) setChip("Sync paused", "off");
      else if (navigator.onLine !== false) hideChip();
      await noticeJoin();
    } catch (error) {
      saveWanted = false;
      clearTimeout(saveTimer);
      saveTimer = null;
      if (error.offline || navigator.onLine === false) setChip("Offline", "offline");
      else if (error.status === 429 && /sync is busy/i.test(error.message || "")) {
        schedule();
      } else if (/that code didn't work/i.test(error.message || "")) {
        // The phone that typed the code hears about it. This phone keeps its pages.
      } else {
        state.saveError = plainError(error.message);
        setChip("Not saved", "error");
        if ((state.screen === "linked" || state.screen === "invite") && !panel.hidden) draw();
      }
      try {
        await noticeJoin();
      } catch {
        // The save already failed. The next poll looks for a join again.
      }
    } finally {
      leading = false;
    }
  }

  function openLive() {
    if (!state.member || !state.desk || !state.account || state.socket || state.removed) return;
    const path = `/v1/mailboxes/${state.account}/live`;
    let signed;
    try {
      signed = JSON.parse(sync.signRequest(state.desk, "GET", path, ""));
    } catch {
      return;
    }
    if (!signed.ok) return;
    const base = syncOrigin().replace(/^http/, "ws");
    const query = `device=${encodeURIComponent(signed.device)}&time=${encodeURIComponent(signed.time)}&nonce=${encodeURIComponent(signed.nonce)}&signature=${encodeURIComponent(signed.signature)}`;
    let socket;
    try {
      socket = new WebSocket(`${base}${path}?${query}`);
    } catch {
      return;
    }
    state.socket = socket;
    socket.onopen = () => {
      state.live = true;
      state.liveWait = 1000;
      paintConnection();
    };
    socket.onmessage = (event) => {
      let rev = 0;
      try {
        const note = JSON.parse(event.data);
        if (note && Object.hasOwn(note, "record")) return;
        rev = Number(note && note.rev) || 0;
      } catch {
        return;
      }
      if (rev && rev !== state.revision) schedule();
    };
    socket.onclose = () => {
      state.live = false;
      if (state.socket === socket) state.socket = null;
      paintConnection();
      const wait = state.liveWait || 1000;
      state.liveWait = Math.min(wait * 2, 30000);
      setTimeout(openLive, wait);
    };
    socket.onerror = () => {
      try { socket.close(); } catch { /* close follows the error */ }
    };
  }

  function nextPoll(delay) {
    clearTimeout(timer);
    timer = setTimeout(runPoll, delay);
  }

  function runPoll() {
    tick().finally(() => {
      const soon = hurry;
      hurry = false;
      openLive();
      nextPoll(pollWait({
        hidden: document.hidden,
        dirty: state.dirty,
        saving: saveWanted,
        soon,
        live: state.live === true,
      }));
    });
  }

  function schedule() {
    hurry = true;
    if (ticking) return;
    nextPoll(pollWait({ soon: true }));
  }

  async function readMailbox() {
    const extra = state.revision ? { "If-None-Match": `"${state.revision}"` } : undefined;
    const mailbox = await callServer("GET", `/v1/mailboxes/${state.account}`, undefined, undefined, undefined, extra);
    if (mailbox.unchanged) return null;
    return mailbox;
  }

  async function findRequest(gate = APPROVAL_POLL_MS) {
    if (!state.invite || Date.now() >= state.invite.expires) {
      state.pending = null;
      return false;
    }
    const fast = gate <= INVITE_POLL_MS;
    const checked = fast ? inviteChecked : requestsChecked;
    if (Date.now() - checked < gate) return false;
    if (fast) inviteChecked = Date.now();
    else requestsChecked = Date.now();
    const data = await callServer("GET", `/v1/mailboxes/${state.account}/requests`);
    for (const request of data.requests || []) {
      if (request.state && request.state !== "pending") continue;
      const check = JSON.parse(sync.checkProof(state.invite.code, request.device, request.proof || ""));
      if (!check.ok) continue;
      const coded = JSON.parse(sync.codeFor(request.device));
      state.pending = { id: request.id, name: request.name, device: request.device, code: coded.code || "", created: request.created || 0 };
      publishPending();
      return true;
    }
    state.pending = null;
    return false;
  }

  function publishPending() {
    if (!inviteChannel || !state.pending || !state.account) return;
    inviteChannel.postMessage({
      kind: "join-request",
      account: state.account,
      pending: {
        id: state.pending.id,
        name: state.pending.name,
        device: state.pending.device,
        code: state.pending.code,
        created: state.pending.created,
      },
    });
  }

  function inviteLive() {
    return !!(state.invite && Date.now() < state.invite.expires && (state.screen === "invite" || state.screen === "linked"));
  }

  function queueInviteWatch() {
    if (inviteWatch || inviteLooking || !inviteLive()) return;
    const delay = inviteBackoff || (document.hidden ? INVITE_POLL_HIDDEN_MS : INVITE_POLL_MS);
    inviteWatch = setTimeout(runInviteWatch, delay);
  }

  function restartInviteWatch() {
    clearTimeout(inviteWatch);
    inviteWatch = 0;
    queueInviteWatch();
  }

  async function runInviteWatch() {
    inviteWatch = 0;
    if (!inviteLive() || inviteLooking) return;
    inviteLooking = true;
    let found = false;
    try {
      found = await findRequest(INVITE_POLL_MS);
      inviteBackoff = 0;
    } catch (error) {
      if (error.status === 429) inviteBackoff = inviteBackoff >= 4000 ? 8000 : 4000;
    } finally {
      inviteLooking = false;
    }
    if (found && (state.screen === "invite" || state.screen === "linked" || state.screen === "ready")) {
      state.screen = "approve";
      draw();
      return;
    }
    queueInviteWatch();
  }

  function applySharedRequest(data) {
    if (!data || data.kind !== "join-request" || !state.member || data.account !== state.account) return;
    if (state.screen === "approve" && state.pending) return;
    if (!["linked", "invite", "ready"].includes(state.screen)) return;
    const pending = data.pending || {};
    if (!pending.id || !pending.device) return;
    state.pending = {
      id: pending.id,
      name: pending.name,
      device: pending.device,
      code: pending.code || "",
      created: pending.created || 0,
    };
    state.screen = "approve";
    draw();
  }

  function stopScan() {
    if (loop) cancelAnimationFrame(loop);
    loop = null;
    if (stream) stream.getTracks().forEach((track) => track.stop());
    stream = null;
    state.video = false;
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
    const started = JSON.parse(sync.startNew(pageOrigin()));
    state.busy = false;
    if (!started.ok) {
      state.error = plainError(started.message);
      state.screen = "off";
      draw();
      return;
    }
    state.desk = started.id;
    state.words = started.words;
    state.wroteWords = false;
    state.screen = "save";
    draw();
  }

  async function confirmWords() {
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
    if (!state.account) {
      let saved;
      try {
        saved = JSON.parse(sync.confirmSaved(state.desk, JSON.stringify(chosen)));
      } catch (error) {
        state.busy = false;
        state.error = plainError(error && error.message ? error.message : "Those words could not be checked.");
        draw();
        return;
      }
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
      const sealed = await sealPacked(await pack(snap.doc, snap.pages));
      await commitSeal(sealed);
      await rememberBase(snap.pages);
      state.member = true;
      state.removed = false;
      state.screen = "ready";
      await refreshDevices();
      calm();
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
      await callServer("POST", "/v1/access", { code }, undefined, "open");
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

  async function applySyncLink(link) {
    const found = link || takeSyncLink();
    if (!found) return;
    api.openSettings("sync");
    if (found.kind === "recover") {
      state.phrase = found.secret;
      state.error = "";
      await openRecover();
      onPhrase();
      return;
    }
    if (found.kind === "join") {
      const parsed = parseJoin(found.secret);
      if (!parsed) {
        state.screen = "join-start";
        state.error = "That code didn't work. Check it on the other phone or computer.";
        draw();
        return;
      }
      await submitJoin(parsed.code, parsed.origin, parsed.account);
    }
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

  function leaveError(error) {
    if (error && (error.offline || navigator.onLine === false)) {
      return `Sync is offline, so ${here()} is still on the list. Try again, or turn off only here. Another phone or computer can still remove this one.`;
    }
    return `Couldn't reach sync, so ${here()} is still on the list. Try again, or turn off only here. Another phone or computer can still remove this one.`;
  }

  async function restoreDesk(prior) {
    if (!prior) return;
    const imported = JSON.parse(sync.importLocal(prior));
    if (!imported.ok) return;
    state.desk = imported.id;
    if (imported.account) state.account = imported.account;
  }

  async function leaveList() {
    const exported = JSON.parse(sync.exportLocal(state.desk));
    const prior = exported.ok ? exported.file : "";
    for (let attempt = 0; attempt < 2; attempt += 1) {
      let changed = false;
      try {
        const mailbox = await callServer("GET", `/v1/mailboxes/${state.account}`);
        if (mailbox.revision !== state.revision) await pull(mailbox);
        await refreshDevices();
        const self = state.devices.find((device) => device.self);
        if (!self) throw new Error("Couldn't sync.");
        changed = true;
        const removed = JSON.parse(sync.removeDevice(state.desk, self.id));
        if (!removed.ok) throw new Error(removed.message || "Couldn't sync.");
        const snap = await api.snapshot();
        const packed = await pack(snap.doc, snap.pages);
        const sealed = await sealPacked(packed);
        await commitSeal(sealed);
        await rememberBase(snap.pages);
        return;
      } catch (error) {
        if (changed) await restoreDesk(prior);
        if (error.status === 409 && attempt === 0) continue;
        throw error;
      }
    }
  }

  async function turnOff(localOnly) {
    if (state.busy) return;
    state.busy = true;
    state.error = "";
    draw();
    await asLeader(async () => {
      try {
        if (!localOnly && !state.leftRemote) await leaveList();
        state.leftRemote = !localOnly;
        await writeVault(snapshotVault({ paused: false, deviceFile: "" }));
        if (api.storeRemove) await api.storeRemove("sync-paused");
      } catch (error) {
        state.busy = false;
        state.offFailed = true;
        state.error = localOnly ? "Sync is still on. Try again." : leaveError(error);
        draw();
        return;
      }
      stopScan();
      state.desk = null;
      state.account = "";
      state.revision = 0;
      state.member = false;
      state.removed = false;
      state.devices = [];
      state.invite = null;
      state.pending = null;
      state.words = [];
      state.requestId = "";
      state.joinProof = "";
      state.dirty = false;
      state.leftRemote = false;
      state.offFailed = false;
      state.error = "";
      state.screen = "off";
      state.busy = false;
      // A save started by the last edit must not put Syncing back after this.
      dropSaving();
      hideChip();
      draw();
    });
  }

  async function startInvite() {
    state.busy = true;
    state.error = "";
    draw();
    const raw = crypto.getRandomValues(new Uint8Array(8));
    raw[0] &= 0x0f;
    const secret = btoa(String.fromCharCode(...raw));
    const made = JSON.parse(sync.makeInvite(state.account, syncOrigin(), pageOrigin(), secret));
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
    requestsChecked = 0;
    inviteChecked = 0;
    inviteBackoff = 0;
    draw();
    schedule();
  }

  async function submitJoin(code, origin, account) {
    stopScan();
    state.joinCode = code || "";
    state.screen = "join-type";
    state.busy = true;
    state.error = "";
    draw();
    const started = JSON.parse(sync.startJoin());
    if (!started.ok) {
      state.busy = false;
      state.error = joinError(started.message || "That code didn't work. Check it on the other phone or computer.");
      draw();
      return;
    }
    const proof = JSON.parse(sync.joinProof(started.device, code || ""));
    if (!proof.ok) {
      state.busy = false;
      state.error = joinError(proof.message || "That code didn't work. Check it on the other phone or computer.");
      draw();
      return;
    }
    const name = typedJoinName();
    try {
      const body = {
        code,
        device: started.device,
        name,
        proof: proof.proof,
      };
      if (/^[0-9a-f]{64}$/i.test(account || "")) body.account = String(account).toLowerCase();
      const created = await callServer("POST", "/v1/join", body, origin, "open");
      state.desk = started.id;
      state.code = started.code;
      state.device = started.device;
      state.account = created.account;
      state.requestId = created.id;
      state.joinProof = proof.proof;
      state.joinUntil = Date.now() + INVITE_MS;
      state.joinWaits = 0;
    } catch (error) {
      state.busy = false;
      state.error = joinError(error.message || "That code didn't work. Check it on the other phone or computer.");
      draw();
      return;
    }
    state.screen = "join";
    state.busy = false;
    draw();
    waitForApproval();
  }

  function parseJoin(raw) {
    const parsed = parseJoinPayload(raw);
    if (!parsed) return null;
    if (!/^[0-9a-f]{64}$/i.test(parsed.account) || !allowedOrigin(parsed.origin)) return null;
    return parsed;
  }

  function allowedOrigin(origin) {
    try {
      const url = new URL(origin);
      if (url.origin === location.origin || url.origin === syncOrigin()) return true;
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
            submitJoin(parsed.code, parsed.origin, parsed.account);
            return;
          }
        }
      }
      loop = requestAnimationFrame(step);
    };
    step();
  }

  function typedJoinName() {
    const field = panel.querySelector('[aria-label="Name this phone or computer"]');
    const typed = cleanName(field && field.value);
    const name = typed || cleanName(state.selfName) || guessDeviceName();
    if (name !== state.selfName) {
      state.selfName = name;
      state.selfRev += 1;
      saveNames();
    }
    return name;
  }

  function joinStillOpen() {
    return Date.now() < (state.joinUntil || 0);
  }

  async function waitForApproval() {
    if (state.screen !== "join" || !state.requestId) return;
    if (!joinStillOpen()) {
      state.error = "That code expired. Ask for a new one on the other phone or computer.";
      state.busy = false;
      draw();
      return;
    }
    try {
      const decision = await callServer("GET", `/v1/mailboxes/${state.account}/requests/${state.requestId}`, undefined, undefined, { proof: state.joinProof });
      if (decision.state === "pending") {
        state.joinWaits = 0;
        setTimeout(waitForApproval, JOIN_WAIT_MS);
        return;
      }
      if (decision.state !== "approved") {
        state.error = "That phone or computer said no.";
        draw();
        return;
      }
      const arrived = cleanName(decision.name);
      if (arrived && arrived !== state.selfName) {
        state.selfName = arrived;
        state.selfRev += 1;
        saveNames();
      }
      state.approval = { chain: decision.chain, wrap: decision.wrap };
      let mailbox = null;
      for (let miss = 0; miss < MAILBOX_TRIES; miss += 1) {
        try {
          mailbox = await callServer("GET", `/v1/mailboxes/${state.account}`);
          break;
        } catch (error) {
          const again = (error.status === 404 || error.status === 429) && miss + 1 < MAILBOX_TRIES && joinStillOpen();
          if (!again) throw error;
          await sleep(Math.min(4000, 500 * 2 ** miss));
        }
      }
      const named = JSON.parse(sync.recordWords(mailbox.record));
      if (!named.ok) throw new Error(named.message || "Couldn't sync.");
      await callServer("POST", `/v1/mailboxes/${state.account}/requests/${state.requestId}`, { done: true });
      state.joinWords = named.words;
      state.joinWaits = 0;
      state.screen = "join-confirm";
      state.busy = false;
      draw();
    } catch (error) {
      if (error.status === 429 && joinStillOpen()) {
        state.joinWaits = Math.min((state.joinWaits || 0) + 1, 5);
        const wait = Math.min(8000, 500 * 2 ** (state.joinWaits - 1));
        setTimeout(waitForApproval, wait);
        return;
      }
      state.joinWaits = 0;
      state.error = plainError(error.message);
      state.busy = false;
      draw();
    }
  }

  function sleep(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  async function waitForJoin(id, deadline) {
    let delay = APPROVAL_POLL_MS;
    while (Date.now() < deadline) {
      let data;
      try {
        data = await callServer("GET", `/v1/mailboxes/${state.account}/requests`);
      } catch (error) {
        if (error.status !== 429 || Date.now() >= deadline) throw error;
        await sleep(delay);
        delay = Math.min(delay * 2, 8000);
        continue;
      }
      const row = (data.requests || []).find((request) => request.id === id);
      if (!row) return true;
      await sleep(delay);
      delay = Math.min(delay * 2, 8000);
    }
    return false;
  }

  async function acceptJoin() {
    if (state.busy || state.screen !== "join-confirm" || !state.approval) return;
    state.busy = true;
    state.error = "";
    draw();
    try {
      const confirmed = JSON.parse(sync.confirmLink(state.desk));
      if (!confirmed.ok) throw new Error(confirmed.message || "Couldn't sync.");
      let mailbox = await callServer("GET", `/v1/mailboxes/${state.account}`);
      let plan = JSON.parse(sync.approvalPlan(state.desk, state.approval.chain, state.approval.wrap, mailbox.record));
      for (let attempt = 0; !plan.ok && attempt < 3 && /saved copy/i.test(plan.message || ""); attempt += 1) {
        mailbox = await callServer("GET", `/v1/mailboxes/${state.account}`);
        plan = JSON.parse(sync.approvalPlan(state.desk, state.approval.chain, state.approval.wrap, mailbox.record));
      }
      if (!plan.ok) throw new Error(plan.message);
      let packed = plan.packed;
      if (!plan.legacy) {
        const fetched = await fetchObjects(plan.want || []);
        const finished = JSON.parse(sync.approvalFinish(state.desk, mailbox.record, JSON.stringify(fetched)));
        if (!finished.ok) throw new Error(finished.message);
        packed = finished.packed;
        if (finished.slots) state.slots = finished.slots;
      }
      const remote = await asSet(packed);
      const snap = await api.snapshot();
      const merged = JSON.parse(sync.mergePacked(await pack(snap.doc, snap.pages), remote.packed));
      if (!merged.ok) throw new Error(merged.message || "Those pages could not be opened.");
      await installMerged(merged);
      state.revision = mailbox.revision;
      state.member = true;
      state.removed = false;
      state.screen = "linked";
      await saveFile();
      await refreshDevices();
      try {
        await publishListedName();
      } catch {
        state.dirty = true;
      }
      calm();
    } catch (error) {
      state.error = plainError(error.message);
    }
    state.busy = false;
    draw();
  }

  async function approve() {
    if (!state.pending || state.busy) return;
    await asLeader(async () => {
      if (!state.pending || state.busy) return;
      const id = state.pending.id;
      let still = false;
      try {
        const data = await callServer("GET", `/v1/mailboxes/${state.account}/requests`);
        still = (data.requests || []).some((request) => request.id === id);
      } catch (error) {
        state.error = plainError(error.message);
        draw();
        return;
      }
      if (!still) {
        state.pending = null;
        state.invite = null;
        state.error = "";
        state.screen = "linked";
        draw();
        return;
      }
      await approvePending();
    });
  }

  async function dropJoined(code) {
    const list = JSON.parse(sync.deviceList(state.desk));
    if (!list.ok) return;
    const added = (list.devices || []).find((device) => !device.self && device.code === code);
    if (!added?.id) return;
    const removed = JSON.parse(sync.removeDevice(state.desk, added.id));
    if (!removed.ok) throw new Error(removed.message || "Couldn't sync.");
    noteLocalEdit(state);
    await push();
  }

  async function releaseJoin(requestId) {
    await callServer("POST", `/v1/mailboxes/${state.account}/requests/${requestId}`, { release: true });
  }

  function adoptAskedName(code, name) {
    const asked = cleanName(name);
    if (!asked || !code || !state.desk) return;
    const list = JSON.parse(sync.deviceList(state.desk));
    if (!list.ok) return;
    const added = (list.devices || []).find((device) => !device.self && device.code === code);
    if (!added?.id) return;
    const row = state.deviceNames.find((item) => item.id === added.id);
    if (!row) state.deviceNames.push({ id: added.id, rev: 1, name: asked });
    else if (row.name !== asked) {
      row.name = asked;
      row.rev = (row.rev || 0) + 1;
    }
    saveNames();
  }

  async function approvePending() {
    const prior = currentVault()?.deviceFile || api.storeGet("device-file") || "";
    const askedName = cleanName(state.pending?.name || "");
    const askedCode = state.pending?.code || "";
    state.busy = true;
    state.error = "";
    draw();
    let published = false;
    let requestId = "";
    try {
      const approval = JSON.parse(sync.approveDevice(state.desk, state.pending.device));
      if (!approval.ok) {
        state.error = approval.message;
        return;
      }
      requestId = state.pending.id;
      adoptAskedName(askedCode, askedName);
      const snap = await api.snapshot();
      const mailbox = await callServer("GET", `/v1/mailboxes/${state.account}`);
      state.revision = mailbox.revision;
      const sealed = await sealPacked(await pack(snap.doc, snap.pages));
      const body = {
        chain: approval.chain,
        wrap: approval.wrap,
        record: sealed.record,
        revision: state.revision,
      };
      if (sealed.objects && sealed.objects.length) body.objects = sealed.objects;
      if (sealed.drop && sealed.drop.length) body.drop = sealed.drop;
      const posted = await callServer("POST", `/v1/mailboxes/${state.account}/requests/${requestId}`, body, undefined, undefined, {
        "If-Match": `"${state.revision}"`,
      });
      const slots = JSON.parse(sealed.slots || "[]");
      for (const row of posted.objects || []) {
        const slot = slots.find((item) => item.objectId === row.id);
        if (slot) slot.serverRev = row.revision;
      }
      state.slots = JSON.stringify(slots);
      state.revision = posted.revision;
      await writeVault(snapshotVault({ slots: state.slots, revision: String(state.revision || 0) }));
      await saveFile();
      published = true;
      await rememberBase(snap.pages);
      const created = Number(state.pending.created) || Date.now();
      const deadline = Math.min(created + INVITE_MS, Date.now() + JOIN_FINISH_MS);
      const finished = await waitForJoin(requestId, deadline);
      if (!finished) throw new Error("That phone or computer didn't finish. Ask them to try again.");
      state.pending = null;
      state.invite = null;
      state.error = "";
      state.screen = "linked";
      await refreshDevices();
      acceptListed();
      calm();
    } catch (error) {
      if (published) {
        try {
          await dropJoined(askedCode);
        } catch {
          // The list still has this phone if the removal did not save.
        }
        try {
          await releaseJoin(requestId);
        } catch {
          // The request can already be gone.
        }
      } else if (prior) {
        await writeVault(snapshotVault({ deviceFile: prior }));
        const imported = JSON.parse(sync.importLocal(prior));
        if (imported.ok) {
          state.desk = imported.id;
          if (imported.account) state.account = imported.account;
        }
      }
      state.error = plainError(error.message);
    } finally {
      state.busy = false;
      draw();
    }
  }

  async function decline() {
    if (!state.pending) {
      state.screen = "linked";
      draw();
      return;
    }
    await asLeader(async () => {
      if (!state.pending) {
        state.screen = "linked";
        draw();
        return;
      }
      const id = state.pending.id;
      let still = true;
      try {
        const data = await callServer("GET", `/v1/mailboxes/${state.account}/requests`);
        still = (data.requests || []).some((request) => request.id === id);
      } catch {
        still = true;
      }
      if (still) {
        try {
          await callServer("POST", `/v1/mailboxes/${state.account}/requests/${id}`, { reject: true });
        } catch {
          // The request can expire. This device still has the pages.
        }
      }
      state.pending = null;
      state.invite = null;
      state.screen = "linked";
      draw();
    });
  }

  function askRemove(device) {
    if (!device || device.self) return;
    state.removing = device;
    state.screen = "remove-ask";
    state.error = "";
    draw();
  }

  async function removeDevice(id) {
    await asLeader(async () => {
      try {
        const mailbox = await callServer("GET", `/v1/mailboxes/${state.account}`);
        if (mailbox.revision !== state.revision) await pull(mailbox);
      } catch (error) {
        state.error = plainError(error.message);
        state.busy = false;
        draw();
        return;
      }
      await refreshDevices();
      if (!state.devices.some((device) => device.id === id && !device.self)) {
        state.removing = null;
        state.screen = "linked";
        state.busy = false;
        draw();
        return;
      }
      await removeListed(id);
    });
  }

  async function removeListed(id) {
    const removed = JSON.parse(sync.removeDevice(state.desk, id));
    if (!removed.ok) {
      state.error = removed.message;
      state.busy = false;
      draw();
      return;
    }
    noteLocalEdit(state);
    try {
      await push();
      await refreshDevices();
      acceptListed();
    } catch (error) {
      state.error = plainError(error.message);
    }
    state.removing = null;
    state.screen = "linked";
    state.busy = false;
    draw();
  }

  async function startRecovery() {
    stopScan();
    const named = cleanName(panel.querySelector("[data-recover-name]")?.value || "");
    if (named && named !== state.selfName) {
      state.selfName = named;
      state.selfRev += 1;
      saveNames();
    }
    const check = JSON.parse(sync.checkPhrase(state.phrase));
    if (!check.ok) {
      state.phraseMsg = "";
      state.phraseOk = false;
      state.error = check.message;
      draw();
      return;
    }
    state.busy = true;
    draw();
    await frame();
    const started = JSON.parse(sync.startRecovery(state.phrase));
    if (!started.ok) {
      state.busy = false;
      state.error = started.message;
      draw();
      return;
    }
    state.desk = started.id;
    const accepted = JSON.parse(sync.acceptRecovery(state.desk));
    if (!accepted.ok) {
      state.error = plainError(accepted.message);
      state.busy = false;
      draw();
      return;
    }
    state.account = accepted.account;
    state.notice = "";
    try {
      const mailbox = await callServer("GET", `/v1/mailboxes/${state.account}`);
      const peeked = JSON.parse(sync.recoveryPeek(state.desk, mailbox.record));
      if (!peeked.ok) throw new Error(peeked.message);
      let opened = peeked;
      if (!peeked.legacy && !peeked.partial) {
        const fetched = await fetchObjects(peeked.want || []);
        const applied = JSON.parse(sync.recoveryApply(state.desk, mailbox.record, JSON.stringify(fetched)));
        if (!applied.ok) throw new Error(applied.message);
        opened = { packed: applied.packed, partial: false, slots: applied.slots };
        if (applied.slots) state.slots = applied.slots;
      }
      const snap = await api.snapshot();
      if (opened.partial) {
        let packed = await pack(snap.doc, snap.pages);
        if (opened.packed) {
          const remote = await asSet(opened.packed);
          const merged = JSON.parse(sync.mergePacked(packed, remote.packed));
          if (!merged.ok) throw new Error(merged.message || "Those pages could not be opened.");
          packed = merged.packed;
        }
        const restored = JSON.parse(sync.reestablishPages(state.desk, packed));
        if (!restored.ok) throw new Error(restored.message || "Those pages could not be opened.");
        await installMerged({ packed });
        state.revision = mailbox.revision;
        state.member = true;
        state.removed = false;
        state.dirty = false;
        state.error = "";
        state.notice = PARTIAL_NOTE;
        state.slots = "[]";
        const sealed = await sealPacked(packed);
        const stale = (mailbox.objects || []).map((row) => row.id);
        sealed.drop = [...(sealed.drop || []), ...stale];
        await commitSeal(sealed);
        const kept = JSON.parse(sync.unpackPages(packed));
        if (kept.ok) await rememberBase(kept.pages);
        state.screen = "linked";
        await refreshDevices();
        calm();
      } else {
        const remote = await asSet(opened.packed);
        const merged = JSON.parse(sync.mergePacked(await pack(snap.doc, snap.pages), remote.packed));
        if (!merged.ok) throw new Error(merged.message || "Those pages could not be opened.");
        const unpacked = JSON.parse(sync.unpackPages(merged.packed));
        if (!unpacked.ok) throw new Error(unpacked.message || "Those pages could not be opened.");
        // The pages stay the copy the words just opened. Publishing adds this
        // computer to that list, so the other phones and computers can see it.
        const doc = remote.doc ? api.b64ToBytes(remote.doc) : new Uint8Array();
        const packed = await pack(doc, unpacked.pages || []);
        await installMerged({ packed });
        state.revision = mailbox.revision;
        state.removed = false;
        state.error = "";
        state.notice = "";
        state.screen = "linked";
        await saveFile();
        await refreshDevices();
        if (!state.member) throw new Error(`${hereCap()} could not be added.`);
        state.dirty = true;
        try {
          await push();
        } catch (error) {
          state.dirty = true;
          state.error = plainError(error.message);
        }
        calm();
      }
    } catch (error) {
      state.error = plainError(error.message);
      state.screen = "recover";
    }
    state.busy = false;
    draw();
  }

  function watchApproval() {
    if (approveWatch || state.screen !== "approve" || !state.pending) return;
    approveWatch = setTimeout(async () => {
      approveWatch = 0;
      if (state.screen !== "approve" || !state.pending) return;
      if (state.busy) {
        watchApproval();
        return;
      }
      const id = state.pending.id;
      const created = state.pending.created;
      try {
        const data = await callServer("GET", `/v1/mailboxes/${state.account}/requests`);
        const requests = data.requests || [];
        const row = requests.find((request) => request.id === id);
        if (
          approvalCard(requests, id, Date.now(), created, INVITE_MS) === "expired" &&
          state.screen === "approve" &&
          state.pending &&
          state.pending.id === id &&
          !state.busy
        ) {
          state.pending = null;
          state.busy = false;
          state.error = row && row.state === "rejected"
            ? "That phone or computer said no."
            : "That code expired. Ask for a new one on the other phone or computer.";
          state.screen = "linked";
          draw();
          return;
        }
      } catch {
        // The prompt stays up. The next check tries again.
      }
      if (state.screen === "approve" && state.pending) watchApproval();
    }, APPROVAL_POLL_MS);
  }

  function errorBesideField() {
    return state.screen === "access" || state.screen === "join-start" || state.screen === "join-type" || state.screen === "recover";
  }

  function fieldError(input) {
    if (!state.error || !input) return null;
    input.setAttribute("aria-invalid", "true");
    input.setAttribute("aria-describedby", FIELD_ERROR_ID);
    return el("p", { class: "sync-err", id: FIELD_ERROR_ID, role: "alert" }, [plainError(state.error)]);
  }

  function draw() {
    paintRemoval();
    if (naming) return;
    if (state.screen !== "approve") {
      clearTimeout(approveWatch);
      approveWatch = 0;
    }
    panel.replaceChildren();
    panel.classList.toggle("sync-sheet", state.screen === "save");
    panel.append(el("p", { class: "sync-label" }, [LABEL]));
    if (state.error && !errorBesideField()) {
      panel.append(el("p", { class: "sync-err", role: "alert" }, [plainError(state.error)]));
    }
    if (state.screen === "intro") drawIntro();
    else if (state.screen === "off") drawOff();
    else if (state.screen === "access") drawAccess();
    else if (state.screen === "save") drawSave();
    else if (state.screen === "confirm") drawConfirm();
    else if (state.screen === "invite") drawInvite();
    else if (state.screen === "join-start" || state.screen === "join-type") drawJoinType();
    else if (state.screen === "join-scan") drawJoinScan();
    else if (state.screen === "join") drawJoin();
    else if (state.screen === "join-confirm") drawJoinConfirm();
    else if (state.screen === "removed") drawRemoved();
    else if (state.screen === "approve") drawApprove();
    else if (state.screen === "remove-ask") drawRemoveAsk();
    else if (state.screen === "recover") drawRecover();
    else if (state.screen === "off-ask") drawOffAsk();
    else if (state.screen === "ready") drawReady();
    else drawLinked();
    if (freshSetup()) hideChip();
    dress();
    refreshCue();
    if (inviteLive()) queueInviteWatch();
    else {
      clearTimeout(inviteWatch);
      inviteWatch = 0;
    }
  }

  function dress() {
    const loose = [...panel.childNodes].filter((node) => {
      const cls = node.classList;
      return !(cls && (cls.contains("settings-group") || cls.contains("sync-label")));
    });
    if (!loose.length) return;
    const section = el("section", { class: "settings-group" });
    loose[0].before(section);
    section.append(...loose);
  }

  function backLink(run, disabled) {
    const node = button("← Back", { type: "button", class: "sync-back" });
    if (disabled) node.disabled = true;
    node.addEventListener("click", run);
    return node;
  }

  function progressBar(step) {
    const bar = el("ol", { class: "sync-progress", "aria-label": `Step ${step} of 4` });
    for (let n = 1; n <= 4; n += 1) {
      const item = el("li", { class: n < step ? "sync-progress-done" : n === step ? "sync-progress-now" : "" });
      if (n === step) item.setAttribute("aria-current", "step");
      bar.append(item);
    }
    return bar;
  }

  function choiceCard(glyph, title, detail, run, disabled) {
    const node = el("button", { type: "button", class: "sync-card" }, [
      el("span", { class: "sync-card-icon", "aria-hidden": "true" }, [glyph]),
      el("span", { class: "sync-card-copy" }, [
        el("b", {}, [title]),
        el("span", {}, [detail]),
      ]),
      el("span", { class: "sync-card-chev", "aria-hidden": "true" }, ["›"]),
    ]);
    if (disabled) node.disabled = true;
    node.addEventListener("click", run);
    return node;
  }

  function actionButton(label, kind, run, disabled, attrs = {}) {
    const wrap = el("div", { class: "sync-actions" });
    const node = button(label, { type: "button", class: kind ? `btn ${kind}` : "btn", ...attrs });
    if (disabled) node.disabled = true;
    node.addEventListener("click", run);
    wrap.append(node);
    return wrap;
  }

  function quietLinks(rows) {
    const wrap = el("p", { class: "sync-links" });
    rows.forEach(([label, run, disabled]) => {
      const dim = label === "Turn sync off";
      const node = button(label, { type: "button", class: dim ? "sync-link sync-off" : "sync-link" });
      if (disabled) node.disabled = true;
      node.addEventListener("click", run);
      wrap.append(node);
    });
    return wrap;
  }

  function deviceNameControl() {
    return [
      el("label", { class: "sync-field" }, [`Name this ${onPhone() ? "phone" : "computer"}`]),
      nameField("Name this phone or computer", state.selfName, (name) => {
        const next = name || guessDeviceName();
        if (next === state.selfName) return;
        state.selfName = next;
        state.selfRev += 1;
        saveNames();
      }),
    ];
  }

  function drawIntro() {
    const here = onPhone() ? "phone" : "computer";
    panel.append(
      el("h2", {}, ["How sync works"]),
      el("p", { class: "setting-help" }, [`Pages are encrypted on this ${here} before they leave. The server keeps them, and it cannot read them.`]),
      el("p", { class: "setting-help" }, ["You get 12 recovery words. They are the only way back in if you lose every phone and computer. Write them down. Omacus does not keep a copy."]),
      el("p", { class: "setting-help" }, ["You link another phone or computer with a code. After that, your pages stay in step, even when one of them is offline."]),
      actionButton("Set up sync", "primary", () => { state.error = ""; state.screen = "off"; draw(); }),
      quietLinks([["Not now", () => { if (api.closeSettings) api.closeSettings(); else api.openSettings("settings"); }]]),
    );
  }

  function drawOff() {
    panel.append(
      el("h2", {}, ["Turn on sync"]),
      el("p", { class: "setting-help" }, ["End-to-end encrypted. Only your devices can read your pages — not even our server."]),
      el("div", { class: "sync-cards" }, [
        choiceCard("✦", "This is my first device", "Start fresh and keep these pages in sync from here.", () => {
          state.error = "";
          state.screen = "access";
          draw();
        }, state.busy),
        choiceCard("⇄", "My pages already sync on another device", "Join with a code from that phone or computer.", () => addDevice(), state.busy),
      ]),
      backLink(() => api.openSettings("settings"), state.busy),
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
    field.placeholder = "Code you were given";
    const inviteLabel = el("label", { class: "sync-field sync-code-label" }, ["Invite code"]);
    const accessError = fieldError(field);
    panel.append(
      progressBar(1),
      backLink(() => { state.screen = "off"; state.error = ""; draw(); }, state.busy),
      el("h2", {}, [`Start fresh on ${onPhone() ? "this phone" : "this computer"}`]),
      el("p", { class: "setting-help" }, ["Your pages get encrypted here before they leave. The server stores them but can't read them."]),
      ...deviceNameControl(),
      el("p", { class: "sync-hint" }, ["Shown on your other devices so you can tell them apart."]),
      inviteLabel,
      field,
      ...(accessError ? [accessError] : []),
      el("p", { class: "sync-hint" }, ["omacus sync is invite-only for now."]),
      el("p", { class: "sync-hint" }, ["Not the code from another device."]),
      actionButton(state.busy ? "Working…" : "Continue", "primary", () => checkAccess(), state.busy),
    );
    field.focus();
  }

  function drawSave() {
    const words = el("div", { class: "sync-words", role: "group", "aria-label": "Recovery words" });
    state.words.forEach((word, index) => {
      const n = index + 1;
      words.append(el("div", { class: "sync-word", "data-n": String(n), "data-word": word }, [
        el("b", {}, [String(n)]),
        word,
      ]));
    });
    words.classList.add("sync-words-grid");
    const wrote = el("input", { type: "checkbox", "data-wrote-words": "" });
    wrote.checked = !!state.wroteWords;
    const check = el("label", { class: "sync-check" }, [wrote, "I wrote these down or printed them somewhere safe."]);
    wrote.addEventListener("change", () => {
      state.wroteWords = wrote.checked;
      const saved = panel.querySelector("[data-saved-words]");
      if (saved) saved.disabled = !wrote.checked;
    });
    panel.append(
      progressBar(2),
      backLink(() => { state.wroteWords = false; leaveFresh(); }),
      el("h2", {}, ["Save your recovery words"]),
      el("p", { class: "setting-help" }, ["These 12 words are the only way back in if you lose every device. Omacus never keeps a copy, and you won't see them again."]),
      words,
      check,
      actionButton("I saved them", "primary", async () => {
        if (!state.wroteWords) return;
        if (!setUpCheck()) {
          state.error = WORD_CHECK_ERROR;
          draw();
          return;
        }
        state.screen = "confirm";
        state.error = "";
        draw();
      }, !state.wroteWords, { "data-saved-words": "" }),
      quietLinks([
        ["Print", () => printWords()],
        ["Copy", () => copyWords()],
      ]),
      el("p", { class: "sync-hint" }, ["A clipboard history may keep these words. Printing them, or writing them down, is safest."]),
    );
  }

  function drawConfirm() {
    const [a, b, c] = state.asks;
    panel.append(
      progressBar(3),
      backLink(() => {
        state.screen = "save";
        state.wrong = 0;
        state.error = "";
        draw();
      }, state.busy),
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
    panel.append(actionButton(
      state.busy ? "Checking the words…" : "These match",
      "primary",
      () => confirmWords(),
      state.busy || picks().length !== 3,
    ));
  }

  function drawInvite() {
    const qr = el("div", { class: "sync-qr sync-join" });
    const picture = qrNode(state.invite?.qr, "Join code");
    if (picture) qr.append(picture);
    qr.append(el("b", {}, ["Or scan this picture"]));
    panel.append(
      backLink(() => { state.screen = "linked"; state.error = ""; draw(); }),
      el("h2", {}, ["Add a phone or computer"]),
      el("p", { class: "setting-help" }, ["On the other phone or computer, open Sync and choose My pages already sync on another device. Type this code. It works for about 5 minutes, and only once."]),
      el("div", { class: "sync-invite-code" }, [state.invite?.code || ""]),
      el("p", { class: "sync-hint" }, ["It looks like K7QM-4R2P-9HNX."]),
      qr,
    );
  }

  function scanCodeButton() {
    const node = el("button", {
      type: "button",
      class: "sync-scan",
      "aria-label": "Scan QR code",
    }, [qrIcon()]);
    if (state.busy) node.disabled = true;
    node.addEventListener("click", () => scanJoin());
    return node;
  }

  function recoveryRow() {
    const link = button("Use recovery words →", { type: "button", class: "sync-link" });
    if (state.busy) link.disabled = true;
    link.addEventListener("click", () => openRecover());
    return el("div", { class: "sync-recover" }, [
      el("span", {}, ["Lost your other devices?"]),
      link,
    ]);
  }

  function drawJoinType() {
    const field = el("input", {
      class: "sync-name sync-join-input",
      "aria-label": "Join code",
      placeholder: "K7QM-4R2P-9HNX",
      autocapitalize: "characters",
      autocomplete: "off",
      spellcheck: "false",
      "data-join-code": "",
    });
    field.value = state.joinCode || "";
    field.addEventListener("input", () => {
      state.joinCode = field.value;
    });
    const joinFieldError = fieldError(field);
    panel.append(
      backLink(() => { stopScan(); state.screen = state.removed ? "removed" : "off"; state.error = ""; draw(); }, state.busy),
      el("h2", {}, ["Join your synced pages"]),
      el("p", { class: "setting-help" }, [
        "On a device that already syncs, open ",
        el("b", {}, ["Sync → Add a phone or computer"]),
        ". Scan the QR code it shows, or type the code.",
      ]),
      el("label", { class: "sync-field sync-code-label" }, ["Join code"]),
      el("div", { class: "sync-code-row" }, [field, scanCodeButton()]),
      ...(joinFieldError ? [joinFieldError] : []),
      el("p", { class: "sync-hint" }, ["12 characters · or tap the scan button to scan the QR code"]),
      recoveryRow(),
      ...deviceNameControl(),
      el("p", { class: "sync-hint" }, [`Pages already on ${here()} will be combined with your synced pages. Nothing is deleted.`]),
      actionButton(state.busy ? "Joining…" : "Join", "primary", () => submitJoin(state.joinCode), state.busy),
    );
    field.focus();
  }

  function drawJoinScan() {
    panel.append(
      backLink(() => { stopScan(); state.screen = "join-type"; state.error = ""; draw(); }),
      el("h2", {}, ["Scan the code"]),
      el("p", { class: "setting-help" }, ["Point the camera at the picture on your other phone or computer."]),
      el("video", { class: "sync-video", autoplay: "", playsinline: "", muted: "" }),
      quietLinks([["Type the code instead", () => { stopScan(); state.screen = "join-type"; state.error = ""; draw(); }]]),
    );
  }

  function shotCard() {
    return new URLSearchParams(location.search).get("syncCard");
  }

  function shotDots(dots) {
    return shotCard() ? dots : "";
  }

  function drawJoin() {
    panel.append(
      el("h2", {}, ["Approve on your other device"]),
      el("p", { class: "setting-help" }, ["The same six numbers should be on that phone or computer. Tap Yes there only if they match."]),
      el("div", { class: "sync-code" }, [state.code || shotDots("······")]),
      el("p", { class: "setting-help" }, [state.busy ? "Opening your pages…" : "Waiting for that phone or computer"]),
      quietLinks([["Cancel", () => { state.screen = "off"; state.requestId = ""; state.error = ""; draw(); }, state.busy]]),
    );
  }

  function drawJoinConfirm() {
    panel.append(
      el("h2", {}, ["Do these match?"]),
      el("p", { class: "setting-help" }, ["The same six numbers and the same four words should be on your other phone or computer."]),
      el("div", { class: "sync-code" }, [state.code || ""]),
      el("div", { class: "sync-words" }, [state.joinWords || ""]),
      actionButton(state.busy ? "Opening your pages…" : "These match", "primary", () => acceptJoin(), state.busy),
      quietLinks([["Cancel", () => {
        state.screen = "off";
        state.requestId = "";
        state.approval = null;
        state.error = "";
        draw();
      }, state.busy]]),
    );
  }

  function drawRemoved() {
    const noun = onPhone() ? "phone" : "computer";
    panel.append(
      el("h2", {}, ["Sync is paused"]),
      el("p", { class: "setting-help", "data-sync-removed": "" }, [
        `This ${noun} is not on the list, so new pages stay here. Your pages are still here. The Sync button in the sidebar stays. Join again adds this ${noun} back. Nothing already here is deleted.`,
      ]),
    );
    const actions = actionButton("Join again", "primary", () => joinAgain(), state.busy);
    const stay = button("Sync", { type: "button", class: "btn" });
    stay.addEventListener("click", () => {
      state.screen = "removed";
      draw();
    });
    const again = button(state.busy ? "Checking…" : "Check again", { type: "button", class: "btn" });
    if (state.busy) again.disabled = true;
    again.addEventListener("click", () => checkAgain());
    actions.prepend(stay);
    actions.append(again);
    panel.append(actions);
  }

  async function checkAgain() {
    state.error = "";
    state.busy = true;
    draw();
    try {
      await refreshDevices();
      if (state.member && !state.removed && state.account) {
        state.screen = "linked";
        await pull(state.account);
      } else {
        state.screen = "removed";
      }
    } catch (err) {
      state.error = plainError(err && err.message ? err.message : "Couldn't check. Try again.");
      state.screen = "removed";
    }
    state.busy = false;
    draw();
    refreshCue();
  }

  async function joinAgain() {
    state.error = "";
    await addDevice();
  }

  function drawApprove() {
    let words = "";
    if (state.desk) {
      const named = JSON.parse(sync.accountWords(state.desk));
      if (named.ok) words = named.words;
    }
    const asking = cleanName(state.pending?.name || "");
    const heading = asking ? `Is this ${asking}?` : "Is this your phone or computer?";
    panel.append(
      el("h2", {}, [heading]),
      el("p", { class: "setting-help" }, ["It wants to open your pages. The same six numbers and the same four words should be on that screen. Those numbers only check this step. Tap Yes, approve it only if they match."]),
      el("div", { class: "sync-code" }, [state.pending?.code || shotDots("······")]),
      el("p", { class: "sync-words" }, [words || shotDots("···· ···· ···· ····")]),
      actionButton(state.busy ? "Approving…" : "Yes, approve it", "primary", () => approve(), state.busy),
      quietLinks([["No", () => decline(), state.busy]]),
    );
    if (!shotCard()) watchApproval();
  }

  function drawRemoveAsk() {
    const name = deviceTitle(state.removing || {});
    const heading = !name || name === "Phone or computer" ? "Remove this phone or computer?" : `Remove ${name}?`;
    panel.append(
      el("h2", {}, [heading]),
      el("p", { class: "setting-help" }, ["It stops getting new pages. Pages it already saved stay readable on it. New edits stay with the phones and computers that are still linked."]),
      actionButton("Remove", "danger", () => removeDevice(state.removing?.id), state.busy || !state.removing || state.removing.self),
      quietLinks([["Keep it", () => { state.screen = "linked"; state.removing = null; state.error = ""; draw(); }, state.busy]]),
    );
  }

  function drawRecover() {
    const phrase = el("textarea", {
      id: "sync-recovery-words",
      class: "sync-phrase",
      autocorrect: "off",
      autocapitalize: "none",
      spellcheck: "false",
      autocomplete: "off",
      "aria-label": "Recovery words",
    });
    phrase.value = state.phrase;
    phrase.addEventListener("input", onPhrase);
    const bits = [el("h2", {}, ["Use your recovery words"])];
    const recoverName = el("input", {
      class: "sync-name",
      "aria-label": "Name this phone or computer",
      maxlength: "40",
      autocomplete: "off",
      spellcheck: "false",
      "data-recover-name": "",
    });
    recoverName.value = state.selfName || "";
    bits.push(
      el("label", { class: "sync-field" }, [`Name this ${onPhone() ? "phone" : "computer"}`]),
      recoverName,
      el("p", { class: "setting-help" }, ["Type or paste the 12 recovery words. Numbers and capitals are fine. A page you delete is in Recently deleted. Restore puts it back."]),
    );
    const merged = mergeLine();
    if (merged) bits.push(el("p", { class: "setting-help" }, [merged]));
    const recoverError = fieldError(phrase);
    bits.push(el("label", { class: "sync-field", for: "sync-recovery-words" }, ["Recovery words"]), phrase);
    if (recoverError) bits.push(recoverError);
    bits.push(el("p", { class: "sync-msg", "data-sync-phrase": "", "data-ok": state.phraseOk ? "1" : "0" }, [state.phraseMsg]));
    bits.unshift(backLink(() => { stopScan(); state.screen = "join-type"; draw(); }, state.busy));
    bits.push(
      actionButton(state.busy ? "Checking the words…" : "Open my pages", "primary", () => startRecovery(), state.busy),
    );
    panel.append(...bits);
    phrase.focus();
  }

  function drawOffAsk() {
    const failed = !!state.offFailed;
    const cancel = () => {
      state.screen = "linked";
      state.error = "";
      state.offFailed = false;
      draw();
    };
    const links = failed
      ? [["Just turn off here", () => turnOff(true), state.busy], ["Not now", cancel, state.busy]]
      : [["Not now", cancel, state.busy]];
    panel.append(
      backLink(cancel, state.busy),
      el("h2", {}, [`Turn sync off on ${here()}?`]),
      el("p", { class: "setting-help" }, [`Your pages stay here. ${hereCap()} leaves the list, so the other phones and computers stop showing it. If this is the last one, that is fine. The recovery words still open the pages.`]),
      actionButton(state.busy ? "Working…" : (failed ? "Try again" : "Turn sync off"), "primary", () => turnOff(), state.busy),
      quietLinks(links),
    );
  }

  function deviceLine(device, right) {
    const row = el("div", { class: "sync-device" });
    if (device.self) row.setAttribute("data-self", "1");
    const name = el("div", { class: "sync-device-name" });
    if (device.self) name.append(el("span", { class: "sync-dot", "aria-hidden": "true" }, ["●"]));
    name.append(el("b", {}, [deviceTitle(device)]));
    if (device.self) name.append(el("span", { class: "sync-quiet" }, [` · this ${onPhone() ? "phone" : "computer"}`]));
    row.append(name);
    if (right) row.append(right);
    return row;
  }

  function renameControl(row, device) {
    const actions = el("div", { class: "sync-device-actions" });
    const rename = button("Rename", { type: "button", class: "btn small" });
    rename.addEventListener("click", () => beginRename(row, device));
    actions.append(rename);
    if (!device.self) {
      const remove = button("Remove", { type: "button", class: "btn small" });
      remove.addEventListener("click", () => askRemove(device));
      actions.append(remove);
    }
    return actions;
  }

  function drawReady() {
    const devices = state.devices.length
      ? [...state.devices].sort((a, b) => Number(!!b.self) - Number(!!a.self))
      : [{ self: true, name: state.selfName }];
    const list = el("div", { class: "sync-devices" });
    for (const device of devices) {
      const row = deviceLine(device, device.self
        ? el("span", { class: "sync-when" }, ["Synced just now"])
        : null);
      if (!device.self) row.append(renameControl(row, device));
      list.append(row);
    }
    panel.append(
      progressBar(4),
      el("h2", {}, ["✓ Sync is on"]),
      el("p", { class: "setting-help", "data-sync-linked": "" }, ["Your pages are encrypted and saved. Add another phone or computer to keep them in step."]),
      el("div", { "data-sync-set": "" }, [list]),
      actionButton(state.busy ? "Working…" : "Add a phone or computer", "primary", () => startInvite(), state.busy),
      quietLinks([
        ["Done", () => { state.screen = "linked"; state.error = ""; draw(); }],
        ["Turn sync off", () => { state.screen = "off-ask"; state.error = ""; draw(); }, state.busy],
      ]),
    );
  }

  function drawLinked() {
    if (state.removed) {
      drawRemoved();
      return;
    }
    if (!state.member) {
      panel.append(
        backLink(() => api.openSettings("settings")),
        el("h2", {}, [`Sync is off on ${here()}`]),
        el("p", { class: "setting-help", "data-sync-linked": "" }, ["Your pages are still here. The recovery words can open them again."]),
        actionButton("Use my recovery words", "primary", () => openRecover()),
      );
      return;
    }
    const home = el("div", { "data-sync-set": "" });
    const others = state.devices.some((device) => !device.self);
    home.append(
      backLink(() => api.openSettings("settings"), state.busy),
      el("h2", {}, ["Sync is on"]),
    );
    if (state.listNote) {
      const note = button(state.listNote, {
        type: "button",
        class: "sync-list-notice",
        "data-sync-list": "",
      });
      note.addEventListener("click", () => {
        state.screen = "linked";
        draw();
      });
      panel.append(el("section", { class: "settings-group" }, [
        el("h2", {}, ["List changed"]),
        note,
        el("p", { class: "setting-help" }, ["Sync is still on. This note does not stop pages from saving."]),
        quietLinks([["Got it", () => {
          rememberDevices(state.devices.map((device) => device.id));
          draw();
          refreshCue();
        }]]),
      ]));
    }
    if (state.notice) {
      home.append(el("p", { class: "sync-note", role: "status", "data-sync-note": "" }, [state.notice]));
    }
    home.append(el("p", { class: "setting-help", "data-sync-linked": "" }, ["Your pages stay on your phones and computers."]));
    const mark = setMark(state.account);
    if (mark) home.append(el("p", { class: "setting-help", "data-sync-set-mark": "" }, [`Set ${mark}`]));
    home.append(el("p", { class: "setting-help" }, ["A page you delete is in Recently deleted. Restore puts it back."]));
    if (state.saveError) {
      home.append(el("p", { class: "sync-err", role: "alert", "data-sync-error": "" }, [state.saveError]));
    }
    home.append(
      el("label", { class: "sync-field" }, ["Name for these pages"]),
      nameField("Name for these pages", state.vaultName, (name) => {
        const next = name || "My pages";
        if (next === state.vaultName) return;
        state.vaultName = next;
        state.vaultRev += 1;
        saveNames();
        showPlaces();
        if (state.member) {
          noteLocalEdit(state);
          armSaving();
          schedule();
        }
        draw();
      }),
    );
    const list = el("div", { class: "sync-devices" });
    const devices = [...state.devices].sort((a, b) => Number(!!b.self) - Number(!!a.self));
    if (!devices.length) list.append(el("p", { class: "setting-help" }, [hereCap()]));
    for (const device of devices) {
      const row = deviceLine(device, null);
      row.append(renameControl(row, device));
      list.append(row);
    }
    if (others) {
      list.append(el("p", { class: "setting-help" }, ["Remove stops that phone or computer from getting new changes. Pages it already saved stay readable on it. New edits stay with the phones and computers that are still linked. Rename changes the name on every phone and computer."]));
    }
    home.append(list);
    const actions = actionButton(state.busy ? "Working…" : "Add a phone or computer", "primary", () => startInvite(), state.busy);
    actions.append(quietLinks([
      ["Turn sync off", () => { state.screen = "off-ask"; state.error = ""; draw(); }, state.busy],
    ]));
    home.append(actions);
    panel.append(home);
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
    doc.title = "Recovery words";
    const heading = doc.createElement("h1");
    heading.textContent = "Recovery words";
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
    return new Promise((resolve) => {
      let settled = false;
      const finish = () => {
        if (settled) return;
        settled = true;
        resolve();
      };
      // WebKit can stop delivering frames after a resize. The paint still
      // happens when a frame arrives, and the check does not wait on one.
      requestAnimationFrame(() => requestAnimationFrame(finish));
      setTimeout(finish, 100);
    });
  }

  api.onEdit(() => {
    if (!state.account || !state.member) return;
    noteLocalEdit(state);
    armSaving();
    hurry = true;
    if (!ticking) nextPoll(PUSH_DEBOUNCE_MS);
  });

  if (typeof api.onDelete === "function") {
    api.onDelete((id) => {
      state.tombstones = noteTombstone(state.tombstones, id);
      if (!state.account || !state.member) return;
      noteLocalEdit(state);
      armSaving();
      schedule();
    });
  }
  if (typeof api.onRestore === "function") {
    api.onRestore((id) => {
      state.tombstones = (state.tombstones || []).filter((row) => row.id !== id);
      if (!state.account || !state.member) return;
      noteLocalEdit(state);
      armSaving();
      schedule();
    });
  }

  vaultBook = loadVaultBook(storeText);
  if (vaultBook.migrated) {
    try {
      await persistVaultBook();
    } catch {
      // The list is already in memory. A failed write still opens this set.
    }
  }
  loadNames();
  showPlaces();

  try {
    const vault = currentVault();
    const stored = vault?.deviceFile || "";
    if (stored) {
      const imported = JSON.parse(sync.importLocal(stored));
      if (imported.ok) {
        state.desk = imported.id;
        state.account = imported.account || vault.account;
        state.revision = Number(vault.revision || 0);
        state.slots = vault.slots || "[]";
        const storedBase = vault.base || "";
        if (storedBase) {
          try {
            const parsed = JSON.parse(storedBase);
            if (Array.isArray(parsed)) state.basePages = parsed;
          } catch {
            state.basePages = null;
          }
        }
        state.screen = "linked";
        state.member = false;
        await refreshDevices().catch(() => {});
        if (state.removed) {
          state.screen = "removed";
          markPaused();
          setChip("Sync paused", "off");
        } else if (state.member) {
          calm();
        } else {
          hideChip();
        }
      } else if (pausedFlag()) {
        state.member = false;
        setChip("Sync paused", "off");
      } else {
        hideChip();
      }
    } else if (pausedFlag()) {
      state.member = false;
      setChip("Sync paused", "off");
    } else {
      hideChip();
    }
  } finally {
    chip.dataset.ready = "1";
  }
  globalThis.omacusOpenSyncLink = () => applySyncLink();
  const opened = takeSyncLink();
  const wanted = new URLSearchParams(location.search).get("sync");
  if (!opened && !state.account && wanted === "1") state.screen = "intro";
  if (!opened) draw();
  if (!opened && !state.account && (wanted === "1" || wanted === "setup")) {
    api.openSettings("sync");
    const url = new URL(location.href);
    url.searchParams.delete("sync");
    history.replaceState(null, "", `${url.pathname}${url.search}${url.hash}`);
  }
  if (opened) await applySyncLink(opened);
  runPoll();
  window.addEventListener("online", () => {
    state.liveWait = 1000;
    openLive();
    tick();
  });
  window.addEventListener("focus", () => {
    if (state.account) schedule();
  });
  window.addEventListener("offline", () => {
    if (!state.account) return;
    saveWanted = false;
    clearTimeout(saveTimer);
    saveTimer = null;
    setChip("Offline", "offline");
  });
  if (inviteChannel) inviteChannel.onmessage = (event) => applySharedRequest(event.data);
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
      if (inviteLive()) restartInviteWatch();
      if (!state.account || ticking) return;
      nextPoll(pollWait({ hidden: true }));
      return;
    }
    if (inviteLive()) {
      requestsChecked = 0;
      inviteChecked = 0;
      inviteBackoff = 0;
      clearTimeout(inviteWatch);
      inviteWatch = 0;
      runInviteWatch();
      if (!ticking) schedule();
      return;
    }
    if (!state.account || ticking) return;
    schedule();
  });
  const settingsRoot = document.querySelector("[data-settings]");
  let syncShown = false;
  function syncOpen() {
    return !panel.hidden && !!settingsRoot && !settingsRoot.hidden;
  }
  function leaveJoined() {
    if (state.screen !== "joined") return;
    state.screen = "linked";
    state.error = "";
    tick();
  }
  function onSyncVisibility() {
    const open = syncOpen();
    if (syncShown && !open) leaveJoined();
    if (!syncShown && open) draw();
    syncShown = open;
  }
  syncShown = syncOpen();
  const watch = new MutationObserver(onSyncVisibility);
  if (settingsRoot) watch.observe(settingsRoot, { attributes: true, attributeFilter: ["hidden"] });
  watch.observe(panel, { attributes: true, attributeFilter: ["hidden"] });
  document.addEventListener("click", (event) => {
    if (event.target.closest?.("[data-settings-tab='sync']")) draw();
  });
  window.addEventListener("resize", () => {
    showPlaces();
    if (panel.hidden || state.screen === "join-scan" || state.video) return;
    draw();
  });

  const card = new URLSearchParams(location.search).get("syncCard");
  if (card) showSyncCard(card);

  function showSyncCard(name) {
    state.error = "";
    state.busy = false;
    state.words = [];
    state.code = "";
    state.phrase = "";
    state.joinCode = "";
    state.accessCode = "";
    state.joinWords = "";
    state.pending = null;
    state.listNote = "";
    state.removed = false;
    state.member = false;
    state.account = "";
    state.devices = [];
    state.removing = null;
    state.vaultName = "My pages";
    state.selfName = "Harbor desk";
    const linked = () => {
      state.account = "harbor";
      state.member = true;
      state.devices = [
        { id: "desk", name: "Harbor desk", self: true },
        { id: "phone", name: "Harbor phone", self: false },
      ];
    };
    if (name === "choice") state.screen = "off";
    else if (name === "fresh") state.screen = "access";
    else if (name === "join") state.screen = "join-type";
    else if (name === "join-wait") state.screen = "join";
    else if (name === "approve" || name === "approve-timeout") {
      linked();
      state.screen = "approve";
      state.pending = { name: "Harbor phone", code: "" };
      if (name === "approve-timeout") state.error = "That took too long. Your pages are still here.";
    } else if (name === "devices") {
      linked();
      state.screen = "linked";
    } else if (name === "remove") {
      linked();
      state.screen = "remove-ask";
      state.removing = { id: "phone", name: "Harbor phone", self: false };
    } else if (name === "paused") {
      state.account = "harbor";
      state.member = false;
      state.removed = true;
      state.screen = "removed";
    } else if (name === "list-changed") {
      linked();
      state.screen = "linked";
      state.listNote = "Harbor phone is now on the list.";
    } else if (name === "off") {
      linked();
      state.screen = "off-ask";
    } else if (name === "recover") state.screen = "recover";
    else state.screen = "intro";
    api.openSettings("sync");
    draw();
    paintEntry();
  }
}

function cloudIcon(mode) {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  const add = (d, mark) => {
    const path = document.createElementNS(ns, "path");
    path.setAttribute("d", d);
    if (mark) path.setAttribute("class", "sync-mark");
    svg.append(path);
  };
  add("M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z");
  if (mode === "on") add("m9.2 13.1 1.9 1.9 3.9-4.1", true);
  else if (mode === "offline") add("M5 19 19 5", true);
  else if (mode === "off") add("M10.2 11.5v4M13.8 11.5v4", true);
  else if (mode === "ask" || mode === "list") add("M12 13.2h.01", true);
  else if (mode === "error") {
    add("M12 10.4v3.4", true);
    add("M12 16.6h.01", true);
  }
  return svg;
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

function qrIcon() {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "2");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  const add = (tag, attrs) => {
    const node = document.createElementNS(ns, tag);
    for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
    svg.append(node);
  };
  add("rect", { x: "3", y: "3", width: "7", height: "7", rx: "1" });
  add("rect", { x: "14", y: "3", width: "7", height: "7", rx: "1" });
  add("rect", { x: "3", y: "14", width: "7", height: "7", rx: "1" });
  add("path", { d: "M14 14h3v3h-3z" });
  add("path", { d: "M18 18h3v3h-3z" });
  add("path", { d: "M14 21h.01" });
  add("path", { d: "M21 14h.01" });
  return svg;
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
  node.setAttribute("aria-label", label || "Recovery words");
  const d = document.createElementNS(ns, "path");
  d.setAttribute("fill", "currentColor");
  d.setAttribute("d", path[1]);
  node.append(d);
  return node;
}
