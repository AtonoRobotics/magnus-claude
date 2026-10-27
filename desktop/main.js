// Magnus desktop app. A native window onto the appliance's local web app,
// plus what a browser tab cannot do: OS notifications for new inbox items,
// and catching the ChatGPT sign-in redirect on localhost:1455.
'use strict';
const { app, BrowserWindow, Notification, ipcMain, shell } = require('electron');
const http = require('http');
const path = require('path');
const fs = require('fs');

const configPath = path.join(app.getPath('userData'), 'config.json');
function config() {
  try { return JSON.parse(fs.readFileSync(configPath, 'utf8')); } catch { return {}; }
}
const applianceUrl = process.env.MAGNUS_URL || config().url || 'http://127.0.0.1:7700/';

let win;
function create() {
  win = new BrowserWindow({
    width: 1200, height: 860, title: 'Magnus',
    webPreferences: { preload: path.join(__dirname, 'preload.js'), contextIsolation: true, sandbox: true, nodeIntegration: false },
  });
  win.loadURL(applianceUrl);
  // Self-test for CI: load, capture the window, quit.
  if (process.env.MAGNUS_DESKTOP_SELFTEST) {
    win.webContents.once('did-finish-load', () => setTimeout(async () => {
      const img = await win.webContents.capturePage();
      fs.writeFileSync(process.env.MAGNUS_DESKTOP_SELFTEST, img.toPNG());
      app.quit();
    }, 1500));
  }
  // Links (sign-in pages, task links) open in the person's own browser.
  win.webContents.setWindowOpenHandler(({ url }) => { shell.openExternal(url); return { action: 'deny' }; });
  win.webContents.on('will-navigate', (e, url) => { if (!url.startsWith(applianceUrl)) { e.preventDefault(); shell.openExternal(url); } });
}

ipcMain.on('notify', (_e, { title, body }) => {
  if (!Notification.isSupported()) return;
  const n = new Notification({ title: String(title).slice(0, 120), body: String(body).slice(0, 300) });
  n.on('click', () => { if (win) { win.show(); win.focus(); } });
  n.show();
});

// The ChatGPT sign-in (used for Codex) returns to http://localhost:1455/auth/callback.
// Catch it here and hand the address to the app, so nobody has to paste it.
function oauthCatcher() {
  const server = http.createServer((req, res) => {
    if (!req.url.startsWith('/auth/callback')) { res.writeHead(404); res.end(); return; }
    const full = 'http://localhost:1455' + req.url;
    if (win) { win.webContents.send('oauth-callback', full); win.show(); win.focus(); }
    res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
    res.end('<html><body style="font-family:sans-serif;padding:40px"><h2>Signed in.</h2><p>You can close this tab and return to Magnus.</p></body></html>');
  });
  server.on('error', () => {}); // port taken: the person can still paste the address
  server.listen(1455, '127.0.0.1');
}

app.whenReady().then(() => { create(); oauthCatcher(); });
app.on('window-all-closed', () => app.quit());
