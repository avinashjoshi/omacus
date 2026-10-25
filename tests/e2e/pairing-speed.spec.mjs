// The phone that shows an invite code notices a join within a few seconds.
// Screenshots cover the setup screens in light and dark, on a desk and a phone.
// Recovery words and their picture are covered before a shot.
import { mkdirSync } from "node:fs";
import { spawn } from "node:child_process";
import { expect, test } from "@playwright/test";

const BOX = "http://127.0.0.1:8798";
const DESKTOP_APP = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) omacus-desktop/1.0";
const SHOTS = "/opt/cursor/artifacts/screenshots";

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

async function theme(page, name) {
  await page.evaluate((value) => {
    document.documentElement.dataset.appTheme = value;
  }, name);
}

async function maskSecrets(page) {
  await page.locator(".sync-word, .sync-key svg").evaluateAll((nodes) => {
    for (const node of nodes) {
      if (node.tagName.toLowerCase() === "svg") {
        node.style.visibility = "hidden";
        continue;
      }
      for (const kid of [...node.childNodes]) {
        if (kid.nodeType === Node.TEXT_NODE && kid.textContent.trim()) kid.textContent = "••••";
      }
    }
  });
}

async function shot(page, name) {
  if (test.info().project.name !== "chromium") return;
  mkdirSync(SHOTS, { recursive: true });
  await maskSecrets(page);
  await page.screenshot({ path: `${SHOTS}/${name}.png` });
}

test("an invite notices a join within 5 seconds", async ({ browser }) => {
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
    await expect(phoneA.getByRole("heading", { name: "Turn on sync" })).toBeVisible();
    await expect(phoneA.getByText("Start fresh and keep these pages in sync from here.")).toBeVisible();
    await expect(phoneA.getByRole("textbox", { name: "Name this phone or computer" })).toHaveCount(0);
    await shot(phoneA, "sync-choice-desktop-light");
    await theme(phoneA, "dark");
    await shot(phoneA, "sync-choice-desktop-dark");
    await theme(phoneA, "light");
    await phoneA.setViewportSize({ width: 390, height: 844 });
    await expect(phoneA.getByRole("heading", { name: "Turn on sync" })).toBeVisible();
    await expect(phoneA.getByText("Join with a code from that phone or computer.")).toBeVisible();
    await shot(phoneA, "sync-choice-phone-light");
    await theme(phoneA, "dark");
    await shot(phoneA, "sync-choice-phone-dark");
    await theme(phoneA, "light");
    await phoneA.setViewportSize({ width: 1280, height: 800 });

    await phoneA.getByRole("button", { name: "This is my first device" }).click();
    await expect(phoneA.getByRole("heading", { name: "Start fresh on this computer" })).toBeVisible();
    await expect(phoneA.getByPlaceholder("Code you were given")).toBeVisible();
    await shot(phoneA, "sync-access-desktop-light");
    await theme(phoneA, "dark");
    await shot(phoneA, "sync-access-desktop-dark");
    await theme(phoneA, "light");
    await phoneA.setViewportSize({ width: 390, height: 844 });
    await expect(phoneA.getByRole("heading", { name: "Start fresh on this phone" })).toBeVisible();
    await shot(phoneA, "sync-access-phone-light");
    await theme(phoneA, "dark");
    await shot(phoneA, "sync-access-phone-dark");
    await theme(phoneA, "light");
    await phoneA.setViewportSize({ width: 1280, height: 800 });

    await phoneA.locator("[data-access-code]").fill(access);
    await phoneA.getByRole("button", { name: "Continue" }).click();
    await expect(phoneA.locator(".sync-word").first()).toBeVisible();
    const words = await phoneA.locator(".sync-word").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-word")));
    await shot(phoneA, "sync-words-desktop-light");
    await theme(phoneA, "dark");
    await shot(phoneA, "sync-words-desktop-dark");
    await theme(phoneA, "light");

    await phoneA.setViewportSize({ width: 390, height: 844 });
    await expect(phoneA.getByRole("heading", { name: "Save your recovery words" })).toBeVisible();
    await shot(phoneA, "sync-words-phone-light");
    await theme(phoneA, "dark");
    await shot(phoneA, "sync-words-phone-dark");
    await theme(phoneA, "light");
    await phoneA.setViewportSize({ width: 1280, height: 800 });
    await expect(phoneA.getByRole("heading", { name: "Save your recovery words" })).toBeVisible();

    await phoneA.getByRole("checkbox", { name: "I wrote these down" }).check();
    await phoneA.getByRole("button", { name: "I saved them" }).click();
    for (let step = 0; step < 3; step += 1) {
      const legend = phoneA.locator(".sync-ask").nth(step).locator("legend");
      const n = Number((await legend.innerText()).replace(/\D/g, ""));
      await phoneA.locator(".sync-ask").nth(step).getByRole("button", { name: words[n - 1], exact: true }).click();
    }
    await phoneA.getByRole("button", { name: "These match" }).click();
    await expect(phoneA.getByRole("heading", { name: "✓ Sync is on" })).toBeVisible();
    await expect(phoneA.getByText("Synced just now")).toBeVisible();
    await shot(phoneA, "sync-ready-desktop-light");
    await theme(phoneA, "dark");
    await shot(phoneA, "sync-ready-desktop-dark");
    await theme(phoneA, "light");

    await phoneA.setViewportSize({ width: 390, height: 844 });
    await expect(phoneA.getByRole("heading", { name: "✓ Sync is on" })).toBeVisible();
    await shot(phoneA, "sync-ready-phone-light");
    await theme(phoneA, "dark");
    await shot(phoneA, "sync-ready-phone-dark");
    await theme(phoneA, "light");
    await phoneA.setViewportSize({ width: 1280, height: 800 });

    await phoneA.getByRole("button", { name: "Add a phone or computer" }).click();
    const code = (await phoneA.locator(".sync-invite-code").innerText()).trim();
    await shot(phoneA, "sync-invite-desktop-light");
    await theme(phoneA, "dark");
    await shot(phoneA, "sync-invite-desktop-dark");
    await theme(phoneA, "light");
    await phoneA.setViewportSize({ width: 390, height: 844 });
    await expect(phoneA.getByRole("heading", { name: "Add a phone or computer" })).toBeVisible();
    await shot(phoneA, "sync-invite-phone-light");
    await theme(phoneA, "dark");
    await shot(phoneA, "sync-invite-phone-dark");
    await theme(phoneA, "light");
    await phoneA.setViewportSize({ width: 1280, height: 800 });

    const harbor = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    contexts.push(harbor);
    await harbor.route("**/v1/**", handle);
    const phoneB = await harbor.newPage();
    await phoneB.goto("/app/", { waitUntil: "domcontentloaded" });
    await phoneB.waitForSelector("[data-loading]", { state: "detached" });
    await syncReady(phoneB);
    await phoneB.getByRole("button", { name: "Settings" }).click();
    await phoneB.locator("[data-settings-tab='sync']").click();
    await phoneB.getByRole("button", { name: "My pages already sync on another device" }).click();
    await expect(phoneB.getByRole("heading", { name: "Join your synced pages" })).toBeVisible();
    await expect(phoneB.getByPlaceholder("K7QM-4R2P")).toBeVisible();
    await shot(phoneB, "sync-join-desktop-light");
    await theme(phoneB, "dark");
    await shot(phoneB, "sync-join-desktop-dark");
    await theme(phoneB, "light");
    await phoneB.setViewportSize({ width: 390, height: 844 });
    await expect(phoneB.getByRole("heading", { name: "Join your synced pages" })).toBeVisible();
    await shot(phoneB, "sync-join-phone-light");
    await theme(phoneB, "dark");
    await shot(phoneB, "sync-join-phone-dark");
    await theme(phoneB, "light");
    await phoneB.setViewportSize({ width: 1280, height: 800 });

    await phoneB.getByRole("textbox", { name: "Name this phone or computer" }).fill("Harbor phone");
    await phoneB.locator("[data-join-code]").fill(code);
    await phoneB.getByRole("button", { name: "Join", exact: true }).click();
    await expect(phoneA.getByRole("heading", { name: "Is this Harbor phone?" })).toBeVisible({ timeout: 5000 });
  } finally {
    for (const context of contexts) await context.close().catch(() => {});
    box.child.kill();
  }
});
