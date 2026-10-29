// Runs the installed-app scripts on this machine against an appliance on
// another (`the_windows_app_passes_against_this_appliance` in
// tests/scratch/tests/desktop_app.rs): gets the run's details from that
// machine, starts each script on a fresh app (its settings folder removed),
// and reports each result back. Exits non-zero if any script failed.
//
// MAGNUS_COORDINATOR  where the appliance machine listens (http://host:port)
// MAGNUS_WEBDRIVER    tauri-driver on this machine
// MAGNUS_APP          the installed app
// MAGNUS_APP_DATA     the app's settings folder, removed before each script
// MAGNUS_SHOTS        optional: where to keep screenshots
import { spawn } from "node:child_process";
import { rmSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { env } from "./appdriver.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const [coordinator, wd, app, appData] = ["MAGNUS_COORDINATOR", "MAGNUS_WEBDRIVER", "MAGNUS_APP", "MAGNUS_APP_DATA"].map(env);
const scripts = ["app.mjs", "app-library.mjs"];

// The appliance machine may still be starting; the CI job's time limit
// bounds the wait.
let run;
for (;;) {
  try { const r = await fetch(`${coordinator}/run`); if (r.ok) { run = await r.json(); break; } } catch { /* not up yet */ }
  await new Promise(r => setTimeout(r, 2000));
}

const node = script => new Promise(done => {
  const p = spawn(process.execPath, [join(here, script)], { env: { ...process.env, ...run, MAGNUS_WEBDRIVER: wd, MAGNUS_APP: app } });
  let log = "";
  p.stdout.on("data", b => { log += b; process.stdout.write(b); });
  p.stderr.on("data", b => { log += b; process.stderr.write(b); });
  p.on("close", code => done({ ok: code === 0, log }));
});

let failed = 0;
for (const [i, script] of scripts.entries()) {
  rmSync(appData, { recursive: true, force: true });
  const { ok, log } = await node(script);
  if (!ok) failed++;
  await fetch(`${coordinator}/result`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ script, ok, log, last: i === scripts.length - 1 }) });
}
process.exit(failed ? 1 : 0);
