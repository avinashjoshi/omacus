// Remove asks first, and Keep it leaves the phone or computer in place.
import fs from "node:fs";
import { chromium } from "playwright";
import { mint } from "./worker/access.js";

const base = process.argv[2] || "http://127.0.0.1:8800";
const out = "/opt/cursor/artifacts/sync-test";
fs.mkdirSync(out, { recursive: true });

const browser = await chromium.launch({
  channel: "chrome",
  headless: true,
  args: ["--no-sandbox", "--disable-dev-shm-usage", "--use-fake-ui-for-media-stream", "--use-fake-device-for-media-stream"],
});

async function openApp(context) {
  const page = await context.newPage();
  await page.goto(base + "/?sw=1", { waitUntil: "domcontentloaded", timeout: 60000 });
  await page.waitForSelector(".cm-content", { timeout: 60000 });
  await page.waitForSelector(".sync-chip", { timeout: 60000 });
  return page;
}

async function openSync(page) {
  await page.locator(".sync-chip").click();
  await page.waitForSelector("[data-settings-panel='sync']:not([hidden])", { timeout: 10000 });
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
const contextB = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: "light" });
const pageA = await openApp(contextA);
const pageB = await openApp(contextB);
const inviteCode = await mint(process.env.ACCESS_SECRET || "omacus-access-test-secret", 1);
if (!inviteCode) throw new Error("could not mint an invite code");

try {
  await openSync(pageA);
  await pageA.getByRole("button", { name: "Start fresh" }).click();
  await pageA.locator("[data-access-code]").fill(inviteCode);
  await pageA.getByRole("button", { name: "Start fresh" }).click();
  await pageA.waitForSelector(".sync-word", { timeout: 30000 });
  const words = await wordsOnSheet(pageA);
  await pageA.getByRole("button", { name: "I wrote them down" }).click();
  await pickWords(pageA, words);
  await pageA.getByRole("button", { name: "These match" }).click();
  await pageA.getByRole("button", { name: "Done" }).click();
  await pageA.locator("[data-sync-set]").waitFor();

  await pageA.getByRole("button", { name: "Add a phone or computer" }).click();
  await pageA.locator(".sync-invite-code").waitFor({ timeout: 20000 });
  const invite = (await pageA.locator(".sync-invite-code").innerText()).trim();

  await openSync(pageB);
  await pageB.getByRole("button", { name: "Join my existing pages" }).click();
  await pageB.locator("[data-join-code]").fill(invite);
  await pageB.getByRole("button", { name: "Join", exact: true }).click();
  await pageB.locator(".sync-code").waitFor({ timeout: 20000 });
  await pageA.getByRole("heading", { name: "Is this your phone or computer?" }).waitFor({ timeout: 20000 });
  await pageA.getByRole("button", { name: "Yes, approve it" }).click();
  await pageA.getByRole("heading", { name: "That phone or computer can open your pages" }).waitFor({ timeout: 60000 });
  await pageA.getByRole("button", { name: "Done" }).click();
  await pageA.locator("[data-sync-set]").waitFor();

  const removes = pageA.getByRole("button", { name: "Remove", exact: true });
  if (await removes.count() !== 1) throw new Error(`expected one Remove, saw ${await removes.count()}`);
  const selfText = await pageA.locator(".sync-device").first().innerText();
  if (!selfText.includes("This computer")) throw new Error(`self row: ${selfText}`);
  await removes.click();
  await pageA.getByRole("heading", { name: "Remove this phone or computer?" }).waitFor();
  const help = await pageA.locator("[data-settings-panel='sync']").innerText();
  if (!help.includes("It will stop getting your pages. Pages already on it stay there.")) {
    throw new Error(`confirm copy missing: ${help}`);
  }
  const shown = (await pageA.locator(".sync-code").innerText()).trim();
  if (!/^\d{3} \d{3}$/.test(shown)) throw new Error(`confirm code ${shown}`);
  if (!help.includes("Phone or computer")) throw new Error("confirm did not name the phone or computer");

  await pageA.setViewportSize({ width: 1280, height: 800 });
  await pageA.evaluate(() => { document.documentElement.dataset.appTheme = "light"; });
  await shot(pageA, "remove-confirm-laptop");
  await pageA.setViewportSize({ width: 390, height: 844 });
  await pageA.evaluate(() => { document.documentElement.dataset.appTheme = "dark"; });
  await pageA.waitForTimeout(100);
  await shot(pageA, "remove-confirm-phone");
  await pageA.setViewportSize({ width: 1280, height: 800 });
  await pageA.evaluate(() => { document.documentElement.dataset.appTheme = "light"; });

  await pageA.getByRole("button", { name: "Keep it", exact: true }).click();
  await pageA.locator("[data-sync-set]").waitFor();
  if (await pageA.getByRole("button", { name: "Remove", exact: true }).count() !== 1) {
    throw new Error("Keep it removed the phone or computer");
  }

  await pageA.getByRole("button", { name: "Remove", exact: true }).click();
  await pageA.getByRole("heading", { name: "Remove this phone or computer?" }).waitFor();
  await pageA.getByRole("button", { name: "Remove", exact: true }).click();
  await pageA.locator("[data-sync-set]").waitFor({ timeout: 20000 });
  if (await pageA.getByRole("button", { name: "Remove", exact: true }).count() !== 0) {
    throw new Error("Remove left the phone or computer on the list");
  }
  const names = await pageA.locator(".side-name").allTextContents();
  const trips = names.filter((name) => name.trim() === "Trip Budget");
  if (trips.length !== 1) throw new Error(`Trip Budget count ${trips.length}: ${names.join(" | ")}`);
  console.log("remove confirm kept, then removed, and Trip Budget stayed one page");
} catch (error) {
  await pageA.screenshot({ path: `${out}/fail-remove.png` }).catch(() => {});
  console.error(error);
  process.exitCode = 1;
} finally {
  await browser.close();
}
