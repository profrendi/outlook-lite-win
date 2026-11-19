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

  // Lampiran Office/PDF dibuka di aplikasi desktop (bukan pratinjau web yang berat).
  //  A. "Open/Edit in ... desktop app" (menu lampiran atau tombol di pratinjau) selalu diubah
  //     menjadi: unduh lalu buka di desktop. (Bawaan Outlook justru membuka Office web di sini.)
  //  B. Bila "Klik lampiran langsung buka di desktop" menyala, klik lampiran biasa juga:
  //     1. lewat menu lampiran → "Download";
  //     2. cadangan: bila pratinjau tetap terbuka, klik "Download" di pratinjau lalu tutup.
  // Bila gagal, perilaku bawaan Outlook tetap berjalan dan alasannya dicatat di log.
  if (cfg.att && Array.isArray(cfg.att.types)) {
    const types = new Set(cfg.att.types.map((t) => String(t).toLowerCase()));
    const intercept = !!cfg.att.intercept && types.size > 0;
    const min = +cfg.att.min || 0;
    const nameRe = /([^\\/:*?"<>|\n,]{1,200}\.([a-z0-9]{2,5}))(?![a-z0-9])/i;
    const nameReG = /[^\\/:*?"<>|\n,\s]{1,200}\.(xlsx|xlsm|xlsb|xls|csv|docx|docm|doc|rtf|pptx|pptm|ppsx|ppt|pdf)(?![a-z0-9])/gi;
    const sizeRe = /(\d+(?:[.,]\d+)?)\s*(KB|MB|GB|bytes?|B)\b/i;
    const DL = /^(download|unduh)\b/i;
    const CLOSE = /^(close|tutup|back|kembali)\b/i;
    const DESKTOP = /(desktop app|desktop application|aplikasi desktop)/i;
    const OPEN_IN_APP = /^open in (excel|word|powerpoint|visio)$/i;
    const WEB = /(web|browser|peramban|online)/i;
    const NOT_CARD_BTN = /(reply|forward|balas|teruskan|viva|apps|aplikasi$)/i;
    const PREVIEW_IFRAME = 'iframe[src*="officeapps.live.com"], iframe[src*="officeapps-df.live.com"]';
    const menuBtnSel = 'button[aria-haspopup], [role="button"][aria-haspopup]';
    const clickableSel = 'button,[role="button"],[role="menuitem"],a[role="menuitem"]';
    // Teks tombol tanpa ikon font (karakter Private Use Area) dan spasi tak terlihat: ikon
    // Fluent ditulis sebagai karakter di awal teks, mis. "Download".
    const norm = (t) => String(t || '').replace(/[-​-‏⁠﻿]/g, '').replace(/\s+/g, ' ').trim();
    const attr = (el, k) => (el.getAttribute && el.getAttribute(k)) || '';
    const btnText = (b) => norm(attr(b, 'aria-label')) || norm(attr(b, 'title')) || norm(b.textContent);
    const btnTexts = (b) => [norm(attr(b, 'aria-label')), norm(attr(b, 'title')), norm(b.textContent)].filter(Boolean);
    const isDl = (b) => btnTexts(b).some((t) => DL.test(t));
    const labelOf = (el) => norm(attr(el, 'aria-label') + ' ' + attr(el, 'title') + ' ' + (el.textContent || '').slice(0, 400));
    const toMb = (txt) => {
      const m = sizeRe.exec(txt || '');
      if (!m) return null;
      const v = parseFloat(m[1].replace(',', '.'));
      const u = m[2].toUpperCase();
      return u === 'GB' ? v * 1024 : u === 'MB' ? v : u === 'KB' ? v / 1024 : v / 1048576;
    };
    const alog = (msg) => post({ t: 'attachlog', msg: String(msg).slice(0, 300) });
    // Label tombol untuk diagnostik, tanpa nama file.
    const safeLabels = (els) => els.map(btnText).filter(Boolean)
      .map((x) => x.replace(nameReG, '<file>').slice(0, 30)).slice(0, 14).join(' | ');
    const extFromIframe = (src) =>
      /\/\/excel\./i.test(src) ? 'xlsx' : /\/\/word-(view|edit)\./i.test(src) ? 'docx'
        : /\/\/powerpoint\./i.test(src) ? 'pptx' : '';
    const sizeOk = (mb) => !(min > 0 && (mb === null || mb < min));
    const wait = (ms) => new Promise((r) => setTimeout(r, ms));
    const visible = (el) => !!(el.offsetParent || el.getClientRects().length);
    const isRow = (el) => el.closest('[data-convid],[data-focusable-row],[contenteditable="true"],[role="textbox"]');
    const inMenu = (el) => el.closest('[role="menu"],[role="listbox"][aria-label*="menu" i]');
    const previewIframes = () => [...document.querySelectorAll(PREVIEW_IFRAME)];
    const dlButtons = () => [...document.querySelectorAll(clickableSel)]
      .filter((b) => !inMenu(b) && isDl(b) && visible(b));
    const escape = () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', code: 'Escape', keyCode: 27, bubbles: true }));

    // Kartu lampiran: wadah kecil berisi tepat satu nama file + tombol menu (panah ˅).
    const fileOf = (el) => {
      for (let n = el, d = 0; n && n !== document.body && d < 8; n = n.parentElement, d++) {
        const m = nameRe.exec(labelOf(n));
        if (!m || !types.has(m[2].toLowerCase())) continue;
        let card = null, btn = null;
        for (let c = n, k = 0; c && c !== document.body && k < 5; c = c.parentElement, k++) {
          const txt = norm(c.textContent);
          if (txt.length > 400 || (txt.match(nameReG) || []).length > 1) break; // terlalu besar
          btn = [...c.querySelectorAll(menuBtnSel)].find((b) => !NOT_CARD_BTN.test(btnText(b)));
          if (btn) { card = c; break; }
        }
        return { card: card || n, btn, name: m[1].trim(), ext: m[2].toLowerCase(), mb: toMb(labelOf(card || n)) };
      }
      return null;
    };
    // Tunggu menu yang baru muncul, lalu cari item di dalamnya.
    const findInNewMenu = async (before, test) => {
      for (let i = 0; i < 25; i++) {
        await wait(80);
        const menus = [...document.querySelectorAll('[role="menu"]')].filter((m) => !before.includes(m));
        for (const m of menus.reverse()) {
          const it = [...m.querySelectorAll('[role="menuitem"],[role="menuitemcheckbox"],button')].find(test);
          if (it) return { item: it, menu: m };
        }
      }
      return null;
    };
    const closeNear = (el) => {
      for (let n = el, d = 0; n && n !== document.body && d < 10; n = n.parentElement, d++) {
        const c = [...n.querySelectorAll('button,[role="button"]')].find((b) => btnTexts(b).some((t) => CLOSE.test(t)));
        if (c) { c.click(); return; }
      }
      escape();
    };

    // Pratinjau terbuka → klik "Download" di pratinjau, lalu tutup pratinjau.
    let watching = 0;
    const handled = new WeakSet();
    const watchPreview = (info, beforeIfr, beforeDl, via) => {
      const my = ++watching;
      (async () => {
        for (let i = 0; i < 48 && my === watching; i++) {
          await wait(250);
          const ifr = previewIframes().find((f) => !beforeIfr.includes(f) && !handled.has(f));
          // Tombol Download baru (mis. bilah pratinjau Excel/Word/PDF) setelah lampiran diklik.
          const newDl = info ? dlButtons().find((b) => !beforeDl.includes(b) && !handled.has(b)) : null;
          if (!ifr && !newDl) continue;
          const ext = (info && info.ext) || extFromIframe((ifr && ifr.src) || '');
          if (via !== 'desktop' && (!ext || !types.has(ext))) return;
          const mb = info ? info.mb : null;
          if (via !== 'desktop' && !sizeOk(mb)) return;
          let btn = newDl;
          for (let k = 0; k < 12 && !btn; k++) {
            btn = dlButtons().find((b) => !beforeDl.includes(b) && !handled.has(b) && !(ifr && ifr.contains(b)));
            if (!btn) await wait(250);
          }
          if (ifr) handled.add(ifr);
          if (!btn) {
            alog('pratinjau terbuka tapi tombol Download tidak ditemukan; tombol: '
              + safeLabels([...document.querySelectorAll('button,[role="button"]')].filter(visible).slice(-20)));
            return;
          }
          handled.add(btn);
          post({ t: 'attach', name: (info && info.name) || '', mb, via: via || 'pratinjau' });
          btn.click();
          await wait(500);
          closeNear(btn); // tutup pratinjau supaya Excel/Word web tidak terus memakan memori
          return;
        }
      })();
    };

    let bypass = false;
    const replay = (t) => { bypass = true; try { t.click(); } finally { bypass = false; } };

    window.addEventListener('click', (ev) => {
      if (bypass || ev.button !== 0 || !ev.isTrusted) return;
      const t = ev.target instanceof Element ? ev.target : null;
      if (!t || isRow(t)) return;

      // A. "Edit in Excel desktop app" / "Open in Excel Desktop app" / tombol "Open in Excel".
      const opt = t.closest(clickableSel);
      if (opt) {
        const txts = btnTexts(opt);
        const wantsDesktop = txts.some((x) => (DESKTOP.test(x) || OPEN_IN_APP.test(x)) && !WEB.test(x));
        if (wantsDesktop) {
          const menu = opt.closest('[role="menu"]');
          const sib = menu && [...menu.querySelectorAll('[role="menuitem"],button')].find(isDl);
          const dl = sib || dlButtons().find((b) => b !== opt);
          if (dl) {
            ev.preventDefault();
            ev.stopImmediatePropagation();
            post({ t: 'attach', name: '', mb: null, via: 'desktop' });
            dl.click();
            if (!sib) setTimeout(() => closeNear(dl), 500);
            else escape();
            return;
          }
          alog('"' + txts[0].replace(nameReG, '<file>').slice(0, 40) + '" tanpa tombol Download di dekatnya; dibiarkan ke Outlook');
          return;
        }
      }
      if (!intercept || inMenu(t)) return;

      // B. Klik lampiran biasa.
      const beforeIfr = previewIframes();
      const beforeDl = dlButtons();
      const hit = t.closest(menuBtnSel) ? null : fileOf(t);
      if (!hit) {
        if (min === 0) watchPreview(null, beforeIfr, beforeDl); // pratinjau Office tanpa kartu dikenali
        return;
      }
      if (!sizeOk(hit.mb)) return; // kecil / ukuran tak diketahui: pratinjau biasa
      if (!hit.btn) {
        watchPreview(hit, beforeIfr, beforeDl);
        return;
      }
      ev.preventDefault();
      ev.stopImmediatePropagation();
      (async () => {
        const beforeMenus = [...document.querySelectorAll('[role="menu"]')];
        hit.btn.click();
        const found = await findInNewMenu(beforeMenus, isDl);
        if (found) {
          post({ t: 'attach', name: hit.name, mb: hit.mb, via: 'menu lampiran' });
          found.item.click();
          return;
        }
        const menus = [...document.querySelectorAll('[role="menu"]')].filter((m) => !beforeMenus.includes(m));
        alog('menu lampiran tanpa Download; item: '
          + safeLabels(menus.flatMap((m) => [...m.querySelectorAll('[role="menuitem"]')])));
        escape();
        await wait(120);
        replay(t);
        watchPreview(hit, beforeIfr, beforeDl);
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
