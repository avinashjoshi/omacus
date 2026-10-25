const STORE = "omacus-admin-key";
const keyForm = document.querySelector("[data-key]");
const keyError = document.querySelector("[data-key-error]");
const app = document.querySelector("[data-app]");
const signups = document.querySelector("[data-signups]");
const pause = document.querySelector("[data-pause]");
const create = document.querySelector("[data-create]");
const once = document.querySelector("[data-once]");
const codes = document.querySelector("[data-codes]");
const counts = document.querySelector("[data-counts]");
const storage = document.querySelector("[data-storage]");
const errors = document.querySelector("[data-errors]");
const limitsForm = document.querySelector("[data-limits]");
const limitsNote = document.querySelector("[data-limits-note]");
const backupsReady = document.querySelector("[data-backups-ready]");
const backupButton = document.querySelector("[data-backup]");
const backups = document.querySelector("[data-backups]");
const backupAccounts = document.querySelector("[data-backup-accounts]");
const backupErrors = document.querySelector("[data-backup-errors]");
const restore = document.querySelector("[data-restore]");
const restoreConfirm = document.querySelector("[data-restore-confirm]");
const restoreAsk = document.querySelector("[data-restore-ask]");
const restoreYes = document.querySelector("[data-restore-yes]");
const restoreNo = document.querySelector("[data-restore-no]");
const restoreNote = document.querySelector("[data-restore-note]");
const remove = document.querySelector("[data-remove]");
const removeNote = document.querySelector("[data-remove-note]");

let key = "";

function el(tag, attrs, kids) {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attrs || {})) {
    if (name === "class") node.className = value;
    else node.setAttribute(name, value);
  }
  for (const kid of kids || []) node.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  return node;
}

function clear(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
}

function utc(ms) {
  if (!ms) return "—";
  const date = new Date(ms);
  const pad = (n) => String(n).padStart(2, "0");
  return `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())} ${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())} UTC`;
}

function showLocked(message) {
  app.hidden = true;
  keyForm.hidden = false;
  keyError.textContent = message || "";
}

async function api(method, path, body) {
  const response = await fetch(path, {
    method,
    headers: {
      authorization: `Bearer ${key}`,
      ...(body ? { "content-type": "application/json" } : {}),
    },
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await response.json().catch(() => ({}));
  if (response.status === 401) {
    key = "";
    sessionStorage.removeItem(STORE);
    showLocked("That key didn't work.");
    throw new Error("That key didn't work.");
  }
  if (!response.ok) throw new Error(data.message || "Try again in a moment.");
  return data;
}

function renderCodes(list) {
  clear(codes);
  if (!list.length) {
    codes.append(el("p", { class: "muted" }, ["No codes yet."]));
    return;
  }
  const table = el("table", {}, [
    el("thead", {}, [el("tr", {}, ["Label", "Uses left", "Created", "Last used", "Status", ""].map((name) => el("th", {}, [name])))]),
  ]);
  const body = el("tbody");
  for (const row of list) {
    const button = el("button", { class: "quiet", type: "button" }, ["Revoke"]);
    button.addEventListener("click", async () => {
      button.disabled = true;
      try {
        await api("POST", `/v1/admin/codes/${row.hash}/revoke`, {});
        await refresh();
      } catch (error) {
        button.disabled = false;
        keyError.textContent = error.message;
      }
    });
    body.append(el("tr", {}, [
      el("td", {}, [row.label || "—"]),
      el("td", {}, [String(row.left)]),
      el("td", {}, [utc(row.created)]),
      el("td", {}, [utc(row.usedAt)]),
      el("td", {}, [row.status]),
      el("td", {}, [button]),
    ]));
  }
  table.append(body);
  codes.append(table);
}

function renderStorage(rows) {
  clear(storage);
  if (!rows.length) {
    storage.append(el("p", { class: "muted" }, ["No accounts yet."]));
    return;
  }
  const table = el("table", {}, [
    el("thead", {}, [el("tr", {}, ["Account", "Size", "Devices"].map((name) => el("th", {}, [name])))]),
  ]);
  const body = el("tbody");
  for (const row of rows) {
    body.append(el("tr", {}, [
      el("td", { class: "id" }, [row.account]),
      el("td", {}, [`${row.bytes} bytes`]),
      el("td", {}, [String(row.devices)]),
    ]));
  }
  table.append(body);
  storage.append(table);
}

function renderErrors(list) {
  clear(errors);
  if (!list.length) {
    errors.append(el("p", { class: "muted" }, ["No recent errors."]));
    return;
  }
  const listNode = el("ul");
  for (const row of list) listNode.append(el("li", {}, [`${row.name} ${row.count}`]));
  errors.append(listNode);
}

const LIMIT_FIELDS = [
  "maxAccounts",
  "maxStorage",
  "maxDevices",
  "accountRate",
  "ipRate",
  "guessRate",
  "joinRate",
  "readAccountRate",
  "readIpRate",
];

function fillLimits(row) {
  if (!row) return;
  for (const name of LIMIT_FIELDS) {
    const input = limitsForm.elements[name];
    if (input && Number.isInteger(row[name])) input.value = String(row[name]);
  }
}

function backupWhen(item) {
  return item && item.at ? utc(item.at) : (item && item.day) || "—";
}

function renderBackups(row) {
  clear(backups);
  clear(backupAccounts);
  clear(backupErrors);
  restoreConfirm.hidden = true;
  if (!row || row.ready === false) {
    backupsReady.textContent = "Backups are not set up.";
    backupButton.disabled = true;
    return;
  }
  backupButton.disabled = false;
  const last = row.last;
  if (!last) {
    backupsReady.textContent = "No nightly copies yet.";
    backups.append(el("p", { class: "muted" }, ["No nightly copies yet."]));
    return;
  }
  const outcome = last.status === "saved" ? "Every account saved." : `${last.failed} didn't save.`;
  backupsReady.textContent = `Last copy ${backupWhen(last)}. ${outcome}`;
  const list = row.backups || [];
  const table = el("table", {}, [
    el("thead", {}, [el("tr", {}, ["When", "Accounts", "Status"].map((name) => el("th", {}, [name])))]),
  ]);
  const body = el("tbody");
  for (const item of list) {
    body.append(el("tr", {}, [
      el("td", {}, [backupWhen(item)]),
      el("td", {}, [String(item.accounts)]),
      el("td", {}, [item.status === "saved" ? "Saved" : "Didn't save"]),
    ]));
  }
  table.append(body);
  backups.append(table);
  const accounts = last.accounts || [];
  if (accounts.length) {
    const accountTable = el("table", {}, [
      el("thead", {}, [el("tr", {}, ["Account", "Objects", "Status"].map((name) => el("th", {}, [name])))]),
    ]);
    const accountBody = el("tbody");
    for (const item of accounts) {
      accountBody.append(el("tr", {}, [
        el("td", { class: "id" }, [item.account]),
        el("td", {}, [String(item.objects || 0)]),
        el("td", {}, [item.ok === false ? "Didn't save" : "Saved"]),
      ]));
    }
    accountTable.append(accountBody);
    backupAccounts.append(accountTable);
  }
  const problems = last.errors || [];
  if (!problems.length) {
    backupErrors.append(el("p", { class: "muted" }, ["No copy errors."]));
    return;
  }
  const listNode = el("ul");
  for (const item of problems) listNode.append(el("li", {}, [`${item.account} ${item.message || "That copy didn't save."}`]));
  backupErrors.append(listNode);
}

async function refresh() {
  const [listed, stats, limits, copies] = await Promise.all([
    api("GET", "/v1/admin/codes"),
    api("GET", "/v1/admin/stats"),
    api("GET", "/v1/admin/limits"),
    api("GET", "/v1/admin/backups"),
  ]);
  signups.textContent = stats.paused ? "New signups are paused." : "New signups are on.";
  pause.textContent = stats.paused ? "Resume new signups" : "Pause new signups";
  counts.textContent = `Accounts ${stats.accounts}. Devices ${stats.devices}.`;
  renderCodes(listed.codes || []);
  renderStorage(stats.storage || []);
  renderErrors(stats.errors || []);
  fillLimits(limits);
  renderBackups(copies);
}

async function unlock(next) {
  key = next;
  sessionStorage.setItem(STORE, key);
  keyForm.hidden = true;
  keyError.textContent = "";
  app.hidden = false;
  await refresh();
  keyForm.querySelector("input").value = "";
}

keyForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const next = new FormData(keyForm).get("key");
  try {
    await unlock(String(next || ""));
  } catch (error) {
    if (!key) keyError.textContent = error.message;
  }
});

limitsForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  limitsNote.textContent = "";
  const body = {};
  const form = new FormData(limitsForm);
  for (const name of LIMIT_FIELDS) body[name] = Number(form.get(name));
  const button = limitsForm.querySelector("button");
  button.disabled = true;
  try {
    fillLimits(await api("POST", "/v1/admin/limits", body));
    limitsNote.textContent = "Limits saved.";
  } catch (error) {
    limitsNote.textContent = error.message;
  } finally {
    button.disabled = false;
  }
});

pause.addEventListener("click", async () => {
  pause.disabled = true;
  try {
    const paused = pause.textContent.startsWith("Pause");
    await api("POST", "/v1/admin/signups", { paused });
    await refresh();
  } catch (error) {
    keyError.textContent = error.message;
  } finally {
    pause.disabled = false;
  }
});

create.addEventListener("submit", async (event) => {
  event.preventDefault();
  const form = new FormData(create);
  clear(once);
  once.hidden = true;
  try {
    const made = await api("POST", "/v1/admin/codes", {
      label: String(form.get("label") || ""),
      uses: Number(form.get("uses")),
      expires: String(form.get("expires") || ""),
    });
    once.hidden = false;
    once.append(
      el("p", {}, ["Copy this code now. It is not shown again."]),
      el("p", { class: "code" }, [made.code || ""]),
    );
    await refresh();
  } catch (error) {
    keyError.textContent = error.message;
  }
});

backupButton.addEventListener("click", async () => {
  backupButton.disabled = true;
  restoreNote.textContent = "";
  try {
    await api("POST", "/v1/admin/backups", {});
    await refresh();
    restoreNote.textContent = "Nightly copy saved.";
  } catch (error) {
    restoreNote.textContent = error.message;
  } finally {
    backupButton.disabled = false;
  }
});

let pendingRestore = null;

restore.addEventListener("submit", (event) => {
  event.preventDefault();
  const form = new FormData(restore);
  const account = String(form.get("account") || "").trim();
  const date = String(form.get("date") || "");
  restoreNote.textContent = "";
  if (!date) return;
  pendingRestore = account
    ? { account, confirm: account, date }
    : { all: true, confirm: date, date };
  restoreAsk.textContent = account
    ? `Restore the sealed copy of this account from ${date}? Phones merge it with the pages they already have.`
    : `Restore every sealed copy from ${date}? Phones merge it with the pages they already have.`;
  restoreConfirm.hidden = false;
});

restoreNo.addEventListener("click", () => {
  pendingRestore = null;
  restoreConfirm.hidden = true;
});

restoreYes.addEventListener("click", async () => {
  if (!pendingRestore) return;
  restoreYes.disabled = true;
  restoreNote.textContent = "";
  try {
    const result = await api("POST", "/v1/admin/restore", pendingRestore);
    restoreNote.textContent = pendingRestore.all
      ? `Sealed copies restored. ${result.restored} saved.`
      : "Sealed copy restored.";
    pendingRestore = null;
    restoreConfirm.hidden = true;
    restore.reset();
    await refresh();
  } catch (error) {
    restoreNote.textContent = error.message;
  } finally {
    restoreYes.disabled = false;
  }
});

remove.addEventListener("submit", async (event) => {
  event.preventDefault();
  const account = String(new FormData(remove).get("account") || "").trim();
  removeNote.textContent = "";
  try {
    await api("DELETE", `/v1/admin/accounts/${account}`, { confirm: account });
    removeNote.textContent = "Sealed copy removed.";
    remove.reset();
    await refresh();
  } catch (error) {
    removeNote.textContent = error.message;
  }
});

const saved = sessionStorage.getItem(STORE) || "";
if (saved) {
  unlock(saved).catch(() => {});
}
