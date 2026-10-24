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

async function refresh() {
  const [listed, stats] = await Promise.all([api("GET", "/v1/admin/codes"), api("GET", "/v1/admin/stats")]);
  signups.textContent = stats.paused ? "New signups are paused." : "New signups are on.";
  pause.textContent = stats.paused ? "Resume new signups" : "Pause new signups";
  counts.textContent = `Accounts ${stats.accounts}. Devices ${stats.devices}.`;
  renderCodes(listed.codes || []);
  renderStorage(stats.storage || []);
  renderErrors(stats.errors || []);
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
