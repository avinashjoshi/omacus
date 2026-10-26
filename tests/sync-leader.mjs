// Two tabs share one lock. A second poll skips. Approve and remove wait their turn.
import assert from "node:assert/strict";
import { ifTabLeader, tabLockName, withTabLock } from "../site/app/sync-leader.mjs";

function fakeLocks() {
  let held = false;
  const queue = [];
  function release() {
    held = false;
    const next = queue.shift();
    if (!next) return;
    held = true;
    Promise.resolve()
      .then(() => next.fn({ name: next.name }))
      .then(next.resolve, next.reject)
      .finally(release);
  }
  return {
    request(name, options, callback) {
      const opts = typeof options === "function" ? {} : options || {};
      const fn = typeof options === "function" ? options : callback;
      if (held && opts.ifAvailable) return Promise.resolve(fn(null));
      if (held) {
        return new Promise((resolve, reject) => {
          queue.push({ name, fn, resolve, reject });
        });
      }
      held = true;
      return Promise.resolve()
        .then(() => fn({ name }))
        .finally(release);
    },
  };
}

assert.equal(tabLockName("relay.example", ""), "omacus");
assert.equal(tabLockName("app.omacus.com", "1"), "omacus-sync-test");
assert.equal(tabLockName("app.omacus.com", ""), "omacus");

const locks = fakeLocks();
let active = 0;
let max = 0;
let release;
const gate = new Promise((done) => {
  release = done;
});
const first = withTabLock(locks, "omacus", async () => {
  active += 1;
  max = Math.max(max, active);
  await gate;
  active -= 1;
  return "first";
});
const second = withTabLock(locks, "omacus", async () => {
  active += 1;
  max = Math.max(max, active);
  active -= 1;
  return "second";
});
const skipped = ifTabLeader(locks, "omacus", async () => "leader");
assert.equal(await skipped, false);
release();
assert.equal(await first, "first");
assert.equal(await second, "second");
assert.equal(max, 1);

let started = 0;
let releaseA;
const gateA = new Promise((done) => {
  releaseA = done;
});
const fresh = fakeLocks();
const leader = ifTabLeader(fresh, "omacus-sync-test", async () => {
  started += 1;
  await gateA;
  return "a";
});
const follower = ifTabLeader(fresh, "omacus-sync-test", async () => {
  started += 1;
  return "b";
});
assert.equal(await follower, false);
assert.equal(started, 1);
releaseA();
assert.equal(await leader, "a");

assert.equal(await ifTabLeader(null, "omacus", async () => "fallback"), "fallback");
assert.equal(await withTabLock(undefined, "omacus", async () => "plain"), "plain");

console.log("sync-leader ok");
