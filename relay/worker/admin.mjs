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
//   node relay/worker/admin.mjs delete <account> --confirm <account>
import { fileURLToPath } from "node:url";

export const DEFAULT_SYNC_URL = "https://sync-test.omacus.com";

function flag(argv, name) {
  const index = argv.indexOf(name);
  if (index === -1) return undefined;
  return argv[index + 1];
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
    console.log("mint, list, revoke, stats, pause, resume, delete");
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
    console.error("Unknown command. Use mint, list, revoke, stats, pause, resume, or delete.");
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
  console.log("Done.");
}

const isMain = process.argv[1] && process.argv[1] === fileURLToPath(import.meta.url);
if (isMain) main();
