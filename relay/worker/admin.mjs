// Admin commands for the sync worker. The code is minted on the server.
// ADMIN_KEY is read from the environment and is never printed.
// The default host is the test worker. Set SYNC_URL to reach production.
//
//   read -s ADMIN_KEY && export ADMIN_KEY
//   node relay/worker/admin.mjs mint --uses 3 --label "Friends"
//   node relay/worker/admin.mjs mint --uses 1 --expires 2026-12-31
//   node relay/worker/admin.mjs list
//   node relay/worker/admin.mjs revoke <hash>
//   node relay/worker/admin.mjs stats
//   node relay/worker/admin.mjs pause
//   node relay/worker/admin.mjs resume
//   node relay/worker/admin.mjs limits
//   node relay/worker/admin.mjs limits --max-accounts 10000
//   node relay/worker/admin.mjs backups
//   node relay/worker/admin.mjs backup
//   node relay/worker/admin.mjs restore <account> --date 2026-10-25 --confirm <account>
//   node relay/worker/admin.mjs restore --all --date 2026-10-25 --confirm 2026-10-25
//   node relay/worker/admin.mjs delete <account> --confirm <account>
import { fileURLToPath } from "node:url";
import { LIMIT_FLAGS } from "./limits.js";

export const DEFAULT_SYNC_URL = "https://sync-test.omacus.com";

function flag(argv, name) {
  const index = argv.indexOf(name);
  if (index === -1) return undefined;
  return argv[index + 1];
}

export function planLimits(argv, current) {
  const next = {};
  let changed = false;
  for (const [name, key] of LIMIT_FLAGS) {
    const index = argv.indexOf(name);
    if (index === -1) continue;
    const raw = argv[index + 1];
    if (raw === undefined || !/^\d+$/.test(raw)) return { error: "That limit didn't work." };
    const value = Number(raw);
    if (!Number.isSafeInteger(value)) return { error: "That limit didn't work." };
    next[key] = value;
    changed = true;
  }
  const limits = {};
  for (const [, key] of LIMIT_FLAGS) {
    if (Object.hasOwn(next, key)) limits[key] = next[key];
    else if (current && Number.isInteger(current[key])) limits[key] = current[key];
  }
  if (!changed) return { changed: false, limits };
  return { changed: true, limits };
}

const LIMIT_LINES = [
  ["maxAccounts", "Max accounts"],
  ["maxStorage", "Max storage per account"],
  ["maxDevices", "Max devices"],
  ["accountRate", "Requests per account"],
  ["ipRate", "Requests per address"],
  ["guessRate", "Code guesses per address"],
  ["joinRate", "Join attempts per address"],
  ["readAccountRate", "Reads per account"],
  ["readIpRate", "Reads per address"],
];

export function formatLimits(data) {
  const lines = LIMIT_LINES.map(([key, label]) => {
    const value = data[key];
    if (key === "maxStorage") return `${label}: ${value} bytes`;
    return `${label}: ${value}`;
  });
  lines.push("Each rate is for 60 seconds.");
  lines.push(data.saved ? "Saved on this server." : "Using the starting limits.");
  return lines.join("\n");
}

function utc(ms) {
  if (!ms) return "—";
  const date = new Date(ms);
  const pad = (n) => String(n).padStart(2, "0");
  return `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())} ${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())} UTC`;
}

async function main() {
  const argv = process.argv.slice(2);
  const command = argv[0];
  const base = (process.env.SYNC_URL || DEFAULT_SYNC_URL).replace(/\/$/, "");
  const key = process.env.ADMIN_KEY || "";
  if (!command || command === "--help") {
    console.log("mint, list, revoke, stats, pause, resume, limits, backups, backup, restore, delete");
    return;
  }
  if (!key) {
    console.error("Set ADMIN_KEY in the environment. It is not printed.");
    process.exit(1);
  }
  let host = "";
  try { host = new URL(base).hostname; } catch { host = ""; }
  if (host === "sync.omacus.com") console.error("This command is aimed at the production sync host.");

  const headers = { authorization: `Bearer ${key}` };
  if (process.env.CF_ACCESS_CLIENT_ID && process.env.CF_ACCESS_CLIENT_SECRET) {
    headers["cf-access-client-id"] = process.env.CF_ACCESS_CLIENT_ID;
    headers["cf-access-client-secret"] = process.env.CF_ACCESS_CLIENT_SECRET;
  }

  let method = "GET";
  let path = "";
  let body;
  if (command === "mint") {
    method = "POST";
    path = "/v1/admin/codes";
    const uses = Number(flag(argv, "--uses") || "1");
    body = { uses, label: flag(argv, "--label") || "", expires: flag(argv, "--expires") || "" };
  } else if (command === "list") {
    path = "/v1/admin/codes";
  } else if (command === "revoke") {
    method = "POST";
    path = `/v1/admin/codes/${argv[1] || ""}/revoke`;
    body = {};
  } else if (command === "stats") {
    path = "/v1/admin/stats";
  } else if (command === "pause" || command === "resume") {
    method = "POST";
    path = "/v1/admin/signups";
    body = { paused: command === "pause" };
  } else if (command === "limits") {
    method = "GET";
    path = "/v1/admin/limits";
  } else if (command === "backups") {
    path = "/v1/admin/backups";
  } else if (command === "backup") {
    method = "POST";
    path = "/v1/admin/backups";
    body = {};
  } else if (command === "restore") {
    const date = flag(argv, "--date") || "";
    const confirm = flag(argv, "--confirm") || "";
    method = "POST";
    path = "/v1/admin/restore";
    if (argv.includes("--all")) {
      if (confirm !== date || !/^\d{4}-\d{2}-\d{2}$/.test(date)) {
        console.error("Type the day to confirm.");
        process.exit(1);
      }
      body = { all: true, confirm, date };
    } else {
      const account = argv[1] || "";
      if (confirm !== account || !/^[0-9a-f]{64}$/.test(account)) {
        console.error("Type the account id to confirm.");
        process.exit(1);
      }
      body = { account, confirm, date };
    }
  } else if (command === "delete") {
    const account = argv[1] || "";
    const confirm = flag(argv, "--confirm") || "";
    if (confirm !== account || !/^[0-9a-f]{64}$/.test(account)) {
      console.error("Type the account id to confirm.");
      process.exit(1);
    }
    method = "DELETE";
    path = `/v1/admin/accounts/${account}`;
    body = { confirm };
  } else {
    console.error("Unknown command. Use mint, list, revoke, stats, pause, resume, limits, backups, backup, restore, or delete.");
    process.exit(1);
  }

  if (body) headers["content-type"] = "application/json";
  let response;
  try {
    response = await fetch(base + path, {
      method,
      headers,
      body: body ? JSON.stringify(body) : undefined,
    });
  } catch {
    console.error("The sync host did not answer.");
    process.exit(1);
  }
  const data = await response.json().catch(() => ({}));
  if (!response.ok) {
    console.error(data.message || "That request did not work.");
    process.exit(1);
  }
  if (command === "mint") {
    console.log(data.code);
    console.log("Copy this code now. It is not shown again.");
    if (data.label) console.log(`Label: ${data.label}`);
    console.log(`Uses: ${data.uses}`);
    console.log(data.expires ? `Expires: ${utc(data.expires)}` : "Expires: none");
    return;
  }
  if (command === "list") {
    if (!data.codes || !data.codes.length) {
      console.log("No codes yet.");
      return;
    }
    for (const row of data.codes) {
      console.log([row.label || "—", `left ${row.left}`, utc(row.created), row.usedAt ? utc(row.usedAt) : "—", row.status, row.hash].join("  "));
    }
    return;
  }
  if (command === "stats") {
    console.log(`Accounts ${data.accounts}. Devices ${data.devices}.`);
    console.log(data.paused ? "New signups are paused." : "New signups are on.");
    if (!data.storage || !data.storage.length) console.log("No accounts yet.");
    for (const row of data.storage || []) console.log(`${row.account}  ${row.bytes} bytes  ${row.devices} devices`);
    if (!data.errors || !data.errors.length) console.log("No recent errors.");
    for (const row of data.errors || []) console.log(`${row.name} ${row.count}`);
    return;
  }
  if (command === "limits") {
    const planned = planLimits(argv, data);
    if (planned.error) {
      console.error(planned.error);
      process.exit(1);
    }
    if (!planned.changed) {
      console.log(formatLimits(data));
      return;
    }
    const saved = await fetch(base + path, {
      method: "POST",
      headers: { ...headers, "content-type": "application/json" },
      body: JSON.stringify(planned.limits),
    });
    const next = await saved.json().catch(() => ({}));
    if (!saved.ok) {
      console.error(next.message || "That request did not work.");
      process.exit(1);
    }
    console.log(formatLimits(next));
    return;
  }
  if (command === "backups") {
    if (data.ready === false) {
      console.log("Backups are not set up.");
      return;
    }
    if (!data.backups || !data.backups.length) {
      console.log("No nightly copies yet.");
      return;
    }
    if (data.last) {
      const when = data.last.at ? utc(data.last.at) : data.last.day;
      const outcome = data.last.status === "saved" ? "Saved." : `${data.last.failed} didn't save.`;
      console.log(`Last copy ${when}. ${outcome}`);
      for (const row of data.last.accounts || []) {
        console.log(`${row.account}  ${row.objects} objects  ${row.ok === false ? "Didn't save" : "Saved"}`);
      }
      for (const row of data.last.errors || []) console.log(`${row.account}  ${row.message}`);
    }
    for (const row of data.backups) {
      const when = row.at ? utc(row.at) : row.day;
      console.log(`${when}  ${row.accounts} accounts  ${row.status === "saved" ? "Saved" : "Didn't save"}`);
    }
    return;
  }
  if (command === "backup") {
    console.log(`${data.day}  ${data.accounts} accounts`);
    return;
  }
  console.log("Done.");
}

const isMain = process.argv[1] && process.argv[1] === fileURLToPath(import.meta.url);
if (isMain) main();
