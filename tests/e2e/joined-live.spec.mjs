// Joined desktop and web on one demo set. Skipped unless OMACUS_JOINED=1 so the
// ordinary Playwright run, which has no desktop binary and no display, stays green.
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { randomBytes } from "node:crypto";
import { mkdirSync, writeFileSync, readFileSync, existsSync, chmodSync } from "node:fs";
import { expect, test } from "@playwright/test";

const BOX = "http://127.0.0.1:8798";
const BIN = process.env.OMACUS_BIN || "/tmp/omacus-106/target/debug/omacus";
const ROOT = "/tmp/omacus-joined-proof";
const ART = "/opt/cursor/artifacts/screenshots";
const ROWS = ["Pantry", "Oats", "Ferry", "Bread", "Cake"];

test.skip(!process.env.OMACUS_JOINED, "joined proof needs the desktop binary and a display");
test.skip(({ browserName }) => browserName !== "chromium", "joined proof runs in chromium");
test.use({ actionTimeout: 20000 });

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
    const body = (await response.text()).replace("<html lang=\"en\">", "<html lang=\"en\" data-sync-test=\"1\" data-sync-blank=\"1\">");
    await route.fulfill({ response, body });
  });
  await context.route("**/v1/**", handle);
}

function secretFile(path, text) {
  writeFileSync(path, text.endsWith("\n") ? text : `${text}\n`, { mode: 0o600 });
  chmodSync(path, 0o600);
}

function readSecret(path) {
  return readFileSync(path, "utf8").trim();
}

async function mint() {
  const minted = await fetch(BOX + "/v1/admin/codes", {
    method: "POST",
    headers: {
      authorization: "Bearer admin-test-key-value",
      "content-type": "application/json",
    },
    body: JSON.stringify({ uses: 8, label: "Harbor", expires: "2099-01-01" }),
  });
  if (!minted.ok) throw new Error(`access code ${minted.status}`);
  const body = await minted.json();
  if (!body.code) throw new Error("access code missing");
  return body.code;
}

function startDriver(dir, passFile, accessFile, inviteFile) {
  const child = spawn(BIN, [], {
    env: {
      ...process.env,
      OMACUS_E2E_SYNC: "1",
      OMACUS_E2E_DIR: dir,
      OMACUS_DATA_DIR: dir,
      OMACUS_SYNC_URL: BOX,
      OMACUS_E2E_PASS_FILE: passFile,
      OMACUS_E2E_ACCESS_FILE: accessFile,
      OMACUS_E2E_INVITE_FILE: inviteFile,
      OMACUS_NO_UPDATE_CHECK: "1",
      RUST_LOG: "",
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

function startGui(env) {
  const child = spawn(BIN, [], {
    env: { ...process.env, ...env, RUST_LOG: "" },
    stdio: ["ignore", "ignore", "pipe"],
  });
  child.stderr.on("data", () => {});
  return child;
}

function approvalFinished(row) {
  const screen = row.screen === "Home" || row.screen === "Closed";
  const chip = row.chip === "Live" || row.chip === "List changed";
  return row.live === true && screen && chip;
}

function waitFor(file, pred, ms, label) {
  const start = Date.now();
  return new Promise((resolve, reject) => {
    const tick = () => {
      let value = null;
      if (existsSync(file)) {
        try {
          value = JSON.parse(readFileSync(file, "utf8"));
        } catch {
          value = null;
        }
      }
      if (value && pred(value)) {
        resolve(value);
        return;
      }
      if (Date.now() - start > ms) {
        reject(new Error(label));
        return;
      }
      setTimeout(tick, 100);
    };
    tick();
  });
}

function waitExists(file, ms, label) {
  const start = Date.now();
  return new Promise((resolve, reject) => {
    const tick = () => {
      if (existsSync(file) && readFileSync(file).length > 100) {
        resolve();
        return;
      }
      if (Date.now() - start > ms) {
        reject(new Error(label));
        return;
      }
      setTimeout(tick, 100);
    };
    tick();
  });
}

async function openApp(page) {
  await page.goto("/app/", { waitUntil: "domcontentloaded" });
  await page.waitForSelector("[data-loading]", { state: "detached", timeout: 60000 });
  await page.waitForFunction(() => typeof globalThis.omacusBlankLibrary === "function");
  await page.evaluate(() => globalThis.omacusBlankLibrary());
}

async function sideState(page) {
  return page.evaluate(() => ({
    rows: [...document.querySelectorAll("[data-sheets] .side-row .side-name span")].map((node) => node.textContent.trim()),
    ids: [...document.querySelectorAll("[data-sheets] .side-row")].map((node) => node.dataset.id),
    trash: [...document.querySelectorAll("[data-deleted] button")].map((node) => node.textContent.trim()),
    chip: document.querySelector(".sync-chip")?.innerText?.replace(/\s+/g, " ").trim() || "",
    theme: document.querySelector("[data-setting=theme]")?.value || "",
    hide: !!document.querySelector("[data-setting=hide-answers]")?.checked,
    lock: document.querySelector("[data-setting=lock-minutes]")?.value || "",
  }));
}

async function redact(page) {
  await page.evaluate(() => {
    for (const node of document.querySelectorAll(".sync-code, .sync-words, .sync-invite-code, .sync-word")) {
      node.textContent = "····";
    }
    for (const node of document.querySelectorAll("[data-join-code], [data-access-code]")) node.value = "";
  });
}

async function closeSettings(page) {
  // A pointer click lands on the card, not the close button: the card is
  // centered over the scrim. Ask the button itself.
  await page.evaluate(() => {
    const root = document.querySelector("[data-settings]");
    if (!root || root.hidden) return;
    root.querySelector(".settings-head [data-action='close-settings']")?.click();
  });
  await expect(page.locator("[data-settings]")).toBeHidden();
}

async function chooseTheme(page, value) {
  await page.locator("[data-setting=theme]").evaluate((node, next) => {
    node.value = next;
    node.dispatchEvent(new Event("input", { bubbles: true }));
    node.dispatchEvent(new Event("change", { bubbles: true }));
  }, value);
}

function shot(page, path) {
  return page.screenshot({ path, animations: "disabled" });
}

async function openSettings(page) {
  if ((await page.locator("[data-settings]").getAttribute("hidden")) !== null) {
    await page.getByRole("button", { name: "Settings" }).click();
  }
  await page.locator("[data-settings-tab='settings']").click();
}

async function joinWith(page, code, name) {
  await page.getByRole("button", { name: "Turn on sync" }).first().click();
  await page.getByRole("button", { name: "Set up sync" }).click();
  await page.getByRole("button", { name: "My pages already sync on another device" }).click();
  await page.getByRole("textbox", { name: "Name this phone or computer" }).fill(name);
  await page.locator("[data-join-code]").fill(code);
  await page.getByRole("button", { name: "Join", exact: true }).click();
}

async function firstDevice(page, access) {
  await page.getByRole("button", { name: "Turn on sync" }).first().click();
  await page.getByRole("button", { name: "Set up sync" }).click();
  await page.getByRole("button", { name: "This is my first device" }).click();
  await page.getByRole("textbox", { name: "Name this phone or computer" }).fill("Harbor web");
  await page.locator("[data-access-code]").fill(access);
  await page.getByRole("button", { name: "Continue" }).click();
  await page.waitForSelector(".sync-word");
  await page.evaluate(() => {
    const words = [...document.querySelectorAll(".sync-word")].map((node) => node.getAttribute("data-word"));
    document.querySelector("[data-wrote-words]")?.click();
    document.querySelector("[data-saved-words]")?.click();
    globalThis.__omacusWordCheck = words;
  });
  for (let step = 0; step < 3; step += 1) {
    await page.waitForSelector(".sync-ask");
    await page.evaluate((index) => {
      const ask = document.querySelectorAll(".sync-ask")[index];
      const n = Number(ask.querySelector("legend").innerText.replace(/\D/g, ""));
      const word = globalThis.__omacusWordCheck[n - 1];
      [...ask.querySelectorAll("button")].find((button) => button.getAttribute("data-word") === word)?.click();
    }, step);
  }
  await page.evaluate(() => {
    delete globalThis.__omacusWordCheck;
  });
  await page.getByRole("button", { name: "These match" }).click({ force: true });
  await expect(page.locator("[data-sync-linked]")).toBeVisible({ timeout: 30000 });
}

function settingsToml() {
  return [
    "check_for_updates = true",
    "theme = \"dusk\"",
    "hide_answers = true",
    "replace_exports = false",
    "lock_minutes = 15",
    "",
  ].join("\n");
}

function emptyLibrary(dir) {
  mkdirSync(`${dir}/pages`, { recursive: true });
  writeFileSync(`${dir}/pages/harbor.txt`, "# Harbor\n");
  writeFileSync(
    `${dir}/state.json`,
    JSON.stringify({
      order: ["harbor"],
      active: "harbor",
      cursors: {},
      sidebar: true,
      sidebar_width: 240,
      tree: [{ type: "page", id: "harbor" }],
      handles: { harbor: "harbor" },
      seeded: true,
    }),
  );
}

function guiEnv(dir, config, extra) {
  return {
    OMACUS_DATA_DIR: dir,
    XDG_CONFIG_HOME: config,
    OMACUS_SYNC_URL: BOX,
    OMACUS_E2E_UNLOCK: "1",
    OMACUS_NO_UPDATE_CHECK: "1",
    DISPLAY: process.env.DISPLAY || ":1",
    ...extra,
  };
}

test("joined devices share one library and finish approval", async ({ browser }) => {
  test.setTimeout(480000);
  writeFileSync("/tmp/omacus-e2e-sw-gen", String(Date.now()));
  mkdirSync(ROOT, { recursive: true });
  mkdirSync(ART, { recursive: true });
  const box = startBox();
  const children = [box.child];
  await box.ready;
  try {
    const pass = randomBytes(18).toString("hex");
    const access = await mint();
    const desk = `${ROOT}/parity`;
    const config = `${ROOT}/config`;
    mkdirSync(`${config}/omacus`, { recursive: true });
    writeFileSync(`${config}/omacus/settings.toml`, settingsToml());
    secretFile(`${ROOT}/pass`, pass);
    secretFile(`${ROOT}/access`, access);
    const inviteFile = `${ROOT}/invite`;
    const driver = startDriver(desk, `${ROOT}/pass`, `${ROOT}/access`, inviteFile);
    children.push(driver.child);
    await driver.send("setup");
    await driver.send("invite");
    const code = readSecret(inviteFile);

    const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    await arm(context);
    const page = await context.newPage();
    await openApp(page);
    await joinWith(page, code, "Harbor web");
    await expect(page.getByRole("heading", { name: "Approve on your other device" })).toBeVisible();
    await driver.send("approve", 40000);
    await expect(page.getByRole("heading", { name: "Do these match?" })).toBeVisible({ timeout: 20000 });
    await page.getByRole("button", { name: "These match" }).click();
    await expect(page.locator(".sync-chip")).toHaveText("Live", { timeout: 20000 });
    await expect.poll(async () => (await sideState(page)).rows, { timeout: 20000 }).toEqual(ROWS);
    const web = await sideState(page);
    expect(web.trash).toEqual(["Kept"]);
    expect(web.theme).toBe("dusk");
    expect(web.hide).toBe(true);
    expect(web.lock).toBe("15");
    driver.child.stdin.write("quit\n");
    await new Promise((resolve) => driver.child.once("exit", resolve));

    const shots = `${ROOT}/parity-shots`;
    const statusFile = `${ROOT}/status.json`;
    mkdirSync(shots, { recursive: true });
    const gui = startGui(guiEnv(desk, config, {
      OMACUS_E2E_PASS_FILE: `${ROOT}/pass`,
      OMACUS_E2E_SHOT_DIR: shots,
      OMACUS_E2E_SETTINGS: "1",
      OMACUS_E2E_STATUS: statusFile,
      OMACUS_E2E_EDIT: `${ROOT}/edit.txt`,
      OMACUS_E2E_EDIT_AT: `${ROOT}/edit-at`,
      OMACUS_E2E_WAIT: `${ROOT}/wait.txt`,
      OMACUS_E2E_SEEN_AT: `${ROOT}/seen-at`,
      OMACUS_E2E_MAKE_INVITE: `${ROOT}/make-invite`,
      OMACUS_E2E_INVITE_FILE: `${ROOT}/invite-2`,
      OMACUS_E2E_AUTO: "approve",
    }));
    children.push(gui);
    const deskStatus = await waitFor(
      statusFile,
      (row) => row.chip === "Live" && row.theme === "dusk" && row.lines?.join(",") === "folder:Pantry,page:Oats,page:Ferry,page:Bread,page:Cake,trash:Kept",
      45000,
      "desktop did not reach the shared library",
    );
    expect(deskStatus.hideAnswers).toBe(true);
    expect(deskStatus.lockMinutes).toBe(15);
    expect(deskStatus.live).toBe(true);
    await waitExists(`${shots}/live-dusk.png`, 20000, "missing desktop sidebar");
    await waitExists(`${shots}/settings-dusk.png`, 20000, "missing desktop settings");
    await closeSettings(page);
    await expect(page.getByText("Pantry", { exact: true }).first()).toBeVisible();
    await redact(page);
    await shot(page, `${ROOT}/web-dusk-sidebar.png`);
    await openSettings(page);
    await redact(page);
    await shot(page, `${ROOT}/web-dusk-settings.png`);

    const themeChange = Date.now();
    await chooseTheme(page, "light");
    await waitFor(statusFile, (row) => row.theme === "light" && row.chip === "Live", 15000, "desktop did not take the light theme");
    const themeMs = Date.now() - themeChange;
    await waitExists(`${shots}/live-light.png`, 20000, "missing light sidebar");
    await waitExists(`${shots}/settings-light.png`, 20000, "missing light settings");
    await closeSettings(page);
    await expect(page.getByText("Pantry", { exact: true }).first()).toBeVisible();
    await redact(page);
    await shot(page, `${ROOT}/web-light-sidebar.png`);
    await openSettings(page);
    await redact(page);
    await shot(page, `${ROOT}/web-light-settings.png`);
    const light = await sideState(page);
    expect(light.rows).toEqual(ROWS);
    expect(light.theme).toBe("light");

    await closeSettings(page);
    const deskToWebAt = Date.now();
    secretFile(`${ROOT}/edit.txt`, "oats\n# Oats tin\nbowl 2\nharbor tin\n");
    await expect.poll(async () => (await sideState(page)).rows[1], { timeout: 15000 }).toBe("Oats tin");
    const desktopToWebMs = Date.now() - deskToWebAt;

    secretFile(`${ROOT}/wait.txt`, "Ferry lane");
    const webToDeskAt = Date.now();
    const replaced = await page.evaluate(() => globalThis.omacusReplacePage("ferry", "# Ferry lane\nbowl 2\n"));
    expect(replaced).toBe(true);
    await waitForFileText(`${ROOT}/seen-at`, 15000, "desktop did not show the web edit");
    const webToDesktopMs = Date.now() - webToDeskAt;

    writeFileSync(`${ROOT}/make-invite`, "1");
    await waitForFileText(`${ROOT}/invite-2`, 20000);
    const second = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    await arm(second);
    const phone = await second.newPage();
    await openApp(phone);
    await joinWith(phone, readSecret(`${ROOT}/invite-2`), "Harbor phone");
    await expect(phone.getByRole("heading", { name: "Approve on your other device" })).toBeVisible();
    await redact(phone);
    await shot(phone, `${ROOT}/web-waiting.png`);
    await waitExists(`${shots}/approve.png`, 30000, "desktop did not show approve");
    await expect(phone.getByRole("heading", { name: "Do these match?" })).toBeVisible({ timeout: 30000 });
    await redact(phone);
    await shot(phone, `${ROOT}/web-match.png`);
    await phone.getByRole("button", { name: "These match" }).click({ force: true });
    await expect(phone.locator(".sync-chip")).toHaveText("Live", { timeout: 20000 });
    await waitFor(statusFile, approvalFinished, 20000, "desktop did not finish approving");
    await second.close();

    gui.kill("SIGTERM");
    await new Promise((resolve) => gui.once("exit", resolve));

    const access2 = await mint();
    // A new page in the pantry context shares its saved library, so this
    // approval starts from an empty browser instead.
    const approveContext = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    await arm(approveContext);
    const approvePage = await approveContext.newPage();
    await openApp(approvePage);
    await firstDevice(approvePage, access2);
    await approvePage.getByRole("button", { name: "Add a phone or computer" }).click();
    await expect(approvePage.locator(".sync-invite-code")).toBeVisible();
    const deskCode = (await approvePage.locator(".sync-invite-code").innerText()).trim();
    secretFile(`${ROOT}/join-desk`, deskCode);
    await approvePage.locator("[data-action='close-settings']").last().click();

    const joinDir = `${ROOT}/joiner`;
    const joinConfig = `${ROOT}/joiner-config`;
    emptyLibrary(joinDir);
    mkdirSync(`${joinConfig}/omacus`, { recursive: true });
    writeFileSync(`${joinConfig}/omacus/settings.toml`, settingsToml());
    const joinShots = `${ROOT}/join-shots`;
    const joinStatus = `${ROOT}/join-status.json`;
    secretFile(`${ROOT}/join-pass`, randomBytes(18).toString("hex"));
    const joiner = startGui(guiEnv(joinDir, joinConfig, {
      OMACUS_E2E_PASS_FILE: `${ROOT}/join-pass`,
      OMACUS_E2E_JOIN_FILE: `${ROOT}/join-desk`,
      OMACUS_E2E_SHOT_DIR: joinShots,
      OMACUS_E2E_STATUS: joinStatus,
      OMACUS_E2E_AUTO: "accept",
    }));
    children.push(joiner);
    await expect(approvePage.locator(".sync-chip")).toHaveText("Device waiting", { timeout: 30000 });
    await approvePage.locator(".sync-chip").click();
    await expect(approvePage.getByRole("heading", { name: "Is this Harbor desk?" })).toBeVisible();
    await waitExists(`${joinShots}/waiting.png`, 20000, "desktop did not show waiting");
    await redact(approvePage);
    await shot(approvePage, `${ROOT}/web-approve.png`);
    await approvePage.getByRole("button", { name: "Yes, approve it" }).click({ force: true });
    await waitExists(`${joinShots}/match.png`, 30000, "desktop did not show the match");
    await waitFor(joinStatus, approvalFinished, 30000, "desktop join did not finish");
    await expect(approvePage.locator(".sync-chip")).toHaveText(/^(Live|List changed)$/, { timeout: 45000 });
    joiner.kill("SIGTERM");
    await new Promise((resolve) => joiner.once("exit", resolve));

    if ((await approvePage.locator("[data-settings]").getAttribute("hidden")) !== null) {
      await approvePage.getByRole("button", { name: "Settings" }).click();
    }
    await approvePage.locator("[data-settings-tab='sync']").click();
    await approvePage.getByRole("button", { name: "Add a phone or computer" }).click();
    await expect(approvePage.locator(".sync-invite-code")).toBeVisible();
    const hungCode = (await approvePage.locator(".sync-invite-code").innerText()).trim();
    await approvePage.route("**/v1/mailboxes/**/requests/**", async (route) => {
      const request = route.request();
      if (request.method() === "POST" && (request.postData() || "").includes("chain")) {
        await new Promise((resolve) => setTimeout(resolve, 15000));
        await route.abort();
        return;
      }
      await handle(route);
    });
    const late = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    await arm(late);
    const latePage = await late.newPage();
    await openApp(latePage);
    await joinWith(latePage, hungCode, "Harbor phone");
    await expect(approvePage.getByRole("heading", { name: "Is this Harbor phone?" })).toBeVisible({ timeout: 30000 });
    await approvePage.getByRole("button", { name: "Yes, approve it" }).click({ force: true });
    await expect(approvePage.getByText("That took too long. Your pages are still here.")).toBeVisible({ timeout: 20000 });
    await expect(approvePage.getByRole("button", { name: "Approving…" })).toHaveCount(0);
    await expect(approvePage.getByRole("button", { name: "Yes, approve it" })).toBeVisible();
    await redact(approvePage);
    await shot(approvePage, `${ROOT}/web-timeout.png`);
    await late.close();

    writeFileSync(
      "/opt/cursor/artifacts/joined-live-latency.json",
      JSON.stringify({ desktopToWebMs, webToDesktopMs, themeMs }, null, 2),
    );
    await compose(`${shots}/live-dusk.png`, `${ROOT}/web-dusk-sidebar.png`, `${ART}/joined-dusk-sidebar.png`);
    await compose(`${shots}/settings-dusk.png`, `${ROOT}/web-dusk-settings.png`, `${ART}/joined-dusk-settings.png`);
    await compose(`${shots}/live-light.png`, `${ROOT}/web-light-sidebar.png`, `${ART}/joined-light-sidebar.png`);
    await compose(`${shots}/settings-light.png`, `${ROOT}/web-light-settings.png`, `${ART}/joined-light-settings.png`);
    await compose(`${joinShots}/waiting.png`, `${ROOT}/web-approve.png`, `${ART}/joined-approve-web.png`);
    await compose(`${shots}/approve.png`, `${ROOT}/web-waiting.png`, `${ART}/joined-approve-desktop.png`);
    await compose(`${joinShots}/match.png`, `${ROOT}/web-match.png`, `${ART}/joined-approve-match.png`);
    await new Promise((resolve, reject) => {
      const child = spawn("python3", ["-c", copyOne(`${ROOT}/web-timeout.png`, `${ART}/joined-approve-timeout.png`)], { stdio: "inherit" });
      child.on("exit", (code) => (code === 0 ? resolve() : reject(new Error("timeout shot"))));
    });
    await approveContext.close();
    await context.close();
  } finally {
    for (const child of children) {
      if (child.exitCode == null && !child.killed) child.kill("SIGKILL");
    }
  }
});

function waitForFileText(file, ms, label = "timed out waiting for a file") {
  const start = Date.now();
  return new Promise((resolve, reject) => {
    const tick = () => {
      if (existsSync(file) && readFileSync(file, "utf8").trim()) {
        resolve();
        return;
      }
      if (Date.now() - start > ms) {
        reject(new Error(label));
        return;
      }
      setTimeout(tick, 100);
    };
    tick();
  });
}

function compose(left, right, out) {
  const child = spawn("python3", ["-c", composePair(left, right, out)], { stdio: "inherit" });
  return new Promise((resolve, reject) => {
    child.on("exit", (code) => (code === 0 ? resolve() : reject(new Error(`compose ${out}`))));
  });
}

function composePair(left, right, out) {
  return `
from PIL import Image, ImageDraw
left = Image.open(${JSON.stringify(left)}).convert("RGB")
right = Image.open(${JSON.stringify(right)}).convert("RGB")
h = 800
def fit(im):
    return im.resize((max(1, int(im.width * h / im.height)), h), Image.Resampling.LANCZOS)
left, right = fit(left), fit(right)
cap = 28
canvas = Image.new("RGB", (left.width + right.width + 8, h + cap), (246, 244, 239))
draw = ImageDraw.Draw(canvas)
draw.text((12, 6), "Desktop", fill=(40, 40, 40))
draw.text((left.width + 16, 6), "Web", fill=(40, 40, 40))
canvas.paste(left, (0, cap))
canvas.paste(right, (left.width + 8, cap))
canvas.save(${JSON.stringify(out)})
`;
}

function copyOne(src, out) {
  return `
from PIL import Image
Image.open(${JSON.stringify(src)}).convert("RGB").save(${JSON.stringify(out)})
`;
}
