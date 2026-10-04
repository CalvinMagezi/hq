use crate::context::cache_strategy::TokenizerDict;
use crate::context::reducer::{ContextReducer, SemanticReducer};
use std::sync::Arc;

#[tokio::test]
async fn test_semantic_truncation() {
    let reducer = SemanticReducer;
    let data = r#"{"data": "very long string here that gets cut"}"#;
    // target_tokens is small enough that it triggers truncation
    let (reduced, _) = reducer
        .reduce(Arc::from(data), 5, TokenizerDict::Cl100kBase)
        .await
        .unwrap();
    println!("REDUCED: {}", reduced);

    assert!(
        reduced.contains("TRUNCATED"),
        "Result should contain truncation marker: {}",
        reduced
    );
    assert!(reduced.starts_with(r#"{"#));
}
