# Runbook: the real-network rows of Spike F

Results from the first pass (a phone over USB, one carrier, one home router) are in the README.
The hotspot and public wifi rows are the ones still open.

The loopback tests and the emulator runs show that pairing works. They cannot show how it behaves on
the networks people actually use. This runbook fills the rows still empty in the README: mobile data,
a carrier hotspot, public wifi, a network that blocks UDP, sleep and wake, a network change in the
middle of a session, and a real camera scan. About an hour of hands-on time, plus travel for the
public wifi row.

Every row measures the same few things, so the answers can be compared and the relay default can be
chosen from data (plan question R1 to R5).

## Set up once

- A Mac (or any machine) at home as the host, a phone as the joiner, a USB cable. The phone must not
  be one that is serving a live HQ instance: these rows turn its wifi and data on and off.
- Build the phone binary for the current protocol (earlier emulator runs used protocol v1):
  `cargo build --release --target aarch64-linux-android` in this folder, with the NDK's clang as in
  Spike D. Then `adb -s <serial> push target/aarch64-linux-android/release/pairspike /data/local/tmp/`
  and `adb -s <serial> shell chmod +x /data/local/tmp/pairspike`. Always pass `-s <serial>`.
- On the host: `B=target/release/pairspike; touch paired.txt; chmod 600 paired.txt`.
- The relay is n0's public one (`--relay default`), which is meant for development and testing. These
  runs send a few kilobytes through it, nothing else.

## One pairing, used by every row

1. Host: `$B show --key host.key --paired paired.txt --relay default --serve`. It prints a QR code and
   a `hqpair1.` ticket. Start a stopwatch.
2. Phone, in `adb shell` from `/data/local/tmp`:
   `./pairspike join --ticket <ticket> --key dev.key --store dev.json --relay default`.
   It prints six digits.
3. Host: type those six digits when asked. The phone prints `paired with ...`. Stop the stopwatch.
4. Reconnect by key alone, from the phone:
   `./pairspike dial --key dev.key --store dev.json --relay default --calls 20`.
   It prints `connected in <time> path=direct|relay quic_rtt=...` and `p50`/`p95` for 20 calls.

Pair once per row with a fresh ticket (they work once and last ten minutes). Delete `dev.json` and
the host's `paired.txt` lines between rows if a row starts from "never paired".

## The rows

| # | Setup | Do | Write down |
|---|---|---|---|
| 1 | Phone: wifi off, mobile data on, USB for adb. Host stays on home wifi | The pairing, then `dial` | Pair time, `path`, connect time, p50 |
| 2 | Host joins a hotspot from a second phone; the joiner phone stays on home wifi | The pairing, then `dial` | Same |
| 3 | Joiner on public wifi (cafe, guest network); host at home | The pairing, then `dial` | Same, and whether the network isolates clients |
| 4 | UDP blocked (see below), relay reachable | The pairing, then `dial` | Whether pairing works, `path` (expect `relay`), p50 |
| 5 | Row 1 paired and `dial --keep` running; lock the phone for 10 minutes, then wake it | Watch the `--keep` output | Seconds from wake to the first `ok`, `path` before and after |
| 6 | `dial --keep` running on wifi; turn wifi off so it falls back to mobile data, then back | Watch the output | Gap in `ok` lines, `path` after each change |
| 7 | Any network | Scan the host's QR with the phone's stock camera app, copy the ticket text it shows, run `join` with it | Scanned first try (yes/no), tries, seconds from QR on screen to ticket in hand, and whether `--no-addrs` (smaller QR) scans better |

Row 4 needs a network that drops outbound UDP. The simplest honest source is a guest or office
network that does; otherwise on the host side only, as an administrator on macOS:
`echo "block drop out quick proto udp from any to any" | sudo pfctl -f - -e` (this also drops DNS over
UDP, so resolve with a hosts entry or accept the failure), and undo it with `sudo pfctl -d`. Do not
leave it on. A block on UDP leaves the TCP relay as the only way through, which is the case that decides
whether the relay must work out of the box.

For row 5, also let the host Mac sleep for a few minutes in a second run; `--keep` on the phone shows
whether the phone reconnects on its own once the Mac is back.

## What decides what

- **Rows 1 to 3 direct, or relay then direct within seconds:** direct-first is right and the relay is
  only a safety net. The relay can stay a single config value with no default hosted by the project.
- **Rows 1 to 3 mostly `relay`:** a relay is part of the product, so the default question (self-hosted,
  paid, or n0's public service) becomes a launch requirement.
- **Row 4 fails:** pairing needs the TCP relay and the plan must say so on the first screen.
- **Rows 5 and 6 slow (more than about ten seconds):** the app needs its own reconnect and wake logic,
  and the idle cost of 0.28 MB an hour argues for connecting only when the app is open.
- **Row 7 unreliable:** the ticket must shrink (`--no-addrs`) or the QR must be shown larger than a
  terminal can.

Put the numbers in a copy of the table above and add it to the README's results.
