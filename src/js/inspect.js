// Outlook Lite: diagnostik struktur halaman Outlook web.
// Hanya mengumpulkan STRUKTUR (atribut, pola label folder, angka di folder Inbox).
// Isi email (pengirim, subjek, isi) TIDAK diambil; label baris email disamarkan.
// Bisa dijalankan dari menu tray aplikasi, atau ditempel di Console DevTools browser.
(() => {
  const INBOX = /^(inbox|kotak masuk|kotak surat masuk)\b/i;
  const KEEP = new Set(['inbox', 'kotak', 'masuk', 'surat', 'unread', 'belum', 'dibaca', 'read', 'item', 'items',
    'selected', 'dipilih', 'folder', 'drafts', 'draf', 'sent', 'terkirim', 'deleted', 'dihapus', 'junk', 'sampah',
    'archive', 'arsip', 'favorites', 'favorit', 'flagged', 'ditandai', 'pinned', 'disematkan', 'has', 'attachment',
    'lampiran', 'meeting', 'rapat', 'reminder', 'pengingat', 'focused', 'other', 'lainnya', 'difokuskan']);

  const mask = (s, keepDigits) => String(s || '').split(/(\s+|[,.;:()\-])/).map((w) => {
    if (/^\s+$/.test(w) || /^[,.;:()\-]$/.test(w)) return w;
    if (keepDigits && /^\d+$/.test(w)) return w;
    return KEEP.has(w.toLowerCase()) ? w : '•';
  }).join('');

  const attrs = (el) => {
    const o = {};
    for (const a of Array.from(el.attributes || [])) {
      if (a.name === 'class' || a.name === 'style') continue;
      if (a.name.startsWith('aria-label') || a.name === 'title') continue;
      o[a.name] = String(a.value).slice(0, 60);
    }
    return o;
  };

  const selectorFor = (el, stop) => {
    const parts = [];
    let cur = el;
    for (let i = 0; cur && cur !== stop && i < 5; i++) {
      let p = cur.tagName.toLowerCase();
      for (const a of Array.from(cur.attributes || [])) {
        if (/^data-|^role$|^aria-hidden$/.test(a.name) && a.value && a.value.length < 40 && !/\d{4,}/.test(a.value)) {
          p += `[${a.name}="${a.value.replace(/"/g, '')}"]`;
        }
      }
      parts.unshift(p);
      cur = cur.parentElement;
    }
    return parts.join(' > ');
  };

  const r = {
    v: 1,
    when: new Date().toISOString(),
    host: location.hostname,
    path: location.pathname,
    title: document.title,
    language: document.documentElement.lang || navigator.language,
    shimActive: !!window.__olite,
    notification: {
      type: typeof window.Notification,
      permission: (window.Notification && window.Notification.permission) || null,
    },
    unreadGuess: typeof window.__oliteUnread === 'function' ? window.__oliteUnread() : null,
    composeEditorPresent: !!document.querySelector('[contenteditable="true"][role="textbox"]'),
  };

  const tree = Array.from(document.querySelectorAll('[role="treeitem"]'));
  r.treeitemCount = tree.length;
  r.folderLabelPatterns = tree.slice(0, 15).map((it) => mask(it.getAttribute('aria-label') || it.getAttribute('title') || '', true));
  r.inboxCandidates = [];
  for (const it of tree) {
    const label = (it.getAttribute('aria-label') || it.getAttribute('title') || '').trim();
    const text = (it.innerText || it.textContent || '').trim();
    const first = (text.split('\n')[0] || '').trim();
    if (!INBOX.test(first) && !INBOX.test(label)) continue;
    const digitEls = Array.from(it.querySelectorAll('*'))
      .filter((e) => e.children.length === 0 && /^\s*\d{1,6}\s*$/.test(e.textContent || ''));
    r.inboxCandidates.push({
      label,
      lines: text.split('\n').map((s) => s.trim()).filter(Boolean).slice(0, 6),
      attrs: attrs(it),
      digitElements: digitEls.slice(0, 3).map((e) => ({
        tag: e.tagName.toLowerCase(),
        attrs: attrs(e),
        text: (e.textContent || '').trim(),
        selector: selectorFor(e, null),
      })),
    });
    if (r.inboxCandidates.length >= 4) break;
  }

  const rows = Array.from(document.querySelectorAll('[role="option"]'));
  r.messageRowCount = rows.length;
  r.messageRowSample = rows.slice(0, 3).map((row) => ({
    attrNames: Array.from(row.attributes || []).map((a) => a.name),
    labelPattern: mask(row.getAttribute('aria-label') || '', false).slice(0, 200),
  }));

  const json = JSON.stringify(r, null, 2);
  if (window.ipc && window.ipc.postMessage) {
    window.ipc.postMessage(JSON.stringify({ t: 'inspect', data: json }));
  } else {
    console.log(json);
  }
  return json;
})();
