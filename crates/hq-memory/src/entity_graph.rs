//! Memory Entity Graph implementation.
//! Handles entity resolution, edge building (co-occurrence and relationships),
//! and spreading activation for retrieval.

use crate::types::{ActivatedEntity, EntityEdge};
use anyhow::Result;
use chrono::Utc;
use hq_db::Database;
use std::collections::{HashMap, HashSet, VecDeque};

/// Resolve an entity name to its canonical ID.
/// Performs lowercase lookup and inserts if missing.
pub fn resolve_entity(db: &Database, name: &str) -> Result<i64> {
    let canonical = name.to_lowercase().trim().to_string();
    if canonical.is_empty() {
        return Err(anyhow::anyhow!("Empty entity name"));
    }

    db.with_conn(|conn| {
        let existing: Option<i64> = conn.query_row(
            "SELECT id FROM entity_nodes WHERE canonical = ?1",
            [&canonical],
            |row| row.get(0),
        ).ok();

        if let Some(id) = existing {
            // Bump mention count
            conn.execute(
                "UPDATE entity_nodes SET mention_count = mention_count + 1, updated_at = ?1 WHERE id = ?2",
                rusqlite::params![Utc::now().to_rfc3339(), id],
            )?;
            Ok(id)
        } else {
            let entity_type = classify_entity_type(name);
            conn.execute(
                "INSERT INTO entity_nodes (canonical, display_name, entity_type, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                rusqlite::params![canonical, name, entity_type, Utc::now().to_rfc3339()],
            )?;
            Ok(conn.last_insert_rowid())
        }
    })
}

/// Upsert an edge between two entities.
/// Increments weight if edge exists, or inserts new if not.
pub fn upsert_edge(
    db: &Database,
    source_id: i64,
    target_id: i64,
    relationship: &str,
    memory_id: Option<i64>,
) -> Result<()> {
    // Canonicalize direction: lower ID is always source to avoid duplicate undirected edges
    let (s, t) = if source_id < target_id {
        (source_id, target_id)
    } else {
        (target_id, source_id)
    };
    if s == t {
        return Ok(());
    }

    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO entity_edges (source_id, target_id, relationship, weight, source_memory_id, updated_at)
             VALUES (?1, ?2, ?3, 1.0, ?4, ?5)
             ON CONFLICT(source_id, target_id, relationship) DO UPDATE SET
                weight = weight + 1.0,
                updated_at = ?5",
            rusqlite::params![s, t, relationship, memory_id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    })
}

/// Simple heuristic to classify entity type.
pub fn classify_entity_type(name: &str) -> &str {
    let lower = name.to_lowercase();
    if lower.contains("inc")
        || lower.contains("ltd")
        || lower.contains("corp")
        || lower.contains("co.")
    {
        "org"
    } else if lower.starts_with('#') {
        "concept"
    } else if [
        "rust",
        "python",
        "javascript",
        "typescript",
        "sqlite",
        "postgresql",
        "docker",
        "kubernetes",
        "ollama",
        "claude",
        "gemini",
    ]
    .iter()
    .any(|&tech| lower.contains(tech))
    {
        "tool"
    } else if name.chars().any(|c| c.is_uppercase()) && name.contains(' ') {
        "person"
    } else {
        "unknown"
    }
}

/// Get all edges from/to a node.
pub fn get_edges_from(db: &Database, node_id: i64) -> Result<Vec<EntityEdge>> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT source_id, target_id, relationship, weight, source_memory_id, updated_at 
             FROM entity_edges WHERE source_id = ?1 OR target_id = ?1",
        )?;
        let rows = stmt.query_map([node_id], |row| {
            Ok(EntityEdge {
                source_id: row.get(0)?,
                target_id: row.get(1)?,
                relationship: row.get(2)?,
                weight: row.get(3)?,
                source_memory_id: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        let mut edges = Vec::new();
        for row in rows {
            edges.push(row?);
        }
        Ok(edges)
    })
}

/// Spreading Activation algorithm for entity retrieval.
/// Starting from seed entities, traverses the graph with decay.
pub fn spreading_activation(
    db: &Database,
    seed_entities: &[String],
    max_hops: usize,
    min_activation: f64,
    max_results: usize,
) -> Result<Vec<ActivatedEntity>> {
    let mut activation: HashMap<i64, f64> = HashMap::new();
    let mut visited: HashSet<i64> = HashSet::new();
    let mut queue: VecDeque<(i64, f64, usize)> = VecDeque::new();

    // 1. Initialize seeds
    db.with_conn(|conn| {
        for name in seed_entities {
            let canonical = name.to_lowercase();
            if let Ok(id) = conn.query_row(
                "SELECT id FROM entity_nodes WHERE canonical = ?1",
                [&canonical],
                |row| row.get::<_, i64>(0),
            ) {
                activation.insert(id, 1.0);
                queue.push_back((id, 1.0, 0));
            }
        }
        Ok::<(), anyhow::Error>(())
    })?;

    // 2. BFS with activation decay
    while let Some((node_id, current_activation, hop)) = queue.pop_front() {
        if hop >= max_hops || visited.contains(&node_id) {
            continue;
        }
        visited.insert(node_id);

        let edges = get_edges_from(db, node_id)?;
        for edge in edges {
            let neighbor = if edge.source_id == node_id {
                edge.target_id
            } else {
                edge.source_id
            };

            // Weight normalization: weight / (weight + 5.0)
            // Plus distance decay (halved each hop)
            let propagated = current_activation * (edge.weight / (edge.weight + 5.0)) * 0.5;

            if propagated >= min_activation {
                let entry = activation.entry(neighbor).or_insert(0.0);
                *entry = entry.max(propagated);
                queue.push_back((neighbor, propagated, hop + 1));
            }
        }
    }

    // 3. Resolve names and sort
    let mut results = Vec::new();
    db.with_conn(|conn| {
        for (id, score) in activation {
            let mut stmt = conn.prepare(
                "SELECT canonical, display_name, entity_type FROM entity_nodes WHERE id = ?1",
            )?;
            let row: Option<(String, String, String)> = stmt
                .query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .ok();

            if let Some((canonical, display, etype)) = row {
                results.push(ActivatedEntity {
                    id,
                    canonical,
                    display_name: display,
                    entity_type: etype,
                    activation: score,
                });
            }
        }
        Ok::<(), anyhow::Error>(())
    })?;

    results.sort_by(|a, b| {
        b.activation
            .partial_cmp(&a.activation)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(results.into_iter().take(max_results).collect())
}

