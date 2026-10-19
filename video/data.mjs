// Every answer in the video comes from the real engine: each sheet below is run
// through `omacus -` and saved with its results to build/data.json.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";

const bin = new URL("../target/release/omacus", import.meta.url).pathname;
const evaluate = (lines) =>
  execFileSync(bin, ["-"], { input: lines.join("\n"), encoding: "utf8" }).split("\n").slice(0, lines.length);
const sheet = (lines) => ({ lines, results: evaluate(lines) });
const demo = (name) => readFileSync(new URL(`../site/tools/demo/${name}.txt`, import.meta.url), "utf8").trimEnd().split("\n");

const invoice = (rate) => [
  "# Invoice — October",
  `rate = $${rate}/hour`,
  "",
  "Design review 6 hours * rate",
  "Build 32 hours * rate",
  "Client calls 4.5 hours * rate",
  "subtotal = sum",
  "tax = 8.875% of subtotal",
  "subtotal + tax",
  "prev in EUR",
];

const data = {
  hook: sheet(["6pm PST in Tokyo"]),
  kyoto: sheet(demo("kyoto")),
  // The shared Kyoto sheet on a phone, with one more line typed there.
  phone: sheet([...demo("kyoto"), "each in AUD"]),
  quick: sheet([
    "# Quick math",
    "6pm PST in Tokyo",
    "3pm PST on Dec 1 in London",
    "",
    "15% off $1,299",
    "$50 + 8% tax",
    "6 is what % of 120",
    "",
    "3.5 km in miles",
    "350 F in C",
    "2 cups in ml",
  ]),
  invoice150: sheet(invoice(150)),
  invoice175: sheet(invoice(175)),
  // The montage: one-liners, one after another.
  montage: sheet([
    "3pm PST on Dec 1 in London",
    "18% tip on $62",
    "$84 split 3 ways",
    "$2.4M / $180k per month",
    "days until Dec 25",
    "255 in hex",
    "4 GB at 100 Mbps",
    "$5M at $20M pre",
  ]),
  // The terminal: one expression, then a sheet piped in.
  cli: {
    one: { cmd: "omacus '$100 in EUR'", out: [execFileSync(bin, ["$100 in EUR"], { encoding: "utf8" }).trim()] },
    pipe: { cmd: "printf 'rent = $1,450\\nrent * 12\\nprev in EUR\\n' | omacus -", out: evaluate(["rent = $1,450", "rent * 12", "prev in EUR"]) },
  },
  sheets: ["Kyoto in autumn", "Quick math", "Invoice — October", "Standup times", "Monthly budget", "Kitchen"],
  version: execFileSync(bin, ["--version"], { encoding: "utf8" }).trim().replace("omacus ", ""),
};
mkdirSync(new URL("build/", import.meta.url), { recursive: true });
writeFileSync(new URL("build/data.json", import.meta.url), JSON.stringify(data, null, 1));
for (const [k, v] of Object.entries(data)) {
  if (v?.lines) v.lines.forEach((l, i) => l && console.log(`${k.padEnd(10)} ${l.padEnd(36)} ${v.results[i]}`));
}
