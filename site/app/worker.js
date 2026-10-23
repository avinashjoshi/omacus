// The calculation for app.omacus.com, off the page's thread: the engine runs here,
// so a slow line (100000!) or a long sheet never stops typing or scrolling. The page
// (app.js) sends the newest text and draws the answers as they come back.
//
// Work is done in slices of about SLICE_MS, with a pause between them for messages:
// newer text replaces the old (the lines worked out so far are kept, so nothing is
// redone), and answers so far go back after every slice, so a long sheet fills in
// from the top even while typing carries on. Copies wait their turn here too.

import init, { Calc } from "./pkg/omacus_web.js";

const SLICE_MS = 50;

let calc;
let started = null; // resolves once the engine is ready
let latest = null; // { seq, sheet, text }: the newest text, until it's all worked out
const copies = []; // { seq, sheet, text } in the order asked
let running = false;

self.onmessage = (e) => {
  const m = e.data;
  if (m.type === "init") {
    started = init({ module_or_path: m.wasm }).then(() => {
      calc = new Calc();
      if (m.rates) calc.setRates(m.rates);
      self.postMessage({ type: "ready" });
    });
    started.catch((err) => self.postMessage({ type: "error", message: String(err) }));
  } else if (m.type === "rates") {
    started?.then(() => calc.setRates(m.rates));
  } else if (m.type === "sheet") {
    latest = m;
  } else if (m.type === "copy") {
    copies.push(m);
  }
  run();
};

async function run() {
  if (running || !started) return;
  running = true;
  try {
    await started;
    while (copies.length || latest) {
      if (copies.length) {
        // The person is waiting on the clipboard: copies first, each starting from the
        // sheet's saved work.
        const job = copies[0];
        const id = `copy:${job.seq}`;
        if (!job.forked) {
          calc.forkSheet(job.sheet, id);
          job.forked = true;
        }
        const pages = JSON.stringify(job.pages || []);
        const out = JSON.parse(
          job.pages
            ? calc.evaluateSheetForWith(id, job.handle || "", job.text, pages, SLICE_MS)
            : calc.evaluateSheetFor(id, job.text, SLICE_MS),
        );
        if (out.done) {
          copies.shift();
          calc.dropSheet(id);
          self.postMessage({ type: "copy", seq: job.seq, results: JSON.stringify(out.results) });
        } else {
          self.postMessage({ type: "working" }); // still here: the page replaces a silent worker
        }
      } else {
        const job = latest;
        const pages = JSON.stringify(job.pages || []);
        const out = JSON.parse(
          job.pages
            ? calc.evaluateSheetForWith(job.sheet, job.handle || "", job.text, pages, SLICE_MS)
            : calc.evaluateSheetFor(job.sheet, job.text, SLICE_MS),
        );
        if (out.done && latest === job) latest = null;
        self.postMessage({
          type: "sheet", seq: job.seq, sheet: job.sheet, text: job.text, done: out.done, results: out.results,
        });
      }
      // Let newer messages in.
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
  } catch (err) {
    self.postMessage({ type: "error", message: String(err) });
  } finally {
    running = false;
  }
}
