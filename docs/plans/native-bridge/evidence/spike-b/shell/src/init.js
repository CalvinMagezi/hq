(() => {
  if (window.top !== window) return;
  const violations = [];
  document.addEventListener('securitypolicyviolation', (e) => violations.push(e.violatedDirective + ' ' + e.blockedURI));
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const run = async () => {
    await wait(6000);
    const r = { origin: location.origin };
    try {
      const n = Number(localStorage.getItem('spike_launches') || '0');
      r.launches_before_this = n;
      localStorage.setItem('spike_launches', String(n + 1));
      r.token_stored = !!localStorage.getItem('hq-web-token');
      r.hash_after_load = location.hash;
    } catch (e) { r.storage_error = String(e); }
    r.sw_supported = 'serviceWorker' in navigator;
    try { r.sw_registrations = (await navigator.serviceWorker.getRegistrations()).length; } catch (e) { r.sw_error = String(e); }
    r.push_manager = 'PushManager' in window;
    r.notification = typeof Notification === 'undefined' ? 'absent' : Notification.permission;
    r.tauri_ipc_visible = typeof window.__TAURI_INTERNALS__ !== 'undefined';
    try { r.ipc_call = 'allowed:' + JSON.stringify(await window.__TAURI_INTERNALS__.invoke('plugin:window|title', { label: 'main' })); } catch (e) { r.ipc_call = 'denied:' + String(e).slice(0, 160); }
    r.body_text_len = document.body ? document.body.innerText.length : -1;
    try {
      const tok = localStorage.getItem('hq-web-token');
      const t = await fetch('/api/ws-ticket', { method: 'POST', headers: { Authorization: 'Bearer ' + tok, 'x-hq-client': 'spike' } });
      r.ticket_status = t.status;
      const { ticket } = await t.json();
      r.ws = await new Promise((res) => {
        const ws = new WebSocket((location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/ws?ticket=' + encodeURIComponent(ticket));
        const timer = setTimeout(() => res('timeout'), 5000);
        ws.onopen = () => { clearTimeout(timer); res('open'); ws.close(); };
        ws.onerror = () => { clearTimeout(timer); res('error'); };
      });
    } catch (e) { r.fetch_ws_error = String(e); }
    r.csp_violations = violations;
    document.title = 'RESULT:' + JSON.stringify(r);
  };
  window.addEventListener('load', run);
})();
