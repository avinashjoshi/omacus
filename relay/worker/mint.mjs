// Mint an invite code for a new set of pages.
// The secret is read from ACCESS_SECRET and is never printed.
//
//   npx wrangler secret put ACCESS_SECRET
//   read -s ACCESS_SECRET && export ACCESS_SECRET
//   node relay/worker/mint.mjs
//   node relay/worker/mint.mjs --uses 3
import { mint } from "./access.js";

const secret = process.env.ACCESS_SECRET || "";
let uses = 1;
const flag = process.argv.indexOf("--uses");
if (flag !== -1) uses = Number(process.argv[flag + 1]);
if (secret.length < 16) {
  console.error("Set ACCESS_SECRET in the environment. It is not printed.");
  process.exit(1);
}
const code = await mint(secret, uses);
if (!code) {
  console.error("Uses must be a number from 1 to 20.");
  process.exit(1);
}
console.log(code);
