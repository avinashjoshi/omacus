// Linking under the real test limits. The clock is the worker's rate clock.
// No x-omacus-limits header. Three phones save, one joins, all on one address.
import { readFile } from "node:fs/promises";
import { Mailboxes, HEARTBEAT_MS } from "./worker.js";
import { MemoryStorage } from "./storage.js";
import { PRODUCTION, TEST } from "./limits.js";
import { appendDevice, authHeaders, deviceChain, makeDevice, makeRecord } from "./auth.js";
import { encodeCode, hashHex, hmacBytes } from "./invite.js";

const HOME = "198.51.100.20";
const POLL_MS = 4000;
// Three mailbox reads, the request list, and the joining phone, every 4 seconds.
const LINK_READS = 15 * 3 + 15 + 15;

function assert(cond, message) {
  if (!cond) throw new Error(message || "assertion failed");
}

assert(LINK_READS < TEST.readAccountRate && LINK_READS < TEST.readIpRate, "test read limits have no room");
assert(LINK_READS < TEST.accountRate && LINK_READS < TEST.ipRate, "test request limits have no room");
assert(LINK_READS < PRODUCTION.readAccountRate && LINK_READS < PRODUCTION.readIpRate, "production read limits have no room");
assert(PRODUCTION.readAccountRate === 180 && PRODUCTION.readIpRate === 500, "production read limits changed");
assert(TEST.readAccountRate === 100 && TEST.readIpRate === 140, "test read limits changed");

const syncJs = await readFile(new URL("../../site/app/sync.js", import.meta.url), "utf8");
assert(syncJs.includes("const APPROVAL_POLL_MS = 4000"), "the joining phone does not poll every 4 seconds");
assert(!syncJs.includes("setTimeout(waitForApproval, 1000)"), "the joining phone still polls every second");
assert(!syncJs.includes("joinWaits || 0) < 6"), "a few waits still end the join");
const joinSrc = syncJs.slice(syncJs.indexOf("async function submitJoin("), syncJs.indexOf("function parseJoin("));
assert(joinSrc.includes("typedJoinName()") && !joinSrc.includes('name: ""'), "the typed name is not sent");

function b64(bytes) {
  let text = "";
  for (const byte of bytes) text += String.fromCharCode(byte);
  return btoa(text);
}

function deviceBytes(device) {
  const out = new Uint8Array(80);
  out.set(device.id, 0);
  out.set(device.pub, 16);
  out.set(device.receive, 48);
  return out;
}

let clock = 1_700_000_000_000;
const storage = new MemoryStorage();
const box = new Mailboxes({ storage }, { SERVE_APP: "1" });
box.clock = () => clock;

async function call(method, path, body, headers = {}) {
  const text = body === undefined ? undefined : (typeof body === "string" ? body : JSON.stringify(body));
  const response = await box.fetch(new Request("https://sync-test.omacus.com" + path, {
    method,
    headers: { "content-type": "application/json", "cf-connecting-ip": HOME, ...headers },
    body: text,
  }));
  const raw = await response.text();
  let json = {};
  if (raw) {
    try { json = JSON.parse(raw); } catch { json = { raw }; }
  }
  return { status: response.status, json, raw };
}

async function authed(device, method, path, body) {
  const text = body === undefined ? "" : JSON.stringify(body);
  const headers = await authHeaders(device, method, path, text);
  if (body && Number.isInteger(body.revision)) headers["if-match"] = `"${body.revision}"`;
  return call(method, path, body === undefined ? undefined : text, headers);
}

const account = "ab".repeat(32);
const phones = [
  await makeDevice(account),
  await makeDevice(account),
  await makeDevice(account),
];
let chain = await deviceChain(phones[0]);
chain = await appendDevice(phones[0], chain, phones[1]);
chain = await appendDevice(phones[0], chain, phones[2]);
const record = await makeRecord(phones[0], "ferry", 2, chain);
const access = {
  "x-omacus-access-hash": "cd".repeat(32),
  "x-omacus-access-uses": "1",
  "x-omacus-access-spend": "1",
};
const created = await call("POST", "/v1/mailboxes", { account, record }, access);
assert(created.status === 201, `create ${created.status} ${created.raw}`);

async function openInvite() {
  const secret = crypto.getRandomValues(new Uint8Array(5));
  const code = encodeCode(secret);
  const hash = await hashHex(secret);
  const posted = await authed(phones[0], "POST", `/v1/mailboxes/${account}/invites`, { hash });
  assert(posted.status === 200, `invite ${posted.status} ${posted.raw}`);
  return secret;
}

async function joinAs(name) {
  const secret = await openInvite();
  const joiner = await makeDevice(account);
  const rawDevice = deviceBytes(joiner);
  const proof = b64(await hmacBytes(secret, rawDevice));
  const joined = await call("POST", "/v1/join", {
    code: encodeCode(secret),
    device: b64(rawDevice),
    name,
    proof,
  });
  assert(joined.status === 201, `join ${joined.status} ${joined.raw}`);
  return { id: joined.json.id, proof };
}

async function pollJoin(id, proof) {
  return call("GET", `/v1/mailboxes/${account}/requests/${id}`, undefined, { "x-omacus-proof": proof });
}

async function listRequests() {
  return authed(phones[0], "GET", `/v1/mailboxes/${account}/requests`);
}

const chainBody = b64(new TextEncoder().encode("OMACUS-DEVICES/1ferry"));
const wrapBody = b64(new TextEncoder().encode("wrap"));

async function approve(id) {
  return authed(phones[0], "POST", `/v1/mailboxes/${account}/requests/${id}`, {
    chain: chainBody,
    wrap: wrapBody,
  });
}

// A fresh rate window, then one minute of the real cadence.
clock += 60_000;
const first = await joinAs("Harbor phone");
clock += 60_000;
let limited = 0;
for (let step = 0; step < 15; step += 1) {
  const decision = await pollJoin(first.id, first.proof);
  if (decision.status === 429) limited += 1;
  else {
    assert(decision.status === 200, `join poll ${step} ${decision.status} ${decision.raw}`);
    assert(decision.json.state === "pending" && decision.json.name === "Harbor phone", decision.raw);
  }
  for (const phone of phones) {
    const read = await authed(phone, "GET", `/v1/mailboxes/${account}`);
    if (read.status === 429) limited += 1;
    else assert(read.status === 200, `phone ${step} ${read.status} ${read.raw}`);
  }
  const listed = await listRequests();
  if (listed.status === 429) limited += 1;
  else {
    assert(listed.status === 200, `list ${step} ${listed.status} ${listed.raw}`);
    const row = (listed.json.requests || []).find((request) => request.id === first.id);
    assert(row && row.name === "Harbor phone" && row.state === "pending", listed.raw);
  }
  clock += POLL_MS;
}
assert(limited === 0, `${limited} reads were limited during the minute`);

const posted = await approve(first.id);
assert(posted.status === 200, `approve ${posted.status} ${posted.raw}`);
const decided = await pollJoin(first.id, first.proof);
assert(decided.status === 200 && decided.json.state === "approved", decided.raw);
assert(decided.json.name === "Harbor phone", `name ${decided.json.name}`);
assert(decided.json.name !== "Phone or computer", "the row fell back to Phone or computer");

// A phone that stops polling cannot be approved, and the typed name stays stored.
clock += 60_000;
const parked = await joinAs("Harbor phone");
const seen = await pollJoin(parked.id, parked.proof);
assert(seen.status === 200 && seen.json.name === "Harbor phone", seen.raw);
clock += HEARTBEAT_MS;
const hidden = await listRequests();
assert(hidden.status === 200, `hidden list ${hidden.status} ${hidden.raw}`);
assert(!(hidden.json.requests || []).some((request) => request.id === parked.id), hidden.raw);
const refused = await approve(parked.id);
assert(refused.status === 409, `abandoned approve ${refused.status} ${refused.raw}`);
assert(refused.json.message === "That phone or computer stopped waiting.", refused.raw);
const kept = await storage.get("m:" + account);
assert(kept.requests[parked.id].state === "pending", "the abandoned request was approved");
assert(kept.requests[parked.id].name === "Harbor phone", "the typed name was dropped");

// Fill the read cap, then a later poll still reaches the approval.
clock += 60_000;
const retry = await joinAs("Harbor phone");
let allowed = 0;
let capped = null;
for (let n = 0; n < TEST.readAccountRate + 3; n += 1) {
  const got = await pollJoin(retry.id, retry.proof);
  if (got.status === 429) {
    capped = got;
    break;
  }
  assert(got.status === 200 && got.json.state === "pending" && got.json.name === "Harbor phone", got.raw);
  allowed += 1;
}
assert(allowed === TEST.readAccountRate, `reads before the cap ${allowed}`);
assert(capped && capped.json.message === "Try again in a moment.", capped && capped.raw);
clock += 60_000;
const quiet = await listRequests();
assert(quiet.status === 200 && !(quiet.json.requests || []).some((request) => request.id === retry.id), quiet.raw);
const again = await pollJoin(retry.id, retry.proof);
assert(again.status === 200 && again.json.state === "pending" && again.json.name === "Harbor phone", again.raw);
const visible = await listRequests();
const asking = (visible.json.requests || []).find((request) => request.id === retry.id);
assert(asking && asking.name === "Harbor phone", visible.raw);
const accepted = await approve(retry.id);
assert(accepted.status === 200, `retry approve ${accepted.status} ${accepted.raw}`);
const finished = await pollJoin(retry.id, retry.proof);
assert(finished.status === 200 && finished.json.state === "approved" && finished.json.name === "Harbor phone", finished.raw);

console.log("linking under the test limits ok");
