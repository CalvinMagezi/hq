//! Creating a task from a vault note, with heuristic or decided placement.

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_core::config::{DecisionMode, SITE_TASK_PLACEMENT};
use hq_core::mailbox;
use hq_core::redact::redact_secrets;
use hq_db::Database;
use hq_db::tasks as t;
use hq_llm::decision::{ChoiceOrAbstain, DecisionRequest, Decisions};
use hq_vault::VaultClient;
use serde_json::{Value, json};
use std::sync::Arc;

use super::json::*;
use super::placement::*;
use crate::registry::HqTool;
use crate::util::{arg_str, generate_id};

const NOTE_EXCERPT_CHARS: usize = 600;
const NOTE_DECISION_CHARS: usize = 800;
const MAX_PLACEMENT_CANDIDATES: usize = 40;
/// Below this length a substring match is too likely to be a coincidence
/// (e.g. a "hq" tag matching half the initiative names in the system).
const MIN_FUZZY_MATCH_LEN: usize = 3;

/// One existing List (Space > Folder > List) a note could be promoted into.
/// `initiative.space_id` is the real Space id to file into — note this is
/// *not* generally the same string as the Space's slug (only the two
/// built-in `personal`/`professional` defaults happen to have id == slug).
pub(super) struct PlacementCandidate {
    pub(super) initiative: t::Initiative,
    pub(super) label: String,
    pub(super) folder_name: Option<String>,
}

fn list_placement_candidates(conn: &rusqlite::Connection) -> Result<Vec<PlacementCandidate>> {
    let spaces = t::list_spaces(conn)?;
    let folders = t::list_folders(conn, None)?;
    let initiatives = t::list_initiatives(conn, None, None)?;
    Ok(initiatives
        .into_iter()
        .filter_map(|initiative| {
            let space = spaces.iter().find(|s| s.id == initiative.space_id)?;
            let folder = initiative
                .folder_id
                .as_deref()
                .and_then(|fid| folders.iter().find(|f| f.id == fid));
            let label = match folder {
                Some(f) => format!("{} > {} > {}", space.name, f.name, initiative.name),
                None => format!("{} > {}", space.name, initiative.name),
            };
            Some(PlacementCandidate {
                folder_name: folder.map(|f| f.name.clone()),
                label,
                initiative,
            })
        })
        .collect())
}

fn nearest_dir_name(note_path: &str) -> Option<String> {
    std::path::Path::new(note_path)
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// The note's top-level containing directory, or `None` if the note sits
/// directly at the vault root (a bare `Path::new("todo.md").components()`
/// would otherwise yield the filename itself as its own "top-level
/// directory" — checking for a second component rules that out).
pub(super) fn top_level_dir_name(note_path: &str) -> Option<String> {
    let path = std::path::Path::new(note_path);
    let mut components = path.components();
    let first = components.next()?;
    components.next()?;
    first
        .as_os_str()
        .to_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn fuzzy_eq(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    a.len() >= MIN_FUZZY_MATCH_LEN
        && b.len() >= MIN_FUZZY_MATCH_LEN
        && (a.contains(b) || b.contains(a))
}

/// Below this length a shared word between a note title and a List name is too
/// generic to mean anything ("the", "data", "team") — longer than
/// `MIN_FUZZY_MATCH_LEN` because titles are free text, not a controlled tag
/// vocabulary, so the false-positive risk is higher.
const MIN_TITLE_WORD_LEN: usize = 5;

fn significant_words(text: &str, min_len: usize) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= min_len)
        .map(|w| w.to_lowercase())
        .collect()
}

/// Cheap zero-config placement guess: matches the note's tags, containing
/// vault folder name, and title against existing List/Folder names. This is
/// the incumbent behavior — Jev, when configured, is only ever allowed to
/// sharpen or override it, never to be a hard dependency (see
/// docs/architecture/decisions.md's consumer pattern). The title check exists
/// because a note's own heading is often the strongest signal of all (e.g. a
/// note titled "AcmeCorp Platform & Infrastructure Research Findings"
/// sharing "acmecorp"/"platform" with a List named "AcmeCorp —
/// Platform & Infra") even when its tags and folder don't match anything.
pub(super) fn heuristic_placement<'a>(
    note_title: &str,
    note_tags: &[String],
    note_path: &str,
    candidates: &'a [PlacementCandidate],
) -> Option<&'a PlacementCandidate> {
    let note_dir = nearest_dir_name(note_path).map(|s| s.to_lowercase());
    let tags_lower: Vec<String> = note_tags.iter().map(|t| t.to_lowercase()).collect();
    let title_words = significant_words(note_title, MIN_TITLE_WORD_LEN);

    let matches = |c: &PlacementCandidate| -> bool {
        let names: Vec<String> = std::iter::once(c.initiative.name.to_lowercase())
            .chain(c.folder_name.as_ref().map(|f| f.to_lowercase()))
            .collect();
        names.iter().any(|n| {
            tags_lower.iter().any(|t| fuzzy_eq(t, n))
                || note_dir.as_deref().is_some_and(|nd| fuzzy_eq(nd, n))
                || significant_words(n, MIN_TITLE_WORD_LEN)
                    .iter()
                    .any(|nw| title_words.contains(nw))
        })
    };

    let mut hits = candidates.iter().filter(|c| matches(c));
    let first = hits.next()?;
    // More than one candidate matched: ambiguous, don't guess.
    if hits.next().is_some() {
        return None;
    }
    Some(first)
}

/// Picks a Space for a brand-new List when nothing existing fits. Only
/// creates a new Space outright when none exist at all — a real Space to
/// file new work under almost always already exists, so guessing the wrong
/// existing Space and filing a new List under it is far less disruptive than
/// inventing Spaces on every miss.
fn choose_or_create_space(
    conn: &rusqlite::Connection,
    note_tags: &[String],
    note_path: &str,
) -> Result<t::Space> {
    let spaces = t::list_spaces(conn)?;
    if spaces.is_empty() {
        let name = top_level_dir_name(note_path).unwrap_or_else(|| "Inbox".to_string());
        return find_or_create_space(conn, &name);
    }
    let tags_lower: Vec<String> = note_tags.iter().map(|t| t.to_lowercase()).collect();
    let top_dir = top_level_dir_name(note_path).map(|s| s.to_lowercase());
    if let Some(found) = spaces.iter().find(|s| {
        let n = s.name.to_lowercase();
        tags_lower.iter().any(|t| fuzzy_eq(t, &n))
            || top_dir.as_deref().is_some_and(|td| fuzzy_eq(td, &n))
    }) {
        return Ok(found.clone());
    }
    Ok(spaces[0].clone())
}

/// Asks Jev to pick the best-fitting existing List for this note, when the
/// `task_placement` decision site is enabled. `None` on anything short of a
/// confident pick (not configured, disabled, timed out, abstained) — callers
/// must already have a heuristic fallback and treat this purely as a
/// sharpening layer, never a dependency. Takes the `Decisions` handle as a
/// parameter (fetched fresh per call by the caller via `hq_llm::decision::get()`,
/// same convention as `email_gate::suppress_fyi`) rather than reaching for the
/// process-wide singleton itself, so this function stays testable with a
/// `FakeDecisionProvider`-backed handle.
pub(super) async fn choose_placement_via_jev(
    decisions: Option<&Arc<Decisions>>,
    note_title: &str,
    note_content: &str,
    note_tags: &[String],
    candidates: &[PlacementCandidate],
) -> Option<String> {
    let decisions = decisions?;
    let mode = decisions.mode(SITE_TASK_PLACEMENT);
    if mode == DecisionMode::Off
        || candidates.is_empty()
        || candidates.len() > MAX_PLACEMENT_CANDIDATES
    {
        return None;
    }

    let title = redact_secrets(note_title);
    let excerpt = redact_secrets(
        &note_content
            .chars()
            .take(NOTE_DECISION_CHARS)
            .collect::<String>(),
    );
    let tags = note_tags.join(", ");
    let state = format!("Note title: {title}\nTags: {tags}\nExcerpt:\n{excerpt}");
    let options: Vec<(&str, &str)> = candidates
        .iter()
        .map(|c| (c.initiative.id.as_str(), c.label.as_str()))
        .collect();
    let request = DecisionRequest::new(&state).choice_or_abstain(
        "task_placement",
        "Which existing task List (shown as Space > Folder > List) best fits filing this note as a \
         task, by topic? Pick the closest match, not an exact wording match.",
        &options,
        "none of the lists are a good fit for this note",
    );

    if mode == DecisionMode::Shadow {
        decisions.shadow(
            SITE_TASK_PLACEMENT,
            request,
            json!({ "decision": "heuristic_or_new" }),
        );
        return None;
    }

    let response = decisions.ask(&request).await.ok()?;
    let picked = match response.choice_or_abstain("task_placement").ok()? {
        ChoiceOrAbstain::Picked(id) => Some(id.to_string()),
        ChoiceOrAbstain::Unclear => None,
    };
    decisions.record(
        SITE_TASK_PLACEMENT,
        json!({ "picked": picked, "candidate_count": candidates.len() }),
    );
    picked
}

pub(super) struct TaskCreateFromNoteTool {
    pub(super) vault: Arc<VaultClient>,
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskCreateFromNoteTool {
    fn name(&self) -> &str {
        "task_create_from_note"
    }
    fn description(&self) -> &str {
        "Promote a vault note into a task. Reads the note and places the task in the best-fitting \
         existing Space > Folder > List, sharpened by the Jev structured-decision layer when it's \
         configured (falls back to a tag/folder-name match otherwise), or creates a new List (and a new \
         Space, only if none exist at all) when nothing fits. Pass space_id/folder/initiative to force a \
         destination instead of auto-placing."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "note_path": { "type": "string", "description": "Vault-relative path to the note to promote, e.g. Notebooks/Projects/foo.md" },
                "space_id": { "type": "string", "description": "Force a specific Space slug instead of auto-placing" },
                "folder": { "type": "string", "description": "Optional folder name, used only when space_id is given explicitly" },
                "initiative": { "type": "string", "description": "Optional List/initiative name, used only when space_id is given explicitly", "default": "Inbox" },
                "created_by": { "type": "string", "description": "Who is filing this (agent id or a name)", "default": "hq" }
            },
            "required": ["note_path"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    fn search_hint(&self) -> Option<&str> {
        Some("promote a vault note into a task, auto-placed into the right list")
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let note_path = arg_str(&args, "note_path");
        if note_path.is_empty() {
            bail!("note_path is required");
        }
        let explicit_space = args
            .get("space_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);
        let explicit_folder = args
            .get("folder")
            .and_then(|v| v.as_str())
            .map(String::from);
        let explicit_initiative = args
            .get("initiative")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);
        let created_by = {
            let v = arg_str(&args, "created_by");
            if v.is_empty() { "hq".to_string() } else { v }
        };

        let vault = self.vault.clone();
        let path_for_read = note_path.clone();
        let note = tokio::task::spawn_blocking(move || vault.read_note(&path_for_read)).await??;

        let title: String = note.title.chars().take(200).collect();
        let excerpt: String = note.content.chars().take(NOTE_EXCERPT_CHARS).collect();
        let description = format!("{excerpt}\n\n---\nSource: vault note `{note_path}`");
        let tags = note.tags.clone();

        let (space_id, folder_name, initiative_name, initiative_id, created_new_destination) =
            match explicit_space {
                Some(space_id) => {
                    let initiative_name =
                        explicit_initiative.unwrap_or_else(|| "Inbox".to_string());
                    (space_id, explicit_folder, initiative_name, None, false)
                }
                None => {
                    let candidates = self.db.with_conn(list_placement_candidates)?;
                    let decisions = hq_llm::decision::get();
                    let jev_pick = choose_placement_via_jev(
                        decisions.as_ref(),
                        &note.title,
                        &note.content,
                        &tags,
                        &candidates,
                    )
                    .await;
                    let chosen = jev_pick
                        .as_deref()
                        .and_then(|id| candidates.iter().find(|c| c.initiative.id == id))
                        .or_else(|| {
                            heuristic_placement(&note.title, &tags, &note_path, &candidates)
                        });
                    match chosen {
                        Some(c) => (
                            c.initiative.space_id.clone(),
                            c.folder_name.clone(),
                            c.initiative.name.clone(),
                            Some(c.initiative.id.clone()),
                            false,
                        ),
                        None => {
                            let new_list_name =
                                nearest_dir_name(&note_path).unwrap_or_else(|| "Inbox".to_string());
                            let (tags_for_space, path_for_space) =
                                (tags.clone(), note_path.clone());
                            let space = self.db.with_conn(move |c| {
                                choose_or_create_space(c, &tags_for_space, &path_for_space)
                            })?;
                            (space.id, None, new_list_name, None, true)
                        }
                    }
                }
            };

        let id = generate_id("tk");
        let (task, _) = self.db.with_conn(move |c| {
            create_task_in(
                c,
                &id,
                initiative_id.as_deref(),
                &Placement {
                    space_id: &space_id,
                    folder_name: folder_name.as_deref(),
                    initiative_name: &initiative_name,
                },
                &t::NewTask {
                    title: &title,
                    description: &description,
                    tags: &tags,
                    created_by: &created_by,
                    ..Default::default()
                },
            )
        })?;

        if !task.tags.is_empty() {
            let _ = mailbox::notify_tagged_agents(
                self.vault.vault_path(),
                &task.id,
                &task.display_id,
                &task.title,
                &task.tags,
            );
        }

        Ok(json!({
            "task": task_json(&task),
            "note_path": note_path,
            "created_new_destination": created_new_destination,
        }))
    }
}
