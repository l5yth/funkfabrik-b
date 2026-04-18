// Copyright (c) 2006-2026 afri & veit
// SPDX-License-Identifier: Apache-2.0

/* ============================================================
   FUNKFABRIK*B — Teletext interactions (vanilla JS, no deps)
   ============================================================ */

'use strict';

/* ── Live clock with blinking colons ── */
(function () {
  const el = document.getElementById('clock');
  if (!el) return;

  function pad(n) { return String(n).padStart(2, '0'); }

  function tick() {
    const now = new Date();
    const h = pad(now.getHours());
    const m = pad(now.getMinutes());
    const s = pad(now.getSeconds());
    // blink colons on odd seconds
    const sep = now.getSeconds() % 2 === 0
      ? '<span class="colon">:</span>'
      : '<span class="colon" style="visibility:hidden">:</span>';
    el.innerHTML = h + sep + m + sep + s;
  }

  tick();
  setInterval(tick, 1000);
})();

/* ── Arrow-key content scrolling ── */
(function () {
  const viewport = document.getElementById('viewport');
  const scroller = document.getElementById('content-scroll');
  if (!viewport || !scroller) return;
  // Games capture arrow keys themselves; yield to them entirely.
  if (document.getElementById('game-canvas')) return;

  let offset = 0;
  const STEP = 300; // pixels per keypress

  function clamp(val, min, max) { return Math.min(Math.max(val, min), max); }

  function scroll(delta) {
    const maxOffset = Math.max(0, scroller.scrollHeight - viewport.clientHeight);
    offset = clamp(offset + delta, 0, maxOffset);
    scroller.style.transform = 'translateY(-' + offset + 'px)';
  }

  // Reset scroll when page loads
  scroller.style.transform = 'translateY(0)';

  window.addEventListener('keydown', function (e) {
    if (e.key === 'ArrowDown') { scroll(+STEP); e.preventDefault(); }
    if (e.key === 'ArrowUp')   { scroll(-STEP); e.preventDefault(); }
  });
})();

/* ── Remote-control digit input ── */
(function () {
  const remote = document.getElementById('remote');
  if (!remote) return;

  let digits = '';
  let timer = null;

  const KNOWN_PAGES = ['100', '101', '170', '300', '404', '666', '777', '999'];

  function render() {
    // Show typed digits + underscores for remaining slots
    remote.textContent = (digits + '___').slice(0, 3);
    remote.classList.add('visible');
  }

  function clear() {
    digits = '';
    remote.classList.remove('visible');
    clearTimeout(timer);
  }

  function navigate() {
    const page = digits;
    clear();
    // Brief flash before navigating
    remote.textContent = page;
    remote.classList.add('visible');
    setTimeout(function () {
      window.location.href = '/' + page;
    }, 250);
  }

  window.addEventListener('keydown', function (e) {
    // Ignore if user is typing in a form field
    const tag = (e.target.tagName || '').toLowerCase();
    if (tag === 'input' || tag === 'textarea') return;

    if (e.key >= '0' && e.key <= '9') {
      e.preventDefault();
      clearTimeout(timer);
      digits += e.key;
      render();

      if (digits.length >= 3) {
        navigate();
        return;
      }

      // Auto-clear after 3 s of inactivity
      timer = setTimeout(clear, 3000);
    }

    if (e.key === 'Escape') {
      clear();
    }
  });
})();
