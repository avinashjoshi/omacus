// The calculation for omacus.com/app, off the page's thread: the engine runs here,
// so a slow line (100000!) or a long sheet never stops typing or scrolling. The page
// (app.js) sends the newest text and draws the answers when they come back.

import init, { Calc } from "./pkg/omacus_web.js";

let calc;

// One message at a time, in order: the engine starts before anything uses it.
let queue = Promise.resolve();
self.onmessage = (e) => {
  queue = queue.then(() => handle(e.data)).catch((err) => self.postMessage({ type: "error", message: String(err) }));
};

async function handle(m) {
  if (m.type === "init") {
    await init({ module_or_path: m.wasm });
    calc = new Calc();
    if (m.rates) calc.setRates(m.rates);
    self.postMessage({ type: "ready" });
  } else if (m.type === "rates") {
    calc.setRates(m.rates);
  } else if (m.type === "sheet") {
    // Reuses the sheet's last calculation from the first changed line on.
    const results = calc.evaluateSheet(m.sheet, m.text);
    self.postMessage({ type: "sheet", seq: m.seq, sheet: m.sheet, text: m.text, results });
  } else if (m.type === "layout") {
    const results = calc.evaluate(m.text);
    self.postMessage({ type: "layout", seq: m.seq, text: m.text, results });
  }
}
