// The handoff from omacus.com to app.omacus.com. The old copy is never deleted.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { mergeIndex } from "../site/app/store.js";
import {
  NEW_ORIGIN, OLD_ORIGIN, decideAck, destinationUrl, validateIncoming,
} from "../site/app/migrate-protocol.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const client = readFileSync(join(root, "site/app/migrate-client.js"), "utf8");
const bridge = readFileSync(join(root, "site/app/bridge.js"), "utf8");
const receiver = readFileSync(join(root, "site/app/migrate.js"), "utf8");

for (const src of [client, bridge, receiver]) {
  assert.equal(src.includes("localStorage.removeItem"), false);
  assert.equal(src.includes("localStorage.clear"), false);
  assert.equal(src.includes(".clear()"), false);
}
assert.match(client, /localStorage\.setItem\(FLAG, "1"\)/);
assert.match(bridge, /result\.status === "failed"/);

const trip = "# Trip Budget\ntotal = 40";
const groceries = "# Groceries\ncost = @trip.total";
const entries = [
  ["omacus:page:trip", trip],
  ["omacus:page:groceries", groceries],
  ["omacus:index", JSON.stringify({ order: ["trip", "groceries"], active: "groceries" })],
];
const message = { type: "omacus-migrate", token: "handoff", entries };

assert.equal(validateIncoming(OLD_ORIGIN, message), true);
assert.equal(validateIncoming(NEW_ORIGIN, message), false);
assert.equal(validateIncoming("https://evilomacus.com", message), false);
assert.equal(validateIncoming("https://omacus.com.evil", message), false);
assert.equal(validateIncoming(OLD_ORIGIN, { ...message, token: "" }), false);
assert.equal(validateIncoming(OLD_ORIGIN, { ...message, entries: [["omacus:migrated", "1"]] }), false);
assert.equal(validateIncoming(OLD_ORIGIN, { ...message, entries: [["omacus:page:trip", 1]] }), false);

assert.deepEqual(
  decideAck({ origin: NEW_ORIGIN, data: { type: "omacus-migrate-ack", ok: true, token: "handoff" }, token: "handoff" }),
  { mark: true, redirect: true, drop: false },
);
for (const origin of [OLD_ORIGIN, "https://evil.example", "https://app.omacus.com.evil"]) {
  const decision = decideAck({
    origin,
    data: { type: "omacus-migrate-ack", ok: true, token: "handoff" },
    token: "handoff",
  });
  assert.equal(decision.mark, false);
  assert.equal(decision.drop, false);
}
assert.equal(
  decideAck({ origin: NEW_ORIGIN, data: { type: "omacus-migrate-ack", ok: true, token: "other" }, token: "handoff" }).mark,
  false,
);

assert.equal(destinationUrl({ search: "?embed", hash: "#v1.abc" }), "https://app.omacus.com/?embed#v1.abc");

const existing = JSON.stringify({
  order: ["trip"],
  active: "trip",
  tree: [{ type: "page", id: "trip" }],
  handles: { trip: "trip" },
});
const incoming = JSON.stringify({
  order: ["groceries"],
  active: "groceries",
  tree: [{ type: "folder", name: "Food", children: [{ type: "page", id: "groceries" }] }],
  handles: { groceries: "groceries", trip: "budget" },
});
const merged = JSON.parse(mergeIndex(existing, incoming));
assert.deepEqual(merged.order, ["trip", "groceries"]);
assert.equal(merged.active, "trip");
assert.equal(merged.handles.trip, "trip");
assert.equal(merged.handles.groceries, "groceries");
assert.ok(merged.tree.some((node) => node.type === "page" && node.id === "groceries"));

console.log("migration checks ok");
