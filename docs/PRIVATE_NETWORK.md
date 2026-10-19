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
   reach another. Add the same table to your own firewall configuration to keep it after a reboot.
   If your firewall drops inbound UDP by default, allow `51820/udp` too.
4. Install the applier (below) and the three systemd units (service, path unit and timer).
5. Start the interface and enable the applier's service, path unit and timer; the exact `systemctl` commands are printed.

The command also reports the machine's real `net.ipv4.ip_forward` value. The hub does not need
forwarding and does not depend on it; step 2 and step 3 hold whether it is 0 or 1. Do not change
it for this.

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
| Parse | Printable ASCII only. First line `# hq-net peers v1`, last line `# hq-net end <n>` with the number of peers read. Only `[Peer]` sections with `PublicKey` (44 characters of base64, 32 bytes), `PresharedKey` (same, optional) and `AllowedIPs` (exactly one IPv4 address written `a.b.c.d/32`, inside the subnet, not the hub, not the network or broadcast address). No duplicate address or key, at most 64 peers. `[Interface]`, `PostUp`, `PreUp`, `Table`, `Endpoint`, `PrivateKey`, any other key, a key in another case, inline comments, tabs and carriage returns all refuse the whole file. |
| Apply | Only then `wg` is started, by absolute path in a distribution directory (and only if root owns it and others cannot write it; use `--wg` for another place), with an empty environment and no shell. It receives text the applier rendered from the parsed peers, never the file's own bytes. |
| Record | After each successful `wg` call it prints how many peers the interface has and what that run added, removed or moved (address and the first characters of the public key); an unchanged run prints one line, so the timer does not fill the journal. The record is written before the input is read again. If that second read or round fails, the error says how many peers were already applied. `journalctl -u 'hq-net-apply@*'` shows the history. |

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
laptop has to keep the path open. A phone's does not: the phone starts every exchange, and a timer
only costs data and battery.

`--psk` also makes a preshared key for the device. It is shown once, in the config, and kept in
`peers.toml`. Treat it like a password.

A device config always has one peer, the hub, and `AllowedIPs` is the hub's single address. HQ never
produces a full-tunnel config (`0.0.0.0/0`).

Other commands:

```
hq net peer list              devices, addresses, short key fingerprints
hq net peer revoke <label>    remove a device; its address is retired and hq net never reuses it
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

## Files

`net.toml` and `peers.toml` are read strictly. The interface name and the server address are
checked again every time the file is loaded (they end up in printed `sudo` commands and in device
configs), every printed path is shell-quoted, and `hq net apply` refuses to write an input that
would remove a device the hub has now unless that device was revoked with `hq net peer revoke`, so a
damaged or emptied `peers.toml` cannot remove everyone. Only the kernel WireGuard is supported: the
unit allows netlink sockets and nothing else, which a userspace implementation such as
wireguard-go would need.

| Path | What | Who writes it |
|---|---|---|
| `~/.hq/net/net.toml` | Subnet, hub address, port, endpoint, hub public key. Nothing secret. | `hq net init` |
| `~/.hq/net/peers.toml` | Devices. Mode 0600. An unknown field or table is an error, not an empty list. | `hq net peer ...` |
| `~/.hq/net/retired` | Addresses of revoked devices, one per line. Kept apart from `peers.toml` so damage to that file cannot lose it. `hq net` never reuses them. The applier does not read this file, so a hand edit of it or of `peers.toml` can hand an address out again. | `hq net peer revoke` |
| `~/.hq/net/hub.key` | The hub's private key, until you move it in step 1. | `hq net init` |
| `/var/lib/hq-net/hq0.conf` | The applier's input. Owned by your user, mode 0600. | `hq net apply` |
| `/etc/hq-net/hq0.env` | Subnet, hub address and owner id the applier trusts. Root-owned. | you, step 4 |
| `/opt/hq-net/hq-net-apply` | The applier. Root-owned directory and file. | you, step 4 |
| `/etc/wireguard/hq0.{conf,key}` | The interface and its key. Root-owned. | you, steps 1 and 2 |

## What is not done yet

- Release builds do not include `hq-net-apply`, so step 4 builds it from source. Packaging it with
  the signed release is tracked separately because the updater replaces only `hq`.
- The systemd units, the firewall table and the real `wg` calls were written to match the
  documentation and an independent reading of systemd's and wireguard-tools' source, not run on a
  live server in the change that added them. Run the first apply with a spare device and watch
  `journalctl -u hq-net-apply@hq0`. The test that drives a real interface is ignored by default;
  see below. A CI job that applies to a dummy WireGuard link in a user namespace is not built.
- The service carries confinement directives (capability bounding set, read-only system, no new
  privileges, no core dumps and others) that were not run under a live systemd either. If the unit
  fails to start, `systemctl status hq-net-apply@hq0` names the directive.
- The web UI and MCP are not yet served on the tunnel address (phase 2), and Tailscale is not
  removed by this command (phase 3).

### First run on a spare VM

Before this is trusted on the real hub, run it once on a throwaway Linux machine (a VM you can
break, not the hub). It needs: a kernel with WireGuard (5.6 or newer, or `wireguard-dkms`), the
`wireguard-tools` and `iproute2` packages, systemd, a Rust toolchain, and root.

1. The ignored test. It creates an interface called `hqnettest0`, applies one peer and then an
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

## Tests

```
cargo test -p hq-net                                      # grammar, store, renderers, randomized tests
cargo test -p hq-net --test grammar_fuzz                  # the seeded randomized grammar tests
sudo -E cargo test -p hq-net -- --ignored                 # drives a real wg interface; Linux, root, wireguard module
```

The applier module and its tests compile only on Linux. On another system run them in a container
as a normal user.
