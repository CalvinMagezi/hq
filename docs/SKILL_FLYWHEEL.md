# Improving skills from evidence

How HQ records what happened when a skill was used, turns that into outcomes you can trust,
and tests a change to a skill before it goes live. Nothing here rewrites a skill by itself.

Skills live in `.vault/skills/<name>/SKILL.md`. Changes to them go through the governed path
(`hq_tools::skill_edit`, the audit trail in `skills/_audit.jsonl`, `hq skills approve|reject`).

## What is recorded

Each time a skill is loaded (the `load_skill` tool, or the session auto-loading skills whose
hints match) one row is written to `skill_invocations`:

| Column | Holds |
|---|---|
| `skill_name`, `session_id`, `trigger`, `loaded_at` | which skill, in which session, how it loaded, when |
| `skill_hash` | SHA-256 of `SKILL.md` at load, the same digest `_audit.jsonl` keeps, so an outcome reads against the version that ran |
| `task_id` | the HQ task the session was working on, only when the runtime can say |
| `purpose` | `code`, `research`, `writing`, `review`, `planning`, `operations` or `general`, from the task's tags |
| `used` | `suggested`, `loaded`, or the review's verdict `helped`, `hurt`, `unused` |
| `outcome_score`, `outcome_status`, `outcome_source`, `outcome_ref` | how it turned out, from which kind of signal, and the id of the record that said so |

Only hashes, ids and values from fixed lists are stored; the database refuses anything else.
**No prompt, no document text and no task title or description is ever stored.** `outcome_ref`
points (`task:<id>`, `run:<id>`, `note:<version id>`), it does not copy. Rows from before the
columns existed keep NULLs, and NULL means "not recorded".

The task is never taken from anything the model wrote. A native session names it from the live
leases it holds; a call to `load_skill` names it from the identity the runtime attested.

## How outcomes are scored

A session exiting cleanly says the process finished, not that the work was good. So the score
comes from what happened to the work afterwards, and the exit is kept only as weak evidence.
The daemon task `skill-outcomes` (every 10 minutes, last 30 days) settles each load from:

| Signal | Outcome |
|---|---|
| the task went `ready_for_review`, then `complete`, completed by someone other than the people who worked it | `accepted` (score 1.0) |
| the task went `ready_for_review`, then back to `in_progress`, `to_do` or `blocked` | `reworked` (0.5) |
| the task went `ready_for_review`, then `complete`, but the completing actor is one that held a lease or moved it into progress or review, or was not recorded | no outcome |
| a sub-agent run review `accepted` / `partial` | `accepted` / `reworked`; `blocked` is ignored |
| someone other than the writer `accepted` / `superseded` / `disputed` a note written under the task | `accepted` / `reworked` / `rejected` (0.0) |

A signal counts for a load only if it came after the load, and the latest one wins. A task
still waiting in review has not been judged, and finishing a task without review is not
acceptance. A session that exits cleanly while its task is reopened therefore scores 0.5, not 1.0.

`outcome_source` says which kind of evidence set the score: `task_acceptance`, `run_review`
and `note_review` are observed outcomes; `review` (the post-session reviewer's opinion) and
`session_result` (how the session ended) are weaker and never overwrite an observed one.

## Regression fixtures

A skill may have `skills/<name>/fixtures.yaml` next to its `SKILL.md`:

```yaml
# Phrases that must make this skill's hints match (any one hint is enough).
triggers:
  - fix this spreadsheet
# Phrases that must not match any of its hints.
not_triggers:
  - write a poem about spring
# Steps the body must still contain. Compared ignoring case and runs of whitespace.
required_steps:
  - back up the original file
  - confirm before overwriting
```

All three keys are optional; an unknown key is an error, so a typo cannot silently disable a
test. At most 50 entries per list and 200 characters per entry. Hint matching is the real
rule the hint index uses (a whole word or phrase, case-insensitive), so a fixture cannot pass
here and fail in a session.

- `hq skills validate` runs them and reports a failure as `FAIL  <skill>: fixture: ...`. A
  skill without a fixtures file validates exactly as before.
- A change made by the post-session skill review (a patch, new hints, a new skill) must pass
  them before it is applied. One that fails is not applied: it is held in
  `skills/_proposed/` with what it broke on the audit line and in the notice, for
  `hq skills approve` or `hq skills reject`. An unreadable fixtures file blocks the change too.
- Approving a held change is your decision to override: update the fixtures if they are out
  of date, or `validate` will keep reporting the failure.

## What the review may change by itself

The post-session review reads the last part of the session, tool results included, so a page,
mail or note the session read reaches the model that writes the skill. What it proposes goes
live without you only when all of these hold:

- it patches or adds hints to a skill the review already manages (one it wrote that you
  approved, or one you adopted with `hq skills adopt`);
- the session read nothing an outsider could have written: no web page, mail, document,
  vault note or other agent's output (the same rule that taints a session, see
  `docs/security/PROMPT_INJECTION.md`);
- the added text names no network command (`curl`, `wget`, `nc`, `scp`, `rsync` and the like)
  or outside URL, no credentials (keys, tokens, passwords) and no email address;
- it passes the skill's fixtures, and `governance.skills_write_approval` is off.

Everything else is held in `skills/_proposed/` for `hq skills approve` or `hq skills reject`,
with the reasons on the audit line and in the notice. A new skill is always held. Text that
tries to override instructions or carries an encoded payload is refused outright and never
staged.

## Evidence reports and proposals

`hq skills evidence [name] [--days N] [--json]` reads the invocation records (after settling the
latest outcomes) and prints, per skill and per version of it: loads, sessions, the outcome mix,
the verdicts, and the patterns that repeat. A pattern is one of:

| Pattern | Means |
|---|---|
| `reworked_outcomes` | work that used the skill keeps being reopened or rejected |
| `judged_harmful` | the post-session review keeps judging the skill harmful |
| `loaded_but_unused` | the skill loads and is then not applied |
| `reworked_for_purpose` | work of one kind (`code`, `research`, ...) that used it keeps coming back |
| `hints_did_not_activate` | an auto-loading skill keeps being loaded by hand |

A step missed again and again is not on the list: that would need the text of the work, which is
never stored.

**Thresholds.** A pattern counts as evidence only when it shows in at least 5 loads across at
least 3 sessions inside the window (30 days unless `--days` says otherwise). Smaller ones are
printed as "too small to count" with their sizes, and a skill with no pattern at that size is
reported as `insufficient evidence`. No proposal is made for it. Two versions of a skill are
compared with their load counts and flagged "small sample" when either has fewer than 5; a
difference is a correlation, not proof that the change caused it.

**Proposals.** `hq skills evidence --propose` writes `skills/_proposed/<name>/PROPOSAL.md` for each
skill with a finding: the pattern, counts, date range, skill version, example loads by invocation
id (with the task and the outcome reference where known, never quoted text), the likely kind of
change, and the regression fixtures that any change has to keep passing. It is evidence, not an
edit: it has no replacement text, `hq skills approve` refuses it, and `hq skills reject <name>`
dismisses it. The skill changes only through `hq skills edit` or the review, where the fixtures
gate the change.

## Retention and deletion

Raw invocation rows are kept 90 days. The daily `vault-cleanup` task then folds older rows into
monthly roll-ups per skill, version and month: counts, a score sum and outcome and verdict
tallies, with no invocation id, session, task or reference. `count_invocations` and
`mean_outcome` read the roll-ups too, so the numbers do not move when rows age out.

`hq skills evidence purge` deletes telemetry on request, for the owner at the command line only
(no tool, MCP route or web endpoint exposes the evidence or the purge):

```
hq skills evidence purge --id inv_1a2b3c4d          # one record
hq skills evidence purge --skill sheets             # one skill's records
hq skills evidence purge --from 2026-01-01 --before 2026-02-01
```

It prints what it would delete and changes nothing until `--yes` is added. A purge with no id,
skill or date is refused. Roll-up months that lie entirely inside a date range go with it.
Skills, their versions and `_audit.jsonl` are never touched.

## Skills HQ ships

Some skills come with HQ itself (`crates/hq-tools/assets/skills/`, listed in `BUILTIN_SKILLS` in
`crates/hq-tools/src/skills/builtin.rs`). Each one declares when it applies. `edge-coding-worker`
needs at least one repository in `github_write.repos`, so an install that cannot publish never
sees it.

**Active.** A built-in is active when it applies and you have not turned it off. `hq` decides this
once per process, from the config it loaded.

These guarantees hold for active built-ins only. Where a built-in does not apply or is off, its
name is free, and any skill under it, yours or an agent's, is an ordinary skill. If the
built-in becomes active later, HQ's text takes the name back, and the earlier text is archived.

**Loading.** A session loading an active built-in always gets the text embedded in the binary,
never the vault copy. An edit to the copy, or a bundle, `SUMMARY.md` or `_proposed/` entry under
its name in any letter case, changes nothing a session sees.

**Changing.** Nothing changes an active built-in in place. `skill_manage`, the post-session
review, `hq skills create|edit|approve|delete|revert|adopt|unadopt` and proposal approval all
refuse its name.

**The vault copy.** Each `hq start` and `hq web`, which covers the phone too, and `hq skills sync`,
write every active built-in into the vault's skills folder for people to read, and bring a
differing copy back in step. The old copy goes to the skill's `archive/`, and the change is logged
in `_audit.jsonl` as `synced`. A copy is a mirror only: a process that never syncs still offers the
skill.

**Turning one off.** List it under `skills.builtin.off` in `config.yaml`:

```yaml
skills:
  builtin:
    off: [edge-coding-worker]
```

The next sync removes HQ's copy (only files that are exactly a text HQ shipped) and keeps its
archive. The name is then released: a skill of
your own under that name loads and behaves like any other, and sync leaves it alone. The setting
is yours alone: `config_manage` refuses the `skills` key, and agents cannot write the config file.
A name in `off` that no built-in has is reported at start, because it is most likely a typo.

**Shipping a new skill.** Add its folder under `assets/skills/` and an entry in `BUILTIN_SKILLS`,
with an `applies` rule. A test fails if the two disagree. The text must pass `hq skills validate`
and the security scan, carry `provenance: {mintedBy: hq}`, and name nothing specific to one
installation. Text an agent writes into a mirror is still readable as a vault note (`vault_read`,
search). It is never loaded as a skill, but treat it like any other note an agent could have
written.
