# Outbound fetches (SSRF)

`web_fetch` takes a URL the model chose, and the model may have been steered by
a web page, an email or a vault note. Those URLs must never reach services on
the machine or its network: loopback, the cloud metadata endpoint
(`169.254.169.254`), RFC 1918 ranges, other hosts on a private network.

## What is enforced

`crates/hq-tools/src/web/ssrf.rs`, used by the client every `web_fetch` goes
through (`web::guarded_client()` for other tools):

- **Syntax check** (`validate_url`). Only `http` and `https`. `localhost`,
  `*.localhost` and a trailing-dot form are refused. A literal IP is classified
  as written. The URL parser folds decimal, octal and hex IPv4 spellings
  (`2130706433`, `0177.0.0.1`, `0x7f.1`) to dotted form first, so they are
  classified as the address they really are.
- **Resolution and pinning** (`GuardedResolver`). A hostname is resolved by HQ.
  If any answer is non-public the fetch is refused, and the connector is handed
  only the checked addresses, so a second lookup (DNS rebinding) cannot change
  where the socket goes. This runs for the first request and for every
  redirect hop.
- **Classifier** (`is_non_public_ip`). Refuses `0.0.0.0/8`, `10/8`,
  `100.64/10` (CGNAT), `127/8`, `169.254/16`, `172.16/12`, `192.0.0/24`,
  `192.0.2/24`, `192.88.99/24`, `192.168/16`, `198.18/15`, `198.51.100/24`,
  `203.0.113/24`, multicast, `240/4`, and for IPv6 `::`, `::1`, `fc00::/7`,
  `fe80::/10`, `fec0::/10`, `ff00::/8`, `100::/64`, `2001::/23` (Teredo and
  protocol assignments), `2001:db8::/32`, `3fff::/20`, `5f00::/16`, plus IPv4-mapped, IPv4-compatible,
  NAT64 (`64:ff9b::/96`), SIIT (`::ffff:0:a.b.c.d`) and 6to4 (`2002::/16`) forms judged by the IPv4
  address they embed.

## Deliberate exceptions and limits

- `searxng_url` is operator configuration, not model input, and is usually a
  loopback or LAN address. Search backends use a separate client and are not
  covered by this guard. Nothing a model supplies reaches that client.
- Proxies are off. The fetch client ignores `HTTP_PROXY`, `HTTPS_PROXY`,
  `ALL_PROXY` and the system proxy, because a proxy resolves names itself and
  the pinned resolver would never run. Set `HQ_WEB_FETCH_USE_PROXY=1` to opt in;
  HQ then logs a warning once, and the proxy's own egress policy is the only
  control left.
- `imagegen` downloads the image URL a model returns through the same guarded
  client, and picks the saved file's extension from a fixed list of raster
  types, never from an svg, html or xml content type.
- The Jina Reader fallback (`r.jina.ai`) receives the URL and fetches it from
  its side. HQ only sends URLs that already passed the checks above.
- A hostname is refused if any of its addresses is non-public, even if others
  are public. That is stricter than filtering and avoids split-horizon surprises.
