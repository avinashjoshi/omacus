// The recovery picture is a link. The words stay after "#", and a request for that
// link does not carry them. The same rule covers the join code.
import { createServer } from "node:http";
import {
  captureSyncLink,
  decodeFragment,
  encodeFragment,
  joinLink,
  pageOrigin,
  parseJoinPayload,
  parseRecovery,
  recoveryLink,
  takeSyncLink,
} from "../site/app/sync-link.mjs";

const PHRASE = "other contact online these service state email music product system policy number";
const TOKEN = "b3RoZXIgY29udGFjdCBvbmxpbmUgdGhlc2Ugc2VydmljZSBzdGF0ZSBlbWFpbCBtdXNpYyBwcm9kdWN0IHN5c3RlbSBwb2xpY3kgbnVtYmVy";
const ACCOUNT = "ab".repeat(32);
const CODE = "K7QM-4R2P";
const RELAY = "https://sync.omacus.com";

function assert(cond, message) {
  if (!cond) throw new Error(message);
}

assert(encodeFragment(PHRASE) === TOKEN, "the recovery encoding drifted");
assert(decodeFragment(TOKEN) === PHRASE, "the recovery fragment did not decode");

for (const page of ["https://app.omacus.com", "https://relay.example"]) {
  const link = recoveryLink(page, PHRASE);
  const hash = link.indexOf("#");
  assert(hash > 0, "the recovery picture is not a link");
  const before = link.slice(0, hash);
  assert(before === `${page}/`, "the recovery link host drifted");
  assert(link.slice(hash) === `#recover=${TOKEN}`, "the words left the fragment");
  assert(!before.includes("other") && !before.includes(TOKEN), "the words sit before the hash");
  assert(!link.includes("?"), "the recovery link has a query");
  assert(parseRecovery(link) === PHRASE, "a scanned recovery link did not fill the words");
}

const numbered = ["omacus-recovery", ...PHRASE.split(" ").map((word, index) => `${index + 1}. ${word}`)].join("\n");
assert(parseRecovery(numbered).startsWith("1. other"), "an older recovery picture was refused");
assert(parseRecovery("https://app.omacus.com/?recover=" + TOKEN) == null, "a query string was treated as a recovery link");
assert(parseRecovery("not a sheet") == null, "a random picture was treated as recovery words");

const join = joinLink("https://app.omacus.com", RELAY, ACCOUNT, CODE);
const joinHash = join.indexOf("#");
const joinBefore = join.slice(0, joinHash);
assert(joinBefore === "https://app.omacus.com/", "the join picture is not an app link");
assert(!joinBefore.includes(CODE) && !joinBefore.includes(ACCOUNT), "the join code sits before the hash");
assert(join.slice(joinHash).startsWith("#join="), "the join code left the fragment");
const parsed = parseJoinPayload(join);
assert(parsed && parsed.code === CODE && parsed.account === ACCOUNT && parsed.origin === RELAY, "a scanned join link dropped the code");
const legacy = `omacus-join\n${RELAY}\n${ACCOUNT}\n${CODE}\n`;
const legacyParsed = parseJoinPayload(legacy);
assert(legacyParsed && legacyParsed.code === CODE, "an older join picture was refused");
assert(joinLink("https://relay.example", RELAY, ACCOUNT, CODE).startsWith("https://relay.example/#join="), "another page's join link drifted");

const loc = { hash: `#recover=${TOKEN}`, pathname: "/app/", search: "" };
const history = { replaceState(_state, _title, url) { history.url = url; loc.hash = ""; } };
assert(captureSyncLink(loc, history) === true, "the recovery fragment was left in the address bar");
assert(history.url === "/app/", "the address bar kept the words");
assert(!history.url.includes(TOKEN) && !history.url.includes("other"), "the cleared address still has the words");
const taken = takeSyncLink();
assert(taken && taken.kind === "recover" && taken.secret === PHRASE, "the words were not kept for the recovery screen");
assert(takeSyncLink() == null, "the words were kept after the screen read them");

assert(pageOrigin({ hostname: "app.omacus.com", origin: "https://app.omacus.com" }) === "https://app.omacus.com", "the published app link drifted");
assert(pageOrigin({ hostname: "relay.example", origin: "https://relay.example" }) === "https://relay.example", "another page left its own host");
assert(pageOrigin({ hostname: "127.0.0.1", origin: "http://127.0.0.1:4173" }, { dataset: { syncTest: "1" } }) === "http://127.0.0.1:4173", "a local page left its own host");
assert(pageOrigin({ hostname: "127.0.0.1", origin: "http://127.0.0.1:4173" }, { dataset: {} }) === "http://127.0.0.1:4173", "a local page left its own host");

const seen = [];
const server = createServer((req, res) => {
  const chunks = [];
  req.on("data", (chunk) => chunks.push(chunk));
  req.on("end", () => {
    seen.push(`${req.method} ${req.url}\n${Buffer.concat(chunks).toString("utf8")}\n${JSON.stringify(req.headers)}`);
    res.writeHead(204);
    res.end();
  });
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const port = server.address().port;
const recoveryUrl = `http://127.0.0.1:${port}/app/#recover=${TOKEN}`;
const joinUrl = joinLink(`http://127.0.0.1:${port}`, RELAY, ACCOUNT, CODE);
await fetch(recoveryUrl);
await fetch(joinUrl, { method: "POST", body: "harbor" });
await fetch(`http://127.0.0.1:${port}/app/sync.js`);
server.close();
const carried = seen.join("\n");
assert(!carried.includes(PHRASE), "a request carried the recovery words");
assert(!carried.includes(TOKEN), "a request carried the recovery fragment");
assert(!carried.includes(CODE), "a request carried the join code");
assert(!carried.includes(ACCOUNT), "a request carried the account id");
assert(!carried.includes("recover=") && !carried.includes("join="), "a request carried the fragment name");

console.log("sync-link ok");
