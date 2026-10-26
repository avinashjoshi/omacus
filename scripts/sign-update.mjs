#!/usr/bin/env node
// Sign an update manifest with a minisign legacy signature (algorithm Ed).
// Ed signs the file itself. It is not the prehashed ED algorithm.
//
// MINISIGN_SECRET_KEY is three lines, and it is not stored in the repository:
//   omacus-update-key-v1
//   <16 hex key id>
//   <64 hex seed>
//
//   node scripts/sign-update.mjs <file>     writes <file>.minisig
//   node scripts/sign-update.mjs --public   writes the public key
//
// TRUSTED_COMMENT fixes the trusted comment. The release leaves it unset, so
// the comment is the current time.

import { createPrivateKey, createPublicKey, sign } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const PKCS8_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

function die(message) {
  console.error(message);
  process.exit(1);
}

function loadSecret() {
  const raw = process.env.MINISIGN_SECRET_KEY;
  if (!raw || !raw.trim()) {
    die("MINISIGN_SECRET_KEY is not set. Refusing to publish an unsigned update.");
  }
  const lines = raw.trim().split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  if (lines[0] !== "omacus-update-key-v1" || lines.length < 3) {
    die("MINISIGN_SECRET_KEY is not an omacus update key.");
  }
  const keyId = Buffer.from(lines[1], "hex");
  const seed = Buffer.from(lines[2], "hex");
  if (keyId.length !== 8 || seed.length !== 32 || lines[1].length !== 16 || lines[2].length !== 64) {
    die("MINISIGN_SECRET_KEY is not an omacus update key.");
  }
  return { keyId, seed };
}

function privateKey(seed) {
  return createPrivateKey({
    key: Buffer.concat([PKCS8_PREFIX, seed]),
    format: "der",
    type: "pkcs8",
  });
}

function publicBytes(seed) {
  const der = createPublicKey(privateKey(seed)).export({ format: "der", type: "spki" });
  if (der.length !== SPKI_PREFIX.length + 32 || !der.subarray(0, SPKI_PREFIX.length).equals(SPKI_PREFIX)) {
    die("the update key has no Ed25519 public key");
  }
  return der.subarray(SPKI_PREFIX.length);
}

function signRaw(seed, message) {
  const sig = sign(null, message, privateKey(seed));
  if (sig.length !== 64) die("the signature is not 64 bytes");
  return sig;
}

function publicFile(keyId, pk) {
  const body = Buffer.concat([Buffer.from("Ed"), keyId, pk]);
  return `untrusted comment: minisign public key ${keyId.toString("hex").toUpperCase()}\n${body.toString("base64")}\n`;
}

function signatureFile(keyId, seed, message, trusted) {
  const sig = signRaw(seed, message);
  const struct = Buffer.concat([Buffer.from("Ed"), keyId, sig]);
  const comment = trusted && trusted.length ? trusted : `timestamp:${Math.floor(Date.now() / 1000)}`;
  if (/[\r\n]/.test(comment)) die("the trusted comment must be one line");
  const global = signRaw(seed, Buffer.concat([sig, Buffer.from(comment, "utf8")]));
  return [
    "untrusted comment: signature from omacus update key",
    struct.toString("base64"),
    `trusted comment: ${comment}`,
    global.toString("base64"),
    "",
  ].join("\n");
}

const { keyId, seed } = loadSecret();
const arg = process.argv[2];
if (arg === "--public") {
  process.stdout.write(publicFile(keyId, publicBytes(seed)));
} else if (arg) {
  const message = readFileSync(arg);
  const out = signatureFile(keyId, seed, message, process.env.TRUSTED_COMMENT);
  writeFileSync(`${arg}.minisig`, out);
} else {
  die("usage: node scripts/sign-update.mjs <file> | --public");
}
