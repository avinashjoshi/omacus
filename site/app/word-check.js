import { RECOVERY_WORDS } from "./recovery-words.js";

export const WORD_CHECK_ERROR = "The word check could not be loaded. Try again.";

export function usableList(words) {
  if (!Array.isArray(words) || words.length !== 2048) return null;
  const seen = new Set();
  for (const word of words) {
    if (typeof word !== "string" || !/^[a-z]+$/.test(word) || seen.has(word)) return null;
    seen.add(word);
  }
  return words;
}

export function bundledWords() {
  return usableList(RECOVERY_WORDS);
}

// Six recovery words for one check, or null when the list cannot be trusted.
export function buildChoices(words, asks, list, randomInt) {
  const usable = usableList(list);
  if (!usable || !Array.isArray(words) || words.length !== 12 || !Array.isArray(asks)) return null;
  if (asks.length !== 3) return null;
  const known = new Set(usable);
  const mine = new Set();
  for (const word of words) {
    if (typeof word !== "string" || !known.has(word)) return null;
    mine.add(word);
  }
  const choices = {};
  for (const n of asks) {
    const real = words[n - 1];
    if (!real) return null;
    const options = new Set([real]);
    let guard = 0;
    while (options.size < 6 && guard < 4000) {
      guard += 1;
      const word = usable[randomInt(usable.length)];
      if (typeof word === "string" && !mine.has(word)) options.add(word);
    }
    if (options.size < 6) return null;
    for (const word of options) {
      if (!/^[a-z]+$/.test(word) || !known.has(word)) return null;
    }
    choices[n] = [...options];
  }
  return choices;
}
