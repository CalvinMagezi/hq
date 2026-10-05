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

  setupCopy();
  setupAnimations();
})();
