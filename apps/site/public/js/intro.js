// Runs in the head, before first paint, so the opening screen never flashes for a returning visitor.
// It only marks the page: a missing or failing script simply means no opening screen.
(function () {
  try {
    var saver = navigator.connection && navigator.connection.saveData;
    var calm = window.matchMedia && matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (saver || calm || sessionStorage.getItem('hq-intro')) return;
    sessionStorage.setItem('hq-intro', '1');
    document.documentElement.classList.add('intro-play');
  } catch (e) {
    /* storage can be blocked, which means no opening screen */
  }
})();
