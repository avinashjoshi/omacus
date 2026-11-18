// A folded phone: a short, narrow screen with a notch. Recently deleted stays
// one row until it is opened, the preview's buttons and the page switcher sit
// under the bar, and a question sits above the pages drawer.

import { test, expect } from "@playwright/test";

test.use({ viewport: { width: 466, height: 678 }, hasTouch: true, isMobile: true });

async function openApp(page, { mockNotch = true } = {}) {
  if (mockNotch) {
    // The notch a test browser reports as 0.
    await page.addInitScript(() => {
      const set = () => document.body?.style.setProperty("--sat", "54px");
      document.addEventListener("DOMContentLoaded", set);
      set();
    });
  }
  await page.goto("/app/", { waitUntil: "domcontentloaded" });
  await page.waitForSelector("[data-loading]", { state: "detached" });
  await page.waitForSelector(".cm-content");
  await page.waitForSelector(".side-row");
}

async function openDrawer(page) {
  if (!(await page.evaluate(() => document.body.classList.contains("drawer")))) {
    await page.locator(".sheets-toggle").tap();
  }
  await expect(page.locator("body")).toHaveClass(/drawer/);
}

async function barBottom(page) {
  return page.evaluate(() => document.querySelector(".bar").getBoundingClientRect().bottom);
}

async function deleteFirstPage(page) {
  await openDrawer(page);
  await page.locator(".sheet-list .side-row[data-kind=page] .row-more").first().tap();
  await page.locator("[data-row-menu] button", { hasText: "Delete page" }).tap();
  await page.locator("[data-confirm-yes]").tap();
  await expect(page.locator("[data-confirm]")).toBeHidden();
}

test("Recently deleted is one row until it is opened", async ({ page }) => {
  await openApp(page);
  for (let n = 0; n < 5; n += 1) await deleteFirstPage(page);
  await openDrawer(page);

  const head = page.locator("[data-deleted] .deleted-head");
  await expect(head).toHaveText("Recently deleted · 5");
  await expect(head).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator("[data-deleted] .deleted-rows")).toBeHidden();

  const closed = await page.evaluate(() => ({
    list: document.querySelector("[data-sheets]").getBoundingClientRect().height,
    trash: document.querySelector("[data-deleted]").getBoundingClientRect().height,
  }));
  expect(closed.trash).toBeLessThan(64);
  expect(closed.list).toBeGreaterThan(closed.trash);

  await head.tap();
  await expect(page.locator("[data-deleted] .deleted-rows button")).toHaveCount(6);
  const open = await page.evaluate(() => {
    const box = (sel) => document.querySelector(sel).getBoundingClientRect();
    return {
      side: box(".side-body").height,
      list: box("[data-sheets]").height,
      trash: box("[data-deleted]"),
      tools: box(".side-tools").top,
    };
  });
  expect(open.trash.height).toBeLessThanOrEqual(open.side * 0.4 + 1);
  expect(open.trash.bottom).toBeLessThanOrEqual(open.tools + 1);
  expect(open.list).toBeGreaterThanOrEqual(96);

  // A deleted page opens over the drawer's page, and the drawer gets out of the way.
  await page.locator("[data-deleted] .deleted-rows button:not(.deleted-empty)").first().tap();
  await expect(page.locator("[data-preview]")).toBeVisible();
  await expect(page.locator("body")).not.toHaveClass(/drawer/);
});

test("the preview's Edit button is under the bar and can be pressed", async ({ page }) => {
  await openApp(page);
  await page.locator("[data-menu-button]").tap();
  await page.locator("[data-menu-open=copy]").tap();
  await page.locator("[data-menu] [data-action=preview]").tap();
  const edit = page.locator("[data-action=close-preview]");
  await expect(edit).toBeVisible();
  const top = (await edit.boundingBox()).y;
  expect(top).toBeGreaterThanOrEqual(await barBottom(page));
  await edit.tap();
  await expect(page.locator("[data-page-preview]")).toBeHidden();
});

test("a question sits above the pages drawer", async ({ page }) => {
  await openApp(page);
  await openDrawer(page);
  await page.locator(".sheet-list .side-row[data-kind=page] .row-more").first().tap();
  await page.locator("[data-row-menu] button", { hasText: "Delete page" }).tap();
  const onTop = await page.evaluate(() => {
    const dialog = document.querySelector("[data-confirm]");
    const box = dialog.getBoundingClientRect();
    // The left edge of the question is over the drawer.
    const hit = document.elementFromPoint(box.left + 12, box.top + 12);
    return dialog.contains(hit);
  });
  expect(onTop).toBe(true);

  // The page and the drawer behind the question cannot be pressed.
  const behind = await page.evaluate(() => {
    const at = (x, y) => document.elementFromPoint(x, y)?.className || "";
    return { page: at(440, 600), drawer: at(60, 600) };
  });
  expect(behind.page).toBe("confirm-scrim");
  expect(behind.drawer).toBe("confirm-scrim");
  await page.locator("[data-confirm-no]").tap();
});

test("the page switcher opens under the bar", async ({ page }) => {
  await openApp(page);
  await page.keyboard.press("Control+j");
  const field = page.locator("[data-switcher-query]");
  await expect(field).toBeVisible();
  expect((await field.boundingBox()).y).toBeGreaterThanOrEqual(await barBottom(page));
});

test("Settings keeps its title above the scrolling groups", async ({ page }) => {
  await openApp(page);
  await openDrawer(page);
  await page.locator(".side-tools [data-action=settings]").tap();
  const gap = await page.evaluate(() => {
    const card = document.querySelector(".settings-card");
    card.scrollTop = card.scrollHeight;
    return document.querySelector(".settings-head").getBoundingClientRect().top - card.getBoundingClientRect().top;
  });
  // Nothing scrolls through a strip above the title.
  expect(gap).toBeLessThanOrEqual(1);
});

async function swipe(page, from, to, { y = 400, hold = 0 } = {}) {
  const cdp = await page.context().newCDPSession(page);
  const point = (x) => [{ x, y, id: 1 }];
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: point(from) });
  if (hold) await page.waitForTimeout(hold);
  const steps = 6;
  for (let n = 1; n <= steps; n += 1) {
    await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: point(from + ((to - from) * n) / steps) });
    await page.waitForTimeout(16);
  }
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
}

test("a swipe from the left edge opens the pages, and a swipe back closes them", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", "native touch injection uses Chromium DevTools");
  await openApp(page);

  await swipe(page, 8, 260);
  await expect(page.locator("body")).toHaveClass(/drawer/);
  await expect(page.locator(".sheets-toggle")).toHaveAttribute("aria-expanded", "true");
  // Nothing is left on the drawer from following the finger.
  expect(await page.evaluate(() => document.getElementById("side").style.transform)).toBe("");

  // Starting beside the drawer, on the dimmed page.
  await swipe(page, 440, 180);
  await expect(page.locator("body")).not.toHaveClass(/drawer/);
  // The swipe did not put the caret in the page or bring up the keyboard.
  expect(await page.evaluate(() => document.activeElement?.closest?.(".cm-editor") != null)).toBe(false);

  // A swipe that starts in the middle of the page is the page's own.
  await swipe(page, 200, 420);
  await expect(page.locator("body")).not.toHaveClass(/drawer/);

  // A finger that rests at the edge first is placing the caret, not opening the pages.
  await swipe(page, 8, 260, { hold: 400 });
  await expect(page.locator("body")).not.toHaveClass(/drawer/);
});

test("Empty Recently deleted asks first, then clears the list", async ({ page }) => {
  await openApp(page);
  for (let n = 0; n < 3; n += 1) await deleteFirstPage(page);
  await openDrawer(page);
  await page.locator("[data-deleted] .deleted-head").tap();
  await page.locator("[data-deleted] .deleted-empty").tap();

  const yes = page.locator("[data-confirm-yes]");
  await expect(page.locator("[data-confirm-text]")).toHaveText("Delete all 3 pages in Recently deleted forever? This can't be undone.");
  await expect(yes).toHaveText("Delete forever");
  await expect(yes).toHaveClass(/danger/);

  // Cancel keeps every page.
  await page.locator("[data-confirm-no]").tap();
  await expect(page.locator("[data-deleted] .deleted-rows button:not(.deleted-empty)")).toHaveCount(3);

  await page.locator("[data-deleted] .deleted-empty").tap();
  await yes.tap();
  await expect(page.locator("[data-deleted]")).toBeHidden();

  // The next ordinary delete is not a danger button.
  await deleteFirstPage(page);
  await page.locator(".sheet-list .side-row[data-kind=page] .row-more").first().tap();
  await page.locator("[data-row-menu] button", { hasText: "Delete page" }).tap();
  await expect(yes).not.toHaveClass(/danger/);
});

test("the wordmark in the bar is not a link", async ({ page }) => {
  await page.setViewportSize({ width: 890, height: 626 });
  await openApp(page);
  await expect(page.locator(".bar .brand")).toBeVisible();
  await expect(page.locator(".bar a.brand")).toHaveCount(0);
});

test("a toast sits above the keyboard and lets taps through its words", async ({ page }) => {
  await openApp(page);
  await deleteFirstPage(page);
  const toast = page.locator("[data-toast]");
  await expect(toast).toBeVisible();
  const rest = (await toast.boundingBox()).y;
  // The keyboard, as frame.js reports it.
  await page.evaluate(() => document.body.style.setProperty("--kb", "300px"));
  expect((await toast.boundingBox()).y).toBeCloseTo(rest - 300, 0);
  const style = await page.evaluate(() => ({
    card: getComputedStyle(document.querySelector("[data-toast]")).pointerEvents,
    undo: getComputedStyle(document.querySelector("[data-toast] button")).pointerEvents,
  }));
  expect(style).toEqual({ card: "none", undo: "auto" });
});

test("on a touch screen a page opens to read, and a tap on it starts editing", async ({ page }) => {
  await openApp(page);
  const editing = () => page.evaluate(() => document.activeElement?.closest?.(".cm-editor") != null);
  const open = async (id) => {
    await openDrawer(page);
    await page.locator(`.side-row[data-id="${id}"] .side-name`).tap();
    await expect(page.locator("body")).not.toHaveClass(/drawer/);
  };

  await open("ex1");
  expect(await editing()).toBe(false);

  // Typing in one page, then opening another, puts the keyboard away.
  await page.locator(".cm-content").tap();
  expect(await editing()).toBe(true);
  await open("ex2");
  expect(await editing()).toBe(false);

  // The switch in Settings starts off here, and turning it on is remembered.
  await openDrawer(page);
  await page.locator(".side-tools [data-action=settings]").tap();
  const toggle = page.locator("[data-setting=type-on-open]");
  await expect(toggle).not.toBeChecked();
  await toggle.tap();
  await page.locator(".settings-head [data-action=close-settings]").tap();
  await open("ex1");
  expect(await editing()).toBe(true);
  await page.reload();
  await page.waitForSelector(".side-row");
  await open("ex2");
  expect(await editing()).toBe(true);
});

test("with the keyboard up the app stays inside what can be seen", async ({ page }) => {
  await openApp(page);
  // What an iPhone reports: the page slid up 18px, and the keys cover 300px.
  const box = await page.evaluate(() => {
    document.documentElement.style.setProperty("--vv-top", "18px");
    document.body.style.setProperty("--kb", "300px");
    const rect = (sel) => document.querySelector(sel).getBoundingClientRect();
    document.body.classList.add("drawer");
    return {
      // The shell's own bottom edge is the bottom of the screen.
      height: rect(".app-page").bottom,
      bar: rect(".bar").top,
      page: rect(".sheet"),
      scroller: rect(".cm-scroller").bottom,
      drawer: rect(".side-body"),
    };
  });
  expect(Math.abs(box.bar - 18)).toBeLessThanOrEqual(1);
  expect(Math.abs(box.page.top - 18)).toBeLessThanOrEqual(1);
  expect(Math.abs(box.page.bottom - (box.height - 300))).toBeLessThanOrEqual(1);
  expect(box.scroller).toBeLessThanOrEqual(box.height - 300 + 1);
  expect(Math.abs(box.drawer.top - 18)).toBeLessThanOrEqual(1);
  expect(Math.abs(box.drawer.bottom - (box.height - 300))).toBeLessThanOrEqual(1);
});

test("the sidebar footer is one row of three under a finger", async ({ page }) => {
  await openApp(page);
  await openDrawer(page);
  const rows = await page.evaluate(() =>
    [...document.querySelectorAll(".side-tools > .side-foot-row")]
      .filter((node) => !node.hidden)
      .map((node) => {
        const box = node.getBoundingClientRect();
        return { word: node.textContent.trim(), top: Math.round(box.top), width: box.width, height: box.height };
      }));
  expect(rows.map((row) => row.word)).toEqual(["Turn on sync", "Settings", "About"]);
  expect(new Set(rows.map((row) => row.top)).size).toBe(1);
  for (const row of rows) {
    expect(row.height).toBeGreaterThanOrEqual(44);
    expect(row.width).toBeGreaterThanOrEqual(44);
  }
  // Every word is shown whole.
  const clipped = await page.evaluate(() =>
    [...document.querySelectorAll(".side-tools > .side-foot-row span")]
      .filter((node) => !node.closest("[hidden]") && node.scrollWidth > node.clientWidth + 1).length);
  expect(clipped).toBe(0);
});

test("the bar keeps its buttons out of an unfolded phone's camera corner", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (iPhone)" });
    Object.defineProperty(navigator, "standalone", { value: true });
  });
  const root = page.locator("html");
  const rightInset = () => page.evaluate(() =>
    window.innerWidth - document.querySelector("[data-menu-button]").getBoundingClientRect().right);
  await openApp(page, { mockNotch: false });
  await expect(root).not.toHaveAttribute("data-island", /.+/);
  expect(await rightInset()).toBeLessThan(40);

  await page.setViewportSize({ width: 951, height: 669 });
  await expect(root).toHaveAttribute("data-island", "wide");
  await expect.poll(rightInset).toBeGreaterThanOrEqual(80);

  // The Duo's installed portrait viewport reports zero top and right insets.
  await page.setViewportSize({ width: 669, height: 951 });
  await expect(root).toHaveAttribute("data-island", "tall");
  await expect.poll(rightInset).toBeGreaterThanOrEqual(100);
  const bar = await page.evaluate(() => {
    const button = document.querySelector("[data-menu-button]").getBoundingClientRect();
    return {
      bottom: document.querySelector(".bar").getBoundingClientRect().bottom,
      center: button.top + button.height / 2,
    };
  });
  expect(bar.bottom).toBeGreaterThanOrEqual(72);
  expect(Math.abs(bar.center - 48)).toBeLessThanOrEqual(2);
});

test("with the keyboard up the sidebar keeps Recently deleted whole", async ({ page }) => {
  await page.setViewportSize({ width: 952, height: 670 });
  await openApp(page);
  await page.locator('[data-id="ex1"] .row-more').tap();
  await page.locator("[data-row-menu] button", { hasText: "Delete page" }).tap();
  await page.locator("[data-confirm-yes]").tap();
  // The keys cover the bottom 330px, as on an unfolded phone held wide.
  const box = await page.evaluate(() => {
    document.body.style.setProperty("--kb", "330px");
    document.body.classList.add("kb-up");
    const rect = (sel) => document.querySelector(sel).getBoundingClientRect();
    return {
      head: rect(".deleted-head"),
      trash: rect("[data-deleted]"),
      tools: rect(".side-tools"),
      side: rect(".side"),
      version: getComputedStyle(document.querySelector(".side-foot")).display,
    };
  });
  expect(box.head.bottom).toBeLessThanOrEqual(box.trash.bottom + 1);
  expect(box.trash.bottom).toBeLessThanOrEqual(box.tools.top + 1);
  expect(box.tools.bottom).toBeLessThanOrEqual(box.side.bottom + 1);
  expect(box.version).toBe("none");
});
