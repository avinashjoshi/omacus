// An account rotated by the old bug. Recovery opens the pages, says what may
// be missing, and the next recovery opens that copy with no warning.
import fs from "node:fs";
import { chromium } from "playwright";
import { mint } from "./worker/access.js";

const base = process.argv[2] || "http://127.0.0.1:8800";
const relay = process.argv[3] || "http://127.0.0.1:8791";
const fixturePath = process.argv[4] || "/tmp/omacus-stale.json";
const out = "/opt/cursor/artifacts/sync-test";
fs.mkdirSync(out, { recursive: true });

const fixture = JSON.parse(fs.readFileSync(fixturePath, "utf8"));
const words = fixture.words.split(/\s+/);
if (words.length !== 12) throw new Error(`fixture words ${words.length}`);

const health = await fetch(relay + "/v1/health");
const label = (await health.json()).label;
if (label !== "1.0.0-beta.28") throw new Error(`relay label ${label}`);

const code = await mint("omacus-access-test-secret", 1);
const created = await fetch(relay + "/v1/mailboxes", {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ account: fixture.account, record: fixture.record, code }),
});
if (!created.ok) throw new Error(`create ${created.status} ${await created.text()}`);

const NOTE = "Some changes made after a phone or computer was removed may be missing. They were locked to phones and computers that are no longer linked. Pages already here were combined with the ones these words could open. Nothing was deleted.";

const leaks = [];
const browser = await chromium.launch({
  channel: "chrome",
  headless: true,
  args: ["--no-sandbox", "--disable-dev-shm-usage"],
});

function watch(page) {
  page.on("request", (req) => {
    const body = (req.postData() || "").toLowerCase();
    if (!body) return;
    for (const word of words) {
      if (body.includes(word)) leaks.push(req.url());
    }
    if (body.includes("lentils = 180 g") || body.includes("sesame = 12 g")) leaks.push(req.url());
  });
}

async function openApp(context) {
  const page = await context.newPage();
  watch(page);
  await page.goto(base + "/?sw=1", { waitUntil: "domcontentloaded", timeout: 60000 });
  await page.waitForSelector(".cm-content", { timeout: 60000 });
  await page.waitForFunction(() => document.querySelector(".sync-chip")?.dataset.ready === "1", null, { timeout: 60000 });
  return page;
}

async function openNamed(page, name) {
  await page.locator(".side-name").filter({ hasText: new RegExp(`^${name}$`) }).click();
  await page.waitForTimeout(150);
}

async function openSync(page) {
  const chip = page.locator(".sync-chip");
  if (await chip.isVisible()) await chip.click();
  else {
    await page.keyboard.press("Control+,");
    await page.locator("[data-settings-tab='sync']").click();
  }
  await page.waitForSelector("[data-settings-panel='sync']:not([hidden])", { timeout: 10000 });
}

async function closeSettings(page) {
  await page.locator(".settings-card [data-action='close-settings']").click();
}

async function shot(page, name) {
  await page.screenshot({ path: `${out}/${name}.png` });
}

async function lightLaptop(page) {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.evaluate(() => { document.documentElement.dataset.appTheme = "light"; });
  await page.waitForTimeout(80);
}

async function recover(page) {
  await openSync(page);
  await page.getByRole("button", { name: "Join my existing pages" }).click();
  await page.getByRole("button", { name: "Use my recovery words instead" }).click();
  await page.locator("textarea").fill(words.map((word, index) => `${index + 1}. ${word}`).join("\n"));
  await page.getByRole("button", { name: "Open my pages" }).click();
  await page.locator("[data-sync-set]").waitFor({ timeout: 60000 });
}

const contextA = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: "light" });
const pageA = await openApp(contextA);
await openNamed(pageA, "Trip Budget");
await pageA.locator(".cm-content").click();
await pageA.keyboard.press("Control+End");
await pageA.keyboard.insertText("\nlentils = 180 g");
await pageA.waitForTimeout(300);
await recover(pageA);

const note = (await pageA.locator("[data-sync-note]").innerText()).replace(/\s+/g, " ").trim();
if (note !== NOTE) throw new Error(`warning\n${note}`);
await lightLaptop(pageA);
const box = await pageA.evaluate(() => {
  const panel = document.querySelector("[data-settings-panel='sync']");
  const title = panel.querySelector("h2");
  const message = panel.querySelector("[data-sync-note]");
  const titleBox = title.getBoundingClientRect();
  const noteBox = message.getBoundingClientRect();
  const style = getComputedStyle(message);
  return {
    title: title.textContent,
    titleBottom: titleBox.bottom,
    noteTop: noteBox.top,
    border: style.borderTopWidth,
    background: style.backgroundColor,
  };
});
if (box.title !== "Sync is on") throw new Error(`title ${box.title}`);
if (box.noteTop < box.titleBottom) throw new Error(`note overlaps the title ${JSON.stringify(box)}`);
if (box.border === "0px") throw new Error("note has no border");
await shot(pageA, "stale-warning-laptop");
await pageA.setViewportSize({ width: 390, height: 844 });
await pageA.evaluate(() => { document.documentElement.dataset.appTheme = "dark"; });
await pageA.waitForTimeout(80);
await shot(pageA, "stale-warning-phone");
await lightLaptop(pageA);
await closeSettings(pageA);
await openNamed(pageA, "Trip Budget");
const firstPage = await pageA.locator(".cm-content").innerText();
if (!firstPage.includes("lentils = 180 g")) throw new Error("local line was deleted");
if (firstPage.includes("sesame = 12 g")) throw new Error("the locked line was opened");
await shot(pageA, "stale-page-laptop");

const contextB = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: "light" });
const pageB = await openApp(contextB);
await recover(pageB);
if (await pageB.locator("[data-sync-note]").count()) throw new Error("second recovery showed the warning");
const home = await pageB.locator("[data-settings-panel='sync']").innerText();
if (home.includes("may be missing")) throw new Error("second recovery still mentions a missing change");
await lightLaptop(pageB);
await shot(pageB, "stale-second-laptop");
await closeSettings(pageB);
await openNamed(pageB, "Trip Budget");
const secondPage = await pageB.locator(".cm-content").innerText();
if (!secondPage.includes("lentils = 180 g")) throw new Error("second recovery missed the pages");
if (secondPage.includes("sesame = 12 g")) throw new Error("second recovery opened the locked line");
await shot(pageB, "stale-second-page");

if (leaks.length) throw new Error(`secret on the wire ${leaks.join(" ")}`);
await browser.close();
console.log("stale recovery opened the pages, then a second recovery opened the latest copy");
