import { test, expect } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";

async function openApp(page) {
  await page.route("**/app/**", async (route) => {
    const name = new URL(route.request().url()).pathname.split("/").pop() || "index.html";
    if (process.env.OMACUS_NAV_BEFORE && ["index.html", "app.js", "app.css"].includes(name)) {
      let body = execFileSync("git", ["show", `HEAD:site/app/${name}`], { encoding: "utf8" });
      body = body.replace('<html lang="en">', '<html lang="en" data-sync-test="1">');
      return route.fulfill({ body, contentType: name.endsWith("css") ? "text/css" : name.endsWith("js") ? "text/javascript" : "text/html" });
    }
    if (route.request().resourceType() !== "document") return route.continue();
    const response = await route.fetch();
    await route.fulfill({ response, body: (await response.text()).replace('<html lang="en">', '<html lang="en" data-sync-test="1">') });
  });
  await page.goto("/app/");
  await page.waitForSelector(".cm-content");
  await page.waitForFunction(() => document.querySelector(".sync-chip")?.dataset.ready === "1");
  await page.evaluate(() => globalThis.omacusShowPlaces({
    vaultName: "Personal", localIds: ["ex5"], here: "On this computer", move() {},
  }));
}

test("vault navigation screenshots", async ({ page, browser }, info) => {
  test.skip(!process.env.OMACUS_NAV_SHOTS);
  const dir = process.env.OMACUS_NAV_SHOTS;
  mkdirSync(dir, { recursive: true });
  await page.setViewportSize({ width: 1120, height: 760 });
  await openApp(page);
  await page.locator('[data-id="ex1"] .side-name').click();
  const phoneContext = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
  const phone = await phoneContext.newPage();
  await openApp(phone);
  await phone.locator(".sheets-toggle").click();
  await phone.locator('[data-id="ex1"] .side-name').click();
  await phone.locator(".sheets-toggle").click();
  for (const theme of ["dark", "light"]) {
    for (const screen of [page, phone]) await screen.evaluate(t => {
      document.documentElement.dataset.appTheme = t;
      globalThis.omacusShowPlaces({ vaultName: "Personal", localIds: ["ex5"], here: "On this device", move() {} });
    }, theme);
    await page.screenshot({ animations: "disabled", path: `${dir}/${info.project.name}-desktop-${theme}.png` });
    await phone.screenshot({ animations: "disabled", path: `${dir}/${info.project.name}-phone-${theme}.png` });
  }
  if (!process.env.OMACUS_NAV_BEFORE) {
    await page.evaluate(() => document.documentElement.dataset.appTheme = "dark");
    await page.locator('[data-vault-switch]').click();
    await page.screenshot({ animations: "disabled", path: `${dir}/${info.project.name}-vault-menu.png` });
  }
  await phoneContext.close();
});

test("switching vaults filters the page list without changing the open page", async ({ page }) => {
  await openApp(page);
  const title = await page.locator("[data-title]").textContent();
  await page.locator("[data-vault-switch]").click();
  await page.locator('[data-vault-scope="local"]').click();
  await expect(page.locator('.sheet-list .side-row[data-kind="page"]')).toHaveCount(1);
  await expect(page.locator('[data-title]')).toHaveText(title);
  await page.locator("[data-vault-switch]").click();
  await page.locator('[data-vault-scope="vault"]').click();
  await expect(page.locator('.sheet-list [data-id="ex5"]')).toHaveCount(0);
  await expect(page.locator('.sheet-list .side-section')).toHaveCount(0);
  await page.locator("[data-vault-switch]").click();
  await page.locator('[data-vault-scope="all"]').click();
  await expect(page.locator('.side-sync-label')).toHaveCount(2);
  await expect(page.locator('.sheet-list [data-id="ex5"]')).toHaveCount(1);
});

test("search stays within the selected vault and Escape returns focus", async ({ page }) => {
  await openApp(page);
  await page.locator('[data-vault-switch]').click();
  await page.locator('[data-vault-scope="local"]').click();
  await page.locator('[data-side-search]').fill("no such page");
  await expect(page.locator('.sheet-list .side-row')).toHaveCount(0);
  await page.locator('[data-side-search]').press("Escape");
  await expect(page.locator('[data-side-search]')).toHaveValue("");
  await expect(page.locator('[data-vault-switch]')).toBeFocused();
  await expect(page.locator('.sheet-list .side-row')).toHaveCount(1);
});

test("search finds pages in collapsed folders without opening the folder", async ({ page }) => {
  await openApp(page);
  await page.locator('[data-id="ex-folder"] .chevron').click();
  await expect(page.locator('[data-id="ex1"]')).toHaveCount(0);
  await page.locator('[data-side-search]').fill("Trip Budget");
  await expect(page.locator('[data-id="ex1"]')).toHaveCount(1);
  await expect(page.locator('[data-id="ex1"] .side-vault-badge')).toHaveText("Personal");
  await page.locator('[data-side-search]').press("Escape");
  await expect(page.locator('[data-id="ex1"]')).toHaveCount(0);
});

test("emptying one vault's trash preserves the other vault's pages", async ({ page }) => {
  await openApp(page);
  await page.evaluate(async () => {
    const store = await import("/app/store.js");
    store.set("trash", JSON.stringify([
      { id: "old-local", text: "Local draft\n2 + 3" },
      { id: "old-vault", text: "Trip draft\n4 + 5" },
    ]));
    globalThis.omacusShowPlaces({ vaultName: "Personal", localIds: ["ex5", "old-local"], move() {} });
  });
  await page.locator('[data-vault-switch]').click();
  await page.locator('[data-vault-scope="local"]').click();
  await page.locator('.deleted-head').click();
  await expect(page.locator('.deleted-rows')).toContainText("Local draft");
  await expect(page.locator('.deleted-rows')).not.toContainText("Trip draft");
  await page.locator('.deleted-empty').click();
  await page.locator('[data-confirm-yes]').click();
  await expect.poll(() => page.evaluate(async () => {
    const store = await import("/app/store.js");
    return JSON.parse(store.get("trash")).map(item => item.id);
  })).toEqual(["old-vault"]);
});

test("creating a local page assigns its destination before it can sync", async ({ page }) => {
  await openApp(page);
  await page.evaluate(() => {
    globalThis.navMoves = [];
    const places = { vaultName: "Personal", localIds: ["ex5"], move(ids, inside) {
      globalThis.navMoves.push({ ids, inside });
      if (!inside) places.localIds.push(...ids);
      globalThis.omacusShowPlaces(places);
    } };
    globalThis.omacusShowPlaces(places);
  });
  await page.locator('[data-side-create-toggle]').click();
  await page.locator('[data-create-destination="local"]').click();
  await expect.poll(() => page.evaluate(() => globalThis.navMoves.length)).toBe(1);
  const move = await page.evaluate(() => globalThis.navMoves[0]);
  expect(move.inside).toBe(false);
  await expect(page.locator('[data-vault-label]')).toHaveText("On this device");
  await expect(page.locator(`.sheet-list [data-id="${move.ids[0]}"]`)).toBeVisible();
});

test("phone navigation fills the screen and keeps 44px page targets", async ({ browser }) => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
  const page = await context.newPage();
  await openApp(page);
  await page.locator('.sheets-toggle').click();
  await expect(page.locator('.side')).toHaveCSS('transform', 'none');
  const geometry = await page.evaluate(() => ({
    width: document.querySelector('.side').getBoundingClientRect().width,
    rows: [...document.querySelectorAll('.sheet-list .side-row')].map(row => row.getBoundingClientRect().height),
  }));
  expect(geometry.width).toBe(390);
  expect(geometry.rows.every(height => height >= 44)).toBe(true);
  await page.locator('[data-vault-switch]').click();
  await page.locator('[data-vault-scope="local"]').click();
  await expect(page.locator('.sheet-list .side-row')).toHaveCount(1);
  await context.close();
});

for (const modifier of ["Control", "Meta"]) {
  test(`${modifier}+F finds in the page without expanding the sidebar`, async ({ page }) => {
    await openApp(page);
    await expect(page.locator('[data-vault-menu] [data-action="collapse-side"]')).toHaveCount(0);
    await page.locator('.side-head [data-action="collapse-side"]').click();
    await expect(page.locator('[data-workspace]')).toHaveClass(/collapsed/);
    await page.keyboard.press(`${modifier}+f`);
    await expect(page.locator('[data-page-find-input]')).toBeFocused();
    await expect(page.locator('[data-workspace]')).toHaveClass(/collapsed/);
    await page.keyboard.type("page");
    await expect(page.locator(".page-match").first()).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-side-search]')).toHaveValue("");
    await page.keyboard.press(`${modifier}+,`);
    await expect(page.locator('[data-settings]')).toBeVisible();
  });
}
