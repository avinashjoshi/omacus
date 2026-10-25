// Drive the Worker mailbox the way a beta.1 device does after it is upgraded
// in place: the saved device file has no invite, and a slow approve overlaps
// the save of the sealed pages.
import { readFile } from "node:fs/promises";
import { Mailboxes } from "./worker.js";
import init, {
  startNew,
  confirmSaved,
  exportLocal,
  importLocal,
  startJoin,
  approveDevice,
  sealPages,
  openApprovalPages,
  confirmLink,
  makeInvite,
  joinProof,
  packPages,
  deviceList,
  removeDevice,
  signRequest,
} from "./public/pkg/omacus_sync_web.js";

const wasm = await readFile(new URL("./public/pkg/omacus_sync_web_bg.wasm", import.meta.url));
await init(wasm);

function parse(raw) {
  const value = JSON.parse(raw);
  if (!value.ok) throw new Error(value.message || "not ok");
  return value;
}

class Storage {
  constructor() {
    this.rows = new Map();
    this.delay = 0;
    this.overlap = false;
    this.pending = [];
    this.chain = Promise.resolve();
  }
  async transaction(fn) {
    const run = this.chain.then(async () => {
      const snapshot = structuredClone(this.rows);
      const txn = {
        get: async (key) => {
          const value = this.rows.get(key);
          return value === undefined ? undefined : structuredClone(value);
        },
        put: async (key, value) => {
          this.rows.set(key, structuredClone(value));
        },
        delete: async (key) => {
          this.rows.delete(key);
        },
        list: async ({ prefix } = {}) => {
          const out = new Map();
          for (const [key, value] of this.rows) {
            if (!prefix || key.startsWith(prefix)) out.set(key, structuredClone(value));
          }
          return out;
        },
      };
      try {
        await fn(txn);
      } catch (error) {
        this.rows = snapshot;
        throw error;
      }
    });
    this.chain = run.then(() => {}, () => {});
    return run;
  }
  async get(key) {
    await this.pause();
    const value = this.rows.get(key);
    return value === undefined ? undefined : structuredClone(value);
  }
  async put(key, value) {
    await this.pause();
    if (this.overlap) {
      await new Promise((resolve) => this.pending.push(resolve));
    }
    this.rows.set(key, structuredClone(value));
  }
  async list({ prefix } = {}) {
    await this.pause();
    const out = new Map();
    for (const [key, value] of this.rows) {
      if (!prefix || key.startsWith(prefix)) out.set(key, structuredClone(value));
    }
    return out;
  }
  async pause() {
    if (this.delay) await new Promise((resolve) => setTimeout(resolve, this.delay));
  }
  release() {
    const waiting = this.pending.splice(0);
    for (const resolve of waiting) resolve();
  }
}

function mailbox(storage) {
  return new Mailboxes({ storage }, { ACCESS_SECRET: "omacus-access-test-secret" });
}

async function call(box, method, path, body, headers = {}) {
  const text = body === undefined || body === null
    ? undefined
    : (typeof body === "string" ? body : JSON.stringify(body));
  const request = new Request("https://sync.test" + path, {
    method,
    headers: { "content-type": "application/json", ...headers },
    body: text,
  });
  const response = await box.fetch(request);
  const raw = await response.text();
  let json = {};
  try { json = JSON.parse(raw); } catch { json = { raw }; }
  return { status: response.status, json };
}

async function authed(box, deskId, method, path, body) {
  const text = body === undefined ? "" : JSON.stringify(body);
  const auth = parse(signRequest(deskId, method, path, text));
  const headers = {
    "x-omacus-device": auth.device,
    "x-omacus-time": auth.time,
    "x-omacus-nonce": auth.nonce,
    "x-omacus-signature": auth.signature,
  };
  if (body && Number.isInteger(body.revision)) headers["if-match"] = `"${body.revision}"`;
  return call(box, method, path, body === undefined ? undefined : text, headers);
}

const started = parse(startNew("https://sync-test.omacus.com"));
const words = started.words;
const picks = [[3, words[2]], [7, words[6]], [11, words[10]]];
const saved = parse(confirmSaved(started.id, JSON.stringify(picks)));
const record = saved.record;
const account = saved.account;
const file = parse(exportLocal(started.id)).file;
const upgraded = parse(importLocal(file));
if (upgraded.account !== account) throw new Error("upgraded account changed");

const storage = new Storage();
const box = mailbox(storage);

// A beta.1 mailbox: create stores revision, record, requests, and created. No invite.
const created = await call(box, "POST", "/v1/mailboxes", { account, record }, {
  "x-omacus-access-hash": "ab".repeat(32),
  "x-omacus-access-uses": "1",
  "x-omacus-access-spend": "1",
});
if (created.status !== 201) throw new Error("create " + JSON.stringify(created));
const stored = await storage.get("m:" + account);
if (stored.invite) throw new Error("beta.1 mailbox should not have an invite yet");
if (!stored.requests || Object.keys(stored.requests).length !== 0) throw new Error("requests missing");

const secret = Buffer.from([1, 2, 3, 4, 5]).toString("base64");
const invite = parse(makeInvite(account, "https://sync-test.omacus.com", "https://sync-test.omacus.com", secret));
const posted = await authed(box, upgraded.id, "POST", `/v1/mailboxes/${account}/invites`, { hash: invite.hash });
if (posted.status !== 200) throw new Error("invite " + JSON.stringify(posted));

const joiner = parse(startJoin());
const proof = parse(joinProof(joiner.device, invite.code));
const joined = await call(box, "POST", "/v1/join", {
  code: invite.code,
  device: joiner.device,
  name: "Kitchen phone",
  proof: proof.proof,
});
if (joined.status !== 201) throw new Error("join " + JSON.stringify(joined));
if (joined.json.account !== account) {
  throw new Error(`join account ${joined.json.account} != approver ${account}`);
}

const pending = await authed(box, upgraded.id, "GET", `/v1/mailboxes/${account}/requests`);
const approval = parse(approveDevice(upgraded.id, pending.json.requests[0].device));
const packed = parse(packPages("", JSON.stringify([["trip", "Trip Budget\nferry = 40\n"]]), ""));
const sealed = parse(sealPages(upgraded.id, packed.packed));

// The approval stores the list. A save of the old revision cannot replace it.
const post = await authed(box, upgraded.id, "POST", `/v1/mailboxes/${account}/requests/${joined.json.id}`, {
  chain: approval.chain,
  wrap: approval.wrap,
  record: sealed.record,
  revision: 1,
});
const stale = await authed(box, upgraded.id, "PUT", `/v1/mailboxes/${account}`, { revision: 1, record: sealed.record });
console.log("POST", post.status, post.json);
console.log("stale PUT", stale.status, stale.json);
if (post.status !== 200) throw new Error("approve " + JSON.stringify(post));
if (stale.status !== 409) throw new Error("stale save replaced the list " + JSON.stringify(stale));
const early = await authed(box, joiner.id, "GET", `/v1/mailboxes/${account}`);
if (early.status !== 200) throw new Error("joiner could not read " + JSON.stringify(early));

const decision = await call(box, "GET", `/v1/mailboxes/${account}/requests/${joined.json.id}`, undefined, {
  "x-omacus-proof": proof.proof,
});
const again = await call(box, "GET", `/v1/mailboxes/${account}/requests/${joined.json.id}`, undefined, {
  "x-omacus-proof": proof.proof,
});
if (again.status !== 200) throw new Error("approval could not be read again " + JSON.stringify(again));
const done = await authed(box, joiner.id, "POST", `/v1/mailboxes/${account}/requests/${joined.json.id}`, { done: true });
if (done.status !== 200) throw new Error("done " + JSON.stringify(done));
const gone = await call(box, "GET", `/v1/mailboxes/${account}/requests/${joined.json.id}`);
if (gone.status !== 404) throw new Error("approved request stayed " + JSON.stringify(gone));
const after = await storage.get("m:" + account);
if (after.requests[joined.json.id]) throw new Error("approved request still stored");
const mailboxBody = await authed(box, upgraded.id, "GET", `/v1/mailboxes/${account}`);
console.log("decision", decision.json.state, "chain", (decision.json.chain || "").slice(0, 24), "recordRev", mailboxBody.json.revision);
if (mailboxBody.json.revision !== 2) {
  console.error("saved revision rolled back to " + mailboxBody.json.revision);
  process.exitCode = 1;
}
if (mailboxBody.json.record !== sealed.record) {
  console.error("saved pages were rolled back");
  process.exitCode = 1;
}
let opened;
try {
  parse(confirmLink(joiner.id));
  opened = JSON.parse(openApprovalPages(joiner.id, decision.json.chain, decision.json.wrap, mailboxBody.json.record));
} catch (error) {
  opened = { ok: false, message: error.message };
}
console.log("OPEN", opened.ok ? "ok" : opened.message);
if (!opened.ok) process.exitCode = 1;

const linked = await authed(box, joiner.id, "GET", `/v1/mailboxes/${account}`);
if (linked.status !== 200) {
  console.error("linked phone could not read " + JSON.stringify(linked));
  process.exitCode = 1;
}
const joinerRow = JSON.parse(deviceList(upgraded.id)).devices.find((device) => !device.self);
const dropped = parse(removeDevice(upgraded.id, joinerRow.id));
const droppedPut = await authed(box, upgraded.id, "PUT", `/v1/mailboxes/${account}`, {
  revision: mailboxBody.json.revision,
  record: dropped.record,
});
if (droppedPut.status !== 200) {
  console.error("remove put " + JSON.stringify(droppedPut));
  process.exitCode = 1;
}
const denied = await authed(box, joiner.id, "GET", `/v1/mailboxes/${account}`);
if (denied.status !== 404) {
  console.error("removed phone still read " + JSON.stringify(denied));
  process.exitCode = 1;
}

// Remove the phone that just joined, upgrade the computer in place, add another.
const phone = parse(startJoin());
parse(approveDevice(upgraded.id, phone.device));
const listed = JSON.parse(deviceList(upgraded.id));
const phoneId = listed.devices.find((device) => !device.self).id;
const removed = JSON.parse(removeDevice(upgraded.id, phoneId));
console.log("remove", removed.ok ? "ok" : removed.message, "devices", JSON.parse(deviceList(upgraded.id)).devices?.length);
const removedFile = parse(exportLocal(upgraded.id)).file;
const computer = parse(importLocal(removedFile));
const third = parse(startJoin());
const thirdApproval = JSON.parse(approveDevice(computer.id, third.device));
console.log("approve after removal", thirdApproval.ok ? "ok" : thirdApproval.message);
if (thirdApproval.ok) {
  const thirdSealed = parse(sealPages(computer.id, packed.packed));
  // A phone added after a removal still has no list. The first list is stored
  // only after the six numbers are checked, same as the first link.
  parse(confirmLink(third.id));
  const thirdOpen = JSON.parse(openApprovalPages(third.id, thirdApproval.chain, thirdApproval.wrap, thirdSealed.record));
  console.log("open after removal", thirdOpen.ok ? "ok" : thirdOpen.message);
  const oldOpen = JSON.parse(openApprovalPages(third.id, thirdApproval.chain, thirdApproval.wrap, record));
  console.log("open old record after removal", oldOpen.ok ? "ok" : oldOpen.message);
  if (!thirdOpen.ok) process.exitCode = 1;
  if (oldOpen.ok) {
    console.error("a copy from before removal opened with the new seal");
    process.exitCode = 1;
  }
  if ((oldOpen.message || "").includes("six numbers")) {
    console.error("the copy from before removal failed the fingerprint check");
    process.exitCode = 1;
  }
} else process.exitCode = 1;
