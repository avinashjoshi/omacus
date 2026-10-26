// One entry per set of pages on this phone.
// The account id still comes from the recovery words, the same derivation as before.
// The sealed marker in the record stays the fixed protocol label. This file changes neither.

export const VAULTS_KEY = "sync-vaults";

const LEGACY = {
  deviceFile: "device-file",
  account: "sync-account",
  revision: "sync-revision",
  slots: "sync-slots",
  base: "sync-base",
  paused: "sync-paused",
  name: "sync-vault-name",
  vaultRev: "sync-vault-rev",
  deviceName: "sync-device-name",
  deviceRev: "sync-device-rev",
  selfId: "sync-self-id",
  names: "sync-names",
  membership: "sync-membership",
};

function text(value) {
  return value == null ? "" : String(value);
}

export function normalizeVault(raw) {
  const row = raw && typeof raw === "object" ? raw : {};
  return {
    account: text(row.account).trim().toLowerCase(),
    deviceFile: text(row.deviceFile),
    revision: text(row.revision || "0"),
    slots: text(row.slots || "[]"),
    base: text(row.base || "[]"),
    paused: row.paused === true || row.paused === "1",
    name: text(row.name).trim() || "My pages",
    deviceName: text(row.deviceName),
    deviceRev: text(row.deviceRev || "1"),
    vaultRev: text(row.vaultRev || "1"),
    selfId: text(row.selfId),
    names: text(row.names || "[]"),
    membership: text(row.membership || "[]"),
  };
}

export function vaultFromLegacy(get) {
  const deviceFile = text(get(LEGACY.deviceFile));
  const account = text(get(LEGACY.account)).trim().toLowerCase();
  const paused = text(get(LEGACY.paused)) === "1";
  if (!deviceFile && !account && !paused) return null;
  return normalizeVault({
    account,
    deviceFile,
    revision: get(LEGACY.revision) || "0",
    slots: get(LEGACY.slots) || "[]",
    base: get(LEGACY.base) || "[]",
    paused,
    name: get(LEGACY.name) || "My pages",
    deviceName: get(LEGACY.deviceName) || "",
    deviceRev: get(LEGACY.deviceRev) || "1",
    vaultRev: get(LEGACY.vaultRev) || "1",
    selfId: get(LEGACY.selfId) || "",
    names: get(LEGACY.names) || "[]",
    membership: get(LEGACY.membership) || "[]",
  });
}

function parseBook(raw) {
  if (!raw) return null;
  try {
    const value = JSON.parse(raw);
    const rows = Array.isArray(value) ? value : value && Array.isArray(value.vaults) ? value.vaults : null;
    if (!rows) return null;
    const vaults = rows.map(normalizeVault);
    const active = text(value && value.active).trim().toLowerCase();
    return { active, vaults };
  } catch {
    return null;
  }
}

// A saved list wins. The older single slot is copied in once, then left alone.
export function loadVaultBook(get) {
  const parsed = parseBook(get(VAULTS_KEY));
  if (parsed) return { active: parsed.active, vaults: parsed.vaults, migrated: false };
  const one = vaultFromLegacy(get);
  if (!one) return { active: "", vaults: [], migrated: false };
  return { active: one.account, vaults: [one], migrated: true };
}

export function activeVault(book) {
  const vaults = book && Array.isArray(book.vaults) ? book.vaults : [];
  if (!vaults.length) return null;
  const active = text(book.active).trim().toLowerCase();
  const hit = vaults.find((row) => row.account && row.account === active);
  if (hit) return hit;
  return vaults.find((row) => row.deviceFile) || vaults[0];
}

export function upsertVault(book, vault) {
  const next = normalizeVault(vault);
  const vaults = (book && Array.isArray(book.vaults) ? book.vaults : [])
    .map(normalizeVault)
    .filter((row) => row.account !== next.account);
  vaults.push(next);
  return { active: next.account || text(book && book.active), vaults };
}

export function bookJson(book) {
  return JSON.stringify({
    active: text(book && book.active).trim().toLowerCase(),
    vaults: (book && book.vaults ? book.vaults : []).map(normalizeVault),
  });
}

// The older keys mirror the open entry, so a phone that already synced keeps working.
export function legacyMirror(vault) {
  const remove = [];
  if (!vault) {
    return {
      set: {},
      remove: Object.values(LEGACY),
    };
  }
  const set = {
    [LEGACY.revision]: vault.revision || "0",
    [LEGACY.slots]: vault.slots || "[]",
    [LEGACY.base]: vault.base || "[]",
    [LEGACY.name]: vault.name || "My pages",
    [LEGACY.vaultRev]: vault.vaultRev || "1",
    [LEGACY.deviceName]: vault.deviceName || "",
    [LEGACY.deviceRev]: vault.deviceRev || "1",
    [LEGACY.selfId]: vault.selfId || "",
    [LEGACY.names]: vault.names || "[]",
    [LEGACY.membership]: vault.membership || "[]",
  };
  if (vault.deviceFile) set[LEGACY.deviceFile] = vault.deviceFile;
  else remove.push(LEGACY.deviceFile);
  if (vault.account) set[LEGACY.account] = vault.account;
  else remove.push(LEGACY.account);
  if (vault.paused) set[LEGACY.paused] = "1";
  else remove.push(LEGACY.paused);
  return { set, remove };
}
