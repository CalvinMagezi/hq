# Spike E: HQ as a hub on Android (results so far)

Brief and register: the vault notes `Android-Hub-Spike-E-Handoff` and `Android-On-Device-Assessment`. Nothing here changes product code on this branch: the one change is a patch under `patches/`, applied to a throwaway worktree by `prepare.sh`.

Run on 2026-10-18 on an Apple Silicon Mac, rustc 1.98.1, Android NDK 28.2.13676358, API 26 clang, base commit `f81d703` (main).

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

Not done yet: the `hardware.rs` and `set_non_dumpable` cfg corrections (they change behaviour, not the build), E2 (memory on an emulator), E3 and later.
