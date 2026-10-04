use crate::context::cache_strategy::TokenizerDict;
use crate::context::reducer::{ContextError, ContextReducer};
use async_trait::async_trait;
use std::sync::Arc;

struct DummyReducer;

#[async_trait]
impl ContextReducer for DummyReducer {
    async fn reduce(
        &self,
        text: Arc<str>,
        _target: usize,
        _dict: TokenizerDict,
    ) -> Result<(String, usize), ContextError> {
        Ok((text.to_string(), text.len()))
    }

    fn name(&self) -> &'static str {
        "Dummy"
    }
}

#[tokio::test]
async fn test_dummy_reducer() {
    let reducer = DummyReducer;
    let res = reducer
        .reduce(Arc::from("hello"), 10, TokenizerDict::Cl100kBase)
        .await
        .unwrap();
    assert_eq!(res.0, "hello");
}
