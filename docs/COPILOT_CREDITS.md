# Copilot credit meter

HQ shows how many GitHub Copilot AI credits are left, how fast they are being spent, and when they will run out. It only appears when an enabled backend talks to Copilot: kind `github-copilot-api`, or an `openai-compatible` backend whose endpoint host contains `githubcopilot.com` (`hq_core::config::copilot_active`).

## What is measured

`hq_llm::copilot_usage::fetch_quota` reads `GET https://api.github.com/copilot_internal/user` with the same token HQ uses for inference. The numbers are the `premium_interactions` snapshot: total credits (`entitlement`), `credits_used`, `remaining`, the cycle reset date, and whether overage is permitted.

Every `copilot_usage.interval_minutes` the daemon task `copilot-usage` stores a sample in `copilot_usage_samples` (migration 066). Samples older than 45 days are pruned. Only metered plans are stored.

## Account identity caveat

The balance belongs to whichever account owns the token HQ resolves, which may not be the Business seat you expect. The `login`, `plan` and `sku` are shown next to the numbers so you can check. A personal free account reports 0 of 0; that is not metered, nothing is stored, and the tool and UI say so instead of showing a balance.

## How burn is computed

`hq_llm::copilot_burn::compute_burn` takes the stored samples plus the live reading.

- For each window (1h, 6h, 24h) it uses the first and last sample inside the window. A drop in credits used means the cycle reset, so the window restarts after the drop.
- The rate is credits used divided by the hours between those two samples.
- The projection uses the 6h rate, falling back to 24h, then 1h. Projected use at reset is used plus rate times hours to reset. Exhaustion is remaining divided by rate.
- `exhausts_before_reset` states the fact; `overage_permitted` is reported separately because running out is not a hard stop when overage is allowed.
- `confidence` is `low` under 3 samples or under 30 minutes of history, `medium` under 6 hours, otherwise `high`.

## Surfaces

- Tool `copilot_credits` (owner sessions, read-only, `refresh` defaults to true and stores a sample first).
- REST `GET /api/copilot-usage`: `{active, quota, burn, samples}`, with the live read cached for 60 seconds. `{active: false}` without a Copilot backend.
- Web: a "Copilot credits" section on the Settings page and a `N left` chip in the header, both polling every 60 seconds.

## Config

```yaml
copilot_usage:
  enabled: true          # default
  interval_minutes: 10   # default
```
