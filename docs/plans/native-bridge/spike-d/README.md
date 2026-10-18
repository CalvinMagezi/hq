# Spike D: iroh on a phone

Throwaway crate, outside the HQ workspace. It answers the plan's group P: can iroh run on iOS and
Android, how big is it, does the web app work inside a phone WebView, and is a native mobile app
worth building next to the installed web app. `mobilehello` is the device role of Spike C without
the coding-agent host: it dials the hub by id and address, then answers whatever the hub asks
with a canned `host.status`.

Run on 2026-10-18: Xcode 27.1 with an iOS 27 simulator, Android NDK 28.2 with an arm64 Android 16
emulator, Rust 1.98, iroh 1.3.0. A real iPhone or Android phone was not used.

## Results

| Target | Build | Run |
|---|---|---|
| `aarch64-linux-android` | Builds with the NDK, no source changes, about 1 minute | Android 16 emulator: dialed the hub through the emulator's NAT, direct path, connected in about 30 ms, the hub called down the connection (transport RTT 0.6 to 1.3 ms) |
| `aarch64-apple-ios-sim` | **Fails as published**; builds after two small patches (below) | iOS simulator: dialed the hub, direct path, connected in about 30 ms, hub called down it |
| `aarch64-apple-ios` (device) | Same: fails as published, builds after the patches | Not run (needs a signed app on a phone) |

Binary size, hello world including iroh, tokio and serde_json:

| Target | Default release | Size-optimized (`opt-level=z`, LTO, strip, `panic=abort`) |
|---|---|---|
| iOS | 13.6 MB | 3.1 MB |
| Android | 19.4 MB | 4.3 MB |

### Why iOS needs patches

iroh 1.3.0 does not compile for any iOS target, because two dependencies use `libc` symbols that
`libc` defines for macOS only:

- `netdev` 0.46.3 calls `libc::in6_ifreq` for every Apple target.
- `netwatch` 0.19.3 treats iOS as a BSD and uses the routing-socket constants `RTM_*`, `RTAX_*`
  and `RTF_GATEWAY`, which `libc` also has for macOS only.

`libc` 0.2.191 is the latest release, and iroh 1.1.0 resolved fresh hits the same errors. The
prebuilt Swift package of `iroh-ffi` has builds for iroh 1.0 and 1.1 on the Swift Package Index,
so it probably builds from an older lockfile; that was not checked.

`patches/netdev-ios.patch` gates the macOS-only function. `patches/netwatch-ios.patch` makes iOS
take netwatch's existing minimal path, so on iOS it has no interface enumeration through the
route table and no route-change events. That costs network-change detection (N5), which an app
would have to supply itself (for example with `NWPathMonitor`), or the crates would have to be
fixed upstream. The fix upstream is small; reporting it to n0-computer is worth doing before any
mobile work.

## Repeat it

```
cd docs/plans/native-bridge/spike-d
./prepare.sh                      # iOS only: vendors and patches netdev and netwatch
rustup target add aarch64-apple-ios-sim aarch64-apple-ios aarch64-linux-android

# iOS simulator
cargo build --release --target aarch64-apple-ios-sim
xcrun simctl boot <an iPhone simulator>
xcrun simctl spawn booted target/aarch64-apple-ios-sim/release/mobilehello \
  --hub <HUB-ID> --addr 127.0.0.1:<port> --key sim.key --secs 25

# Android: set the NDK clang for cargo, then run it in an emulator
TC=$ANDROID_HOME/ndk/<version>/toolchains/llvm/prebuilt/darwin-x86_64/bin
export CC_aarch64_linux_android=$TC/aarch64-linux-android24-clang \
       AR_aarch64_linux_android=$TC/llvm-ar \
       CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$TC/aarch64-linux-android24-clang
cargo build --release --target aarch64-linux-android
adb push target/aarch64-linux-android/release/mobilehello /data/local/tmp/
adb shell /data/local/tmp/mobilehello --hub <HUB-ID> --addr 10.0.2.2:<port> --key /data/local/tmp/k --secs 25
```

Start the hub from `../spike-c` on the host (`bridgespike hub --port <port> --key hub.key
--paired paired.txt`), run the device once to learn its id, add that id to `paired.txt`, run it
again. The simulator shares the host's network; the Android emulator reaches the host at
`10.0.2.2`.

## P3: the web app inside a phone WebView (iOS)

A 60-line Swift app (a `WKWebView` that loads the web UI from a loopback server, with the same
checks as the Tauri test in `../evidence/spike-b`) ran in the simulator:

| Check | Result |
|---|---|
| Plain `http://127.0.0.1:<port>` | Page loads, `#token=` sign-in works, chat ticket 200, chat WebSocket opens, no CSP violations. App Transport Security does not get in the way of a loopback IP address |
| Service worker | `navigator.serviceWorker` is **undefined** in a plain WKWebView |
| With App-Bound Domains (`WKAppBoundDomains` = `localhost` in Info.plist, `limitsNavigationsToAppBoundDomains = true`) and the page loaded from `http://localhost:<port>` | Service worker registers, WebSocket opens, no CSP violations. It must be a host name, so `localhost`, not `127.0.0.1` |
| `PushManager`, `Notification` | Absent in both cases. Web Push does not exist inside an app WebView, so alerts must come from the app |

Not tested: `WKWebsiteDataStore.proxyConfigurations` (not needed if the app serves the page from
its own loopback server), Android WebView and `ProxyController` (no test app was built; the
emulator is there for a later look), the in-app loopback server itself.

## P2: Tauri mobile (from the vendors' docs, 2026-10-18)

- Official plugins that list iOS and Android support: notification (local only), deep-link,
  barcode-scanner, biometric, http, opener, os, store, sql. `process` is untested on both.
  Source: the tauri-apps/plugins-workspace README.
- Secure storage: no official Keychain or Keystore plugin. Stronghold's status on mobile is
  contradictory between the workspace table and its own README; a community plugin exists and is
  young.
- Remote push: the official notification plugin is local only. A community plugin
  (`tauri-plugin-notifications`) covers FCM and APNs.
- The Tauri team's own statements on mobile are from 2024 (the stable release post said not every
  plugin is ported); nothing newer was found. Core was 2.12.x in October 2026.

## P4: push notifications, the decision note

The problem: an app cannot hold a background connection on iOS, and on Android a held connection
costs battery and data. Alerts for a blocked or finished agent therefore need APNs on iOS and FCM
(or a persistent foreground service, or UnifiedPush) on Android.

- The credentials belong to whoever publishes the app. Apple signs push requests with the
  publisher team's key and does not let one connection serve several teams; FCM refuses tokens
  that belong to another sender. A self-hosted HQ cannot push to a project-published app by
  itself.
- What projects do: Home Assistant and Element (Sygnal) run a gateway that holds the app's
  credentials and relays; ntfy relays only a message id and a hash of the topic for iOS; Mastodon
  uses relays that can be blind to content; UnifiedPush is Android and Linux only. Content-free
  "something happened" pushes keep the gateway from seeing anything private.
- What the installed web app already does: iOS 16.4 and later supports Web Push for apps added to
  the Home Screen (no developer account), and Chrome on Android delivers Web Push through FCM with
  the browser closed. Neither needs a project-run relay. That is HQ's current path (VAPID).
- Decision for Calvin: a native app needs a project-run push gateway for iOS (and for Android
  unless it ships a UnifiedPush or foreground-service build). That is a new central service, an
  Apple account and a privacy surface. Until that is accepted (D7), the default stays Web Push in
  the installed web app.

## P5: stores and reach

Apple Developer Program $99 a year; Google Play $25 once, and a new personal account needs a
closed test with 12 testers for 14 days. Sideloaded Android apps face Google's developer
verification from 2026-09-30 in four countries and globally in 2027; Uganda is not named. F-Droid
needs a build without Firebase or Play Services. For an app that shows remote terminals, Apple's
guidelines 4.2 (more than a repackaged website) and 2.1 (a demo account and a live backend for
review) are likelier to matter than 2.5.2. Android is 93.1 percent of mobile in Uganda (StatCounter,
September 2026), so Android first (D6) holds. Small data bundles cost about UGX 2,500 to 3,250 a
GB, from news reports of MTN and Airtel prices in August and September 2026.

## P6: does a native app beat the installed web app

Not yet. For most Ugandan owners (Android, no extra paid accounts) the installed web app already
gives install, Web Push through FCM and no store account. What a native app would add is reaching
HQ without a VPN app or a public web address, and that is only worth a whole app if owners
refuse both. The measured idle cost also argues against it today: an always-connected iroh
device used about 750 bytes a second idle in Spike C, which is about 2 GB a month, one full
small data bundle, against close to nothing for Web Push.

Decide later, with these triggers: owners who will not use Tailscale or a public HTTPS address;
an idle cost under about 100 kB an hour; iroh building for iOS without patches; and a decision on
the push gateway.
