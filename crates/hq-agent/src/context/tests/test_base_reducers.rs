use crate::context::cache_strategy::TokenizerDict;
use crate::context::reducer::{ContextReducer, WhitespaceReducer};
use std::sync::Arc;

#[tokio::test]
async fn test_whitespace_reducer() {
    let reducer = WhitespaceReducer;
    let test_str = "hello    world\n\n\ntest";
    let (reduced, _tokens) = reducer
        .reduce(Arc::from(test_str), 50, TokenizerDict::Cl100kBase)
        .await
        .unwrap();
    assert_eq!(reduced, "hello world test");
}

#[tokio::test]
async fn test_deduplication_reducer() {
    use crate::context::reducer::DeduplicationReducer;
    let reducer = DeduplicationReducer;
    let test_str = "error 1\nerror 1\nerror 2\nerror 2";
    let (reduced, _tokens) = reducer
        .reduce(Arc::from(test_str), 50, TokenizerDict::Cl100kBase)
        .await
        .unwrap();
    assert_eq!(reduced, "error 1\nerror 2");
}
