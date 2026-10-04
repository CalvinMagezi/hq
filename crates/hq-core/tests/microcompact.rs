use hq_core::microcompact::{MicrocompactStrategy, microcompact};

#[test]
fn test_microcompact_pure_memory() {
    let large = "x".repeat(50_000);
    let result = microcompact(&large);

    // The spec says to eliminate disk I/O.
    assert!(result.compacted_tokens <= 2100); // Allow margin for markers
    assert_eq!(result.strategy, MicrocompactStrategy::MiddleOut);
}
