import { test, expect } from "@playwright/test";
import { mkdirSync } from "node:fs";

const shots = process.env.OMACUS_SHOTS || "";

async function theme(page, name) {
  await page.evaluate((name) => {
    document.documentElement.dataset.appTheme = name;
  }, name);
}

async function editorFrame(page) {
  return page.locator(".cm-content").evaluate((el) => {
    const cs = getComputedStyle(el);
    const editor = getComputedStyle(el.closest(".cm-editor"));
    return {
      focused: el.closest(".cm-editor").classList.contains("cm-focused"),
      outlineStyle: cs.outlineStyle,
      outlineWidth: cs.outlineWidth,
      boxShadow: cs.boxShadow,
      radius: cs.borderRadius,
      editorOutline: editor.outlineStyle,
      editorShadow: editor.boxShadow,
      tap: cs.getPropertyValue("-webkit-tap-highlight-color"),
    };
  });
}

function expectSeamless(frame) {
  expect(frame.focused).toBe(true);
  expect(frame.outlineStyle).toBe("none");
  expect(frame.outlineWidth).toBe("0px");
  expect(frame.boxShadow).toBe("none");
  expect(frame.radius).toBe("0px");
  expect(frame.editorOutline).toBe("none");
  expect(frame.editorShadow).toBe("none");
  expect(["transparent", "rgba(0, 0, 0, 0)"]).toContain(frame.tap);
}

async function focusEditor(page) {
  await page.locator(".cm-content").evaluate((el) => el.focus());
  await page.waitForFunction(() => document.querySelector(".cm-editor")?.classList.contains("cm-focused"));
}

test.describe("editor focus stays a page", () => {
  for (const name of ["light", "dark"]) {
    for (const viewport of [
      { label: "mobile", width: 390, height: 844 },
      { label: "desktop", width: 1280, height: 800 },
    ]) {
      test(`${name} ${viewport.label}: open, switch, and a new page`, async ({ page }) => {
        await page.setViewportSize({ width: viewport.width, height: viewport.height });
        await page.goto("/app/", { waitUntil: "domcontentloaded" });
        await page.waitForSelector("[data-loading]", { state: "detached" });
        await theme(page, name);
        await expect(page.locator(".cm-content")).toContainText("Welcome");

        await focusEditor(page);
        expectSeamless(await editorFrame(page));

        if (viewport.label === "mobile") await page.locator(".sheets-toggle").click();
        await page.locator('.side-head [data-action="new"]').click();
        await expect(page.locator("[data-empty]")).toBeVisible();
        await expect(page.locator("[data-empty]")).toContainText("Pages are in the sidebar");
        await page.waitForFunction(() => document.querySelector(".cm-editor")?.classList.contains("cm-focused"));
        expectSeamless(await editorFrame(page));
        const hintIsDim = await page.evaluate(() => {
          const hint = getComputedStyle(document.querySelector("[data-empty]")).color;
          const probe = document.createElement("span");
          probe.style.color = "var(--app-dim)";
          document.body.append(probe);
          const dim = getComputedStyle(probe).color;
          probe.remove();
          return hint === dim;
        });
        expect(hintIsDim).toBe(true);
        if (shots && viewport.label === "mobile") {
          mkdirSync(shots, { recursive: true });
          await page.screenshot({ path: `${shots}/editor-focus-after-mobile-${name}.png` });
        }

        if (viewport.label === "mobile") {
          await page.evaluate(() => document.body.classList.remove("drawer"));
          await page.locator(".sheets-toggle").click();
        }
        await page.locator(".side-name", { hasText: "Welcome to Omacus" }).click();
        await expect(page.locator(".cm-content")).toContainText("Welcome");
        await page.waitForFunction(() => document.querySelector(".cm-editor")?.classList.contains("cm-focused"));
        expectSeamless(await editorFrame(page));

        const more = page.locator("[data-menu-button]");
        await more.focus();
        await page.keyboard.press("Tab");
        await page.keyboard.press("Shift+Tab");
        const ring = await more.evaluate((el) => {
          const cs = getComputedStyle(el);
          return {
            active: document.activeElement === el,
            outlineStyle: cs.outlineStyle,
            outlineWidth: parseFloat(cs.outlineWidth),
          };
        });
        expect(ring.active).toBe(true);
        expect(ring.outlineStyle).not.toBe("none");
        expect(ring.outlineWidth).toBeGreaterThan(0);
      });
    }
  }
});
