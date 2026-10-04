use super::fts::{extract_tags_from_frontmatter, sanitize_fts_query, strip_frontmatter};
use super::semantic::embedding_to_bytes;
use super::*;

fn setup_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!("../../sql/001_initial.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../sql/002_graph_links.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../sql/010_vault_cache.sql"))
        .unwrap();
    conn
}

#[test]
fn test_cosine_similarity_identical() {
    let a = vec![1.0, 2.0, 3.0];
    assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);
}

#[test]
fn test_cosine_similarity_orthogonal() {
    let a = vec![1.0, 0.0];
    let b = vec![0.0, 1.0];
    assert!((cosine_similarity(&a, &b)).abs() < 1e-6);
}

#[test]
fn test_cosine_similarity_zero_vector() {
    let a = vec![1.0, 2.0];
    let b = vec![0.0, 0.0];
    assert_eq!(cosine_similarity(&a, &b), 0.0);
}

#[test]
fn test_batch_cosine_similarity() {
    let query = vec![1.0, 0.0];
    let matrix = vec![1.0, 0.0, 0.0, 1.0, 0.5, 0.5];
    let scores = batch_cosine_similarity(&query, &matrix, 2);
    assert_eq!(scores.len(), 3);
    assert!((scores[0] - 1.0).abs() < 1e-6);
    assert!((scores[1]).abs() < 1e-6);
}

#[test]
fn test_embedding_roundtrip() {
    let original = vec![1.0f32, -2.5, std::f32::consts::PI, 0.0];
    let bytes = embedding_to_bytes(&original);
    let recovered = bytes_to_embedding(&bytes);
    assert_eq!(original, recovered);
}

#[test]
fn test_sanitize_fts_query() {
    assert_eq!(sanitize_fts_query("hello:world"), "\"hello\" \"world\"");
    assert_eq!(sanitize_fts_query("foo-bar"), "\"foo\" \"bar\"");
    assert_eq!(sanitize_fts_query("  spaces  "), "\"spaces\"");
    assert_eq!(sanitize_fts_query("?!-:*()\"' "), "");
}

#[test]
fn test_keyword_search_never_errors_on_free_text() {
    let conn = setup_test_db();
    index_note(
        &conn,
        "Notebooks/a.md",
        "AcmeCorp",
        "AcmeCorp is a platform AND more",
        "",
    )
    .unwrap();
    for q in [
        "What is AcmeCorp?",
        "\"unbalanced quote",
        "acme -corp",
        "a AND OR NOT b",
        "NEAR(x y)",
        "(paren",
        "star* col:on",
        "caf\u{e9} \u{65e5}\u{672c}\u{8a9e}",
        "?!?",
        "",
        "   ",
    ] {
        keyword_search(&conn, q, 5).unwrap_or_else(|e| panic!("{q:?}: {e}"));
    }
    let hits = keyword_search(&conn, "AcmeCorp?", 5).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(keyword_search(&conn, "?!?", 5).unwrap().is_empty());
    assert!(keyword_search(&conn, "", 5).unwrap().is_empty());
    // Operators are plain words, so "AND" matches the literal word.
    assert_eq!(
        keyword_search(&conn, "platform AND more", 5).unwrap().len(),
        1
    );
}

#[test]
fn test_index_and_keyword_search() {
    let conn = setup_test_db();
    index_note(
        &conn,
        "Notebooks/Projects/test.md",
        "Test Note",
        "hello world content",
        "rust search",
    )
    .unwrap();
    let results = keyword_search(&conn, "hello world", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].note_path, "Notebooks/Projects/test.md");
    assert_eq!(results[0].title, "Test Note");
    assert_eq!(results[0].notebook, "Projects");
}

#[test]
fn test_remove_note() {
    let conn = setup_test_db();
    index_note(&conn, "test.md", "Test", "content", "").unwrap();
    store_embedding(&conn, "test.md", &[1.0, 2.0, 3.0], "test-model").unwrap();
    remove_note(&conn, "test.md").unwrap();
    assert_eq!(indexed_count(&conn).unwrap(), 0);
    assert!(get_embedding(&conn, "test.md").unwrap().is_none());
}

#[test]
fn test_store_and_get_embedding() {
    let conn = setup_test_db();
    let emb = vec![0.1, 0.2, 0.3, 0.4];
    store_embedding(&conn, "note.md", &emb, "test-model").unwrap();
    let loaded = get_embedding(&conn, "note.md").unwrap().unwrap();
    assert_eq!(emb, loaded);
}

#[test]
fn test_get_stats() {
    let conn = setup_test_db();
    index_note(&conn, "a.md", "A", "content a", "").unwrap();
    index_note(&conn, "b.md", "B", "content b", "").unwrap();
    store_embedding(&conn, "a.md", &[1.0], "m").unwrap();

    let stats = get_stats(&conn).unwrap();
    assert_eq!(stats.fts_count, 2);
    assert_eq!(stats.embedding_count, 1);
}

#[test]
fn test_get_all_tags() {
    let conn = setup_test_db();
    index_note(&conn, "a.md", "A", "content", "rust search").unwrap();
    index_note(&conn, "b.md", "B", "content", "rust ai").unwrap();

    let tags = get_all_tags(&conn).unwrap();
    assert_eq!(tags.get("rust"), Some(&2));
    assert_eq!(tags.get("search"), Some(&1));
    assert_eq!(tags.get("ai"), Some(&1));
}

#[test]
fn test_semantic_search() {
    let conn = setup_test_db();
    index_note(&conn, "a.md", "Note A", "content a", "").unwrap();
    index_note(&conn, "b.md", "Note B", "content b", "").unwrap();
    store_embedding(&conn, "a.md", &[1.0, 0.0, 0.0], "m").unwrap();
    store_embedding(&conn, "b.md", &[0.0, 1.0, 0.0], "m").unwrap();

    let results = semantic_search(&conn, &[1.0, 0.0, 0.0], 10).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].note_path, "a.md"); // most similar
    assert!((results[0].relevance - 1.0).abs() < 1e-6);
}

#[test]
fn test_hybrid_search() {
    let conn = setup_test_db();
    index_note(
        &conn,
        "a.md",
        "Rust Programming",
        "rust language systems",
        "rust",
    )
    .unwrap();
    store_embedding(&conn, "a.md", &[1.0, 0.0], "m").unwrap();

    let results = hybrid_search(&conn, "rust", Some(&[1.0, 0.0]), 10).unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].match_type, MatchType::Hybrid);
}

#[test]
fn test_find_similar_notes() {
    let conn = setup_test_db();
    index_note(&conn, "a.md", "A", "c", "").unwrap();
    index_note(&conn, "b.md", "B", "c", "").unwrap();
    index_note(&conn, "c.md", "C", "c", "").unwrap();
    store_embedding(&conn, "a.md", &[1.0, 0.0], "m").unwrap();
    store_embedding(&conn, "b.md", &[0.9, 0.1], "m").unwrap();
    store_embedding(&conn, "c.md", &[0.0, 1.0], "m").unwrap();

    let results = find_similar_notes(&conn, "a.md", 5, 0.5).unwrap();
    assert_eq!(results.len(), 1); // only b.md is above 0.5 threshold
    assert_eq!(results[0].note_path, "b.md");
}

#[test]
fn test_strip_frontmatter() {
    let raw = "---\ntitle: Test\ntags:\n  - foo\n---\n# Hello\nWorld";
    let content = strip_frontmatter(raw);
    assert!(content.contains("# Hello"));
    assert!(!content.contains("title: Test"));
}

#[test]
fn test_extract_tags_list() {
    let raw = "---\ntitle: Test\ntags:\n  - foo\n  - bar\n---\ncontent";
    let tags = extract_tags_from_frontmatter(raw);
    assert_eq!(tags, "foo bar");
}

#[test]
fn test_extract_tags_inline() {
    let raw = "---\ntags: [alpha, beta]\n---\ncontent";
    let tags = extract_tags_from_frontmatter(raw);
    assert_eq!(tags, "alpha beta");
}

fn insert_vault_cache_row(conn: &Connection, path: &str, title: &str, mtime: i64) {
    conn.execute(
        "INSERT INTO vault_cache (path, mtime, hash, title, content_preview) VALUES (?1, ?2, 'h', ?3, 'preview')",
        rusqlite::params![path, mtime, title],
    )
    .unwrap();
}

#[test]
fn recent_notes_orders_newest_first_and_respects_limit() {
    let conn = setup_test_db();
    insert_vault_cache_row(&conn, "a.md", "A", 100);
    insert_vault_cache_row(&conn, "b.md", "B", 300);
    insert_vault_cache_row(&conn, "c.md", "C", 200);

    let results = recent_notes(&conn, 2).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].note_path, "b.md");
    assert_eq!(results[1].note_path, "c.md");
    assert_eq!(results[0].match_type, MatchType::Recent);
}

#[test]
fn get_related_paths_bidirectional_excludes_wikilinks() {
    let conn = setup_test_db();
    add_graph_link(&conn, "a.md", "b.md", 0.9, "suggested").unwrap();
    add_graph_link(&conn, "c.md", "a.md", 0.8, "applied").unwrap();
    add_graph_link(&conn, "/abs/a.md", "Some Title", 1.0, "wikilink").unwrap();

    let related = get_related_paths(&conn, "a.md", 10).unwrap();
    assert_eq!(related.len(), 2);
    assert!(related.contains(&"b.md".to_string()));
    assert!(related.contains(&"c.md".to_string()));
}
