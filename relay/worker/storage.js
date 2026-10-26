// A mailbox store with the same methods as Durable Object storage.
// The worker talks to this shape. Tests use memory or a directory of files.
// Object files are named by the object id, not by a page id.

import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

function clone(value) {
  return value === undefined ? undefined : structuredClone(value);
}

export class MemoryStorage {
  constructor() {
    this.rows = new Map();
    this.chain = Promise.resolve();
  }
  async transaction(fn) {
    const run = this.chain.then(async () => {
      const snapshot = clone(this.rows);
      const txn = this.view();
      try {
        await fn(txn);
      } catch (error) {
        this.rows = snapshot;
        throw error;
      }
    });
    this.chain = run.then(() => {}, () => {});
    return run;
  }
  view() {
    return {
      get: async (key) => clone(this.rows.get(key)),
      put: async (key, value) => {
        this.rows.set(key, clone(value));
      },
      delete: async (key) => {
        this.rows.delete(key);
      },
      list: async ({ prefix } = {}) => this.listed(prefix),
    };
  }
  listed(prefix) {
    const out = new Map();
    for (const [key, value] of this.rows) {
      if (!prefix || key.startsWith(prefix)) out.set(key, clone(value));
    }
    return out;
  }
  async get(key) {
    return clone(this.rows.get(key));
  }
  async put(key, value) {
    this.rows.set(key, clone(value));
  }
  async delete(key) {
    this.rows.delete(key);
  }
  async list({ prefix } = {}) {
    return this.listed(prefix);
  }
}

export class FileStorage {
  constructor(dir) {
    this.dir = dir;
    this.rows = new Map();
    this.chain = Promise.resolve();
    mkdirSync(dir, { recursive: true });
    this.load();
  }
  load() {
    this.rows = new Map();
    const kvDir = join(this.dir, "kv");
    if (existsSync(kvDir)) {
      for (const name of readdirSync(kvDir)) {
        if (!name.endsWith(".json")) continue;
        const key = Buffer.from(name.slice(0, -5), "hex").toString("utf8");
        this.rows.set(key, JSON.parse(readFileSync(join(kvDir, name), "utf8")));
      }
    }
    const objDir = join(this.dir, "obj");
    if (!existsSync(objDir)) return;
    for (const account of readdirSync(objDir)) {
      const accountDir = join(objDir, account);
      for (const name of readdirSync(accountDir)) {
        if (!name.endsWith(".json")) continue;
        const id = name.slice(0, -5);
        this.rows.set(`o:${account}:${id}`, JSON.parse(readFileSync(join(accountDir, name), "utf8")));
      }
    }
  }
  fileFor(key) {
    if (key.startsWith("o:")) {
      const rest = key.slice(2);
      const split = rest.indexOf(":");
      const account = rest.slice(0, split);
      const id = rest.slice(split + 1);
      return join(this.dir, "obj", account, `${id}.json`);
    }
    return join(this.dir, "kv", `${Buffer.from(key, "utf8").toString("hex")}.json`);
  }
  flush() {
    rmSync(this.dir, { recursive: true, force: true });
    mkdirSync(this.dir, { recursive: true });
    for (const [key, value] of this.rows) {
      const path = this.fileFor(key);
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, JSON.stringify(value));
    }
  }
  async transaction(fn) {
    const run = this.chain.then(async () => {
      const snapshot = clone(this.rows);
      const txn = {
        get: async (key) => clone(this.rows.get(key)),
        put: async (key, value) => {
          this.rows.set(key, clone(value));
        },
        delete: async (key) => {
          this.rows.delete(key);
        },
        list: async ({ prefix } = {}) => {
          const out = new Map();
          for (const [key, value] of this.rows) {
            if (!prefix || key.startsWith(prefix)) out.set(key, clone(value));
          }
          return out;
        },
      };
      try {
        await fn(txn);
        this.flush();
      } catch (error) {
        this.rows = snapshot;
        this.flush();
        throw error;
      }
    });
    this.chain = run.then(() => {}, () => {});
    return run;
  }
  async get(key) {
    return clone(this.rows.get(key));
  }
  async put(key, value) {
    this.rows.set(key, clone(value));
    this.flush();
  }
  async delete(key) {
    this.rows.delete(key);
    this.flush();
  }
  async list({ prefix } = {}) {
    const out = new Map();
    for (const [key, value] of this.rows) {
      if (!prefix || key.startsWith(prefix)) out.set(key, clone(value));
    }
    return out;
  }
}
