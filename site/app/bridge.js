// omacus.com/app. Copies saved pages to app.omacus.com, then redirects.
// The copy on this origin stays until the new origin confirms, and stays after that too.

import { destinationUrl } from "./migrate-protocol.js";
import { migrateStoredPages } from "./migrate-client.js";

const status = document.querySelector("[data-status]");
const retry = document.querySelector("[data-retry]");

async function go() {
  if (retry) retry.hidden = true;
  if (status) status.textContent = "Moving your saved pages, then opening the app.";
  const result = await migrateStoredPages();
  if (result.status === "failed") {
    if (status) status.textContent = "Couldn't move your pages yet. They're still saved in this browser.";
    if (retry) retry.hidden = false;
    return;
  }
  location.replace(destinationUrl(location));
}

document.querySelector("[data-retry-btn]")?.addEventListener("click", () => {
  go();
});

go();
