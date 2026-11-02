# Desktop app updates (design, not built)

Status: proposal, waiting for the owner's yes. Nothing here is implemented.

## Goal

The macOS desktop app updates itself the way herdr-app does: check in the background, download a
new build beside the running one, and switch to it when the person quits or presses Restart. It
must not open a second, weaker path to running code on the machine. HQ already has one trusted
path, the signed release channel behind `hq update` (`docs/UPDATE_SYSTEM.md`), and the app reuses
its trust chain instead of adding Tauri's updater plugin with a second key.

## Trust chain

The same chain `hq update` checks, through `hq-update`'s library code:

1. Fetch `channel-<channel>.json` and its `.minisig` and verify the signature with the release
   public key compiled into the app (`release/minisign.pub`, the same key `hq` trusts). Check the
   pointer's channel, its `seq` against the last one seen (no replay), and that `manifest_url` sits
   under the configured release base (`ChannelPointer::parse` and the URL rules already in
   `hq-update`).
2. Fetch `manifest.json` and its `.minisig`, verify, check its SHA-256 against the pointer
   (`Manifest::parse`).
3. Look up the desktop artifact for this platform, `hq-desktop-<version>-darwin-aarch64.tar.gz`,
   and download it with the size cap from the manifest, checking its SHA-256.
4. Unpack with `hq_update::archive::extract_tree` (strict tar reader: no absolute paths, no `..`,
   no links that leave the tree, size and entry limits).
5. Check the unpacked bundle before it can ever run: `codesign --verify --strict` and, on a signed
   build, that its Team ID equals the running app's own (`codesign -dv`), and `spctl --assess`.
   A notarized app that passes these is what Gatekeeper would accept anyway; the minisign check
   above is what stops a validly signed but unreleased build.

Only `Engine`'s parts that are about checking are reused: `manifest`, `archive`, the minisign
verification and the URL rules. `Engine` itself is built around a service manager, database
snapshots and health checks, none of which the app has.

## Staging and the swap

- The new bundle is unpacked to `<App>.app.new.noindex` next to the running one (Spotlight skips a
  `.noindex` folder, so search never shows two HQ apps), on the same volume so the swap is a
  rename.
- The swap happens when the app quits (it already holds quit until it has closed everything, see
  "What it does"), or when the person presses Restart in the window. It renames the running
  bundle to `<App>.app.old.noindex`, renames the new one into place, and on Restart opens it with
  `open -n`. The old bundle is deleted on the next start, after the new one is running.
- Crash recovery, at start: a `.new.noindex` with no matching running version is applied before
  the window opens; an `.old.noindex` while the current bundle fails `codesign --verify` is renamed
  back. A failed swap leaves the old bundle in place and records the version as blocked for a day,
  like `hq update`'s blocked versions.
- An app that cannot write its own folder (installed in `/Applications` by another user, or run
  from the DMG or a translocated path) does not try: it shows that a newer version exists and links
  to the download.

## What the release pipeline adds

- The `build` matrix gains a desktop leg on `macos-15` that builds `HQ.app` unsigned, as
  `desktop-release.yml` does (PR #375). A sign job with the Apple secrets signs and notarizes it
  (`scripts/desktop/sign-macos.sh`) and packs `hq-desktop-<version>-darwin-aarch64.tar.gz`.
- The `package` and `sign` jobs list that file in the manifest beside the `hq` artifacts. Existing
  updaters ignore it: `Manifest::parse` accepts artifacts it does not know. `release.yml`'s check
  of the exact artifact set must list it, and `scripts/release/verify.sh` checks it like the rest.
- The app and `hq` share a version, so one release carries both.

## What the person sees

Settings gains Updates: off, check only, or download and switch on quit (the default once
releases are notarized). The window shows "Version X is ready, restart to use it". Nothing
updates in the background while the host is starting or stopping. A build from source never
updates (the check is compiled in only for release builds, as herdr does with a build-time flag).

## Not in this design

- Updating `hq` itself from the app. `hq` keeps its own updater; when `hq` is later bundled inside
  the app (`bundle.externalBin`), it updates with the app.
- Delta updates, Windows and Linux.
- Rollback to an older version by hand; a new download covers it.

## Open questions for the owner

1. Ship this, or keep "a new version is a new download" until the app has outside users?
2. Default setting: check only, or download and switch on quit?
3. Same channel as `hq` (`stable` / `main`), or a separate desktop channel?
