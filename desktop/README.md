# Magnus desktop app

A client of the appliance for Linux and Windows. It signs in as the person, keeps only their session (in the OS keyring), and is where people make every decision the agent needs: approvals never pass through the agent's context. It holds no provider or tool credentials. Keys typed in the Models view go straight to the appliance's vault.

The appliance serves the same frontend (`ui/`) to browsers.

## Layout

- `ui/`: the frontend, plain JavaScript with no build step.
  - Operator view (`app.js`): Home, Inbox, New task, Tasks, Task detail, Models.
  - Developer view (`dev.js`): Runs and the run inspector, Experiments, System, Policy, Playbook. It needs the `developer` role.
  - `compose.js` draws composed outcomes inside a frame that cannot run scripts.
  - `bridge.js` is the only part that differs between the app and a browser.
- `src-tauri/`: the Rust side.
  - `pin.rs`: certificate pinning.
  - `store.rs`: settings and the session.
  - `speech.rs`: read-aloud.
  - `lib.rs`: the commands the page may call.
- `tests/flow.mjs`: the operator flow in a browser.
- `tests/app.mjs`: the same flow in the built app, through tauri-driver.

## Build

You need `cargo-tauri` (`cargo install tauri-cli --version ^2`). Run these in `desktop/src-tauri`.

Linux (needs the WebKitGTK 4.1 development packages):

```
cargo tauri build --bundles deb,appimage
```

This writes `target/release/bundle/deb/Magnus_<version>_amd64.deb` and `target/release/bundle/appimage/Magnus_<version>_amd64.AppImage`.

Windows, cross-built on Linux. It needs `rustup target add x86_64-pc-windows-gnu`, mingw-w64 and NSIS:

```
cargo tauri build --runner cargo --target x86_64-pc-windows-gnu --bundles nsis
```

This writes `target/x86_64-pc-windows-gnu/release/bundle/nsis/Magnus_<version>_x64-setup.exe`. The installer installs for the current user, and brings the WebView2 runtime if the machine lacks it.

## Before the first sign-in (owner, on the appliance)

1. Turn on HTTPS: set `appliance.tls_listen` (for example `"0.0.0.0:7700"`) and restart. The appliance prints its certificate's SHA-256 fingerprint at start. People compare it once, when the app first connects.
2. Give each person a password:

   ```
   magnus password --config magnus.toml --person <id>
   ```

   The password is read from standard input. Passwords are refused over plain HTTP except from the appliance itself.
3. Optional: add the `developer` role to anyone who should see the developer view.

## The recorded run

On each platform:

1. Install the app and open it.
2. Type the appliance's address (for example `https://192.168.0.50:7700`). Check that the fingerprint matches the one the appliance printed, then choose **They match**.
3. Sign in with your name and password.
4. Go to **Models**. On the Claude adapter choose **Connect**, paste the API key, then **Save to the vault**. Pick the model and choose **Use this model**.
5. In **New task**, type the task in plain language and choose **Start**.
6. In **Inbox**, the card under **Needs you now** shows exactly what will happen. Choose **Approve once**. If a review card follows, accept it once you have checked the work.
7. Open the task. **Outcome** shows the result, the delivered files and the evidence for each criterion. **Full trace** shows every step.

## Tests

- `cargo test -p magnus-scratch --test desktop_flow`: the frontend in Chromium against a real appliance.
- `cargo test -p magnus-scratch --test desktop_app -- --ignored`: the built Linux app.
  - It needs `tauri-driver` (`cargo install tauri-driver --locked`), `WebKitWebDriver` (package `webkit2gtk-driver`, or set `MAGNUS_WEBKIT_DRIVER`) and `xvfb-run`.
  - Set `MAGNUS_SHOTS=<dir>` to keep screenshots.
- `cargo test` in `src-tauri`: what the page may call, and address handling.
