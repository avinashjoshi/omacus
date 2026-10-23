// Receives pages from omacus.com. Confirms only after IndexedDB has been read
// back. The sender decides what to do with its own copy.

import { acceptEntries } from "./store.js";
import { OLD_ORIGIN, validateIncoming } from "./migrate-protocol.js";

addEventListener("message", async (e) => {
  if (e.source !== parent || !validateIncoming(e.origin, e.data)) return;
  const ok = await acceptEntries(e.data.entries);
  if (!ok) return;
  parent.postMessage({ type: "omacus-migrate-ack", token: e.data.token, ok: true }, e.origin);
});

parent.postMessage({ type: "omacus-migrate-ready" }, OLD_ORIGIN);
