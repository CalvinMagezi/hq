// Progressive enhancement only. The page reads fine without this file.
(function () {
  var COPIED_MS = 2000;

  function selectText(node) {
    var range = document.createRange();
    range.selectNodeContents(node);
    var sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  }

  function setupCopy() {
    var live = document.getElementById('copy-status');
    document.querySelectorAll('[data-copy]').forEach(function (btn) {
      var target = document.getElementById(btn.getAttribute('data-copy'));
      if (!target) return;
      btn.hidden = false;
      btn.addEventListener('click', function () {
        var text = target.textContent.replace(/\n$/, '');
        var done = function (ok) {
          var label = btn.getAttribute('data-label') || 'Copy';
          btn.textContent = ok ? 'Copied' : 'Press Ctrl+C';
          if (live) live.textContent = ok ? 'Copied to clipboard' : 'Copy failed, text selected';
          setTimeout(function () {
            btn.textContent = label;
            if (live) live.textContent = '';
          }, COPIED_MS);
        };
        if (navigator.clipboard && window.isSecureContext) {
          navigator.clipboard.writeText(text).then(
            function () { done(true); },
            function () { selectText(target); done(false); }
          );
        } else {
          selectText(target);
          done(false);
        }
      });
    });
  }

  // GIFs cannot be paused, so a still poster shows first and the animation
  // is swapped in only when the visitor asks for it.
  function setupAnimations() {
    document.querySelectorAll('[data-anim]').forEach(function (fig) {
      var img = fig.querySelector('img');
      var btn = fig.querySelector('[data-anim-toggle]');
      var controls = fig.querySelector('.media-controls');
      if (!img || !btn) return;
      var poster = img.getAttribute('data-poster');
      var anim = img.getAttribute('data-anim-src');
      var playing = false;
      controls.hidden = false;
      btn.addEventListener('click', function () {
        playing = !playing;
        img.src = playing ? anim : poster;
        btn.textContent = playing ? 'Pause animation' : 'Play animation';
        btn.setAttribute('aria-pressed', playing ? 'true' : 'false');
      });
    });
  }

  // Without this script every panel shows in sequence. With it, a row of
  // toggle buttons shows one panel at a time.
  function setupTabs() {
    document.querySelectorAll('[data-tabs]').forEach(function (root) {
      var list = root.querySelector('[data-tablist]');
      var buttons = Array.prototype.slice.call(root.querySelectorAll('[data-tab]'));
      var panels = Array.prototype.slice.call(root.querySelectorAll('[data-panel]'));
      if (!list || !buttons.length) return;
      list.hidden = false;
      function show(id) {
        buttons.forEach(function (b, i) {
          var on = b.getAttribute('data-tab') === id;
          b.setAttribute('aria-pressed', on ? 'true' : 'false');
          if (on) list.style.setProperty('--i', String(i));
        });
        panels.forEach(function (p) {
          p.hidden = p.getAttribute('data-panel') !== id;
        });
        root.setAttribute('data-active', id);
      }
      function panelById(id) {
        return panels.filter(function (p) { return p.getAttribute('data-panel') === id; })[0];
      }
      buttons.forEach(function (b) {
        b.addEventListener('click', function () {
          var id = b.getAttribute('data-tab');
          show(id);
          // Keep the address in step so a table-of-contents link to this panel fires again later.
          var panel = panelById(id);
          if (panel && panel.id) history.replaceState(null, '', '#' + panel.id);
        });
      });
      // A link such as /install/#windows must land on its panel even though panels start hidden.
      function hashTarget() {
        var raw = location.hash.slice(1);
        try {
          raw = decodeURIComponent(raw);
        } catch (e) {
          /* a malformed hash matches no panel */
        }
        return panels.filter(function (p) { return p.id && p.id === raw; })[0] || null;
      }
      function openFromHash() {
        var target = hashTarget();
        if (!target) return false;
        show(target.getAttribute('data-panel'));
        target.scrollIntoView();
        return true;
      }
      var first = buttons.filter(function (b) { return b.getAttribute('aria-pressed') === 'true'; })[0] || buttons[0];
      if (!openFromHash()) show(first.getAttribute('data-tab'));
      window.addEventListener('hashchange', openFromHash);
    });
  }

  // A row of buttons that sets one attribute on a target, used to focus a table column.
  function setupFocusSwitch() {
    document.querySelectorAll('[data-focus-switch]').forEach(function (list) {
      var target = document.getElementById(list.getAttribute('data-focus-switch'));
      var buttons = Array.prototype.slice.call(list.querySelectorAll('[data-focus]'));
      if (!target || !buttons.length) return;
      list.hidden = false;
      buttons.forEach(function (b, i) {
        b.addEventListener('click', function () {
          buttons.forEach(function (o) { o.setAttribute('aria-pressed', o === b ? 'true' : 'false'); });
          list.style.setProperty('--i', String(i));
          target.setAttribute('data-focus', b.getAttribute('data-focus'));
        });
      });
    });
  }

  // A soft light follows the pointer inside the hero.
  function setupSpotlight() {
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    document.querySelectorAll('[data-spotlight]').forEach(function (el) {
      el.addEventListener('pointermove', function (e) {
        var r = el.getBoundingClientRect();
        el.style.setProperty('--mx', (e.clientX - r.left) + 'px');
        el.style.setProperty('--my', (e.clientY - r.top) + 'px');
      });
    });
  }

  // The scrolling list of agents carries information, so it needs a way to stop (WCAG 2.2.2).
  function setupMarqueePause() {
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    document.querySelectorAll('[data-marquee-pause]').forEach(function (btn) {
      var band = btn.closest('.band');
      if (!band) return;
      btn.hidden = false;
      btn.addEventListener('click', function () {
        var paused = band.classList.toggle('is-paused');
        btn.setAttribute('aria-pressed', paused ? 'true' : 'false');
        btn.textContent = paused ? 'Play' : 'Pause';
      });
    });
  }

  setupCopy();
  setupAnimations();
  setupMarqueePause();
  setupTabs();
  setupFocusSwitch();
  setupSpotlight();
})();
