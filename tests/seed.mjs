// First launch seeds example pages once. An existing library is only marked,
// and deleting the examples does not bring them back.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { PAGES, allocateHandle, freshLibrary, seedAction } from "../site/app/seed.js";

const root = dirname(fileURLToPath(import.meta.url));
const appJs = readFileSync(join(root, "../site/app/app.js"), "utf8");
const examples = readFileSync(join(root, "../site/app/examples.js"), "utf8");
const sw = readFileSync(join(root, "../site/app/sw.js"), "utf8");

const failures = [];
function check(cond, message) {
  if (!cond) failures.push(message);
}

check(seedAction({ pages: 0, seeded: false, trash: 0 }) === "examples", "an empty new library gets the examples");
check(seedAction({ pages: 2, seeded: true, trash: 0 }) === "none", "a library that was already seeded is left alone");
check(seedAction({ pages: 2, seeded: false, trash: 0 }) === "mark", "an existing library is marked and not added to");
check(seedAction({ pages: 1, seeded: false, trash: 3 }) === "mark", "pages win over trash: nothing is added");
check(seedAction({ pages: 0, seeded: true, trash: 0 }) === "welcome", "after the examples are deleted, only welcome comes back");
check(seedAction({ pages: 0, seeded: false, trash: 1 }) === "welcome-mark", "old trash means this is not a first launch");

const welcome = "# Welcome to Omacus\n// A note.\n";
const lib = freshLibrary(welcome);
check(lib.seeded === true, "the seeded library records the flag");
check(lib.active === "ex-welcome", "welcome is the open page");
check(lib.texts["ex-welcome"] === welcome, "welcome text is kept");
check(lib.handles["ex-welcome"] === "welcome", "welcome gets a handle");

const welcomeNode = lib.tree[0];
const folder = lib.tree[1];
check(lib.tree.length === 2, "the root holds the welcome page, then the folder");
check(folder.type === "folder" && folder.name === "Examples", "the folder is named Examples");
check(folder.children.length === PAGES.length, "every example page is in the folder");
check(welcomeNode.type === "page" && welcomeNode.id === "ex-welcome", "welcome stays at the root");
check(!folder.children.some((n) => n.id === "ex-welcome"), "welcome is not inside Examples");

const titles = ["Trip Budget", "Groceries", "Monthly budget", "Cooking", "Time zones"];
check(PAGES.length === titles.length, "there are five example pages");
PAGES.forEach((page, i) => {
  check(page.title === titles[i], `page ${i + 1} is ${titles[i]}`);
  check(folder.children[i].id === `ex${i + 1}`, "example pages stay in reading order");
  check(lib.texts[`ex${i + 1}`] === page.text, `${page.title} text is copied once`);
  check(lib.handles[`ex${i + 1}`] === allocateHandle(page.title, new Set()), `${page.title} handle matches its title`);
});
check(lib.handles.ex1 === "trip", "Trip Budget is @trip");
check(lib.handles.ex2 === "groceries", "Groceries is @groceries");
check(lib.texts.ex3.includes("@trip.total"), "monthly budget reads @trip.total");
check(lib.texts.ex3.includes("@groceries.total"), "monthly budget reads @groceries.total");
check(lib.texts.ex1.includes("total = sum"), "the trip total is an assignment other pages can read");

const again = freshLibrary(welcome);
check(JSON.stringify(again) === JSON.stringify(lib), "the seeded library is the same every time");

check(!/function allocateHandle\(/.test(appJs), "the web app uses the shared handle allocator");
check(appJs.includes('from "./seed.js"'), "the web app imports the seed");
check(appJs.includes("installLibrary(freshLibrary(WELCOME))"), "a first launch installs the examples");
check(appJs.includes("seeded: librarySeeded"), "the flag is saved with the pages");
check(sw.includes('"seed.js"') && sw.includes('"examples.js"'), "the example pages are in the offline shell");
const buildWeb = readFileSync(join(root, "../.github/scripts/build-web.sh"), "utf8");
check(buildWeb.includes("site/app/seed.js") && buildWeb.includes("site/app/examples.js"), "the web deploy copies the files the app imports");
const markers = examples.split("text: `").length - 1;
check(markers === PAGES.length, "the desktop reader finds one text block per page");
check(PAGES.every((page) => !page.text.includes("`")), "page text has no backtick the desktop reader would stop on");

if (failures.length) {
  console.error(failures.map((f) => "  - " + f).join("\n"));
  process.exit(1);
}
console.log("seed checks ok");
