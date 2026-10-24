// Code guesses, join attempts, and reads each have a tighter cap.
// The cap answers before it looks the account up, with the same sentence either way.
import { Mailboxes } from "./worker.js";
import { MemoryStorage } from "./storage.js";
import { makeDevice, makeRecord } from "./auth.js";

const GUESS = 10;
const JOIN = 10;
const READ = 180;
const missing = "cd".repeat(32);
const present = "ce".repeat(32);

function assert(cond, message) {
  if (!cond) throw new Error(message);
}

async function call(box, method, path, body, ip, extra = {}) {
  const response = await box.fetch(new Request("https://sync-test.omacus.com" + path, {
    method,
    headers: {
      "content-type": "application/json",
      "cf-connecting-ip": ip,
      ...extra,
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  }));
  const json = await response.json();
  return { status: response.status, json };
}

async function statuses(box, method, path, body, ip, times) {
  const out = [];
  for (let i = 0; i < times; i += 1) out.push(await call(box, method, path, body, ip));
  return out;
}

const box = new Mailboxes({ storage: new MemoryStorage() }, {});
const device = await makeDevice(present);
const record = await makeRecord(device, "mustard");
const created = await call(box, "POST", "/v1/mailboxes", { account: present, record }, "203.0.113.1", {
  "x-omacus-access-hash": "ab".repeat(32),
  "x-omacus-access-uses": "1",
  "x-omacus-access-spend": "1",
});
assert(created.status === 201, `create ${created.status} ${created.json.message}`);

const guesses = await statuses(box, "POST", "/v1/access", { code: "nope" }, "203.0.113.50", GUESS + 1);
assert(guesses.slice(0, GUESS).every((row) => row.status === 400), "a wrong code is refused");
assert(guesses[GUESS].status === 429, `guess cap ${guesses[GUESS].status}`);
assert(guesses[GUESS].json.message === "Try again in a moment.", "guess sentence");

const joins = await statuses(box, "POST", "/v1/join", { code: "nope" }, "203.0.113.51", JOIN + 1);
assert(joins.slice(0, JOIN).every((row) => row.status === 400), "a wrong join is refused");
assert(joins[JOIN].status === 429, `join cap ${joins[JOIN].status}`);
assert(joins[JOIN].json.message === guesses[GUESS].json.message, "join sentence matches a guess");

const hidden = await statuses(box, "GET", `/v1/mailboxes/${missing}`, undefined, "203.0.113.52", READ + 1);
const known = await statuses(box, "GET", `/v1/mailboxes/${present}`, undefined, "203.0.113.53", READ + 1);
assert(hidden[0].status === 404 && known[0].status === 404, "a read starts as not found");
assert(hidden[0].json.message === known[0].json.message, "missing and present read the same");
assert(hidden[READ].status === 429 && known[READ].status === 429, "the next read waits");
assert(hidden[READ].json.message === known[READ].json.message, "the wait sentence matches");
assert(hidden[READ].json.message === "Try again in a moment.", "read sentence");
assert(!JSON.stringify(hidden[READ].json).includes(missing), "the wait does not name the account");
assert(!JSON.stringify(known[READ].json).includes(present), "the wait does not name the other account");

const still = await call(box, "GET", `/v1/mailboxes/${"cf".repeat(32)}`, undefined, "203.0.113.54");
assert(still.status === 404 && still.json.message === hidden[0].json.message, "another account is not capped");

console.log("limits ok");
