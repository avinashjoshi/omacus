// Two browsers, one sealed mailbox. Pages move both ways and survive a reload.
import fs from "node:fs";
import { chromium } from "playwright";

const base = process.argv[2] || "http://127.0.0.1:8800";
const out = "/opt/cursor/artifacts/sync-test";
fs.mkdirSync(out, { recursive: true });

const leaks = [];
const browser = await chromium.launch({
  channel: "chrome",
  headless: true,
  args: ["--no-sandbox", "--disable-dev-shm-usage"],
});

function watch(page, words) {
  page.on("request", (req) => {
    const body = req.postData() || "";
    if (!body) return;
    const hay = body.toLowerCase();
    if (hay.includes("omacus-device/1") || hay.includes("ferry = 40") || hay.includes("milk $3")) {
      leaks.push(req.url());
    }
    for (const word of words) {
      if (word && hay.includes(String(word).toLowerCase())) leaks.push(req.url() + " word");
    }
  });
}

async function openApp(context) {
  const page = await context.newPage();
  await page.goto(base + "/?sw=1", { waitUntil: "domcontentloaded", timeout: 60000 });
  await page.waitForSelector(".cm-content", { timeout: 60000 });
  await page.waitForSelector(".sync-chip", { timeout: 60000 });
  return page;
}

async function writePage(page, text) {
  await page.locator(".cm-content").click();
  await page.keyboard.press("Control+A");
  await page.keyboard.insertText(text);
  await page.waitForTimeout(400);
}

async function openNamed(page, name) {
  await page.locator(".side-name").filter({ hasText: new RegExp(`^${name}$`) }).click();
  await page.waitForTimeout(150);
}

async function openSync(page) {
  await page.locator(".sync-chip").click();
  await page.waitForSelector("[data-settings-panel='sync']:not([hidden])", { timeout: 10000 });
}

async function closeSettings(page) {
  await page.locator(".settings-card [data-action='close-settings']").click();
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
  const count = await page.locator(".sync-choice").count();
  if (count < 18) throw new Error(`expected 6 choices for each of 3 words, saw ${count}`);
}

async function setTheme(page, theme) {
  await openSync(page);
  await page.locator("[data-settings-tab='settings']").click();
  await page.locator("[data-setting='theme']").selectOption(theme);
  await page.locator("[data-settings-tab='sync']").click();
}

async function shot(page, name) {
  await page.screenshot({ path: `${out}/${name}.png` });
}

async function dump(page, name) {
  try {
    await page.screenshot({ path: `${out}/fail-${name}.png` });
    const text = await page.locator("body").innerText();
    fs.writeFileSync(`${out}/fail-${name}.txt`, text.slice(0, 4000));
  } catch (err) {
    console.error("dump failed", name, err.message);
  }
}

const contextA = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: "dark" });
const contextB = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: "light" });
const pageA = await openApp(contextA);
const pageB = await openApp(contextB);
const foundWords = [];
watch(pageA, foundWords);
watch(pageB, foundWords);

let pageC;
try {
await openNamed(pageA, "Trip Budget");
await writePage(pageA, "# Trip Budget\nferry = 40\n");
await openNamed(pageA, "Groceries");
await writePage(pageA, "# Groceries\nmilk $3\n");
await openNamed(pageA, "Trip Budget");

await openSync(pageA);
await pageA.getByRole("button", { name: "Turn on sync" }).click();
await pageA.getByRole("button", { name: "Turn on sync", exact: true }).click();
await pageA.waitForSelector(".sync-word", { timeout: 30000 });
const words = await wordsOnSheet(pageA);
if (words.length !== 12) throw new Error(`expected 12 words, saw ${words.length}`);
foundWords.push(...words);
await pageA.getByRole("button", { name: "I saved them" }).click();
await pickWords(pageA, words);
await pageA.getByRole("button", { name: "Confirm" }).click();
await pageA.getByRole("heading", { name: "Sync" }).waitFor({ timeout: 60000 });
await pageA.getByText("Sync is on").waitFor({ timeout: 20000 });

await pageA.getByRole("button", { name: "Add a device" }).click();
await pageA.locator(".sync-invite-code").waitFor({ timeout: 20000 });
const invite = (await pageA.locator(".sync-invite-code").innerText()).trim();
if (!/^[0-9A-Z]{4}-[0-9A-Z]{4}$/.test(invite)) throw new Error(`invite ${invite}`);

await openSync(pageB);
await pageB.getByRole("button", { name: "Add this device" }).click();
await pageB.getByRole("button", { name: "Type code" }).click();
await pageB.locator("[data-join-code]").waitFor({ timeout: 10000 });
await pageA.setViewportSize({ width: 390, height: 844 });
await pageB.setViewportSize({ width: 390, height: 844 });
await pageA.evaluate(() => { document.documentElement.dataset.appTheme = "dark"; });
await pageB.evaluate(() => { document.documentElement.dataset.appTheme = "light"; });
await shot(pageA, "invite-phone-dark");
await shot(pageB, "type-code-phone-light");

await pageB.locator("[data-join-code]").fill("0000-0001");
await pageB.getByRole("button", { name: "Continue" }).click();
await pageB.getByText("That code didn't work").waitFor({ timeout: 20000 });
const bare = await pageB.evaluate(async () => {
  const res = await fetch("/v1/join", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ name: "Nope" }),
  });
  return res.status;
});
if (bare !== 400) throw new Error(`missing code was accepted (${bare})`);
await pageA.waitForTimeout(1500);
if (await pageA.getByRole("heading", { name: "Approve a new device?" }).count()) {
  throw new Error("a bad code opened the approve prompt");
}

await pageB.locator("[data-join-code]").fill(invite);
await pageB.getByRole("button", { name: "Continue" }).click();
await pageB.locator(".sync-code").waitFor({ timeout: 20000 });
const codeB = (await pageB.locator(".sync-code").innerText()).trim();
await pageA.getByRole("heading", { name: "Approve a new device?" }).waitFor({ timeout: 20000 });
const codeA = (await pageA.locator(".sync-code").innerText()).trim();
if (codeA !== codeB || !/^\d{3} \d{3}$/.test(codeA)) throw new Error(`codes ${codeA} / ${codeB}`);
await shot(pageA, "approve-phone-dark");
await pageA.setViewportSize({ width: 1280, height: 800 });
await pageB.setViewportSize({ width: 1280, height: 800 });
await pageA.getByRole("button", { name: "Approve" }).click();
await pageB.getByText("Sync is on").waitFor({ timeout: 60000 });
await closeSettings(pageB);
await openNamed(pageB, "Trip Budget");
await pageB.locator(".cm-content").getByText("ferry = 40").waitFor({ timeout: 20000 });
const linked = await pageB.locator(".cm-content").innerText();
if (linked.includes("flights") || linked.includes("hotel")) throw new Error(`old lines came back: ${linked}`);
await openNamed(pageB, "Groceries");
await pageB.waitForFunction(() => {
  const text = document.querySelector(".cm-content")?.innerText || "";
  return text.split("\n").some((line) => line.trim() === "milk $3");
}, null, { timeout: 20000 });

await openNamed(pageB, "Trip Budget");
await contextA.setOffline(true);
await contextB.setOffline(true);
await closeSettings(pageA).catch(() => {});
await openNamed(pageA, "Trip Budget");
await pageA.locator(".cm-content").click();
await pageA.keyboard.press("Control+End");
await pageA.keyboard.insertText("\ncake $5\n");
await pageB.locator(".cm-content").click();
await pageB.keyboard.press("Control+End");
await pageB.keyboard.insertText("\nbread $2\n");
await pageA.waitForTimeout(600);
await pageB.waitForTimeout(600);
await contextA.setOffline(false);
await contextB.setOffline(false);
await pageA.locator(".cm-content").getByText("bread $2").waitFor({ timeout: 30000 });
await pageB.locator(".cm-content").getByText("cake $5").waitFor({ timeout: 30000 });
const merged = await pageA.locator(".cm-content").innerText();
for (const line of ["ferry = 40", "cake $5", "bread $2"]) {
  if (!merged.includes(line)) throw new Error(`A lost ${line}`);
}
await openNamed(pageA, "Groceries");
if (!(await pageA.locator(".cm-content").innerText()).split("\n").some((line) => line.trim() === "milk $3")) {
  throw new Error("A lost groceries");
}

await pageA.reload({ waitUntil: "domcontentloaded" });
await pageB.reload({ waitUntil: "domcontentloaded" });
await pageA.waitForSelector(".sync-chip", { timeout: 60000 });
await pageB.waitForSelector(".sync-chip", { timeout: 60000 });
await openSync(pageA);
await pageA.getByText("Sync is on").waitFor({ timeout: 20000 });
await openSync(pageB);
await pageB.getByText("Sync is on").waitFor({ timeout: 20000 });
await closeSettings(pageA);
await openNamed(pageA, "Trip Budget");
const afterReload = await pageA.locator(".cm-content").innerText();
for (const line of ["ferry = 40", "cake $5", "bread $2"]) {
  if (!afterReload.includes(line)) throw new Error(`reload lost ${line}`);
}

const contextC = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: "dark" });
pageC = await openApp(contextC);
watch(pageC, foundWords);
await openSync(pageC);
await pageC.getByRole("button", { name: "Use your recovery key" }).click();
await pageC.locator("textarea").fill(words.map((word, i) => `${i + 1}. ${word}`).join("\n"));
await pageC.getByRole("button", { name: "Open my pages" }).click();
await pageC.getByRole("heading", { name: "Pick the three words" }).waitFor({ timeout: 60000 });
await pickWords(pageC, words);
await pageC.getByRole("button", { name: "Confirm" }).click();
await pageC.getByText("Sync is on").waitFor({ timeout: 60000 });
await closeSettings(pageC);
await openNamed(pageC, "Trip Budget");
const recovered = await pageC.locator(".cm-content").innerText();
for (const line of ["ferry = 40", "cake $5", "bread $2"]) {
  if (!recovered.includes(line)) throw new Error(`recovery lost ${line}`);
}
await openNamed(pageC, "Groceries");
if (!(await pageC.locator(".cm-content").innerText()).split("\n").some((line) => line.trim() === "milk $3")) {
  throw new Error("recovery lost groceries");
}

await pageA.waitForFunction(() => navigator.serviceWorker?.controller, null, { timeout: 20000 });
await contextA.setOffline(true);
await pageA.reload({ waitUntil: "domcontentloaded" });
await pageA.waitForSelector(".cm-content", { timeout: 60000 });
await openNamed(pageA, "Trip Budget");
const offline = await pageA.locator(".cm-content").innerText();
if (!offline.includes("ferry = 40")) throw new Error("offline lost the page");
await contextA.setOffline(false);

await closeSettings(pageB).catch(() => {});
await openNamed(pageA, "Trip Budget");
await openNamed(pageB, "Trip Budget");
await contextA.setOffline(true);
await contextB.setOffline(true);
await writePage(pageA, "# Trip Budget\nmuseum = 12\n");
await pageB.locator(".cm-content").click();
await pageB.keyboard.press("Control+End");
await pageB.keyboard.insertText("\npark = 2\n");
await pageA.waitForTimeout(700);
await pageB.waitForTimeout(700);
await contextA.setOffline(false);
await contextB.setOffline(false);
await pageA.waitForFunction(() => document.querySelector(".sync-chip")?.textContent === "Saved", null, { timeout: 20000 });
await pageB.locator(".cm-content").getByText("museum = 12").waitFor({ timeout: 30000 });
await pageA.locator(".cm-content").getByText("park = 2").waitFor({ timeout: 30000 });
const afterDelete = await pageA.locator(".cm-content").innerText();
if (afterDelete.includes("ferry = 40")) throw new Error("deleted line came back");
const afterDeleteB = await pageB.locator(".cm-content").innerText();
if (afterDeleteB.includes("ferry = 40")) throw new Error("deleted line came back on B");
if (!afterDeleteB.includes("park = 2")) throw new Error("B lost the new line");

await contextA.setOffline(true);
await pageA.waitForFunction(() => document.querySelector(".sync-chip")?.textContent === "Offline", null, { timeout: 15000 });
await contextA.setOffline(false);
await pageA.waitForFunction(() => document.querySelector(".sync-chip")?.textContent === "Saved", null, { timeout: 20000 });

await pageA.setViewportSize({ width: 1280, height: 800 });
await pageB.setViewportSize({ width: 1280, height: 800 });
await setTheme(pageA, "dark");
await shot(pageA, "settings-desktop-dark");
await closeSettings(pageA);
await openNamed(pageA, "Trip Budget");
await shot(pageA, "page-a-desktop-dark");
await setTheme(pageB, "light");
await shot(pageB, "settings-desktop-light");
await closeSettings(pageB);
await openNamed(pageB, "Trip Budget");
await shot(pageB, "page-b-desktop-light");

await pageA.setViewportSize({ width: 390, height: 844 });
await pageB.setViewportSize({ width: 390, height: 844 });
const version = (await pageA.locator(".side-version").innerText()).trim();
if (version !== "omacus 1.0.0-beta.1") throw new Error(`version line ${version}`);
const beta = await pageA.locator(".sync-beta").innerText();
if (!beta.includes("1.0.0-beta.1")) throw new Error(`phone label ${beta}`);
await pageA.waitForFunction(() => document.querySelector(".side")?.getBoundingClientRect().right <= 0, null, { timeout: 2000 });
const edge = await pageA.locator(".side").evaluate((node) => {
  const style = getComputedStyle(node);
  return {
    shadow: style.boxShadow,
    right: node.getBoundingClientRect().right,
    border: style.borderRightColor,
  };
});
if (edge.shadow !== "none") throw new Error(`closed drawer shadow ${edge.shadow}`);
if (edge.right > 0) throw new Error(`closed drawer edge ${edge.right}`);
if (edge.border !== "rgba(0, 0, 0, 0)") throw new Error(`closed drawer border ${edge.border}`);
await setTheme(pageA, "dark");
await shot(pageA, "settings-phone-dark");
await closeSettings(pageA);
await shot(pageA, "page-a-phone-dark");
await setTheme(pageB, "light");
await shot(pageB, "settings-phone-light");
await closeSettings(pageB);
await shot(pageB, "page-b-phone-light");

if (leaks.length) throw new Error(`a request carried a page or a word: ${leaks.join(", ")}`);
await browser.close();
console.log("two devices synced, reloaded, and recovered");
} catch (err) {
  if (typeof pageA !== "undefined") await dump(pageA, "a");
  if (typeof pageB !== "undefined") await dump(pageB, "b");
  if (pageC) await dump(pageC, "c");
  console.error(err);
  await browser.close();
  process.exit(1);
}
