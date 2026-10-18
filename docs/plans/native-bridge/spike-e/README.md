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

## E3: a minimal app that runs `hq` as a foreground service, on the 3 GB emulator (Android 16, API 36)

The app is `apk/` (package `example.hq.spike`, `minSdk` 26, `targetSdk` 35, plain Java, no Gradle): `build.sh` builds it with the SDK command line tools (`aapt2`, `javac`, `d8`, `zipalign`, `apksigner`) from the patched `hq` binary and the built web app, and generates a debug keystore at build time (never committed). `HubService` is a `specialUse` foreground service that runs `libhq.so` (the renamed `hq` executable, from `nativeLibraryDir`) as `hq web` under `HQ_PROFILE=lite`; `MainActivity` shows the UI in a WebView after `/health` answers. Scripts and raw output are under `scripts/` and `evidence/`. Everything below is the emulator (a stock Google APIs image, no vendor power manager) unless said otherwise.

| Check | Result |
|---|---|
| 1 Exec | `libhq.so` runs from `nativeLibraryDir` (`.../lib/arm64/libhq.so`, 78,954,424 bytes) at `targetSdk` 35: `hq 0.9.1 (c39a328...)`. The same file copied into `filesDir` and made executable is refused: `error=13, Permission denied`. `extractNativeLibs="true"` and the uncompressed `lib/arm64-v8a/libhq.so` entry work; no `useLegacyPackaging` setting was needed because there is no Gradle here |
| 2 Survival | The hq child and the app survived Home plus 60 s, screen off for 300 s, swiping the app away in recents (service still `isForeground=true`, type `specialUse`), and `am kill`. `kill -9` of the app process: `START_STICKY` restarted the service and a new hq child within 0.1 s. `kill -9` of the hq child alone: the spike does not respawn it, so a real shell needs a supervisor loop with backoff. `am force-stop` ends everything and nothing restarts it, as expected |
| 3 WebView (Chrome 133) | Sign-in by `#token=`, `POST /api/ws-ticket` then `/ws` opens, a service worker registers and becomes ready, `isSecureContext` is true, `/health` and `localStorage` work and persist across restarts, no console errors or CSP violations. This holds at `http://127.0.0.1:5678` and at `http://localhost:5678` (iOS needed a host name; Android does not). `PushManager` and `Notification` are absent in the WebView, so notifications must come from the shell. Cleartext: with no network security config, or a config that lists only the other name, requests fail with `net::ERR_CLEARTEXT_NOT_PERMITTED`; the entries match the exact host string, so `127.0.0.1` and `localhost` each need their own |
| 4 Phantom processes | 20 minutes (the brief asks for 30): the service spawned `hq reindex` over a 2,000 note vault every 10 s as a child of the app (75 runs, 5.8 to 6.4 s each, all rc 0) plus four searches each time, next to the long-lived `hq web` child. No phantom trimming or excessive CPU kill lines; the same app process and the same server pid throughout. `max_phantom_processes` is 32. The server child's RSS drifted from about 70 MB to about 120 MB over the 20 minutes, with dips and no proof of a plateau, so it needs watching in a longer soak |
| 5 Doze | With the foreground service, outbound requests kept working in forced deep Doze (`cmd deviceidle force-idle deep`, screen off, unplugged): 0 failures in 60 s, 0 failures with the app on the allow list, and 0 failures with the App Standby bucket at `rare`. `/health` on the hq child kept answering. This is forced idle on a stock image; a real device and a vendor battery manager are not covered |
| 6 Boot | After `adb reboot` a `BOOT_COMPLETED` receiver started the `specialUse` foreground service and hq at `targetSdk` 35 on Android 16 (`BOOT start accepted`, `/health` 200). An app that was force-stopped gets no boot broadcast until it is launched once |
| 7 Cold start | Process start to `/health`: 0.56 to 1.1 s; first paint (`onPageFinished`) 0.8 to 2.0 s after the activity starts |
| 8 Size | `libhq.so` 78.95 MB. APK with the library stored uncompressed: 82,873,373 bytes. APK with it compressed, which is the download size: 40,459,293 bytes |

Memory of the whole experience (`dumpsys meminfo`, a few seconds after the UI loads): the WebView renderer process 112 MB PSS, the app process 101 MB, and the hq child about 46 MB; steady state of the hq child after a restart is about 43.5 MB RSS. On the very first run after an install it reached 65 to 75 MB (92 MB once) while it set up its data, and it kept that. The UI, not hq, is the larger share.

Register: A5 answered, A6 answered (closes P3 for Android on this image), A7 answered for the emulator, A8 answered for 20 minutes on the emulator, A11 answered. Not done: the 30 minute soak and the 24 hour run, the real-device rows (a Samsung Note 10 on Android 12 is available), vendor battery managers, and E4 to E7.

Design points that follow: ship `hq` as `libhq.so` and exec it (the in-process library design is not needed for the exec rule or for survival on this image); add a supervisor loop for the child; draw notifications from the shell; list `127.0.0.1` and `localhost` in the network security config, or pick one name and use it everywhere; keep the loopback token and origin guard on (the app's token is random per install and lives in `SharedPreferences`).
