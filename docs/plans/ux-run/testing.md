# Testing on a laptop and a phone

How the owner tries each wave on real devices, and how the building agent hands a wave over. Exact
commands come from the repo's own docs; follow those if they change.

## Before every hand-over (the agent)

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            # read the totals: a partial run is not a green run
cd apps/hq-web && bun install && bun run build && bun test
cd apps/android && ./gradlew :app:assembleDebug :app:testDebugUnitTest :app:lintDebug
cd apps/desktop && bun install && bun run typecheck     # when the hub changed
cargo test -p hq-desktop          # includes shell_config.rs (commands, capabilities, no plugins)
```

Then write the wave's "Try this now" note in `progress.md`: what to install, the steps, and what the
owner should see, one line each.

## The laptop

1. Build and install hq: `./scripts/install-hq.sh` (once per machine, `sudo ./scripts/install-hq.sh --link`).
   `./scripts/install-hq.sh --check` says whether anything is stale. Do not compare `--version`.
2. Open the web app: `hq web --json` starts it detached and prints `login_url`. Open that address.
3. For web work in progress: `cd apps/hq-web && bun run dev` serves on :4747 and proxies `/api` and `/ws`
   to hq on :5678.
4. The desktop hub: `cd apps/desktop && bun install && bun run build:ui && bun run tauri dev` for a dev
   window, or `bun run build` for the app. See `docs/DESKTOP.md` for pairing and the HQ window.
5. Shared-computer and signed-out checks: a second browser profile, signed in with a one-time code from
   Settings › Other devices.

## The phone

1. Build a test copy that installs beside the real app, with its own data (`apps/android/README.md`):

   ```bash
   export JAVA_HOME=<a JDK 17 or newer> ANDROID_HOME=<the Android SDK>
   cd apps/android
   ./gradlew :app:assembleDebug \
     -Phq.bin=<hq aarch64-linux-android> -Phq.caddy=<caddy linux arm64> \
     -Phq.web=<apps/hq-web/dist/client> -Phq.keystore=<the signing key> \
     -Phq.appId=<a test id>
   adb install -r app/build/outputs/apk/debug/app-debug.apk
   ```

   Build `hq` for Android with `--features tunnel,bridge` (the recipe in `.github/workflows/android.yml`).
   Keep the `-P` paths in `~/.gradle/gradle.properties`, never in the repository.
2. Never build a release locally. Releases come from CI (`release.yml`), and the update channel
   (`docs/UPDATE_SYSTEM.md`).
3. Phone as the hub: pair the laptop with `hq bridge` or HQ Hub's Devices tab (`docs/PAIRING.md`), then
   open HQ in HQ Hub; sign in once with a code from the phone (Settings › Other devices).

## Devices to cover

Per wave, from `mobile/device-checks.md` and `web/device-checks.md`:

- Phones: a Galaxy A32 class phone (Android 13), one Tecno, Infinix or itel phone, one Android Go phone,
  and Android 8.1 and 10 emulators.
- Browsers: Chrome, Firefox and Safari on the laptop; the HQ window inside HQ Hub; one browser at 320 px
  and at 200% zoom; Windows forced colours if a Windows machine is at hand.
- Networks: Wi-Fi, mobile data (metered), a paid-by-the-MB Wi-Fi (a phone hotspot marked metered and not),
  airplane mode mid-reply.

## Owner checkpoints

- **After web wave 0:** confirm the four live bugs are gone (see web/wave-0/01-live-bugs.md "Check on a
  laptop or phone").
- **Before phone wave 3:** log the locked Home hold on the A32 and a Pixel image (Pass, Fail A or Fail B,
  mobile/wave-3). The agent waits for this result.
- **After each wave:** the owner tries the "Try this now" steps and lists what is wrong in `progress.md`;
  those become the next fixes before the next wave starts.
