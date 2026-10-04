//! Exact BPE token counts for context assembly, with a heuristic fallback.

use super::cache_strategy::TokenizerDict;
use blake3;
use hq_core::tokens::count_tokens_fast;
use lru::LruCache;
use once_cell::sync::Lazy;
use std::num::NonZeroUsize;
use std::sync::RwLock; // Switched to RwLock for concurrency
use tiktoken_rs::CoreBPE;

/// A global LRU cache for token counts.
/// Using RwLock to allow parallel cache hits without blocking.
pub static TOKEN_CACHE: Lazy<RwLock<LruCache<(blake3::Hash, TokenizerDict), usize>>> =
    Lazy::new(|| RwLock::new(LruCache::new(NonZeroUsize::new(10000).unwrap())));

// Building a BPE parses its whole rank table, so each dictionary is built once.
static CL100K_BPE: Lazy<Option<CoreBPE>> = Lazy::new(|| tiktoken_rs::cl100k_base().ok());
static O200K_BPE: Lazy<Option<CoreBPE>> = Lazy::new(|| tiktoken_rs::o200k_base().ok());

/// Safety factor (10%) to prevent context overflow when using heuristics.
const DRIFT_SAFETY_FACTOR: f64 = 1.10;

/// Deterministic token count using BPE.
/// Fallback to heuristic with safety buffer if exact BPE is missing.
pub fn count_tokens_deterministic(text: &str, dict: TokenizerDict) -> usize {
    let hash = blake3::hash(text.as_bytes());

    // Check cache (Read lock)
    if let Some(count) = TOKEN_CACHE
        .read()
        .ok()
        .and_then(|cache| cache.peek(&(hash, dict)).copied())
    {
        return count;
    }

    let count = match dict {
        TokenizerDict::Cl100kBase | TokenizerDict::O200kBase => {
            let bpe = match dict {
                TokenizerDict::Cl100kBase => &*CL100K_BPE,
                _ => &*O200K_BPE,
            };
            if let Some(bpe) = bpe {
                bpe.encode_with_special_tokens(text).len()
            } else {
                (count_tokens_fast(text) as f64 * DRIFT_SAFETY_FACTOR).ceil() as usize
            }
        }
        TokenizerDict::Claude | TokenizerDict::Llama3 => {
            // ADVERSARIAL FIX: Never proxy Claude/Lama with GPT-4.
            // Better to use safe heuristic with 10% overflow buffer.
            (count_tokens_fast(text) as f64 * DRIFT_SAFETY_FACTOR).ceil() as usize
        }
    };

    // Update cache (Write lock)
    if let Ok(mut cache) = TOKEN_CACHE.write() {
        cache.put((hash, dict), count);
    }

    count
}
