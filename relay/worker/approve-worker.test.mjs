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
  makeInvite,
  joinProof,
  packPages,
  deviceList,
  removeDevice,
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
  const request = new Request("https://sync.test" + path, {
    method,
    headers: { "content-type": "application/json", ...headers },
    body: body ? JSON.stringify(body) : undefined,
  });
  const response = await box.fetch(request);
  const text = await response.text();
  let json = {};
  try { json = JSON.parse(text); } catch { json = { raw: text }; }
  return { status: response.status, json };
}

const started = parse(startNew());
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
const invite = parse(makeInvite(account, "https://sync-test.omacus.com", secret));
const posted = await call(box, "POST", `/v1/mailboxes/${account}/invites`, { hash: invite.hash });
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

const pending = await call(box, "GET", `/v1/mailboxes/${account}/requests`);
const approval = parse(approveDevice(upgraded.id, pending.json.requests[0].device));
const packed = parse(packPages("", JSON.stringify([["trip", "Trip Budget\nferry = 40\n"]]), ""));
const sealed = parse(sealPages(upgraded.id, packed.packed));

// Overlap the page save and the approval the way a slow phone does.
storage.overlap = true;
const putPromise = call(box, "PUT", `/v1/mailboxes/${account}`, { revision: 1, record: sealed.record });
const postPromise = call(box, "POST", `/v1/mailboxes/${account}/requests/${joined.json.id}`, {
  chain: approval.chain,
  wrap: approval.wrap,
});
await new Promise((resolve) => setTimeout(resolve, 20));
storage.release();
const put = await putPromise;
const post = await postPromise;
storage.overlap = false;
console.log("PUT", put.status, put.json);
console.log("POST", post.status, post.json);

const decision = await call(box, "GET", `/v1/mailboxes/${account}/requests/${joined.json.id}`);
const again = await call(box, "GET", `/v1/mailboxes/${account}/requests/${joined.json.id}`);
if (again.status !== 404) throw new Error("approved request stayed " + JSON.stringify(again));
const after = await storage.get("m:" + account);
if (after.requests[joined.json.id]) throw new Error("approved request still stored");
const mailboxBody = await call(box, "GET", `/v1/mailboxes/${account}`);
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
  opened = JSON.parse(openApprovalPages(joiner.id, decision.json.chain, decision.json.wrap, mailboxBody.json.record));
} catch (error) {
  opened = { ok: false, message: error.message };
}
console.log("OPEN", opened.ok ? "ok" : opened.message);
if (!opened.ok) process.exitCode = 1;

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
  const thirdOpen = JSON.parse(openApprovalPages(third.id, thirdApproval.chain, thirdApproval.wrap, thirdSealed.record));
  console.log("open after removal", thirdOpen.ok ? "ok" : thirdOpen.message);
  const oldOpen = JSON.parse(openApprovalPages(third.id, thirdApproval.chain, thirdApproval.wrap, record));
  console.log("open old record after removal", oldOpen.ok ? "ok" : oldOpen.message);
  if (!thirdOpen.ok) process.exitCode = 1;
  if (oldOpen.ok) {
    console.error("a copy from before removal opened with the new seal");
    process.exitCode = 1;
  }
} else process.exitCode = 1;
