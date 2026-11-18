// Outlook Lite: skrip pemantau yang disuntik ke halaman Outlook web.
// - Mengganti Notification API supaya notifikasi bawaan Outlook (email baru,
//   pengingat rapat) diteruskan ke aplikasi dan ditampilkan sebagai toast Windows.
// - Membaca jumlah email belum dibaca di Inbox dan status sedang menulis email.
// Tidak mengirim apa pun ke luar; hanya ke aplikasi lewat window.ipc.
(() => {
  'use strict';
  if (window.__olite) return;
  const host = location.hostname.toLowerCase();
  if (!/^outlook\.(office|office365|live)\.com$|^outlook\.cloud\.microsoft$/.test(host)) return;
  window.__olite = true;

  const cfg = window.__oliteCfg || {};
  const post = (o) => {
    try { window.ipc.postMessage(JSON.stringify(o)); } catch (e) { /* belum siap */ }
  };

  // ---------- Hemat RAM: tanpa service worker ----------
  // Service worker Outlook berjalan di proses renderer sendiri (~30-45 MB) hanya untuk
  // cache/offline. Notifikasi tidak memakainya (diteruskan lewat shim di bawah).
  if (cfg.nosw && navigator.serviceWorker) {
    try {
      navigator.serviceWorker.getRegistrations()
        .then((rs) => rs.forEach((r) => r.unregister()))
        .catch(() => {});
      const blocked = () => Promise.reject(
        new DOMException('Service worker dimatikan oleh Outlook Lite', 'SecurityError'));
      Object.defineProperty(ServiceWorkerContainer.prototype, 'register', {
        value: blocked, configurable: true, writable: true,
      });
    } catch (e) { /* abaikan */ }
  }

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

  // ---------- Badge API (sumber tanda merah di taskbar pada PWA) ----------
  try {
    const sendBadge = (n) => {
      const c = Number(n);
      post({ t: 'badge', count: Number.isFinite(c) && c >= 0 ? Math.floor(c) : 0 });
      return Promise.resolve();
    };
    navigator.setAppBadge = (n) => sendBadge(n === undefined ? 1 : n);
    navigator.clearAppBadge = () => sendBadge(0);
  } catch (e) {}

  // ---------- Suara Outlook (email masuk / pengingat) ----------
  let lastSound = 0;
  const onSound = (src) => {
    const now = Date.now();
    if (now - lastSound < 1500) return; // satu suara bisa memicu beberapa panggilan
    lastSound = now;
    post({ t: 'sound', src: String(src || '').split('?')[0].slice(-80) });
  };
  try {
    const origPlay = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function () {
      try {
        const isAudio = this instanceof HTMLAudioElement || !this.videoWidth;
        if (isAudio && !this.muted && this.volume > 0) onSound(this.currentSrc || this.src);
      } catch (e) {}
      return origPlay.apply(this, arguments);
    };
  } catch (e) {}
  try {
    if (window.AudioBufferSourceNode) {
      const origStart = AudioBufferSourceNode.prototype.start;
      AudioBufferSourceNode.prototype.start = function () {
        try {
          // abaikan "buka kunci audio" (buffer sangat pendek/kosong)
          if (this.buffer && this.buffer.duration > 0.15) onSound('webaudio');
        } catch (e) {}
        return origStart.apply(this, arguments);
      };
    }
  } catch (e) {}

  // ---------- Jumlah belum dibaca ----------
  // Sumber utama: angka di folder Kotak Masuk/Inbox pada panel folder (sama dengan yang
  // Anda lihat di Outlook). Badge aplikasi Outlook BUKAN total belum dibaca (hanya email
  // baru sejak terakhir dilihat), jadi hanya dipakai aplikasi sebagai sinyal email masuk.
  const INBOX = /^(inbox|kotak masuk|kotak surat masuk)(?![a-z])/i;

  const leaves = (el) => {
    const out = [];
    for (const x of el.querySelectorAll('span,div,i,b')) {
      if (x.children.length === 0) {
        const t = (x.textContent || '').trim();
        if (t) out.push(t);
      }
    }
    return out;
  };

  function readInbox(it) {
    const label = (it.getAttribute('aria-label') || it.getAttribute('title') || '').trim();
    const texts = leaves(it);
    const name = texts.find((t) => !/^\d+$/.test(t)) || (it.textContent || '').trim();
    // data-folder-name="inbox" tidak tergantung bahasa tampilan Outlook.
    const isInbox = (it.getAttribute('data-folder-name') || '').toLowerCase() === 'inbox';
    if (!isInbox && !INBOX.test(name) && !INBOX.test(label)) return null;
    // "Inbox, 5 unread", "Kotak Masuk 5 item belum dibaca", "Inbox unread 5"
    let m = label.match(/(\d+)\s*(?:items?\s*|item\s*)?(?:unread|belum)/i)
      || label.match(/(?:unread|belum dibaca)\D{0,12}(\d+)/i);
    if (m) return { n: +m[1], how: 'label' };
    const nums = texts.filter((t) => /^\d{1,6}$/.test(t));
    if (nums.length) return { n: +nums[0], how: 'angka' };
    // Abaikan jumlah total ("1,172 items", "50 item") supaya tidak terbaca sebagai belum dibaca.
    m = label.replace(INBOX, '').replace(/\d[\d.,]*\s*(items?|elemen|elemente)\b/gi, '')
      .match(/(\d{1,6})/);
    if (m) return { n: +m[1], how: 'label-angka' };
    return { n: 0, how: 'tanpa-angka' }; // Outlook tidak menampilkan angka bila 0
  }

  function readUnread() {
    if (cfg.sel) {
      try {
        const el = document.querySelector(cfg.sel);
        if (el) {
          const m = (el.textContent || '').match(/\d+/);
          return { n: m ? +m[0] : 0, how: 'selector' };
        }
      } catch (e) {}
    }
    let best = null;
    for (const it of document.querySelectorAll('[role="treeitem"]')) {
      const r = readInbox(it);
      if (!r) continue;
      if (!best || (r.how === 'label' && best.how !== 'label') || r.n > best.n) best = r;
      if (best.how === 'label') break;
    }
    if (best) return best;
    const mt = (document.title || '').match(/\((\d+)\)/);
    if (mt) return { n: +mt[1], how: 'judul' };
    // Panel folder tidak tampil (mis. jendela sempit) atau halaman belum siap.
    return { n: -1, how: 'tak-terbaca' };
  }
  window.__oliteUnread = () => readUnread().n;

  // Eksperimen: klik lampiran Office = unduh lalu dibuka di aplikasi desktop (bukan pratinjau
  // web yang berat). Lewat menu "Unduh" milik Outlook sendiri; bila menu tidak ditemukan,
  // klik diteruskan seperti biasa (pratinjau).
  if (cfg.att && Array.isArray(cfg.att.types) && cfg.att.types.length) {
    const types = new Set(cfg.att.types.map((t) => String(t).toLowerCase()));
    const min = +cfg.att.min || 0;
    const nameRe = /([^\\/:*?"<>|\n,]{1,200}\.([a-z0-9]{2,5}))(?![a-z0-9])/i;
    const sizeRe = /(\d+(?:[.,]\d+)?)\s*(KB|MB|GB|bytes?|B)\b/i;
    const toMb = (txt) => {
      const m = sizeRe.exec(txt || '');
      if (!m) return null;
      const v = parseFloat(m[1].replace(',', '.'));
      const u = m[2].toUpperCase();
      return u === 'GB' ? v * 1024 : u === 'MB' ? v : u === 'KB' ? v / 1024 : v / 1048576;
    };
    const menuBtnSel = 'button[aria-haspopup], [role="button"][aria-haspopup]';
    const labelOf = (el) => (el.getAttribute('aria-label') || '') + ' ' + (el.getAttribute('title') || '') + ' ' + (el.textContent || '').slice(0, 300);
    // Cari kartu lampiran: leluhur terdekat (maks. 6 tingkat) yang menyebut nama file berjenis
    // terpilih dan punya tombol menu ("Opsi lainnya").
    const cardOf = (el) => {
      for (let n = el, d = 0; n && n !== document.body && d < 7; n = n.parentElement, d++) {
        const m = nameRe.exec(labelOf(n));
        if (m && types.has(m[2].toLowerCase()) && n.querySelector(menuBtnSel)) {
          return { card: n, name: m[1].trim(), ext: m[2].toLowerCase() };
        }
      }
      return null;
    };
    const wait = (ms) => new Promise((r) => setTimeout(r, ms));
    let bypass = false;
    window.addEventListener('click', (ev) => {
      if (bypass || ev.button !== 0 || !ev.isTrusted) return;
      const t = ev.target instanceof Element ? ev.target : null;
      if (!t || t.closest(menuBtnSel) || t.closest('[role="menu"],[role="menuitem"],a[href]')) return;
      if (t.closest('[contenteditable="true"]')) return;
      const hit = cardOf(t);
      if (!hit) return;
      const mb = toMb(labelOf(hit.card));
      if (min > 0 && (mb === null || mb < min)) return; // kecil / tak diketahui: pratinjau biasa
      ev.preventDefault();
      ev.stopImmediatePropagation();
      (async () => {
        const btn = hit.card.querySelector(menuBtnSel);
        btn.click();
        let item = null;
        for (let i = 0; i < 20 && !item; i++) {
          await wait(75);
          item = [...document.querySelectorAll('[role="menuitem"]')].find((m) =>
            /^\s*(download|unduh)/i.test(m.textContent || m.getAttribute('aria-label') || ''));
        }
        if (item) {
          post({ t: 'attach', name: hit.name, mb });
          item.click();
        } else {
          document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
          bypass = true;
          try { t.click(); } finally { bypass = false; }
        }
      })();
    }, true);
  }

  let last = null;
  let lastSent = 0;
  let prevRaw = null;
  let stable = -1;
  let how = '';
  const start = Date.now();
  function tick(force) {
    let composing = false;
    try {
      // Angka dianggap pasti setelah sama dua kali berturut-turut (3 detik),
      // supaya angka sementara saat halaman baru dimuat tidak memicu notifikasi palsu.
      // force = permintaan aplikasi (email masuk / Cek email sekarang): pakai angka saat ini.
      const r = readUnread();
      if (force || r.n === prevRaw) { stable = r.n; how = r.how; }
      prevRaw = r.n;
      composing = !!document.querySelector('[contenteditable="true"][role="textbox"]');
    } catch (e) {}
    const unread = stable;
    const now = Date.now();
    const key = unread + '|' + composing + '|' + how;
    if (force || key !== last || now - lastSent > 60000 || now - start < 30000) {
      last = key;
      lastSent = now;
      post({ t: 'state', unread, composing, how, force: !!force });
    }
  }
  // Dipanggil aplikasi saat email masuk dan "Cek email sekarang": baca ulang segera.
  window.__oliteForce = () => tick(true);
  // Saat tersembunyi (tray/minimize) panel folder cukup dibaca tiap 9 detik: email masuk tetap
  // terdeteksi seketika lewat suara/badge/notifikasi, dan aplikasi lalu meminta baca ulang.
  let n = 0;
  setInterval(() => {
    n++;
    if (document.hidden && n % 3 !== 0) return;
    tick(false);
  }, 3000);
  document.addEventListener('DOMContentLoaded', () => setTimeout(() => tick(false), 500));
})();
