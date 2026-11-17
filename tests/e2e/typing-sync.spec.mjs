// A line from the other phone lands in the page that is being typed in.
// The cursor used to jump to the top of the page, and the letters typed while
// the copy was being installed were written over. Both phones keep both.
import { spawn } from "node:child_process";
import { expect, test } from "@playwright/test";

const BOX = "http://127.0.0.1:8799";
const TYPED = "ferry pier seven lanterns and a rope";

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

async function openApp(page) {
  await page.goto("/app/", { waitUntil: "domcontentloaded" });
  await page.waitForSelector("[data-loading]", { state: "detached" });
  await page.waitForFunction(() => typeof globalThis.omacusTurnOnSync === "function");
}

async function pageText(page) {
  return page.evaluate(() => document.querySelector(".cm-content").cmView?.view?.state.doc.toString()
    ?? [...document.querySelectorAll(".cm-content .cm-line")].map((line) => line.textContent).join("\n"));
}

async function saved(page) {
  return page.waitForResponse((res) => res.request().method() === "PUT" && res.url().includes("/v1/mailboxes/") && res.status() === 200);
}

test("a line from the other phone lands while this one is typing", async ({ browser }) => {
  test.setTimeout(180000);
  const box = startBox();
  await box.ready;
  try {
    await fetch(BOX + "/reset", { method: "POST" });
    const first = await browser.newContext();
    const second = await browser.newContext();
    await first.route("**/v1/**", handle);
    await second.route("**/v1/**", handle);
    const phoneA = await first.newPage();
    const phoneB = await second.newPage();

    await openApp(phoneA);
    await phoneA.getByRole("button", { name: "Turn on sync" }).first().click();
    await phoneA.getByRole("button", { name: "Set up sync" }).click();
    await phoneA.getByRole("button", { name: "This is my first device" }).click();
    await phoneA.locator("[data-access-code]").fill("FERRY-40");
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
    await expect(phoneA.locator("[data-sync-linked]")).toBeVisible({ timeout: 45000 });
    await phoneA.locator("[data-action='close-settings']").last().click();
    await phoneA.getByRole("button", { name: "New page" }).click();
    const made = saved(phoneA);
    await phoneA.locator(".cm-content").click();
    await phoneA.keyboard.insertText("Harbor ledger\nbread 2\n");
    await made;

    await openApp(phoneB);
    await phoneB.getByRole("button", { name: "Settings" }).click();
    await phoneB.locator("[data-settings-tab='sync']").click();
    await phoneB.getByRole("button", { name: "My pages already sync on another device" }).click();
    await phoneB.getByRole("button", { name: "Use recovery words" }).click();
    await phoneB.locator("[data-recover-name]").fill("Harbor phone");
    await phoneB.locator(".sync-phrase").fill(words.join(" "));
    await phoneB.getByRole("button", { name: "Open my pages" }).click();
    await expect(phoneB.locator("[data-sync-linked]")).toBeVisible({ timeout: 20000 });
    await phoneB.locator("[data-action='close-settings']").last().click();
    await phoneB.locator(".side-name", { hasText: "Harbor ledger" }).click();
    await expect(phoneB.locator(".cm-content")).toContainText("bread 2", { timeout: 20000 });
    await expect(phoneA.locator(".sync-chip")).toHaveAttribute("data-state", /^(on|list)$/, { timeout: 20000 });
    await phoneA.locator(".side-name", { hasText: "Harbor ledger" }).click();

    // Phone B types one long line at the end. Phone A adds a line above it
    // while those keys are still going down.
    await phoneB.locator(".cm-content").click();
    await phoneB.keyboard.press("ControlOrMeta+a");
    await phoneB.keyboard.press("ArrowRight");
    const typing = phoneB.keyboard.type(TYPED, { delay: 150 });
    await phoneA.locator(".cm-content").click();
    await phoneA.keyboard.press("ControlOrMeta+a");
    await phoneA.keyboard.insertText("Harbor ledger\ncake 5\nbread 2\n");
    await expect(phoneB.locator(".cm-content")).toContainText("cake 5", { timeout: 20000 });
    await typing;
    // The cursor is still where the typing was.
    await phoneB.keyboard.type("!");

    await expect.poll(() => pageText(phoneB), { timeout: 20000 }).toContain(`${TYPED}!`);
    const here = await pageText(phoneB);
    expect(here).toContain("Harbor ledger\ncake 5\nbread 2");
    expect(here.startsWith("Harbor ledger")).toBe(true);
    await expect.poll(() => pageText(phoneA), { timeout: 30000 }).toContain(`${TYPED}!`);
    expect(await pageText(phoneA)).toContain("cake 5");
    await expect(phoneB.locator(".sync-chip")).not.toHaveAttribute("data-state", "error");

    await first.close();
    await second.close();
  } finally {
    box.child.kill();
  }
});
