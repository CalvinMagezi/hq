# A private network for HQ with `hq net`

`hq net` sets up a WireGuard network between your HQ server and your own devices, so the web UI,
the MCP endpoint and the host gate are reachable only from devices you admitted. No account, no
third-party service: the tunnel is the WireGuard that ships with Linux, and the official WireGuard
apps for laptops and phones.

The design and the threat model are in [plans/private-network.md](plans/private-network.md) and
[security/NETWORK.md](security/NETWORK.md). This page is the owner's checklist. Addresses use the
documentation placeholders `10.77.0.0/24` and `example.com`; yours are printed by the commands.

## What you need

- A Linux server with a public address, `wg` (the `wireguard-tools` package), `nft` or `iptables`,
  systemd, and root through `sudo` for a handful of steps. A Rust toolchain until releases include
  the applier (see "What is not done yet").
- UDP port 51820 reachable on the server. WireGuard does not answer a packet from a key it does not
  know, so a scan sees nothing there. A network that blocks UDP cannot use this.
- The WireGuard app on each device.

`hq net` never runs a command as root and never writes outside `~/.hq/net` (plus the one spool
directory below). It prints what you must run, and each command says what it will change before it
changes it.

## 1. Set up the hub

```
hq net init --endpoint example.com:51820
```

This makes the hub's key pair and chooses a subnet (`10.77.0.0/24`, or the next free `/24` if the
machine already routes it; the Tailscale range `100.64.0.0/10` is always avoided, so the two can
run side by side during a move). The routing check reads only the main routing table, not policy
tables such as the one Tailscale keeps its routes in, so also run `ip route show table all`. It
prints five steps. Read them, then run them:

1. Move the hub's private key to `/etc/wireguard/hq0.key`, readable by root only, and delete the
   staging copy in `~/.hq/net`. The private key is never printed and never appears in a config.
   The copy and the delete are one command joined with `&&`, so the staged copy is not deleted if
   the install fails. Until it is gone, `hq net peer add` and `hq net apply` refuse to run (a
   revoke does not wait), because the key must not sit where the HQ user and its agents can read it.
2. Create `/etc/wireguard/hq0.conf`. It reads the key from its file and turns forwarding off for
   the interface.
3. Load a small firewall table that drops anything that would cross `hq0`, so one device cannot
   reach another, except the pairs you link later (below; the table has an empty set for them).
   Add the same table to your own firewall configuration to keep it after a reboot. If your
   firewall drops inbound UDP by default, allow `51820/udp` too.
4. Install the applier (below) and the three systemd units (service, path unit and timer).
5. Start the interface and enable the applier's service, path unit and timer; the exact `systemctl` commands are printed.

The command also reports the machine's real `net.ipv4.ip_forward` value. The hub does not need
forwarding and does not depend on it; step 2 and step 3 hold whether it is 0 or 1. Do not change
it for this. (Linking two devices does need forwarding on `hq0`; see "Linking two devices".)

Running `hq net init` again never makes a second key. It shows the steps again, and `--endpoint`
updates the server address.

## 2. The applier, and why it is separate

Admitting a device means changing the live WireGuard interface, which needs root. HQ itself runs
as your user, so a small program does that one job. It is installed in `/opt/hq-net`, owned by
root (not under `/usr/local`, which a group can write on some systems), and three systemd units
run it: the service after every start of the tunnel, a path unit whenever `hq net` writes a new
peers file, and a timer every five minutes. The timer is a backstop: systemd's path unit does not
fire when it starts and cannot see a change made while the applier is running, and applying the
same peers again changes nothing. A missed revocation is therefore applied within five minutes at
worst, and the applier reads its input a second time after it applies to catch a change made
meanwhile.

Build it without the rest of `hq`, so it links `base64` and `libc` and nothing else:

```
cargo build --release --locked -p hq-net --no-default-features --bin hq-net-apply
```

Build it as an account other than the one HQ runs as, in a clean checkout. HQ's agents can rewrite
files in the HQ account between your build and your `sudo install`. If you have to build in that
account, note `sha256sum target/release/hq-net-apply` after the build and check it again just
before installing. A signed release would remove this step; it is listed under "What is not done
yet".

Do not point a root unit at `~/bin/hq` or `/usr/local/bin/hq`: the installer makes that a link to a
file your user can rewrite.

What the applier does, in order, and what it refuses:

| Step | Rule |
|---|---|
| Arguments | Interface, subnet, hub address and the owner's user id come from the unit and a root-owned file in `/etc/hq-net`, never from the input. Each flag once, nothing unknown. |
| Open | The input path is walked one component at a time with links refused at every step, and the file is opened without blocking, so a link or a FIFO planted there cannot redirect or hang it. |
| Check the open file | A plain file, owned by the expected user, mode without group or other access, exactly one name, at most 64 KiB, in a directory only root or that user owns and others cannot write. |
| Parse | Printable ASCII only. First line `# hq-net peers v1`, last line `# hq-net end <n>` with the number of peers read. (Version 2, written only after the owner links two devices, has `# hq-net peers v2`, `[Link]` sections with `A` and `B` after all the peers, each naming the address of a peer in the file, and the last line `# hq-net end <n> links <m>`.) Only `[Peer]` sections with `PublicKey` (44 characters of base64, 32 bytes), `PresharedKey` (same, optional) and `AllowedIPs` (exactly one IPv4 address written `a.b.c.d/32`, inside the subnet, not the hub, not the network or broadcast address). No duplicate address or key, at most 64 peers. `[Interface]`, `PostUp`, `PreUp`, `Table`, `Endpoint`, `PrivateKey`, any other key, a key in another case, inline comments, tabs and carriage returns all refuse the whole file. |
| Apply | For a version 2 file, first `nft` replaces the contents of the set of linked pairs in one transaction (see "Linking two devices"); a failure there is reported and does not stop `wg`. Then `wg` is started, by absolute path in a distribution directory (and only if root owns it and others cannot write it; use `--wg` for another place), with an empty environment and no shell. It receives text the applier rendered from the parsed peers, never the file's own bytes. |
| Record | After each successful `wg` call it prints how many peers the interface has and what that run added, removed or moved (address and the first characters of the public key); an unchanged run prints one line, so the timer does not fill the journal. The record is written before the input is read again. If that second read or round fails, the error says how many peers were already applied. If `wg` itself fails, the run records what it was trying to change as "attempted", because the kernel may have taken part of it. A changed preshared key is not shown in these lines. `journalctl -u 'hq-net-apply@*'` shows the history. |

A refusal prints the line number and the rule, never the text of the line, and changes nothing. A
file that fails parsing does not even cause `wg showconf` to run. A failure inside `wg` can leave a
partial update, because a long peer list reaches the kernel in more than one message; the next run
repairs it.

The applier limits what a peers file can do. It does not decide who may be admitted: anything
running as your HQ user can add a device, within the subnet, and the journal is where that shows.

`wg syncconf` treats its file as the whole truth about the interface: a file without `PrivateKey`
and `ListenPort` would clear both. So the applier first reads what `wg showconf` says the running
interface has, keeps `PrivateKey`, `ListenPort` and `FwMark`, and puts the validated peers after
them. An empty peer list uses `wg setconf`, which states "replace every peer with none" outright
instead of leaning on how `syncconf` returns early for a file with no peers.

Reconfigure the tunnel with `systemctl restart wg-quick@hq0`, never `reload`: the stock `reload`
runs `wg syncconf` with a config that has neither the private key (it is read from its file) nor
any peers, and would clear both. The applier runs again after every restart.

## 3. Admit a device

On the device, install the WireGuard app and make a key pair there. The private key never leaves
the device; you give HQ only the public key.

Laptop: in the WireGuard app choose "Add Empty Tunnel". The app writes a `PrivateKey` line and shows
the public key.

```
hq net peer add mac --pubkey <public key from the app> --kind laptop
```

HQ refuses a key that has the shape of a WireGuard private key (the clamped bits `wg genkey` sets):
a device's private key must never leave the device, and anything pasted here is stored and sent to
the hub. About one real public key in sixteen has that shape by chance; if you are sure it is a
public key, repeat the command with `--confirm-public`.

Before you give the printed config to the device, compare the hub public key in it with what the
server itself reports, `sudo wg show hq0 public-key`. A config with another hub key or another
endpoint would send the device's traffic somewhere else; the output of `peer add` says the same.

HQ gives the device the next free address (`10.77.0.2`, then `.3`, and so on), updates
`~/.hq/net/peers.toml` and the applier's input, and prints the device's config. Paste its lines
below the `PrivateKey` line the app wrote, and activate the tunnel.

Phone: in the WireGuard app create the tunnel from scratch and let the app generate the key, then:

```
hq net peer add pixel --pubkey <public key from the app> --kind phone
```

and enter the printed address, hub public key, endpoint and allowed address in the app. The
printed QR code is the same text as the config. The Android app refuses to import a config without a
`PrivateKey` line (checked in its source; the iOS app was not checked), and HQ never has a device's
private key, so expect a scan to be refused; use the code to read the values from a second screen.
Leave Android's "Block connections without VPN" off for this tunnel. Budget phones with an
aggressive battery manager can stop a VPN in the background: exempt the WireGuard app.

A laptop's config sets `PersistentKeepalive = 25`, because the server calls the laptop and the
laptop has to keep the path open. A phone's does not by default: a phone that only starts exchanges
gains nothing from a timer, which only costs data and battery.

A phone that other devices must reach (HQ running on it, a device linked to it) is the exception:
without a keepalive its carrier closes the path, and the hub's packets for it are dropped. Add it
with `--keepalive`:

```
hq net peer add pixel --pubkey <public key from the app> --kind phone --keepalive 25
```

`--keepalive <seconds>` sets the interval for any device (10 to 300; the in-app tunnel refuses a join
text outside that range), and `--keepalive 0` turns the laptop's default off. It applies when the
device is added: the printed config carries it, and the phone's app has to have it entered ("Persistent keepalive" on the hub peer). For a device that already
exists, set it in its app; there is no command to change it afterwards. `hq net peer list` shows
the value in the config `hq net` printed, not what the device runs. The setting changes only that
config; the applier's input is the same with or without it. A keepalive keeps a phone's radio
busy, so expect extra battery use.

`peers.toml` gains an optional `keepalive` line for such a device. A device without one writes
nothing, so the file reads as before; once a device has the line, an older `hq net` refuses the
whole file (it reads it strictly), revokes included. Do not roll `hq` back on the hub while a
device carries the line; if you must, delete the `keepalive` lines from `peers.toml` first.

`--psk` also makes a preshared key for the device. It is shown once, in the config, and kept in
`peers.toml`. Treat it like a password.

A device config always has one peer, the hub, and `AllowedIPs` is the hub's single address. HQ never
produces a full-tunnel config (`0.0.0.0/0`).

Other commands:

```
hq net peer list              devices, kinds, addresses, short key fingerprints, keepalive
hq net peer revoke <label>    remove a device; its address is retired and hq net never reuses it
hq net peer forget <address>  retire an address no device owns and remove what the hub has on it
hq net status                 each device's last handshake
hq net apply                  write the applier's input again from peers.toml
```

`hq net status` needs root to read WireGuard. Without it, run
`sudo wg show hq0 latest-handshakes | hq net status --handshakes -`. HQ asks only for that
command, which prints public keys and times; `wg show dump` and `wg showconf` print the private
key and are never used for this.

## 4. When a device is lost

1. `hq net peer revoke <label>`. The applier removes the peer within seconds in the normal case
   and within five minutes at the latest (the timer), and any session keys it held stop working.
   If the command says the device is NOT yet removed from the hub, it is still connected: fix what
   it names and run `hq net apply`.
2. Check: `sudo wg show hq0 peers` no longer lists its key.
3. Sign the device out of the web UI and rotate any token it held.

A revoke is never held up by the check above. If the hub has an entry that no device in
`peers.toml` owns (one a writer put straight into the applier's input, or a device lost when
`peers.toml` was restored from an older copy), the revoke goes through, drops that entry too, and
prints a warning that names its address. That also means a legitimate device whose record was lost
can be removed by a revoke of another device: it is always warned about, and entries are matched by
public key, so look at the warning. One more thing to know about the check: it excuses an address
that is in `retired`, by address and not by key, so a hub entry sitting on a retired address is
dropped without a warning.

`hq net apply` and `peer add` stop instead and name the way out for each address.
`hq net peer forget <address>` retires an address no device owns and removes whatever the hub has
on it; with several unnamed entries it needs one `forget` each, and it also refuses to go on while
another unnamed entry is there. It is refused for an address a device in `peers.toml` owns, even if
the hub has a different key at that address (a restored `peers.toml` with an old key): there the
way out is to correct `peers.toml`, or `hq net peer revoke <label>` for the owning device, which
retires the address and removes the entry. The refusal message says which of the two applies.

A damaged `retired` file does not stop a revoke or a forget either. It is moved aside untouched as
`~/.hq/net/retired.damaged` (never over an earlier copy), the lines that can be read are kept (a
file other users can read, one byte that is not UTF-8, or one junk line does not lose the rest),
and the owner is warned. The new file is written first and the original moved aside only after
that, and put back if the last step fails, so a full disk or a missing permission cannot leave the
record missing. If the original cannot be moved aside at all (every name for a copy is taken, or the
rename fails), nothing is rewritten and the damaged file stays exactly as it is: a revoke still
cuts the device off and says the address could NOT be recorded as retired; move the file aside by
hand and run `hq net peer forget <address>` to record it. `peer add` and `apply` still refuse to
run on a damaged file.

If the hub's key might have leaked, make a new plane: remove `~/.hq/net`, stop and delete the
interface, run `hq net init` again and re-enrol every device. Remove the old server key from every
laptop's `gate_authorized_keys` as well (see [AGENT_HOST.md](AGENT_HOST.md)).

## 5. Check from outside

After every network change, scan the server from a machine that is not on the tunnel. Only the
ports you meant to expose should answer: `22/tcp` if you use ssh, and `51820/udp` should show as
open or filtered but must not answer a probe.

```
nmap -sU -p 51820 example.com
nmap -p- example.com
```

HQ's web UI, MCP and host gate must not appear. Putting the web UI on the tunnel address behind a
trusted certificate is the next phase of the plan and is not part of `hq net` yet.

## Linking two devices

By default no device can reach another: the hub drops everything that crosses `hq0`. When one
device must reach one other (a Mac reaching an HQ that runs on a phone, say), link the two:

```
hq net link add mac phone --applier-updated   # the first time; see below
hq net link list
hq net link remove mac phone
```

The hub then forwards between exactly those two addresses, in both directions and on every port,
and between no other pair. A link names two devices that exist; revoking either device removes its
links in the same write, and its address is retired, so a device added later cannot inherit them.
`hq net link add` also prints, for each of the two devices, the `AllowedIPs` line to set in its
WireGuard app (the hub's address plus the other device's), because a device only sends into the
tunnel what its own `AllowedIPs` names. A phone that must be reachable from outside its own
network also needs `PersistentKeepalive = 25` on its side, or the carrier closes the path. Set
"Persistent keepalive" in that device's app; `hq net peer add --keepalive 25` only does it for a
device added with it.

Two things on the hub have to be in place first, once. Do them before the first `link add`: an
applier that does not know links refuses the version 2 file that `link add` writes, and applies
nothing from it until it is replaced (the timer tries again every five minutes).

1. Build and install the applier again (step 4 of the setup), from a checkout that has links.
2. Load the firewall table with its set and turn forwarding on in the same place, so a hub that
   lost its table cannot be left forwarding. Put the table in `/etc/hq-net/hq_net.nft`:

   ```
   add table inet hq_net
   delete table inet hq_net
   table inet hq_net {
     set links_hq0 {
       type ipv4_addr . ipv4_addr
     }
     chain forward {
       type filter hook forward priority -10; policy accept;
       iifname "hq0" oifname "hq0" ip saddr . ip daddr @links_hq0 accept
       iifname "hq0" drop
       oifname "hq0" drop
     }
   }
   ```

   and in `/etc/wireguard/hq0.conf` replace the `sysctl` line of step 2 with these two, in this
   order. If `nft` fails, `wg-quick` stops before the second line, so forwarding stays off:

   ```
   PostUp = nft -f /etc/hq-net/hq_net.nft
   PostUp = sysctl -q -w net.ipv4.conf.%i.forwarding=1
   ```

   Also include the same file from the system's nftables configuration, after any `flush
   ruleset` it starts with (on Debian and Ubuntu `/etc/nftables.conf`: add `include
   "/etc/hq-net/hq_net.nft"` as its last line). Without that, `systemctl reload nftables` removes
   the table while `hq0` still forwards, and every device can reach every other until the table is
   back. The set is empty after such a reload and the applier fills it again within five minutes;
   until then the linked pairs do not work, which is the safe way round.

   Then `sudo systemctl restart wg-quick@hq0`. The applier starts with the tunnel and fills the
   set from the input file. Never `reload` (see step 5 of the setup).

How the applier handles it: a version 1 file (no link was ever added) is applied as before and
never touches `nft`. A version 2 file replaces the contents of the `links_hq0` set in one `nft`
transaction, before the peers go to `wg`, so a pair is closed no later than a device is cut off.
If `nft` fails, the peers are still applied (a revocation never waits for the firewall), the run
fails, and the journal says that forwarding is unchanged and may allow more than the file does. The
set holds addresses and nothing else; the rules are yours and never change.

Check it after every change, from a third device that is in no pair: it must not reach either end.
`sudo nft list set inet hq_net links_hq0` shows the pairs the hub allows, both directions of each.

What linking does not do:

- It does not restrict ports. A linked device can reach every port of the other. Put the guard on
  the service: the web login, the gate's signature.
- It does not hide traffic from the hub. The hub decrypts a packet from one device and encrypts it
  again for the other, so whoever controls the hub can read what passes between linked devices.
  Every service across a link has to protect itself (TLS for the web UI, an encrypted channel for
  the gate).
- If another firewall (ufw, firewalld) also filters forwarded traffic, it has to allow traffic from
  `hq0` out through `hq0`, and then the table above is the only thing limiting who may talk. Keep
  it loaded at boot (the `PostUp` line does that) and run the check above after every change.
- The set is IPv4 only, like the plane.
- Once a link has been added there is no way back to a version 1 input, and a build of `hq`
  from before links refuses the new `peers.toml` (an unknown table is an error, on purpose), so a
  revocation made with it would not be written. Keep `hq` and the applier current together.

## A phone without the WireGuard app

The default for a phone is the WireGuard app (step 3). The alternative runs the tunnel inside the HQ
app, as a second process, with no system VPN slot and no second app. It is a different kind of peer,
not a VPN: it has two fixed maps and routes nothing else. A **serve** entry hands one port on the
phone's tunnel address to one loopback port, for the tunnel addresses it names. A **reach** entry
forwards one loopback port to one fixed tunnel destination. Nothing else gets in or out.

The commands exist only in a build with the `tunnel` feature (`cargo build -p hq-cli --features
tunnel`); the Android build turns it on, a server build leaves it out. Everything lives in
`~/.hq/tunnel` (or the directory given by `--tunnel-dir`). The commands make the directory 0700 and
write their files 0600, and they refuse a directory that is a link, belongs to someone else or is
open to others, rather than tightening it, because files others could have put there are not to be
trusted. `maps.toml` is written by you; it has to belong to you but may have any mode.

```
hq net tunnel keygen                  make the private key (a file only you can read), print the public key
hq net peer add pixel --pubkey <public key> --kind phone --keepalive 25 > added.txt      on the hub
hq net tunnel import added.txt --confirm-hub <hub public key>                  on the phone
hq net tunnel check                   the join text, the maps and the key, without connecting
hq net tunnel run --key-file ~/.hq/tunnel/key --no-control        or: the key as the first line of stdin
hq net tunnel status                  what the running peer last reported
```

`import` takes what `hq net peer add` printed, as it is or as the bare join text, from a file or
stdin; it keeps only the join text inside (from `[Interface]` to the end of the `[Peer]` section)
and parses that strictly. A hub the device has not seen before is taken only when you name its
public key with `--confirm-hub`. Read that key on the hub (`sudo wg show hq0 public-key`, or the
"Hub public key" line `hq net init` printed) and type or paste it from there, not from the message
`import` prints: it shows the key the text names so that you can compare, and it does not offer a
command to copy, because a text that points the phone at someone else's hub would otherwise be
confirmed by whoever pastes the suggestion. A wrong key is an error. Taking a new hub also deletes
`maps.toml`, before the new join text is written, because the entries in it were written for the old
hub and would otherwise now be trusted by the new one. The same hub again (a new keepalive, say)
needs no confirmation and keeps the maps. `import` exits 0 when it saved the text, 2 when the hub
needs confirming and nothing was written, and 1 for any refusal.

`maps.toml` is yours to write; import never does. A missing file serves and reaches nothing:

```toml
[[serve]]
port = 4751                  # the port on the phone's tunnel address
to = "127.0.0.1:4751"        # loopback only
from = ["10.77.0.1"]         # the tunnel addresses allowed to connect; a connection from any other is refused

[[reach]]
listen = "127.0.0.1:47801"   # loopback only
to = "10.77.0.9:4751"        # one tunnel destination
```

Optional limits in the same file: `tcp_buffer_kb` (16 to 4096, default 256), `idle_listeners` (up to 8,
default 4), `max_connections` (up to 128, default 32, per entry) and `reach_idle_secs` (30 to 86400,
default 600). Anything else in the file is an error. A service behind a serve entry sees every tunnel
peer as `127.0.0.1`, so it must not trust "localhost" and must have its own login; one behind a reach
entry is reachable by any app on the phone, so it needs one too.

The peer reads two control lines from stdin after the key, `network-changed` (the device moved to
another network: rebind and shake hands again) and `quit`. The end of stdin stops it, because the app
that started it is gone, so `hq net tunnel run < key` ends at once: from a shell or a service, use
`--key-file` with `--no-control`, and stop it with a signal. It prints nothing (set `RUST_LOG` to
see its logs, on stderr) and writes `status.json` (`state`: `up`, `connecting`, `down`, `idle` for a session that aged out with
nothing in flight, `stopped`) and a bounded `events.log` beside it. A peer that was killed leaves its
last report behind, and `hq net tunnel status` says it is not running once that is 15 seconds old. It
opens one UDP socket to the hub, the loopback listeners of the reach entries and nothing else; there
is no listener for the status. The private key never goes on a command line or in the environment:
`keygen --stdout` prints it and the public key to a pipe for an app that keeps it in a keystore, and
refuses a terminal.

The threat model and the tests behind each claim are in `docs/security/NETWORK.md`, section 9.
Nothing here has run on a phone yet.

## Files

`net.toml` and `peers.toml` are read strictly. The interface name (always `hq0`; anything else was
edited in) and the server address are checked again every time `net.toml` is loaded, because they
end up in printed `sudo` commands, unit names and device configs. Device labels are checked on load
too, so a terminal escape in an edited file cannot reach `peer list`. Every printed path is
shell-quoted. `hq net apply` and `peer add` refuse to write an input that would remove a device the
hub has now unless that device was revoked here, so a damaged, emptied or restored-from-an-older-copy
`peers.toml` cannot remove everyone. If the applier's current input cannot be read or parsed, the
check cannot run and they warn instead of refusing, so a damaged input can still be replaced. Only the kernel WireGuard is supported: the
unit allows netlink sockets and nothing else, which a userspace implementation such as
wireguard-go would need.

| Path | What | Who writes it |
|---|---|---|
| `~/.hq/net/net.toml` | Subnet, hub address, port, endpoint, hub public key. Nothing secret. | `hq net init` |
| `~/.hq/net/peers.toml` | Devices and, once `hq net link add` has been used, their links. Mode 0600. An unknown field or table is an error, not an empty list. | `hq net peer ...`, `hq net link ...` |
| `~/.hq/net/retired` | Addresses of revoked devices, one per line. Kept apart from `peers.toml` so damage to that file cannot lose it. `hq net` never reuses them. The applier does not read this file, so a hand edit of it or of `peers.toml` can hand an address out again. | `hq net peer revoke` |
| `~/.hq/net/hub.key` | The hub's private key, until you move it in step 1. | `hq net init` |
| `~/.hq/tunnel/join.conf` | The join text for the in-app tunnel. Mode 0600: it may hold a preshared key. | `hq net tunnel import` |
| `~/.hq/tunnel/maps.toml` | The serve and reach entries and their limits. | you; import deletes it for a new hub |
| `~/.hq/tunnel/key` | The tunnel's private key, for a desktop run. Mode 0600, never overwritten. | `hq net tunnel keygen` |
| `~/.hq/tunnel/status.json`, `events.log` | What the running peer reports. No keys, no traffic. | `hq net tunnel run` |
| `/var/lib/hq-net/hq0.conf` | The applier's input. Owned by your user, mode 0600. Version 1 until the first link is added, version 2 after. | `hq net apply` |
| `/etc/hq-net/hq0.env` | Subnet, hub address and owner id the applier trusts. Root-owned. | you, step 4 |
| `/opt/hq-net/hq-net-apply` | The applier. Root-owned directory and file. | you, step 4 |
| `/etc/wireguard/hq0.{conf,key}` | The interface and its key. Root-owned. | you, steps 1 and 2 |
| `/etc/hq-net/hq_net.nft` | The firewall table with the set of linked pairs, if you link devices. Root-owned. | you, "Linking two devices" |

## What is not done yet

- Release builds do not include `hq-net-apply`, so step 4 builds it from source. Packaging it with
  the signed release is tracked separately because the updater replaces only `hq`.
- The real `wg` calls have run as root in a throwaway container (the ignored real-interface test:
  rust:1.98.1-slim-bookworm on Docker Desktop's Linux VM, kernel 7.0.14, wireguard-tools v1.0.20210914),
  creating and deleting one interface.
- The units and the applier have run under real systemd. `deploy/net/test/run.sh` starts the real
  units and the real applier in throwaway privileged containers (Debian 12 with systemd 252 and
  Ubuntu 22.04 with systemd 249, on the same Linux VM, nftables 1.0.6), puts three devices in
  network namespaces, and checks, each check printing PASS or FAIL: starting the tunnel applies the
  peers and the pair; the pair works both ways and a third device reaches neither; a file replaced
  by a rename is picked up by the path unit; a removed link and a revoked device are cut off;
  twenty rewrites in five seconds then a revoke still revokes and the path unit keeps watching; a
  missing table fails the run visibly while the peers are still applied (and, with forwarding on,
  leaves every pair open, as documented); another firewall's drop policy breaks the pair until
  `hq0` to `hq0` is allowed, after which the table still blocks unlinked pairs; a file with a
  `PostUp` line is refused and nothing runs; and `systemd-analyze security` scores the service 2.1
  (OK). That covers `PrivateDevices`, `SystemCallFilter=@system-service`,
  `RestrictAddressFamilies=AF_NETLINK`, the capability set, `nft -f -` and `wg` under them, the
  path unit on both systemd versions (249 ignores `TriggerLimitIntervalSec`; the burst of rewrites
  did not stop it) and glibc 2.35. What it is not: a real host, a real `ufw` or `firewalld`
  (iptables-nft with a drop policy stands in), or the droplet. Run the first apply there with a
  spare device and watch `journalctl -u hq-net-apply@hq0`. No CI job runs it.
- Linking has been run on a real kernel and under real systemd (above), not on a live hub. Still
  not checked: a real `ufw` or `firewalld`, whether a later write to the global
  `net.ipv4.ip_forward` undoes the per-interface setting (it rewrites every interface's value),
  and the droplet itself. In the container runs, with `hq0` forwarding at 1 and the global
  `ip_forward` at 0 and then at 1, an empty set forwarded nothing, one pair worked both ways while
  a third device reached neither, an empty script closed the pair, a script with a bad line left
  the set as it was, and with the table deleted and forwarding still 1 every device reached every
  other (why the table is included from the nftables configuration).
- The web UI and MCP are not yet served on the tunnel address (phase 2), and Tailscale is not
  removed by this command (phase 3).

### First run on a spare VM

`deploy/net/test/run.sh` does most of the checks below in containers (see above); it needs Docker
with WireGuard and nf_tables in the Linux kernel. Still run the list on a real machine before the
hub, because a container shares its host VM's kernel and networking.

Before this is trusted on the real hub, run it once on a throwaway Linux machine (a VM you can
break, not the hub). It needs: a kernel with WireGuard (5.6 or newer, or `wireguard-dkms`), the
`wireguard-tools` and `iproute2` packages, systemd, a Rust toolchain, and root.

1. The ignored test (it has passed once in a container; see above, but a container has no systemd,
   so the checks below still need a VM). It creates an interface called `hqnettest0`, applies one peer and then an
   empty list with the real `wg`, and deletes the interface; it touches no other interface,
   firewall rule or file. `sudo -E env "PATH=$PATH" cargo test -p hq-net -- --ignored --nocapture`
   should report one test passed.
2. The printed steps from `hq net init`, on that VM, then `hq net peer add` for a made-up public
   key. Look for `hq-net-apply: 1 peer(s) on hq0 after this run` in
   `journalctl -u 'hq-net-apply@*'`, and for the peer in `sudo wg show hq0`.
3. Touch the input file twenty times in five seconds, then revoke the device: it must still be
   removed, and `systemctl status` for the applier's path unit (instance `hq0`) must still say `active (waiting)`.
4. `sudo systemctl restart wg-quick@hq0`: the peer must be back within a few seconds. (Do not try
   `reload`.)
5. `systemd-analyze security` on the applier's service (instance `hq0`), and `systemctl status` after a run: the
   confinement directives have only been read against upstream documentation so far. If one makes
   the unit fail, the journal names it.
6. On Ubuntu 22.04 (systemd 249, glibc 2.35), confirm `wg` still runs under
   `SystemCallFilter=@system-service`: start the service and read the journal for an `EPERM` from
   `clone3` or `posix_spawn`. The ignored cargo test runs outside the unit's sandbox and cannot
   catch this. systemd 249 also ignores `TriggerLimitIntervalSec` (the man page marks it v250),
   so there the one-second pause before each run is the only bound on how often the applier starts.

## Tests

```
cargo test -p hq-net                                      # grammar, store, renderers, randomized tests
cargo test -p hq-net --test grammar_fuzz                  # the seeded randomized grammar tests
sudo -E cargo test -p hq-net -- --ignored                 # drives a real wg interface; Linux, root, wireguard module
deploy/net/test/run.sh                                    # the real units and applier under systemd 252 and 249, in Docker
```

The applier module and its tests compile only on Linux. On another system run them in a container
as a normal user.
