# SQL string-building audit

Scope: every place a SQL statement is assembled from more than one piece of
text, across all crates that use `rusqlite` (`hq-db`, `hq-memory`, `hq-tools`,
`hq-web`, `hq-cli`). Found by scanning every `format!` call that contains an SQL
keyword, plus every `prepare`, `prepare_cached`, `execute`, `execute_batch`,
`query_row` and `query_map` call whose first argument is not a literal. Values
always travel as bound parameters (`?1`, `params![]`); this audit is about the
text of the statement itself.

Verdict key: **const** means only compile-time constants are interpolated,
**static** means a `&'static str` chosen by a `match` or `if` on the input,
**int** means a number the code computed, **marks** means a run of `?`
placeholders sized by a slice length.

No site interpolates caller-supplied text. Three changes make that provable by
the compiler or a test instead of by reading:

- `harness_sessions_registry::nudge_column(keys: bool) -> &'static str` is the
  single place the nudge counter column is chosen (previously two copies of an
  inline `if`). A test checks both names are real columns.
- `tasks.rs` collects `SET` and `WHERE` fragments in `Vec<&'static str>`, so a
  runtime `String` cannot be pushed into them without a compile error.
- Tests reject hostile input for the two functions that map a request string
  to statement text: `tasks::event_for_status` and
  `notifications_api::state_where`. A migration test checks the
  `ALTER TABLE` column table holds only plain identifiers and type names.

## Sites

| File:line | Interpolated | Source | Verdict |
|-----------|--------------|--------|---------|
| `hq-cli/src/commands/usage.rs:132` | `{key}` | `GroupBy::key_sql`, a `match` on an enum returning string literals | static |
| `hq-db/src/ask_requests.rs:97,111` | `{COLUMNS}` | module const | const |
| `hq-db/src/background_turns.rs:97,141,181,230,287` | `{COLS}` | module const | const |
| `hq-db/src/chat.rs:160,169` | `{base}` | a string literal local to `list_threads` | const |
| `hq-db/src/harness_sessions_registry.rs:188,203,209,228,684,700` | `{COLS}` | module const | const |
| `hq-db/src/harness_sessions_registry.rs:573,585` | `{column}` | `nudge_column(keys: bool)`, two literals | static |
| `hq-db/src/migrations.rs:300` | `{table} {col} {typedef}` | `MEMORY_COLUMNS` const table, shape checked by `memory_columns_are_plain_identifiers_and_types` | const |
| `hq-db/src/migrations.rs:517` | same | test-only copy of the loop above | const |
| `hq-db/src/migrations.rs:338` | `sql` passed to `execute_batch` | `include_str!` migration files | const |
| `hq-db/src/self_update_runs.rs:63,75,108` | `{COLS}` | module const | const |
| `hq-db/src/self_update_runs.rs:93` | `{installed_at_sql}` | `if status == STATUS_INSTALLED { literal } else { "" }` | static |
| `hq-db/src/subagent_runs.rs:352,385,406,434,504` | `{RUN_COLS}` | module const | const |
| `hq-db/src/subagent_runs.rs:625` | `{ROUTABLE_PLATFORMS}` | module const `('web','telegram','discord')` | const |
| `hq-db/src/task_outcomes.rs:122` | `{}` filter clause | `match task_hint_filter { Some => "AND o.task_hint = ?3", None => "" }`, the hint itself is bound | static |
| `hq-db/src/tasks.rs:279` | `{summary_col}` | `event_for_status`, a `match` returning two literals, `None` for anything else (hostile input test) | static |
| `hq-db/src/tasks.rs:323,333,354` | `{marks}` | `placeholders(ids.len())` | marks |
| `hq-db/src/tasks.rs:354` | `{STATUS_COMPLETE}` | module const | const |
| `hq-db/src/tasks.rs:570,585,706,743,753` | `{INITIATIVE_COLS}`, `{TASK_COLS}` | module consts | const |
| `hq-db/src/tasks.rs:585-605` | `conditions.join(" AND ")` | `Vec<&'static str>` | const |
| `hq-db/src/tasks.rs:753-794` | `conditions.join(" AND ")`, the `JOIN task_tags` suffix | `Vec<&'static str>` and a literal | const |
| `hq-db/src/tasks.rs:897` | `sets.join(", ")` | `Vec<&'static str>`; the `AND status = ?` suffix is a literal | const |
| `hq-db/src/tasks.rs:531,1037` | `sql` argument | literals chosen by `match` (`list_folders`) or passed by in-file callers as literals (`tasks_by_ids`) | const |
| `hq-db/src/value_items.rs:95,127` | `{COLS}` | module const | const |
| `hq-db/src/value_items.rs:205,208` | `{where_clause}`, `?{}` | clauses are `state = ?1` and `kind = ?N` where N is `1` or `2`; the limit placeholder number is `clauses.len() + 1` | int |
| `hq-memory/src/db.rs:163` | `{placeholders}` | `"?"` repeated and comma joined | marks |
| `hq-memory/src/db.rs:232` | `sql` argument of `query_memories` | callers pass literals | const |
| `hq-memory/src/forgetter.rs:79,96` | `{placeholders}` | `"?"` repeated and comma joined | marks |
| `hq-web/src/notifications_api.rs:137` | `state_where(state_filter)` | `fn -> &'static str`; `state_filter` comes from an HTTP query parameter and unknown values select no clause (hostile input test) | static |
| `hq-tools/src/session_search.rs:53` | `sql` variable | one literal, `LIKE ?1 ESCAPE` with bound pattern | const |

## Re-running the audit

```sh
grep -rnE '(execute|execute_batch|prepare|prepare_cached|query_row|query_map)\(\s*&?format!' crates
grep -rnE 'sql\.push_str|sql \+=' crates
```

Every new hit needs a row above. A parameter that would become part of the
statement text must go through a `match` that returns `&'static str`, as
`nudge_column`, `event_for_status` and `state_where` do.
