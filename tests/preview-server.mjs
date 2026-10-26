// A dropped page resets that connection only. The listener keeps answering
// after the drop is lifted.
import { spawn } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const dir = mkdtempSync(join(tmpdir(), "omacus-preview-"));
const down = join(dir, "down");
const gen = join(dir, "gen");
writeFileSync(gen, "1");
const port = 4317;
const child = spawn(process.execPath, ["tests/e2e/server.mjs"], {
  env: {
    ...process.env,
    OMACUS_PREVIEW_PORT: String(port),
    OMACUS_SW_DOWN: down,
    OMACUS_SW_GEN: gen,
  },
  stdio: ["ignore", "ignore", "pipe"],
});
let stderr = "";
child.stderr.on("data", (buf) => {
  stderr += String(buf);
});

function fail(message) {
  throw new Error(message);
}

async function answer() {
  const deadline = Date.now() + 4000;
  let last = "not started";
  while (Date.now() < deadline) {
    if (child.exitCode != null) fail(`preview server exited ${child.exitCode}: ${stderr}`);
    try {
      const res = await fetch(`http://127.0.0.1:${port}/app/`, { signal: AbortSignal.timeout(2000) });
      if (res.ok) return;
      last = String(res.status);
    } catch (err) {
      last = err instanceof Error ? err.message : String(err);
    }
    await new Promise((resolve) => setTimeout(resolve, 40));
  }
  fail(`preview server did not answer (${last})`);
}

try {
  await answer();
  writeFileSync(down, "1");
  for (let n = 0; n < 3; n += 1) {
    let dropped = false;
    try {
      await fetch(`http://127.0.0.1:${port}/app/`, { signal: AbortSignal.timeout(2000) });
    } catch {
      dropped = true;
    }
    if (!dropped) fail("a dropped origin still answered");
    if (child.exitCode != null) fail(`preview server exited ${child.exitCode}: ${stderr}`);
  }
  unlinkSync(down);
  const again = await fetch(`http://127.0.0.1:${port}/app/`, { signal: AbortSignal.timeout(2000) });
  if (!again.ok) fail(`the preview server did not survive (${again.status})`);
  if (child.exitCode != null) fail(`preview server exited ${child.exitCode}: ${stderr}`);
  console.log("preview server ok");
} finally {
  child.kill();
  rmSync(dir, { recursive: true, force: true });
}
