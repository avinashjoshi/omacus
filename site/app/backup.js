// A backup of every page, folder, and setting. Locked pages stay ciphertext.
// The same zip is what the desktop app writes.

export const BACKUP_KIND = "omacus-backup";
export const BACKUP_FORMAT = 1;

const CRC = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

function crc32(bytes) {
  let c = 0xffffffff;
  for (const b of bytes) c = CRC[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function u16(view, at, n) {
  view.setUint16(at, n, true);
}

function u32(view, at, n) {
  view.setUint32(at, n, true);
}

function safeId(id) {
  return typeof id === "string" && !!id && !id.includes("/") && !id.includes("\\") && !id.startsWith(".");
}

/// Files for one backup. `pages` is `{id, text}` and `text` is whatever is stored.
export function backupEntries({ version, library, settings, trash, pages }) {
  const enc = new TextEncoder();
  const files = [
    {
      name: "manifest.json",
      text: JSON.stringify({
        format: BACKUP_FORMAT,
        kind: BACKUP_KIND,
        version: version || "",
        pages: (pages || []).length,
      }),
    },
    { name: "library.json", text: JSON.stringify(library || {}) },
    { name: "settings.json", text: JSON.stringify(settings || {}) },
    { name: "trash.json", text: JSON.stringify(trash || []) },
  ];
  for (const page of pages || []) {
    if (!safeId(page.id)) continue;
    files.push({ name: `pages/${page.id}.txt`, text: String(page.text ?? "") });
  }
  return files.map((file) => ({ name: file.name, data: enc.encode(file.text) }));
}

/// A stored (uncompressed) zip. Returns bytes, not a Blob, so a test can read them.
export function zipBytes(files) {
  const parts = [];
  const central = [];
  let offset = 0;
  for (const file of files) {
    const name = new TextEncoder().encode(file.name);
    const crc = crc32(file.data);
    const local = new DataView(new ArrayBuffer(30));
    u32(local, 0, 0x04034b50);
    u16(local, 4, 20);
    u16(local, 6, 0x0800);
    u32(local, 14, crc);
    u32(local, 18, file.data.length);
    u32(local, 22, file.data.length);
    u16(local, 26, name.length);
    parts.push(new Uint8Array(local.buffer), name, file.data);
    const entry = new DataView(new ArrayBuffer(46));
    u32(entry, 0, 0x02014b50);
    u16(entry, 4, 20);
    u16(entry, 6, 20);
    u16(entry, 8, 0x0800);
    u32(entry, 16, crc);
    u32(entry, 20, file.data.length);
    u32(entry, 24, file.data.length);
    u16(entry, 28, name.length);
    u32(entry, 42, offset);
    central.push(new Uint8Array(entry.buffer), name);
    offset += 30 + name.length + file.data.length;
  }
  const size = central.reduce((n, part) => n + part.byteLength, 0);
  const end = new DataView(new ArrayBuffer(22));
  u32(end, 0, 0x06054b50);
  u16(end, 8, files.length);
  u16(end, 10, files.length);
  u32(end, 12, size);
  u32(end, 16, offset);
  const chunks = [...parts, ...central, new Uint8Array(end.buffer)];
  const total = chunks.reduce((n, part) => n + part.byteLength, 0);
  const out = new Uint8Array(total);
  let at = 0;
  for (const part of chunks) {
    out.set(part, at);
    at += part.byteLength;
  }
  return out;
}

function decode(bytes) {
  return new TextDecoder().decode(bytes);
}

/// Every file in a zip, stored or deflated. Names containing `..` are skipped.
export async function unzipNamed(input) {
  const buf = input instanceof Uint8Array ? input : new Uint8Array(await input.arrayBuffer());
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  let eocd = -1;
  for (let i = buf.length - 22; i >= Math.max(0, buf.length - 65557); i--) {
    if (view.getUint32(i, true) === 0x06054b50) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) return [];
  const entries = [];
  let p = view.getUint32(eocd + 16, true);
  for (let n = view.getUint16(eocd + 10, true); n > 0 && p + 46 <= buf.length; n--) {
    if (view.getUint32(p, true) !== 0x02014b50) break;
    const method = view.getUint16(p + 10, true);
    const size = view.getUint32(p + 20, true);
    const nameLen = view.getUint16(p + 28, true);
    const extra = view.getUint16(p + 30, true);
    const note = view.getUint16(p + 32, true);
    const local = view.getUint32(p + 42, true);
    const name = decode(buf.subarray(p + 46, p + 46 + nameLen));
    p += 46 + nameLen + extra + note;
    if (!name || name.includes("..") || name.endsWith("/")) continue;
    const start = local + 30 + view.getUint16(local + 26, true) + view.getUint16(local + 28, true);
    const data = buf.subarray(start, start + size);
    let text = "";
    if (method === 0) text = decode(data);
    else if (method === 8 && typeof DecompressionStream !== "undefined") {
      const out = new Blob([data]).stream().pipeThrough(new DecompressionStream("deflate-raw"));
      text = await new Response(out).text();
    } else continue;
    entries.push({ name, text });
  }
  return entries;
}

function pageIdFrom(name) {
  const match = /^pages\/([^/]+)\.txt$/.exec(name);
  return match && safeId(match[1]) ? match[1] : null;
}

/// A parsed backup, or null when the zip is not one.
export function parseBackup(entries) {
  const manifestEntry = entries.find((entry) => entry.name === "manifest.json");
  if (!manifestEntry) return null;
  let manifest;
  try {
    manifest = JSON.parse(manifestEntry.text);
  } catch {
    return null;
  }
  if (manifest.kind !== BACKUP_KIND || manifest.format !== BACKUP_FORMAT) return null;
  const readJson = (name, fallback) => {
    const entry = entries.find((item) => item.name === name);
    if (!entry) return fallback;
    try {
      return JSON.parse(entry.text);
    } catch {
      return fallback;
    }
  };
  const pages = [];
  for (const entry of entries) {
    const id = pageIdFrom(entry.name);
    if (id) pages.push({ id, text: entry.text });
  }
  const trash = readJson("trash.json", []);
  return {
    version: typeof manifest.version === "string" ? manifest.version : "",
    library: readJson("library.json", {}),
    settings: readJson("settings.json", {}),
    trash: Array.isArray(trash) ? trash : [],
    pages,
  };
}

function idsIn(tree) {
  const out = [];
  const walk = (nodes) => {
    for (const node of nodes || []) {
      if (node?.type === "page" && node.id) out.push(node.id);
      else if (node?.type === "folder") walk(node.children);
    }
  };
  walk(tree);
  return out;
}

/// Keep every local page. Add backup pages that are not already here.
/// An empty library takes the backup's folders. A library that already has
/// pages keeps them and puts new ones at the end.
export function mergeBackup(local, backup) {
  const pages = (local.pages || []).map((page) => ({ id: page.id, text: page.text }));
  const have = new Set(pages.map((page) => page.id));
  const added = [];
  for (const page of backup.pages || []) {
    if (!safeId(page.id) || have.has(page.id)) continue;
    pages.push({ id: page.id, text: String(page.text ?? "") });
    have.add(page.id);
    added.push(page.id);
  }
  let tree = Array.isArray(local.tree) ? structuredClone(local.tree) : [];
  if (pages.length === added.length && Array.isArray(backup.library?.tree)) {
    const known = new Set(pages.map((page) => page.id));
    tree = (backup.library.tree || []).filter((node) => node);
    const placed = new Set(idsIn(tree));
    for (const id of pages.map((page) => page.id)) {
      if (!placed.has(id) && known.has(id)) tree.push({ type: "page", id });
    }
  } else {
    const placed = new Set(idsIn(tree));
    for (const id of added) {
      if (!placed.has(id)) tree.push({ type: "page", id });
    }
  }
  const trash = Array.isArray(local.trash) ? local.trash.slice() : [];
  const trashed = new Set(trash.map((item) => item?.id));
  if (!trash.length) {
    for (const item of backup.trash || []) {
      if (item?.id && !have.has(item.id) && !trashed.has(item.id)) trash.push(item);
    }
  }
  const settings = { ...(local.settings || {}) };
  for (const [key, value] of Object.entries(backup.settings || {})) {
    if (settings[key] == null) settings[key] = value;
  }
  return { pages, tree, trash, settings, added };
}
