use super::*;
use crate::tools::AgentTool;
use hq_tools::file_edit::{FileHistory, FileStateCache};
use serde_json::json;

#[test]
fn truncate_output_does_not_panic_on_a_multibyte_boundary() {
    // 50 bytes of a 3-byte-per-char string lands mid-character at byte 48
    // or 49 with a raw slice — this must truncate cleanly instead of
    // panicking on a non-UTF-8-boundary slice.
    let output = "€".repeat(30); // 90 bytes, 3 bytes per '€'
    let truncated = truncate_output(&output, 50);
    assert!(truncated.starts_with("€€€€€€€€€€€€€€€€"));
    assert!(truncated.contains("truncated at 50KB"));
}

#[test]
fn truncate_output_leaves_short_output_untouched() {
    assert_eq!(truncate_output("short", 50), "short");
}

#[tokio::test]
async fn find_tool_and_grep_tool_report_read_only() {
    assert!(FindTool.is_read_only());
    assert!(GrepTool.is_read_only());
}

fn edit_tool() -> (EditTool, tempfile::TempDir) {
    let cache = FileStateCache::default();
    let history = FileHistory::default();
    let tmp = tempfile::tempdir().unwrap();
    (EditTool::new(cache, history), tmp)
}

#[tokio::test]
async fn edit_tool_refuses_to_edit_a_file_it_has_not_read() {
    let (tool, tmp) = edit_tool();
    let path = tmp.path().join("a.txt");
    std::fs::write(&path, "hello world\n").unwrap();

    let result = tool
        .execute(
            "id",
            json!({
                "file_path": path.to_string_lossy(),
                "old_string": "hello",
                "new_string": "goodbye",
            }),
        )
        .await
        .unwrap();
    let text = &result.content[0].text;
    assert!(
        text.contains("has not been read"),
        "expected a staleness error, got: {text}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world\n");
}

#[tokio::test]
async fn edit_tool_refuses_a_no_op_edit() {
    let (tool, tmp) = edit_tool();
    let path = tmp.path().join("a.txt");
    std::fs::write(&path, "hello world\n").unwrap();
    tool.state_cache.record_read(&path);

    let result = tool
        .execute(
            "id",
            json!({
                "file_path": path.to_string_lossy(),
                "old_string": "hello",
                "new_string": "hello",
            }),
        )
        .await
        .unwrap();
    let text = &result.content[0].text;
    assert!(
        text.contains("identical"),
        "expected a no-op error, got: {text}"
    );
}

#[tokio::test]
async fn edit_tool_applies_a_valid_edit_after_a_read() {
    let (tool, tmp) = edit_tool();
    let path = tmp.path().join("a.txt");
    std::fs::write(&path, "hello world\n").unwrap();
    tool.state_cache.record_read(&path);

    let result = tool
        .execute(
            "id",
            json!({
                "file_path": path.to_string_lossy(),
                "old_string": "hello",
                "new_string": "goodbye",
            }),
        )
        .await
        .unwrap();
    let text = &result.content[0].text;
    assert!(text.contains("Successfully replaced"), "got: {text}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "goodbye world\n");
}

#[tokio::test]
async fn edit_tool_re_stales_after_a_disk_change_since_the_last_read() {
    let (tool, tmp) = edit_tool();
    let path = tmp.path().join("a.txt");
    std::fs::write(&path, "hello world\n").unwrap();
    tool.state_cache.record_read(&path);

    // A change on disk after the read, not through this tool.
    std::thread::sleep(std::time::Duration::from_millis(10));
    std::fs::write(&path, "something else entirely\n").unwrap();

    let result = tool
        .execute(
            "id",
            json!({
                "file_path": path.to_string_lossy(),
                "old_string": "hello",
                "new_string": "goodbye",
            }),
        )
        .await
        .unwrap();
    let text = &result.content[0].text;
    assert!(
        text.contains("modified on disk"),
        "expected a staleness error, got: {text}"
    );
}

#[tokio::test]
async fn read_tool_populates_the_cache_edit_tool_checks() {
    let cache = FileStateCache::default();
    let history = FileHistory::default();
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a.txt");
    std::fs::write(&path, "hello world\n").unwrap();

    let read_tool = ReadTool::new(cache.clone());
    read_tool
        .execute("id", json!({"file_path": path.to_string_lossy()}))
        .await
        .unwrap();

    // The same cache instance ReadTool just populated is what EditTool
    // checks — this is the load-bearing assumption behind adding
    // record_read() to ReadTool instead of bundling a swap of both tools.
    let edit_tool = EditTool::new(cache, history);
    let result = edit_tool
        .execute(
            "id",
            json!({
                "file_path": path.to_string_lossy(),
                "old_string": "hello",
                "new_string": "goodbye",
            }),
        )
        .await
        .unwrap();
    assert!(result.content[0].text.contains("Successfully replaced"));
}
