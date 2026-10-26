// One open tab holds the lock. The others skip the poll.
// Approve and remove take the same lock and wait, so they run once.

export function tabLockName(hostname, syncTestFlag) {
  if (hostname === "sync-test.omacus.com" || syncTestFlag === "1" || syncTestFlag === true) {
    return "omacus-sync-test";
  }
  return "omacus";
}

export async function ifTabLeader(locks, name, fn) {
  if (!locks || typeof locks.request !== "function") return fn();
  return locks.request(name, { mode: "exclusive", ifAvailable: true }, (lock) => {
    if (!lock) return false;
    return fn();
  });
}

export async function withTabLock(locks, name, fn) {
  if (!locks || typeof locks.request !== "function") return fn();
  return locks.request(name, { mode: "exclusive" }, () => fn());
}
