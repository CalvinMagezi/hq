//! `brand_assets_add` / `brand_assets_list` — the growing per-brand logo/icon
//! knowledge base. Lets an agent file an inbound asset (an email attachment,
//! a Telegram photo) straight into the right brand's `Branding/Assets/`
//! without the user doing it by hand, and lets a document-generation skill
//! discover what's already registered before inventing anything. Brands
//! themselves are vault content (`Notebooks/Projects/<Name>/Branding/`), not
//! a compiled-in list — see `hq_convert::brand::discover_brands`.

use std::path::{Component, Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use base64::Engine;
use serde_json::{Value, json};

use hq_convert::brand::{branding_dir, discover_brands, load_brand_kit};

use crate::registry::HqTool;

/// JSON Schema fragment for the `brand` parameter: an enum of the vault's
/// currently discovered brands when there are any, else a plain string (an
/// empty `enum` would reject every value). Built fresh per call since the
/// vault's registered brands can change without a rebuild.
pub(crate) fn brand_param_schema(vault_path: &Path) -> Value {
    let known: Vec<String> = discover_brands(vault_path).into_iter().map(|b| b.slug).collect();
    if known.is_empty() {
        json!({
            "type": "string",
            "description": "Brand slug. No brands registered yet — add Notebooks/Projects/<Name>/Branding/ with a brand.yaml."
        })
    } else {
        json!({
            "type": "string",
            "enum": known,
            "description": "Brand slug."
        })
    }
}

/// Rejects `..`, an absolute path, or a Windows drive/root prefix. Real
/// boundary, not defensive noise: this can be fed by inbound email/Telegram
/// attachments, so the filename is untrusted input.
fn safe_relative_filename(filename: &str) -> Result<PathBuf> {
    if filename.trim().is_empty() {
        bail!("filename must not be empty");
    }
    let path = Path::new(filename);
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            other => bail!("filename '{filename}' has an unsafe path component: {other:?}"),
        }
    }
    Ok(path.to_path_buf())
}

fn known_brand_or_err(vault_path: &Path, brand: &str) -> Result<()> {
    let known = discover_brands(vault_path);
    if known.iter().any(|b| b.slug.eq_ignore_ascii_case(brand)) {
        return Ok(());
    }
    let names: Vec<&str> = known.iter().map(|b| b.slug.as_str()).collect();
    bail!(
        "unknown brand '{brand}'; known brands: {}",
        if names.is_empty() {
            "(none registered yet — add Notebooks/Projects/<Name>/Branding/)".to_string()
        } else {
            names.join(", ")
        }
    )
}

pub struct BrandAssetsListTool {
    vault_path: PathBuf,
}

#[async_trait]
impl HqTool for BrandAssetsListTool {
    fn name(&self) -> &str {
        "brand_assets_list"
    }

    fn description(&self) -> &str {
        "List a brand's registered assets (logos/icons) and manifest (palette, font, \
         reference-doc paths) from Notebooks/Projects/<Brand>/Branding/. Call this before \
         generating any docx/pptx/xlsx/pdf so brand colors/logos are never invented."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["brand"],
            "properties": {
                "brand": brand_param_schema(&self.vault_path)
            }
        })
    }

    fn category(&self) -> &str {
        "brand"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let brand = args
            .get("brand")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing required arg: brand"))?;
        known_brand_or_err(&self.vault_path, brand)?;

        let dir = branding_dir(&self.vault_path, brand)?;
        let manifest = load_brand_kit(&self.vault_path, brand).ok().map(|kit| {
            json!({
                "primary_color": kit.primary_color,
                "secondary_colors": kit.secondary_colors,
                "font": kit.font,
                "logo_light": kit.logo_light,
                "logo_dark": kit.logo_dark,
                "reference_docx": kit.reference_docx,
                "reference_pptx": kit.reference_pptx,
                "voice_notes": kit.voice_notes,
            })
        });

        let assets_dir = dir.join("Assets");
        let mut assets = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&assets_dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    assets.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
        assets.sort();

        Ok(json!({
            "brand": brand,
            "branding_dir": dir.to_string_lossy(),
            "manifest": manifest,
            "assets": assets,
        }))
    }
}

pub struct BrandAssetsAddTool {
    vault_path: PathBuf,
}

#[async_trait]
impl HqTool for BrandAssetsAddTool {
    fn name(&self) -> &str {
        "brand_assets_add"
    }

    fn description(&self) -> &str {
        "Save a logo/icon into a brand's Assets/ folder (Notebooks/Projects/<Brand>/Branding/Assets/), \
         creating the folder if this is the brand's first asset. Use when a logo arrives via email, \
         Telegram, or any other inbound source — this is what grows the brand knowledge base over \
         time instead of re-asking for the same asset."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["brand", "filename", "content_base64"],
            "properties": {
                "brand": brand_param_schema(&self.vault_path),
                "filename": {
                    "type": "string",
                    "description": "Filename to save as under Assets/ (e.g. 'logo-dark.svg'). No '..' or absolute paths."
                },
                "content_base64": {
                    "type": "string",
                    "description": "Base64-encoded file content."
                },
                "kind": {
                    "type": "string",
                    "enum": ["logo", "icon", "other"],
                    "default": "other",
                    "description": "What kind of asset this is, for the caller's own record-keeping."
                }
            }
        })
    }

    fn category(&self) -> &str {
        "brand"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let brand = args
            .get("brand")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing required arg: brand"))?;
        known_brand_or_err(&self.vault_path, brand)?;

        let filename = args
            .get("filename")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing required arg: filename"))?;
        let relative = safe_relative_filename(filename)?;

        let content_b64 = args
            .get("content_base64")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing required arg: content_base64"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(content_b64)
            .map_err(|e| anyhow!("invalid base64 in content_base64: {e}"))?;

        let kind = args.get("kind").and_then(|v| v.as_str()).unwrap_or("other");

        let dir = branding_dir(&self.vault_path, brand)?;
        let assets_dir = dir.join("Assets");
        let dest = assets_dir.join(&relative);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, &bytes)?;

        Ok(json!({
            "brand": brand,
            "kind": kind,
            "path": dest.to_string_lossy(),
            "size_bytes": bytes.len(),
        }))
    }
}

/// Create the brand-asset tools.
pub fn create_brand_tools(vault_path: PathBuf) -> Vec<Box<dyn HqTool>> {
    vec![
        Box::new(BrandAssetsListTool {
            vault_path: vault_path.clone(),
        }),
        Box::new(BrandAssetsAddTool { vault_path }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_dir_traversal() {
        assert!(safe_relative_filename("../etc/passwd").is_err());
    }

    #[test]
    fn rejects_absolute_path() {
        assert!(safe_relative_filename("/etc/passwd").is_err());
    }

    #[test]
    fn rejects_empty_filename() {
        assert!(safe_relative_filename("").is_err());
    }

    #[test]
    fn accepts_a_nested_relative_filename() {
        let p = safe_relative_filename("flags/country.svg").unwrap();
        assert_eq!(p, PathBuf::from("flags/country.svg"));
    }

    #[tokio::test]
    async fn add_then_list_round_trips() {
        let vault = tempfile::tempdir().unwrap();
        // A brand is "known" once its Branding/ dir exists in the vault.
        std::fs::create_dir_all(vault.path().join("Notebooks/Projects/ExampleProject/Branding")).unwrap();
        let add = BrandAssetsAddTool {
            vault_path: vault.path().to_path_buf(),
        };
        let content = base64::engine::general_purpose::STANDARD.encode(b"fake-svg");
        add.execute(json!({
            "brand": "exampleproject",
            "filename": "logo.svg",
            "content_base64": content,
        }))
        .await
        .unwrap();

        let list = BrandAssetsListTool {
            vault_path: vault.path().to_path_buf(),
        };
        let out = list.execute(json!({ "brand": "exampleproject" })).await.unwrap();
        let assets = out["assets"].as_array().unwrap();
        assert!(assets.iter().any(|a| a == "logo.svg"));
    }

    #[tokio::test]
    async fn add_rejects_unknown_brand() {
        let vault = tempfile::tempdir().unwrap();
        let add = BrandAssetsAddTool {
            vault_path: vault.path().to_path_buf(),
        };
        let err = add
            .execute(json!({ "brand": "acme", "filename": "x.png", "content_base64": "eA==" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown brand"));
    }
}
