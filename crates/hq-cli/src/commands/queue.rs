//! `hq queue` — list/clear the `value_items` notification queue from the
//! CLI, so clearing it never requires raw SQL against `_data/vault.db`.
//! See `FEATURE-REQUESTS.md` FR-001a.

use anyhow::Result;
use clap::Subcommand;
use hq_core::config::HqConfig;
use hq_core::types::{ValueKind, ValueState};
use std::io::Write;

#[derive(Subcommand)]
pub enum QueueAction {
    /// List value-bus items (default: most recent 20, any state/kind)
    List {
        /// Filter by state: pending|routed|delivered|engaged|dismissed|expired
        #[arg(long)]
        state: Option<String>,
        /// Filter by kind: action_needed|insight|proposal|fyi
        #[arg(long)]
        kind: Option<String>,
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Per-state item counts
    Stats,
    /// Approve one item by id or id prefix, after showing it. Needs an
    /// interactive terminal, for operators who run no chat relay.
    Approve { id: String },
    /// Dismiss items without waiting for delivery/engagement
    Clear {
        /// Dismiss one item by id (or id prefix / token)
        id: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        state: Option<String>,
    },
}

pub async fn run(config: &HqConfig, action: QueueAction) -> Result<()> {
    run_with_output(config, action, &mut std::io::stdout()).await
}

async fn run_with_output(
    config: &HqConfig,
    action: QueueAction,
    output: &mut dyn Write,
) -> Result<()> {
    let db = hq_db::Database::open(&config.db_path())?;

    match action {
        QueueAction::List { state, kind, limit } => {
            let state = parse_state(state.as_deref())?;
            let kind = parse_kind(kind.as_deref())?;
            let items = hq_db::value_items::list_filtered(&db, state, kind, limit)?;
            if items.is_empty() {
                writeln!(output, "No matching items.")?;
                return Ok(());
            }
            for item in &items {
                writeln!(
                    output,
                    "{}  {:<9} {:<13} {:.2}  {}",
                    &item.id[..8.min(item.id.len())],
                    item.kind.as_str(),
                    item.state.as_str(),
                    item.score,
                    item.title,
                )?;
            }
            writeln!(output, "\n{} item(s)", items.len())?;
        }
        QueueAction::Stats => {
            let counts = hq_db::value_items::count_by_state(&db)?;
            if counts.is_empty() {
                writeln!(output, "No items in the queue.")?;
            }
            for (state, n) in &counts {
                writeln!(output, "{state:<12} {n}")?;
            }
        }
        QueueAction::Approve { id } => approve_interactively(&db, &id, output)?,
        QueueAction::Clear {
            id,
            all,
            kind,
            state,
        } => {
            let kind = parse_kind(kind.as_deref())?;
            let state = parse_state(state.as_deref())?;
            if let Some(s) = state
                && !matches!(s, ValueState::Pending | ValueState::Routed | ValueState::Delivered)
            {
                anyhow::bail!(
                    "queue clear --state {}: clearing only makes sense for an active state \
                     (pending|routed|delivered) — an already-terminal item is not something to dismiss",
                    s.as_str()
                );
            }
            match (id, all) {
                (Some(id), _) => {
                    if kind.is_some() || state.is_some() {
                        anyhow::bail!(
                            "queue clear: an id and --kind/--state are mutually exclusive — \
                             pass one item id, or --all with --kind/--state to scope a bulk clear"
                        );
                    }
                    let matched = hq_db::value_items::dismiss_by_id(&db, &id)?;
                    if matched {
                        writeln!(output, "Dismissed {id}.")?;
                    } else {
                        writeln!(output, "No active item matched '{id}'.")?;
                    }
                }
                (None, true) => {
                    let n = hq_db::value_items::dismiss_all(&db, kind, state)?;
                    writeln!(output, "Dismissed {n} item(s).")?;
                }
                (None, false) => {
                    anyhow::bail!("queue clear: pass an id, or --all to clear everything matching --kind/--state");
                }
            }
        }
    }
    Ok(())
}

fn parse_kind(s: Option<&str>) -> Result<Option<ValueKind>> {
    match s {
        None => Ok(None),
        Some(s) => ValueKind::from_str(s)
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("unknown kind '{s}' (expected action_needed|insight|proposal|fyi)")),
    }
}

fn parse_state(s: Option<&str>) -> Result<Option<ValueState>> {
    match s {
        None => Ok(None),
        Some(s) => ValueState::from_str(s).map(Some).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown state '{s}' (expected pending|routed|delivered|engaged|dismissed|expired)"
            )
        }),
    }
}

/// Show the item and approve it only on a typed `yes` at a terminal.
fn approve_interactively(db: &hq_db::Database, id: &str, output: &mut dyn Write) -> Result<()> {
    use std::io::{BufRead, IsTerminal};

    if !std::io::stdin().is_terminal() {
        anyhow::bail!("queue approve needs an interactive terminal; approve from a chat relay instead");
    }
    let items = hq_db::value_items::list_filtered(db, None, None, 500)?;
    let Some(item) = items.iter().find(|i| i.id.starts_with(id) && id.len() >= 4) else {
        anyhow::bail!("no item matches '{id}'");
    };
    writeln!(output, "{}\n\n{}\n", item.title, item.body)?;
    write!(output, "Type yes to approve: ")?;
    output.flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    if answer.trim() != "yes" {
        writeln!(output, "Not approved.")?;
        return Ok(());
    }
    let matched = hq_db::value_items::approve_by_id(db, &item.id)?;
    writeln!(output, "{}", if matched { "Approved." } else { "That item is no longer active." })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::types::ValueItem;

    fn test_config() -> (tempfile::TempDir, HqConfig) {
        let dir = tempfile::tempdir().unwrap();
        let config = HqConfig {
            vault_path: dir.path().to_path_buf(),
            ..Default::default()
        };
        std::fs::create_dir_all(dir.path().join("_data")).unwrap();
        (dir, config)
    }

    #[tokio::test]
    async fn list_and_clear_round_trip() {
        let (_dir, config) = test_config();
        let db = hq_db::Database::open(&config.db_path()).unwrap();
        let item = ValueItem::new("t", ValueKind::Fyi, "title", "body");
        hq_db::value_items::insert(&db, &item).unwrap();

        let mut out = Vec::new();
        run_with_output(
            &config,
            QueueAction::List {
                state: None,
                kind: None,
                limit: 20,
            },
            &mut out,
        )
        .await
        .unwrap();
        assert!(String::from_utf8(out).unwrap().contains("title"));

        let mut out = Vec::new();
        run_with_output(
            &config,
            QueueAction::Clear {
                id: None,
                all: true,
                kind: None,
                state: None,
            },
            &mut out,
        )
        .await
        .unwrap();
        assert!(String::from_utf8(out).unwrap().contains("Dismissed 1"));

        let remaining =
            hq_db::value_items::list_filtered(&db, Some(ValueState::Pending), None, 20).unwrap();
        assert!(remaining.is_empty());
    }

    #[tokio::test]
    async fn clear_with_id_and_kind_is_rejected() {
        let (_dir, config) = test_config();
        let mut out = Vec::new();
        let result = run_with_output(
            &config,
            QueueAction::Clear {
                id: Some("abcd1234".to_string()),
                all: false,
                kind: Some("fyi".to_string()),
                state: None,
            },
            &mut out,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn clear_with_a_terminal_state_is_rejected() {
        let (_dir, config) = test_config();
        let mut out = Vec::new();
        let result = run_with_output(
            &config,
            QueueAction::Clear {
                id: None,
                all: true,
                kind: None,
                state: Some("engaged".to_string()),
            },
            &mut out,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn clear_with_no_id_and_no_all_is_an_error() {
        let (_dir, config) = test_config();
        let mut out = Vec::new();
        let result = run_with_output(
            &config,
            QueueAction::Clear {
                id: None,
                all: false,
                kind: None,
                state: None,
            },
            &mut out,
        )
        .await;
        assert!(result.is_err());
    }
}
