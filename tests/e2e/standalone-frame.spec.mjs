// The installed iPhone app (display-mode: standalone) used to leave a frosted
// status bar and an empty strip under the page. Safari's viewport is the page,
// and it stays that way. A screen-reader announcement stays in the tree and
// off the page.

import { test, expect } from "@playwright/test";
import { mkdirSync } from "node:fs";

const shots = process.env.OMACUS_SHOTS || "";
const GAP = 70;

// Same phone frame as the before shots: iPhone 13's layout viewport (the
// screen is taller; that extra is not part of this picture).
test.use({
  viewport: { width: 390, height: 664 },
  deviceScaleFactor: 3,
  isMobile: true,
  hasTouch: true,
});

async function pretendInstalled(page) {
  await page.addInitScript((gap) => {
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
      // Already defined. frame.js also accepts the media query.
    }
    const height = () => window.innerHeight + gap;
    const define = (object) => {
      try {
        Object.defineProperty(object, "height", { configurable: true, get: height });
      } catch {
        // Some engines freeze screen.height. The test retries after load.
      }
    };
    define(window.screen);
    define(Screen.prototype);
  }, GAP);
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

async function applyGap(page) {
  return page.evaluate((gap) => {
    const root = document.documentElement;
    const read = () => parseFloat(root.style.getPropertyValue("--standalone-gap")) || 0;
    if (root.classList.contains("standalone") && read() === gap) return { gap: read(), fromFrame: true };
    const height = () => window.innerHeight + gap;
    let defined = false;
    try {
      Object.defineProperty(window.screen, "height", { configurable: true, get: height });
      defined = window.screen.height === window.innerHeight + gap;
    } catch {
      defined = false;
    }
    if (defined) {
      window.dispatchEvent(new Event("resize"));
      if (root.classList.contains("standalone") && read() === gap) return { gap: read(), fromFrame: true };
    }
    root.classList.add("standalone");
    root.style.setProperty("--standalone-gap", gap + "px");
    return { gap: read(), fromFrame: false };
  }, GAP);
}

async function frame(page) {
  return page.evaluate(() => {
    const shell = document.querySelector(".app-page");
    const bar = document.querySelector(".bar");
    const side = document.querySelector(".side");
    const foot = document.querySelector(".side-foot");
    const barCs = getComputedStyle(bar);
    const gap = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--standalone-gap")) || 0;
    return {
      standalone: document.documentElement.classList.contains("standalone"),
      gap,
      innerHeight: window.innerHeight,
      shellBottom: shell.getBoundingClientRect().bottom,
      sideBottom: side.getBoundingClientRect().bottom,
      sidePad: parseFloat(getComputedStyle(side).paddingBottom),
      footBottom: foot.getBoundingClientRect().bottom,
      barH: bar.getBoundingClientRect().height,
      barPadTop: parseFloat(barCs.paddingTop),
      barBg: barCs.backgroundColor,
      pageBg: getComputedStyle(shell).backgroundColor,
      blur: barCs.backdropFilter,
      status: document.querySelector('meta[name="apple-mobile-web-app-status-bar-style"]')?.outerHTML || "",
    };
  });
}

function expectBar(box) {
  expect(box.status).toBe("");
  expect(box.blur).toBe("none");
  expect(box.barBg).toBe(box.pageBg);
  expect(box.barPadTop).toBe(59);
  expect(Math.abs(box.barH - (56 + 59))).toBeLessThanOrEqual(1);
  expect(box.sidePad).toBe(34);
}

test.describe("installed iPhone", () => {
  test.beforeEach(async ({ page }) => {
    await pretendInstalled(page);
  });

  test("the page and the sidebar fill the screen, and the bar is solid", async ({ page }, info) => {
    await openApp(page);
    const applied = await applyGap(page);
    expect(applied.fromFrame).toBe(true);
    const box = await frame(page);
    expect(box.standalone).toBe(true);
    expect(box.gap).toBe(GAP);
    const edge = box.innerHeight + GAP;
    expect(Math.abs(box.shellBottom - edge)).toBeLessThanOrEqual(1);
    expect(Math.abs(box.sideBottom - edge)).toBeLessThanOrEqual(1);
    expect(box.footBottom).toBeLessThanOrEqual(edge - 34 + 1);
    expectBar(box);

    if (shots && info.project.name === "webkit") {
      mkdirSync(shots, { recursive: true });
      for (const name of ["dark", "light"]) {
        await page.evaluate((theme) => {
          document.documentElement.dataset.appTheme = theme;
        }, name);
        await page.screenshot({ path: `${shots}/iphone-standalone-after-${name}.png` });
        await page.locator(".sheets-toggle").click();
        await page.waitForFunction(() => document.querySelector(".side").getBoundingClientRect().left >= -1);
        await page.screenshot({ path: `${shots}/iphone-standalone-after-drawer-${name}.png` });
        await page.evaluate(() => document.body.classList.remove("drawer"));
      }
    }

    const capped = await page.evaluate(() => {
      Object.defineProperty(window.screen, "height", {
        configurable: true,
        get: () => window.innerHeight + 400,
      });
      window.dispatchEvent(new Event("resize"));
      const root = document.documentElement;
      const shell = document.querySelector(".app-page").getBoundingClientRect().bottom;
      return {
        gap: parseFloat(root.style.getPropertyValue("--standalone-gap")) || 0,
        shell,
        innerHeight: window.innerHeight,
      };
    });
    expect(capped.gap).toBe(0);
    expect(Math.abs(capped.shell - capped.innerHeight)).toBeLessThanOrEqual(1);
  });

  test("Selection deleted stays off the page and in the tree", async ({ page }) => {
    await openApp(page);
    await applyGap(page);
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
    expect(box.standalone).toBe(false);
    expect(box.gap).toBe(0);
    expect(Math.abs(box.shellBottom - box.innerHeight)).toBeLessThanOrEqual(1);
    expect(Math.abs(box.sideBottom - box.innerHeight)).toBeLessThanOrEqual(1);
    expect(box.footBottom).toBeLessThanOrEqual(box.innerHeight - 34 + 1);
    expectBar(box);
    if (shots && info.project.name === "webkit") {
      mkdirSync(shots, { recursive: true });
      await page.evaluate(() => {
        document.documentElement.dataset.appTheme = "dark";
      });
      await page.screenshot({ path: `${shots}/iphone-safari-after-dark.png` });
    }
  });
});
