# Pairing configures everything

Status: design, for the owner's review (MOBILE-FIRST-002). Builds on `hq bridge` services and
forwards (PR #321) and on what test zero had to do by hand on 2026-10-31.

## The problem

After a QR pairing, the phone and a laptop trust each other's key, but nothing else works yet. In
test zero every one of these was written by hand:

| Side | What was set by hand |
|---|---|
| Laptop | `bridge.services.gate` pointing at the host gate; the gate moved to loopback; the phone's gate key in `gate_authorized_keys` pinned `from=127.0.0.1`; a forward to the phone's `web` service; `remote_mcp` and `task_peer` at that forward; an MCP entry for Claude Code and Claude Desktop |
| Phone | `bridge.web_listen` and the `web` service; a forward to the laptop's `gate`; an `agent_host.hosts` entry at that forward; `agent_host.agent_mcp_url` |

A non-technical person cannot do any of this. The goal: scan, type six digits, done. Revoking the
device undoes all of it.

## Roles

A pairing joins a **hub** (the machine that holds HQ: the phone app, or a laptop or VPS that runs
the full daemon) and a **machine** (a laptop, desktop, WSL2 install or server that runs coding agents
for it). The phone app is always a hub. `hq bridge pair` and `hq bridge join` take `--as machine`
(the default on a computer) or `--as hub`. Two hubs pair only as peers with no setup.

## Setup after pairing: one more exchange

Right after the digits match, inside the same authenticated connection, each side sends one setup
message (a new ALPN `hq/setup/1`, admitted only from paired peers, the same gate as `hq/svc/1`):

- **Hub to machine:** "I am a hub, named N. My web app and `/mcp` are the service `web`. Here is an
  MCP key minted for you alone, and my host-gate public key."
- **Machine to hub:** "I am a machine, named M. My host gate is the service `gate`."

The MCP key is new: a per-device key with the owner's full scope, stored hashed on the hub next to
the peer and revoked with it. Today there is one shared `AGENTHQ_API_KEY`; a key per device means
revoking a laptop never touches another.

The message is versioned. A side that does not speak `hq/setup/1` leaves the pairing as it is today,
so old builds still pair.

## Where the result lives: the database, not config.yaml

Setup never edits the owner's `config.yaml`. Each side stores what it set up in the database, next
to the peer (`bridge_peers` gains a `setup` JSON column, migration 100):

- **Machine:**
  - The `gate` service.
  - A forward to the hub's `web` on a port it picks from 47400 to 47999 and keeps.
  - The hub's gate key with `from=127.0.0.1`, written into `gate_authorized_keys` with the label
    `bridge:<peer id prefix>`, so the line can be found and removed.
  - A remote MCP entry named after the hub, used by `task_peer` when none is configured.
- **Hub:**
  - The `web` service.
  - A forward to the machine's `gate` on a picked port.
  - A host entry named after the machine, with the `tcp` transport at that port.
  - `agent_mcp_url` at the machine's forward when unset (the machine side reports its forward port in
    its message).

`hq bridge serve` and `hq bridge run` read these rows as well as the config sections. Config wins
when both name the same thing, so a hand-written setup keeps working. The host list, `task_peer`
and `agent_mcp_url` read the same rows the same way.

On the machine, `hq bridge pair` finishes by offering to add HQ to the coding tools it finds:
- Claude Code: `claude mcp add --scope user --transport http <hub> http://127.0.0.1:<port>/mcp`
  with the per-device key.
- Claude Desktop: `hq mcp-serve --remote <hub>`.
It asks before each one, because these are the owner's files.

## Revoke

Revoking a peer, from either side, the app, the CLI or the desktop app, removes its setup row:
- Its forwards close.
- Its services stop answering it.
- Its gate key line is deleted.
- Its per-device MCP key is revoked.
- Its host entry disappears.
- The Claude Code and Desktop entries added for it are removed. They are found by name and by the
  exact URL that was written.

## Security notes

- Setup runs only on a connection whose key is already on the paired list. A stranger gets
  nothing, the same as `hq/svc/1`.
- Every listener stays on loopback. Ports are picked locally, never by the peer.
- The per-device MCP key travels inside the encrypted iroh connection, once, and is stored hashed
  on the hub. On the machine it sits where the remote MCP key sits today (the database, mode 0600),
  never in the owner's `config.yaml`.
- The gate key pin is `from=127.0.0.1`, which is what bridge traffic looks like. A local process
  still needs the hub's gate private key to use it.
- `docs/security/PAIRING.md` gets a section on setup: what each message may set, and the test for
  each rule.

## Tests

- Two machines on loopback (the existing harness in `crates/hq-bridge/tests`): pair, then
  `host_list` on the hub shows the machine reachable, and a `task_get` from the machine reaches the
  hub. No config edits.
- Revoke from each side, then each row's effect is gone: the forward port refuses, the gate key line
  is absent, the MCP key gets 401 and the host entry is gone.
- An old peer without `hq/setup/1` still pairs, with no setup.
- A peer may not choose a port or an address. The setup message carries service names and a key only.

## Not in this step

- Windows-native pairing. Windows pairs from WSL2.
- Choosing the relay (MOBILE-FIRST-006).
- The desktop app's buttons for this. It calls the same library afterwards.

## Open questions for the owner

1. **Should a laptop that runs its own full HQ** (not only agents) **pair as a hub?** Two hubs would
   then sync nothing and only see each other. Proposed: yes, peers with no setup.
2. **Per-device MCP keys:** full scope by default, or the tasks scope unless the owner widens it?
   Proposed: full, since the laptop is the owner's own machine and agents get their own narrow
   session tokens anyway.
