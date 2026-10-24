// Per-page objects: compare-and-swap, and a file store that is not a Durable Object.
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Mailboxes } from "./worker.js";
import { FileStorage } from "./storage.js";

const account = "cd".repeat(32);
const object = "0123456789abcdef0123456789abcdef";
const pageId = "page-lentils";

function assert(cond, message) {
  if (!cond) throw new Error(message);
}

function b64(text) {
  return btoa(text);
}

async function call(box, method, path, body, headers = {}) {
  const response = await box.fetch(new Request("https://sync-test.omacus.com" + path, {
    method,
    headers: { "content-type": "application/json", ...headers },
    body: body === undefined ? undefined : JSON.stringify(body),
  }));
  const json = await response.json();
  return { status: response.status, json };
}

const access = {
  "x-omacus-access-hash": "ab".repeat(32),
  "x-omacus-access-uses": "2",
  "x-omacus-access-spend": "1",
};

const record = b64("OMACUS-SYNC/2\nmanifest");
const body = b64(String.fromCharCode(...new Uint8Array(64).fill(7)));
const dir = mkdtempSync(join(tmpdir(), "omacus-objects-"));

try {
  const first = new Mailboxes({ storage: new FileStorage(dir) }, {});
  const created = await call(first, "POST", "/v1/mailboxes", {
    account,
    record,
    objects: [{ id: object, base: 0, body }],
  }, access);
  assert(created.status === 201, `create ${created.status} ${JSON.stringify(created.json)}`);
  assert(created.json.revision === 1, "revision");
  assert(created.json.objects[0].revision === 1, "object revision");
  assert(!JSON.stringify(created.json).includes(pageId), "create response names a page");

  const stale = await call(first, "PUT", `/v1/mailboxes/${account}`, {
    revision: 1,
    record,
    objects: [{ id: object, base: 0, body }],
  });
  assert(stale.status === 409, `stale base ${stale.status}`);

  const got = await call(first, "GET", `/v1/mailboxes/${account}`);
  assert(got.status === 200, "get");
  assert(got.json.objects.length === 1, "index");
  assert(got.json.objects[0].id === object, "object id");
  assert(got.json.objects[0].bytes === 64, "stored length");
  assert(!JSON.stringify(got.json).includes(body), "index hides the body");

  const read = await call(first, "POST", `/v1/mailboxes/${account}/objects`, { ids: [object] });
  assert(read.status === 200 && read.json.objects[0].body === body, "body");

  const other = "fedcba9876543210fedcba9876543210";
  const mixed = await call(first, "PUT", `/v1/mailboxes/${account}`, {
    revision: 1,
    record: b64("OMACUS-SYNC/2\nnext"),
    objects: [
      { id: object, base: 1, body },
      { id: other, base: 4, body },
    ],
  });
  assert(mixed.status === 409, "one stale object rolls the whole save back");
  const still = await call(first, "GET", `/v1/mailboxes/${account}`);
  assert(still.json.revision === 1, "revision stayed");
  assert(still.json.objects.length === 1, "the new object was not stored");

  const second = new Mailboxes({ storage: new FileStorage(dir) }, {});
  const again = await call(second, "POST", `/v1/mailboxes/${account}/objects`, { ids: [object] });
  assert(again.status === 200 && again.json.objects[0].body === body, "reloaded file store");

  const names = readdirSync(join(dir, "obj", account)).map((name) => name.replace(/\.json$/, ""));
  assert(names.length === 1 && names[0] === object, `file name ${names.join(",")}`);
  assert(!names.some((name) => name.includes("lentils") || name.includes("page")), "file name is not a page id");
  const raw = readFileSync(join(dir, "obj", account, `${object}.json`), "utf8");
  assert(!raw.includes(pageId) && !raw.includes("lentils"), "file bytes name a page");

  const v1 = await call(second, "PUT", `/v1/mailboxes/${account}`, {
    revision: 1,
    record: b64("OMACUS-SYNC/1\nstill-one-record"),
  });
  assert(v1.status === 200 && v1.json.revision === 2, "a version 1 record still saves");
  console.log("objects ok");
} finally {
  rmSync(dir, { recursive: true, force: true });
}
