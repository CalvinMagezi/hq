//! Per-brand asset resolution — logo/palette/font/reference-doc lookup so
//! generated documents never mix branding across clients.
//!
//! Deliberately independent of `hq-core::config::company::CompanyConfig`:
//! that registry exists for email/budget routing under
//! `Notebooks/Companies/<id>`, an unrelated tree whose entries need not match
//! the brands here. Branding reads straight from the vault convention
//! instead, so a brand doesn't need a company registered to get styled.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::types::ConvertError;

/// A brand discovered in the vault: its slug (used in tool calls and
/// `brand.yaml`) paired with its `Notebooks/Projects/<dir_name>` directory.
#[derive(Debug, Clone)]
pub struct BrandProject {
    pub slug: String,
    pub dir_name: String,
}

#[derive(Deserialize)]
struct BrandSlugOnly {
    brand: String,
}

/// Discover every brand registered in the vault: any
/// `Notebooks/Projects/<X>/Branding/` directory. The slug comes from
/// `brand.yaml`'s `brand` field when a manifest already exists, else the
/// project directory name lowercased. Brands are vault content, not a
/// compiled-in list — registering a new one needs only a new vault folder,
/// no code change or rebuild.
pub fn discover_brands(vault_path: &Path) -> Vec<BrandProject> {
    let mut brands = Vec::new();
    let Ok(entries) = fs::read_dir(vault_path.join("Notebooks/Projects")) else {
        return brands;
    };
    for entry in entries.flatten() {
        let Ok(true) = entry.file_type().map(|t| t.is_dir()) else {
            continue;
        };
        let branding_dir = entry.path().join("Branding");
        if !branding_dir.is_dir() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().into_owned();
        let slug = fs::read_to_string(branding_dir.join("brand.yaml"))
            .ok()
            .and_then(|raw| serde_yaml::from_str::<BrandSlugOnly>(&raw).ok())
            .map(|b| b.brand)
            .unwrap_or_else(|| dir_name.to_lowercase());
        brands.push(BrandProject { slug, dir_name });
    }
    brands.sort_by(|a, b| a.slug.cmp(&b.slug));
    brands
}

fn unknown_brand_error(vault_path: &Path, brand_slug: &str) -> ConvertError {
    let known: Vec<String> = discover_brands(vault_path).into_iter().map(|b| b.slug).collect();
    let known_str = if known.is_empty() {
        "(none registered yet — add Notebooks/Projects/<Name>/Branding/)".to_string()
    } else {
        known.join(", ")
    };
    ConvertError::Other(format!(
        "unknown brand '{brand_slug}'; known brands: {known_str}"
    ))
}

#[derive(Debug, Clone, Deserialize)]
pub struct BrandKit {
    pub brand: String,
    pub primary_color: String,
    #[serde(default)]
    pub secondary_colors: Vec<String>,
    pub font: String,
    #[serde(default)]
    pub logo_light: Option<PathBuf>,
    #[serde(default)]
    pub logo_dark: Option<PathBuf>,
    #[serde(default)]
    pub reference_docx: Option<PathBuf>,
    #[serde(default)]
    pub reference_pptx: Option<PathBuf>,
    /// Points at `prose_lint` rather than restating its style rules.
    #[serde(default = "default_voice_notes")]
    pub voice_notes: String,
}

fn default_voice_notes() -> String {
    "No brand-specific voice override; pass drafts through prose_lint before delivery."
        .to_string()
}

/// `Notebooks/Projects/<Brand>/Branding` for a known brand slug (matched
/// case-insensitively against the vault's discovered brands).
pub fn branding_dir(vault_path: &Path, brand_slug: &str) -> Result<PathBuf, ConvertError> {
    discover_brands(vault_path)
        .into_iter()
        .find(|b| b.slug.eq_ignore_ascii_case(brand_slug))
        .map(|b| {
            vault_path
                .join("Notebooks/Projects")
                .join(b.dir_name)
                .join("Branding")
        })
        .ok_or_else(|| unknown_brand_error(vault_path, brand_slug))
}

fn resolve_relative(base: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

/// Read `Notebooks/Projects/<Brand>/Branding/brand.yaml`, resolving asset
/// paths relative to that `Branding/` directory so the manifest itself stays
/// portable.
pub fn load_brand_kit(vault_path: &Path, brand_slug: &str) -> Result<BrandKit, ConvertError> {
    let dir = branding_dir(vault_path, brand_slug)?;
    let manifest_path = dir.join("brand.yaml");
    let raw = std::fs::read_to_string(&manifest_path).map_err(|e| {
        ConvertError::Other(format!(
            "no brand.yaml for '{brand_slug}' at {}: {e}",
            manifest_path.display()
        ))
    })?;
    let mut kit: BrandKit = serde_yaml::from_str(&raw).map_err(|e| {
        ConvertError::Other(format!("invalid brand.yaml for '{brand_slug}': {e}"))
    })?;

    kit.logo_light = kit.logo_light.map(|p| resolve_relative(&dir, p));
    kit.logo_dark = kit.logo_dark.map(|p| resolve_relative(&dir, p));
    kit.reference_docx = kit.reference_docx.map(|p| resolve_relative(&dir, p));
    kit.reference_pptx = kit.reference_pptx.map(|p| resolve_relative(&dir, p));

    Ok(kit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_northwind_kit(vault: &Path) {
        let dir = vault.join("Notebooks/Projects/Northwind/Branding");
        std::fs::create_dir_all(dir.join("Assets")).unwrap();
        std::fs::write(dir.join("Assets/logo.png"), b"fake-png").unwrap();
        std::fs::write(
            dir.join("brand.yaml"),
            r##"
brand: northwind
primary_color: "#35b544"
secondary_colors: ["#e1f4e4", "#2a8a34"]
font: Inter
logo_light: Assets/logo.png
"##,
        )
        .unwrap();
    }

    #[test]
    fn loads_and_resolves_asset_paths_relative_to_branding_dir() {
        let vault = tempfile::tempdir().unwrap();
        write_northwind_kit(vault.path());

        let kit = load_brand_kit(vault.path(), "northwind").unwrap();
        assert_eq!(kit.primary_color, "#35b544");
        assert_eq!(kit.font, "Inter");
        assert_eq!(
            kit.logo_light.unwrap(),
            vault
                .path()
                .join("Notebooks/Projects/Northwind/Branding/Assets/logo.png")
        );
    }

    #[test]
    fn unknown_brand_is_rejected() {
        let vault = tempfile::tempdir().unwrap();
        let err = load_brand_kit(vault.path(), "acme").unwrap_err();
        assert!(err.to_string().contains("unknown brand"));
    }

    #[test]
    fn missing_manifest_is_a_clear_error_not_a_panic() {
        let vault = tempfile::tempdir().unwrap();
        // A brand is "known" once its Branding/ dir exists, even before it
        // has a brand.yaml manifest (see BrandAssetsAddTool, which creates
        // this dir on a brand's first uploaded asset).
        std::fs::create_dir_all(vault.path().join("Notebooks/Projects/ExampleProject/Branding")).unwrap();
        let err = load_brand_kit(vault.path(), "exampleproject").unwrap_err();
        assert!(err.to_string().contains("no brand.yaml"));
    }
}
