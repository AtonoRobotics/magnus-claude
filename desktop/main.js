// Magnus desktop app. A native window onto the appliance's local web app,
// plus what a browser tab cannot do: OS notifications for new inbox items.
'use strict';
const { app, BrowserWindow, Notification, ipcMain, shell } = require('electron');
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

app.whenReady().then(create);
app.on('window-all-closed', () => app.quit());
