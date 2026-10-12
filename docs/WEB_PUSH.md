# Web Push

HQ can send push notifications to the web app on a phone or desktop, so they arrive even
when HQ is closed. HQ itself, and any agent that reaches HQ over MCP, sends one with the
`notify_owner` tool. Approvals waiting on you are pushed too.

Push is off until you turn it on.

## Turning it on

1. In `~/.hq/config.yaml`:

   ```yaml
   web_push:
     enabled: true
     subject: "mailto:admin@example.com"   # how push services can reach you
   ```

   Restart the server (`hq web` or the daemon). On first start it creates a VAPID key at
   `.vault/_data/web_push_vapid.p8`, readable by the owner only.

2. Open HQ in the browser you want notified, go to **Notifications** and press
   **Turn on push**. Allow notifications when the browser asks, then press **Test**.

Each browser or device turns push on for itself. Turning it off on one leaves the others on.

### What it needs

- **HTTPS.** Browsers only offer push to a secure page. A tailnet name such as
  `https://hq.example.ts.net` works, and so does `http://localhost`. Plain HTTP on a LAN
  address does not.
- **iPhone and iPad.** Push works only for HQ added to the Home Screen (Share, then
  Add to Home Screen) on iOS 16.4 or later. Open it from the Home Screen icon, then turn
  push on.
- **Outbound HTTPS from the server** to the push services: `fcm.googleapis.com`
  (Chrome, Edge on Android, most Chromium browsers), `web.push.apple.com` (Safari),
  `updates.push.services.mozilla.com` (Firefox) and `*.notify.windows.com` (Edge on
  Windows). Under `profile: lite` add `web-push` to `lite.allow_egress`, or the server
  refuses to start. `hq doctor --egress` lists it.

## Settings

| Key | Default | Meaning |
|-----|---------|---------|
| `web_push.enabled` | `false` | Turns push on. |
| `web_push.subject` | first `https` entry of `web_allowed_origins`, else `mailto:hq@localhost` | VAPID contact, a `mailto:` or `https:` URL. Apple refuses pushes without a real one, so set it if you use Safari. |
| `web_push.push_all` | `false` | Push every new inbox item. Off: only `notify_owner` items and items waiting on an answer (approvals). |
| `web_push.max_agent_per_hour` | `20` | `notify_owner` calls allowed per rolling hour, across all agents. |
| `web_push.extra_push_hosts` | `[]` | Push services to accept besides the four above. |

## Sending one

From HQ's own chat, a `/watch`, or any MCP client (`hq_call` with `notify_owner`):

```json
{
  "title": "Deploy finished",
  "body": "v1.4 is live on the VPS. Smoke tests passed.",
  "link": "/tasks",
  "dedup_key": "deploy-v1.4",
  "channels": "web"
}
```

- `title` is required, at most 120 characters. The push shows it and the first 300
  characters of `body`; the full body is in the inbox.
- `link` opens a page of the web app on tap: a path such as `/tasks`,
  `/notifications` or `/chat?thread=<id>`. Full URLs are refused, and the service worker
  refuses any other origin too, so a notification can never send you off-site.
- `dedup_key` drops a repeat while an item with the same key is still in the inbox, or
  after you dismissed it. Put the date or version in it for events that recur.
- `channels: web_and_chat` also sends it to the chat relay (Telegram or Discord).

The tool writes the inbox row and nothing else. Every HQ process shares the vault
database, so `hq mcp` (stdio), the `/mcp` endpoint and the daemon can all call it; the web
server picks the row up within a few seconds and pushes it.

## How it works

- `crates/hq-tools/src/notify.rs`: the `notify_owner` tool. It writes a `value_items`
  row with source `agent_notify` (or `agent_notify_relay`) and the optional `link`.
- `crates/hq-web/src/web_push/`: the server end.
  - `vapid.rs` holds the P-256 key and signs a short-lived ES256 token per push
    (RFC 8292).
  - `encrypt.rs` encrypts each payload for one browser (RFC 8291, `aes128gcm`), so the
    push service relays it without being able to read it.
  - `mod.rs` is the dispatcher. Every 3 seconds it takes new inbox items (claiming each in
    the database, so two server processes never push it twice), and posts them to every
    subscription. Items older than an hour are never pushed, so a server that was down
    does not replay a backlog. A subscription the push service reports gone (404 or 410)
    is deleted, and one that fails 5 times in a row is dropped.
  - `api.rs` serves `GET /api/notifications/push` (on/off and the public key) and `POST
    .../subscribe`, `.../unsubscribe` and `.../test`, behind the web token. A subscription
    must point at a known push service over https, so the server never posts to an
    address a client made up.
- `crates/hq-db/src/push_subscriptions.rs` and `sql/088_web_push.sql`: the
  subscriptions table, and the `link` and `pushed_at` columns on `value_items`.
- `apps/hq-web/src/lib/webPush.ts`, `components/notifications/PushToggle.tsx` and
  `sw.ts`: subscribing in the browser, the toggle, and the service worker's `push`
  handler. On every start the app re-sends its subscription, so the server keeps it after
  a browser rotates it.

## Privacy

The payload (title, a clipped body and the link) is encrypted end to end. Google, Apple,
Mozilla or Microsoft see when a push is sent and roughly how big it is, not what it says.

Deleting `.vault/_data/web_push_vapid.p8` makes a new key on the next start. Every
existing subscription then stops working; each device re-subscribes the next time HQ is
opened on it.

## Troubleshooting

- **Notifications page says push is off on the server:** `web_push.enabled` is not set,
  or the server could not load its key (see the startup log line `web push not enabled`).
- **Test says the push was not delivered:** the error comes from the push service. A 403
  usually means the key changed since the device subscribed: turn push off and on again.
  A connection error means the server cannot reach the push service.
- **Nothing arrives on iPhone:** HQ must be opened from the Home Screen, and
  `web_push.subject` must be set to a real `mailto:` or `https:` address.
- **Chat replies:** a finished chat reply still alerts only while the page is running
  (`apps/hq-web/src/lib/replyAlerts.ts`). Ask HQ to `notify_owner` when a long job is done
  if you want it on a closed phone.
