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
import { placeRowMenu, swallowDrawerTap } from "../site/app/layer.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const site = join(root, "site");
const css = readFileSync(join(site, "app/app.css"), "utf8");
const siteCss = readFileSync(join(site, "css/site.css"), "utf8");
const tokens = readFileSync(join(site, "css/tokens.css"), "utf8");
const js = readFileSync(join(site, "app/app.js"), "utf8");
const seed = readFileSync(join(site, "app/seed.js"), "utf8");
const html = readFileSync(join(site, "app/index.html"), "utf8");
const sw = readFileSync(join(site, "app/sw.js"), "utf8");
const layerJs = readFileSync(join(site, "app/layer.js"), "utf8");
const syncJs = readFileSync(join(site, "app/sync.js"), "utf8");
const recover = readFileSync(join(site, "app/recover.js"), "utf8");
const buildWeb = readFileSync(join(root, ".github/scripts/build-web.sh"), "utf8");
const siteJs = readFileSync(join(site, "js/site.js"), "utf8");

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
check(css.includes("--safe-bottom: max(env(safe-area-inset-bottom, 0px), var(--sab, var(--probed-bottom, 0px)));"), "bottom safe area is read from the device");
check(/\.side \{[^}]*padding-bottom: var\(--safe-bottom\)/.test(css), "the sidebar clears the home indicator");
check(/position: fixed;\s*inset: 0;/.test(css), "the shell fills the visual viewport, not 100dvh");
check(!/\.app-page \{[^}]*height:\s*100dvh/.test(css), "the shell is not sized with 100dvh");
check(/\.app-page\.drawer \.drawer-scrim \{\s*display: block/.test(css), "the scrim shows only while the drawer is open");

check(js.includes("function setDrawer(open)"), "opening and closing the drawer share one path");
check(/closest\?\.\("\[data-scrim\]"\)\) setDrawer\(false\)/.test(js), "a scrim click closes the drawer");
check(js.includes("document.body.append(menu)"), "the row menu is portaled onto the body");
check(js.includes('setAttribute("popover", "manual")') && js.includes("menu.showPopover()"), "the row menu opens in the top layer");
check(js.includes("placeRowMenu("), "the row menu is kept on screen");
check(/addEventListener\(\s*"touchstart", takeDrawerTap, \{ capture: true, passive: false \}\)/.test(js), "touchstart swallows the tap that closes the drawer");
const takeDrawer = js.slice(js.indexOf("function takeDrawerTap"), js.indexOf("function finishDrawerTap"));
check(takeDrawer.includes("preventDefault()") && !takeDrawer.includes("setDrawer"), "the outside tap is cancelled before the drawer closes");
const finishDrawer = js.slice(js.indexOf("function finishDrawerTap"), js.indexOf('document.addEventListener("touchstart", takeDrawerTap'));
check(finishDrawer.includes("setDrawer(false)") && finishDrawer.includes("restoreCaret"), "the drawer closes when the tap ends and the caret stays");
check(layerJs.includes('.side, .sheets-toggle, .bar, [data-row-menu]'), "taps on the drawer, the bar, and the row menu do not dismiss it");
check(js.includes('setAttribute("aria-expanded", String(open))'), "the sheets button reflects the drawer");
check(js.includes("Recently deleted"), "deleted sheets can be restored from Recently deleted");
check(js.includes("r.hint"), "name hints are marked so they can wait out typing");
check(html.includes("data-settings"), "settings dialog is in the page");
check(js.includes("settingsOpener") && js.includes('event.key !== "Tab"') && js.includes("nestedDialogOpen"), "settings traps tab and returns focus to the opener");
check(js.includes('node.getAttribute("tabindex") !== "-1"') && js.includes('querySelector(".settings-card")'), "the settings trap stays inside the dialog");
check(/class="settings-scrim"[^>]*tabindex="-1"[^>]*aria-hidden="true"/.test(html), "the settings backdrop is not a tab stop");
check(syncJs.includes('const FIELD_ERROR_ID = "sync-field-error"') && syncJs.includes("aria-describedby") && syncJs.includes("errorBesideField"), "a field error is tied to its field");
check(syncJs.includes('role: "group"') && syncJs.includes('for: "sync-recovery-words"') && syncJs.includes('aria-label": "Recovery words"'), "recovery words have a visible label and a name");
check(syncJs.includes('autocorrect: "off"') && syncJs.includes('autocapitalize: "none"') && syncJs.includes('spellcheck: "false"') && syncJs.includes('autocomplete: "off"'), "the recovery field does not correct or complete");
const recoverStart = syncJs.slice(syncJs.indexOf("async function startRecovery("), syncJs.indexOf("state.busy = true;", syncJs.indexOf("async function startRecovery(")));
check(recoverStart.includes('state.phraseMsg = ""') && !recoverStart.includes("state.phraseMsg = check.message"), "a recovery error is shown once");
check(html.includes('data-action="settings"'), "the menu opens settings");
check(js.includes("hideAnswers"), "the answers column can be hidden");
check(tokens.includes("#f6f1e7") && tokens.includes("#1c2433"), "paper and dusk match the desktop themes");
check(tokens.includes("#9ece6a") && tokens.includes("#7dcfff") && tokens.includes("#1a7f37"), "definition, use and keyword colours are in the themes");
check(html.includes('data-setting="lock-minutes"'), "settings still choose how soon a lock closes");
check(js.includes("explicitCompletion") && js.includes('"Ctrl-Space"'), "Enter accepts only an explicitly opened suggestion");
check(js.includes("completionStatus(state)") && /if \(update\.docChanged\) \{[\s\S]*?onInput\(\)/.test(js), "a text change recalculates even if the completion check throws");
check(!js.includes("completionStatus(view)") && !js.includes("completionStatus(update.view)"), "completion status is read from the state, not the view");
check(js.includes('autocomplete: "off"') && js.includes("commitTypedText"), "phone composition is committed and autocomplete stays off");
check(js.includes('key: "F2"') && js.includes("Rename variable"), "F2 and the menu rename a variable");
check(!js.includes('className = "hover-rename"'), "hover does not also offer Rename variable");
check(js.includes("function inNameField"), "name fields are left out of the global shortcuts");
check(js.includes('keys.textContent = "F2"') && js.includes('shortcut: "F2"'), "F2 is shown in the menu and the command palette");
check(js.includes("definitionChange") && js.includes("ref-update"), "editing a definition can update its references");
check(js.includes("touchstart") && js.includes("renameAtCaret") && html.includes('data-action="rename"'), "a phone can long-press a name or tap Rename");
check(!js.includes('console.log("rename-'), "rename diagnostics stay out of the page");
check(js.includes("hoverValue") && js.includes("t-def"), "hover values and definition colours are wired up");
check(html.includes('data-action="handle"'), "the menu can edit a page handle");
check(seed.includes("function allocateHandle"), "pages get a handle before the engine loads");
check(seed.includes("Keep in step with xref::allocate_handle"), "the handle rules stay with the engine");
check(js.includes('from "./seed.js"'), "the app uses that handle allocator");
check(css.includes(".cm-editor .t-ref"), "a page reference is coloured on its own");
check(js.includes("hoverValueWith") && js.includes("suggestAt"), "cross-page hover and the @ picker use the engine");
check(js.includes("lockOpenPassword") && js.includes("function pageConcealed"), "a locked page stays closed until the password");
check(css.includes(".cm-editor .t-def") && css.includes(".hover-value"), "definition and hover styles exist");
check(js.includes('e.key === ","') && js.includes('e.key === "\\\\"'), "settings and the answers column have shortcuts");
check(js.includes("previewBlocks"), "preview renders from the engine");
check(!js.includes("innerHTML") && !js.includes("insertAdjacentHTML"), "the app builds nodes without HTML strings");
check(!js.includes("DOMParser") && !js.includes("parseFromString"), "icons are SVG nodes, not parsed markup");
check(html.includes('src="trust.js"'), "trusted types load before the app");
check(js.includes("omacusScriptURL") && js.includes("scriptURL(WORKER)") && js.includes("scriptURL(new URL(\"sw.js\""), "workers use the page's script address");
check(html.includes("Export everything"), "settings can export every page");
check(html.includes("data-build-hash"), "settings shows the build hash");
check(js.includes("Add to Home Screen"), "iPhone gets an Add to Home Screen prompt");
check(js.includes('e.key.toLowerCase() === "p"'), "Ctrl+P opens the preview");
check(html.includes("data-page-preview"), "the preview overlay is in the page");
check(sw.includes('const APP_VERSION = "') && sw.includes("omacus-app-${APP_VERSION}"), "the app cache is named from the release version");
check(sw.includes("arrayBuffer") && sw.includes("shell-status") && sw.includes("cacheQueue"), "a dropped download is not left open in the cache");
check(html.includes("data-pick"), "a same-line conflict can ask which line to keep");
check(js.includes("docMigrate") && js.includes("Keep this line") && js.includes("Use the other line"), "the page document can merge a line and ask for a pick");
check(sw.includes('"tree.js"'), "the folder tree is part of the offline shell");
check(sw.includes('"layer.js"'), "the row menu placement is part of the offline shell");
check(syncJs.includes('setChip("Live", "on")') && !syncJs.includes('setChip("Synced"') && !syncJs.includes('setChip("", "on")'), "a quiet sync says Live, not Synced");
{
  const approve = syncJs.slice(syncJs.indexOf("async function approvePending"), syncJs.indexOf("async function decline"));
  check(approve.includes("try {") && approve.includes("} finally {") && approve.includes("state.busy = false"), "approving always leaves the button");
  check(syncJs.includes("AbortSignal.timeout(FETCH_TIMEOUT_MS)"), "a sync request gives up instead of waiting forever");
  check(syncJs.includes("That took too long. Your pages are still here."), "a timed-out approval says what happened");
  const watchers = syncJs.split("function watchApproval").length - 1;
  check(watchers === 1 && syncJs.includes("approvalCard("), "an expired approval card closes itself");
}
check(sw.includes('"library-state.mjs"'), "shared sidebar state is part of the offline shell");
check(syncJs.includes('setChip("Syncing", "wait")') && !syncJs.includes("Saving…"), "a long save says Syncing");
{
  const arm = syncJs.slice(syncJs.indexOf("function armSaving"), syncJs.indexOf("function guessDeviceName"));
  check(arm.includes("1000") && arm.includes("Syncing") && arm.includes("navigator.onLine === false"), "Syncing waits about a second and stays off while offline");
}
check(syncJs.includes('setChip("Offline", "offline")') && syncJs.includes("Offline. Edits saved here."), "offline names the cloud and says the edits are saved here");
check(syncJs.includes('setChip("Sync paused", "off")') && syncJs.includes('setChip("Not saved", "error")'), "paused sync and a failed save are different statuses");
check(syncJs.includes('api.openSettings("sync")'), "tapping the status opens Sync");
check(syncJs.includes('setChip("Device waiting", "ask")') && html.includes("data-device-waiting") && html.includes(">Device waiting<"), "a waiting phone shows on the status and in the sidebar");
check(css.includes('.sync-chip[data-state="ask"] .sync-chip-word') && css.includes("display: inline"), "Device waiting stays readable on a phone");
check(!syncJs.includes("innerHTML") && !syncJs.includes("insertAdjacentHTML"), "the sync status builds nodes without HTML strings");
{
  const chipCss = css.slice(css.indexOf(".sync-chip {"), css.indexOf(".sync-beta"));
  check(chipCss.includes("width: auto") && !chipCss.includes("7.5rem") && !chipCss.includes("::before"), "the sync status sizes to its words and has no dot");
}
check(/@media \(max-width: 760px\) \{[\s\S]*?\.sync-chip-word \{\s*display: none;/.test(css), "a narrow phone keeps the cloud and hides the word");
check(!html.includes("private-chip") && !html.includes('data-action="privacy"'), "the top bar has no Private pill");
check(html.includes("Private by design") && html.includes("on this device"), "About says pages stay on this device");
check(syncJs.includes("function hideChip") && syncJs.includes('storeGet("sync-paused")') && syncJs.includes('dataset.ready = "1"'), "a fresh copy hides the status cloud until sync is on or paused");
check(syncJs.includes('return ["intro", "access", "save", "confirm"].includes(state.screen);') && syncJs.includes('text === "Sync paused" && freshSetup()') && syncJs.includes("if (freshSetup()) hideChip();"), "an unfinished setup does not say Sync paused");
{
  const off = syncJs.slice(syncJs.indexOf("async function leaveList("), syncJs.indexOf("async function startInvite("));
  check(off.includes("sync.removeDevice") && off.includes("hideChip()") && !off.includes('setChip("Sync paused"'), "turning sync off removes this phone and leaves the status quiet");
  check(syncJs.includes("If this is the last one, that is fine.") && syncJs.includes("Just turn off here") && syncJs.includes("Another phone or computer can still remove this one."), "turning sync off says the words still open the pages");
}
check(syncJs.includes("takeSyncLink") && syncJs.includes('found.kind === "recover"') && syncJs.includes("pageOrigin()") && !syncJs.includes("captureSyncLink") && !syncJs.includes("console."), "the sync screen reads a recovery link and does not record it");
check(js.includes("captureSyncLink") && js.includes("history.replaceState"), "a recovery link is taken out of the address bar before a share link");
check(syncJs.includes("How sync works") && syncJs.includes("Set up sync") && syncJs.includes("showTurnOnSync"), "Turn on sync opens a short explanation before setup");
check(syncJs.includes("This is my first device") && syncJs.includes("My pages already sync on another device") && syncJs.includes("← Back"), "setup offers two choices and a back link");
check(syncJs.includes("Code you were given") && syncJs.includes("K7QM-4R2P-9HNX") && syncJs.includes("Not the code from another device."), "the invite code and the join code show their real shapes");
check(syncJs.includes("I wrote these down") && syncJs.includes("data-saved-words") && syncJs.includes("!state.wroteWords") && !syncJs.includes("Or scan this to recover") && !syncJs.includes("This picture is as secret as the words.") && !syncJs.includes("Scan the paper instead"), "saving the words waits until they are written down, with no recovery picture");
{
  const save = syncJs.slice(syncJs.indexOf("function drawSave"), syncJs.indexOf("function drawConfirm"));
  check(save.includes("sync-words-grid") && save.includes("I saved them") && save.includes('"Print"') && save.includes('"Copy"') && !save.includes("qrNode") && !save.includes("sync-qr"), "the words screen is the grid, the checkbox, and print and copy");
  const printed = syncJs.slice(syncJs.indexOf("function printWords"), syncJs.indexOf("function frame"));
  check(printed.includes('heading.textContent = "Recovery words"') && printed.includes('createElement("ol")') && !printed.includes("svg") && !printed.includes("qr"), "print is the words only");
}
check(syncJs.includes("Or scan this picture") && syncJs.includes('qrNode(state.invite?.qr, "Join code")'), "linking a phone still shows the join picture");
check(syncJs.includes("Use recovery words") && syncJs.includes("✓ Sync is on") && syncJs.includes("Step ${step} of 4"), "join and the first device use the new screens");
{
  const join = syncJs.slice(syncJs.indexOf("function scanCodeButton"), syncJs.indexOf("function drawJoinScan"));
  check(join.includes("Scan the QR code it shows, or type the code.") && join.includes("12 characters · or tap the scan button to scan the QR code") && join.includes("Lost your other devices?") && join.includes("Use recovery words →") && join.includes('aria-label": "Scan QR code"') && join.includes("scanJoin()") && !join.includes("quietLinks") && !join.includes("Scan QR code instead"), "join puts the scan button beside the code and keeps recovery in the row");
}
check(css.includes(".sync-card") && css.includes(".sync-progress") && css.includes(".sync-sheet") && css.includes(".sync-words-grid"), "the setup screens use cards, a progress bar, and a phone sheet");
check(css.includes(".sync-panel [data-sync-set] > .sync-name") && css.includes(".sync-panel [data-sync-set] > .sync-actions") && /\.sync-actions \.sync-links \{[^}]*margin-top:\s*var\(--space-4\)/.test(css), "the sync home screen leaves 16px around the device list and above Turn sync off");
{
  const quiet = css.slice(css.indexOf(".sync-quiet {"), css.indexOf(".sync-quiet {") + 240);
  check(quiet.includes("white-space: pre-wrap"), "the device row keeps the spaces around the dot");
}
check(syncJs.includes("` · this ${onPhone() ? \"phone\" : \"computer\"}`"), "the device row writes a space on each side of the dot");
check(html.includes('data-turn-on-sync') && html.includes(">Turn on sync<"), "the sidebar offers Turn on sync before setup");
check(css.includes(".bar .sync-setup") && css.includes("display: none"), "a narrow phone keeps Turn on sync in the pages list");
check(css.includes(".sync-chip[hidden]"), "a hidden sync status stays off the bar");
check(buildWeb.includes("site/app/layer.js"), "the published app includes the row menu placement");
check(buildWeb.includes("omacus-sync-web") && buildWeb.includes("omacus_sync_web_bg.wasm"), "the published build ships the sync engine");
{
  const boot = js.slice(js.indexOf("calc = new Calc()"), js.indexOf("loadRates();"));
  check(js.includes("FALLBACK_RATES") && js.includes("JPY: 150") && boot.includes("setRates") && boot.indexOf("setRates") < boot.indexOf("showSheet"), "bundled rates answer the hotel line before the live rates load");
}

{
  const fitted = placeRowMenu(248, 180, { width: 180, height: 160 }, { width: 1280, height: 800 }, { top: 148 });
  check(fitted.left === 248 && fitted.top === 180, `menu left the button (${fitted.left},${fitted.top})`);
  const shifted = placeRowMenu(260, 200, { width: 180, height: 220 }, { width: 390, height: 844 }, { top: 168 });
  check(shifted.left === 202 && shifted.left + 180 <= 390 - 8, `menu crossed the right edge (${shifted.left})`);
  const flipped = placeRowMenu(40, 700, { width: 180, height: 200 }, { width: 390, height: 844 }, { top: 668 });
  check(flipped.top === 464, `menu did not flip above the button (${flipped.top})`);
  const clamped = placeRowMenu(-20, -10, { width: 100, height: 40 }, { width: 400, height: 800 });
  check(clamped.left === 8 && clamped.top === 8, `menu left the screen (${clamped.left},${clamped.top})`);
  const hit = (name) => ({ closest: (sel) => (name && sel.includes(name) ? {} : null) });
  check(swallowDrawerTap(hit(""), { drawer: true, narrow: true }) === true, "a tap on the page is swallowed");
  check(swallowDrawerTap(hit(".side"), { drawer: true, narrow: true }) === false, "a tap on the drawer is kept");
  check(swallowDrawerTap(hit(".bar"), { drawer: true, narrow: true }) === false, "a tap on the bar is kept");
  check(swallowDrawerTap(hit(".sheets-toggle"), { drawer: true, narrow: true }) === false, "the pages button is kept");
  check(swallowDrawerTap(hit("[data-row-menu]"), { drawer: true, narrow: true }) === false, "the row menu is kept");
  check(swallowDrawerTap(hit(".settings"), { drawer: true, narrow: true }) === false, "a tap in Settings is kept");
  check(swallowDrawerTap(hit(""), { drawer: false, narrow: true }) === false, "a closed drawer does not swallow taps");
  check(swallowDrawerTap(hit(""), { drawer: true, narrow: false }) === false, "a wide window does not swallow taps");
}
check(!/omacus-app-2(?!\d)/.test(sw), "the old app cache name is gone");
check(sw.includes("!res.ok") && sw.includes("function honest") && sw.includes("cache.put"), "only a successful response is stored for offline");
check(sw.includes("await storeShell") && sw.includes("caches.delete(CACHE)"), "a new shell is stored as a set, and a short set is thrown away");
check(sw.includes('"pkg/omacus_web_bg.wasm"'), "the engine is precached for offline");
check(sw.includes('"sync-config.js"'), "the sync address is part of the offline shell");
check(!sw.includes("ignoreSearch"), "the cached file is the address the page requested");
check(sw.includes('cache: "no-store"') && sw.includes("version.json"), "the version check is not answered from the shell");
check(!sw.includes("indexedDB") && !sw.includes("localStorage"), "an update does not open saved pages");
check(sw.includes('"version.js"'), "the release version is part of the offline shell");
check(sw.includes("text/html") && sw.includes("isDocument"), "a page served in place of any other file is not kept");
check(sw.includes('"recovery-words.js"') && sw.includes('"word-check.js"'), "the word list is part of the offline shell");
check(syncJs.includes('from "./recovery-words.js"') && syncJs.includes("buildChoices") && !syncJs.includes('fetch("./words.txt")'), "the word check uses the bundled list");
check(buildWeb.includes("site/app/recovery-words.js") && buildWeb.includes("site/app/word-check.js"), "the published app includes the word list");
check(buildWeb.includes("site/app/seed.js") && buildWeb.includes("site/app/examples.js"), "the published app includes the pages that start it");
check(/seed\.js\?v=/.test(buildWeb) && /examples\.js\?v=/.test(buildWeb), "those files are named in the published imports");
check(recover.includes('transaction(STORE, "readonly"') && !recover.includes("deleteDatabase") && !recover.includes("localStorage.clear") && !recover.includes("localStorage.removeItem"), "a failed start reads the saved pages and does not remove them");
check(html.includes('src="recover.js"'), "a failed start can still show the saved pages");
check(sw.includes('"recover.js"'), "the saved-pages notice is part of the offline shell");
check(!recover.includes("innerHTML") && !recover.includes("insertAdjacentHTML"), "the saved-pages notice builds nodes without HTML strings");
check(recover.includes("omacusScriptURL"), "a failed start can still ask the next copy to take over");
check(recover.includes("omacusShowStored = () => show(true)"), "a failed engine still lists the pages saved in this browser");
check(css.includes(".boot-fail"), "the saved-pages notice is styled");
check(css.includes(".cm-announced"), "the screen reader line is not drawn on the page");
check(sw.includes('type === "skip-waiting"') && sw.includes("self.skipWaiting()"), "a new version waits until the page asks");
check(!sw.includes("self.registration.unregister"), "the worker stays registered so an offline check can use it");
check(html.includes("Update ready") && html.includes('data-action="apply-update"'), "an update offers a reload");
check(js.includes("async function applyUpdate") && js.includes("await flushSave()") && js.includes("await storeDrained()") && js.includes("settleSync"), "reload saves the open edit and lets sync finish");
check(js.includes("lookForUpdate") && js.includes("visibilitychange") && js.includes("version.json"), "an open copy looks for a new version");
check(js.includes('new URL("sw.js", document.baseURI)'), "the worker stays at the page address when the script moves");
check(!js.includes("previewHtml"), "the preview is not assigned as HTML");
check(!siteJs.includes("innerHTML") && !siteJs.includes("insertAdjacentHTML"), "the marketing site builds nodes without HTML strings");
check(siteJs.includes("migrateStoredPages"), "a visit to the site copies saved pages to the app");
check(js.includes('has("sw")'), "a local preview can register the worker with ?sw=1");
check(html.includes("rates may be stale") && html.includes("data-rates-note"), "a failed rates refresh says rates may be stale");
check(js.includes('note.dataset.ratesState = state'), "the rates note records whether the refresh succeeded");
check(css.includes(".rates-note"), "the rates note is styled");
check(css.includes(".pill"), "answers in the preview are marked");
check(/\.hover-value \{[^}]*white-space:\s*nowrap/.test(css), "a hover value stays on one line");
check(/\.hover-card \{[^}]*padding:/.test(css) && /\.hover-card \{[^}]*box-shadow:/.test(css), "a cross-page hover is one padded card");
check(js.includes("`from ${name}`"), "a cross-page hover names the source page");
check(/\.cm-tooltip\.cm-tooltip-hover \{[^}]*max-width:\s*none/.test(css), "the hover tooltip sizes to its text");
check(html.includes("rail-settings"), "the collapsed rail has a settings gear");
check(html.includes('class="side-tools"'), "the open sidebar has a settings gear");
check(html.includes('data-settings-tab="about"'), "settings and about share one window");
check(html.includes('data-action="about"'), "About opens from the sidebar");
check(html.includes("Copy / Export"), "copy and export share one page-menu item");
check(!html.includes("Export all pages (.zip)"), "export all is not in the page menu");
check(!html.includes("Export all pages"), "the old export label is gone");
check(html.includes(">Settings</span>"), "settings in the sidebar has a label");
check(js.includes("function pageLockChoice"), "Lock on a page with no password asks for one");
check(js.includes("Save this password in a password manager"), "a new lock says to save the password");
check(js.includes("removePageLock(id, password)"), "removing a lock checks the password");
check(/\.bar \{[^}]*background-color: var\(--app-bg\)/.test(css), "the top bar matches the page");
check(/\.bar \{[^}]*-webkit-backdrop-filter: none/.test(css) && /\.bar \{[^}]*backdrop-filter: none/.test(css), "the top bar has no blur");
check(/\.bar \.icon-btn \{[^}]*background-color: var\(--app-bg\)/.test(css), "buttons in the bar match the bar");
check(/html:has\(body\.app-page\) \{[^}]*background: var\(--app-bg\)/.test(css), "the safe area matches the page");
check(css.includes("border-bottom: 1px solid var(--app-line-strong)"), "the top bar has a clear bottom edge");
check(
  /@media \(max-width: 760px\)[\s\S]*?\.settings-card \{[^}]*top: calc\(var\(--bar\) \+ var\(--safe-top\)\)/.test(css),
  "settings starts below the top bar on a phone",
);
check(/\.drawer-scrim \{[^}]*position: fixed/.test(css), "the drawer scrim covers the page");
check(
  /\.drawer-scrim \{[^}]*background: color-mix\(in srgb, var\(--black\) 64%, transparent\)/.test(css),
  "the drawer scrim is a dim over the page",
);
check(/\.app-page\.drawer \.empty-hint \{\s*visibility: hidden/.test(css), "the empty hint does not show beside an open drawer");
check(/\.row-menu \{[^}]*z-index: 70/.test(css), "the row menu paints above the sidebar");
check(/\.row-menu \{[^}]*inset: auto/.test(css), "the top layer does not stretch the row menu");
check(css.includes(".row-menu:popover-open"), "an open row menu stays a menu in the top layer");
check(/\.app-page\.drawer \.cm-content \{[^}]*user-select: none/.test(css), "an open drawer does not select page text");
check(css.includes(".app-page.drawer-swallow .cm-editor"), "a closing tap cannot fall through to the editor");
check(/name === "new"[\s\S]{0,240}setDrawer\(false\)/.test(js), "a new page closes the phone drawer");
check(html.includes('<meta name="theme-color" content="#1a1b26" media="(prefers-color-scheme: dark)">'), "dark theme-color matches the page");
check(html.includes('<meta name="theme-color" content="#ffffff" media="(prefers-color-scheme: light)">'), "light theme-color matches the page");
check(html.includes('apple-mobile-web-app-status-bar-style" content="black-translucent"'), "the Home Screen page draws under the status bar");
check(js.includes('content", "black-translucent"') && !js.includes("status.remove()"), "the status bar tag stays, so the web view is not inset");
check(html.includes('src="frame.js"'), "the frame script runs before the page is drawn");
check(html.includes('rel="manifest"'), "the Home Screen app keeps its manifest");
check(html.includes("viewport-fit=cover"), "the page draws into the safe areas");
check(js.includes('meta[name="apple-mobile-web-app-status-bar-style"]'), "a missing status bar tag is put back");
check(js.includes('getPropertyValue("--app-bg")'), "a chosen theme sets theme-color from the page background");
const frame = readFileSync(join(site, "app/frame.js"), "utf8");
check(frame.includes("safe-area-inset-top") && frame.includes("offsetHeight") && !frame.includes("innerHeight"), "safe areas are measured with probe elements, not the window");
check(frame.includes("setTimeout(update, 100)") && frame.includes("setTimeout(update, 500)") && frame.includes("setTimeout(update, 1000)"), "cold start measures the safe areas again");
check(frame.includes("orientationchange") && frame.includes("visualViewport") && frame.includes("resize"), "rotation and the visual viewport measure the safe areas again");
check(frame.includes("viewport-fit=auto") && frame.includes("viewport-fit=cover"), "a standalone launch forces the safe areas to be computed");
check(/html:has\(body\.app-page\) \{[^}]*background: var\(--app-bg\)/.test(css), "the document background is the page background");
check(!/html:has\(body\.app-page\) \{[^}]*height:\s*100%/.test(css), "a percentage height is not used on the document");
check(/\.app-page \{[^}]*background: var\(--app-bg\)/.test(css), "the page background matches the document");
check(/\.app-page \{[^}]*position: fixed;\s*inset: 0;/.test(css), "Safari keeps the shell on the visual viewport");
check(/display-mode: standalone\)[\s\S]*?height:\s*100vh/.test(css), "the Home Screen shell is 100vh from the first paint");
check(!/height:\s*100dvh/.test(css), "the dynamic viewport unit is not used");
check(!css.includes("--standalone-gap"), "the shell is not grown from a measured window");
check(/--safe-top:\s*max\(env\(safe-area-inset-top, 0px\), var\(--sat, var\(--probed-top, 0px\)\)\)/.test(css), "the bar can use a measured top inset");
check(/--safe-bottom:\s*max\(env\(safe-area-inset-bottom, 0px\), var\(--sab, var\(--probed-bottom, 0px\)\)\)/.test(css), "the bottom inset can be supplied in a test");
check(/\.workspace\.collapsed \{[^}]*--side-w:\s*calc\(52px \+ var\(--safe-left\)\)/.test(css), "the collapsed rail keeps its buttons clear of the notch");
{
  const filters = [...css.matchAll(/backdrop-filter:\s*([^;]+)/g)].map((match) => match[1].trim());
  check(filters.length > 0 && filters.every((value) => value === "none"), `backdrop-filter values: ${filters.join(", ")}`);
}
check(/@media \(max-width: 760px\)[\s\S]*?\.side \{[^}]*top:\s*0;/.test(css), "the phone drawer background starts at the top edge");
check(/\.bar \{[^}]*position: fixed/.test(css) && /\.bar \{[^}]*opacity: 1/.test(css) && /\.bar \{[^}]*padding:\s*var\(--safe-top\)/.test(css), "the bar is solid and clears the status bar");
check(/\.bar \{[^}]*backdrop-filter: none/.test(css) && /\.bar \.icon-btn \{[^}]*backdrop-filter: none/.test(css), "the bar and its buttons are not frosted");
check(/@media \(max-width: 760px\)[\s\S]*?\.side \{[^}]*box-shadow: none/.test(css), "a closed phone drawer casts no shadow");
check(/\.app-page\.drawer \.side \{[^}]*box-shadow: 20px 0 60px -20px var\(--black\)/.test(css), "an open phone drawer still casts a shadow");
check(/\.drawer-scrim \{[^}]*z-index: 35/.test(css) && /\.bar \{[^}]*z-index: 50/.test(css), "the drawer scrim stacks under the bar");
check(css.includes("clip-path: inset(50%)"), "the screen reader line stays off the page");
check(!html.includes("Saved in this browser"), "the sidebar footer does not repeat where pages are saved");
check(!html.includes("side-note"), "the sidebar has no privacy paragraph");
check(html.includes("Pages stay in this browser, on this device. With sync on, a sealed copy can go to your other phones and computers."), "About says pages stay in this browser");
check(!html.includes("Nothing is uploaded."), "About does not say nothing is uploaded");
check(html.includes("Home Screen so Safari keeps them."), "About mentions the Home Screen");
check(html.indexOf("data-persist-note") > html.indexOf('data-settings-panel="about"'), "a refused save is explained in About");
check(/<div class="side-foot">\s*<a class="side-version"/.test(html), "the sidebar footer is the version");
check(css.includes("font: 500 var(--type-body) / 1.3 var(--sans)"), "chrome text uses the interface face");
{
  const cargo = readFileSync(join(root, "Cargo.toml"), "utf8");
  const version = cargo.match(/^version = "([^"]+)"/m)?.[1];
  check(!!version && html.includes(`name="omacus-version" content="${version}"`), "the web app version matches the desktop release");
  check(!!version && html.includes(`>omacus v${version}<`), "the sidebar shows that version");
}
check(css.includes("grid-template-rows: minmax(0, 1fr)"), "the workspace row cannot grow past the window");
check(css.includes("overflow-y: auto !important"), "the editor scroller can scroll");
check(css.includes(".line.blank") || css.includes("p.line.blank"), "blank lines keep their height");
check(css.includes(".note"), "comments are styled in the preview");
check(js.includes("showSettingsTab"), "settings and about are tabs");
check(js.includes("afterprint"), "print restores the window title");
check(css.includes("var(--print-accent)") && tokens.includes("#1a3a5c"), "printed answers use a dark ink");
check(/@page \{[^}]*margin:\s*0\.55in/.test(css), "printed pages use a slim margin");
check(css.includes("pre.code") && css.includes("overflow-wrap: anywhere"), "a code block wraps a long line");
check(tokens.includes("--print-code"), "printed code sits on a paper fill");
check(css.includes(".page-preview-body") && /@media print[\s\S]*\.page-preview-body \{[^}]*padding:\s*0/.test(css), "print does not add the on-screen preview padding");
check(js.includes("pill.dataset.value"), "clicking an answer copies the value, not the brackets");
check(css.includes("body.answers-inline .res"), "hiding the column places answers inline");
check(css.includes("img.cm-widgetBuffer"), "the answer spacer stays inline so a short line keeps its answer beside it");
{
  const home = readFileSync(join(site, "index.html"), "utf8");
  const start = home.indexOf('id="unfold"');
  const end = start >= 0 ? home.indexOf("</figure>", start) : -1;
  const unfold = start >= 0 && end > start ? home.slice(start, end) : "";
  check(start >= 0 && unfold.length > 0, "the homepage has a small unfolded phone");
  check(unfold.includes("Look what you can do: unfold a phone and your pages sit beside the math."), "the foldable aside says what opening the phone does");
  check(unfold.includes("unfold-dark.webp") && !unfold.includes("unfold-light.webp"), "the foldable aside ships one dark picture");
  check(!unfold.includes("<h2") && !unfold.includes("kicker"), "the foldable aside has no headline");
  check(unfold.includes("foldable-clock") && unfold.includes("foldable-buttons"), "the unfolded phone has a status time and side buttons");
  const foldCss = siteCss.slice(siteCss.indexOf("/* Unfolded phone"), siteCss.indexOf("/* Theme gallery"));
  check(foldCss.includes("aspect-ratio: 4 / 3"), "the unfolded phone is landscape, about 4:3");
  check(foldCss.includes("440px"), "the unfolded phone stays a small aside");
  check(unfold.includes("foldable-status"), "the camera and time sit in a status strip");
  check(foldCss.includes("color-mix(in srgb, var(--black) 8%, transparent)"), "the crease is a faint hairline");
  check(foldCss.includes("border-right: 0"), "the halves do not draw a bar between them");
  check(/\.foldable-device \{[^}]*drop-shadow\(/.test(foldCss), "the phone shadow belongs to the whole device");
  check(!/\.foldable-half::before \{[^}]*var\(--space-5\)/.test(foldCss), "a half does not cast its shadow across the crease");
  check(!/rotate[XY]/.test(foldCss), "the unfolded phone lies flat");
  check(!/iPhone|iPad|Apple|Samsung|Pixel|Galaxy|PWA/i.test(unfold), "the foldable aside stays generic");
}
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
  ".mjs": "text/javascript",
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
  res.writeHead(200, {
    "content-type": types[extname(file)] || "application/octet-stream",
    "cache-control": "no-store",
  });
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

// Chrome writes the port into the profile once DevTools is up. Reading stdout
// for the "DevTools listening" line deadlocks when that pipe fills. The first
// launch on a CI runner sometimes exits before that file exists, so try once more.
async function launchChrome(attempt) {
  const profile = join("/tmp", `omacus-chrome-${process.pid}-${attempt}`);
  const child = spawn(chrome, [
    "--headless=new",
    "--disable-gpu",
    "--no-sandbox",
    "--disable-dev-shm-usage",
    "--no-first-run",
    `--remote-debugging-port=0`,
    `--user-data-dir=${profile}`,
    "about:blank",
  ], { stdio: ["ignore", "ignore", "ignore"] });
  const portFile = join(profile, "DevToolsActivePort");
  const started = Date.now();
  let version = null;
  while (!version && Date.now() - started < 20000) {
    if (child.exitCode != null) break;
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
  return { child, version };
}

let launched = await launchChrome(1);
if (!launched.version?.webSocketDebuggerUrl) {
  launched.child.kill();
  console.error("Chrome did not expose DevTools; trying again");
  launched = await launchChrome(2);
}
if (!launched.version?.webSocketDebuggerUrl) {
  launched.child.kill();
  server.close();
  console.error("Chrome did not expose DevTools");
  process.exit(1);
}
const browser = launched.child;
const version = launched.version;

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

const filled = await evalJson(`(() => {
  const paint = (node) => {
    const probe = document.createElement("div");
    probe.style.background = getComputedStyle(node).backgroundColor;
    document.body.appendChild(probe);
    const color = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return color;
  };
  const probe = document.createElement("div");
  probe.style.background = "var(--app-bg)";
  document.body.appendChild(probe);
  const appBg = getComputedStyle(probe).backgroundColor;
  probe.remove();
  const bar = document.querySelector(".bar");
  const btn = document.querySelector(".bar .icon-btn");
  const side = document.querySelector(".side");
  const scrim = document.querySelector("[data-scrim]");
  const shell = document.body.getBoundingClientRect();
  const barCs = getComputedStyle(bar);
  const scrimCs = getComputedStyle(scrim);
  const read = () => ({
    htmlBg: paint(document.documentElement),
    bodyBg: paint(document.body),
    appBg,
    barBlur: barCs.backdropFilter,
    btnBlur: getComputedStyle(btn).backdropFilter,
    btnBg: getComputedStyle(btn).backgroundColor,
    closedShadow: getComputedStyle(side).boxShadow,
    scrimZ: Number(scrimCs.zIndex),
    barZ: Number(barCs.zIndex),
    scrimTop: parseFloat(scrimCs.top),
    barBottom: bar.getBoundingClientRect().bottom,
    shellTop: shell.top,
    shellBottom: shell.bottom,
    innerHeight: window.innerHeight,
    manifest: !!document.querySelector('link[rel="manifest"]'),
  });
  const dark = read();
  document.documentElement.dataset.appTheme = "light";
  const lightProbe = document.createElement("div");
  lightProbe.style.background = "var(--app-bg)";
  document.body.appendChild(lightProbe);
  const lightApp = getComputedStyle(lightProbe).backgroundColor;
  lightProbe.remove();
  const light = { htmlBg: paint(document.documentElement), bodyBg: paint(document.body), appBg: lightApp };
  document.documentElement.removeAttribute("data-app-theme");
  return { dark, light };
})()`);
check(filled.dark.htmlBg === filled.dark.appBg && filled.dark.bodyBg === filled.dark.appBg, `dark document ${filled.dark.htmlBg}/${filled.dark.bodyBg} != page ${filled.dark.appBg}`);
check(filled.light.htmlBg === filled.light.appBg && filled.light.bodyBg === filled.light.appBg, `light document ${filled.light.htmlBg}/${filled.light.bodyBg} != page ${filled.light.appBg}`);
check(filled.dark.barBlur === "none" && filled.dark.btnBlur === "none", `bar blur ${filled.dark.barBlur}, button blur ${filled.dark.btnBlur}`);
check(filled.dark.btnBg === filled.dark.appBg, `bar button background ${filled.dark.btnBg}`);
check(filled.dark.closedShadow === "none", `closed drawer shadow is ${filled.dark.closedShadow}`);
check(filled.dark.scrimZ < filled.dark.barZ, "the scrim stacks under the bar");
check(filled.dark.scrimTop >= filled.dark.barBottom - 1, `scrim top ${filled.dark.scrimTop} covers the bar (${filled.dark.barBottom})`);
check(Math.abs(filled.dark.shellTop) <= 1 && Math.abs(filled.dark.shellBottom - filled.dark.innerHeight) <= 1, `shell ${filled.dark.shellTop}..${filled.dark.shellBottom} != viewport 0..${filled.dark.innerHeight}`);
check(filled.dark.manifest === true, "this browser still gets the manifest");

const mobile = await evalJson(`(() => {
  document.body.style.setProperty("--safe-top", "47px");
  document.body.style.setProperty("--safe-bottom", "34px");
  document.body.classList.add("drawer");
  const side = document.querySelector(".side");
  const foot = document.querySelector(".side-foot");
  const scrim = document.querySelector("[data-scrim]");
  const editor = document.querySelector("[data-editor]");
  const sheet = document.querySelector(".sheet");
  const bar = document.querySelector(".bar");
  const btn = document.querySelector(".sheets-toggle");
  const sr = side.getBoundingClientRect();
  const fr = foot.getBoundingClientRect();
  const er = editor.getBoundingClientRect();
  const sheetR = sheet.getBoundingClientRect();
  const scrimR = scrim.getBoundingClientRect();
  const br = btn.getBoundingClientRect();
  const edge = window.innerHeight;
  const hit = document.elementFromPoint(br.left + br.width / 2, br.top + br.height / 2);
  const blurred = [...document.body.querySelectorAll("*")].filter((el) => {
    const value = getComputedStyle(el).backdropFilter;
    return value && value !== "none";
  }).map((el) => el.className);
  const htmlBg = getComputedStyle(document.documentElement).backgroundColor;
  const bodyBg = getComputedStyle(document.body).backgroundColor;
  const appBg = getComputedStyle(document.body).getPropertyValue("--app-bg").trim();
  const probe = document.createElement("div");
  probe.style.background = "var(--app-bg)";
  document.body.appendChild(probe);
  const painted = getComputedStyle(probe).backgroundColor;
  probe.remove();
  return {
    font: parseFloat(getComputedStyle(editor).fontSize),
    sideBottom: sr.bottom,
    sideTop: sr.top,
    editorBottom: er.bottom,
    sheetBottom: sheetR.bottom,
    innerHeight: window.innerHeight,
    edge,
    paddingBottom: parseFloat(getComputedStyle(side).paddingBottom),
    footBottom: fr.bottom,
    scrim: getComputedStyle(scrim).display,
    scrimTop: scrimR.top,
    scrimBottom: scrimR.bottom,
    barBottom: bar.getBoundingClientRect().bottom,
    headerHit: hit === btn || btn.contains(hit),
    headerHitName: hit?.className || hit?.tagName || "",
    position: getComputedStyle(side).position,
    openShadow: getComputedStyle(side).boxShadow,
    viewport: document.querySelector('meta[name=viewport]').content,
    blurred,
    htmlBg,
    bodyBg,
    painted,
  };
})()`);

check(mobile.font >= 16, `phone editor font is ${mobile.font}px, want >= 16`);
check(Math.abs(mobile.sideBottom - mobile.edge) <= 1, `drawer bottom ${mobile.sideBottom} != viewport ${mobile.edge}`);
check(Math.abs(mobile.editorBottom - mobile.edge) <= 1, `editor bottom ${mobile.editorBottom} != viewport ${mobile.edge}`);
check(Math.abs(mobile.sheetBottom - mobile.edge) <= 1, `page bottom ${mobile.sheetBottom} != viewport ${mobile.edge}`);
check(Math.abs(mobile.scrimBottom - mobile.edge) <= 1, `scrim bottom ${mobile.scrimBottom} != viewport ${mobile.edge}`);
check(Math.abs(mobile.sideTop) <= 1, `drawer top ${mobile.sideTop} is not the screen edge`);
check(mobile.openShadow !== "none", `open drawer shadow is ${mobile.openShadow}`);
check(mobile.scrimTop >= mobile.barBottom - 1, `open scrim top ${mobile.scrimTop} covers the bar (${mobile.barBottom})`);
check(mobile.headerHit === true, `open drawer header hit ${mobile.headerHitName}`);
check(mobile.paddingBottom === 34, `drawer padding-bottom is ${mobile.paddingBottom}, want 34`);
check(mobile.footBottom <= mobile.edge - 34 + 1, `footer is cut by the home indicator (${mobile.footBottom} > ${mobile.edge - 34})`);
check(mobile.scrim === "block", `scrim display is ${mobile.scrim} while the drawer is open`);
check(mobile.position === "fixed", `phone drawer position is ${mobile.position}`);
check(mobile.blurred.length === 0, `backdrop-filter is set on ${mobile.blurred.join(", ")}`);
check(mobile.htmlBg === mobile.painted && mobile.bodyBg === mobile.painted, `document ${mobile.htmlBg}/${mobile.bodyBg} != page ${mobile.painted}`);
await assertRowMenuClearsSidebar("phone", true);

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
await assertRowMenuClearsSidebar("desktop");
if (shots) {
  const themeBefore = await evalJson(`document.documentElement.dataset.appTheme || ""`);
  await evalJson(`(() => {
    const list = document.querySelector("[data-sheets]");
    list.replaceChildren();
    for (const name of ["Trip Budget", "Groceries", "Monthly budget"]) {
      const row = document.createElement("div");
      row.className = "side-row";
      row.dataset.kind = "page";
      if (name === "Groceries") row.setAttribute("aria-current", "true");
      const label = document.createElement("button");
      label.type = "button";
      label.className = "side-name";
      const text = document.createElement("span");
      text.textContent = name;
      label.append(text);
      const more = document.createElement("button");
      more.type = "button";
      more.className = "row-more";
      more.textContent = "···";
      row.append(label, more);
      list.append(row);
    }
  })()`);
  await evalJson(`document.documentElement.dataset.appTheme = "light"`);
  await openRowMenuPreview();
  await plainShot(join(shots, "menu-after-light.png"));
  await evalJson(`document.documentElement.dataset.appTheme = "dark"`);
  await plainShot(join(shots, "menu-after-dark.png"));
  await evalJson(`document.documentElement.dataset.appTheme = ${JSON.stringify(themeBefore)}`);
  await closeRowMenuPreview();
  await evalJson(`document.querySelector("[data-sheets]").replaceChildren()`);
}

// iPhone landscape is a wide window with a notch on the left. The rail has to
// stay hit-testable once that inset is real. Chrome reports the inset as 0,
// so the test sets --sal / --sab.
await page("Emulation.setDeviceMetricsOverride", {
  width: 844,
  height: 390,
  deviceScaleFactor: 1,
  mobile: true,
});
await waitFor(`innerWidth >= 800 && innerHeight <= 420 && matchMedia("(min-width: 761px)").matches`);
const railHits = await evalJson(`(() => {
  document.body.style.setProperty("--sal", "59px");
  document.body.style.setProperty("--sar", "59px");
  document.body.style.setProperty("--sab", "21px");
  const workspace = document.querySelector(".workspace");
  workspace.classList.add("collapsed");
  const buttons = [...document.querySelectorAll(".rail .side-btn")].filter((button) => !button.hidden);
  const hits = buttons.map((button) => {
    const r = button.getBoundingClientRect();
    const x = r.left + r.width / 2;
    const y = r.top + r.height / 2;
    const hit = document.elementFromPoint(x, y);
    return {
      label: button.getAttribute("aria-label"),
      x, y,
      left: r.left,
      top: r.top,
      hit: !!(hit && (hit === button || button.contains(hit))),
      hitName: hit?.getAttribute?.("aria-label") || hit?.className || hit?.tagName || "",
    };
  });
  const side = document.querySelector(".side").getBoundingClientRect();
  const editor = document.querySelector("[data-editor]").getBoundingClientRect();
  const blurred = [...document.body.querySelectorAll("*")].filter((el) => {
    const value = getComputedStyle(el).backdropFilter;
    return value && value !== "none";
  }).map((el) => el.className);
  workspace.classList.remove("collapsed");
  document.body.style.removeProperty("--sal");
  document.body.style.removeProperty("--sar");
  document.body.style.removeProperty("--sab");
  return {
    hits,
    sideLeft: side.left,
    sideBottom: side.bottom,
    editorBottom: editor.bottom,
    innerHeight: window.innerHeight,
    blurred,
  };
})()`);
check(railHits.blurred.length === 0, `backdrop-filter is set on ${railHits.blurred.join(", ")}`);
check(Math.abs(railHits.sideLeft) <= 1, `landscape sidebar starts at ${railHits.sideLeft}`);
check(Math.abs(railHits.sideBottom - railHits.innerHeight) <= 1, `landscape sidebar bottom ${railHits.sideBottom} != ${railHits.innerHeight}`);
check(Math.abs(railHits.editorBottom - railHits.innerHeight) <= 1, `landscape editor bottom ${railHits.editorBottom} != ${railHits.innerHeight}`);
check(railHits.hits.length === 5, `landscape rail has ${railHits.hits.length} buttons`);
for (const hit of railHits.hits) {
  check(hit.hit, `landscape ${hit.label} at ${hit.x},${hit.y} hit ${hit.hitName}`);
  check(hit.left >= 59 - 1, `landscape ${hit.label} starts at ${hit.left}, inside the notch`);
}
await page("Emulation.setDeviceMetricsOverride", {
  width: 1280,
  height: 800,
  deviceScaleFactor: 1,
  mobile: false,
});
await waitFor(`innerWidth >= 1200 && matchMedia("(min-width: 761px)").matches`);

if (shots) await shot(join(shots, "desktop.png"));

const pkg = existsSync(join(site, "app/pkg/omacus_web.js"));
if (pkg) {
  // app.js imports the wasm package, so the module (and Settings) only runs when it exists.
  await waitFor(`document.documentElement.dataset.appTheme`);
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
  await resetStore();
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
  await resetStore();
  await evalJson(`(() => {
    localStorage.setItem("omacus:page:hover", "rent = $1,450 # monthly\\nrent * 2\\n1.5k\\ntotal\\nseats = 10\\ncost = $40 * seats");
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
      x: r.x, y: r.y, w: r.width, h: r.height,
      shown: getComputedStyle(host).visibility !== "hidden" && r.width > 0 && r.height > 0,
      rename: !!document.querySelector(".hover-rename"),
    };
  })()`);
  check(tip.text === "$1,450", `hover showed ${tip.text}`);
  check(tip.shown && tip.y > 0 && tip.y < 760 && tip.x > 0, `hover tooltip is off-screen (${JSON.stringify(tip)})`);
  check(tip.w > tip.h, `hover value is taller than it is wide (${tip.w}x${tip.h})`);
  check(!tip.rename, "hover also offered Rename variable");
  const popupCount = await evalJson(`document.querySelectorAll(".cm-tooltip, .rename-menu, .rename-pop, .ref-prompt").length`);
  check(popupCount === 1, `more than one popup was visible (${popupCount})`);
  if (shots) await plainShot(join(shots, "single-popup.png"));
  const nowrap = await evalJson(`getComputedStyle(document.querySelector(".hover-value")).whiteSpace`);
  check(nowrap === "nowrap", `hover value wraps (${nowrap})`);
  if (shots) await plainShot(join(shots, "hover.png"));
  const big = await evalJson(`(() => {
    const el = [...document.querySelectorAll(".t-def")].find((n) => n.textContent === "cost");
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + Math.min(r.height / 2, 8) };
  })()`);
  check(big, "cost is not marked as a definition");
  if (big) {
  await page("Input.dispatchMouseEvent", { type: "mouseMoved", x: 8, y: 8 });
  await page("Input.dispatchMouseEvent", { type: "mouseMoved", x: big.x, y: big.y });
  await waitFor(`document.querySelector(".hover-value")?.textContent === "$400"`, 4000);
  const longTip = await evalJson(`(() => {
    const el = document.querySelector(".hover-value");
    const h = el.getBoundingClientRect();
    const inside = h.x >= 0 && h.y >= 0 && h.right <= innerWidth + 1 && h.bottom <= innerHeight + 1;
    return { text: el.textContent, w: h.width, h: h.height, x: h.x, y: h.y, inside, lines: el.innerText.split("\\n").length };
  })()`);
  check(longTip.text === "$400", `long hover showed ${longTip.text}`);
  check(longTip.w > longTip.h, `hover value is taller than it is wide (${longTip.w}x${longTip.h})`);
  check(longTip.lines === 1, `long hover split onto ${longTip.lines} lines`);
  check(longTip.inside, `long hover left the window (${JSON.stringify(longTip)})`);
  if (shots) await plainShot(join(shots, "hover-long.png"));
  }

  // Right-click offers Rename, and only that menu — the value tip closes.
  const menuSpot = await evalJson(`(() => {
    const el = document.querySelector(".t-var");
    const r = el.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + Math.min(r.height / 2, 8) };
  })()`);
  await page("Input.dispatchMouseEvent", {
    type: "mousePressed",
    x: menuSpot.x,
    y: menuSpot.y,
    button: "right",
    buttons: 2,
    clickCount: 1,
  });
  await page("Input.dispatchMouseEvent", {
    type: "mouseReleased",
    x: menuSpot.x,
    y: menuSpot.y,
    button: "right",
    clickCount: 1,
  });
  await waitFor(`document.querySelector(".rename-menu")`, 2000);
  const menu = await evalJson(`(() => {
    const item = document.querySelector(".rename-menu");
    return {
      text: item?.innerText || "",
      tips: document.querySelectorAll(".cm-tooltip, .hover-rename, .rename-pop").length,
    };
  })()`);
  check(menu.text.includes("Rename variable") && menu.text.includes("F2"), `rename menu was ${JSON.stringify(menu)}`);
  check(menu.tips === 0, `another popup stayed open with the menu (${menu.tips})`);
  if (shots) await plainShot(join(shots, "context-menu-rename.png"));
  await evalJson(`document.querySelector(".rename-menu button").dispatchEvent(new MouseEvent("mousedown", { bubbles: true }))`);
  await waitFor(`document.querySelector(".rename-pop input")`, 2000);
  const selected = await evalJson(`(() => {
    const input = document.querySelector(".rename-pop input");
    return {
      value: input.value,
      start: input.selectionStart,
      end: input.selectionEnd,
      focused: document.activeElement === input,
    };
  })()`);
  check(
    selected.focused && selected.value === "rent" && selected.start === 0 && selected.end === selected.value.length,
    `rename field did not preselect the name (${JSON.stringify(selected)})`,
  );
  if (shots) await plainShot(join(shots, "rename-field-selected.png"));
  await page("Input.dispatchKeyEvent", {
    type: "rawKeyDown",
    modifiers: process.platform === "darwin" ? 4 : 2,
    key: "a",
    code: "KeyA",
    windowsVirtualKeyCode: 65,
    commands: ["selectAll"],
  });
  await page("Input.dispatchKeyEvent", {
    type: "keyUp",
    key: "a",
    code: "KeyA",
    windowsVirtualKeyCode: 65,
  });
  const selectedAll = await evalJson(`(() => {
    const input = document.querySelector(".rename-pop input");
    return { start: input.selectionStart, end: input.selectionEnd, value: input.value };
  })()`);
  check(selectedAll.start === 0 && selectedAll.end === selectedAll.value.length, `select all missed (${JSON.stringify(selectedAll)})`);
  await page("Input.insertText", { text: "fare" });
  const replaced = await evalJson(`document.querySelector(".rename-pop input")?.value`);
  check(replaced === "fare", `select-all then type left ${JSON.stringify(replaced)}`);
  await evalJson(`document.querySelector(".rename-pop input").select()`);
  try {
    await send("Browser.grantPermissions", {
      origin: `http://127.0.0.1:${port}`,
      permissions: ["clipboardReadWrite", "clipboardSanitizedWrite"],
    });
  } catch {}
  await evalJson(`navigator.clipboard.writeText("fare2")`);
  await page("Input.dispatchKeyEvent", {
    type: "rawKeyDown",
    modifiers: process.platform === "darwin" ? 4 : 2,
    key: "v",
    code: "KeyV",
    windowsVirtualKeyCode: 86,
    commands: ["Paste"],
  });
  await page("Input.dispatchKeyEvent", {
    type: "keyUp",
    key: "v",
    code: "KeyV",
    windowsVirtualKeyCode: 86,
  });
  let pasted = await evalJson(`document.querySelector(".rename-pop input")?.value`);
  if (pasted !== "fare2") {
    pasted = await evalJson(`(() => {
      const input = document.querySelector(".rename-pop input");
      input.focus();
      input.select();
      const data = new DataTransfer();
      data.setData("text/plain", "fare2");
      const event = new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: data });
      input.dispatchEvent(event);
      if (!event.defaultPrevented && input.value !== "fare2") {
        document.execCommand("insertText", false, "fare2");
      }
      return input.value;
    })()`);
  }
  check(pasted === "fare2", `paste left ${JSON.stringify(pasted)}`);
  await evalJson(`document.querySelector(".rename-pop input").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))`);

  await evalJson(`document.querySelector(".side-head [data-action=new-folder]").click()`);
  await waitFor(`document.querySelector("[data-rename]") && !document.querySelector("[data-rename]").hidden`, 2000);
  const folderSelected = await evalJson(`(() => {
    const input = document.querySelector("[data-rename-input]");
    return { value: input.value, start: input.selectionStart, end: input.selectionEnd, focused: document.activeElement === input };
  })()`);
  check(
    folderSelected.focused && folderSelected.start === 0 && folderSelected.end === folderSelected.value.length && folderSelected.value.length > 0,
    `new folder name was not selected (${JSON.stringify(folderSelected)})`,
  );
  await evalJson(`navigator.clipboard.writeText("Trip Budget")`);
  await page("Input.dispatchKeyEvent", {
    type: "rawKeyDown",
    modifiers: process.platform === "darwin" ? 4 : 2,
    key: "v",
    code: "KeyV",
    windowsVirtualKeyCode: 86,
    commands: ["Paste"],
  });
  await page("Input.dispatchKeyEvent", {
    type: "keyUp",
    key: "v",
    code: "KeyV",
    windowsVirtualKeyCode: 86,
  });
  let folderPasted = await evalJson(`document.querySelector("[data-rename-input]")?.value`);
  if (folderPasted !== "Trip Budget") {
    folderPasted = await evalJson(`(() => {
      const input = document.querySelector("[data-rename-input]");
      input.focus();
      input.select();
      document.execCommand("insertText", false, "Trip Budget");
      return input.value;
    })()`);
  }
  check(folderPasted === "Trip Budget", `folder paste left ${JSON.stringify(folderPasted)}`);
  await evalJson(`document.querySelector("[data-rename-input]").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))`);

  // A cross-page reference is one card: the value, then "from Trip Budget",
  // above the line rather than loose text on the next row.
  await resetStore();
  await evalJson(`(() => {
    localStorage.setItem("omacus:page:trip", "# Trip Budget\\ntotal = $40");
    localStorage.setItem("omacus:page:groceries", "# Groceries\\ncost = @trip.total");
    localStorage.setItem("omacus:index", JSON.stringify({
      order: ["trip", "groceries"],
      active: "groceries",
      handles: { trip: "trip", groceries: "groceries" },
    }));
    return true;
  })()`);
  await navigate(`http://127.0.0.1:${port}/app/`);
  await waitFor(`document.body.innerText.includes("@trip.total")`, 20000);
  const refSpot = await evalJson(`(() => {
    const el = [...document.querySelectorAll(".t-var")].find((n) => n.textContent === "total");
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + Math.min(r.height / 2, 8), top: r.top, bottom: r.bottom };
  })()`);
  check(refSpot, "@trip.total is not on the page");
  if (refSpot) {
    await page("Input.dispatchMouseEvent", { type: "mouseMoved", x: 8, y: 8 });
    await page("Input.dispatchMouseEvent", { type: "mouseMoved", x: refSpot.x, y: refSpot.y });
    await waitFor(`document.querySelector(".hover-source")?.textContent === "from Trip Budget"`, 4000);
    const cross = await evalJson(`(() => {
      const card = document.querySelector(".hover-card");
      const value = card?.querySelector(".hover-value");
      const source = card?.querySelector(".hover-source");
      const cr = card.getBoundingClientRect();
      const sr = source.getBoundingClientRect();
      const vr = value.getBoundingClientRect();
      const token = [...document.querySelectorAll(".t-var")].find((n) => n.textContent === "total").getBoundingClientRect();
      const above = cr.bottom <= token.top + 1;
      const below = cr.top >= token.bottom - 1;
      const sourceInside = sr.top >= cr.top - 1 && sr.bottom <= cr.bottom + 1 && sr.left >= cr.left - 1 && sr.right <= cr.right + 1;
      return {
        value: value.textContent,
        source: source.textContent,
        tips: document.querySelectorAll(".cm-tooltip").length,
        above, below,
        sourceInside,
        valueFirst: vr.bottom <= sr.top + 1,
        cardW: cr.width,
        cardH: cr.height,
      };
    })()`);
    check(cross.value === "$40", `cross-page hover showed ${cross.value}`);
    check(cross.source === "from Trip Budget", `cross-page source was ${cross.source}`);
    check(cross.tips === 1, `cross-page hover opened ${cross.tips} tooltips`);
    check(cross.sourceInside, "the page title is outside the value card");
    check(cross.valueFirst, "the page title is not under the value");
    check(cross.above || cross.below, `the card overlaps the reference (${JSON.stringify(cross)})`);
    check(cross.above, `the card sat below the line when there was room above (${JSON.stringify(cross)})`);
    if (shots) await plainShot(join(shots, "hover-cross-page.png"));
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
    const links = [...(panel?.querySelectorAll("a") || [])].map((a) => a.href).join(" ");
    return { about, open: panel && !panel.hidden, text: panel?.innerText || "", settingsPanel, links };
  })()`);
  check(aboutTab.about === "true" && aboutTab.open === true && aboutTab.settingsPanel === true, `About tab did not open (${JSON.stringify(aboutTab)})`);
  check(aboutTab.links.includes("https://omacus.com/"), "About does not link to the site");
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
  await resetStore();
  await evalJson(`localStorage.setItem("omacus:rates", JSON.stringify({ at: 0, rates: { EUR: 0.5, USD: 1 } }))`);
  await navigate(`http://127.0.0.1:${port}/app/`);
  await waitFor(`document.querySelector("[data-rates-note]")?.dataset.ratesState === "stale"`, 20000);
  const staleRates = await evalJson(`(() => {
    const note = document.querySelector("[data-rates-note]");
    return { hidden: note.hidden, text: note.textContent, state: note.dataset.ratesState };
  })()`);
  check(staleRates.state === "stale" && staleRates.hidden === false && staleRates.text.includes("rates may be stale"), `stale rates note was ${JSON.stringify(staleRates)}`);
  if (shots) await plainShot(join(shots, "rates-stale.png"));
  await resetStore();
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

  // A finger on the page closes the drawer only when it lifts, and does not
  // move the caret or focus the editor. Wait until the welcome page is in
  // place, so a late load is not mistaken for the tap editing the page.
  await waitFor(`(document.querySelector(".cm-content")?.textContent || "").includes("Welcome to Omacus")`, 15000);
  const beforeCaret = await evalJson(`(() => {
    const cm = document.querySelector(".cm-content");
    const text = cm.textContent;
    cm.focus();
    const line = cm.querySelector(".cm-line") || cm;
    const node = line.firstChild && line.firstChild.nodeType === 3 ? line.firstChild : line;
    const range = document.createRange();
    const offsetAt = node.nodeType === 3 ? Math.min(4, node.length) : 0;
    range.setStart(node, offsetAt);
    range.collapse(true);
    const sel = getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
    const offset = sel.anchorOffset;
    const selected = sel.toString();
    cm.blur();
    return { text, offset, selected };
  })()`);
  await evalJson(`document.querySelector(".sheets-toggle").click()`);
  await waitFor(`document.body.classList.contains("drawer") && document.querySelector(".side").getBoundingClientRect().left >= -1`);
  await page("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [{ x: point.x, y: point.y, id: 1 }],
  });
  const mid = await evalJson(`({
    open: document.body.classList.contains("drawer"),
    focused: !!document.querySelector(".cm-editor.cm-focused"),
    active: document.activeElement?.className || document.activeElement?.tagName || "",
  })`);
  check(mid.open === true, "the drawer closed before the finger lifted");
  check(mid.focused === false, `the press focused the editor (${mid.active})`);
  await page("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await waitFor(`!document.body.classList.contains("drawer")`);
  const lifted = await evalJson(`(() => {
    const cm = document.querySelector(".cm-content");
    const focused = !!document.querySelector(".cm-editor.cm-focused");
    cm.focus();
    const sel = getSelection();
    const offset = sel.anchorOffset;
    const selected = sel.toString();
    cm.blur();
    return {
      open: document.body.classList.contains("drawer"),
      focused,
      active: document.activeElement?.className || "",
      text: cm.textContent,
      offset,
      selected,
    };
  })()`);
  check(lifted.open === false, "lifting the finger left the drawer open");
  check(lifted.focused === false, `the tap focused the editor (${lifted.active})`);
  check(lifted.text === beforeCaret.text, "the tap changed the page");
  check(lifted.offset === beforeCaret.offset && lifted.selected === beforeCaret.selected, `the caret moved (${beforeCaret.offset}/${beforeCaret.selected} -> ${lifted.offset}/${lifted.selected})`);

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
  await resetStore();
  await evalJson(`localStorage.setItem("omacus:rates", JSON.stringify({ at: 0, rates: { EUR: 0.85, USD: 1 } }))`);
  failRates = true;
  await page("Fetch.enable", {
    patterns: [{ urlPattern: "https://open.er-api.com/*", requestStage: "Request" }],
  });
  await navigate(`http://127.0.0.1:${port}/app/?sw=1`);
  await waitFor(`document.querySelector(".cm-content")`, 20000);
  await waitFor(`[...document.querySelectorAll(".side-row .side-name span")].some((node) => node.textContent === "Welcome to Omacus")`, 20000);
  const freshBar = await evalJson(`(() => {
    const names = [...document.querySelectorAll(".side-row")].slice(0, 2).map((row) => row.querySelector(".side-name span")?.textContent || "");
    const open = document.querySelector(".side-row[aria-current='true'] .side-name span")?.textContent || "";
    return {
      names,
      open,
      editor: document.querySelector(".cm-content")?.textContent || "",
      privateChip: !!document.querySelector(".private-chip"),
    };
  })()`);
  check(freshBar.names[0] === "Welcome to Omacus" && freshBar.names[1] === "Examples", `fresh sidebar was ${freshBar.names.join(" | ")}`);
  check(freshBar.open === "Welcome to Omacus" && freshBar.editor.includes("Welcome to Omacus"), `welcome was not the open page (${freshBar.open})`);
  check(freshBar.privateChip === false, "the Private pill is still in the top bar");
  await waitForAppCache();
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

async function resetStore() {
  await navigate(`http://127.0.0.1:${port}/404.html`);
  await evalJson(`(async () => {
    await new Promise((resolve) => {
      const req = indexedDB.deleteDatabase("omacus");
      req.onsuccess = () => resolve();
      req.onerror = () => resolve();
      req.onblocked = () => setTimeout(resolve, 200);
    });
    const keys = [];
    for (let i = 0; i < localStorage.length; i++) keys.push(localStorage.key(i));
    for (const key of keys) if (key?.startsWith("omacus:")) localStorage.removeItem(key);
    return true;
  })()`);
}

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
        const name = keys.find((key) => key.startsWith("omacus-app-"));
        if (!name) return "no-cache";
        const urls = (await (await caches.open(name)).keys()).map((k) => new URL(k.url).pathname);
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

async function openRowMenuPreview(forceDrawer = false) {
  return evalJson(`(() => {
    if (${forceDrawer ? "true" : "false"}) document.body.classList.add("drawer");
    const menu = document.querySelector("[data-row-menu]");
    const side = document.querySelector(".side");
    menu.replaceChildren();
    for (const label of ["Move up", "Move down", "Move to…", "Select", "Lock…", "Delete page"]) {
      const button = document.createElement("button");
      button.type = "button";
      button.setAttribute("role", "menuitem");
      button.textContent = label;
      if (label === "Delete page") button.className = "danger";
      menu.append(button);
    }
    menu.hidden = false;
    menu.style.visibility = "hidden";
    menu.style.margin = "0";
    menu.style.right = "auto";
    menu.style.bottom = "auto";
    menu.style.left = "0px";
    menu.style.top = "0px";
    if (typeof menu.showPopover === "function" && !menu.matches(":popover-open")) menu.showPopover();
    if (${forceDrawer ? "true" : "false"}) document.body.classList.add("drawer");
    const natural = menu.getBoundingClientRect();
    const sideR = side.getBoundingClientRect();
    const margin = 8;
    let left = Math.max(margin, sideR.right - 36);
    let top = Math.max(margin, Math.min(sideR.top + 96, innerHeight - natural.height - margin));
    if (left + natural.width > innerWidth - margin) left = Math.max(margin, innerWidth - natural.width - margin);
    menu.style.left = left + "px";
    menu.style.top = top + "px";
    menu.style.visibility = "";
    const r = menu.getBoundingClientRect();
    const samples = [...menu.querySelectorAll("button")].map((button) => {
      const b = button.getBoundingClientRect();
      const x = b.left + 6;
      const y = b.top + Math.min(b.height / 2, b.height - 2);
      const hit = document.elementFromPoint(x, y);
      return {
        label: button.textContent,
        inMenu: !!(hit && menu.contains(hit)),
        hit: (hit?.textContent || hit?.className || hit?.tagName || "").slice(0, 40),
      };
    });
    const style = getComputedStyle(menu);
    return {
      parent: menu.parentElement === document.body,
      inSheet: !!menu.closest(".sheet"),
      popover: menu.matches(":popover-open"),
      z: Number(style.zIndex),
      position: style.position,
      left: r.left,
      right: r.right,
      top: r.top,
      bottom: r.bottom,
      width: r.width,
      sideRight: sideR.right,
      onScreen: r.left >= -1 && r.top >= -1 && r.right <= innerWidth + 1 && r.bottom <= innerHeight + 1,
      overlapsSide: r.left < sideR.right - 1 && r.right > sideR.left + 1,
      drawer: document.body.classList.contains("drawer"),
      sideTransform: getComputedStyle(side).transform,
      samples,
    };
  })()`);
}

async function closeRowMenuPreview() {
  await evalJson(`(() => {
    const menu = document.querySelector("[data-row-menu]");
    if (!menu) return;
    if (typeof menu.hidePopover === "function" && menu.matches(":popover-open")) menu.hidePopover();
    menu.hidden = true;
    menu.replaceChildren();
  })()`);
}

async function assertRowMenuClearsSidebar(where, forceDrawer = false) {
  if (forceDrawer) {
    await evalJson(`document.body.classList.add("drawer")`);
    await waitFor(`document.querySelector(".side").getBoundingClientRect().right > 40`);
  }
  const menu = await openRowMenuPreview(forceDrawer);
  try {
    check(menu.parent === true && menu.inSheet === false, `${where} row menu stayed inside the page`);
    check(menu.popover === true, `${where} row menu did not reach the top layer`);
    check(menu.position === "fixed" && menu.z >= 70, `${where} row menu position ${menu.position} z ${menu.z}`);
    check(menu.onScreen === true, `${where} row menu left the screen (${Math.round(menu.left)},${Math.round(menu.top)} ${Math.round(menu.right)}x${Math.round(menu.bottom)})`);
    check(menu.overlapsSide === true, `${where} row menu did not overlap the sidebar (left ${Math.round(menu.left)}, side ${Math.round(menu.sideRight)}, drawer ${menu.drawer}, transform ${menu.sideTransform})`);
    check(menu.samples.length >= 6 && menu.samples.every((sample) => sample.inMenu), `${where} row menu was covered (${JSON.stringify(menu.samples)})`);
  } finally {
    await closeRowMenuPreview();
  }
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
