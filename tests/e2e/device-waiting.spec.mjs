// A phone asks to join while Sync is closed. The status says Device waiting
// in the top bar and the sidebar, on a wide window and on a phone. Tapping
// it opens the approve prompt. The cue leaves when that phone stops waiting,
// and again after Yes.
import { existsSync, mkdirSync } from "node:fs";
import { spawn } from "node:child_process";
import { expect, test } from "@playwright/test";

const BOX = "http://127.0.0.1:8798";
const DESKTOP_APP = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) omacus-desktop/1.0";

function startBox() {
  const child = spawn(process.execPath, ["--import", "./relay/worker/register-cf.mjs", "tests/e2e/relay-box.mjs"], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  const ready = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("sync box did not start")), 8000);
    child.stdout.on("data", (buf) => {
      if (String(buf).includes("ready")) {
        clearTimeout(timer);
        resolve();
      }
    });
    child.once("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`sync box exited ${code}`));
    });
  });
  return { child, ready };
}

async function handle(route) {
  const request = route.request();
  const url = new URL(request.url());
  if (!url.pathname.startsWith("/v1/")) {
    await route.continue();
    return;
  }
  const headers = new Headers();
  for (const [key, value] of Object.entries(request.headers())) {
    if (key === "host" || key === "content-length" || key === "connection") continue;
    headers.set(key, value);
  }
  headers.delete("if-none-match");
  const method = request.method();
  const response = await fetch(BOX + url.pathname + url.search, {
    method,
    headers,
    body: method === "GET" || method === "HEAD" ? undefined : request.postDataBuffer(),
  });
  await route.fulfill({
    status: response.status,
    contentType: response.headers.get("content-type") || "application/json",
    body: Buffer.from(await response.arrayBuffer()),
  });
}

async function syncReady(page) {
  await page.waitForFunction(() => typeof globalThis.omacusTurnOnSync === "function");
}

async function openSync(page) {
  await page.goto("/app/", { waitUntil: "domcontentloaded" });
  await page.waitForSelector("[data-loading]", { state: "detached" });
  await syncReady(page);
  await page.getByRole("button", { name: "Settings" }).click();
  await page.locator("[data-settings-tab='sync']").click();
}

async function joinWith(page, code, name) {
  await openSync(page);
  await page.getByRole("button", { name: "My pages already sync on another device" }).click();
  await page.getByRole("textbox", { name: "Name this phone or computer" }).fill(name);
  await page.locator("[data-join-code]").fill(code);
  await page.getByRole("button", { name: "Join", exact: true }).click();
}

async function shot(page, name) {
  const dir = "/opt/cursor/artifacts/screenshots";
  if (!existsSync("/opt/cursor/artifacts")) return;
  mkdirSync(dir, { recursive: true });
  await page.screenshot({ path: `${dir}/${name}.png` });
}

test("a closed Sync panel still shows a phone that is waiting", async ({ browser }) => {
  test.setTimeout(240000);
  const box = startBox();
  await box.ready;
  const contexts = [];
  try {
    const minted = await fetch(BOX + "/v1/admin/codes", {
      method: "POST",
      headers: {
        authorization: "Bearer admin-test-key-value",
        "content-type": "application/json",
      },
      body: JSON.stringify({ uses: 5, label: "Harbor", expires: "2099-01-01" }),
    });
    const mintedText = await minted.text();
    if (!minted.ok) throw new Error(`access code ${minted.status} ${mintedText}`);
    const access = JSON.parse(mintedText).code;

    const first = await browser.newContext({
      userAgent: DESKTOP_APP,
      viewport: { width: 1280, height: 800 },
    });
    contexts.push(first);
    await first.route("**/v1/**", handle);
    const phoneA = await first.newPage();

    await phoneA.goto("/app/", { waitUntil: "domcontentloaded" });
    await phoneA.waitForSelector("[data-loading]", { state: "detached" });
    await syncReady(phoneA);
    await phoneA.getByRole("button", { name: "Turn on sync" }).first().click();
    await expect(phoneA.getByRole("heading", { name: "How sync works" })).toBeVisible();
    await phoneA.getByRole("button", { name: "Set up sync" }).click();
    await phoneA.getByRole("button", { name: "This is my first device" }).click();
    await phoneA.locator("[data-access-code]").fill(access);
    await phoneA.getByRole("button", { name: "Continue" }).click();
    await expect(phoneA.locator(".sync-word").first()).toBeVisible();
    const words = await phoneA.locator(".sync-word").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-word")));
    await phoneA.getByRole("checkbox", { name: "I wrote these down" }).check();
    await phoneA.getByRole("button", { name: "I saved them" }).click();
    for (let step = 0; step < 3; step += 1) {
      const legend = phoneA.locator(".sync-ask").nth(step).locator("legend");
      const n = Number((await legend.innerText()).replace(/\D/g, ""));
      await phoneA.locator(".sync-ask").nth(step).getByRole("button", { name: words[n - 1], exact: true }).click();
    }
    await phoneA.getByRole("button", { name: "These match" }).click();
    await expect(phoneA.locator("[data-sync-linked]")).toBeVisible();
    await phoneA.getByRole("button", { name: "Add a phone or computer" }).click();
    const code = (await phoneA.locator(".sync-invite-code").innerText()).trim();
    await phoneA.locator("[data-action='close-settings']").last().click();
    await expect(phoneA.getByRole("heading", { name: "Is this Harbor phone?" })).toBeHidden();

    const chip = phoneA.locator(".sync-chip");
    const side = phoneA.locator(".side-foot-row[data-device-waiting]");
    await expect(chip).toHaveText("Synced");
    await expect(side).toBeHidden();
    await shot(phoneA, "device-waiting-before-desktop-light");

    const harbor = await browser.newContext();
    contexts.push(harbor);
    await harbor.route("**/v1/**", handle);
    const phoneB = await harbor.newPage();
    await joinWith(phoneB, code, "Harbor phone");

    await expect(chip).toHaveText("Device waiting", { timeout: 45000 });
    await expect(chip).toHaveAttribute("data-state", "ask");
    await expect(side).toBeVisible();
    await expect(phoneA.getByRole("heading", { name: "Is this Harbor phone?" })).toBeHidden();
    await shot(phoneA, "device-waiting-after-desktop-light");
    await phoneA.evaluate(() => {
      document.documentElement.dataset.appTheme = "dark";
    });
    await shot(phoneA, "device-waiting-after-desktop-dark");
    await phoneA.evaluate(() => {
      document.documentElement.dataset.appTheme = "light";
    });

    await phoneA.getByRole("button", { name: "Collapse sidebar" }).click();
    await expect(phoneA.locator(".rail [data-device-waiting]")).toBeVisible();
    await phoneA.getByRole("button", { name: "Expand sidebar" }).click();
    await expect(side).toBeVisible();

    await phoneA.setViewportSize({ width: 390, height: 844 });
    await expect(chip).toHaveText("Device waiting");
    await phoneA.locator(".sheets-toggle").click();
    await expect(side).toBeVisible();
    await shot(phoneA, "device-waiting-after-phone-light");
    await phoneA.evaluate(() => {
      document.documentElement.dataset.appTheme = "dark";
    });
    await shot(phoneA, "device-waiting-after-phone-dark");
    await phoneA.evaluate(() => {
      document.documentElement.dataset.appTheme = "light";
    });
    await phoneA.locator(".sheets-toggle").click();
    await phoneA.setViewportSize({ width: 1280, height: 800 });

    await chip.click();
    await expect(phoneA.getByRole("heading", { name: "Is this Harbor phone?" })).toBeVisible();
    await expect(phoneA.getByRole("button", { name: "Yes, approve it" })).toBeVisible();
    await phoneA.locator("[data-action='close-settings']").last().click();
    await expect(chip).toHaveText("Device waiting");

    await phoneB.close();
    await harbor.close();
    contexts.pop();
    await new Promise((resolve) => setTimeout(resolve, 1500));
    const clocked = await fetch(BOX + "/__clock", { method: "POST" });
    if (!clocked.ok) throw new Error(await clocked.text());
    await expect(chip).toHaveText("Synced", { timeout: 20000 });
    await expect(side).toBeHidden();
    await expect(phoneA.locator(".rail [data-device-waiting]")).toBeHidden();

    const ferry = await browser.newContext();
    contexts.push(ferry);
    await ferry.route("**/v1/**", handle);
    const phoneC = await ferry.newPage();
    await joinWith(phoneC, code, "Ferry phone");
    await expect(chip).toHaveText("Device waiting", { timeout: 50000 });
    await expect(side).toBeVisible();
    await side.click();
    await expect(phoneA.getByRole("heading", { name: "Is this Ferry phone?" })).toBeVisible();
    await phoneA.getByRole("button", { name: "Yes, approve it" }).click();
    await expect(phoneC.getByRole("heading", { name: "Do these match?" })).toBeVisible({ timeout: 30000 });
    await phoneC.getByRole("button", { name: "These match" }).click();
    await expect(phoneC.locator("[data-sync-linked]")).toBeVisible({ timeout: 20000 });
    await expect(chip).toHaveText("Synced", { timeout: 30000 });
    await expect(side).toBeHidden();
    await expect(phoneA.getByText("That code didn't work")).toHaveCount(0);
    await expect(phoneC.getByText("That code didn't work")).toHaveCount(0);
  } finally {
    for (const context of contexts) await context.close().catch(() => {});
    box.child.kill();
  }
});
