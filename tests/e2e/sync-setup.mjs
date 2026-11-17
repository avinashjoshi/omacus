// An empty library continues right away. These scenarios bring any pages
// already on the device into the set so both copies can be compared.
export async function includeExistingPages(page) {
  const bring = page.getByRole("button", { name: "Bring these pages into sync" });
  await bring.or(page.locator("[data-sync-linked]")).first().waitFor({ state: "visible", timeout: 45000 });
  if (await bring.isVisible()) await bring.click();
}
