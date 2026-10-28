'use strict';
const { contextBridge, ipcRenderer } = require('electron');
// The only thing the page can ask of the desktop: show a notification.
// No file or process access.
contextBridge.exposeInMainWorld('magnusDesktop', {
  notify: (title, body) => ipcRenderer.send('notify', { title, body }),
});
