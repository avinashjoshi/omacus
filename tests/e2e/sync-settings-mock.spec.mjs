// Local screenshots of the Sync tab. Skipped in CI.
import { test } from "@playwright/test";
import { mkdirSync } from "node:fs";

test.skip(process.env.OMACUS_SYNC_MOCK_SHOTS !== "1", "local mock screenshots");

const SCREENS = [
  "intro", "fresh", "words", "check", "join-words", "join-wait",
  "approve", "approve-timeout", "devices", "remove", "paused",
  "list-changed", "keyring", "off",
];

test("Sync tab screens in light and dusk", async ({ page }) => {
  const dir = "/opt/cursor/artifacts/screenshots";
  mkdirSync(dir, { recursive: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  for (const theme of ["light", "dusk"]) {
    for (const name of SCREENS) {
      await page.goto(`/app/?syncMock=${name}&theme=${theme}`, { waitUntil: "domcontentloaded" });
      await page.waitForSelector("[data-loading]", { state: "detached" });
      await page.waitForSelector(`[data-settings-panel="sync"] .settings-group`);
      await page.waitForSelector(".side-row");
      await page.evaluate((themeName) => {
        document.documentElement.dataset.appTheme = themeName;
      }, theme);
      await page.screenshot({ path: `${dir}/sync-tab-web-${theme}-${name}.png` });
    }
  }
});
