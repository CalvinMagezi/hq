#!/usr/bin/env bash
# Decides which CI jobs a pull request can affect, from the files it changes.
#
# Input (environment): BASE and HEAD (commit ids), EVENT_NAME (defaults to GITHUB_EVENT_NAME).
# Output: one `area=true|false` line each for rust, web_app, android, site, installer and workflows,
# appended to $GITHUB_OUTPUT (stdout when that is unset, for local runs).
#
# The rule is: when in doubt, run everything. Every flag is true when this is not a pull request
# (pushes to main refresh the caches and run the whole suite), when BASE or HEAD is missing, when
# git fails, when the diff is empty, and when any changed path is not listed below. A skipped job
# never hides a failure; a wrongly skipped one would, so the list is short and every entry says why.
#
# Run: BASE=<sha> HEAD=<sha> EVENT_NAME=pull_request bash scripts/ci/changed-areas.sh
set -uo pipefail

rust=false web_app=false android=false site=false installer=false workflows=false

all_true() { rust=true web_app=true android=true site=true installer=true workflows=true; }

emit() {
    local out=${GITHUB_OUTPUT:-/dev/stdout}
    {
        echo "rust=$rust"
        echo "web_app=$web_app"
        echo "android=$android"
        echo "site=$site"
        echo "installer=$installer"
        echo "workflows=$workflows"
    } >> "$out"
}

fail_open() {
    echo "changed-areas: $1; running everything" >&2
    all_true
    emit
    exit 0
}

event=${EVENT_NAME:-${GITHUB_EVENT_NAME:-}}
base=${BASE:-}
head=${HEAD:-}

[ "$event" = pull_request ] || fail_open "event is '${event:-unset}', not a pull request"
[ -n "$base" ] && [ -n "$head" ] || fail_open "BASE or HEAD is not set"
case "$base" in *[!0]*) ;; *) fail_open "BASE is the null commit" ;; esac

files=$(git diff --name-only --no-renames -z "$base...$head" | tr '\0' '\n') \
    || fail_open "git diff failed"
[ -n "$files" ] || fail_open "the diff is empty"

unknown=
while IFS= read -r f; do
    case "$f" in
        # A change to this file can change every job in it.
        .github/workflows/ci.yml) all_true ;;
        .github/workflows/*) workflows=true ;;
        # CODEOWNERS, issue and PR templates, actionlint config: no job reads them.
        .github/*) ;;

        # The Rust workspace and everything that feeds a Rust build or test.
        crates/* | Cargo.toml | Cargo.lock) rust=true android=true ;;
        .cargo/* | deny.toml | .config/*) rust=true ;;
        # hq-web embeds the built app; hq-desktop tests read apps/desktop.
        apps/hq-web/*) rust=true web_app=true ;;
        apps/strings/*) rust=true web_app=true android=true ;;
        apps/desktop/*) rust=true ;;
        apps/android/*) android=true ;;
        apps/site/* | apps/deploy/*) site=true ;;
        packages/*) installer=true ;;
        # The Windows installer scripts are tested by the PowerShell job and checked by the site.
        scripts/windows/*) rust=true site=true installer=true ;;
        scripts/check-site-install-scripts.sh) rust=true site=true ;;
        scripts/check-android-alignment.sh) rust=true android=true ;;
        scripts/*) rust=true ;;

        # Two Rust tests read these files (hq-core config tests, hq-desktop shell_config).
        README.md) rust=true ;;
        # The site renders docs/*.md and serves docs/media, and one Rust test reads DESKTOP.md.
        docs/media/*) site=true ;;
        docs/DESKTOP.md) rust=true site=true ;;
        # Prose nobody builds: nested docs (plans, security), and Markdown or licence files at the root.
        docs/*/*) ;;
        docs/*.md) site=true ;;
        docs/*) ;;
        changelog.d/*) ;;
        */*) unknown=$f ;;
        *.md | LICENSE | NOTICE) ;;
        *) unknown=$f ;;
    esac
    [ -z "$unknown" ] || fail_open "unlisted path '$unknown'"
done <<< "$files"

emit
