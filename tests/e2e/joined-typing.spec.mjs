// The desktop app and the web app on one set, both being typed in.
// Skipped unless OMACUS_JOINED=1, like the other joined proof: it needs the
// desktop binary (`cargo build`, or OMACUS_BIN).
//
// The web app starts with its own example pages and its default settings,
// the way a browser does the first time. Joining used to replace the set's
// theme with that default. A line from the desktop used to send the web
// cursor to the top of the page and drop the letters typed meanwhile.
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { randomBytes } from "node:crypto";
import { mkdirSync, writeFileSync, readFileSync, chmodSync, rmSync } from "node:fs";
import { expect, test } from "@playwright/test";

const BOX = "http://127.0.0.1:8798";
const BIN = process.env.OMACUS_BIN || "target/debug/omacus";
const ROOT = "/tmp/omacus-joined-typing";

test.skip(!process.env.OMACUS_JOINED, "joined proof needs the desktop binary");
test.skip(({ browserName }) => browserName !== "chromium", "joined proof runs in chromium");
// The page and the sync box are both on this machine. Chromium asks before a
// page opens a socket to a local address, and a test has nobody to say yes.
test.use({
  actionTimeout: 20000,
  launchOptions: { args: ["--disable-features=LocalNetworkAccessChecks,LocalNetworkAccessChecksWebSockets"] },
});

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

async function arm(context) {
  await context.route("**/app/**", async (route) => {
    if (route.request().resourceType() !== "document") return route.continue();
    const response = await route.fetch();
    const body = (await response.text()).replace("<html lang=\"en\">", "<html lang=\"en\" data-sync-test=\"1\">");
    await route.fulfill({ response, body });
  });
  await context.route("**/v1/**", handle);
}

function secretFile(path, text) {
  writeFileSync(path, text.endsWith("\n") ? text : `${text}\n`, { mode: 0o600 });
  chmodSync(path, 0o600);
}

async function mint() {
  const minted = await fetch(BOX + "/v1/admin/codes", {
    method: "POST",
    headers: { authorization: "Bearer admin-test-key-value", "content-type": "application/json" },
    body: JSON.stringify({ uses: 8, label: "Harbor", expires: "2099-01-01" }),
  });
  if (!minted.ok) throw new Error(`access code ${minted.status}`);
  return (await minted.json()).code;
}

function startDriver(dir, env) {
  const child = spawn(BIN, [], {
    env: {
      ...process.env,
      OMACUS_E2E_SYNC: "1",
      OMACUS_E2E_DIR: dir,
      OMACUS_DATA_DIR: dir,
      OMACUS_SYNC_URL: BOX,
      OMACUS_NO_UPDATE_CHECK: "1",
      RUST_LOG: "",
      ...env,
    },
    stdio: ["pipe", "pipe", "inherit"],
  });
  const pending = [];
  const waiters = [];
  const rl = createInterface({ input: child.stdout });
  rl.on("line", (line) => {
    if (!line.startsWith("{")) return;
    if (waiters.length) waiters.shift()(line);
    else pending.push(line);
  });
  return {
    child,
    send(cmd, ms = 60000) {
      child.stdin.write(`${cmd}\n`);
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error(`driver timed out on ${cmd.split(" ")[0]}`)), ms);
        const take = (line) => {
          clearTimeout(timer);
          const msg = JSON.parse(line);
          if (!msg.ok) reject(new Error(`${msg.cmd || cmd} ${msg.error || "sync failed"}`));
          else resolve(msg);
        };
        if (pending.length) take(pending.shift());
        else waiters.push(take);
      });
    },
  };
}

async function sideState(page) {
  return page.evaluate(() => ({
    rows: [...document.querySelectorAll("[data-sheets] .side-row .side-name span")].map((node) => node.textContent.trim()),
    ids: [...document.querySelectorAll("[data-sheets] .side-row")].map((node) => node.dataset.id),
    trash: [...document.querySelectorAll("[data-deleted] button")].map((node) => node.textContent.trim()),
    chip: document.querySelector(".sync-chip")?.innerText?.replace(/\s+/g, " ").trim() || "",
  }));
}

async function joinWith(page, code, name) {
  await page.getByRole("button", { name: "Turn on sync" }).first().click();
  await page.getByRole("button", { name: "Set up sync" }).click();
  await page.getByRole("button", { name: "My pages already sync on another device" }).click();
  await page.getByRole("textbox", { name: "Name this phone or computer" }).fill(name);
  await page.locator("[data-join-code]").fill(code);
  await page.getByRole("button", { name: "Join", exact: true }).click();
}

async function settings(page) {
  return page.evaluate(() => ({
    theme: document.querySelector("[data-setting=theme]")?.value || "",
    hide: !!document.querySelector("[data-setting=hide-answers]")?.checked,
    lock: document.querySelector("[data-setting=lock-minutes]")?.value || "",
  }));
}

function webText(page, id) {
  return page.evaluate((key) => globalThis.omacusPageText(key), id);
}

async function deskText(driver, id) {
  await driver.send("sync");
  return (await driver.send(`text ${id}`)).detail.text;
}

async function openPage(page, title) {
  await page.locator(".side-name", { hasText: title }).first().click();
  await expect(page.locator(".cm-content")).toContainText("bowl 2");
  await page.locator(".cm-content").click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("ArrowRight");
}

test("desktop and web keep each other's lines while both type", async ({ browser }) => {
  test.setTimeout(300000);
  rmSync(ROOT, { recursive: true, force: true });
  mkdirSync(ROOT, { recursive: true });
  const box = startBox();
  const children = [box.child];
  await box.ready;
  try {
    secretFile(`${ROOT}/pass`, randomBytes(18).toString("hex"));
    secretFile(`${ROOT}/access`, await mint());
    const driver = startDriver(`${ROOT}/desk`, {
      OMACUS_E2E_PASS_FILE: `${ROOT}/pass`,
      OMACUS_E2E_ACCESS_FILE: `${ROOT}/access`,
      OMACUS_E2E_INVITE_FILE: `${ROOT}/invite`,
    });
    children.push(driver.child);
    await driver.send("setup");
    await driver.send("invite");
    const code = readFileSync(`${ROOT}/invite`, "utf8").trim();

    const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    await arm(context);
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (error) => errors.push(String(error)));
    await page.goto("/app/", { waitUntil: "domcontentloaded" });
    await page.waitForSelector("[data-loading]", { state: "detached", timeout: 60000 });
    await page.waitForFunction(() => typeof globalThis.omacusTurnOnSync === "function");
    expect((await settings(page)).theme).toBe("auto");

    await joinWith(page, code, "Harbor web");
    await expect(page.getByRole("heading", { name: "Approve on your other device" })).toBeVisible();
    await driver.send("approve", 40000);
    await expect(page.getByRole("heading", { name: "Do these match?" })).toBeVisible({ timeout: 20000 });
    await page.getByRole("button", { name: "These match" }).click();
    await expect(page.locator(".sync-chip")).toHaveText("Live", { timeout: 30000 });
    await expect.poll(async () => (await sideState(page)).rows, { timeout: 20000 }).toContain("Pantry");

    // The set's settings and handle are the ones both devices end on.
    expect(await settings(page)).toEqual({ theme: "dusk", hide: true, lock: "15" });
    await expect
      .poll(async () => (await driver.send("sync")).detail.present.sidebar.some((row) => row.id === "ex-folder"), { timeout: 20000 })
      .toBe(true);
    const shared = (await driver.send("sync")).detail.present;
    expect(shared.prefs.theme).toBe("dusk");
    expect(shared.prefs.lockMinutes).toBe(15);
    expect(shared.handles.find((row) => row.id === "ferry")?.handle).toBe("pier");
    expect((await sideState(page)).trash).toEqual(["Kept"]);
    await page.evaluate(() => {
      const root = document.querySelector("[data-settings]");
      if (root && !root.hidden) root.querySelector(".settings-head [data-action='close-settings']")?.click();
    });

    // Typed on the web, read on the desktop.
    await openPage(page, "Oats");
    await page.keyboard.type("\nweb tin 3", { delay: 20 });
    await expect.poll(() => deskText(driver, "oats"), { timeout: 15000 }).toContain("web tin 3");

    // Typed on the desktop, shown in the page that is open on the web.
    writeFileSync(`${ROOT}/oats.txt`, "# Oats\nbowl 2\n\nweb tin 3\ndesk jar 4\n");
    await driver.send(`rename oats ${ROOT}/oats.txt`);
    await expect(page.locator(".cm-content")).toContainText("desk jar 4", { timeout: 15000 });

    // Both at once, page after page. Each side keeps the other's line, and
    // the web cursor stays on the line it was typing.
    for (const title of ["Bread", "Cake", "Ferry"]) {
      const id = title.toLowerCase();
      await openPage(page, title);
      writeFileSync(`${ROOT}/${id}.txt`, `# ${title}\nbowl 2\ndesk crust 1\n`);
      const typing = page.keyboard.type("\nweb crumb 2\nweb crumb 3\nweb crumb 4", { delay: 60 });
      await Promise.all([typing, driver.send(`rename ${id} ${ROOT}/${id}.txt`)]);
      await expect(page.locator(".cm-content")).toContainText("desk crust 1", { timeout: 20000 });
      await page.keyboard.type(" end");
      const want = "web crumb 2\nweb crumb 3\nweb crumb 4 end";
      await expect.poll(() => webText(page, id), { timeout: 15000 }).toContain(want);
      await expect.poll(() => deskText(driver, id), { timeout: 15000 }).toContain(want);
      const both = await deskText(driver, id);
      expect(both).toContain("desk crust 1");
      expect(both.startsWith(`# ${title}\nbowl 2\n`)).toBe(true);
      expect(await webText(page, id)).toBe(both);
    }

    // Deleted on the web, gone on the desktop.
    writeFileSync(`${ROOT}/kettle.txt`, "# Kettle\nsteam 5\n");
    await driver.send(`rename kettle ${ROOT}/kettle.txt`);
    await expect.poll(async () => (await sideState(page)).rows, { timeout: 15000 }).toContain("Kettle");
    await page.locator(".side-row", { hasText: "Kettle" }).first().click({ button: "right" });
    await page.getByText("Delete page", { exact: true }).click();
    await page.locator("[data-confirm-yes]").click();
    await expect
      .poll(async () => {
        await driver.send("sync");
        return (await driver.send("status")).detail.pages.some((row) => row.startsWith("kettle:"));
      }, { timeout: 15000 })
      .toBe(false);

    // A folder made on the web shows there at once, stays, and reaches the desktop.
    const deskFolder = async (name) => {
      const shown = (await driver.send("sync")).detail.present.sidebar;
      return shown.find((row) => row.type === "folder" && row.name === name) || null;
    };
    await page.locator(".side-head [data-action='new-folder']").click();
    await page.locator("[data-rename-input]").fill("Tins");
    await page.keyboard.press("Enter");
    await expect.poll(async () => (await sideState(page)).rows, { timeout: 5000 }).toContain("Tins");
    await expect.poll(async () => !!(await deskFolder("Tins")), { timeout: 15000 }).toBe(true);
    expect((await sideState(page)).rows).toContain("Tins");

    // A page moved into it on the web is inside it on the desktop.
    await page.locator(".side-row", { hasText: "Cake" }).first().click({ button: "right" });
    await page.getByText("Move to…", { exact: true }).locator("visible=true").first().click();
    await page.locator("[data-sync-dest-list] button", { hasText: "Tins" }).click();
    await expect
      .poll(async () => ((await deskFolder("Tins"))?.children || []).map((row) => row.id), { timeout: 15000 })
      .toEqual(["cake"]);

    // Renamed on the web, renamed on the desktop.
    await page.locator(".side-row", { hasText: "Tins" }).first().click({ button: "right" });
    await page.getByText("Rename", { exact: true }).locator("visible=true").first().click();
    await page.locator("[data-rename-input]").fill("Tin shelf");
    await page.keyboard.press("Enter");
    await expect.poll(async () => !!(await deskFolder("Tin shelf")), { timeout: 15000 }).toBe(true);
    expect(await deskFolder("Tins")).toBe(null);

    // A collapsed folder keeps its row, and opens again.
    const pantry = page.locator(".side-row", { hasText: "Pantry" }).first();
    await pantry.click();
    await expect.poll(async () => (await sideState(page)).rows, { timeout: 5000 }).not.toContain("Bread");
    expect((await sideState(page)).rows).toContain("Pantry");
    await pantry.click();
    await expect.poll(async () => (await sideState(page)).rows, { timeout: 5000 }).toContain("Bread");

    // An empty folder made on the desktop shows on the web.
    writeFileSync(
      `${ROOT}/jars.json`,
      JSON.stringify({
        structural: { sidebar: [{ id: "jars", kind: "folder", name: "Jars", parent: "", index: 9, collapsed: false }] },
      }),
    );
    await driver.send(`overlay ${ROOT}/jars.json`, 40000);
    await expect.poll(async () => (await sideState(page)).rows, { timeout: 15000 }).toContain("Jars");
    expect((await sideState(page)).rows).toContain("Tin shelf");

    // A deleted page restored on the web comes back on the desktop.
    await page.locator("[data-deleted] button", { hasText: "Kettle" }).click();
    await page.locator("[data-action='restore-trash']").click();
    await expect
      .poll(async () => {
        await driver.send("sync");
        return (await driver.send("status")).detail.pages.some((row) => row.startsWith("kettle:"));
      }, { timeout: 15000 })
      .toBe(true);

    // A setting changed on either side reaches the other.
    await page.locator("[data-setting=theme]").evaluate((node) => {
      node.value = "paper";
      node.dispatchEvent(new Event("input", { bubbles: true }));
      node.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await expect.poll(async () => (await driver.send("sync")).detail.present.prefs.theme, { timeout: 15000 }).toBe("paper");
    writeFileSync(`${ROOT}/prefs.json`, JSON.stringify({ structural: { prefs: { theme: "light", lockMinutes: 30 } } }));
    await driver.send(`overlay ${ROOT}/prefs.json`, 40000);
    await expect.poll(async () => (await settings(page)).theme, { timeout: 15000 }).toBe("light");
    expect((await settings(page)).lock).toBe("30");

    // After a reload the web still has the same sidebar and goes back to Live.
    const before = (await sideState(page)).rows;
    await page.reload({ waitUntil: "domcontentloaded" });
    await page.waitForSelector("[data-loading]", { state: "detached", timeout: 60000 });
    await expect(page.locator(".sync-chip")).toHaveText("Live", { timeout: 30000 });
    expect((await sideState(page)).rows).toEqual(before);
    // The theme control is filled in when Settings opens. The page itself wears the theme.
    expect(await page.evaluate(() => document.documentElement.dataset.appTheme)).toBe("light");
    expect((await driver.send("sync")).detail.present.prefs.theme).toBe("light");

    expect(errors).toEqual([]);
    await expect(page.locator(".sync-chip")).toHaveText("Live");
    await context.close();
  } finally {
    for (const child of children) child.kill();
  }
});
