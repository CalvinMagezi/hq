use super::*;

pub fn create_space(conn: &Connection, id: &str, name: &str, slug: &str) -> Result<Space> {
    conn.execute(
        "INSERT INTO spaces (id, name, slug) VALUES (?1, ?2, ?3)",
        params![id, name, slug],
    )?;
    notify_on_ok(conn, 
        get_space(conn, id)?
            .ok_or_else(|| anyhow::anyhow!("space {id} vanished immediately after creation")),
    )
}

pub fn get_space(conn: &Connection, id: &str) -> Result<Option<Space>> {
    Ok(conn
        .query_row(
            "SELECT id, name, slug, created_at FROM spaces WHERE id = ?1",
            params![id],
            row_to_space,
        )
        .optional()?)
}

pub fn list_spaces(conn: &Connection) -> Result<Vec<Space>> {
    let mut stmt = conn.prepare("SELECT id, name, slug, created_at FROM spaces ORDER BY name")?;
    let rows = stmt
        .query_map([], row_to_space)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

pub fn update_space(conn: &Connection, id: &str, name: &str) -> Result<Space> {
    let affected = conn.execute(
        "UPDATE spaces SET name = ?1 WHERE id = ?2",
        params![name, id],
    )?;
    if affected == 0 {
        anyhow::bail!("space {id} not found");
    }
    notify_on_ok(conn, 
        get_space(conn, id)?.ok_or_else(|| anyhow::anyhow!("space {id} vanished after update")),
    )
}


pub fn create_folder(
    conn: &Connection,
    id: &str,
    space_id: &str,
    name: &str,
    slug: &str,
) -> Result<Folder> {
    conn.execute(
        "INSERT INTO folders (id, space_id, name, slug) VALUES (?1, ?2, ?3, ?4)",
        params![id, space_id, name, slug],
    )?;
    notify_on_ok(conn, 
        get_folder(conn, id)?
            .ok_or_else(|| anyhow::anyhow!("folder {id} vanished immediately after creation")),
    )
}

pub fn get_folder(conn: &Connection, id: &str) -> Result<Option<Folder>> {
    Ok(conn
        .query_row(
            "SELECT id, space_id, name, slug, created_at FROM folders WHERE id = ?1",
            params![id],
            row_to_folder,
        )
        .optional()?)
}

pub fn list_folders(conn: &Connection, space_id: Option<&str>) -> Result<Vec<Folder>> {
    let (sql, filtered) = match space_id {
        Some(_) => (
            "SELECT id, space_id, name, slug, created_at FROM folders WHERE space_id = ?1 ORDER BY name",
            true,
        ),
        None => (
            "SELECT id, space_id, name, slug, created_at FROM folders ORDER BY name",
            false,
        ),
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = if filtered {
        stmt.query_map(params![space_id.unwrap()], row_to_folder)?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map([], row_to_folder)?
            .filter_map(|r| r.ok())
            .collect()
    };
    Ok(rows)
}


#[allow(clippy::too_many_arguments)]
pub fn create_initiative(
    conn: &Connection,
    id: &str,
    space_id: &str,
    folder_id: Option<&str>,
    name: &str,
    slug: &str,
    id_prefix: &str,
) -> Result<Initiative> {
    conn.execute(
        "INSERT INTO initiatives (id, space_id, folder_id, name, slug, id_prefix) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, space_id, folder_id, name, slug, id_prefix],
    )?;
    notify_on_ok(conn, 
        get_initiative(conn, id)?
            .ok_or_else(|| anyhow::anyhow!("initiative {id} vanished immediately after creation")),
    )
}

pub fn get_initiative(conn: &Connection, id: &str) -> Result<Option<Initiative>> {
    Ok(conn
        .query_row(
            &format!("SELECT {INITIATIVE_COLS} FROM initiatives WHERE id = ?1"),
            params![id],
            row_to_initiative,
        )
        .optional()?)
}

/// `folder_id` filter: `None` (the arg omitted) means no filter on folder;
/// `Some(None)` filters to folderless initiatives only; `Some(Some(id))`
/// filters to that folder.
pub fn list_initiatives(
    conn: &Connection,
    space_id: Option<&str>,
    folder_id: Option<Option<&str>>,
) -> Result<Vec<Initiative>> {
    let mut sql = format!("SELECT {INITIATIVE_COLS} FROM initiatives");
    let mut conditions: Vec<&'static str> = Vec::new();
    let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(sid) = space_id {
        conditions.push("space_id = ?");
        vals.push(Box::new(sid.to_string()));
    }
    match folder_id {
        Some(Some(fid)) => {
            conditions.push("folder_id = ?");
            vals.push(Box::new(fid.to_string()));
        }
        Some(None) => conditions.push("folder_id IS NULL"),
        None => {}
    }
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    sql.push_str(" ORDER BY name");

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), row_to_initiative)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Atomically allocates and returns the next `"{prefix}-{seq:03}"` display id
/// for an initiative. A single `UPDATE ... RETURNING` statement so the
/// read-and-increment can't race across pooled connections (a plain
/// read-then-write here would let two concurrent callers both observe the
/// same pre-increment value and mint duplicate display ids).
pub fn next_display_id(conn: &Connection, initiative_id: &str) -> Result<String> {
    let (seq, prefix): (i64, String) = conn.query_row(
        "UPDATE initiatives SET next_sequence = next_sequence + 1
         WHERE id = ?1
         RETURNING next_sequence - 1, id_prefix",
        params![initiative_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(format!("{prefix}-{seq:03}"))
}
