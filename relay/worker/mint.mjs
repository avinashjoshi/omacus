// Forwards to admin.mjs. The code is minted on the server, not on this machine.
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(new URL("./admin.mjs", import.meta.url));
const child = spawn(process.execPath, [script, "mint", ...process.argv.slice(2)], { stdio: "inherit" });
child.on("exit", (code) => process.exit(code ?? 1));
