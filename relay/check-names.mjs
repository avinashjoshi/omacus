// Device names, the page-set name, and a page that stays on one computer.
// The server stores the sealed record and never the names.
import fs from "node:fs";
import { chromium } from "playwright";
import { mint } from "./worker/access.js";

const base = process.argv[2] || "http://127.0.0.1:8800";
const relay = process.argv[3] || "http://127.0.0.1:8791";
const out = "/opt/cursor/artifacts/sync-test";
fs.mkdirSync(out, { recursive: true });

const PHONE = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1";
const NAMES = ["Trip notes", "Desk", "Harbor phone", "Cabin phone", "Window desk"];
const leaks = [];
let account = "";
let puts = 0;

const browser = await chromium.launch({
  channel: "chrome",
  headless: true,
  args: ["--no-sandbox", "--disable-dev-shm-usage"],
});

function watch(page) {
  page.on("request", (req) => {
    const match = req.url().match(/\/v1\/mailboxes\/([0-9a-f]{64})/i);
    if (match) account = match[1];
    const publish = (req.method() === "POST" && /\/v1\/mailboxes$/.test(new URL(req.url()).pathname))
      || (req.method() === "PUT" && req.url().includes("/v1/mailboxes/"));
    if (publish) puts += 1;
    const body = req.postData() || "";
    for (const word of NAMES) {
      if (body.includes(word)) leaks.push(`${req.method()} ${req.url()} ${word}`);
    }
  });
}

async function openApp(context) {
  const page = await context.newPage();
  await page.goto(base + "/?sw=1", { waitUntil: "domcontentloaded", timeout: 60000 });
  await page.waitForSelector(".cm-content", { timeout: 60000 });
  await page.waitForSelector(".sync-chip", { timeout: 60000 });
  watch(page);
  return page;
}

async function openSync(page) {
  await page.locator(".sync-chip").click();
  await page.waitForSelector("[data-settings-panel='sync']:not([hidden])", { timeout: 10000 });
}

async function closeSettings(page) {
  await page.locator(".settings-card [data-action='close-settings']").click();
}

async function writePage(page, text) {
  await page.locator(".cm-content").click();
  await page.keyboard.press("Control+A");
  await page.keyboard.insertText(text);
  await page.waitForTimeout(500);
}

async function openNamed(page, name) {
  await page.locator(".side-name").filter({ hasText: new RegExp(`^${name}$`) }).click();
  await page.waitForTimeout(150);
}

async function titles(page) {
  return page.locator(".side-name span").allTextContents();
}

async function countTitle(page, name) {
  const rows = await titles(page);
  return rows.filter((row) => row.trim() === name).length;
}

async function placeOf(page, name) {
  return page.evaluate((title) => {
    const row = [...document.querySelectorAll(".side-row")].find((node) =>
      [...node.querySelectorAll(".side-name span")].some((span) => span.textContent.trim() === title));
    return row?.dataset.place || "";
  }, name);
}

async function moveViaMenu(page, name, destination) {
  const row = page.locator(".side-row").filter({ has: page.locator(".side-name span", { hasText: new RegExp(`^${name}$`) }) });
  await row.locator(".row-more").click();
  await page.getByRole("menuitem", { name: "Move to…" }).click();
  await page.locator("[data-sync-dest]").waitFor();
  await page.getByRole("button", { name: destination, exact: true }).click();
}

async function dragToSection(page, name, section) {
  const row = page.locator(".side-row").filter({ has: page.locator(".side-name span", { hasText: new RegExp(`^${name}$`) }) });
  const target = page.locator(`[data-drop-section='${section}']`);
  const from = await row.boundingBox();
  const to = await target.boundingBox();
  if (!from || !to) throw new Error(`drag boxes ${name} ${section}`);
  await page.mouse.move(from.x + 36, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(to.x + 28, to.y + to.height / 2, { steps: 18 });
  await page.mouse.up();
}

async function commitField(page, label, value) {
  const field = page.getByRole("textbox", { name: label });
  await field.fill(value);
  await field.evaluate((node) => node.blur());
}

async function settled(page) {
  await page.waitForFunction(() => {
    const chip = document.querySelector(".sync-chip");
    return chip?.dataset.state === "on" && chip.textContent !== "Saving…";
  }, null, { timeout: 20000 });
}

async function waitPuts(before) {
  const start = Date.now();
  while (puts <= before) {
    if (Date.now() - start > 20000) throw new Error(`no publish after ${before}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  await settled(pageA);
}

async function wordsOnSheet(page) {
  return page.locator(".sync-word").evaluateAll((nodes) => nodes.map((node) => node.dataset.word));
}

async function pickWords(page, words) {
  await page.locator(".sync-choice").first().waitFor({ timeout: 20000 });
  const legends = await page.locator("fieldset legend").allTextContents();
  for (const legend of legends) {
    const n = Number(legend.replace(/\D/g, ""));
    const word = words[n - 1];
    await page.locator(`.sync-choice[data-n="${n}"][data-word="${word}"]`).click();
  }
}

async function shot(page, name) {
  await page.screenshot({ path: `${out}/${name}.png` });
}

const contextA = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: "light" });
const contextB = await browser.newContext({
  viewport: { width: 1280, height: 800 },
  colorScheme: "light",
  userAgent: PHONE,
});
const pageA = await openApp(contextA);
const pageB = await openApp(contextB);

try {
  const health = await (await fetch(relay + "/v1/health")).json();
  if (health.label !== "1.0.0-beta.11") throw new Error(`relay label ${health.label}`);

  await openSync(pageA);
  const guessed = await pageA.getByRole("textbox", { name: "Name this phone or computer" }).inputValue();
  if (!/Chrome on /.test(guessed)) throw new Error(`desktop guess ${guessed}`);
  await commitField(pageA, "Name this phone or computer", "Desk");
  await pageA.getByRole("button", { name: "Start fresh" }).click();
  const inviteCode = await mint(process.env.ACCESS_SECRET || "omacus-access-test-secret", 1);
  await pageA.locator("[data-access-code]").fill(inviteCode);
  await pageA.getByRole("button", { name: "Start fresh" }).click();
  await pageA.waitForSelector(".sync-word", { timeout: 30000 });
  const words = await wordsOnSheet(pageA);
  if (words.length !== 12) throw new Error(`words ${words.length}`);
  await pageA.getByRole("button", { name: "I saved them" }).click();
  await pickWords(pageA, words);
  const beforeCreate = puts;
  await pageA.getByRole("button", { name: "These match" }).click();
  await pageA.locator("[data-sync-set]").waitFor({ timeout: 60000 });
  await waitPuts(beforeCreate);
  if (!(await pageA.locator(".sync-devices").innerText()).includes("Desk")) throw new Error("Desk missing after setup");

  const beforeRename = puts;
  await commitField(pageA, "Name for these pages", "Trip notes");
  await pageA.getByRole("heading", { name: "Sync is on" }).waitFor();
  await waitPuts(beforeRename);
  if (!(await pageA.locator("[data-sync-section='vault']").innerText()).includes("Trip notes")) {
    throw new Error("sidebar did not take the page-set name");
  }
  const before = await titles(pageA);

  await pageA.getByRole("button", { name: "Add a phone or computer" }).click();
  await pageA.locator(".sync-invite-code").waitFor({ timeout: 20000 });
  const invite = (await pageA.locator(".sync-invite-code").innerText()).trim();

  await openSync(pageB);
  await pageB.getByRole("button", { name: "Join my existing pages" }).click();
  const phoneGuess = await pageB.getByRole("textbox", { name: "Name this phone or computer" }).inputValue();
  if (phoneGuess !== "iPhone") throw new Error(`phone guess ${phoneGuess}`);
  await commitField(pageB, "Name this phone or computer", "Harbor phone");
  await pageB.locator("[data-join-code]").fill(invite);
  await pageB.getByRole("button", { name: "Join", exact: true }).click();
  await pageA.getByRole("heading", { name: "Is this your phone or computer?" }).waitFor({ timeout: 20000 });
  await pageA.getByRole("button", { name: "Yes, approve it" }).click();
  await pageB.locator("[data-sync-set]").waitFor({ timeout: 60000 });
  await pageA.locator("[data-sync-set]").waitFor({ timeout: 60000 });
  await pageA.waitForFunction(() => document.querySelector(".sync-devices")?.innerText.includes("Harbor phone"), null, { timeout: 20000 });
  await pageB.waitForFunction(() => document.querySelector(".sync-devices")?.innerText.includes("Desk"), null, { timeout: 20000 });
  if (!(await pageB.locator("[data-sync-section='vault']").innerText()).includes("Trip notes")) {
    throw new Error("the other phone did not show Trip notes");
  }
  const afterRename = await titles(pageA);
  if (afterRename.length < before.length) throw new Error("renaming dropped a page");

  const beforeDevice = puts;
  await pageA.locator(".sync-device").filter({ hasText: "Harbor phone" }).getByRole("button", { name: "Rename" }).click();
  await commitField(pageA, "Name for this phone or computer", "Cabin phone");
  await waitPuts(beforeDevice);
  await pageA.waitForFunction(() => document.querySelector(".sync-devices")?.innerText.includes("Cabin phone"), null, { timeout: 10000 });
  await pageB.waitForFunction(() => {
    const field = document.querySelector("[aria-label='Name this phone or computer']");
    return field?.value === "Cabin phone";
  }, null, { timeout: 20000 });
  const beforeDesk = puts;
  await pageB.locator(".sync-device").filter({ hasText: "Desk" }).getByRole("button", { name: "Rename" }).click();
  await commitField(pageB, "Name for this phone or computer", "Window desk");
  await pageB.waitForFunction(() => document.querySelector(".sync-devices")?.innerText.includes("Window desk"), null, { timeout: 10000 });
  await pageA.waitForFunction(() => {
    const field = document.querySelector("[aria-label='Name this phone or computer']");
    return field?.value === "Window desk";
  }, null, { timeout: 20000 });
  if (puts <= beforeDesk) throw new Error("renaming the other computer did not publish");
  if ((await titles(pageA)).length < before.length) throw new Error("renaming a phone dropped a page");

  await closeSettings(pageA);
  await closeSettings(pageB);
  await pageA.locator(".side-head [aria-label='New page']").click();
  await writePage(pageA, "# Lentils\nlentils = 180 g\n");
  await pageA.waitForFunction(() => [...document.querySelectorAll(".side-name span")].some((node) => node.textContent.trim() === "Lentils"), null, { timeout: 10000 });
  await pageA.locator(".side-head [aria-label='New page']").click();
  await writePage(pageA, "# Pantry\noats = 2\n");
  await pageA.waitForFunction(() => [...document.querySelectorAll(".side-name span")].some((node) => node.textContent.trim() === "Pantry"), null, { timeout: 10000 });
  await pageB.waitForFunction(() => [...document.querySelectorAll(".side-name span")].filter((node) => node.textContent.trim() === "Lentils").length === 1, null, { timeout: 20000 });
  await pageB.waitForFunction(() => [...document.querySelectorAll(".side-name span")].filter((node) => node.textContent.trim() === "Pantry").length === 1, null, { timeout: 20000 });

  await pageA.locator(".side-name").filter({ hasText: /^Lentils$/ }).click({ modifiers: ["Control"] });
  await pageA.locator(".side-name").filter({ hasText: /^Pantry$/ }).click({ modifiers: ["Control"] });
  await moveViaMenu(pageA, "Lentils", "On this computer");
  const ask = await pageA.locator("[data-sync-move-detail]").innerText();
  const askTitle = await pageA.locator("[data-sync-move-title]").innerText();
  if (!askTitle.includes("2 pages")) throw new Error(`confirm title ${askTitle}`);
  if (!ask.includes("Nothing is deleted") || !ask.includes("will not get later changes")) {
    throw new Error(`confirm copy ${ask}`);
  }
  await pageA.setViewportSize({ width: 1280, height: 800 });
  await pageA.evaluate(() => { document.documentElement.dataset.appTheme = "light"; });
  await shot(pageA, "names-move-laptop");
  await pageA.getByRole("button", { name: "Not now" }).click();
  if (await placeOf(pageA, "Lentils") !== "synced") throw new Error("cancel moved Lentils");
  if (await placeOf(pageA, "Pantry") !== "synced") throw new Error("cancel moved Pantry");

  const beforeMove = puts;
  await moveViaMenu(pageA, "Lentils", "On this computer");
  await pageA.getByRole("button", { name: "Move", exact: true }).click();
  await waitPuts(beforeMove);
  if (await placeOf(pageA, "Lentils") !== "local") throw new Error("Lentils stayed in the set");
  if (await placeOf(pageA, "Pantry") !== "local") throw new Error("Pantry stayed in the set");
  if (await countTitle(pageA, "Lentils") !== 1 || await countTitle(pageA, "Pantry") !== 1) {
    throw new Error("moving two pages duplicated one");
  }
  const beforeUndo = puts;
  await pageA.getByRole("button", { name: "Undo" }).click();
  await waitPuts(beforeUndo);
  if (await placeOf(pageA, "Lentils") !== "synced" || await placeOf(pageA, "Pantry") !== "synced") {
    throw new Error("undo did not put the pages back");
  }
  if (await countTitle(pageA, "Lentils") !== 1 || await countTitle(pageA, "Pantry") !== 1) {
    throw new Error("undo duplicated a page");
  }

  await dragToSection(pageA, "Lentils", "local");
  await pageA.locator("[data-sync-move]").waitFor({ timeout: 5000 });
  const one = await pageA.locator("[data-sync-move-title]").innerText();
  if (!one.includes("Lentils")) throw new Error(`drag confirm ${one}`);
  const beforeOne = puts;
  await pageA.getByRole("button", { name: "Move", exact: true }).click();
  await waitPuts(beforeOne);
  if (await placeOf(pageA, "Lentils") !== "local") throw new Error("drag left Lentils in the set");
  if (await placeOf(pageA, "Pantry") !== "synced") throw new Error("drag moved Pantry too");
  if (await countTitle(pageA, "Lentils") !== 1) throw new Error("dragging Lentils duplicated it");
  await pageA.evaluate(() => {
    const toast = document.querySelector("[data-toast]");
    if (toast) toast.hidden = true;
  });
  await pageB.waitForFunction(() => {
    const place = (title) => {
      const row = [...document.querySelectorAll(".side-row")].find((node) =>
        [...node.querySelectorAll(".side-name span")].some((span) => span.textContent.trim() === title));
      return row?.dataset.place || "";
    };
    return place("Lentils") === "local" && place("Pantry") === "synced";
  }, null, { timeout: 20000 });

  await pageA.locator("[data-sync-section='vault'] button").click();
  if (await pageA.locator("[data-sync-section='vault'] button").getAttribute("aria-expanded") !== "false") {
    throw new Error("the page set did not collapse");
  }
  if ((await titles(pageA)).some((row) => row.trim() === "Trip Budget")) throw new Error("a collapsed section still listed Trip Budget");
  if (await countTitle(pageA, "Lentils") !== 1) throw new Error("collapsing hid the page that stays here");
  await pageA.locator("[data-sync-section='vault'] button").click();

  await openNamed(pageA, "Lentils");
  await pageA.locator(".cm-content").click();
  await pageA.keyboard.press("Control+End");
  await pageA.keyboard.insertText("\nmustard = 1\n");
  await pageA.locator(".cm-content").getByText("mustard = 1").waitFor({ timeout: 5000 });
  await openNamed(pageA, "Trip Budget");
  await pageA.locator(".cm-content").click();
  await pageA.keyboard.press("Control+End");
  await pageA.keyboard.insertText("\nsesame = 12 g\n");
  await pageA.waitForTimeout(400);
  await openNamed(pageB, "Trip Budget");
  await pageB.locator(".cm-content").getByText("sesame = 12 g").waitFor({ timeout: 20000 });
  await settled(pageA);
  await pageB.waitForTimeout(4500);
  await openNamed(pageB, "Lentils");
  const held = await pageB.locator(".cm-content").innerText();
  if (held.includes("mustard = 1")) throw new Error("a page kept here was sent to the other phone");
  if (!held.includes("lentils = 180 g")) throw new Error("the other phone lost Lentils");

  await openNamed(pageA, "Lentils");
  const beforeMoveText = await pageA.locator(".cm-content").innerText();
  if (!beforeMoveText.includes("mustard = 1")) throw new Error(`Desk lost mustard before the move: ${beforeMoveText.slice(0, 180)}`);
  await moveViaMenu(pageA, "Lentils", "Trip notes");
  await pageA.getByRole("button", { name: "Move", exact: true }).click();
  await settled(pageA);
  await openNamed(pageB, "Lentils");
  await pageB.locator(".cm-content").getByText("mustard = 1").waitFor({ timeout: 20000 });
  if (await countTitle(pageA, "Lentils") !== 1) throw new Error("moving Lentils back duplicated it on Desk");
  if (await countTitle(pageB, "Lentils") !== 1) throw new Error("moving Lentils back duplicated it on the phone");
  await openNamed(pageA, "Lentils");
  const lentilText = await pageA.locator(".cm-content").innerText();
  if (!lentilText.includes("lentils = 180 g") || !lentilText.includes("mustard = 1")) throw new Error("Desk lost the Lentils lines");

  await pageA.reload({ waitUntil: "domcontentloaded" });
  await pageB.reload({ waitUntil: "domcontentloaded" });
  await pageA.waitForSelector(".sync-chip", { timeout: 60000 });
  await pageB.waitForSelector(".sync-chip", { timeout: 60000 });
  await pageA.waitForFunction(() => [...document.querySelectorAll(".side-name span")].filter((node) => node.textContent.trim() === "Lentils").length === 1, null, { timeout: 20000 });
  await pageB.waitForFunction(() => [...document.querySelectorAll(".side-name span")].filter((node) => node.textContent.trim() === "Lentils").length === 1, null, { timeout: 20000 });
  await openNamed(pageA, "Lentils");
  const reloaded = await pageA.locator(".cm-content").innerText();
  if (!reloaded.includes("mustard = 1")) throw new Error("reload lost mustard");
  await openNamed(pageA, "Trip Budget");
  if (!(await pageA.locator(".cm-content").innerText()).includes("sesame = 12 g")) throw new Error("reload lost sesame");

  await settled(pageA);
  const samples = [];
  const widths = [];
  for (let i = 0; i < 12; i += 1) {
    const chip = await pageA.locator(".sync-chip").evaluate((node) => ({
      text: node.textContent,
      state: node.dataset.state,
      width: node.getBoundingClientRect().width,
    }));
    samples.push(`${chip.state}:${chip.text}`);
    widths.push(chip.width);
    await pageA.waitForTimeout(400);
  }
  if (samples.some((sample) => sample.includes("Saving") || sample.includes("Saved"))) {
    throw new Error(`chip flickered ${samples.join(" | ")}`);
  }
  if (new Set(widths).size !== 1) throw new Error(`chip width jumped ${widths.join(",")}`);

  await openSync(pageA);
  await pageA.locator(".sync-device").filter({ hasText: "Cabin phone" }).getByRole("button", { name: "Remove" }).click();
  await pageA.getByRole("heading", { name: "Remove Cabin phone?" }).waitFor();
  await pageA.setViewportSize({ width: 1280, height: 800 });
  await pageA.evaluate(() => { document.documentElement.dataset.appTheme = "light"; });
  await shot(pageA, "names-remove-laptop");
  await pageA.getByRole("button", { name: "Keep it" }).click();
  await pageA.locator("[data-sync-set]").waitFor();
  await shot(pageA, "names-settings-laptop");
  await closeSettings(pageA);
  await shot(pageA, "names-sidebar-laptop");

  await pageB.setViewportSize({ width: 390, height: 844 });
  await pageB.evaluate(() => { document.documentElement.dataset.appTheme = "dark"; });
  const dismiss = pageB.locator("[data-action='dismiss-install']");
  if (await dismiss.isVisible()) await dismiss.click();
  await pageB.locator(".sheets-toggle").click();
  await pageB.waitForFunction(() => document.querySelector(".side")?.getBoundingClientRect().left >= 0, null, { timeout: 5000 });
  await shot(pageB, "names-sidebar-phone");
  const phoneRow = pageB.locator(".side-row").filter({ has: pageB.locator(".side-name span", { hasText: /^Pantry$/ }) });
  await phoneRow.locator(".row-more").click();
  await pageB.getByRole("menuitem", { name: "Move to…" }).click();
  await pageB.locator("[data-sync-dest]").waitFor();
  await shot(pageB, "names-move-phone");
  await pageB.getByRole("button", { name: "Cancel" }).click();
  await pageB.locator(".sheets-toggle").click();
  await openSync(pageB);
  await shot(pageB, "names-settings-phone");

  if (!account) throw new Error("no mailbox");
  const stored = await (await fetch(`${relay}/v1/mailboxes/${account}`)).text();
  let decoded = stored;
  try {
    const record = JSON.parse(stored).record || "";
    decoded += Buffer.from(record, "base64").toString("utf8");
  } catch {
    decoded = stored;
  }
  for (const word of NAMES) {
    if (decoded.includes(word)) throw new Error(`the server record contains ${word}`);
  }
  if (leaks.length) throw new Error(`a request carried a name: ${leaks.join(", ")}`);
  await browser.close();
  console.log("names stay sealed, and a page can move without a copy");
} catch (err) {
  await pageA.screenshot({ path: `${out}/names-fail-a.png` }).catch(() => {});
  await pageB.screenshot({ path: `${out}/names-fail-b.png` }).catch(() => {});
  const textA = await pageA.locator("body").innerText().catch(() => "");
  const textB = await pageB.locator("body").innerText().catch(() => "");
  fs.writeFileSync(`${out}/names-fail-a.txt`, textA.slice(0, 4000));
  fs.writeFileSync(`${out}/names-fail-b.txt`, textB.slice(0, 4000));
  console.error(err);
  await browser.close();
  process.exit(1);
}
