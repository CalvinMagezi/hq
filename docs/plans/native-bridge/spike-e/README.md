# Spike E: HQ as a hub on Android (results so far)

Brief and register: the vault notes `Android-Hub-Spike-E-Handoff` and `Android-On-Device-Assessment`. Nothing here changes product code on this branch: the one change is a patch under `patches/`, applied to a throwaway worktree by `prepare.sh`.

Run on 2026-10-18 on an Apple Silicon Mac, rustc 1.98.1, Android NDK 28.2.13676358, API 26 clang. E1 ran on `f81d703` with patch 0001 only; E2 ran on `c39a328` (current main) with patches 0001 to 0003, and `prepare.sh` now uses that base.

## E1: build the real `hq` for arm64 Android

Environment, with `$TC` the NDK's `toolchains/llvm/prebuilt/darwin-x86_64/bin`:

```
export ANDROID_NDK_HOME=<ndk> ANDROID_NDK_ROOT=<ndk> PATH=$TC:$PATH
export CC_aarch64_linux_android=$TC/aarch64-linux-android26-clang
export AR_aarch64_linux_android=$TC/llvm-ar
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$TC/aarch64-linux-android26-clang
rustup target add aarch64-linux-android
cargo check -p hq-cli --target aarch64-linux-android --keep-going
cargo build -p hq-cli --profile release-small --target aarch64-linux-android --locked
```

| Step | Result |
|---|---|
| `cargo check`, unpatched | One failure: the `openssl-sys` build script cannot find OpenSSL (`OPENSSL_DIR` unset, no pkg-config). No `error[E...]` from rustc. The crates that do not depend on native-tls (`hq-convert`, `hq-export`, `hq-host`, `hq-sandbox`) checked clean. The three other C build scripts the assessment feared (`libsqlite3-sys`, `aws-lc-sys`, `libmimalloc-sys`) did not fail with the NDK's clang |
| Patch 0001 (one line: vendor OpenSSL for `android` as well as `linux`) | `cargo check` exits 0: no errors across the whole `hq-cli` dependency graph |
| `cargo build --profile release-small --locked` with the patch | Exit 0, 298 s wall clock from a cold target directory. C crates built with the NDK: `openssl-src` 300.6.1+3.6.3, `aws-lc-sys` 0.45.0, `libsqlite3-sys` 0.31.0, `libmimalloc-sys` 0.1.44 |
| Binary | ELF 64-bit aarch64 PIE, interpreter `/system/bin/linker64`, stripped: 78,803,768 bytes. gzip -6: 36,410,780. xz -6: 24,948,048. The Linux x86_64 reference in the assessment is 88.1 MB, 35.4 MB gzipped |
| 16 KB alignment | All 4 LOAD segments say `align 2**14` (`llvm-objdump -p`), so the Play rule for 2027 is met without extra flags |

Register: A1 answered (builds with one one-line patch), A2 answered, A10 answered (no Rust-level errors in any of the 17 crates), A9 partly (a vendored OpenSSL builds; roots at runtime are E6).

Patches 0002 (memory detection) and 0003 (`set_non_dumpable`) are the two cfg corrections from the assessment's B2. They change behaviour, not the build. The patched build is 78,954,424 bytes and links in 281 s.

## E2: Lite `hq web` and the daemon on an arm64 Android 16 emulator

Two throwaway AVDs from the installed `system-images;android-36;google_apis;arm64-v8a` image, 4 cores, 2 GB and 3 GB of RAM (the guest reports `MemTotal` 2,531,868 kB and 3,046,940 kB), started headless. `scripts/e2.py` pushes the binary and the built web app, starts `hq web` under `HQ_PROFILE=lite`, and follows the Linux reference run: an empty vault, then 2,000 synthetic notes (49,964,039 bytes of markdown, from a different generator than the reference run), `hq reindex` sampled for its peak, four searches and one repeat, then 45 s idle. Raw output is in `evidence/`.

| Moment | Linux x86_64 reference (VmRSS) | 2 GB AVD | 3 GB AVD |
|---|---|---|---|
| Start, empty vault | 68,976 kB, 11 threads | 37,888 kB, 11 threads | 38,528 kB, 11 threads |
| 30 s later | 69,176 kB | 37,312 kB | 30,008 kB |
| Start on the 2,000 note vault, before indexing | 74,960 kB, 14 threads | 38,144 kB, 14 threads | 36,864 kB, 14 threads |
| `hq reindex` process peak (VmHWM) | about 97,464 kB | 88,064 kB | 89,984 kB |
| After reindex and searches | 92,816 kB | 57,376 kB | 57,116 kB |
| 45 s idle after that | 93,192 kB | 57,888 kB | 57,244 kB |

PSS from `dumpsys meminfo` is within 3 MB of RSS in every row. Start to `/health`: 0.24 s and 0.45 s on a warm device, against 1.9 s cold on the reference. `hq reindex`: 4.7 s and 5.3 s (reference 20.5 s), `vault.db` 64.3 MB. Search: 20 to 43 ms. Index and search times are not comparable with the reference: the host is a fast Mac, and the synthetic text draws from a small vocabulary. The memory rows are the comparable ones.

Daemon: `hq start daemon` runs under Lite with the vault watcher on the `Native` (inotify) backend, 7 scheduled tasks, 33,280 kB RSS and 9 threads, next to `hq web` at 37,888 kB. Together about 71 MB with a 200 note vault, 0.0 percent CPU at idle.

Storage (`scripts/e2b.py`): with the vault on `/data/local/tmp` and again on `/sdcard` (shared storage), `vault.db` opens in WAL mode (`-wal` and `-shm` files present), the daemon reports "watching Notebooks with file events", and a note written from outside with `adb shell` was searchable within 0.1 s. That one number does not separate the watcher from a read-time path, so it does not by itself prove event delivery on shared storage. `/data/local/tmp` is not an app's private directory; the real `filesDir` check is E3.

Memory detection (patch 0002): `hq models` under the full profile reports "CPU (2GB RAM) total" on the 2 GB AVD. Unpatched, the code assumes 16 GB (read in `hardware.rs`; the unpatched binary was not run).

Thresholds in the brief: idle within 1.5 times the reference (about 105 MB), under about 150 MB after the workload, alarm over 250 MB idle. Both AVDs are far inside the comfortable band.

Register: A3 answered, A12 answered, A4 partly (emulator, not an app's private storage). Not done: the real-device rows, E3 and later.
