# Identity-scoped disclosure

Code: `crates/hq-core/src/privacy.rs` (resolver, labels, scope),
`crates/hq-tools/src/vault/scoped.rs` (vault enforcement),
`crates/hq-relay/src/discord/family.rs` (Discord wiring).

## Resolution

A channel builds an `IdentityClaim` (platform, account id, optional display name)
and calls `resolve(&registry, &claim)`. Only an exact platform account binding
resolves. The result is one of:

- `Resolved(person, role, confidence, provenance)`: exactly one person is bound.
- `Ambiguous(people)`: the account maps to more than one person. Nobody is assumed.
- `Unknown`: no binding. A display name or a name in the message text never counts.

Bindings come from config (`relay.discord_allowed_user_ids` for owners,
`relay.discord_family_users` for members). The registry is rebuilt on every turn,
so a removed or changed mapping applies to the next turn. `identity_changed`
tells a caller that an account now resolves to someone else, so it can drop that
person's session context. Confidence is `Verified` for a binding or `Assumed`
for one the operator marked unconfirmed; assumed people are treated as unverified.

## Labels and scope

Vault notes and tasks carry a `Label`:

| Label | How it is set | Who may see it |
|-------|---------------|----------------|
| Private(people) | `people:` frontmatter, or a note under `Notebooks/People/<slug>/` | only turns whose whole audience is named |
| Shared | `visibility: shared` | any verified audience |
| Public | `visibility: public` | everyone, including unresolved |
| Unmarked | nothing | owners only |

The most restrictive marker wins. A `DisclosureScope` is built from the audience:
owners alone are unrestricted; anyone else restricts to the verified people
present, and one unresolved, ambiguous or assumed participant drops the audience
to public only. So a shared channel discloses only what every participant may
see, and a cross-person task is visible only to the people it names. Unresolved
identity is deny by default.

## Enforcement and provenance

`scope_vault_tools` wraps the vault tools for a restricted scope. Reads check the
note's label, list and search results are filtered, batch reads are all or
nothing, writes are allowed only into the caller's own private space, and tools
that cannot be filtered (graph, tags, reorg, reindex, context) are refused.
Refusals return `DENIED_MESSAGE` and never say whether the note exists.

A `Fact` records its `Provenance`. Only config, platform and vault sources are
verified; user-stated, quoted and inferred statements never are. Quoted messages
are attributed to their author and never change who HQ is talking to.

## People notes

`vault_person_record_fact` (`crates/hq-tools/src/vault/person.rs`) is the only
writer of facts into `Notebooks/People/<slug>/`. It is a deterministic helper,
not an LLM step, and `scoped.rs` gates it like a write to that person's private
space, so ambiguous, unknown and cross-person audiences are refused. Facts go
under a `## Facts` heading of the existing note (a new `profile.md` only when
the folder is empty) as `- **topic**: text _(kind, trust: source; recorded date)_`.
An inferred fact is labelled and can never correct one. A differing fact on the
same topic stays as a visible conflict; `corrects: true` strikes the old line
through and keeps it. Health, religion, politics, finances, credentials and
similar topics are refused, and the write is re-read to verify it.

## Reusing it

A new channel calls `resolve` with its own claim and sets `RequestIdentity.scope`;
the agent builder applies it to the vault tools. Task and document automation
should classify with `Label::classify` and gate with `DisclosureScope::allows`.

## Person-scoped task spaces (FR-073)

A person's tasks live in the task space `people-<slug>` (`person_space_slug`), the
counterpart of the vault folder `Notebooks/People/<slug>/`. `space_label` labels
that space `Private(<slug>)`; every other space is unmarked, so owner only.

`scope_task_tools` (`crates/hq-tools/src/tasks/scoped.rs`) wraps the task tools
for a restricted scope, and the agent builder applies it next to the vault
wrapper. Reads and lists keep only tasks, initiatives, folders and spaces whose
label the audience may see. Writes need `DisclosureScope::sole_person`, meaning
exactly one verified person is present:

- `task_create`, `folder_create` and `initiative_create` are forced into the
  caller's own space. A requested `space_id`, `initiative_id`, parent or
  dependency that belongs elsewhere is refused.
- The space is created on first use and reused after that. The slug is unique,
  so `ensure_person_space` is idempotent and recovers from a lost creation race.
- Unresolved, ambiguous, assumed or multi-person audiences create nothing.
- `space_create`, `space_update`, `task_related` and `task_create_from_note`
  are refused because they cannot be filtered.

Spaces that already exist under other names stay owner only until an owner moves
their tasks into a `people-<slug>` space; nothing is migrated automatically.

## Not yet covered

Generated documents and non-vault context sources (memory
entities, session search) are not filtered yet. Discord passes only the message
author as the audience, so an owner mention in a busy guild channel is treated
as owner-only.
