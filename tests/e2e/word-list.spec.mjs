import { test, expect } from "@playwright/test";

test("an installed page does not use the app page as the word list", async ({ page }) => {
  await page.goto("/app/?sw=1", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(() => navigator.serviceWorker?.controller);
  const result = await page.evaluate(async () => {
    const response = await fetch("./words.txt");
    const type = response.headers.get("content-type") || "";
    const text = await response.text();
    return {
      html: type.includes("text/html") || text.includes("embed-open") || text.includes("<!DOCTYPE") || text.includes("move-folder"),
      offline: text.trim() === "Offline",
    };
  });
  expect(result.html).toBe(false);
  expect(result.offline).toBe(true);
});
