# Three places: handover for verification on a phone

Branch: `claude/ui-redesign-notes-chat-inbox-epc34r`. No pull request is open yet.
Plan and reasoning: `docs/plans/three-places.md`. Read that first.

Written 2026-11-10 by the cloud session that made the change, for a local agent with a
machine that has the Android SDK and the Samsung A32 on `adb`.

## Where things stand

| Part | State |
|---|---|
| Server (Rust) | Tested. Clippy clean. `cargo test --workspace --no-fail-fast`: 6012 passed, 31 failed, all in crates this branch does not touch and all environmental in the cloud container (see "Known noise"). Rerun locally to confirm. |
| Web and desktop | Typecheck, `bun test` and `bun run build` pass. Checked in a running `hq web` with a browser: the bar, live inbox refresh, Mark all as read, Silence and the badge. |
| Android | **Never compiled.** The cloud session could not install the Android SDK. The Kotlin and Java edits were written by reading the code only. Treat a compile error as likely, not surprising. |

## Do this first: compile the Android app

```bash
export JAVA_HOME=<a JDK 17 or newer> ANDROID_HOME=<the Android SDK>
cd apps/android
./gradlew :app:assembleDebug :app:testDebugUnitTest :app:lintDebug --console=plain
```

Files touched on Android, in the order a compile error is most likely:

1. `app/src/main/kotlin/example/hq/spike/ShellActivity.kt`: a `LaunchedEffect(replying)`
   that reads the inbox every `CHAT_NEEDS_POLL_MS` while a reply runs; the `needs = { ... }`
   argument to `ChatScreen`; two new imports (`ChatNeeds`, `needsOfChat`).
2. `ui/inbox/InboxScreen.kt`: new `ChatNeeds` composable (calls the file-private `NeedCard`)
   and the pure function `needsOfChat`.
3. `ui/shell/HqShell.kt`: `Place` gained `onBar` and a `companion object { val bar }`;
   `BottomBar` loops over `Place.bar`.
4. `ui/chat/ChatScreen.kt`: new `needs: @Composable () -> Unit = {}` parameter, drawn above
   the composer.
5. `ui/inbox/InboxViewModel.kt`: `suspend fun refreshNow()`.
6. `app/src/main/java/example/hq/spike/Alerts.java`: `itemsOf` skips an item whose
   `silenced` is true.
7. New test `app/src/test/kotlin/example/hq/spike/ui/shell/ThreePlacesTest.kt`.

The custom lint fails the build on literal UI strings. The one new string used on the phone is
`R.string.chat_needs_ok_title`, generated from `apps/strings/en.yaml` by
`bun scripts/strings/build.ts`. `chat_needs_ok_open` is in the catalogue and unused; remove it
or use it.

Fix what does not compile, keep each fix small, and commit on this branch.

## Then: install on the Samsung A32 and check

Build a phone APK as `apps/android/README.md` describes (`-Phq.bin`, `-Phq.caddy`, `-Phq.web`,
`-Phq.keystore`; `hq` built with `--features tunnel,bridge`, the web app from this branch's
`apps/hq-web/dist/client`). Use `-Phq.appId=<id>` if the phone's real app and data must stay
untouched. Never put a device serial, address or token in the repository.

Check each of these and write down pass, fail or not tested:

1. **The bar.** Three items: Notes, Chat (raised, in the middle), Inbox. Spacing looks right
   with three instead of five on the A32's width. Back from Notes or Inbox goes to Chat.
2. **Tasks and Helpers still open** though they are off the bar:
   `adb shell am start -a android.intent.action.VIEW -d hq://tasks` and `-d hq://helpers`,
   a search result for a task and for a helper, and "Take a look" on an Inbox card. On these
   pages no bar item is lit; Back returns to Chat.
3. **The reported bug.** In Chat, ask for something that needs approval after the chat has
   read outside text (for example: search the web for a topic, then ask it to open one of the
   result pages). Expected, all three:
   - a notification arrives;
   - the request shows in the Inbox under Needs you;
   - the same card shows in the chat above the composer within about four seconds, with
     Decline and Allow once.
   Allow it from the chat card: the reply continues, and the card leaves both the chat and
   the Inbox. Repeat and Decline.
   A request in a chat that has no id yet (the very first message of a new chat) has no
   `thread_id`, so it shows in the Inbox only. Note whether that happens in practice.
4. **Server HQ.** If the phone is also signed in to an HQ on a server, trigger an approval
   there. Today the phone posts a notice and its Inbox does not list the item (the notice
   opens the server's web app). This is the most likely cause of the original report and is
   **not fixed** on this branch. Record what happens; if confirmed, it is the next piece of
   work.
5. **Silence.** In the web app (Settings, Notifications, Silence) silence Helpers. A helper
   that stops at a prompt must post no notice on the phone, still appear in the Inbox, and
   not count on the badge. Turn it back on and check a new one notifies. Approvals from HQ
   itself must notify while everything is paused.
6. **Helpers ask HQ first.** Start a helper from a chat with Drive on and let it reach a
   permission prompt. Nothing should reach the Inbox or the lock screen for five minutes
   (`agent_host.driver_grace_minutes`) while HQ answers it. With Drive off, the item appears
   at once. After the helper moves on, its "needs an answer" item leaves Needs you.
7. **Battery and data.** The inbox is read over loopback every four seconds only while a
   reply is running. Watch for a visible cost on a long reply; if there is one, raise
   `CHAT_NEEDS_POLL_MS` or move to the socket's `inbox:change` event.

## Not built on the phone yet

- Mark all as read, Clear this list (Done) and Silence controls in the phone's Inbox. The
  server routes exist: `POST /api/notifications/read-all`, `/{id}/read`, `/clear-done`,
  `GET|POST /api/notifications/silences`, `DELETE /api/notifications/silences/{topic}`. Items
  carry `read`, `topic` and `silenced`. The web versions are in
  `apps/hq-web/src/components/notifications/` and `components/settings/SilenceSection.tsx`.
- A door to Tasks and Helpers in the phone's Settings (the web has "What HQ is working on").
- Listing a server HQ's requests in the phone's Inbox (item 4 above).

## Before opening a pull request

- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --no-fail-fast`, and read the totals
- `cd apps/hq-web && bun install && bun run typecheck && bun test && bun run build`
- `bun scripts/strings/build.ts --check`
- the Gradle line above

## Known noise from the cloud run

These failed there and are unrelated to this branch. Expect them to pass on a normal machine:

- 28 tests in `hq-bridge` (`tests/e2e.rs`): the container has no IPv6, "Address family not
  supported by protocol".
- 1 test in `hq-backup`: asserts `ParentUnsafe`, which does not hold when run as root.
- 2 tests in `hq-agent` (`coding::edit`): pass when run alone.
- 1 web test, "a 401 from the exchange says the code did not work and wipes nothing"
  (`src/lib/signInSafety.test.ts`): fails in a full `bun test`, passes alone, and fails the
  same way without this branch.

If any of these fail locally too, that is a real finding: say so rather than skip the test.

## One thing to undo

The cloud environment named `hq` had `dl.google.com` added to its allowed hosts for the
Android SDK. The install was blocked anyway, so the entry is unused and can be removed in the
environment's settings.

Delete this file when the verification is done and its findings are in the pull request.
