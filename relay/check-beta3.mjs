// Sync keeps running after "Your pages are here" is closed, a removed
// phone shows a plain screen, and a long device list does not cover the buttons.
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
  await page.waitForFunction(() => document.querySelector(".sync-chip")?.dataset.ready === "1", null, { timeout: 60000 });
  return page;
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
  if (await page.locator("[data-settings]").isHidden()) return;
  await page.locator(".settings-card [aria-label='Close']").click();
  await page.locator("[data-settings]").waitFor({ state: "hidden" });
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

async function openNamed(page, name) {
  await page.locator(".side-name").filter({ hasText: new RegExp(`^${name}$`) }).click();
}

async function addLine(page, line) {
  await closeSettings(page);
  await openNamed(page, "Trip Budget");
  await page.locator(".cm-content").click();
  await page.keyboard.press("Control+End");
  await page.keyboard.insertText(`\n${line}\n`);
}

async function shot(page, name) {
  await page.screenshot({ path: `${out}/${name}.png` });
}

async function frame(page, theme, width, height) {
  await page.setViewportSize({ width, height });
  await page.evaluate((value) => {
    document.documentElement.dataset.appTheme = value;
  }, theme);
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

async function startFresh(page, inviteCode) {
  await openNamed(page, "Trip Budget");
  await page.locator(".cm-content").click();
  await page.keyboard.press("Control+A");
  await page.keyboard.insertText("# Trip Budget\nferry = 40\n");
  await openSync(page);
  await page.getByRole("button", { name: "This is my first device" }).click();
  await page.locator("[data-access-code]").fill(inviteCode);
  await page.getByRole("button", { name: "Continue" }).click();
  await page.waitForSelector(".sync-word", { timeout: 30000 });
  const words = await wordsOnSheet(page);
  await page.getByRole("checkbox", { name: "I wrote these down" }).check();
  await page.getByRole("button", { name: "I saved them" }).click();
  await pickWords(page, words);
  await page.getByRole("button", { name: "These match" }).click();
  await page.locator("[data-sync-set]").waitFor();
  return words;
}

async function joinFrom(page, host) {
  await openNamed(page, "Trip Budget");
  if (await host.locator("[data-settings]").isHidden()) await openSync(host);
  await host.getByRole("button", { name: "Add a phone or computer" }).click();
  await host.locator(".sync-invite-code").waitFor({ timeout: 20000 });
  const invite = (await host.locator(".sync-invite-code").innerText()).trim();
  await openSync(page);
  await page.getByRole("button", { name: "My pages already sync on another device" }).click();
  await page.locator("[data-join-code]").fill(invite);
  await page.getByRole("button", { name: "Join", exact: true }).click();
  await page.getByRole("heading", { name: "Approve on your other device" }).waitFor({ timeout: 20000 });
  await host.getByRole("heading", { name: "Is this your phone or computer?" }).waitFor({ timeout: 20000 });
  await host.getByRole("button", { name: "Yes, approve it" }).click();
  await page.getByRole("heading", { name: "Do these match?" }).waitFor({ timeout: 20000 });
  await page.getByRole("button", { name: "These match" }).click();
  await page.locator("[data-sync-set]").waitFor({ timeout: 60000 });
  await host.locator("[data-sync-set]").waitFor({ timeout: 60000 });
}

async function padDevices(page) {
  await page.locator(".sync-devices .sync-device").first().waitFor();
  return page.evaluate(() => {
    const list = document.querySelector(".sync-devices");
    const row = list?.querySelector(".sync-device");
    if (!row) return 0;
    list.querySelectorAll("[data-clone]").forEach((node) => node.remove());
    for (let i = 0; i < 8; i += 1) {
      const copy = row.cloneNode(true);
      copy.setAttribute("data-clone", "1");
      const code = copy.querySelector("span");
      if (code) code.textContent = `100 10${i}`;
      list.append(copy);
    }
    return list.querySelectorAll(".sync-device").length;
  });
}

async function assertButtonsClear(page, phone) {
  const result = await page.evaluate(async (onPhone) => {
    const list = document.querySelector(".sync-devices");
    const card = document.querySelector(".settings-card");
    if (list) list.scrollTop = list.scrollHeight;
    if (card && !onPhone) card.scrollTop = card.scrollHeight;
    await new Promise((resolve) => requestAnimationFrame(resolve));
    const buttons = [...document.querySelectorAll(".sync-actions .btn, .sync-back, .sync-link")];
    const rows = [...document.querySelectorAll(".sync-device")];
    const actions = document.querySelector(".sync-actions")?.getBoundingClientRect();
    const cardBox = card?.getBoundingClientRect();
    const hit = (a, b) => a.bottom > b.top + 1 && a.top < b.bottom - 1 && a.right > b.left + 1 && a.left < b.right - 1;
    const bad = [];
    for (const button of buttons) {
      const box = button.getBoundingClientRect();
      if (cardBox && (box.top < cardBox.top - 1 || box.bottom > cardBox.bottom + 1)) {
        bad.push(`outside card ${button.textContent.trim()}`);
      }
      if (onPhone && (box.top < 0 || box.bottom > window.innerHeight + 1)) {
        bad.push(`off screen ${button.textContent.trim()}`);
      }
      for (const row of rows) {
        const rowBox = row.getBoundingClientRect();
        if (rowBox.width < 1 || rowBox.height < 1) continue;
        if (hit(box, rowBox)) bad.push(`${button.textContent.trim()} overlaps a device`);
      }
    }
    if (onPhone && list && actions) {
      const last = [...list.querySelectorAll(".sync-device")].pop()?.getBoundingClientRect();
      if (last && last.height > 0 && last.bottom > actions.top + 1) bad.push("last device sits on the buttons");
    }
    return {
      bad,
      buttons: buttons.map((button) => button.textContent.trim()),
      rows: rows.length,
      listScrolls: !!list && list.scrollHeight > list.clientHeight + 1,
    };
  }, phone);
  if (result.bad.length) throw new Error(`${result.bad.join("; ")} (${result.rows} rows)`);
  for (const label of ["Add a phone or computer", "Turn sync off", "← Back"]) {
    if (!result.buttons.includes(label)) throw new Error(`missing ${label}`);
  }
  if (result.rows < 8) throw new Error(`short list ${result.rows}`);
  if (phone && !result.listScrolls) throw new Error("the phone list does not scroll");
}

const contextA = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: "light" });
const contextB = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: "light" });
const contextC = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: "light" });
const pageA = await openApp(contextA);
const pageB = await openApp(contextB);
const pageC = await openApp(contextC);
const inviteCode = await mint(process.env.ACCESS_SECRET || "omacus-access-test-secret", 1);
if (!inviteCode) throw new Error("could not mint an invite code");

try {
  const words = await startFresh(pageA, inviteCode);
  await joinFrom(pageB, pageA);

  await frame(pageB, "light", 1280, 800);
  await shot(pageB, "joined-laptop");
  await frame(pageB, "dark", 390, 844);
  await shot(pageB, "joined-phone");
  await frame(pageB, "light", 1280, 800);
  const joined = await pageB.locator("[data-settings-panel='sync'] h2").innerText();
  if (joined !== "Sync is on") throw new Error(`joined heading ${joined}`);

  await addLine(pageA, "museum = 12");
  await pageB.locator(".cm-content").getByText("museum = 12").waitFor({ timeout: 20000 });
  const during = await pageB.locator("[data-settings-panel='sync'] h2").innerText();
  if (during !== "Sync is on") throw new Error(`left sync while an edit arrived: ${during}`);

  await pageB.locator(".settings-card [aria-label='Close']").click();
  await pageB.locator("[data-settings]").waitFor({ state: "hidden" });
  await addLine(pageA, "park = 2");
  await pageB.locator(".cm-content").getByText("park = 2").waitFor({ timeout: 20000 });
  await frame(pageB, "light", 1280, 800);
  await shot(pageB, "after-close-laptop");
  await frame(pageB, "dark", 390, 844);
  await shot(pageB, "after-close-phone");
  await frame(pageB, "light", 1280, 800);

  await openSync(pageB);
  const home = await pageB.locator("[data-settings-panel='sync'] h2").innerText();
  if (home !== "Sync is on") throw new Error(`reopened on ${home}`);
  await closeSettings(pageB);

  await joinFrom(pageC, pageA);
  await pageC.keyboard.press("Escape");
  await pageC.locator("[data-settings]").waitFor({ state: "hidden" });
  await addLine(pageA, "lantern = 4");
  await pageC.locator(".cm-content").getByText("lantern = 4").waitFor({ timeout: 20000 });
  await openSync(pageC);
  const afterEsc = await pageC.locator("[data-settings-panel='sync'] h2").innerText();
  if (afterEsc !== "Sync is on") throw new Error(`Escape reopened on ${afterEsc}`);
  await closeSettings(pageC);

  await openSync(pageA);
  await pageA.locator("[data-sync-set]").waitFor();
  await frame(pageA, "light", 1280, 800);
  if (await padDevices(pageA) < 8) throw new Error("could not build a long device list");
  await pageA.locator(".sync-actions").scrollIntoViewIfNeeded();
  await shot(pageA, "devices-laptop");
  await assertButtonsClear(pageA, false);
  await frame(pageA, "dark", 390, 844);
  if (await padDevices(pageA) < 8) throw new Error("could not build a long phone list");
  await shot(pageA, "devices-phone");
  await assertButtonsClear(pageA, true);
  await frame(pageA, "light", 1280, 800);
  await pageA.locator("[data-settings-tab='sync']").click();

  const removes = pageA.getByRole("button", { name: "Remove", exact: true });
  if (await removes.count() < 1) throw new Error("no device to remove");
  await removes.first().click();
  await pageA.getByRole("heading", { name: /^Remove .+\?$/ }).waitFor();
  await pageA.getByRole("button", { name: "Remove", exact: true }).click();
  await pageA.locator("[data-sync-set]").waitFor({ timeout: 20000 });

  const removedPage = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("neither phone showed Sync paused")), 22000);
    for (const page of [pageB, pageC]) {
      page.locator(".sync-chip").filter({ hasText: "Sync paused" }).waitFor({ timeout: 20000 }).then(() => {
        clearTimeout(timer);
        resolve(page);
      }, () => {});
    }
  });
  await openSync(removedPage);
  await removedPage.locator("[data-sync-removed]").waitFor({ timeout: 20000 });
  const removedText = await removedPage.locator("[data-settings-panel='sync']").innerText();
  if (!removedText.includes("was removed from sync")) throw new Error(`removed heading missing: ${removedText}`);
  if (!removedText.includes("Your pages are still here.")) throw new Error(`removed copy missing: ${removedText}`);
  if (removedText.includes("Sync is on")) throw new Error("removed screen still says Sync is on");
  if (removedText.includes("Turn sync off")) throw new Error("removed screen offers Turn sync off");
  if (await removedPage.getByRole("button", { name: "Remove", exact: true }).count()) {
    throw new Error("removed screen still has Remove");
  }
  await frame(removedPage, "light", 1280, 800);
  await shot(removedPage, "removed-laptop");
  await frame(removedPage, "dark", 390, 844);
  await shot(removedPage, "removed-phone");
  const phoneHeading = await removedPage.locator("[data-settings-panel='sync'] h2").innerText();
  if (phoneHeading !== "This phone was removed from sync") throw new Error(`phone heading ${phoneHeading}`);
  await frame(removedPage, "light", 1280, 800);
  const laptopHeading = await removedPage.locator("[data-settings-panel='sync'] h2").innerText();
  if (laptopHeading !== "This computer was removed from sync") throw new Error(`laptop heading ${laptopHeading}`);

  await removedPage.getByRole("button", { name: "Join again", exact: true }).click();
  await removedPage.getByRole("heading", { name: "Join my existing pages" }).waitFor();
  const joinText = await removedPage.locator("[data-settings-panel='sync']").innerText();
  if (joinText.includes("Turn sync off")) throw new Error("Join again asked to turn sync off");
  await removedPage.getByRole("button", { name: "Back", exact: true }).click();
  await removedPage.locator("[data-sync-removed]").waitFor();

  await removedPage.reload({ waitUntil: "domcontentloaded" });
  await removedPage.waitForSelector(".sync-chip", { timeout: 60000 });
  await removedPage.locator(".sync-chip").filter({ hasText: "Sync paused" }).waitFor({ timeout: 20000 });
  await openSync(removedPage);
  await removedPage.locator("[data-sync-removed]").waitFor({ timeout: 20000 });

  await addLine(pageA, "bread $2");
  const contextR = await browser.newContext({ viewport: { width: 390, height: 844 }, colorScheme: "dark" });
  const pageR = await openApp(contextR);
  await openSync(pageR);
  await pageR.getByRole("button", { name: "My pages already sync on another device" }).click();
  await pageR.getByRole("button", { name: "Use recovery words" }).click();
  await pageR.locator("textarea").fill(words.map((word, index) => `${index + 1}. ${word}`).join("\n"));
  await frame(pageR, "dark", 390, 844);
  await shot(pageR, "recover-words-phone");
  await pageR.getByRole("button", { name: "Open my pages" }).click();
  await pageR.locator("[data-sync-set]").waitFor({ timeout: 60000 });
  if (await pageR.locator(".sync-choice").count()) throw new Error("recovery asked to match words");
  const recoveredHeading = await pageR.locator("[data-settings-panel='sync'] h2").innerText();
  if (recoveredHeading !== "Sync is on") throw new Error(`recovery landed on ${recoveredHeading}`);
  await closeSettings(pageR);
  await frame(pageR, "dark", 1280, 800);
  await openNamed(pageR, "Trip Budget");
  await pageR.locator(".cm-content").getByText("bread $2").waitFor({ timeout: 20000 });
  await frame(pageR, "dark", 390, 844);
  await shot(pageR, "recover-pages-phone");
  await contextR.close();

  await frame(pageA, "dark", 390, 844);
  await pageA.locator(".side-name").first().waitFor();
  const version = (await pageA.locator(".side-version").innerText()).trim();
  if (version !== "omacus 1.0.0-beta.42") throw new Error(`version line ${version}`);
  const beta = await pageA.locator(".sync-beta").innerText();
  if (!beta.includes("1.0.0-beta.42")) throw new Error(`phone label ${beta}`);
  const panelLabel = await removedPage.locator(".sync-label").innerText();
  if (!panelLabel.includes("1.0.0-beta.42")) throw new Error(`panel label ${panelLabel}`);

  console.log("sync stays on after close, a removed phone can join again, and the device list stays clear of the buttons");
} catch (error) {
  await pageA.screenshot({ path: `${out}/fail-beta3-a.png` }).catch(() => {});
  await pageB.screenshot({ path: `${out}/fail-beta3-b.png` }).catch(() => {});
  await pageC.screenshot({ path: `${out}/fail-beta3-c.png` }).catch(() => {});
  console.error(error);
  process.exitCode = 1;
} finally {
  await browser.close();
}
