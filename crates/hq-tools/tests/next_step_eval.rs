//! How well the next-step suggestions do on invented tasks from domains the example bank does not
//! cover, with the real embedder. Opt in, since it downloads the model:
//! `cargo test -p hq-tools --test next_step_eval -- --ignored --nocapture`

use std::sync::Arc;

use hq_tools::capabilities::next_step::{nearest_capability, warm};
use hq_tools::vault::QueryEmbedder;
use serde::Deserialize;

const HELD_OUT: &str = include_str!("fixtures/next_step_heldout.jsonl");
const MIN_RIGHT_WHEN_SHOWN: f64 = 0.8;
const MIN_SILENT_WHEN_NOTHING_FITS: f64 = 0.7;

#[derive(Deserialize)]
struct Example {
    t: String,
    c: Option<String>,
}

#[tokio::test]
#[ignore = "downloads the embedding model from huggingface.co"]
async fn suggestions_are_right_when_shown_and_silent_when_nothing_fits() {
    let embed: QueryEmbedder =
        Arc::new(|q| Box::pin(async move { hq_memory::generate_embedding_local(&q).await }));
    warm(&embed).await.unwrap();
    let (mut shown, mut right, mut silent, mut none) = (0, 0, 0, 0);
    for line in HELD_OUT.lines() {
        let example: Example = serde_json::from_str(line).unwrap();
        let got = nearest_capability(&embed, &example.t).await;
        if example.c.is_none() {
            none += 1;
            silent += usize::from(got.is_none());
        }
        if let Some(capability) = got {
            shown += 1;
            right += usize::from(example.c.as_deref() == Some(capability));
        }
    }
    println!("shown {shown}, right {right}, silent {silent} of {none}");
    assert!(shown > 0);
    assert!(right as f64 / shown as f64 >= MIN_RIGHT_WHEN_SHOWN);
    assert!(silent as f64 / none as f64 >= MIN_SILENT_WHEN_NOTHING_FITS);
}
