# Provenance: herdr

Agent HQ is building its own host for long-lived coding agents. herdr
(https://github.com/herdrdev/herdr) is the project whose design this follows and
whose published agent-detection rules may be adapted. This file records exactly
what was taken, from which version, under which license.

## Reference version

| | |
|---|---|
| Repository | https://github.com/herdrdev/herdr |
| Pinned commit | `3d9d2b18dab139ba226ebc5a1c9a9f2c9c3ee4df` (master, 2026-10-05) |
| Latest release when pinned | v0.9.3 (2026-09-29) |
| License at that commit | Apache License 2.0 |
| Relicense commit | `cd5ea1be0e69ed49b6f32f7ed5b333f6c8526874` (2026-07-22, "relicense herdr under apache-2.0") |

License history of herdr's `LICENSE` file: initial release 2026-03-22, "clarify
dual licensing" 2026-05-26, relicense to Apache-2.0 2026-07-22. Before the
relicense the project was available under AGPL-3.0-or-later with commercial
licenses. Nothing here relies on code from that period. Check again with:

```
gh api 'repos/herdrdev/herdr/commits?path=LICENSE' --jq '.[] | "\(.commit.author.date) \(.sha) \(.commit.message | split("\n")[0])"'
```

herdr vendors libghostty-vt (MIT) and portable-pty (MIT). Agent HQ uses neither
vendored copy: the pure-Rust `portable-pty` crate is a normal dependency and the
terminal emulation backend is chosen separately.

## What has been taken so far

Nothing. No herdr source or data file is copied into this repository yet.

## Rules for anything taken later

1. Prefer writing new code against the documented behavior. Copy only when the
   original is the clearest statement of the rule, as with the agent-detection
   manifests.
2. Every derived file starts with a header comment naming the origin file, the
   pinned commit above, the Apache-2.0 license, and that it was modified.
3. The first derived file adds a `NOTICE` file at the repository root listing
   herdr and the files derived from it, and updates this document.
4. A derived file keeps the Apache-2.0 terms for its own content. The rest of
   the repository stays MIT.
5. If herdr's license changes again, the pinned commit stays usable under the
   license it carried, and new material is taken only from commits whose
   license has been checked with the command above.
6. Do not fetch manifests or code from herdr at run time.
