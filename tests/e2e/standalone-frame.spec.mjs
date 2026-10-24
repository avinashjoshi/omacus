// Home Screen: the document is 100vh and the shell is pinned to the top and
// bottom of the viewport. A percentage height drops viewport-fit=cover, and
// the dynamic viewport unit is wrong until the phone rotates. A taller
// screen.height must not move the shell. black-translucent lets the page
// paint under the status bar; the bar itself is solid. Playwright cannot
// draw the iOS status bar.

import { test, expect } from "@playwright/test";
import { mkdirSync } from "node:fs";

const shots = process.env.OMACUS_SHOTS || "";

test.use({
  viewport: { width: 390, height: 664 },
  deviceScaleFactor: 3,
  isMobile: true,
  hasTouch: true,
});

async function pretendInstalled(page) {
  await page.addInitScript(() => {
    const real = window.matchMedia.bind(window);
    window.matchMedia = (query) => {
      const text = String(query);
      if (text.includes("display-mode") && text.includes("standalone")) {
        return {
          matches: true,
          media: text,
          onchange: null,
          addListener() {},
          removeListener() {},
          addEventListener() {},
          removeEventListener() {},
          dispatchEvent() {
            return false;
          },
        };
      }
      return real(query);
    };
    try {
      Object.defineProperty(Navigator.prototype, "standalone", {
        configurable: true,
        get: () => true,
      });
    } catch {
      // Already defined.
    }
  });
}

async function openApp(page) {
  await page.goto("/app/", { waitUntil: "domcontentloaded" });
  await page.waitForSelector("[data-loading]", { state: "detached" });
  await page.waitForSelector(".cm-content");
  await page.evaluate(() => {
    document.body.style.setProperty("--safe-top", "59px");
    document.body.style.setProperty("--safe-bottom", "34px");
  });
}

async function frame(page) {
  return page.evaluate(() => {
    const shell = document.querySelector(".app-page");
    const bar = document.querySelector(".bar");
    const btn = document.querySelector(".bar .icon-btn");
    const side = document.querySelector(".side");
    const foot = document.querySelector(".side-foot");
    const scrim = document.querySelector("[data-scrim]");
    const barCs = getComputedStyle(bar);
    const btnCs = getComputedStyle(btn);
    const probe = document.createElement("div");
    probe.style.background = "var(--app-bg)";
    document.body.appendChild(probe);
    const appBg = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return {
      innerHeight: window.innerHeight,
      shellTop: shell.getBoundingClientRect().top,
      shellBottom: shell.getBoundingClientRect().bottom,
      sideBottom: side.getBoundingClientRect().bottom,
      sidePad: parseFloat(getComputedStyle(side).paddingBottom),
      footBottom: foot.getBoundingClientRect().bottom,
      closedShadow: getComputedStyle(side).boxShadow,
      barH: bar.getBoundingClientRect().height,
      barTop: bar.getBoundingClientRect().top,
      barPadTop: parseFloat(barCs.paddingTop),
      barBg: barCs.backgroundColor,
      pageBg: getComputedStyle(shell).backgroundColor,
      htmlBg: getComputedStyle(document.documentElement).backgroundColor,
      appBg,
      blur: barCs.backdropFilter,
      btnBlur: btnCs.backdropFilter,
      btnBg: btnCs.backgroundColor,
      scrimTop: parseFloat(getComputedStyle(scrim).top),
      barBottom: bar.getBoundingClientRect().bottom,
      status: document.querySelector('meta[name="apple-mobile-web-app-status-bar-style"]')?.outerHTML || "",
      manifest: !!document.querySelector('link[rel="manifest"]'),
    };
  });
}

function expectFlush(box) {
  expect(box.status).toContain("black-translucent");
  expect(box.blur).toBe("none");
  expect(box.btnBlur).toBe("none");
  expect(box.barBg).toBe(box.pageBg);
  expect(box.htmlBg).toBe(box.appBg);
  expect(box.pageBg).toBe(box.appBg);
  expect(box.btnBg).toBe(box.appBg);
  expect(box.barTop).toBe(0);
  expect(box.barPadTop).toBe(59);
  expect(Math.abs(box.barH - (56 + 59))).toBeLessThanOrEqual(1);
  expect(box.scrimTop).toBeGreaterThanOrEqual(box.barBottom - 1);
  expect(box.sidePad).toBe(34);
  expect(box.closedShadow).toBe("none");
  expect(Math.abs(box.shellTop)).toBeLessThanOrEqual(1);
  expect(Math.abs(box.shellBottom - box.innerHeight)).toBeLessThanOrEqual(1);
  expect(Math.abs(box.sideBottom - box.innerHeight)).toBeLessThanOrEqual(1);
  expect(box.footBottom).toBeLessThanOrEqual(box.innerHeight - 34 + 1);
}

async function lieAboutScreen(page, extra) {
  return page.evaluate((extra) => {
    Object.defineProperty(window.screen, "height", {
      configurable: true,
      get: () => window.innerHeight + extra,
    });
    window.dispatchEvent(new Event("resize"));
    if (window.visualViewport) window.visualViewport.dispatchEvent(new Event("resize"));
    const shell = document.querySelector(".app-page").getBoundingClientRect();
    return { top: shell.top, bottom: shell.bottom, innerHeight: window.innerHeight };
  }, extra);
}

test.describe("installed iPhone", () => {
  test.beforeEach(async ({ page }) => {
    await pretendInstalled(page);
  });

  test("the page and the sidebar fill the viewport, and the bar is solid", async ({ page }, info) => {
    await openApp(page);
    const box = await frame(page);
    expectFlush(box);

    for (const extra of [62, 70, 400]) {
      const lied = await lieAboutScreen(page, extra);
      expect(Math.abs(lied.top)).toBeLessThanOrEqual(1);
      expect(Math.abs(lied.bottom - lied.innerHeight)).toBeLessThanOrEqual(1);
    }

    if (shots && info.project.name === "webkit") {
      mkdirSync(shots, { recursive: true });
      await page.waitForFunction(() => document.querySelectorAll(".res").length >= 8);
      for (const name of ["dark", "light"]) {
        await page.evaluate((theme) => {
          document.documentElement.dataset.appTheme = theme;
        }, name);
        await page.screenshot({ path: `${shots}/iphone-fill-after-${name}.png` });
        await page.locator(".sheets-toggle").click();
        await page.waitForFunction(() => document.querySelector(".side").getBoundingClientRect().left >= -1);
        await page.screenshot({ path: `${shots}/iphone-fill-drawer-${name}.png` });
        await page.evaluate(() => document.body.classList.remove("drawer"));
      }
    }
  });

  test("Selection deleted stays off the page and in the tree", async ({ page }) => {
    await openApp(page);
    const content = page.locator(".cm-content");
    await content.evaluate((el) => el.focus());
    await page.keyboard.press("Control+a");
    await page.keyboard.press("Backspace");
    const announced = page.locator(".cm-announced");
    await expect(announced).toHaveText(/Selection deleted/, { timeout: 5000 });
    const box = await announced.evaluate((el) => {
      const cs = getComputedStyle(el);
      const r = el.getBoundingClientRect();
      return {
        live: el.getAttribute("aria-live"),
        display: cs.display,
        visibility: cs.visibility,
        width: r.width,
        height: r.height,
        clip: cs.clipPath,
      };
    });
    expect(box.live).toBe("polite");
    expect(box.display).not.toBe("none");
    expect(box.visibility).not.toBe("hidden");
    expect(box.width).toBeLessThanOrEqual(2);
    expect(box.height).toBeLessThanOrEqual(2);
    expect(box.clip).toContain("inset");
  });
});

test.describe("Safari", () => {
  test("the page stays on the viewport", async ({ page }, info) => {
    await openApp(page);
    const box = await frame(page);
    expectFlush(box);
    if (shots && info.project.name === "webkit") {
      mkdirSync(shots, { recursive: true });
      await page.evaluate(() => {
        document.documentElement.dataset.appTheme = "dark";
      });
      await page.screenshot({ path: `${shots}/iphone-safari-after-dark.png` });
    }
  });
});
