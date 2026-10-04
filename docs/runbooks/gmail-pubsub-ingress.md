# Gmail Pub/Sub Push Ingress (Primary Email Path)

The primary email ingress is the `/hooks/gmail` webhook. Gmail watches the
operator inbox and publishes change notifications to a Pub/Sub topic, which
push-delivers to HQ. On any push HQ fetches the inbox via `gws gmail +triage`,
dedups by message id, and enqueues `agent-worker` triage events. The
`email-poll` daemon task is a time-gated fallback that runs the same logic.

This one-time external setup wires Gmail to the webhook. Run it once per
operator inbox.

## 1. Create the Pub/Sub topic and push subscription

Replace `PROJECT_ID` and pick a strong `GMAIL_WEBHOOK_SECRET`.

```bash
gcloud config set project PROJECT_ID

gcloud pubsub topics create gmail-inbox

# Gmail's service account must be able to publish to the topic.
gcloud pubsub topics add-iam-policy-binding gmail-inbox \
  --member="serviceAccount:gmail-api-push@system.gserviceaccount.com" \
  --role="roles/pubsub.publisher"

# Push subscription delivers each notification to the HQ webhook. The shared
# secret rides in the query string (Pub/Sub preserves the push endpoint URL).
gcloud pubsub subscriptions create gmail-inbox-push \
  --topic=gmail-inbox \
  --push-endpoint="https://hq.your-domain.com/hooks/gmail?token=GMAIL_WEBHOOK_SECRET" \
  --ack-deadline=30
```

## 2. Register the Gmail watch (expires ~7 days, must be renewed)

`users.watch` tells Gmail to publish inbox changes to the topic. The watch
expires in roughly 7 days, so renew it daily.

```bash
ACCESS_TOKEN=$(gcloud auth print-access-token)

curl -sX POST \
  "https://gmail.googleapis.com/gmail/v1/users/me/watch" \
  -H "Authorization: Bearer ${ACCESS_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{
        "topicName": "projects/PROJECT_ID/topics/gmail-inbox",
        "labelIds": ["INBOX"],
        "labelFilterBehavior": "include"
      }'
```

Renew daily (the same call is idempotent). A minimal crontab entry:

```cron
0 6 * * * /usr/bin/curl -sX POST \
  "https://gmail.googleapis.com/gmail/v1/users/me/watch" \
  -H "Authorization: Bearer $(gcloud auth print-access-token)" \
  -H "Content-Type: application/json" \
  -d '{"topicName":"projects/PROJECT_ID/topics/gmail-inbox","labelIds":["INBOX"]}' \
  >> /var/log/gmail-watch-renew.log 2>&1
```

## 3. Add the `/hooks/gmail` route to the VPS Caddy config

The webhook lives on the same hq-web server as the rest of the API, so it only
needs to be reachable through the existing reverse proxy. Proxy only
`/hooks/gmail`, never the whole host: `/api` and `/ws` run tool-using agent
sessions and must stay off the public internet (see
`deploy/Caddyfile.production`). Add a handler to the public site block:

```caddyfile
hq.your-domain.com {
    @gmail path /hooks/gmail
    handle @gmail {
        reverse_proxy 127.0.0.1:5678
    }
    handle {
        respond "Not found" 404
    }
}
```

Reload: `caddy reload --config /etc/caddy/Caddyfile`.

## 4. Set `GMAIL_WEBHOOK_SECRET` in HQ's environment

The handler accepts the secret as `?token=` or `Authorization: Bearer`. It must
match the value baked into the push endpoint URL in step 1. If the variable is
unset the webhook refuses every request with `503 Service Unavailable`, so
nothing is ingested until it is set.

```bash
# launchd plist EnvironmentVariables, or the systemd unit, or shell profile:
export GMAIL_WEBHOOK_SECRET="<the same value used in the push endpoint>"
```

Restart HQ so the variable is picked up.

## Verify

```bash
# Should enqueue (or no-op if inbox unchanged / no email listener configured).
curl -sX POST "https://hq.your-domain.com/hooks/gmail?token=${GMAIL_WEBHOOK_SECRET}" \
  -H "Content-Type: application/json" -d '{}'
# => {"enqueued": N}
```

A wrong/missing token (when the secret is set) returns `401`. A push that
arrives before config is loaded returns `503`.
