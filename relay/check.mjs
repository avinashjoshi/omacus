// Drives the private sync test in a browser. Not part of the app's e2e suite.
import { chromium } from "@playwright/test";
import fs from "node:fs";
import { mint } from "./worker/access.js";

const base = process.env.SYNC_TEST_URL || "http://127.0.0.1:8787";
const out = "/opt/cursor/artifacts/screenshots";
fs.mkdirSync(out, { recursive: true });

const browser = await chromium.launch();
const founder = await browser.newContext({ viewport: { width: 1280, height: 800 } });
const joiner = await browser.newContext({ viewport: { width: 1280, height: 800 } });
const recover = await browser.newContext({ viewport: { width: 1280, height: 800 } });
for (const context of [founder, joiner, recover]) {
  context.on("request", (request) => {
    if (request.url().includes("omacus.com")) {
      throw new Error(`live site requested: ${request.url()}`);
    }
  });
}
const pageA = await founder.newPage();
const pageB = await joiner.newPage();
const pageC = await recover.newPage();

function guard(page) {
  page.on("pageerror", (error) => {
    throw error;
  });
}
guard(pageA);
guard(pageB);
guard(pageC);

const access = await mint(process.env.ACCESS_SECRET || "omacus-access-test-secret", 1);
if (!access) throw new Error("could not mint an invite code");
await pageA.goto(base);
await pageA.getByRole("button", { name: "Start fresh" }).click();
await pageA.locator("#access-code").fill("nope");
await pageA.getByRole("button", { name: "Start fresh" }).click();
await pageA.getByText("That invite code didn't work").waitFor({ timeout: 10000 });
await pageA.locator("#access-code").fill(access);
await pageA.getByRole("button", { name: "Start fresh" }).click();
await pageA.locator("[data-screen=save]").waitFor();
const words = [];
for (let n = 1; n <= 12; n += 1) {
  words.push(await pageA.locator(`.word[data-n="${n}"]`).getAttribute("data-word"));
}
if (words.some((word) => !word)) throw new Error("missing words");
await pageA.getByRole("button", { name: "I wrote them down" }).click();
await pageA.locator("[data-screen=confirm]").waitFor();
const legendsA = await pageA.locator("fieldset legend").allTextContents();
for (const legend of legendsA) {
  const n = Number(legend.replace(/\D/g, ""));
  await pageA.locator(`.choice[data-n="${n}"][data-word="${words[n - 1]}"]`).click();
}
await pageA.getByRole("button", { name: "These match" }).click();
await pageA.locator("[data-screen=ready]").waitFor({ timeout: 30000 });
await pageA.getByText("ferry = 40").waitFor();
await pageA.getByRole("button", { name: "Add a phone or computer" }).click();
await pageA.locator("[data-screen=invite]").waitFor();
const invite = (await pageA.locator(".invite-code").textContent()).trim();
if (!/^[0-9A-Z]{4}-[0-9A-Z]{4}$/.test(invite)) throw new Error(`invite ${invite}`);

await pageB.goto(base);
await pageB.getByRole("button", { name: "Join my existing pages" }).click();
await pageB.locator("#join-code").fill("0000-0001");
await pageB.getByRole("button", { name: "Join", exact: true }).click();
await pageB.getByText("That code didn't work").waitFor({ timeout: 10000 });
await pageA.waitForTimeout(1200);
if (await pageA.locator("[data-screen=approve]").count()) throw new Error("a bad code opened approve");
await pageB.locator("#join-code").fill(invite);
await pageB.getByRole("button", { name: "Join", exact: true }).click();
await pageB.locator("[data-screen=join]").waitFor();
const code = (await pageB.locator(".code").textContent()).trim();
if (!/^\d{3} \d{3}$/.test(code)) throw new Error(`bad code ${code}`);
await pageA.locator("[data-screen=approve]").waitFor({ timeout: 10000 });
const shown = (await pageA.locator(".code").textContent()).trim();
if (shown !== code) throw new Error(`codes differ ${shown} ${code}`);

async function shot(page, name) {
  const themes = ["dark", "light"];
  const sizes = [
    ["desktop", 1280, 800],
    ["phone", 390, 844],
  ];
  for (const theme of themes) {
    await page.evaluate((value) => {
      document.documentElement.dataset.appTheme = value;
    }, theme);
    for (const [label, width, height] of sizes) {
      await page.setViewportSize({ width, height });
      await page.screenshot({ path: `${out}/sync-${name}-${label}-${theme}.png` });
    }
  }
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.evaluate(() => {
    document.documentElement.dataset.appTheme = "dark";
  });
}

await shot(pageA, "approve");

await pageA.getByRole("button", { name: "Yes, approve it" }).click();
await pageB.locator("[data-screen=join-confirm]").waitFor({ timeout: 20000 });
await pageB.getByRole("button", { name: "These match" }).click();
await pageB.locator("[data-screen=opened]").waitFor({ timeout: 20000 });
await pageB.getByText("milk $3").waitFor();
await pageA.locator("[data-screen=ready]").waitFor();

await pageC.goto(base);
await pageC.getByRole("button", { name: "Join my existing pages" }).click();
await pageC.getByRole("button", { name: "Use my recovery key instead" }).click();
await pageC.locator("[data-screen=recover]").waitFor();
await pageC.getByRole("button", { name: "Open my pages" }).click();
await pageC.locator("#phrase-msg", { hasText: "That's 0 words." }).waitFor();
const messy = [
  `1. ${words[0][0].toUpperCase()}${words[0].slice(1)}`,
  `2. ${words[1]}, ${words[2]}`,
  words[3],
  `${words[4]} ${words[5]}`,
  `6. ${words[6]}`,
  words[7],
  words[8],
  words[9],
  `10. ${words[10]}`,
  words[11],
].join("\n");
await pageC.locator("#phrase").fill(messy);
await pageC.getByText("Looks right. 12 words.").waitFor();
await shot(pageC, "recover");
await pageC.getByRole("button", { name: "Open my pages" }).click();
await pageC.locator("[data-screen=recover-confirm]").waitFor();
const legendsC = await pageC.locator("fieldset legend").allTextContents();
for (const legend of legendsC) {
  const n = Number(legend.replace(/\D/g, ""));
  await pageC.locator(`.choice[data-n="${n}"][data-word="${words[n - 1]}"]`).click();
}
await pageC.getByRole("button", { name: "These match" }).click();
await pageC.locator("[data-screen=opened]").waitFor({ timeout: 30000 });
await pageC.getByText("Your pages are here.").waitFor();
await pageC.getByText("ferry = 40").waitFor();

for (const page of [pageA, pageB, pageC]) {
  const names = await page.evaluate(async () => {
    if (!indexedDB.databases) return [];
    return (await indexedDB.databases()).map((db) => db.name);
  });
  if (names.length) throw new Error(`database opened: ${names.join(", ")}`);
}

await browser.close();
console.log("sync test passed");
