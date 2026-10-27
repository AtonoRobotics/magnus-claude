'use strict';
const { contextBridge, ipcRenderer } = require('electron');
// The only things the page can ask of the desktop: show a notification, and
// receive a captured sign-in address. No file or process access.
contextBridge.exposeInMainWorld('magnusDesktop', {
  notify: (title, body) => ipcRenderer.send('notify', { title, body }),
  onOAuthCallback: fn => ipcRenderer.on('oauth-callback', (_e, url) => fn(url)),
});
