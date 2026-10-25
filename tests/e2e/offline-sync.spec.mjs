import { spawn } from "node:child_process";
import { expect, test } from "@playwright/test";

const BOX = "http://127.0.0.1:8799";

function startBox() {
  const child = spawn(process.execPath, ["tests/e2e/sync-box.mjs"], { stdio: ["ignore", "pipe", "inherit"] });
  const ready = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("sync box did not start")), 5000);
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

function routed(offline) {
  return async (route) => {
    if (offline.on) {
      await route.abort("internetdisconnected");
      return;
    }
    await handle(route);
  };
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
  // WebKit refuses a fulfilled 304, so the mailbox is always read in full.
  // The app still skips a copy it already has.
  headers.delete("if-none-match");
  const method = request.method();
  const response = await fetch(BOX + url.pathname, {
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

async function replacePage(page, text) {
  const editor = page.locator(".cm-content");
  await editor.click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.insertText(text);
}

test("an offline edit is merged with the other phone", async ({ browser }) => {
  test.setTimeout(180000);
  const box = startBox();
  await box.ready;
  try {
  await fetch(BOX + "/reset", { method: "POST" });
  const first = await browser.newContext();
  const second = await browser.newContext();
  const phoneBOffline = { on: false };
  await first.route("**/v1/**", routed({ on: false }));
  await second.route("**/v1/**", routed(phoneBOffline));
  const phoneA = await first.newPage();
  const phoneB = await second.newPage();

  await phoneA.goto("/app/", { waitUntil: "domcontentloaded" });
  await phoneA.waitForSelector("[data-loading]", { state: "detached" });
  await syncReady(phoneA);
  await expect(phoneA.locator(".sync-chip")).toBeHidden();
  await phoneA.getByRole("button", { name: "Turn on sync" }).first().click();
  await expect(phoneA.getByRole("heading", { name: "How sync works" })).toBeVisible();
  await phoneA.getByRole("button", { name: "Set up sync" }).click();
  await phoneA.getByRole("button", { name: "Start fresh" }).click();
  await phoneA.locator("[data-access-code]").fill("FERRY-40");
  await phoneA.getByRole("button", { name: "Start fresh" }).click();
  await expect(phoneA.locator(".sync-word").first()).toBeVisible();
  const words = await phoneA.locator(".sync-word").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-word")));
  await phoneA.getByRole("button", { name: "I saved them" }).click();
  for (let step = 0; step < 3; step += 1) {
    const legend = phoneA.locator(".sync-ask").nth(step).locator("legend");
    const n = Number((await legend.innerText()).replace(/\D/g, ""));
    await phoneA.locator(".sync-ask").nth(step).getByRole("button", { name: words[n - 1], exact: true }).click();
  }
  await phoneA.getByRole("button", { name: "These match" }).click();
  await expect(phoneA.locator("[data-sync-linked]")).toBeVisible();
  await phoneA.locator("[data-action='close-settings']").last().click();
  await phoneA.getByRole("button", { name: "New page" }).click();
  const saved = phoneA.waitForResponse((res) => res.request().method() === "PUT" && res.url().includes("/v1/mailboxes/") && res.status() === 200);
  await replacePage(phoneA, "Harbor ledger\nferry = 40\n");
  await saved;
  await expect(phoneA.locator(".sync-chip")).toHaveAttribute("data-state", "on", { timeout: 20000 });

  await openSync(phoneB);
  await phoneB.getByRole("button", { name: "Join my existing pages" }).click();
  await phoneB.getByRole("button", { name: "Use my recovery words instead" }).click();
  await phoneB.locator("[data-recover-name]").fill("Harbor phone");
  await phoneB.locator(".sync-phrase").fill(words.join(" "));
  await phoneB.getByRole("button", { name: "Open my pages" }).click();
  await expect(phoneB.locator("[data-sync-linked]")).toBeVisible({ timeout: 20000 });
  await phoneB.locator("[data-action='close-settings']").last().click();
  await phoneB.locator(".side-name", { hasText: "Harbor ledger" }).click();
  await expect(phoneB.locator(".cm-content")).toContainText("ferry = 40", { timeout: 20000 });
  await expect(phoneB.locator(".sync-chip")).not.toHaveAttribute("data-state", "error");

  await expect(phoneA.locator(".sync-chip")).toHaveAttribute("data-state", "on", { timeout: 20000 });
  await phoneA.locator(".side-name", { hasText: "Harbor ledger" }).click();

  phoneBOffline.on = true;
  await second.setOffline(true);
  await replacePage(phoneB, "Harbor ledger\nferry = 40\ncake $5\n");
  await expect(phoneB.locator(".cm-content")).toContainText("cake $5");
  const breadSaved = phoneA.waitForResponse((res) => res.request().method() === "PUT" && res.url().includes("/v1/mailboxes/") && res.status() === 200);
  await replacePage(phoneA, "Harbor ledger\nferry = 40\nbread $2\n");
  await expect(phoneA.locator(".cm-content")).toContainText("bread $2");
  await breadSaved;
  phoneBOffline.on = false;
  await second.setOffline(false);
  await expect(phoneB.locator(".cm-content")).toContainText("bread $2", { timeout: 30000 });
  await expect(phoneB.locator(".cm-content")).toContainText("cake $5");
  await expect(phoneA.locator(".cm-content")).toContainText("cake $5", { timeout: 30000 });
  await expect(phoneA.locator(".cm-content")).toContainText("bread $2");
  await expect(phoneA.locator(".sync-chip")).not.toHaveAttribute("data-state", "error");

  await first.close();
  await second.close();
  } finally {
    box.child.kill();
  }
});
