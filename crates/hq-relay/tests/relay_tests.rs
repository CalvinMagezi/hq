#[cfg(feature = "discord")]
use hq_relay::discord_helpers::format_attachment_descriptor;

#[cfg(feature = "discord")]
#[test]
fn attachment_descriptor_image() {
    let desc = format_attachment_descriptor("photo.png", 204_800, true);
    assert!(desc.contains("photo.png"), "should contain filename");
    assert!(desc.contains("200 KB"), "should contain human size");
    assert!(desc.contains("[Image]"), "should mark as image");
}

#[cfg(feature = "discord")]
#[test]
fn attachment_descriptor_document() {
    let desc = format_attachment_descriptor("notes.pdf", 1_048_576, false);
    assert!(desc.contains("notes.pdf"));
    assert!(desc.contains("1.0 MB"));
    assert!(desc.contains("[File]"));
}

#[cfg(feature = "discord")]
#[test]
fn embed_status_contains_required_fields() {
    use hq_relay::discord_helpers::build_status_embed;
    let debug = format!("{:?}", build_status_embed("sonnet", 15));
    assert!(debug.contains("HQ Status") && debug.contains("sonnet"));
}

#[test]
fn split_message_respects_discord_limit() {
    let long_text = "a".repeat(5000);
    let chunks = hq_relay::relay_common::split_message(&long_text, 2000);
    assert!(
        chunks.iter().all(|c| c.len() <= 2000),
        "all chunks must be <= 2000"
    );
    assert!(
        chunks.len() >= 3,
        "5000-char input needs at least 3 chunks at 2000 limit"
    );
    assert_eq!(
        chunks.join(""),
        long_text,
        "reassembled must equal original"
    );
}

#[test]
fn split_message_telegram_limit() {
    let long_text = "b".repeat(10000);
    let chunks = hq_relay::relay_common::split_message(&long_text, 4096);
    assert!(
        chunks.iter().all(|c| c.len() <= 4096),
        "all chunks must be <= 4096"
    );
    assert!(
        chunks.len() >= 3,
        "10000-char input needs at least 3 chunks at 4096 limit"
    );
    assert_eq!(
        chunks.join(""),
        long_text,
        "reassembled must equal original"
    );
}

#[test]
fn split_message_never_cuts_inside_a_character() {
    // "—" is 3 bytes, so 1999 ASCII bytes put the 2000-byte limit inside it.
    let text = format!("{}—{}", "a".repeat(1999), "é🙂".repeat(1500));
    let chunks = hq_relay::relay_common::split_message(&text, 2000);
    assert!(chunks.iter().all(|c| c.len() <= 2000));
    assert_eq!(chunks.concat(), text);
}

#[test]
fn split_message_makes_progress_when_the_limit_is_below_one_char() {
    let chunks = hq_relay::relay_common::split_message("🙂🙂", 2);
    assert_eq!(chunks, vec!["🙂", "🙂"]);
}

#[test]
fn split_message_preserves_short_text() {
    let text = "hello world";
    let chunks = hq_relay::relay_common::split_message(text, 2000);
    assert_eq!(
        chunks.len(),
        1,
        "short text should produce exactly one chunk"
    );
    assert_eq!(chunks[0], text, "chunk must equal original");
}
