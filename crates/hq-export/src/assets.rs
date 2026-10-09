//! Safe loading of the images a note refers to.
//!
//! Every writer that embeds pictures goes through [`AssetLoader`], so the rules
//! live in one place: only files below the asset root, only known image types,
//! a size cap, and never anything remote or inline.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "svg", "webp"];
const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct Asset {
    /// Generated, collision-free file name such as `asset0.png`.
    pub name: String,
    pub ext: String,
    pub bytes: Vec<u8>,
}

impl Asset {
    pub fn mime(&self) -> &'static str {
        match self.ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "svg" => "image/svg+xml",
            "webp" => "image/webp",
            _ => "application/octet-stream",
        }
    }
}

pub(crate) struct AssetLoader {
    root: Option<PathBuf>,
    assets: Vec<Asset>,
    by_path: HashMap<PathBuf, usize>,
}

impl AssetLoader {
    /// With `root` of `None` no image is ever read.
    pub fn new(root: Option<&Path>) -> Self {
        AssetLoader {
            root: root.and_then(|r| r.canonicalize().ok()),
            assets: Vec::new(),
            by_path: HashMap::new(),
        }
    }

    /// Load the image `src` names, or `None` when it must not be embedded.
    pub fn load(&mut self, src: &str) -> Option<&Asset> {
        let index = self.load_index(src)?;
        self.assets.get(index)
    }

    fn load_index(&mut self, src: &str) -> Option<usize> {
        if src.contains("://") || src.starts_with("data:") {
            return None;
        }
        let root = self.root.as_ref()?;
        let path = Path::new(src);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let path = path.canonicalize().ok()?;
        if !path.starts_with(root) {
            return None;
        }
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        if !IMAGE_EXTS.contains(&ext.as_str()) {
            return None;
        }
        if let Some(&i) = self.by_path.get(&path) {
            return Some(i);
        }
        if std::fs::metadata(&path).ok()?.len() > MAX_IMAGE_BYTES {
            return None;
        }
        let bytes = std::fs::read(&path).ok()?;
        let index = self.assets.len();
        self.assets.push(Asset {
            name: format!("asset{index}.{ext}"),
            ext,
            bytes,
        });
        self.by_path.insert(path, index);
        Some(index)
    }

    pub fn into_assets(self) -> Vec<Asset> {
        self.assets
    }
}
