//! Opt-in diagnostics for the link grammar: a census of a real vault, and a scale benchmark.
//!
//! Neither is a regression gate. Both are read-only, print counts to the terminal and send
//! nothing anywhere.
//!
//! ```text
//! HQ_VAULT_DIAG_PATH=/path/to/vault cargo test -p hq-vault --test links_diag \
//!     --release -- --ignored --nocapture real_vault_link_census
//! cargo test -p hq-vault --test links_diag --release -- --ignored --nocapture scale_benchmark
//! ```

use hq_vault::links::{NoteInput, NoteSet, Resolution, aliases_of, extract};
use hq_vault::walk::walk;
use std::path::Path;
use std::time::Instant;

const MAX_NOTE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Default, Debug)]
struct Tally {
    links: usize,
    unique: usize,
    ambiguous: usize,
    attachment: usize,
    external: usize,
    same_note: usize,
    dangling: usize,
    in_frontmatter: usize,
}

/// What one pass over a vault found and how long each step took.
struct Census {
    tally: Tally,
    /// Dangling links as `(note, line, target)`.
    dangling: Vec<(String, usize, String)>,
    walk_ms: u128,
    read_ms: u128,
    resolve_ms: u128,
    notes: usize,
}

/// Read, tokenize and resolve every note under `vault`.
fn census(vault: &Path) -> Census {
    let t0 = Instant::now();
    let walked = walk(vault, false);
    let t_walk = t0.elapsed().as_millis();

    let t1 = Instant::now();
    let mut texts = Vec::with_capacity(walked.notes.len());
    for note in &walked.notes {
        if note.size > MAX_NOTE_BYTES {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&note.abs) {
            texts.push((note.rel.clone(), text));
        }
    }
    let t_read = t1.elapsed().as_millis();

    let t2 = Instant::now();
    let set = NoteSet::new(
        texts.iter().map(|(rel, text)| NoteInput {
            path: rel.clone(),
            aliases: aliases_of(text),
        }),
        walked.attachments.iter().cloned(),
    );
    let mut tally = Tally::default();
    let mut dangling = Vec::new();
    for (rel, text) in &texts {
        for link in extract(text) {
            tally.links += 1;
            tally.in_frontmatter += usize::from(link.in_frontmatter);
            match set.resolve(rel, &link) {
                Resolution::Unique(_) => tally.unique += 1,
                Resolution::Ambiguous(_) => tally.ambiguous += 1,
                Resolution::Attachment(_) => tally.attachment += 1,
                Resolution::External => tally.external += 1,
                Resolution::SameNote => tally.same_note += 1,
                Resolution::Dangling => {
                    tally.dangling += 1;
                    dangling.push((rel.clone(), link.line, link.target.clone()));
                }
            }
        }
    }
    let t_resolve = t2.elapsed().as_millis();
    Census {
        tally,
        dangling,
        walk_ms: t_walk,
        read_ms: t_read,
        resolve_ms: t_resolve,
        notes: texts.len(),
    }
}

#[test]
#[ignore = "reads a real vault: set HQ_VAULT_DIAG_PATH to its folder, run with --ignored --nocapture"]
fn real_vault_link_census() {
    let path =
        std::env::var("HQ_VAULT_DIAG_PATH").expect("set HQ_VAULT_DIAG_PATH to a vault folder");
    let Census {
        tally,
        dangling,
        walk_ms,
        read_ms,
        resolve_ms,
        notes,
    } = census(Path::new(&path));
    println!("notes read:      {notes}");
    println!("links:           {}", tally.links);
    println!("  one note:      {}", tally.unique);
    println!("  several notes: {}", tally.ambiguous);
    println!("  attachment:    {}", tally.attachment);
    println!("  outside:       {}", tally.external);
    println!("  same note:     {}", tally.same_note);
    println!("  nothing:       {}", tally.dangling);
    println!("  in frontmatter:{}", tally.in_frontmatter);
    println!("time: walk {walk_ms} ms, read {read_ms} ms, resolve {resolve_ms} ms");
    // Paths and link text stay on the terminal; set the variable to see a few.
    if std::env::var("HQ_VAULT_DIAG_SAMPLES").is_ok() {
        for (note, line, target) in dangling.iter().take(15) {
            println!("  {note}:{line}  [[{target}]]");
        }
    }
}

/// A vault of `n` notes on disk, `links_per_note` links each, in the shapes a real vault has.
fn synthetic_vault(root: &Path, n: usize, links_per_note: usize) {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    for i in 0..n {
        let dir = root.join(format!("Notebooks/Folder {}", i % 40));
        std::fs::create_dir_all(&dir).unwrap();
        let mut body = format!("---\ntitle: Note {i}\ntags: [bench]\n---\n\n# Note {i}\n\n");
        for l in 0..links_per_note {
            let target = next() % (n + n / 10); // about one in eleven points nowhere
            match l % 5 {
                0 => body.push_str(&format!("See [[Note {target}]] for more.\n")),
                1 => body.push_str(&format!(
                    "Also [[Folder {}/Note {target}#Part|label]].\n",
                    target % 40
                )),
                2 => body.push_str(&format!(
                    "Read [it](../Folder {}/Note {target}.md).\n",
                    target % 40
                )),
                3 => body.push_str(&format!("![[Note {target}]]\n")),
                _ => body.push_str("Some `[[code]]` and https://example.com/a text.\n"),
            }
        }
        std::fs::write(dir.join(format!("Note {i}.md")), body).unwrap();
    }
}

#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture scale_benchmark"]
fn scale_benchmark() {
    for n in [5_000usize, 20_000] {
        let dir = tempfile::tempdir().unwrap();
        synthetic_vault(dir.path(), n, 20);
        let Census {
            tally,
            walk_ms,
            read_ms,
            resolve_ms,
            notes,
            ..
        } = census(dir.path());
        println!(
            "{n} notes ({notes} read), {} links: walk {walk_ms} ms, read {read_ms} ms, \
             tokenize+resolve {resolve_ms} ms, {} dangling",
            tally.links, tally.dangling
        );
        assert_eq!(notes, n);
        assert!(tally.links > 0);
    }
}

/// The whole check as the daemon runs it: read every note, scan for problems, plan the repairs.
/// The planner looks for a near-match name for every broken link, so this is where a vault with
/// many broken links would be slow.
#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture pipeline_benchmark"]
fn pipeline_benchmark() {
    use hq_vault::{fixes, health, relink};
    for n in [5_000usize, 20_000] {
        let dir = tempfile::tempdir().unwrap();
        synthetic_vault(dir.path(), n, 20);
        let t0 = Instant::now();
        let snap = relink::snapshot(dir.path());
        let read_ms = t0.elapsed().as_millis();
        // One index of note names for both, as the daemon does it: a missing name is searched once.
        let names = hq_vault::check::NameIndex::for_snapshot(&snap);
        let t1 = Instant::now();
        let report = health::scan_full_with(&snap, &names);
        let scan_ms = t1.elapsed().as_millis();
        let t2 = Instant::now();
        let plan = fixes::plan_with(&snap, &fixes::Context::default(), &names);
        let plan_ms = t2.elapsed().as_millis();
        println!(
            "{n} notes: read {read_ms} ms, scan {scan_ms} ms ({} dead links), plan {plan_ms} ms ({} fixes)",
            report.counts.links.dead,
            plan.fixes.len()
        );
        assert_eq!(snap.notes.len(), n);
    }
}
