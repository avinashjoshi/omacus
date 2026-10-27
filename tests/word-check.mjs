import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { WORD_CHECK_ERROR, buildChoices, bundledWords, usableList } from "../site/app/word-check.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const engine = readFileSync(join(root, "engine/src/recovery_words.txt"), "utf8")
  .split("\n")
  .map((line) => line.trim())
  .filter((line) => line && !line.startsWith("#"));

let failed = 0;
function check(cond, message) {
  if (!cond) {
    failed += 1;
    console.error(message);
  }
}

const bundled = bundledWords();
check(bundled && bundled.length === 2048, "the bundled list is not 2048 words");
check(bundled && engine.length === bundled.length && engine.every((word, i) => word === bundled[i]), "the bundled list drifted from the engine list");
check(usableList(["<meta", "viewport"]) === null, "html was accepted as a word list");
check(usableList(engine.slice(0, 10)) === null, "a short list was accepted");
check(usableList([...engine, engine[0]].slice(0, 2048)) === null || new Set(engine).size === 2048, "a repeated word was accepted");
const doubled = engine.slice();
doubled[1] = doubled[0];
check(usableList(doubled) === null, "a duplicate word was accepted");

let cursor = 0;
function randomInt(max) {
  cursor = (cursor + 17) % max;
  return cursor;
}
const phrase = engine.slice(0, 12);
const choices = buildChoices(phrase, [3, 7, 9], bundled, randomInt);
check(choices && choices[3].length === 6 && choices[7].length === 6 && choices[9].length === 6, "a check did not offer six words");
if (choices) {
  for (const row of Object.values(choices)) {
    check(row.every((word) => /^[a-z]+$/.test(word) && bundled.includes(word)), "a choice was not a recovery word");
    check(!row.some((word) => word.includes("<")), "a choice contained markup");
  }
}
check(buildChoices(phrase, [3, 7, 9], ["only"], randomInt) === null, "a bad list still built choices");
check(buildChoices(["notaword", ...phrase.slice(1)], [3, 7, 9], bundled, randomInt) === null, "an unknown word still built choices");
check(WORD_CHECK_ERROR === "The word check could not be loaded. Try again.", "the failure sentence changed");

if (failed) {
  console.error(`${failed} word check failed`);
  process.exit(1);
}
console.log("word check ok");
