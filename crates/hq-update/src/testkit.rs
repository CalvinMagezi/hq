//! Test doubles and a minisign signer (production code only verifies).

use crate::archive::tests::{Item, make_tgz};
use crate::config::UpdateConfig;
use crate::engine::Engine;
use crate::error::{Result, UpdateError};
use crate::manifest::{self, SCHEMA_VERSION};
use crate::ports::{Health, HealthInfo, Host, Http, Restarter};
use crate::state::{Installed, Layout};
use crate::verify::parse_public_key;
use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use blake2::{Blake2b512, Digest};
use ed25519_dalek::{Signer as _, SigningKey};
use sha2::Sha256;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

pub const BASE: &str = "https://fake.test";
pub const REPO_BASE: &str = "https://fake.test/acme/hq/releases/download";

pub struct Signer {
    sk: SigningKey,
    key_id: [u8; 8],
}

impl Signer {
    pub fn new() -> Self {
        let mut seed = [0u8; 32];
        let mut key_id = [0u8; 8];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut seed);
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut key_id);
        Self {
            sk: SigningKey::from_bytes(&seed),
            key_id,
        }
    }

    pub fn public_key_text(&self) -> String {
        let mut bin = b"Ed".to_vec();
        bin.extend_from_slice(&self.key_id);
        bin.extend_from_slice(self.sk.verifying_key().as_bytes());
        format!("untrusted comment: test key\n{}\n", B64.encode(bin))
    }

    pub fn public_key(&self) -> minisign_verify::PublicKey {
        parse_public_key(&self.public_key_text()).unwrap()
    }

    /// A prehashed (`minisign -S` default) signature file.
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        let hash = Blake2b512::digest(data);
        let sig = self.sk.sign(&hash).to_bytes();
        let mut bin1 = b"ED".to_vec();
        bin1.extend_from_slice(&self.key_id);
        bin1.extend_from_slice(&sig);
        let comment = "t=1";
        let mut global_msg = sig.to_vec();
        global_msg.extend_from_slice(comment.as_bytes());
        let global = self.sk.sign(&global_msg).to_bytes();
        format!(
            "untrusted comment: signature\n{}\ntrusted comment: {comment}\n{}\n",
            B64.encode(bin1),
            B64.encode(global)
        )
        .into_bytes()
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn binary_content(version: &str, sha: &str) -> Vec<u8> {
    format!("version={version} sha={sha}\n").into_bytes()
}

pub struct ReleaseSpec {
    pub version: String,
    pub git_sha: String,
    pub requires_db_snapshot: bool,
    pub with_web: bool,
    pub min_updater: String,
    /// What the binary inside the tarball claims to be; defaults to the manifest.
    pub binary: Option<Vec<u8>>,
    pub web_items: Option<Vec<(String, Vec<u8>)>>,
}

impl ReleaseSpec {
    pub fn new(version: &str, sha: &str) -> Self {
        Self {
            version: version.into(),
            git_sha: sha.into(),
            requires_db_snapshot: false,
            with_web: true,
            min_updater: "0.9.0".into(),
            binary: None,
            web_items: None,
        }
    }
}

pub struct FakeHttp {
    pub base: String,
    pub files: Mutex<HashMap<String, Vec<u8>>>,
}

impl FakeHttp {
    pub fn new(base: &str) -> Self {
        Self {
            base: base.into(),
            files: Mutex::default(),
        }
    }

    pub fn put(&self, url: impl Into<String>, bytes: Vec<u8>) {
        self.files.lock().unwrap().insert(url.into(), bytes);
    }

    pub fn get_raw(&self, url: &str) -> Option<Vec<u8>> {
        self.files.lock().unwrap().get(url).cloned()
    }

    pub fn remove(&self, url: &str) {
        self.files.lock().unwrap().remove(url);
    }

    /// Publishes a signed release under `REPO_BASE/v<version>/`.
    pub fn publish_release(&self, signer: &Signer, spec: &ReleaseSpec, scratch: &Path) -> Vec<u8> {
        let v = &spec.version;
        let dir = format!("{}/v{v}", self.base);
        let bin_name = manifest::binary_artifact_name(v);
        let tgz = scratch.join(&bin_name);
        let content = spec
            .binary
            .clone()
            .unwrap_or_else(|| binary_content(v, &spec.git_sha));
        make_tgz(&tgz, &[Item::File("hq", &content)]);
        let bin_bytes = std::fs::read(&tgz).unwrap();
        let mut artifacts = vec![
            serde_json::json!({"name": bin_name, "sha256": sha256_hex(&bin_bytes), "size": bin_bytes.len()}),
        ];
        self.put(format!("{dir}/{bin_name}"), bin_bytes);

        if spec.with_web {
            let web_name = manifest::web_artifact_name(v);
            let items: Vec<(String, Vec<u8>)> = spec.web_items.clone().unwrap_or_else(|| {
                ["index.html", "sw.js", "manifest.json"]
                    .iter()
                    .map(|n| (n.to_string(), format!("web {v} {n}").into_bytes()))
                    .collect()
            });
            let refs: Vec<Item> = items.iter().map(|(n, d)| Item::File(n, d)).collect();
            let web_tgz = scratch.join(&web_name);
            make_tgz(&web_tgz, &refs);
            let web_bytes = std::fs::read(&web_tgz).unwrap();
            artifacts.push(serde_json::json!({"name": web_name, "sha256": sha256_hex(&web_bytes), "size": web_bytes.len()}));
            self.put(format!("{dir}/{web_name}"), web_bytes);
        }
        let manifest = serde_json::json!({
            "schema": SCHEMA_VERSION, "version": v, "git_sha": spec.git_sha, "channel": "main",
            "built_at": "2026-10-04T00:00:00Z", "min_updater_version": spec.min_updater,
            "requires_db_snapshot": spec.requires_db_snapshot, "artifacts": artifacts,
        });
        self.put_signed(
            signer,
            &format!("{dir}/manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
    }

    pub fn put_signed(&self, signer: &Signer, url: &str, body: Vec<u8>) -> Vec<u8> {
        self.put(format!("{url}.minisig"), signer.sign(&body));
        self.put(url, body.clone());
        body
    }

    /// Points `channel` at an already published release.
    pub fn point_channel(&self, signer: &Signer, channel: &str, version: &str) {
        self.point_channel_with(signer, channel, version, None, None);
    }

    pub fn point_channel_with(
        &self,
        signer: &Signer,
        channel: &str,
        version: &str,
        seq: Option<u64>,
        issued_at: Option<&str>,
    ) {
        let manifest_url = format!("{}/v{version}/manifest.json", self.base);
        let manifest = self
            .get_raw(&manifest_url)
            .expect("release published first");
        let mut pointer = serde_json::json!({
            "schema": SCHEMA_VERSION, "channel": channel, "version": version, "manifest_url": manifest_url,
            "manifest_sha256": sha256_hex(&manifest),
        });
        if let Some(seq) = seq {
            pointer["seq"] = seq.into();
        }
        if let Some(at) = issued_at {
            pointer["issued_at"] = at.into();
        }
        let url = format!("{}/channel-{channel}/channel-{channel}.json", self.base);
        self.put_signed(signer, &url, serde_json::to_vec(&pointer).unwrap());
    }
}

#[async_trait]
impl Http for FakeHttp {
    async fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>> {
        let body = self
            .get_raw(url)
            .ok_or_else(|| UpdateError::Download(format!("{url}: HTTP 404")))?;
        if body.len() as u64 > max_bytes {
            return Err(UpdateError::SizeLimit {
                name: url.into(),
                limit: max_bytes,
            });
        }
        Ok(body)
    }

    async fn download(&self, url: &str, dest: &Path, max_bytes: u64) -> Result<()> {
        let body = self.get(url, max_bytes).await?;
        std::fs::write(dest, body)?;
        Ok(())
    }
}

/// A simulated host: "restarting" reads the installed binary file to decide
/// what is running, so swaps and rollbacks are observed, not assumed.
pub struct World {
    pub bin_path: std::path::PathBuf,
    pub running_sha: Mutex<Option<String>>,
    pub unhealthy_shas: Mutex<HashSet<String>>,
    pub migrating_shas: Mutex<HashSet<String>>,
    pub count_fails_shas: Mutex<HashSet<String>>,
    pub migrations: Mutex<Option<u64>>,
    pub db_content: Mutex<String>,
    pub events: Mutex<Vec<String>>,
    pub upgrade_fails: Mutex<bool>,
    pub snapshot_fails: Mutex<bool>,
    pub restore_fails: Mutex<bool>,
    pub crash_on_restart: Mutex<bool>,
}

impl World {
    pub fn new(bin_path: &Path) -> Self {
        Self {
            bin_path: bin_path.to_path_buf(),
            running_sha: Mutex::new(None),
            unhealthy_shas: Mutex::default(),
            migrating_shas: Mutex::default(),
            count_fails_shas: Mutex::default(),
            migrations: Mutex::new(Some(5)),
            db_content: Mutex::new("db-v0".into()),
            events: Mutex::default(),
            upgrade_fails: Mutex::new(false),
            snapshot_fails: Mutex::new(false),
            restore_fails: Mutex::new(false),
            crash_on_restart: Mutex::new(false),
        }
    }

    pub fn log(&self, event: impl Into<String>) {
        self.events.lock().unwrap().push(event.into());
    }

    pub fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }

    fn sha_of(path: &Path) -> Option<String> {
        let text = std::fs::read_to_string(path).ok()?;
        text.split_whitespace()
            .find_map(|w| w.strip_prefix("sha="))
            .map(str::to_string)
    }
}

#[async_trait]
impl Restarter for World {
    async fn restart(&self) -> Result<()> {
        if std::mem::take(&mut *self.crash_on_restart.lock().unwrap()) {
            panic!("simulated crash before restart");
        }
        let sha = Self::sha_of(&self.bin_path).unwrap_or_default();
        self.log(format!("restart:{sha}"));
        if self.count_fails_shas.lock().unwrap().contains(&sha) {
            *self.migrations.lock().unwrap() = None;
        }
        if self.migrating_shas.lock().unwrap().contains(&sha) {
            let mut m = self.migrations.lock().unwrap();
            *m = m.map(|n| n + 1);
            *self.db_content.lock().unwrap() = format!("db-migrated-by-{sha}");
        }
        *self.running_sha.lock().unwrap() = Some(sha);
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.log("stop");
        *self.running_sha.lock().unwrap() = None;
        Ok(())
    }
}

#[async_trait]
impl Health for World {
    async fn probe(&self) -> Option<HealthInfo> {
        let sha = self.running_sha.lock().unwrap().clone()?;
        if self.unhealthy_shas.lock().unwrap().contains(&sha) {
            return None;
        }
        Some(HealthInfo {
            git_sha: Some(sha),
            version: None,
        })
    }
}

#[async_trait]
impl Host for World {
    async fn staged_version(&self, bin: &Path) -> Result<String> {
        let text = std::fs::read_to_string(bin).map_err(|e| anyhow::anyhow!(e))?;
        if text.contains("ENVFAULT") {
            return Err(anyhow::anyhow!("timed out running staged binary").into());
        }
        if text.contains("BROKEN") {
            return Err(anyhow::anyhow!("exec format error").into());
        }
        let field = |k: &str| {
            text.split_whitespace()
                .find_map(|w| w.strip_prefix(k))
                .unwrap_or("?")
                .to_string()
        };
        Ok(format!("hq {} ({})", field("version="), field("sha=")))
    }

    async fn post_upgrade(&self) -> Result<()> {
        self.log("install --upgrade");
        if *self.upgrade_fails.lock().unwrap() {
            return Err(anyhow::anyhow!("upgrade hook failed").into());
        }
        Ok(())
    }

    async fn notify(&self, phase: &str, sha: &str) {
        self.log(format!("notify:{phase}:{sha}"));
    }

    async fn alert(&self, message: &str) {
        self.log(format!("alert:{message}"));
    }

    async fn db_prune(
        &self,
        dir: &Path,
        keep: usize,
        protected: &[std::path::PathBuf],
    ) -> Result<()> {
        crate::dbops::prune_snapshots(dir, keep, protected);
        Ok(())
    }

    async fn db_snapshot(&self, dest: &Path) -> Result<bool> {
        if *self.snapshot_fails.lock().unwrap() {
            return Err(anyhow::anyhow!("snapshot failed").into());
        }
        std::fs::create_dir_all(dest.parent().unwrap())?;
        std::fs::write(dest, self.db_content.lock().unwrap().as_bytes())?;
        self.log("db-snapshot");
        Ok(true)
    }

    async fn db_restore(&self, snapshot: &Path) -> Result<()> {
        if *self.restore_fails.lock().unwrap() {
            return Err(anyhow::anyhow!("restore failed").into());
        }
        *self.db_content.lock().unwrap() = std::fs::read_to_string(snapshot)?;
        *self.migrations.lock().unwrap() = Some(5);
        self.log("db-restore");
        Ok(())
    }

    async fn db_migrations(&self) -> Result<Option<u64>> {
        Ok(*self.migrations.lock().unwrap())
    }
}

pub const OLD_SHA: &str = "aaaaaaa1111";
pub const NEW_SHA: &str = "bbbbbbb2222";

pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub signer: Signer,
    pub cfg: UpdateConfig,
    pub http: FakeHttp,
    pub world: World,
}

impl Fixture {
    /// Installed: 0.9.0 (`OLD_SHA`) with a web tree, service running.
    pub fn new() -> Self {
        Self::with_origin(BASE)
    }

    pub fn with_origin(origin: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("usr/bin")).unwrap();
        std::fs::create_dir_all(root.join("web/dist")).unwrap();
        std::fs::create_dir_all(root.join("scratch")).unwrap();
        let bin = root.join("usr/bin/hq");
        std::fs::write(&bin, binary_content("0.9.0", OLD_SHA)).unwrap();
        std::fs::write(root.join("web/dist/index.html"), "web 0.9.0 index.html").unwrap();
        let cfg = UpdateConfig {
            repo: "acme/hq".into(),
            channel: "main".into(),
            base_url: origin.into(),
            bin_path: bin.clone(),
            web_dist: root.join("web/dist"),
            state_dir: root.join("state"),
            snapshots_dir: root.join("snapshots"),
            health_tries: 2,
            health_interval_secs: 0,
            notify: crate::config::NotifyConfig {
                enabled: true,
                grace_secs: 0,
            },
            ..UpdateConfig::default()
        };
        let world = World::new(&bin);
        *world.running_sha.lock().unwrap() = Some(OLD_SHA.into());
        let http = FakeHttp::new(&format!("{origin}/acme/hq/releases/download"));
        Self {
            dir,
            signer: Signer::new(),
            cfg,
            http,
            world,
        }
    }

    pub fn scratch(&self) -> std::path::PathBuf {
        self.dir.path().join("scratch")
    }

    pub fn publish(&self, spec: &ReleaseSpec) {
        self.http
            .publish_release(&self.signer, spec, &self.scratch());
    }

    pub fn publish_and_point(&self, spec: &ReleaseSpec) {
        self.publish(spec);
        self.http.point_channel(&self.signer, "main", &spec.version);
    }

    pub fn engine_as<'a>(&'a self, version: &str, sha: &str) -> Engine<'a> {
        Engine {
            cfg: &self.cfg,
            key: self.signer.public_key(),
            updater_version: semver::Version::parse("0.9.0").unwrap(),
            current: Installed {
                version: version.into(),
                git_sha: sha.into(),
            },
            layout: Layout::from_config(&self.cfg),
            http: &self.http,
            restarter: &self.world,
            health: &self.world,
            host: &self.world,
        }
    }

    pub fn engine(&self) -> Engine<'_> {
        self.engine_as("0.9.0", OLD_SHA)
    }

    pub fn installed_binary(&self) -> String {
        std::fs::read_to_string(&self.cfg.bin_path).unwrap()
    }

    pub fn web_index(&self) -> String {
        std::fs::read_to_string(self.cfg.web_dist.join("index.html")).unwrap()
    }
}
