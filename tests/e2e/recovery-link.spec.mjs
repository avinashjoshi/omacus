// A camera opens the recovery link. The words are filled in, the address bar
// drops the fragment, and no request carries the words. A stored pause stays
// off those setup screens.
import { spawn } from "node:child_process";
import { expect, test } from "@playwright/test";
import { encodeFragment } from "../../site/app/sync-link.mjs";

const PHRASE = "other contact online these service state email music product system policy number";
const BOX = "http://127.0.0.1:8798";

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

test("a recovery link fills the words and no request carries them", async ({ page }) => {
  const encoded = encodeFragment(PHRASE);
  const seen = [];
  page.on("request", (request) => {
    seen.push(`${request.method()} ${request.url()}\n${request.postData() || ""}`);
  });
  await page.goto(`/app/#recover=${encoded}`, { waitUntil: "domcontentloaded" });
  await page.waitForSelector("[data-loading]", { state: "detached" });
  await expect(page.getByRole("heading", { name: "Use your recovery words" })).toBeVisible();
  await expect(page.locator("textarea[aria-label='Recovery words']")).toHaveValue(PHRASE);
  await expect.poll(() => page.evaluate(() => location.hash)).toBe("");
  await expect(page.getByText("That link couldn't be opened")).toHaveCount(0);
  const carried = seen.join("\n");
  expect(carried).not.toContain(PHRASE);
  expect(carried).not.toContain(encoded);
  expect(carried).not.toContain(`recover=${encoded}`);
});

test("setup does not say Sync paused after a stored pause", async ({ page }) => {
  test.setTimeout(120000);
  const box = startBox();
  await box.ready;
  try {
    const minted = await fetch(BOX + "/v1/admin/codes", {
      method: "POST",
      headers: {
        authorization: "Bearer admin-test-key-value",
        "content-type": "application/json",
      },
      body: JSON.stringify({ uses: 2, label: "Harbor", expires: "2099-01-01" }),
    });
    const mintedText = await minted.text();
    if (!minted.ok) throw new Error(`access code ${minted.status} ${mintedText}`);
    const access = JSON.parse(mintedText).code;

    await page.addInitScript(() => {
      localStorage.setItem("omacus:sync-paused", "1");
    });
    await page.route("**/v1/**", handle);
    await page.goto("/app/", { waitUntil: "domcontentloaded" });
    await page.waitForSelector("[data-loading]", { state: "detached" });
    await page.waitForFunction(() => typeof globalThis.omacusTurnOnSync === "function");
    const chip = page.locator(".sync-chip");
    await expect(chip).toHaveText("Sync paused");

    await page.getByRole("button", { name: "Settings" }).click();
    await page.locator("[data-settings-tab='sync']").click();
    await expect(page.getByRole("heading", { name: "Turn on sync" })).toBeVisible();
    await expect(chip).toHaveText("Sync paused");

    await page.getByRole("button", { name: "This is my first device" }).click();
    await expect(page.getByRole("heading", { name: "Start fresh on this computer" })).toBeVisible();
    await expect(chip).toBeHidden();

    await page.locator("[data-access-code]").fill(access);
    await page.getByRole("button", { name: "Continue" }).click();
    await expect(page.locator(".sync-word").first()).toBeVisible();
    await expect(page.getByRole("heading", { name: "Save your recovery words" })).toBeVisible();
    await expect(chip).toBeHidden();
    await expect(chip).not.toHaveText("Sync paused");
  } finally {
    box.child.kill();
  }
});
