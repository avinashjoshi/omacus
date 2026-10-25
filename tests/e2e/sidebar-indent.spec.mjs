// Section captions, pages, and folder children share one indent step.
// Chevrons line up on that step, and a page name lines up with a folder name.

import { test, expect } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.route("**/app/**", async (route) => {
    if (route.request().resourceType() !== "document") return route.continue();
    const response = await route.fetch();
    const body = (await response.text()).replace("<html lang=\"en\">", "<html lang=\"en\" data-sync-test=\"1\">");
    await route.fulfill({ response, body });
  });
});

async function openApp(page) {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/app/", { waitUntil: "domcontentloaded" });
  await page.waitForSelector("[data-loading]", { state: "detached" });
  await page.waitForSelector(".cm-content");
  await page.waitForSelector(".side-row");
}

function near(actual, expected) {
  expect(Math.abs(actual - expected), `${actual} vs ${expected}`).toBeLessThanOrEqual(1);
}

test("a page and a folder on one level share a name column", async ({ page }) => {
  await openApp(page);
  const box = await page.evaluate(() => {
    const left = (sel) => document.querySelector(sel).getBoundingClientRect().left;
    const step = parseFloat(getComputedStyle(document.querySelector(".side")).getPropertyValue("--row-step"));
    return {
      step,
      sections: document.querySelectorAll(".side-section").length,
      folderChevron: left('[data-id="ex-folder"] .chevron'),
      folderText: left('[data-id="ex-folder"] .side-name span'),
      pageText: left('[data-id="ex-welcome"] .side-name span'),
      childText: left('[data-id="ex1"] .side-name span'),
    };
  });
  expect(box.sections).toBe(0);
  expect(box.step).toBe(16);
  near(box.folderText, box.pageText);
  near(box.childText - box.folderText, box.step);
  expect(box.folderChevron).toBeLessThan(box.folderText - 8);
});

test("captions, pages, and folder children step in together", async ({ page }) => {
  await openApp(page);
  // Mount clears places once, after the loading screen is gone. Set them after that.
  await page.waitForFunction(() => document.querySelector(".sync-chip")?.dataset.ready === "1");
  const box = await page.evaluate(() => {
    globalThis.omacusShowPlaces({
      vaultName: "My pages",
      localIds: ["ex5"],
      here: "On this computer",
      move() {},
    });
    const left = (sel) => document.querySelector(sel).getBoundingClientRect().left;
    const right = (sel) => document.querySelector(sel).getBoundingClientRect().right;
    const step = parseFloat(getComputedStyle(document.querySelector(".sheet-list")).getPropertyValue("--row-step"));
    const mores = [...document.querySelectorAll(".side-row .row-more")].map((el) => Math.round(el.getBoundingClientRect().right));
    return {
      step,
      sectionChevron: left('.side-sync-label[data-sync-section="vault"] .chevron'),
      sectionText: left('.side-sync-label[data-sync-section="vault"] .side-section span:not(.chevron)'),
      localText: left('.side-sync-label[data-sync-section="local"] .side-section span:not(.chevron)'),
      folderChevron: left('[data-id="ex-folder"] .chevron'),
      folderText: left('[data-id="ex-folder"] .side-name span'),
      pageText: left('[data-id="ex-welcome"] .side-name span'),
      childText: left('[data-id="ex1"] .side-name span'),
      hereText: left('[data-id="ex5"] .side-name span'),
      headText: left(".side-head span"),
      mores,
      sectionRight: right('.side-sync-label[data-sync-section="vault"] .side-section'),
    };
  });
  expect(box.step).toBe(16);
  near(box.folderText, box.pageText);
  near(box.hereText, box.pageText);
  near(box.sectionText, box.localText);
  near(box.headText, box.sectionText);
  near(box.pageText - box.sectionText, box.step);
  near(box.childText - box.pageText, box.step);
  near(box.folderChevron - box.sectionChevron, box.step);
  near(box.folderText - box.folderChevron, box.sectionText - box.sectionChevron);
  const column = box.mores[0];
  expect(box.mores.length).toBeGreaterThan(2);
  for (const edge of box.mores) near(edge, column);
});
