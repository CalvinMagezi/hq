// Runs in <head> before first paint so a saved theme never flashes.
(function () {
  var KEY = 'hq-theme';
  var root = document.documentElement;
  try {
    var saved = localStorage.getItem(KEY);
    if (saved === 'light' || saved === 'dark') root.setAttribute('data-theme', saved);
  } catch (e) {
    /* storage can be blocked; the system preference still applies */
  }

  function effective() {
    var set = root.getAttribute('data-theme');
    if (set) return set;
    return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
  }

  document.addEventListener('DOMContentLoaded', function () {
    var btn = document.querySelector('[data-theme-toggle]');
    if (!btn) return;
    // The label names the theme a click switches to, so it must follow every change.
    function sync() {
      var now = effective();
      btn.textContent = now === 'light' ? 'Dark theme' : 'Light theme';
    }
    btn.hidden = false;
    sync();
    window.matchMedia('(prefers-color-scheme: light)').addEventListener('change', sync);
    btn.addEventListener('click', function () {
      var next = effective() === 'light' ? 'dark' : 'light';
      root.setAttribute('data-theme', next);
      try {
        localStorage.setItem(KEY, next);
      } catch (e) {
        /* the choice still applies for this page view */
      }
      sync();
    });
  });
})();
