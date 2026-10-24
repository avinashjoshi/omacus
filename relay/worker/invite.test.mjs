import assert from "node:assert/strict";
import { decodeCode, encodeCode, hashHex, hmacBytes } from "./invite.js";

const secret = Uint8Array.from([1, 2, 3, 4, 5]);
assert.equal(encodeCode(secret), "0410-6105");
assert.deepEqual([...decodeCode("o410-6105")], [...secret]);
assert.equal(await hashHex(secret), "74f81fe167d99b4cb41d6d0ccda82278caee9f3e2f25d5e5a3936ff3dcec60d0");
const mac = await hmacBytes(secret, new Uint8Array(80).fill(7));
const hex = [...mac].map((byte) => byte.toString(16).padStart(2, "0")).join("");
assert.equal(hex, "afafdd003f0810b9ea25a10e555b13e163697cec5710fa8cc39f9f497b9f6b4a");
console.log("invite code matches");
