# Recording clips for the site

Every clip is re-recordable by script, small enough for people paying for data, and tied to the
build it was recorded on. The site promises no cookies and nothing from other origins, so clips are
same-origin files in `public/media/`, never a video host.

## Rules

- A guide clip is 45 to 90 seconds with captions, at most 3 MB. A home page loop is silent, 6 to 10
  seconds, at most 600 KB. One task per clip.
- Phone flows are vertical (480 wide after encoding). Computer flows are 16:9, 1280 wide at most.
- No autoplay. `Media.astro` renders `preload="none"`, a poster, a captions track, the size and a
  "save to share" link.
- Only invented names on screen: seed a demo Vault, never record real notes, hosts or people.

## The pipeline

1. **Demo instance.** `scripts/media/seed-demo-vault.sh /tmp/hq-demo-vault`, then start a throwaway
   HQ with `HQ_VAULT_PATH` and `HQ_CONFIG_PATH` pointing at it.
2. **Record.**
   - Phone: `scripts/media/record-phone.sh out.mp4` on one fixed reference device. It sets Android's
     demo mode (clean clock and battery) and shows touches, and uses `scrcpy` when installed.
   - Web or desktop app: `bun scripts/media/record-web.mjs steps.json out.webm`. The first run needs
     `bunx playwright-core install ffmpeg`.
   - Terminal: `vhs scripts/media/tapes/install-mac.tape` renders a reproducible clip.
3. **Edit.** Cut dead air, add captions and write a `.vtt` file. Any editor works. The ffmpeg build
   on most machines has no subtitle filter, so burn captions in the editor, not in step 4.
4. **Encode and register.**
   `bun scripts/media/encode-media.mjs out.mp4 guide-ai-key guide --vertical --alt "..." --caption "..." --build v0.9.1-main.598 --captions out.vtt`
   Over budget is refused and nothing is written. On success it writes the mp4, a poster and the
   `.vtt` to `public/media/` and updates `src/data/media.json` with `recordedWith`, `bytes`,
   `duration` and `captions`.
5. **Commit.** A guide page (`/guides/<id>/`) shows its clip as soon as an entry named
   `guide-<id>` has a file. `bun run check` fails a clip with no captions, no alt text, no build
   tag or more than 3 MB. `bun run check:external` warns when a clip is more than 40 builds behind
   the newest release.

## The first twelve guides

1 Install on Android (with the Play Protect screen). 2 Get an AI key and set a spending limit.
3 Your first note and first question. 4 Connect your laptop. 5 Install on a Mac. 6 Windows, Full or
Lite. 7 Back up your Vault. 8 Approve or decline what HQ asks. 9 Start a coding helper. 10 Code from
your phone with your own GitHub. 11 Make a big document small. 12 Update HQ and keep it running.
Guides 1 to 3, 5 and 7 have text pages already (`src/data/guides.ts`). Record only after the first-run
flow has settled, and re-record from what the first five real testers stumble on.
