# Shared compute, phase 1: the phone uses the laptop's model

Status: test zero run 2026-11-05 (results in section 5 below); feature work not started. Phase 1 of a larger idea (devices lending model capacity to each other);
phase 2 and beyond are listed at the end and are out of scope here.

## The problem

HQ on a phone (the hub) sends every model call to a cloud API. A paired laptop often sits next to it
with far more memory and a GPU that idles. The owner should be able to say "use my laptop's model
when it is around" and have HQ do that, fall back to the cloud when the laptop is closed or away, and
expose nothing to anyone else.

## What this is not

Not splitting one model across devices. A phone hub has a fraction of the laptop's memory and speed;
putting layers on it makes every token cross Wi-Fi to reach the slower device. Phase 1 moves whole
requests: the phone keeps chat, vault and tasks, the laptop answers model calls. A request and its
reply are a few KB of text, so this also works when the two are on different networks (through the
iroh relay), not only on the same Wi-Fi.

## What already exists (no new code needed for the proof)

| Piece | Where |
|---|---|
| Named services a paired device may open, each leading to a loopback port on the offering machine | `crates/hq-bridge/src/services.rs` (`hq/svc/1`), config `bridge.services` |
| Forwards: a loopback port that reaches a paired device's service | `BridgeForward` in `crates/hq-core/src/config/bridge.rs`, started by `hq_bridge::start_forwards` |
| The desktop app's bridge reads `bridge.services` and `bridge.forwards` from config at start | `crates/hq-desktop/src/pairing.rs` (`services_from_config`, `start_forwards`) |
| The phone's bridge helper (`hq bridge run`) starts the configured forwards | `run_control` and `start_set_up` in `crates/hq-cli/src/commands/bridge.rs` |
| Pairing setup lives in the database, not `config.yaml`, so owner edits to `bridge.*` are not overwritten | `docs/plans/pairing-autoconfig.md`, `crates/hq-bridge/src/setup.rs` |
| Primary backend plus ordered fallbacks | `backends:` (`crates/hq-core/src/config/backends.rs`), `crates/hq-llm/src/backend_chain.rs` |
| An `openai-compatible` backend on `127.0.0.1` needs no key (a placeholder is sent) | `resolve_credential` in `crates/hq-llm/src/backend_chain.rs` |
| Direct or relay path of a bridge connection | `path_kind` in `crates/hq-bridge/src/bridge.rs`; logged by the phone app's `keep` trace (`docs/HQ_MOBILE.md`, adb diagnostics) |

## Test zero: prove it by configuration

Run from a session on the laptop, with the phone attached over USB (`adb`). The phone and laptop are
already paired through the desktop app. Write down every step that had to be done by hand; that
list is the input to the feature work below.

### 1. A model on the laptop

1. Install Ollama. It listens on `127.0.0.1:11434` by default; leave it there (never `0.0.0.0`).
2. Pull a model with native tool calling, because HQ's agent loop needs `tool_calls`. Candidates
   for a 24 GB Apple silicon machine (macOS gives the GPU roughly two thirds of unified memory by
   default): `qwen3:14b` (about 9 GB) or `gpt-oss:20b` (about 13 GB). Check with
   `ollama show <model>` that the capabilities include `tools`.
   Avoid Gemma 4 for this test: it ignores the `tools` parameter, and HQ's prompted-tools shim
   (`crates/hq-llm/src/prompted_tools.rs`) only runs inside `OllamaProvider`, not on an
   `openai-compatible` backend.
3. Sanity check on the laptop:
   `curl -s http://127.0.0.1:11434/v1/chat/completions -d '{"model":"qwen3:14b","messages":[{"role":"user","content":"hi"}]}'`

### 2. The laptop offers the model as a service

Add one entry to `bridge.services` in the laptop's `~/.hq/config.yaml` (or `$HQ_CONFIG_PATH`). Leave
everything pairing set up alone; it lives in the database.

```yaml
bridge:
  services:
    llm: 127.0.0.1:11434
```

Restart the desktop app so its bridge rereads the config. The app's paired-devices view lists the
services it offers; `llm` should be among them.

### 3. The phone reaches it through a forward

The phone's config is `files/hqhome/config.yaml` in the app's private storage. Find the package
with `adb shell pm list packages | grep -i hq`. On a debuggable build, `adb shell run-as <package>`
reaches the file; on a release build there may be no way to edit it from adb, which is itself a
finding to record.

```yaml
bridge:
  forwards:
    - peer: <laptop machine id>     # `hq bridge peers` on the laptop shows ids; use the laptop's own
      service: llm
      listen: 127.0.0.1:47600       # outside the ports pairing already picked; any free loopback port

backends:
  primary: laptop
  fallbacks: [cloud]
  backends:
    - name: laptop
      kind: openai-compatible
      endpoint: http://127.0.0.1:47600/v1
      model: qwen3:14b
      credential_env: HQ_LAPTOP_LLM_KEY   # unset is fine: loopback gets a placeholder.
                                          # Set explicitly so the generic OpenAI key is never sent here.
    - name: cloud
      kind: openrouter                    # or whatever the phone uses today
      model: <current model>
```

If the phone already has a `backends:` section from first run, keep its entry and make it the
fallback rather than replacing it. Restart the app (or turn Paired devices off and on) so both the
bridge helper and the `hq` child reread the config.

### 4. What to check and record

| # | Check | Expected | Record |
|---|---|---|---|
| T1 | Chat from the phone on the same Wi-Fi as the laptop | Reply comes from the `laptop` backend | Time to first token, tokens/s, how you confirmed the backend (`hq usage`, the daemon log, Ollama's log on the laptop) |
| T2 | A turn that uses tools (for example "list my open tasks") | Tool calls run normally | Whether the model's tool calls parsed |
| T3 | Path type during T1 | `direct` | From the phone app's `keep` trace (`adb shell am start ... --es keep <laptop id>`, then `adb logcat`) |
| T4 | Phone on mobile data, laptop on home Wi-Fi | Still served by the laptop, path `relay` | Latency difference from T1, data used on the phone for one turn |
| T5 | Close the laptop lid, then chat | Falls back to `cloud` | **How long the fallback takes.** The forward accepts the local connection and then spends up to 20 s dialing (`DIAL_TIMEOUT` in `services.rs`), so expect a long stall |
| T6 | Quit Ollama but keep the laptop awake | Falls back to `cloud` | Time to fall back (the service answers "down", which should be fast) |
| T7 | From another app on the phone (for example a terminal app), `curl http://127.0.0.1:47600/v1/models` | Today: it answers | Confirms gap 3 below |

### 5. Results (run 2026-11-05)

Laptop: 24 GB Apple silicon, Ollama 0.40.14, `qwen3:8b` (already pulled; the 14b and 20b candidates
did not fit on a nearly full disk). Phone: release build 0.9.1-main.299, then 0.9.1-main.364 (the
config edits survived the update). HQ's chat prompt was about 26,200 tokens.

| # | Result |
|---|---|
| T1 | Served by `laptop` (Ollama log, and `budget_status` lists `qwen3:8b` at $0). A cold turn took 3 min 30 s: prompt read at about 136 tokens/s (192 s), output at 10 to 16 tokens/s. Later turns in the same thread took 13 to 24 s because Ollama reused its prompt cache. |
| T2 | Tool calls parsed and ran (two round trips per turn). The answer was wrong (it said 0 tasks where 7 exist): a model quality limit, not a transport one. |
| T3 | `direct`, round trips mostly 5 to 50 ms. |
| T4 | Worked on mobile data with path `relay`. A cold turn took about 4 min 10 s plus a second call of 1 min 20 s. The phone moved about 1.5 MB in and 1.3 MB out over that time, other traffic included. |
| T5 | Laptop bridge stopped: the forward accepted the local connection and returned nothing for at least 35 s (the test's own limit). `stream_to` dials up to twice with a 20 s `DIAL_TIMEOUT`, so the stall can reach 40 s before the chain even tries the fallback. |
| T6 | Ollama quit, laptop awake: fell back to `luna` and answered in 6.2 s end to end. |
| T7 | Confirmed: the `adb shell` user, a different process from the app, got `/v1/models` through `127.0.0.1:47600`. |

What had to be done by hand, and what the plan got wrong:

1. **Ollama's default context silently truncates HQ's prompt.** With `default_num_ctx=4096` Ollama cut
   the 26,184-token prompt to 2,050 tokens with only a log warning; the reply still looked fine.
   The `/v1` API cannot raise it per request. Fix used: `launchctl setenv OLLAMA_CONTEXT_LENGTH 32768`
   and restart Ollama (lost on reboot; the Ollama app's context setting is the durable way). Sharing
   must check the served context length and refuse or warn when it is smaller than HQ's prompt.
2. **On the laptop the bridge is the launchd `com.agent-hq.bridge` (`hq-bridge bridge serve`), not the
   desktop app.** Restarting it (`launchctl kickstart -k`) drops every forward through it for a few seconds.
3. **The phone's config on a release build** was edited with `file_edit_batch` over MCP (no `run-as`).
   `config_manage get` on the phone answers "no config file": it does not look at `hqhome/config.yaml`.
4. **Restarts:** the backend chain picked up `backends:` on the next turn with no restart. The new forward
   needed Paired devices turned off and on.
5. **The phone's other key goes to the laptop.** `resolve_credential` falls back to the generic OpenAI
   credential whenever the `credential_env` variable is unset, even when `credential_env` is named, so
   naming it does not keep that key away from the laptop. Only a loopback endpoint with no generic key
   gets the placeholder.
6. **Everything moves, not only chat.** Background work (memory, reviews) also ran on the laptop once it
   was primary, which makes the slow cold prompt read worse. Phase 2's separate chains matter sooner.
7. **Ollama checks the `Host` header**: a request with `Host` other than a loopback name gets 403. HQ's
   client sends `127.0.0.1:47600`, which passes.

## Feature work after test zero

Each is a small, focused PR with tests. Order by what test zero shows hurts most.

1. **Opt-in switch on the laptop.** "Share this computer's model" in the desktop app. Off by
   default; when on, the bridge offers `llm` (pointed at a detected Ollama or a configured target).
   Optional: only while on mains power. Turning it off closes open streams the way revocation does.
2. **Fast failover.** A request should not wait 20 s for a laptop that is gone. Options: the
   backend chain asks the bridge whether the peer is reachable before choosing that link, or forwards
   for model services use a short dial timeout and close the local connection at once on failure so
   the HTTP client errors immediately. Test zero measured a stall of at least 35 s (two 20 s dials).
   Target: fallback within 2 s of the laptop being unreachable.
3. **The forward is open to every app on the phone.** `docs/security/PAIRING.md` S8 accepts that any
   local process can use a forward and says to use forwards on single-user machines. Android is not
   one: every app can reach the phone's loopback, and Ollama has no authentication. Fix by putting a
   key in front of the model service (the laptop side checks a per-peer key that the phone's backend
   sends as its bearer token, minted at pairing like the per-device MCP key), or by letting a forward
   require a credential. Update S8 either way.
4. **Same network only, as an option.** Per forward or per backend, `when: direct | any`, using
   `path_kind`. Default `any`, since requests are small; `direct` for owners on metered data.
5. **Pairing sets it up.** When the laptop turns sharing on, the setup exchange
   (`hq/setup/1`) tells the hub, and the hub adds the forward and a `laptop` backend in its database,
   the same way pairing sets up `web` and `gate` today. No hand edits on the phone. Revoking the laptop
   removes both.
6. **Tool calling for local models served elsewhere.** Let an `openai-compatible` entry opt into the
   prompted-tools shim, so models like Gemma 4 work through the forward too.

## Done when

- An owner turns sharing on in the desktop app and, with no config edits, the phone's chats run on
  the laptop's model when it is reachable (same Wi-Fi or not) and on the cloud backend when it is not.
- Fallback after the laptop disappears takes under 2 s.
- No other app on the phone, and no device other than the paired hub, can use the laptop's model.
- `docs/PAIRING.md`, `docs/security/PAIRING.md` (S8) and the backends docs describe it.

## Later phases (out of scope here)

- **An always-on home device** (for example an iPhone that stays home on Wi-Fi and a charger) as a
  second link in the chain for background work with a small model. HQ does not run on iOS, so the
  likely way in is the WireGuard app joining `hq net` and a llama.cpp server listening only on its
  tunnel address. It answers whole requests; it does not hold layers of the laptop's model over the
  internet (30 to 80 ms per hop on every token, and gigabytes of weights to upload).
- **Separate chains for interactive and background work**, so background calls prefer the always-on
  device and chat prefers the laptop.
- **Splitting one model across devices on the same LAN** (llama.cpp RPC with the laptop in charge),
  only where pooled memory buys a meaningfully larger model. llama.cpp's RPC server has no
  authentication, so it would run only over `hq net` or the bridge, never on an open port.
