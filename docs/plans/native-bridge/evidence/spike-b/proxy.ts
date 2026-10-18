// Tiny loopback reverse proxy for the M6 check: bun proxy.ts <listen> <upstream> keep|rewrite
const [listen, upstream, mode] = process.argv.slice(2);
const target = `127.0.0.1:${upstream}`;
Bun.serve({
  port: Number(listen),
  hostname: "127.0.0.1",
  async fetch(req, server) {
    const url = new URL(req.url);
    if (req.headers.get("upgrade")?.toLowerCase() === "websocket") {
      const ok = server.upgrade(req, { data: { path: url.pathname + url.search, host: mode === "keep" ? req.headers.get("host") : target, origin: req.headers.get("origin") } });
      return ok ? undefined : new Response("upgrade failed", { status: 400 });
    }
    const headers = new Headers(req.headers);
    if (mode === "rewrite") headers.set("host", target);
    headers.set("x-forwarded-for", "device-123");
    const r = await fetch(`http://${target}${url.pathname}${url.search}`, { method: req.method, headers, body: req.body, redirect: "manual" });
    const out = new Headers(r.headers);
    out.delete("content-encoding"); out.delete("content-length");
    return new Response(r.body, { status: r.status, headers: out });
  },
  websocket: {
    open(ws: any) {
      const up = new WebSocket(`ws://${target}${ws.data.path}`, { headers: { host: ws.data.host, origin: ws.data.origin ?? "" } } as any);
      ws.data.up = up; ws.data.queue = [];
      up.onopen = () => { for (const m of ws.data.queue) up.send(m); ws.data.queue = null; };
      up.onmessage = (e) => ws.send(e.data as any);
      up.onclose = () => ws.close();
    },
    message(ws: any, m) { if (ws.data.queue) ws.data.queue.push(m); else ws.data.up.send(m); },
    close(ws: any) { ws.data.up?.close(); },
  },
});
