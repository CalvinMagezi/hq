//! `hq vault inventory` and `hq vault import`: the terminal side of the import pipeline.
//!
//! Both run the same code as the `vault_inventory` and `vault_import` tools
//! (`hq_tools::vault::Importer`), so a note made here is the same note an agent would make.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use hq_core::config::HqConfig;
use hq_tools::provenance::Writer;
use hq_tools::vault::{Caller, Conflict, ImportSettings, Importer, InventoryFilter, Source};
use hq_vault::VaultClient;
use serde_json::Value;

const INVENTORY_USAGE: &str = "Usage: hq vault inventory [folder] [--status pending,changed,imported,blocked] [--format pdf,docx,...] [--limit N] [--json]";
const IMPORT_USAGE: &str = "Usage: hq vault import <file>... | <folder> | --all [--apply] [--dry-run] [--destination <folder>] [--on-conflict skip|suffix|overwrite] [--archive] [--tag <tag>]... [--limit N] [--preview N] [--json]\n\
     A folder (or --all) is a dry run until you add --apply. Files named one by one are imported straight away.";

/// What the command line asked for, before it meets the vault.
#[derive(Debug, Default, PartialEq)]
struct ImportArgs {
    targets: Vec<String>,
    all: bool,
    apply: bool,
    dry_run: bool,
    destination: Option<String>,
    on_conflict: Option<String>,
    archive: bool,
    tags: Vec<String>,
    limit: Option<usize>,
    preview: Option<usize>,
    json: bool,
}

fn value_of<'a>(
    it: &mut std::slice::Iter<'a, String>,
    flag: &str,
    usage: &str,
) -> Result<&'a String> {
    it.next()
        .ok_or_else(|| anyhow!("{flag} needs a value\n{usage}"))
}

fn number(raw: &str, flag: &str) -> Result<usize> {
    raw.parse()
        .map_err(|_| anyhow!("{flag} needs a whole number, not '{raw}'"))
}

fn parse_import(args: &[String]) -> Result<ImportArgs> {
    let mut out = ImportArgs::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--all" => out.all = true,
            "--apply" | "--yes" => out.apply = true,
            "--dry-run" | "-n" => out.dry_run = true,
            "--archive" => out.archive = true,
            "--json" => out.json = true,
            "--destination" | "--dest" => {
                out.destination = Some(value_of(&mut it, a, IMPORT_USAGE)?.clone())
            }
            "--on-conflict" => out.on_conflict = Some(value_of(&mut it, a, IMPORT_USAGE)?.clone()),
            "--tag" => out.tags.push(value_of(&mut it, a, IMPORT_USAGE)?.clone()),
            "--limit" => out.limit = Some(number(value_of(&mut it, a, IMPORT_USAGE)?, a)?),
            "--preview" => out.preview = Some(number(value_of(&mut it, a, IMPORT_USAGE)?, a)?),
            other if other.starts_with('-') => bail!("unknown option {other}\n{IMPORT_USAGE}"),
            other => out.targets.push(other.to_string()),
        }
    }
    Ok(out)
}

/// A path as the vault names it: relative, with no `./`. An absolute path must be inside the vault.
fn vault_relative(vault: &Path, given: &str) -> Result<String> {
    let given = given.trim();
    let path = Path::new(given);
    if path.is_absolute() {
        let root = std::fs::canonicalize(vault)?;
        let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        return match real.strip_prefix(&root) {
            Ok(rel) => Ok(rel.to_string_lossy().into_owned()),
            Err(_) => bail!(
                "{given} is outside the vault; copy it into the vault first (for example into Notebooks/ or _media/), then import it"
            ),
        };
    }
    Ok(given
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string())
}

/// `hq vault inventory`: files that are not notes, and what can be done with each.
pub async fn inventory(
    config: &HqConfig,
    args: &[String],
    deny: &dyn Fn(&str) -> Result<()>,
) -> Result<()> {
    let mut folder: Option<String> = None;
    let mut statuses: Vec<String> = Vec::new();
    let mut kinds: Vec<String> = Vec::new();
    let mut limit = 100usize;
    let mut json = false;
    let split = |raw: &str| -> Vec<String> {
        raw.split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => json = true,
            "--status" => statuses.extend(split(value_of(&mut it, a, INVENTORY_USAGE)?)),
            "--format" => kinds.extend(split(value_of(&mut it, a, INVENTORY_USAGE)?)),
            "--limit" => limit = number(value_of(&mut it, a, INVENTORY_USAGE)?, a)?,
            other if other.starts_with('-') => bail!("unknown option {other}\n{INVENTORY_USAGE}"),
            other if folder.is_none() => folder = Some(other.to_string()),
            _ => bail!(INVENTORY_USAGE),
        }
    }
    let folder = lite_start(config, folder.as_deref(), deny)?;
    let importer = importer(config)?;
    let result = importer.inventory(
        &folder,
        InventoryFilter {
            statuses: Some(statuses).filter(|s| !s.is_empty()),
            kinds: Some(kinds).filter(|k| !k.is_empty()),
            limit: limit.clamp(1, 5000),
        },
    )?;
    if json {
        println!("{result}");
    } else {
        print!("{}", inventory_text(&result));
    }
    Ok(())
}

/// `hq vault import`: convert files in the vault into notes.
pub async fn import(
    config: &HqConfig,
    args: &[String],
    deny: &dyn Fn(&str) -> Result<()>,
) -> Result<()> {
    let a = parse_import(args)?;
    let on_conflict = Conflict::parse(a.on_conflict.as_deref().unwrap_or("suffix"))?;
    if let Some(d) = &a.destination {
        deny(d)?;
    }
    let vault = config.vault_path.as_path();

    let (source, is_folder) = match (a.all, a.targets.as_slice()) {
        (true, []) => (Source::Directory(lite_start(config, None, deny)?), true),
        (true, _) => bail!("give files or a folder, or --all, not both\n{IMPORT_USAGE}"),
        (false, []) => bail!("{IMPORT_USAGE}"),
        (false, [one]) => {
            let rel = vault_relative(vault, one)?;
            deny(&rel)?;
            if vault.join(&rel).is_dir() {
                (
                    Source::Directory(lite_start(config, Some(&rel), deny)?),
                    true,
                )
            } else {
                (Source::Path(rel), false)
            }
        }
        (false, many) => {
            let mut rels = Vec::new();
            for m in many {
                let rel = vault_relative(vault, m)?;
                deny(&rel)?;
                if vault.join(&rel).is_dir() {
                    bail!("give one folder or any number of files, not a mix ({rel} is a folder)");
                }
                rels.push(rel);
            }
            (Source::Paths(rels), false)
        }
    };

    // A folder is previewed unless --apply; named files are imported unless --dry-run.
    let dry_run = a.dry_run || (is_folder && !a.apply);
    let settings = ImportSettings {
        dry_run,
        on_conflict,
        destination: a.destination.clone(),
        tags: a.tags.clone(),
        archive_originals: a.archive,
        preview_chars: a.preview.unwrap_or(0).min(5000),
    };
    // A person at a terminal can take a bigger batch than an agent in one turn.
    let limit = a.limit.unwrap_or(200);
    let result = importer(config)?
        .run(source, &settings, limit, &Caller::Known(Writer::owner()))
        .await?;
    if a.json {
        println!("{result}");
    } else {
        print!(
            "{}",
            import_text(&result, is_folder && dry_run && !a.dry_run)
        );
    }
    let failed = result["failed"].as_u64().unwrap_or(0);
    if failed > 0 {
        bail!("{failed} file(s) could not be imported");
    }
    Ok(())
}

/// Under HQ Lite a folder search starts in `Notebooks`, never the vault root, so HQ's own folders
/// and `_media` stay out of view; an explicit folder is checked like any other path.
fn lite_start(
    config: &HqConfig,
    folder: Option<&str>,
    deny: &dyn Fn(&str) -> Result<()>,
) -> Result<String> {
    let rel = match folder {
        Some(f) => vault_relative(&config.vault_path, f)?,
        None => String::new(),
    };
    if config.profile.is_lite() {
        if rel.is_empty() {
            return Ok("Notebooks".to_string());
        }
        deny(&rel)?;
    }
    Ok(rel)
}

/// The importer over this vault, with the database when there is one (so new notes are indexed
/// and their provenance is recorded) and without it when HQ has not run here yet.
fn importer(config: &HqConfig) -> Result<Importer> {
    let vault = Arc::new(VaultClient::new(config.vault_path.clone())?);
    let db = hq_db::Database::open_existing(&config.db_path())
        .ok()
        .map(Arc::new);
    Ok(Importer::new(vault, db))
}

// ------------------------------------------------------------------ printing

fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / 1_048_576.0),
        b if b >= 1 << 10 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}

fn counts(v: &Value) -> String {
    v.as_object()
        .map(|o| {
            o.iter()
                .map(|(k, n)| format!("{k} {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

fn yes(v: &Value) -> &'static str {
    if v.as_bool().unwrap_or(false) {
        "yes"
    } else {
        "no"
    }
}

fn inventory_text(inv: &Value) -> String {
    let folder = match inv["folder"].as_str() {
        Some(f) if !f.starts_with('(') => format!("under {f}"),
        _ => "in the whole vault".to_string(),
    };
    let mut out = format!(
        "Files that are not notes {folder}: {} found ({})\n",
        inv["found"],
        size(inv["total_bytes"].as_u64().unwrap_or(0)),
    );
    if inv["found"].as_u64() == Some(0) {
        out.push_str("  Nothing to import here.\n");
        return out;
    }
    out.push_str(&format!("  status:  {}\n", counts(&inv["by_status"])));
    out.push_str(&format!("  formats: {}\n", counts(&inv["by_format"])));
    let can = &inv["can_do_here"];
    out.push_str(&format!(
        "  this host: OCR {}, PDF tables {}, old .doc/.ppt {}\n",
        yes(&can["ocr"]),
        yes(&can["pdf_tables"]),
        yes(&can["legacy_office"]),
    ));
    if inv["search_cut_short"] == true {
        out.push_str("  (the search stopped early: the vault is very large; name a folder)\n");
    }
    let entries = inv["entries"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if !entries.is_empty() {
        out.push('\n');
    }
    for e in entries {
        let note = e["note"].as_str().filter(|n| !n.is_empty());
        out.push_str(&format!(
            "  {:<9} {:<6} {:>8}  {}{}\n",
            e["status"].as_str().unwrap_or("").to_ascii_uppercase(),
            e["format"].as_str().unwrap_or(""),
            size(e["bytes"].as_u64().unwrap_or(0)),
            e["path"].as_str().unwrap_or(""),
            note.map(|n| format!("  ({n})")).unwrap_or_default(),
        ));
    }
    let (shown, matching) = (
        inv["files"].as_u64().unwrap_or(0),
        inv["matching"].as_u64().unwrap_or(0),
    );
    if matching > shown {
        out.push_str(&format!(
            "  ... {} more; raise --limit to see them\n",
            matching - shown
        ));
    }
    let waiting = |k: &str| inv["by_status"][k].as_u64().unwrap_or(0);
    if waiting("pending") + waiting("changed") > 0 {
        out.push_str(
            "\nNext: hq vault import <folder>   (a dry run), then add --apply to write the notes.\n",
        );
    }
    out
}

fn import_text(r: &Value, offer_apply: bool) -> String {
    let dry = r["dry_run"] == true;
    let mut out = String::new();
    for item in r["results"].as_array().into_iter().flatten() {
        let source = item["source"].as_str().unwrap_or("");
        let action = item["action"].as_str().unwrap_or("");
        let dest = item["destination"].as_str().unwrap_or("");
        match action {
            "failed" => out.push_str(&format!(
                "  failed      {source}: {}\n",
                item["reason"].as_str().unwrap_or("")
            )),
            "skipped" => out.push_str(&format!(
                "  skipped     {source} ({})\n",
                item["reason"].as_str().unwrap_or("")
            )),
            _ => {
                out.push_str(&format!(
                    "  {:<11} {source} -> {dest}  [{}, {}]\n",
                    action.replace('_', " "),
                    item["engine"].as_str().unwrap_or(""),
                    item["outcome"].as_str().unwrap_or(""),
                ));
                if let Some(to) = item["archived_to"].as_str() {
                    out.push_str(&format!("              original moved to {to}\n"));
                }
            }
        }
        if action != "failed" && action != "skipped" {
            for w in item["warnings"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                out.push_str(&format!("              - {w}\n"));
            }
            if let Some(p) = item["preview"].as_str() {
                for line in p.lines() {
                    out.push_str(&format!("              | {line}\n"));
                }
            }
        }
    }
    let n = |k: &str| r[k].as_u64().unwrap_or(0);
    if n("requested") == 0 {
        return "Nothing pending: every file here is already imported.\n".to_string();
    }
    let wrote = if dry {
        n("would_create")
    } else {
        n("created") + n("updated")
    };
    out.push_str(&format!(
        "\n{}: {wrote} note(s), skipped {}, failed {}, with something left out {}\n",
        if dry { "Dry run, would write" } else { "Wrote" },
        n("skipped"),
        n("failed"),
        n("degraded"),
    ));
    if n("more_pending") > 0 {
        out.push_str(&format!(
            "{} more pending; run the same command again for the next batch (or raise --limit).\n",
            n("more_pending")
        ));
    }
    if offer_apply && wrote > 0 {
        out.push_str("Nothing was written. Add --apply to write these notes.\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::Profile;

    const SHEET: &[u8] = include_bytes!("../../../hq-convert/tests/fixtures/corpus/tracker.xlsx");
    const LETTER: &[u8] = include_bytes!("../../../hq-convert/tests/fixtures/corpus/invoice.docx");

    fn vault(profile: Profile) -> (tempfile::TempDir, HqConfig) {
        let dir = tempfile::tempdir().unwrap();
        for (rel, bytes) in [
            ("Notebooks/Docs/tracker.xlsx", SHEET),
            ("Notebooks/Docs/invoice.docx", LETTER),
            ("_media/2026-01-02/scan.docx", LETTER),
            ("_system/secret.docx", LETTER),
        ] {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        }
        let cfg = HqConfig {
            vault_path: dir.path().to_path_buf(),
            profile,
            ..HqConfig::default()
        };
        (dir, cfg)
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn allow(_: &str) -> Result<()> {
        Ok(())
    }

    #[test]
    fn options_parse_and_unknown_ones_are_refused() {
        let a = parse_import(&args(&[
            "Notebooks/Docs",
            "--apply",
            "--archive",
            "--tag",
            "q3",
            "--tag",
            "ops",
            "--limit",
            "5",
            "--on-conflict",
            "skip",
            "--destination",
            "Notebooks/In",
            "--json",
        ]))
        .unwrap();
        assert_eq!(a.targets, ["Notebooks/Docs"]);
        assert_eq!(a.tags, ["q3", "ops"]);
        assert_eq!(
            (a.apply, a.archive, a.json, a.limit),
            (true, true, true, Some(5))
        );
        assert_eq!(a.on_conflict.as_deref(), Some("skip"));
        assert!(parse_import(&args(&["--wat"])).is_err());
        assert!(parse_import(&args(&["--limit"])).is_err());
        assert!(parse_import(&args(&["--limit", "many"])).is_err());
    }

    #[test]
    fn absolute_paths_must_be_inside_the_vault() {
        let (dir, _) = vault(Profile::Full);
        let inside = dir.path().join("Notebooks/Docs/invoice.docx");
        assert_eq!(
            vault_relative(dir.path(), &inside.to_string_lossy()).unwrap(),
            "Notebooks/Docs/invoice.docx"
        );
        assert_eq!(
            vault_relative(dir.path(), "./Notebooks/Docs/").unwrap(),
            "Notebooks/Docs"
        );
        let err = vault_relative(dir.path(), "/etc/hosts")
            .unwrap_err()
            .to_string();
        assert!(err.contains("outside the vault"), "{err}");
    }

    #[tokio::test]
    async fn a_folder_is_previewed_first_and_written_only_with_apply() {
        let (dir, cfg) = vault(Profile::Full);
        import(&cfg, &args(&["Notebooks/Docs"]), &allow)
            .await
            .unwrap();
        assert!(
            !dir.path().join("Notebooks/Docs/invoice.md").exists(),
            "a folder is a dry run"
        );

        import(&cfg, &args(&["Notebooks/Docs", "--apply"]), &allow)
            .await
            .unwrap();
        assert!(dir.path().join("Notebooks/Docs/invoice.md").is_file());
        assert!(dir.path().join("Notebooks/Docs/tracker.md").is_file());
        let body = std::fs::read_to_string(dir.path().join("Notebooks/Docs/invoice.md")).unwrap();
        assert!(
            body.contains("source: Notebooks/Docs/invoice.docx"),
            "{body}"
        );

        // Running it again changes nothing and still succeeds.
        import(&cfg, &args(&["Notebooks/Docs", "--apply"]), &allow)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn named_files_are_imported_straight_away_and_dry_run_holds_them_back() {
        let (dir, cfg) = vault(Profile::Full);
        import(
            &cfg,
            &args(&["Notebooks/Docs/invoice.docx", "--dry-run"]),
            &allow,
        )
        .await
        .unwrap();
        assert!(!dir.path().join("Notebooks/Docs/invoice.md").exists());
        import(&cfg, &args(&["Notebooks/Docs/invoice.docx"]), &allow)
            .await
            .unwrap();
        assert!(dir.path().join("Notebooks/Docs/invoice.md").is_file());
    }

    #[tokio::test]
    async fn a_file_that_cannot_be_imported_makes_the_command_fail_after_printing() {
        let (_dir, cfg) = vault(Profile::Full);
        let err = import(&cfg, &args(&["Notebooks/Docs/missing.docx"]), &allow)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("could not be imported"), "{err}");
        assert!(import(&cfg, &args(&[]), &allow).await.is_err());
        assert!(
            import(
                &cfg,
                &args(&["Notebooks/Docs", "Notebooks/Docs/invoice.docx"]),
                &allow
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn the_inventory_runs_and_lite_keeps_to_notebooks() {
        let (_dir, cfg) = vault(Profile::Full);
        inventory(&cfg, &args(&["--json"]), &allow).await.unwrap();
        inventory(
            &cfg,
            &args(&["--status", "pending", "--format", "docx,xlsx"]),
            &allow,
        )
        .await
        .unwrap();
        assert!(inventory(&cfg, &args(&["--limit"]), &allow).await.is_err());
        assert!(inventory(&cfg, &args(&["--bogus"]), &allow).await.is_err());

        let (_dir, lite) = vault(Profile::Lite);
        assert_eq!(lite_start(&lite, None, &allow).unwrap(), "Notebooks");
        let deny = |rel: &str| {
            if hq_web::lite_hides(rel) {
                bail!("`{rel}` is not part of the notes in HQ Lite")
            }
            Ok(())
        };
        assert!(lite_start(&lite, Some("_media"), &deny).is_err());
        assert!(inventory(&lite, &args(&["_system"]), &deny).await.is_err());
        assert!(
            import(&lite, &args(&["_media/2026-01-02/scan.docx"]), &deny)
                .await
                .is_err()
        );
        assert!(
            import(
                &lite,
                &args(&["Notebooks/Docs/invoice.docx", "--destination", "_system"]),
                &deny
            )
            .await
            .is_err()
        );
    }

    #[test]
    fn the_text_views_say_what_happened_and_what_to_do_next() {
        let inv = serde_json::json!({
            "folder": "(whole vault)", "found": 2, "files": 1, "matching": 2, "total_bytes": 3_000_000u64,
            "by_status": {"pending": 1, "imported": 1}, "by_format": {"pdf": 2},
            "can_do_here": {"ocr": false, "pdf_tables": true, "legacy_office": false},
            "search_cut_short": false,
            "entries": [{"path": "_media/a.pdf", "bytes": 2_000_000u64, "format": "pdf", "status": "pending", "note": ""}],
        });
        let text = inventory_text(&inv);
        assert!(
            text.contains("2 found (2.9 MB)")
                && text.contains("OCR no")
                && text.contains("PENDING"),
            "{text}"
        );
        assert!(
            text.contains("1 more") && text.contains("--apply"),
            "{text}"
        );

        let r = serde_json::json!({
            "dry_run": true, "requested": 1, "would_create": 1, "skipped": 0, "failed": 0, "degraded": 1, "more_pending": 3,
            "results": [{"source": "a.docx", "action": "would_create", "destination": "Notebooks/a.md",
                "engine": "native:docx", "outcome": "degraded", "warnings": ["2 images not carried into the note"]}],
        });
        let text = import_text(&r, true);
        assert!(
            text.contains("would create")
                && text.contains("2 images")
                && text.contains("3 more pending"),
            "{text}"
        );
        assert!(text.contains("Add --apply"), "{text}");

        let idle = serde_json::json!({"dry_run": false, "requested": 0, "results": []});
        assert!(import_text(&idle, false).starts_with("Nothing pending"));
    }
}
