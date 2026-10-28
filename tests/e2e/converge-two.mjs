// Two desktop clients, one mailbox, on the real worker.
// The first library has a folder and a Recently deleted page.
// The second sends a flat list, a page of its own, and a different deleted page.
// Both must end on the same sidebar, the same order, and both deleted pages.
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { randomBytes } from "node:crypto";
import { mkdirSync, writeFileSync, rmSync } from "node:fs";

const BOX = "http://127.0.0.1:8798";
const BIN = process.env.OMACUS_BIN || "/tmp/omacus-converge-target/debug/omacus";
const ROOT = "/tmp/omacus-converge-two";

function secretFile(path, text) {
  writeFileSync(path, text.endsWith("\n") ? text : `${text}\n`, { mode: 0o600 });
}

function startBox() {
  const child = spawn(process.execPath, ["--import", "./relay/worker/register-cf.mjs", "tests/e2e/relay-box.mjs"], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  const ready = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("sync box did not start")), 8000);
    child.stdout.on("data", (buf) => {
      if (String(buf).includes("ready")) {
        clearTimeout(timer);
        resolve();
      }
    });
    child.once("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`sync box exited ${code}`));
    });
  });
  return { child, ready };
}

function startDriver(dir, passFile, accessFile, inviteFile, joinFile) {
  const child = spawn(BIN, [], {
    env: {
      ...process.env,
      OMACUS_E2E_SYNC: "1",
      OMACUS_E2E_DIR: dir,
      OMACUS_DATA_DIR: dir,
      OMACUS_SYNC_URL: BOX,
      OMACUS_E2E_PASS_FILE: passFile,
      OMACUS_E2E_ACCESS_FILE: accessFile,
      OMACUS_E2E_INVITE_FILE: inviteFile,
      OMACUS_E2E_JOIN_FILE: joinFile,
      OMACUS_NO_UPDATE_CHECK: "1",
      RUST_LOG: "",
    },
    stdio: ["pipe", "pipe", "inherit"],
  });
  const pending = [];
  const waiters = [];
  const rl = createInterface({ input: child.stdout });
  rl.on("line", (line) => {
    if (!line.startsWith("{")) return;
    if (waiters.length) waiters.shift()(line);
    else pending.push(line);
  });
  return {
    child,
    send(cmd, ms = 60000) {
      child.stdin.write(`${cmd}\n`);
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error(`driver timed out on ${cmd.split(" ")[0]}`)), ms);
        const take = (line) => {
          clearTimeout(timer);
          const msg = JSON.parse(line);
          if (!msg.ok) reject(new Error(`${msg.cmd || cmd} ${msg.error || "sync failed"}`));
          else resolve(msg);
        };
        if (pending.length) take(pending.shift());
        else waiters.push(take);
      });
    },
  };
}

function shape(present) {
  const sidebar = present?.sidebar || [];
  const folder = sidebar.find((node) => node && node.type === "folder" && node.id === "pantry");
  const roots = sidebar.map((node) => `${node.type}:${node.id}`);
  const inside = (folder?.children || []).map((node) => node.id);
  const trash = (present?.trash || []).map((row) => row.id).sort();
  return { roots, inside, trash };
}

rmSync(ROOT, { recursive: true, force: true });
mkdirSync(ROOT, { recursive: true });
const box = startBox();
const children = [box.child];
let failed = 0;
try {
  await box.ready;
  const minted = await fetch(BOX + "/v1/admin/codes", {
    method: "POST",
    headers: {
      authorization: "Bearer admin-test-key-value",
      "content-type": "application/json",
    },
    body: JSON.stringify({ uses: 8, label: "Harbor", expires: "2099-01-01" }),
  });
  if (!minted.ok) throw new Error(`access code ${minted.status}`);
  const access = (await minted.json()).code;
  if (!access) throw new Error("access code missing");
  secretFile(`${ROOT}/access`, access);
  secretFile(`${ROOT}/desk-pass`, randomBytes(18).toString("hex"));
  secretFile(`${ROOT}/phone-pass`, randomBytes(18).toString("hex"));
  const desk = startDriver(`${ROOT}/desk`, `${ROOT}/desk-pass`, `${ROOT}/access`, `${ROOT}/invite`, `${ROOT}/invite`);
  const phone = startDriver(`${ROOT}/phone`, `${ROOT}/phone-pass`, `${ROOT}/access`, `${ROOT}/invite`, `${ROOT}/invite`);
  children.push(desk.child, phone.child);
  const started = await desk.send("setup");
  if (started.detail?.chip !== "Live") throw new Error(`desk chip ${started.detail?.chip || "missing"}`);
  await desk.send("invite");
  await phone.send("join");
  await desk.send("approve", 40000);
  await phone.send("accept", 40000);
  writeFileSync(
    `${ROOT}/flat.json`,
    JSON.stringify({
      pages: [{ id: "loaf", text: "# Loaf\nbowl 2\n" }],
      structural: {
        sidebar: [
          { id: "cake", kind: "page", name: "", parent: "", index: 0 },
          { id: "bread", kind: "page", name: "", parent: "", index: 1 },
          { id: "loaf", kind: "page", name: "", parent: "", index: 2 },
          { id: "ferry", kind: "page", name: "", parent: "", index: 3 },
          { id: "oats", kind: "page", name: "", parent: "", index: 4 },
        ],
        handles: [{ id: "ferry", handle: "pier" }],
        trash: [{ id: "tin", text: "# Tin\nold loaf\n", handle: "" }],
      },
    }),
  );
  const pushed = await phone.send(`overlay ${ROOT}/flat.json`, 40000);
  const pulled = await desk.send("sync", 40000);
  const again = await phone.send("show");
  const left = pushed.detail;
  const right = pulled.detail;
  const third = again.detail;
  const mark = left.mark;
  if (!/^[0-9a-f]{6}$/.test(mark)) throw new Error("set code missing");
  if (right.mark !== mark || third.mark !== mark) throw new Error("set codes differ");
  const a = shape(left.present);
  const b = shape(right.present);
  const c = shape(third.present);
  const same = JSON.stringify(a) === JSON.stringify(b) && JSON.stringify(b) === JSON.stringify(c);
  if (!same) {
    console.log(JSON.stringify({ phone: a, desk: b, phoneAgain: c }));
    throw new Error("libraries differ");
  }
  if (a.inside.join(",") !== "oats,ferry,bread,cake") throw new Error(`folder ${a.inside.join(",")}`);
  if (!a.roots.includes("folder:pantry") || !a.roots.includes("page:loaf")) throw new Error(`roots ${a.roots.join(",")}`);
  if (a.trash.join(",") !== "kept,tin") throw new Error(`trash ${a.trash.join(",")}`);
  if (left.live !== true || right.live !== true) throw new Error(`live desk ${right.live} phone ${left.live}`);
  console.log(JSON.stringify({
    mark,
    deskChip: right.chip,
    phoneChip: left.chip,
    deskLive: right.live,
    phoneLive: left.live,
    roots: a.roots,
    inside: a.inside,
    trash: a.trash,
  }));
} catch (error) {
  failed = 1;
  console.error(String(error && error.message ? error.message : error));
} finally {
  for (const child of children) {
    try { child.kill("SIGTERM"); } catch { /* already gone */ }
  }
  await new Promise((resolve) => setTimeout(resolve, 300));
}
process.exit(failed);
