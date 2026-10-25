// Linking under the real test limits. The clock is the worker's rate clock.
// No x-omacus-limits header. Three phones save, one joins, all on one address.
import { readFile } from "node:fs/promises";
import { Mailboxes } from "./worker.js";
import { HEARTBEAT_MS } from "./heartbeat.js";
import { MemoryStorage } from "./storage.js";
import { PRODUCTION, TEST } from "./limits.js";
import { appendDevice, authHeaders, deviceChain, dropDevice, makeDevice, makeRecord } from "./auth.js";
import { encodeCode, hashHex, hmacBytes } from "./invite.js";

const HOME = "198.51.100.20";
const POLL_MS = 4000;
// A live invite checks the request list about every 2 seconds (30 a minute),
// and the joining phone does the same. Quiet mailbox reads stay on the slow poll.
const LINK_READS = 30 + 30 + 15;

function assert(cond, message) {
  if (!cond) throw new Error(message || "assertion failed");
}

assert(LINK_READS < TEST.readAccountRate && LINK_READS < TEST.readIpRate, "test read limits have no room");
assert(LINK_READS < TEST.accountRate && LINK_READS < TEST.ipRate, "test request limits have no room");
assert(LINK_READS < PRODUCTION.readAccountRate && LINK_READS < PRODUCTION.readIpRate, "production read limits have no room");
assert(PRODUCTION.readAccountRate === 180 && PRODUCTION.readIpRate === 500, "production read limits changed");
assert(TEST.readAccountRate === 100 && TEST.readIpRate === 140, "test read limits changed");

const syncJs = await readFile(new URL("../../site/app/sync.js", import.meta.url), "utf8");
const workerSrc = await readFile(new URL("./worker.js", import.meta.url), "utf8");
assert(syncJs.includes("const APPROVAL_POLL_MS = 4000"), "the approve screen still waits 4 seconds between checks");
assert(syncJs.includes("const JOIN_WAIT_MS = 2000"), "the joining phone does not check every 2 seconds");
assert(syncJs.includes("const INVITE_POLL_MS = 2000"), "an invite does not look for a join every 2 seconds");
assert(!syncJs.includes("setTimeout(waitForApproval, 1000)"), "the joining phone still polls every second");
assert(syncJs.includes("setTimeout(waitForApproval, JOIN_WAIT_MS)"), "the joining phone still waits 4 seconds");
assert(!syncJs.includes("joinWaits || 0) < 6"), "a few waits still end the join");
const joinSrc = syncJs.slice(syncJs.indexOf("async function submitJoin("), syncJs.indexOf("function parseJoin("));
assert(joinSrc.includes("typedJoinName()") && !joinSrc.includes('name: ""'), "the typed name is not sent");
const waitSrc = syncJs.slice(syncJs.indexOf("async function waitForApproval("), syncJs.indexOf("async function acceptJoin("));
assert(waitSrc.includes("MAILBOX_TRIES") && waitSrc.includes("error.status === 404"), "a missing list ends the join");
assert(waitSrc.includes("done: true") && waitSrc.indexOf('state.screen = "join-confirm"') > waitSrc.indexOf("done: true"), "the match screen comes before the list is ready");
assert(syncJs.includes('["Do these match?"]'), "the match screen is missing");
const approveSrc = syncJs.slice(syncJs.indexOf("async function approvePending("), syncJs.indexOf("async function decline("));
assert(approveSrc.includes("record: sealed.record") && approveSrc.includes("releaseJoin("), "approval does not publish the list");
const watchSrc = syncJs.slice(syncJs.indexOf("function watchApproval("), syncJs.indexOf("function draw("));
assert(watchSrc.includes('state.screen = "linked"'), "the approve prompt stays after the phone stops waiting");
assert(!watchSrc.includes("state.invite = null"), "a phone that stops waiting takes the code with it");
assert(!workerSrc.includes("export const HEARTBEAT_MS"), "the worker exports a plain number");

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
  return { id: joined.json.id, proof, secret, joiner, rawDevice };
}

async function pollJoin(id, proof) {
  return call("GET", `/v1/mailboxes/${account}/requests/${id}`, undefined, { "x-omacus-proof": proof });
}

async function listRequests() {
  return authed(phones[0], "GET", `/v1/mailboxes/${account}/requests`);
}

const wrapBody = b64(new TextEncoder().encode("wrap"));
let liveChain = chain;
let revision = 1;

async function approve(joined) {
  const nextChain = await appendDevice(phones[0], liveChain, joined.joiner);
  assert(nextChain, "the list could not add that phone");
  const record = await makeRecord(phones[0], "ferry", 2, nextChain);
  const posted = await authed(phones[0], "POST", `/v1/mailboxes/${account}/requests/${joined.id}`, {
    chain: b64(nextChain),
    wrap: wrapBody,
    record,
    revision,
  });
  if (posted.status === 200) {
    liveChain = nextChain;
    revision = posted.json.revision;
  }
  return posted;
}

// The test relay allows four phones. A finished join is taken off the list
// so the next one still fits.
async function removeJoined(joiner) {
  const without = await dropDevice(phones[0], liveChain, joiner.id);
  assert(without, "the list could not drop that phone");
  const rolled = await makeRecord(phones[0], "ferry", 2, without);
  const removal = await authed(phones[0], "PUT", `/v1/mailboxes/${account}`, { revision, record: rolled });
  assert(removal.status === 200, `removal ${removal.status} ${removal.raw}`);
  liveChain = without;
  revision = removal.json.revision;
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

const posted = await approve(first);
assert(posted.status === 200, `approve ${posted.status} ${posted.raw}`);
const decided = await pollJoin(first.id, first.proof);
assert(decided.status === 200 && decided.json.state === "approved", decided.raw);
assert(decided.json.name === "Harbor phone", `name ${decided.json.name}`);
assert(decided.json.name !== "Phone or computer", "the row fell back to Phone or computer");
const opened = await authed(first.joiner, "GET", `/v1/mailboxes/${account}`);
assert(opened.status === 200, `joiner read ${opened.status} ${opened.raw}`);
const retryRead = await pollJoin(first.id, first.proof);
assert(retryRead.status === 200 && retryRead.json.state === "approved", retryRead.raw);
const done = await authed(first.joiner, "POST", `/v1/mailboxes/${account}/requests/${first.id}`, { done: true });
assert(done.status === 200, `done ${done.status} ${done.raw}`);
const gone = await pollJoin(first.id, first.proof);
assert(gone.status === 404, `approval stayed ${gone.status} ${gone.raw}`);
const saved = await makeRecord(first.joiner, "bread", 2, liveChain);
const put = await authed(first.joiner, "PUT", `/v1/mailboxes/${account}`, { revision, record: saved });
assert(put.status === 200, `first save ${put.status} ${put.raw}`);
revision = put.json.revision;
const arrived = await authed(phones[0], "GET", `/v1/mailboxes/${account}`);
assert(arrived.status === 200, `approver read ${arrived.status} ${arrived.raw}`);
assert(arrived.json.record === saved, "the new phone's first save did not arrive");
assert(arrived.json.revision === revision, `approver revision ${arrived.json.revision}`);
await removeJoined(first.joiner);

// A phone that stops polling cannot be approved, and the typed name stays stored.
clock += 60_000;
const parked = await joinAs("Harbor phone");
const seen = await pollJoin(parked.id, parked.proof);
assert(seen.status === 200 && seen.json.name === "Harbor phone", seen.raw);
clock += HEARTBEAT_MS;
const hidden = await listRequests();
assert(hidden.status === 200, `hidden list ${hidden.status} ${hidden.raw}`);
assert(!(hidden.json.requests || []).some((request) => request.id === parked.id), hidden.raw);
const refused = await approve(parked);
assert(refused.status === 409, `abandoned approve ${refused.status} ${refused.raw}`);
assert(refused.json.message === "That phone or computer stopped waiting.", refused.raw);
const kept = await storage.get("m:" + account);
assert(kept.requests[parked.id].state === "pending", "the abandoned request was approved");
assert(kept.requests[parked.id].name === "Harbor phone", "the typed name was dropped");
const reused = await call("POST", "/v1/join", {
  code: encodeCode(parked.secret),
  device: b64(parked.rawDevice),
  name: "Harbor phone",
  proof: parked.proof,
});
assert(reused.status === 201, `abandoned code ${reused.status} ${reused.raw}`);
assert(reused.json.id !== parked.id, "the abandoned request was reused");

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
const accepted = await approve(retry);
assert(accepted.status === 200, `retry approve ${accepted.status} ${accepted.raw}`);
const finished = await pollJoin(retry.id, retry.proof);
assert(finished.status === 200 && finished.json.state === "approved" && finished.json.name === "Harbor phone", finished.raw);
const retryOpened = await authed(retry.joiner, "GET", `/v1/mailboxes/${account}`);
assert(retryOpened.status === 200, `retry read ${retryOpened.status} ${retryOpened.raw}`);
await removeJoined(retry.joiner);

// The joining phone never says it finished. The row comes off and the code works again.
clock += 60_000;
const left = await joinAs("Ferry phone");
const waiting = await pollJoin(left.id, left.proof);
assert(waiting.status === 200 && waiting.json.name === "Ferry phone", waiting.raw);
const added = await approve(left);
assert(added.status === 200, `ferry approve ${added.status} ${added.raw}`);
const early = await authed(left.joiner, "GET", `/v1/mailboxes/${account}`);
assert(early.status === 200, `ferry read ${early.status} ${early.raw}`);
const released = await authed(phones[0], "POST", `/v1/mailboxes/${account}/requests/${left.id}`, { release: true });
assert(released.status === 200, `release ${released.status} ${released.raw}`);
await removeJoined(left.joiner);
const denied = await authed(left.joiner, "GET", `/v1/mailboxes/${account}`);
assert(denied.status === 404, `removed phone still read ${denied.status} ${denied.raw}`);
const rejoined = await call("POST", "/v1/join", {
  code: encodeCode(left.secret),
  device: b64(left.rawDevice),
  name: "Ferry phone",
  proof: left.proof,
});
assert(rejoined.status === 201, `same code ${rejoined.status} ${rejoined.raw}`);

console.log("linking under the test limits ok");
