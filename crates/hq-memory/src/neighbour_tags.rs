//! Tags borrowed from the closest tagged notes, with the on-device embedder: what the inbox
//! triage and the model-free memory extraction use instead of asking a model for tags. Tags stay
//! inside the vocabulary the vault already uses, and a text with no close, agreeing neighbours
//! gets none rather than a guess.

use anyhow::Result;
use hq_db::Database;

/// Neighbours consulted.
const NEIGHBOURS: usize = 8;
/// A neighbour below this cosine similarity is too far away to lend its tags. Tuned on
/// potion-base-8M, where related notes score about 0.4 to 0.6 and unrelated ones under 0.25.
const MIN_SIMILARITY: f32 = 0.4;
/// A tag is borrowed only when at least this many neighbours carry it.
const MIN_VOTES: usize = 2;
const MAX_TAGS: usize = 3;

/// The tags the notes closest to `text` agree on. `own_path` (vault-relative) is left out of
/// the neighbours, for a note that is already embedded.
pub async fn neighbour_tags(
    db: &Database,
    text: &str,
    own_path: Option<&str>,
) -> Result<Vec<String>> {
    // No local vectors yet means no neighbours: skip loading (or fetching) the model.
    let indexed: bool = db.with_conn(|conn| {
        Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM embeddings WHERE model = ?1)",
            [crate::LOCAL_EMBEDDING_MODEL],
            |row| row.get(0),
        )?)
    })?;
    if !indexed {
        return Ok(Vec::new());
    }
    let embedding = crate::generate_embedding_local(text).await?;
    let neighbours = db.with_conn(|conn| {
        hq_db::search::semantic_search(
            conn,
            &embedding,
            crate::LOCAL_EMBEDDING_MODEL,
            NEIGHBOURS + 1,
        )
    })?;
    let votes: Vec<(f32, Vec<String>)> = neighbours
        .into_iter()
        .filter(|n| Some(n.note_path.as_str()) != own_path)
        .take(NEIGHBOURS)
        .map(|n| (n.relevance as f32, n.tags))
        .collect();
    Ok(vote_tags(&votes))
}

/// The tags at least `MIN_VOTES` close neighbours share, strongest support first.
// ponytail: similarity-weighted vote with fixed thresholds; calibrate against reviewed tags once
// the owner's corrections are recorded.
pub fn vote_tags(neighbours: &[(f32, Vec<String>)]) -> Vec<String> {
    let mut support: Vec<(String, usize, f32)> = Vec::new();
    for (similarity, tags) in neighbours {
        if *similarity < MIN_SIMILARITY {
            continue;
        }
        for tag in tags {
            match support.iter_mut().find(|(t, _, _)| t == tag) {
                Some(entry) => {
                    entry.1 += 1;
                    entry.2 += similarity;
                }
                None => support.push((tag.clone(), 1, *similarity)),
            }
        }
    }
    support.retain(|(_, votes, _)| *votes >= MIN_VOTES);
    support.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    support
        .into_iter()
        .take(MAX_TAGS)
        .map(|(tag, _, _)| tag)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::vote_tags;

    fn n(similarity: f32, tags: &[&str]) -> (f32, Vec<String>) {
        (similarity, tags.iter().map(|t| t.to_string()).collect())
    }

    #[test]
    fn tags_come_from_close_neighbours_that_agree() {
        let tags = vote_tags(&[
            n(0.7, &["android", "edge-ai"]),
            n(0.6, &["edge-ai", "research"]),
            n(0.5, &["android", "edge-ai"]),
            n(0.3, &["research", "research-noise"]),
            n(0.2, &["research"]),
        ]);
        // research has one close vote; the far neighbours do not count.
        assert_eq!(tags, ["edge-ai", "android"]);
    }

    #[test]
    fn no_agreement_means_no_tags() {
        assert!(vote_tags(&[n(0.9, &["a"]), n(0.8, &["b"])]).is_empty());
        assert!(vote_tags(&[]).is_empty());
    }
}
