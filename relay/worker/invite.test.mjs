import assert from "node:assert/strict";
import { decodeCode, encodeCode, hashHex, hmacBytes } from "./invite.js";

const secret = Uint8Array.from([1, 2, 3, 4, 5, 6, 7, 8]);
assert.equal(encodeCode(secret), "20G3-0G2G-C1R8");
assert.deepEqual([...decodeCode("2OG3-0G2G-C1R8")], [...secret]);
assert.equal(decodeCode("0410-6105"), null);
assert.equal(await hashHex(secret), "6afc99534f22743635cd2fef5b3220f2193a8b75f15fe3601bc9266859f9f1a6");
const mac = await hmacBytes(secret, new Uint8Array(80).fill(7));
const hex = [...mac].map((byte) => byte.toString(16).padStart(2, "0")).join("");
assert.equal(hex, "20553d58135273ffe333faffbc1632d38acb36a703c76f8c2ae3af0f470d0404");
console.log("invite code matches");
