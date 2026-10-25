// Server limits. Production starts at 10,000 accounts, 50 MB per account,
// and 10 devices. The test worker starts lower. A saved config replaces
// either set. The window for every rate is 60 seconds.

export const LIMIT_HEADER = "x-omacus-limits";
export const LIMIT_KEY = "limits";

export const FIELDS = [
  ["maxAccounts", 1, 1_000_000],
  ["maxStorage", 1024, 1024 * 1024 * 1024 * 1024],
  ["maxDevices", 1, 64],
  ["accountRate", 1, 100_000],
  ["ipRate", 1, 100_000],
  ["guessRate", 1, 100_000],
  ["joinRate", 1, 100_000],
  ["readAccountRate", 1, 100_000],
  ["readIpRate", 1, 100_000],
];

export const PRODUCTION = {
  maxAccounts: 10_000,
  maxStorage: 50 * 1024 * 1024,
  maxDevices: 10,
  accountRate: 240,
  ipRate: 800,
  guessRate: 10,
  joinRate: 10,
  readAccountRate: 180,
  readIpRate: 500,
};

export const TEST = {
  maxAccounts: 3,
  maxStorage: 64 * 1024,
  maxDevices: 4,
  accountRate: 60,
  ipRate: 80,
  guessRate: 4,
  joinRate: 4,
  readAccountRate: 30,
  readIpRate: 40,
};

export function defaultsFor(env) {
  const source = String(env && env.SERVE_APP) === "1" ? TEST : PRODUCTION;
  return { ...source };
}

export function normalizeLimits(input, base) {
  if (!input || typeof input !== "object" || Array.isArray(input)) return null;
  const next = { ...(base || PRODUCTION) };
  for (const [key, min, max] of FIELDS) {
    if (!Object.hasOwn(input, key)) continue;
    const value = input[key];
    if (!Number.isInteger(value) || value < min || value > max) return null;
    next[key] = value;
  }
  return next;
}

export function limitsFromHeader(headers) {
  const raw = headers && typeof headers.get === "function" ? headers.get(LIMIT_HEADER) : "";
  if (!raw) return null;
  try {
    return normalizeLimits(JSON.parse(raw), PRODUCTION);
  } catch {
    return null;
  }
}

export function rejectSignup({ devices, bytes }, limits) {
  if (!limits) return "";
  if (devices > limits.maxDevices) return "Devices";
  if (bytes > limits.maxStorage) return "Room";
  return "";
}

export const LIMIT_FLAGS = [
  ["--max-accounts", "maxAccounts"],
  ["--max-storage", "maxStorage"],
  ["--max-devices", "maxDevices"],
  ["--account-rate", "accountRate"],
  ["--address-rate", "ipRate"],
  ["--guess-rate", "guessRate"],
  ["--join-rate", "joinRate"],
  ["--read-rate", "readAccountRate"],
  ["--read-address-rate", "readIpRate"],
];
