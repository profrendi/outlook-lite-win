// Outlook Lite: skrip pemantau yang disuntik ke halaman Outlook web.
// - Mengganti Notification API supaya notifikasi bawaan Outlook (email baru,
//   pengingat rapat) diteruskan ke aplikasi dan ditampilkan sebagai toast Windows.
// - Membaca jumlah email belum dibaca di Inbox dan status sedang menulis email.
// Tidak mengirim apa pun ke luar; hanya ke aplikasi lewat window.ipc.
(() => {
  'use strict';
  if (window.__olite) return;
  const host = location.hostname.toLowerCase();
  if (!/^outlook\.(office|office365|live)\.com$/.test(host)) return;
  window.__olite = true;

  const cfg = window.__oliteCfg || {};
  const post = (o) => {
    try { window.ipc.postMessage(JSON.stringify(o)); } catch (e) { /* belum siap */ }
  };

  // ---------- Notification shim ----------
  let seq = 0;
  const live = new Map();

  class OLNotification extends EventTarget {
    constructor(title, options) {
      super();
      const o = options || {};
      this.title = String(title == null ? '' : title);
      this.body = String(o.body == null ? '' : o.body);
      this.tag = String(o.tag || '');
      this.data = o.data === undefined ? null : o.data;
      this.icon = o.icon || '';
      this.badge = o.badge || '';
      this.image = o.image || '';
      this.dir = o.dir || 'auto';
      this.lang = o.lang || '';
      this.silent = !!o.silent;
      this.renotify = !!o.renotify;
      this.requireInteraction = !!o.requireInteraction;
      this.timestamp = Date.now();
      this.actions = [];
      this.vibrate = [];
      const id = ++seq;
      this.__id = id;
      live.set(id, this);
      if (live.size > 50) live.delete(live.keys().next().value);
      post({ t: 'notify', id, title: this.title.slice(0, 300), body: this.body.slice(0, 600) });
      setTimeout(() => this.dispatchEvent(new Event('show')), 0);
    }
    close() {
      if (live.delete(this.__id)) this.dispatchEvent(new Event('close'));
    }
    static get permission() { return 'granted'; }
    static requestPermission(cb) {
      try { if (typeof cb === 'function') cb('granted'); } catch (e) {}
      return Promise.resolve('granted');
    }
    static get maxActions() { return 0; }
  }
  for (const n of ['click', 'show', 'close', 'error']) {
    Object.defineProperty(OLNotification.prototype, 'on' + n, {
      configurable: true,
      get() { return this['__on' + n] || null; },
      set(f) {
        const k = '__on' + n;
        if (this[k]) this.removeEventListener(n, this[k]);
        this[k] = typeof f === 'function' ? f : null;
        if (this[k]) this.addEventListener(n, this[k]);
      },
    });
  }
  try {
    Object.defineProperty(window, 'Notification', { value: OLNotification, writable: true, configurable: true });
  } catch (e) {
    window.Notification = OLNotification;
  }

  // Dipanggil aplikasi saat toast diklik: teruskan klik ke Outlook (buka email/rapat terkait).
  window.__oliteClick = (id) => {
    const n = live.get(id);
    if (!n) return;
    try { window.focus(); } catch (e) {}
    n.dispatchEvent(new Event('click', { cancelable: true }));
  };

  try {
    const perms = navigator.permissions;
    if (perms && perms.query) {
      const orig = perms.query.bind(perms);
      perms.query = (d) => (d && d.name === 'notifications')
        ? Promise.resolve({ name: 'notifications', state: 'granted', status: 'granted', onchange: null,
            addEventListener() {}, removeEventListener() {}, dispatchEvent() { return true; } })
        : orig(d);
    }
  } catch (e) {}

  try {
    if (window.ServiceWorkerRegistration) {
      ServiceWorkerRegistration.prototype.showNotification = function (title, options) {
        new OLNotification(title, options);
        return Promise.resolve();
      };
    }
  } catch (e) {}

  // ---------- Jumlah belum dibaca ----------
  const INBOX = /^(inbox|kotak masuk|kotak surat masuk)\b/i;

  function readUnread() {
    if (cfg.sel) {
      try {
        const el = document.querySelector(cfg.sel);
        if (el) {
          const m = (el.textContent || '').match(/\d+/);
          return m ? +m[0] : 0;
        }
      } catch (e) {}
    }
    const items = document.querySelectorAll('[role="treeitem"]');
    for (const it of items) {
      const label = (it.getAttribute('aria-label') || it.getAttribute('title') || '').trim();
      const text = (it.innerText || it.textContent || '').trim();
      const first = (text.split('\n')[0] || '').trim();
      if (!INBOX.test(first) && !INBOX.test(label)) continue;
      const m = label.match(/(\d+)\s*(unread|belum)/i);
      if (m) return +m[1];
      for (const line of text.split('\n')) {
        const s = line.trim();
        if (/^\d{1,6}$/.test(s)) return +s;
      }
      return 0;
    }
    const mt = (document.title || '').match(/\((\d+)\)/);
    if (mt) return +mt[1];
    return -1;
  }
  window.__oliteUnread = readUnread;

  let last = null;
  let lastSent = 0;
  let prevRaw = null;
  let stable = -1;
  const start = Date.now();
  function tick() {
    let composing = false;
    try {
      // Angka dianggap pasti setelah sama dua kali berturut-turut (3 detik),
      // supaya angka sementara saat halaman baru dimuat tidak memicu notifikasi palsu.
      const raw = readUnread();
      if (raw === prevRaw) stable = raw;
      prevRaw = raw;
      composing = !!document.querySelector('[contenteditable="true"][role="textbox"]');
    } catch (e) {}
    const unread = stable;
    const now = Date.now();
    const key = unread + '|' + composing;
    if (key !== last || now - lastSent > 60000 || now - start < 30000) {
      last = key;
      lastSent = now;
      post({ t: 'state', unread, composing });
    }
  }
  setInterval(tick, 3000);
  document.addEventListener('DOMContentLoaded', () => setTimeout(tick, 500));
})();
