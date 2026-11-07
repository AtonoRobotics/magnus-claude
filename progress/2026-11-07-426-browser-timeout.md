# Browser navigation budgets (#426)

`browser/sidecar.js` goto 45s to 90s; `browser.rs` request wrapper
90s to 120s with a cross-referencing comment. `node --check`
passes; `cargo check` clean. The browser suite cannot run in this
sandbox (no `node_modules` until installed per CI, and chromium
never answers here — identical failure on the untouched base), so
CI is the gate for the previously-flaking test.
