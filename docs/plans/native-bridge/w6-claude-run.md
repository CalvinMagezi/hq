# W6 run: a real claude.ai account through an edge to a device

What the tests in the repository cannot settle. Everything below the line "Claude's servers" is
covered on loopback by `cargo test -p hq-web --lib tls::tests::chain` (a real `hq edge`, a real
device bridge and the real connector door, with a test CA); what is left is the public internet,
a real CA and a real claude.ai account.

## Questions this run answers

1. **The path on the internet.** Does a Let's Encrypt certificate for `<label>.<edge domain>` get
   issued with the validation arriving through the edge, and do Claude's calls arrive?
2. **A consent page that is private to the device.** Does claude.ai send the browser to an
   `authorization_endpoint` that only the device's own browser can open?
   - **Run A, a private https name** (a tailnet name, as slice 0 proved). Known to work; run it
     first as the control.
   - **Run B, plain http on the device's loopback address** (`http://127.0.0.1:<port>`), the
     in-app case. This is the open question.
3. **The request cookie.** With the origin on plain http, is the binder cookie (`Secure` dropped,
   `HttpOnly; SameSite=Strict`) kept by the browser Claude opens, so Approve works?
4. **After approval.** Does Claude list the tools and run `hq_discover` through the edge, and does a
   revoke on the device (`hq bridge forget-edge`, or `hq connector revoke`) cut it?

Not part of this run: the phone sleeping (W8), edge load (W8), the default relay (W8).

## What the owner provides

- A machine on the internet you control, with TCP 443 and the UDP ports reachable (the relay's
  7842 and the iroh port you choose, `edge.iroh_bind`), running the `hq` binary.
- A domain whose DNS you can edit: a wildcard `*.edge.<your domain>` pointing at that machine.
- A device to stand in for the phone: a Mac or Linux machine at home running the `hq` binary built
  with `--features bridge`. It is the same code path the phone app runs.
- Your claude.ai account (a plan that can add custom connectors), and the browser you normally use
  on the device.
- For run A only, a private https name for the device's web server (Tailscale `serve`, for
  example). Run B needs none.

## Steps

### 1. The edge (public machine)

```yaml
# config.yaml on the edge machine
edge:
  enabled: true
  domain: edge.<your domain>
  iroh_bind: "0.0.0.0:7843"
  relay:
    enabled: true
    hostname: relay.edge.<your domain>
    acme_email: <your address>
    acme_production: true       # devices verify the relay's certificate, so a staging one fails
```

```bash
hq edge run              # leave it running; it prints nothing secret
hq edge invite           # QR code and link for one device, valid ten minutes
```

### 2. Register the device

On the device (the link is a secret for ten minutes; paste it, do not leave it in shell history):

```bash
hq bridge register '<link>'    # prints six digits
```

Type those six digits at the `hq edge invite` prompt on the edge machine. The device prints the
name it is reached at. `hq bridge edge` shows it again. Expect `<16 hex>.edge.<your domain>`.

### 3. The device's connector

```yaml
# config.yaml on the device
web_auth_token: <a long random string>
mcp_connector:
  enabled: true
  public_url: https://<16 hex>.edge.<your domain>
  authorize_url: https://<private name>     # run A
  # authorize_url: http://127.0.0.1:5678     # run B (the port `hq web` listens on)
  tls:
    enabled: true
    agree_to_ca_terms: true
    acme_directory: letsencrypt-staging      # first; switch to letsencrypt once it works
```

```bash
hq web --detach                              # serves the connector and opens the door
hq bridge serve --relay auto                 # pipes the edge's streams to the door
hq connector status                          # enabled, public URL, consent origin
```

HQ orders the certificate by itself once the door is up. There is no status command for it yet:
look for the files in `<vault>/_data/connector_tls/` (a `.pem` for the name) and, in `hq web`'s
log, the line that says it will try again if the CA refused. Staging certificates are not trusted
by Claude. When a staging certificate has been issued (question 1, first half), change
`acme_directory` to `letsencrypt`, restart `hq web`, and wait for a production one. The CA allows
five certificates per name per week: do not loop on this.

Check from outside the device's network (a phone on mobile data, no VPN):

```bash
curl -s https://<16 hex>.edge.<your domain>/.well-known/oauth-authorization-server
```

You should see the metadata with `authorization_endpoint` on your chosen consent origin.

### 4. Connect from claude.ai

1. In HQ's Settings, Claude connector, **Allow a new connection** (or `hq connector arm`).
2. In claude.ai, Settings, Connectors, add a custom connector with
   `https://<16 hex>.edge.<your domain>/mcp`, then Connect.
3. Watch where the browser goes. Record the address bar at each step (write down the host and
   port, not the query string, which carries one-time values).
4. On the consent page, approve. Ask Claude to run `hq_discover`.

### 5. Revoke

`hq bridge forget-edge` on the device, then ask Claude to run a tool again. It must fail within a
few seconds. Re-register with `hq bridge register --replace '<new link>'` if you want to go on.

## What to record

| Check | Pass looks like | If it fails |
|---|---|---|
| Certificate through the edge (staging, then production) | A `.pem` for the name appears in `_data/connector_tls/`; `curl` works from mobile data | The edge dropped the validation connection: read `hq edge` logs; check UDP/TCP ports and DNS |
| Run A: Claude reaches the consent page | Browser opens `https://<private name>/oauth/authorize...` then `/connect` | Control failed: the cause is not the origin; stop and report |
| Run B: Claude reaches the consent page | Browser opens `http://127.0.0.1:5678/oauth/authorize...` then `/connect` | Claude (or its browser flow) refused a non-https endpoint: note the exact message. Public-name consent becomes necessary (a separate design and review) |
| Run B: Approve works | The consent page's Approve returns to Claude | The cookie was dropped or the page needs the web token in that browser: note which browser and whether a sign-in link fixes it |
| Tools through the edge | `hq_discover` lists tools | Note the HTTP status from `hq connector log` |
| Revoke | Next call fails | The edge or device kept the old connection longer than a few seconds: note how long |

Send the table filled in (and any message Claude showed verbatim) back to the session that
ran this work. Do not paste tokens, the invite link or the web token.

## Tear down

`hq connector off` on the device, `hq bridge forget-edge`, `hq edge revoke <device id>` on the
edge, then remove the wildcard record if this was a trial.
