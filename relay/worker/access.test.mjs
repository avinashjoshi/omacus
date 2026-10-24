import { mint, openCode, spend } from "./access.js";

const secret = "omacus-access-test-secret";
const id = Uint8Array.from([1, 2, 3, 4, 5, 6]);
const code = await mint(secret, 1, id);
if (code !== "0410-6105-0R0N-S1ZK-97T3-78B1") throw new Error(`code ${code}`);
const opened = await openCode(secret, code);
if (!opened || opened.uses !== 1 || opened.hash !== "171c9e446cd1b267ec7309325edc0cefa1f9f169f869a0aee36a2de0f00f3502") throw new Error("open");
if (await openCode(secret, "nope")) throw new Error("short code opened");
if (await openCode("short-secret", code)) throw new Error("short secret opened");
const table = {};
if (!spend(table, opened.hash, opened.uses, true)) throw new Error("first use");
if (spend(table, opened.hash, opened.uses, true)) throw new Error("second use");
if (table[opened.hash] !== 0) throw new Error("left");
const twice = await mint(secret, 2, Uint8Array.from([9, 8, 7, 6, 5, 4]));
const again = await openCode(secret, twice);
const many = {};
if (!spend(many, again.hash, again.uses, false)) throw new Error("peek");
if (Object.keys(many).length) throw new Error("peek stored a row");
if (!spend(many, again.hash, again.uses, true)) throw new Error("use 1");
if (!spend(many, again.hash, again.uses, true)) throw new Error("use 2");
if (spend(many, again.hash, again.uses, true)) throw new Error("use 3");
const kept = JSON.stringify(table);
if (kept.includes(code.replaceAll("-", "")) || kept.includes(secret)) {
  throw new Error("the stored row kept the code or the secret");
}
console.log("access code matches");
