//! Slugs, id prefixes, and find-or-create lookups for spaces, folders, and initiatives.

use anyhow::{Result, bail};
use hq_db::tasks as t;

use crate::util::generate_id;

pub fn slug(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    s.split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(48)
        .collect()
}

pub(super) fn prefix_slug(name: &str) -> String {
    slug(name).to_uppercase()
}

/// Derives a collision-free `id_prefix` for a new initiative from its space
/// and name (`{SPACE}-{NAME}`, uppercased). Three real ClickUp Lists were all
/// literally named "List" in one Space each — a prefix from the name alone
/// isn't enough, so the space is folded in, with a numeric suffix as a last
/// resort against `initiatives.id_prefix`'s `UNIQUE` constraint. Folder is
/// deliberately not part of this: it's a display/organizational grouping,
/// not part of the id scheme.
pub fn derive_id_prefix(
    conn: &rusqlite::Connection,
    space_slug: &str,
    name: &str,
) -> Result<String> {
    let base = format!("{}-{}", space_slug.to_uppercase(), prefix_slug(name));
    let existing = t::list_initiatives(conn, None, None)?;
    if !existing.iter().any(|i| i.id_prefix == base) {
        return Ok(base);
    }
    for n in 2..100 {
        let candidate = format!("{base}-{n}");
        if !existing.iter().any(|i| i.id_prefix == candidate) {
            return Ok(candidate);
        }
    }
    bail!("could not derive a unique id_prefix from '{base}' after 100 attempts")
}

/// Resolves a caller-supplied Space reference to its real `id`, accepting
/// either the actual id or the slug. Every tool description in this file
/// that takes a `space_id` param calls it a "slug" (matching ClickUp-style
/// human-readable names like "personal"/"professional"), but `hq-db`'s own
/// lookups (`get_space`, the `initiatives.space_id`/`folders.space_id` FK
/// columns) only ever match the real `id` — which happens to equal the slug
/// for the two seeded default Spaces, masking this everywhere else. Errors
/// if neither matches, same as an unresolvable id always has.
pub fn resolve_space_id(conn: &rusqlite::Connection, space_id_or_slug: &str) -> Result<String> {
    if t::get_space(conn, space_id_or_slug)?.is_some() {
        return Ok(space_id_or_slug.to_string());
    }
    t::list_spaces(conn)?
        .into_iter()
        .find(|s| s.slug == space_id_or_slug)
        .map(|s| s.id)
        .ok_or_else(|| anyhow::anyhow!("space '{space_id_or_slug}' does not exist"))
}

/// Finds a folder by (space, name), creating one if none matches. The space
/// may be given by id or slug.
pub fn find_or_create_folder(
    conn: &rusqlite::Connection,
    space_id: &str,
    name: &str,
) -> Result<t::Folder> {
    let space_id = &resolve_space_id(conn, space_id)?;
    let target_slug = slug(name);
    let existing = t::list_folders(conn, Some(space_id))?;
    if let Some(found) = existing.into_iter().find(|f| f.slug == target_slug) {
        return Ok(found);
    }
    t::create_folder(conn, &generate_id("fo"), space_id, name, &target_slug)
}

/// Finds an initiative by (space, folder, name), creating one if none
/// matches. `folder_name` is optional — omit it for a folderless initiative
/// directly under the Space, same as ClickUp allows both.
pub fn find_or_create_initiative(
    conn: &rusqlite::Connection,
    space_id: &str,
    folder_name: Option<&str>,
    name: &str,
) -> Result<t::Initiative> {
    let space_id = &resolve_space_id(conn, space_id)?;
    let folder_id = match folder_name {
        Some(fname) if !fname.trim().is_empty() => {
            Some(find_or_create_folder(conn, space_id, fname)?.id)
        }
        _ => None,
    };
    let target_slug = slug(name);
    let existing = t::list_initiatives(conn, Some(space_id), None)?;
    if let Some(found) = existing
        .into_iter()
        .find(|i| i.slug == target_slug && i.folder_id == folder_id)
    {
        return Ok(found);
    }
    let space = t::get_space(conn, space_id)?
        .ok_or_else(|| anyhow::anyhow!("space '{space_id}' does not exist"))?;
    let id_prefix = derive_id_prefix(conn, &space.slug, name)?;
    t::create_initiative(
        conn,
        &generate_id("in"),
        space_id,
        folder_id.as_deref(),
        name,
        &target_slug,
        &id_prefix,
    )
}

/// Finds a space by slug, creating one (named `name`) if none matches. Only
/// used by the note-to-task auto-placement path when no existing Space fits —
/// unlike `find_or_create_folder`/`find_or_create_initiative`, `task_create`
/// itself never auto-creates a Space, since an unknown `space_id` there is
/// almost always a typo, not intent.
pub(super) fn find_or_create_space(conn: &rusqlite::Connection, name: &str) -> Result<t::Space> {
    let target_slug = slug(name);
    let existing = t::list_spaces(conn)?;
    if let Some(found) = existing.into_iter().find(|s| s.slug == target_slug) {
        return Ok(found);
    }
    t::create_space(conn, &generate_id("sp"), name, &target_slug)
}

/// Where a new task should be filed when no explicit initiative id is given.
pub struct Placement<'a> {
    pub space_id: &'a str,
    pub folder_name: Option<&'a str>,
    pub initiative_name: &'a str,
}

/// Resolves the target initiative and inserts the task row. A sub-task always
/// lands in its parent's initiative; otherwise the initiative comes from its
/// id or find-or-create by space/folder/name. Shared by `task_create`,
/// `task_create_from_note` and `harness_session_handoff`. The flag is false when
/// `new.external_id` matched a task the space already held.
pub fn create_task_in(
    conn: &rusqlite::Connection,
    id: &str,
    initiative_id: Option<&str>,
    placement: &Placement,
    new: &t::NewTask,
) -> Result<(t::Task, bool)> {
    let parent_initiative = match new.parent_task_id {
        Some(parent) => Some(
            t::get_task(conn, parent)?
                .ok_or_else(|| anyhow::anyhow!("parent task {parent} not found"))?
                .initiative_id,
        ),
        None => None,
    };
    let initiative_id = match (initiative_id, parent_initiative) {
        (Some(iid), _) => {
            t::get_initiative(conn, iid)?
                .ok_or_else(|| anyhow::anyhow!("initiative '{iid}' does not exist"))?
                .id
        }
        (None, Some(parent_iid)) => parent_iid,
        (None, None) => {
            find_or_create_initiative(
                conn,
                placement.space_id,
                placement.folder_name,
                placement.initiative_name,
            )?
            .id
        }
    };
    t::create_task_dedup(conn, id, &initiative_id, new)
}
