//! Token-budgeted context assembly: frames built from vault data, history and injected notes.

pub mod block;
pub mod bpe;
pub mod budget;
pub mod cache_strategy;
pub mod compactor;
pub mod engine;
pub mod layers;
pub mod reducer;
pub mod trust;

pub use engine::ContextEngine;
pub use layers::{ContextLayer, FrameInput};

#[cfg(test)]
mod tests {
    mod test_base_reducers;
    mod test_cache_strategy;
    mod test_ironclad_stress;
    mod test_reducer_trait;
    mod test_semantic_reducer;
}
