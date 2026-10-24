// Optional list of private phrases. One phrase per line.
//
// The checks read OMACUS_PRIVATE_TERMS, and an untracked file at
// tests/private-terms.local when it exists. A line starting with # is a
// comment. When both sources are absent or blank, there is nothing to check
// and callers skip. A failure must not include the phrase.

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
export const localTermsPath = join(here, "private-terms.local");

export function termsFromText(raw) {
  const terms = [];
  const seen = new Set();
  for (const line of String(raw ?? "").split(/\r?\n/)) {
    const term = line.trim();
    if (!term || term.startsWith("#")) continue;
    const key = term.toLocaleLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    terms.push(term);
  }
  return terms;
}

export function loadPrivateTerms({ env = process.env, file = localTermsPath } = {}) {
  const fromEnv = termsFromText(env.OMACUS_PRIVATE_TERMS || "");
  const fromFile = file && existsSync(file) ? termsFromText(readFileSync(file, "utf8")) : [];
  return termsFromText([...fromEnv, ...fromFile].join("\n"));
}

// How many configured phrases occur in text. Zero when the list is empty.
export function privateTermHits(text, terms) {
  if (!text || !terms || terms.length === 0) return 0;
  const haystack = String(text).toLocaleLowerCase();
  let hits = 0;
  for (const term of terms) {
    if (haystack.includes(term.toLocaleLowerCase())) hits += 1;
  }
  return hits;
}
