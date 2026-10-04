//! Skill types, bundle lookup, SKILL.md parsing, and listing.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::skill_bundle::{SkillBundle, load_skill_bundles};

/// Parsed skill definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDefinition {
    pub name: String,
    pub description: String,
    pub auto_load: bool,
    /// When true, inject full SKILL.md content instead of SUMMARY.md on auto-load.
    pub load_full: bool,
    /// Keywords that trigger contextual loading when found in a task/instruction.
    pub hints: Vec<String>,
    /// Skills to load after this one completes (chaining cues).
    pub next_skills: Vec<String>,
    /// Hidden from the catalog but still loadable by name and by bundles.
    /// Lets a family of related skills be reachable as one bundle entry
    /// without each member also costing a catalog line in every prompt.
    #[serde(default)]
    pub bundle_only: bool,
    /// Executables the skill shells out to, from `metadata.requires.bins` or `requires.bins`.
    #[serde(default)]
    pub requires_bins: Vec<String>,
    /// Vault context the skill wants retrieved at task time (a `context:` frontmatter block).
    /// Retrieval guidance only: it never holds note text.
    #[serde(default)]
    pub context_need: Option<hq_memory::context_packet::ContextNeed>,
    /// How this skill came to exist (user-written vs. auto-minted by genesis).
    pub provenance: SkillProvenance,
    pub content: String,
}

/// Where a skill file came from and how it has evolved.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillProvenance {
    /// "user" (manually written) or "genesis" (auto-minted from a successful run).
    pub minted_by: String,
    /// Identifier of the run (AIDC run id, coordinator run id, etc.) that triggered minting.
    pub minted_from_run_id: Option<String>,
    /// Monotonically incremented each time a refiner rewrites the skill body.
    pub version: u32,
}

/// Compact metadata returned by list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub auto_load: bool,
    pub load_full: bool,
    pub hints: Vec<String>,
    pub next_skills: Vec<String>,
    #[serde(default)]
    pub bundle_only: bool,
}

/// Cap for the SKILL.md fallback when an autoLoad skill has no SUMMARY.md.
pub(super) const SUMMARY_FALLBACK_CHARS: usize = 1200;

// ─── Parsing ────────────────────────────────────────────────────

pub(super) fn bundles_dir(skills_dir: &Path) -> PathBuf {
    skills_dir.join("skill-bundles")
}

fn find_bundle_path(skills_dir: &Path, name: &str) -> Option<PathBuf> {
    let dir = bundles_dir(skills_dir);
    let yaml = crate::skill_bundle::bundle_path(&dir, name);
    if yaml.is_file() {
        return Some(yaml);
    }
    let yml = dir.join(format!("{name}.yml"));
    if yml.is_file() {
        return Some(yml);
    }
    None
}

pub(super) fn is_bundle_skill(skills_dir: &Path, name: &str) -> bool {
    find_bundle_path(skills_dir, name).is_some()
}

/// Load a skill bundle as a virtual `SkillDefinition`.
///
/// Concatenates the bundle's optional instruction with each member skill's
/// content. Missing skills are replaced by a placeholder note. Nested bundles
/// are skipped to avoid cycles.
fn load_bundle_skill(skills_dir: &Path, name: &str) -> Option<SkillDefinition> {
    let path = find_bundle_path(skills_dir, name)?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let bundle: SkillBundle = serde_yaml::from_str(&raw).ok()?;

    let mut parts: Vec<String> = Vec::new();
    if let Some(instruction) = &bundle.instruction {
        parts.push(format!("# Bundle Instructions\n{instruction}"));
    }

    for member in &bundle.skills {
        if is_bundle_skill(skills_dir, member) {
            // Do not recurse into nested bundles.
            continue;
        }

        match parse_skill(skills_dir, member) {
            Some(skill) => parts.push(skill.content),
            None => parts.push(format!("Missing skill: `{member}`")),
        }
    }

    let content = parts.join("\n\n---\n\n");

    Some(SkillDefinition {
        name: name.to_string(),
        description: bundle.description,
        auto_load: false,
        load_full: true,
        hints: Vec::new(),
        next_skills: Vec::new(),
        bundle_only: false,
        requires_bins: Vec::new(),
        context_need: None,
        provenance: SkillProvenance {
            // Bundles are authored as YAML by hand; they are not minted.
            minted_by: "user".to_string(),
            minted_from_run_id: None,
            version: 1,
        },
        content,
    })
}

/// Parse a single skill from `<skills_dir>/<name>/SKILL.md`.
/// Name is sanitized to prevent path traversal.
pub fn parse_skill(skills_dir: &Path, name: &str) -> Option<SkillDefinition> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return None;
    }

    if let Some(bundle) = load_bundle_skill(skills_dir, name) {
        return Some(bundle);
    }

    let skill_path = skills_dir.join(name).join("SKILL.md");
    let raw = std::fs::read_to_string(&skill_path).ok()?;

    let matter = gray_matter::Matter::<gray_matter::engine::YAML>::new();
    let result = matter.parse(&raw);

    let description = result
        .data
        .as_ref()
        .and_then(|d| match d {
            gray_matter::Pod::Hash(map) => map.get("description").and_then(|v| match v {
                gray_matter::Pod::String(s) => Some(s.clone()),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_else(|| format!("Skill: {name}"));

    let auto_load = result
        .data
        .as_ref()
        .and_then(|d| match d {
            gray_matter::Pod::Hash(map) => map.get("autoLoad").and_then(|v| match v {
                gray_matter::Pod::Boolean(b) => Some(*b),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or(false);

    let load_full = result
        .data
        .as_ref()
        .and_then(|d| match d {
            gray_matter::Pod::Hash(map) => map.get("loadFull").and_then(|v| match v {
                gray_matter::Pod::Boolean(b) => Some(*b),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or(false);

    let bundle_only = result
        .data
        .as_ref()
        .and_then(|d| match d {
            gray_matter::Pod::Hash(map) => map.get("bundleOnly").and_then(|v| match v {
                gray_matter::Pod::Boolean(b) => Some(*b),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or(false);

    let hints = result
        .data
        .as_ref()
        .and_then(|d| match d {
            gray_matter::Pod::Hash(map) => map.get("hints").and_then(|v| match v {
                gray_matter::Pod::Array(arr) => Some(
                    arr.iter()
                        .filter_map(|item| match item {
                            gray_matter::Pod::String(s) => Some(s.to_lowercase()),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_default();

    let next_skills = result
        .data
        .as_ref()
        .and_then(|d| match d {
            gray_matter::Pod::Hash(map) => map.get("nextSkills").and_then(|v| match v {
                gray_matter::Pod::Array(arr) => Some(
                    arr.iter()
                        .filter_map(|item| match item {
                            gray_matter::Pod::String(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_default();

    let provenance = result
        .data
        .as_ref()
        .and_then(|d| match d {
            gray_matter::Pod::Hash(map) => map.get("provenance").and_then(|v| match v {
                gray_matter::Pod::Hash(pmap) => {
                    let minted_by = pmap.get("mintedBy").and_then(|vv| match vv {
                        gray_matter::Pod::String(s) => Some(s.clone()),
                        _ => None,
                    });
                    let minted_from_run_id = pmap.get("mintedFromRunId").and_then(|vv| match vv {
                        gray_matter::Pod::String(s) => Some(s.clone()),
                        _ => None,
                    });
                    let version = pmap.get("version").and_then(|vv| match vv {
                        gray_matter::Pod::Integer(i) => Some(*i as u32),
                        _ => None,
                    });
                    Some(SkillProvenance {
                        minted_by: minted_by.unwrap_or_else(|| "user".to_string()),
                        minted_from_run_id,
                        version: version.unwrap_or(1),
                    })
                }
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_else(|| SkillProvenance {
            minted_by: "user".to_string(),
            minted_from_run_id: None,
            version: 1,
        });

    Some(SkillDefinition {
        name: name.to_string(),
        description,
        auto_load,
        load_full,
        hints,
        next_skills,
        bundle_only,
        requires_bins: result.data.as_ref().map(required_bins).unwrap_or_default(),
        context_need: parse_context_need(&raw),
        provenance,
        content: result.content,
    })
}

#[derive(Deserialize)]
struct ContextFrontmatter {
    context: Option<hq_memory::context_packet::ContextNeed>,
}

/// A malformed `context:` block means no declaration, never a failed skill load.
fn parse_context_need(raw: &str) -> Option<hq_memory::context_packet::ContextNeed> {
    let matter = gray_matter::Matter::<gray_matter::engine::YAML>::new();
    let parsed = matter.parse_with_struct::<ContextFrontmatter>(raw)?;
    parsed.data.context.filter(|need| !need.is_empty())
}

/// Published skills nest the list under `metadata` or `metadata.<vendor>`; older ones put it at the top.
fn required_bins(data: &gray_matter::Pod) -> Vec<String> {
    fn at<'a>(pod: &'a gray_matter::Pod, path: &[&str]) -> Option<&'a gray_matter::Pod> {
        path.iter().try_fold(pod, |pod, key| match pod {
            gray_matter::Pod::Hash(map) => map.get(*key),
            _ => None,
        })
    }
    let vendor_nested = || match at(data, &["metadata"]) {
        Some(gray_matter::Pod::Hash(vendors)) => {
            vendors.values().find_map(|v| at(v, &["requires", "bins"]))
        }
        _ => None,
    };
    let Some(gray_matter::Pod::Array(bins)) = at(data, &["metadata", "requires", "bins"])
        .or_else(vendor_nested)
        .or_else(|| at(data, &["requires", "bins"]))
    else {
        return Vec::new();
    };
    bins.iter()
        .filter_map(|b| match b {
            gray_matter::Pod::String(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// List all skills in the skills directory.
pub fn list_skills(skills_dir: &Path) -> Vec<SkillMeta> {
    let Ok(entries) = std::fs::read_dir(skills_dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            // Proposed (auto-minted, pending user approval) skills are hidden from
            // the active catalog — user must move them out of _proposed/ to apply.
            if name.starts_with('_') {
                continue;
            }
            if let Some(skill) = parse_skill(skills_dir, &name) {
                // Bundle members stay loadable by name but cost no catalog
                // line — the bundle is the single entry agents see.
                if skill.bundle_only {
                    continue;
                }
                out.push(SkillMeta {
                    name: skill.name,
                    description: skill.description,
                    auto_load: skill.auto_load,
                    load_full: skill.load_full,
                    hints: skill.hints,
                    next_skills: skill.next_skills,
                    bundle_only: false,
                });
            }
        }
    }

    let bundles_dir = bundles_dir(skills_dir);
    for bundle in load_skill_bundles(&bundles_dir) {
        out.push(SkillMeta {
            name: bundle.name,
            description: bundle.description,
            auto_load: false,
            load_full: true,
            hints: Vec::new(),
            next_skills: Vec::new(),
            bundle_only: false,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}
