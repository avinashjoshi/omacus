// Turn sync off stores a list that drops this phone. The other phone's list
// loses that row on its next poll. The top bar stays quiet, and Turn on sync
// comes back. The page that was already here stays.
import { existsSync, mkdirSync } from "node:fs";
import { spawn } from "node:child_process";
import { expect, test } from "@playwright/test";
import { POLL_IDLE_MS } from "../../site/app/sync-save.mjs";

// WebKit sometimes misses one approval poll. A retry is the same check again.
test.describe.configure({ retries: process.env.CI ? 2 : 0 });

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

async function shot(page, name) {
  const dir = "/opt/cursor/artifacts/screenshots";
  if (!existsSync("/opt/cursor/artifacts")) return;
  mkdirSync(dir, { recursive: true });
  await page.screenshot({ path: `${dir}/${name}.png` });
}

test("turning sync off drops this phone from the other list within one poll", async ({ browser }) => {
  test.setTimeout(180000);
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

    const harbor = await browser.newContext({
      userAgent: DESKTOP_APP,
      viewport: { width: 1280, height: 800 },
    });
    contexts.push(harbor);
    await harbor.route("**/v1/**", handle);
    const phoneB = await harbor.newPage();
    await openSync(phoneB);
    await phoneB.getByRole("button", { name: "My pages already sync on another device" }).click();
    await phoneB.getByRole("textbox", { name: "Name this phone or computer" }).fill("Harbor phone");
    await phoneB.locator("[data-join-code]").fill(code);
    await phoneB.getByRole("button", { name: "Join", exact: true }).click();

    await expect(phoneA.getByRole("heading", { name: "Is this Harbor phone?" })).toBeVisible({ timeout: 45000 });
    await phoneA.getByRole("button", { name: "Yes, approve it" }).click();
    await expect(phoneB.getByRole("heading", { name: "Do these match?" })).toBeVisible({ timeout: 30000 });
    await phoneB.getByRole("button", { name: "These match" }).click();
    await expect(phoneB.locator("[data-sync-linked]")).toBeVisible({ timeout: 45000 });
    await expect(phoneA.getByText("Harbor phone")).toBeVisible({ timeout: 45000 });

    await phoneB.locator("[data-action='close-settings']").last().click();
    await phoneB.locator(".cm-content").click();
    await phoneB.keyboard.type("ferry = 40");
    await expect(phoneB.locator(".cm-content")).toContainText("ferry = 40");
    await phoneB.getByRole("button", { name: "Settings" }).click();
    await phoneB.locator("[data-settings-tab='sync']").click();
    await phoneB.getByRole("button", { name: "Turn sync off" }).click();
    await expect(phoneB.getByRole("heading", { name: "Turn sync off on this computer?" })).toBeVisible();
    await expect(phoneB.getByText("If this is the last one, that is fine.")).toBeVisible();
    await shot(phoneB, "turn-off-confirm-light");
    await phoneB.evaluate(() => { document.documentElement.dataset.appTheme = "dark"; });
    await shot(phoneB, "turn-off-confirm-dark");
    await phoneB.evaluate(() => { document.documentElement.dataset.appTheme = "light"; });
    await phoneB.getByRole("button", { name: "Turn sync off", exact: true }).click();
    await expect(phoneB.getByRole("heading", { name: "Turn on sync" })).toBeVisible({ timeout: 20000 });
    // The saving timer is about a second. It used to show Syncing again after turn-off.
    await phoneB.waitForTimeout(1200);
    await expect(phoneB.locator(".sync-chip")).toBeHidden();
    await expect(phoneB.locator(".sync-chip")).not.toHaveText(/Sync paused/);
    await expect(phoneB.locator("[data-turn-on-sync]").first()).toBeVisible();
    await expect(phoneB.locator(".cm-content")).toContainText("ferry = 40");
    await shot(phoneB, "turn-off-quiet-light");
    await phoneB.evaluate(() => { document.documentElement.dataset.appTheme = "dark"; });
    await shot(phoneB, "turn-off-quiet-dark");

    await expect(phoneA.getByText("Harbor phone")).toHaveCount(0, { timeout: POLL_IDLE_MS + 8000 });
    await expect(phoneA.locator("[data-sync-linked]")).toBeVisible();
  } finally {
    for (const context of contexts) await context.close().catch(() => {});
    box.child.kill();
  }
});
