// A phone stops waiting. The same code asks about the next phone, and a save
// on either phone opens on the other.
import { spawn } from "node:child_process";
import { expect, test } from "@playwright/test";

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
  // WebKit refuses a fulfilled 304, so the mailbox is always read in full.
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
  // The loading screen leaves before Turn on sync can open the explanation.
  await page.waitForFunction(() => typeof globalThis.omacusTurnOnSync === "function");
}

async function openSync(page) {
  await page.goto("/app/", { waitUntil: "domcontentloaded" });
  await page.waitForSelector("[data-loading]", { state: "detached" });
  await syncReady(page);
  await page.getByRole("button", { name: "Settings" }).click();
  await page.locator("[data-settings-tab='sync']").click();
}

async function syncFailure(page) {
  const chip = page.locator(".sync-chip");
  if ((await chip.getAttribute("data-state")) !== "error") return "";
  await chip.click();
  const text = await page.locator("[data-sync-error]").innerText().catch(() => "");
  await page.locator("[data-action='close-settings']").last().click();
  return text;
}

async function replacePage(page, text) {
  const editor = page.locator(".cm-content");
  await editor.click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.insertText(text);
}

async function putChange(page, text) {
  const saved = page.waitForResponse(
    (res) => res.request().method() === "PUT" && res.url().includes("/v1/mailboxes/") && res.status() === 200,
    { timeout: 40000 },
  );
  await replacePage(page, text);
  try {
    await saved;
  } catch (error) {
    const why = await syncFailure(page);
    throw new Error(why || error.message);
  }
}

async function joinWith(page, code, name) {
  await openSync(page);
  await page.getByRole("button", { name: "My pages already sync on another device" }).click();
  await page.getByRole("textbox", { name: "Name this phone or computer" }).fill(name);
  await page.locator("[data-join-code]").fill(code);
  await page.getByRole("button", { name: "Join", exact: true }).click();
}

test("the same code asks again after a phone stops waiting", async ({ browser }) => {
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

    const first = await browser.newContext();
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
    await phoneA.locator("[data-action='close-settings']").last().click();
    await phoneA.getByRole("button", { name: "New page" }).click();
    const saved = phoneA.waitForResponse((res) => res.request().method() === "PUT" && res.url().includes("/v1/mailboxes/") && res.status() === 200);
    await replacePage(phoneA, "Harbor ledger\nferry = 40\n");
    await saved;
    await expect(phoneA.locator(".sync-chip")).toHaveAttribute("data-state", "on", { timeout: 20000 });

    await openSync(phoneA);
    await phoneA.getByRole("button", { name: "Add a phone or computer" }).click();
    const code = (await phoneA.locator(".sync-invite-code").innerText()).trim();

    const harbor = await browser.newContext();
    contexts.push(harbor);
    await harbor.route("**/v1/**", handle);
    const phoneB = await harbor.newPage();
    await joinWith(phoneB, code, "Harbor phone");
    await expect(phoneA.getByRole("heading", { name: "Is this Harbor phone?" })).toBeVisible({ timeout: 45000 });
    await expect(phoneA.getByRole("button", { name: "Yes, approve it" })).toBeVisible();

    await phoneB.close();
    await harbor.close();
    contexts.pop();
    await new Promise((resolve) => setTimeout(resolve, 1500));
    const clocked = await fetch(BOX + "/__clock", { method: "POST" });
    if (!clocked.ok) throw new Error(await clocked.text());

    await expect(phoneA.getByRole("button", { name: "Yes, approve it" })).toBeHidden({ timeout: 20000 });
    await expect(phoneA.getByRole("heading", { name: "Sync is on" })).toBeVisible();

    const ferry = await browser.newContext();
    contexts.push(ferry);
    await ferry.route("**/v1/**", handle);
    const phoneC = await ferry.newPage();
    await joinWith(phoneC, code, "Ferry phone");
    await expect(phoneA.getByRole("heading", { name: "Is this Ferry phone?" })).toBeVisible({ timeout: 50000 });
    await phoneA.getByRole("button", { name: "Yes, approve it" }).click();
    await expect(phoneC.getByRole("heading", { name: "Do these match?" })).toBeVisible({ timeout: 30000 });
    await phoneC.getByRole("button", { name: "These match" }).click();
    await expect(phoneC.locator("[data-sync-linked]")).toBeVisible({ timeout: 20000 });
    await expect(phoneA.getByRole("heading", { name: "Sync is on" })).toBeVisible({ timeout: 30000 });
    await phoneA.locator("[data-action='close-settings']").last().click();
    await phoneC.locator("[data-action='close-settings']").last().click();

    await phoneC.locator(".side-name", { hasText: "Harbor ledger" }).click();
    await expect(phoneC.locator(".cm-content")).toContainText("ferry = 40", { timeout: 20000 });

    await putChange(phoneA, "Harbor ledger\nferry = 40\nbread $2\n");
    await expect(phoneC.locator(".cm-content")).toContainText("bread $2", { timeout: 30000 });

    await putChange(phoneC, "Harbor ledger\nferry = 40\nbread $2\ncake $5\n");
    await expect(phoneA.locator(".cm-content")).toContainText("cake $5", { timeout: 30000 });
    await expect(phoneA.locator(".cm-content")).toContainText("bread $2");
    await expect(phoneC.locator(".cm-content")).toContainText("ferry = 40");
    await expect(phoneA.getByText("That code didn't work")).toHaveCount(0);
    await expect(phoneC.getByText("That code didn't work")).toHaveCount(0);
    await expect(phoneA.locator(".sync-chip")).not.toHaveAttribute("data-state", "error");
    await expect(phoneC.locator(".sync-chip")).not.toHaveAttribute("data-state", "error");
  } finally {
    for (const context of contexts) await context.close().catch(() => {});
    box.child.kill();
  }
});
