//! Note embeddings made on this machine with a static Model2Vec model (`potion-base-8M`): a
//! token lookup and a mean, about 36 MB resident and well under a millisecond per note on a
//! budget phone. The model files are fetched once from a pinned revision and checked against
//! pinned hashes; after that nothing leaves the machine.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use model2vec_rs::model::StaticModel;
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;

/// The label stored with every vector this model makes. A new revision is a new label, so
/// every note is embedded again when the model changes.
pub const LOCAL_EMBEDDING_MODEL: &str = "model2vec/potion-base-8M@bf8b0566";

const REPO: &str = "minishlab/potion-base-8M";
const REVISION: &str = "bf8b056651a2c21b8d2565580b8569da283cab23";
const DOWNLOAD_BASE: &str = "https://huggingface.co";

/// The files the model needs, each with the SHA-256 of the pinned revision.
const MODEL_FILES: [(&str, &str); 3] = [
    (
        "config.json",
        "2a6ac0e9aaa356a68a5688070db78fc3a464fefe85d2f06a1905ce3718687553",
    ),
    (
        "model.safetensors",
        "f65d0f325faadc1e121c319e2faa41170d3fa07d8c89abd48ca5358d9a223de2",
    ),
    (
        "tokenizer.json",
        "e67e803f624fb4d67dea1c730d06e1067e1b14d830e2c2202569e3ef0f70bb50",
    ),
];

/// Tokens read from each text. Pooling is a mean, so a longer window costs little; the library
/// default of 512 would describe a long note by its opening only.
const MAX_TOKENS: usize = 2048;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The largest model file accepted; the real ones are under 30 MB.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// After a failed fetch or load, calls fail at once for this long instead of each waiting on
/// the network again; searches fall back to keywords meanwhile.
const RETRY_AFTER: Duration = Duration::from_secs(600);

static MODEL: OnceCell<Arc<StaticModel>> = OnceCell::const_new();
static LAST_FAILURE: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

/// Where the model files live: `~/.hq/models/potion-base-8M`.
pub fn local_model_dir() -> PathBuf {
    hq_core::config::HqConfig::hq_dir()
        .join("models")
        .join("potion-base-8M")
}

/// Embed `text` on this machine, fetching and checking the model files on first use. A failed
/// fetch or load is returned as an error and tried again on the next call.
pub async fn generate_embedding_local(text: &str) -> Result<Vec<f32>> {
    let model = model().await?;
    let text = text.to_string();
    let started = Instant::now();
    let embedding = tokio::task::spawn_blocking(move || {
        model
            .encode_with_args(&[text], Some(MAX_TOKENS), 1)
            .pop()
            .unwrap_or_default()
    })
    .await?;
    record(started.elapsed());
    if embedding.is_empty() {
        bail!("the local model returned an empty embedding");
    }
    Ok(embedding)
}

async fn model() -> Result<Arc<StaticModel>> {
    if let Some(model) = MODEL.get() {
        return Ok(model.clone());
    }
    let failed_at = *LAST_FAILURE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(at) = failed_at.filter(|at| at.elapsed() < RETRY_AFTER) {
        bail!(
            "the local embedding model is unavailable; retrying in {}s",
            (RETRY_AFTER - at.elapsed()).as_secs()
        );
    }
    let loaded = MODEL.get_or_try_init(|| load(local_model_dir())).await;
    if loaded.is_err() {
        *LAST_FAILURE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }
    Ok(loaded?.clone())
}

/// Files the ledger row so the spend report shows local embeddings beside remote ones.
fn record(elapsed: Duration) {
    let call = hq_llm::ExternalCall {
        provider: "local",
        class: hq_llm::cost::ProviderClass::Flat,
        model: LOCAL_EMBEDDING_MODEL,
        origin: hq_llm::origin::EMBEDDINGS,
        task_hint: "embed",
    };
    hq_llm::Instruments::global().record_external(&call, None, elapsed, None);
}

async fn load(dir: PathBuf) -> Result<Arc<StaticModel>> {
    ensure_model_files(&dir).await?;
    let started = Instant::now();
    let model = tokio::task::spawn_blocking(move || {
        StaticModel::from_pretrained(&dir, None, None, None)
            .with_context(|| format!("loading the embedding model from {}", dir.display()))
    })
    .await??;
    tracing::info!(
        model = LOCAL_EMBEDDING_MODEL,
        load_ms = started.elapsed().as_millis() as u64,
        "local embedding model loaded"
    );
    Ok(Arc::new(model))
}

/// Make sure every model file is in `dir` with its pinned hash, downloading any that is missing
/// or wrong. A download is written beside its target and renamed only once its hash matches, so a
/// cut-off transfer never leaves a file that looks complete.
async fn ensure_model_files(dir: &Path) -> Result<()> {
    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("creating {}", dir.display()))?;
    for (name, sha256) in MODEL_FILES {
        let target = dir.join(name);
        if file_matches(&target, sha256).await {
            continue;
        }
        let url = format!("{DOWNLOAD_BASE}/{REPO}/resolve/{REVISION}/{name}");
        tracing::info!(%url, "downloading local embedding model file");
        let bytes = download(&url).await?;
        install(Arc::new(bytes), &target, sha256).await?;
    }
    Ok(())
}

async fn download(url: &str) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .context("building the download client")?;
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("fetching {url}"))?;
    if !response.status().is_success() {
        bail!("fetching {url}: HTTP {}", response.status());
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_FILE_BYTES)
    {
        bail!("fetching {url}: larger than {MAX_FILE_BYTES} bytes");
    }
    let bytes = response.bytes().await?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        bail!("fetching {url}: larger than {MAX_FILE_BYTES} bytes");
    }
    Ok(bytes.to_vec())
}

/// SHA-256 of a model file, off the async threads: hashing 30 MB takes a while on a phone.
async fn sha256_hex(bytes: Arc<Vec<u8>>) -> Result<String> {
    Ok(tokio::task::spawn_blocking(move || hex::encode(Sha256::digest(bytes.as_slice()))).await?)
}

async fn file_matches(path: &Path, sha256: &str) -> bool {
    match tokio::fs::read(path).await {
        Ok(bytes) => sha256_hex(Arc::new(bytes))
            .await
            .is_ok_and(|actual| actual == sha256),
        Err(_) => false,
    }
}

/// Write `bytes` to `target` only if they hash to `sha256`. The temporary name is unique to this
/// process, so two processes fetching at once never rename each other's half-written file; the
/// rename is atomic, so the target is always one complete, checked file.
async fn install(bytes: Arc<Vec<u8>>, target: &Path, sha256: &str) -> Result<()> {
    let actual = sha256_hex(bytes.clone()).await?;
    if actual != sha256 {
        bail!(
            "{} does not match its pinned hash (got {actual}, want {sha256})",
            target.display()
        );
    }
    let partial = target.with_extension(format!("part-{}", std::process::id()));
    tokio::fs::write(&partial, bytes.as_slice())
        .await
        .with_context(|| format!("writing {}", partial.display()))?;
    tokio::fs::rename(&partial, target)
        .await
        .with_context(|| format!("moving {} into place", target.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_file_whose_hash_does_not_match_is_never_installed() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("model.safetensors");
        let err = install(Arc::new(b"truncated".to_vec()), &target, MODEL_FILES[1].1)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("pinned hash"), "{err}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn a_matching_file_is_installed_and_then_recognised() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("x.json");
        let body = Arc::new(b"{}".to_vec());
        let sha = sha256_hex(body.clone()).await.unwrap();
        install(body, &target, &sha).await.unwrap();
        assert!(file_matches(&target, &sha).await);
        assert!(!file_matches(&target, MODEL_FILES[0].1).await);
    }

    /// Downloads the real model (about 30 MB) and embeds two related texts and one unrelated
    /// one. Opt in with `cargo test -p hq-memory local_embed -- --ignored`; needs the network.
    #[tokio::test]
    #[ignore = "downloads the model from huggingface.co"]
    async fn the_real_model_puts_related_texts_closer() {
        let dir = tempfile::tempdir().unwrap();
        let model = load(dir.path().to_path_buf()).await.unwrap();
        let texts = [
            "pair the phone with the laptop by scanning a QR code".to_string(),
            "scan the pairing code to connect a new device".to_string(),
            "a recipe for banana bread".to_string(),
        ];
        let v = model.encode_with_args(&texts, Some(MAX_TOKENS), texts.len());
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        assert!(dot(&v[0], &v[1]) > dot(&v[0], &v[2]));
    }
}
