//! Every sentence the suite's commands print, in one place, so another language can replace this
//! file's contents without touching the logic.

use hq_suite_shrink::{OriginalAt, Preview};
use hq_suite_store::Step;

pub fn size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KIB {
        format!("{bytes} B")
    } else if b < KIB * KIB {
        format!("{:.0} KB", b / KIB)
    } else {
        format!("{:.1} MB", b / (KIB * KIB))
    }
}

/// A loss the importer reported, in words.
fn loss(kind: &str) -> String {
    match kind {
        "charts_omitted" => "charts and diagrams".into(),
        "embedded_objects_omitted" => "embedded objects".into(),
        "nested_tables_flattened" => "tables inside tables (kept as text)".into(),
        "truncated" => "the end of a very long file".into(),
        "layout_not_preserved" => "page layout".into(),
        "ocr_derived" => "text read from a picture".into(),
        other => other.replace('_', " "),
    }
}

pub fn preview(p: &Preview) -> String {
    let mut out = format!(
        "{} is ready to be made smaller.\n  Original {}, record {}: {} saved.\n  The record is the note {}",
        p.source,
        size(p.source_bytes),
        size(p.record_bytes),
        size(p.saved_bytes.max(0) as u64),
        p.note
    );
    if p.pictures_recompressed + p.pictures_kept > 0 {
        out.push_str(&format!(
            ", with {} picture{} made smaller and {} kept as they were",
            p.pictures_recompressed,
            if p.pictures_recompressed == 1 {
                ""
            } else {
                "s"
            },
            p.pictures_kept
        ));
    }
    out.push_str(".\n");
    out.push_str("  The note does not keep page layout (margins, fonts, columns).\n");
    if p.loss_kinds.is_empty() {
        out.push_str("  Nothing else was left out.\n");
    } else {
        let list: Vec<String> = p.loss_kinds.iter().map(|k| loss(k)).collect();
        out.push_str(&format!("  Left out: {}.\n", list.join(", ")));
    }
    out.push_str("  Your original has not been moved.\n");
    out
}

pub fn ask_to_move() -> &'static str {
    "Move the original to the trash? You can undo this for 30 days. [y/N] "
}

pub fn next_steps(job: &str) -> String {
    format!(
        "To go ahead: hq suite confirm {job}\nTo leave everything as it was: hq suite abandon {job}\n"
    )
}

pub fn moved(source: &str, trash: &str, saved: i64) -> String {
    format!(
        "Done. {source} is in the trash at {trash} for 30 days; {} saved. To bring it back: hq suite undo <job>.\n",
        size(saved.max(0) as u64)
    )
}

pub fn undone(source: &str) -> String {
    format!("{source} is back where it was.\n")
}

pub fn ask_recovery_code() -> &'static str {
    "Recovery code for this sink: "
}

pub fn no_sinks() -> &'static str {
    "No sinks yet. Make one with: hq suite sink add <name> <folder>\n"
}

pub fn sink_made(id: &str, folder: &str, code: &str) -> String {
    format!(
        "The sink {id} is ready in {folder}.\n\nYour recovery code is:\n\n    {code}\n\nWrite it down and keep it somewhere safe that is not this phone. It is shown only now. HQ does not keep it, and without it nothing sealed into this sink can be opened, by you or by anyone.\n"
    )
}

pub fn sink_registered(id: &str) -> String {
    format!(
        "The sink {id} is registered. To put its originals back: hq suite restore-originals --sink {id}\n"
    )
}

pub fn restored(r: &crate::restore::Restored) -> String {
    let mut out = format!(
        "{} original(s) put back, {} already in place.\n",
        r.restored.len(),
        r.already_there.len()
    );
    for p in &r.in_the_way {
        out.push_str(&format!(
            "  {p}: another file is there, so nothing was written\n"
        ));
    }
    for (p, why) in &r.failed {
        out.push_str(&format!("  {p}: {why}\n"));
    }
    out
}

pub fn offloaded(source: &str, sink: &str, saved: i64, job: &str) -> String {
    format!(
        "Done. {source} was sealed into the sink {sink} and removed from this phone; {} saved. To bring it back: hq suite undo {job}.\n",
        size(saved.max(0) as u64)
    )
}

pub fn drill(d: &hq_suite_sinks::Drill) -> String {
    if d.checked == 0 {
        return "The sink holds nothing yet, so there was nothing to check.\n".into();
    }
    if d.failed.is_empty() {
        format!(
            "Checked {} stored original(s): every one opened and matched.\n",
            d.checked
        )
    } else {
        format!(
            "Checked {} stored original(s): {} could not be restored ({}). Keep the originals you still have.\n",
            d.checked,
            d.failed.len(),
            d.failed
                .iter()
                .map(|h| &h[..12])
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

pub fn abandoned() -> &'static str {
    "Left as it was. The new note was removed and the original was not touched.\n"
}

pub fn step_words(step: Step, at: OriginalAt) -> &'static str {
    match (step, at) {
        (Step::Planned | Step::NoteWritten, _) => "being prepared",
        (Step::Verified, _) => "waiting for your answer",
        (Step::Consented | Step::OriginalMoved, _) => "finishing",
        (Step::Done, OriginalAt::Trash) => "done; original in the trash",
        (Step::Done, OriginalAt::Sink) => "done; original sealed in a sink",
        (Step::Done, _) => "done",
        (Step::Undone, _) => "undone",
        (Step::Abandoned, _) => "left as it was",
    }
}

pub fn no_jobs() -> &'static str {
    "No shrink jobs yet.\n"
}

pub fn recovered(n: usize) -> String {
    if n == 0 {
        "Nothing needed finishing.\n".into()
    } else {
        format!(
            "Looked at {n} unfinished job{}.\n",
            if n == 1 { "" } else { "s" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_plainly() {
        assert_eq!(size(512), "512 B");
        assert_eq!(size(2048), "2 KB");
        assert_eq!(size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn a_preview_says_what_was_left_out_and_that_nothing_moved() {
        let p = Preview {
            job: "sj-1".into(),
            source: "Notebooks/a.docx".into(),
            note: "Notebooks/a.md".into(),
            step: Step::Verified,
            fidelity: "lossy".into(),
            source_format: "docx".into(),
            loss_kinds: vec!["charts_omitted".into(), "something_new".into()],
            source_bytes: 200 * 1024,
            record_bytes: 20 * 1024,
            saved_bytes: 180 * 1024,
            pictures_recompressed: 1,
            pictures_kept: 2,
            original_at: OriginalAt::Source,
        };
        let t = preview(&p);
        assert!(t.contains("180 KB saved"), "{t}");
        assert!(t.contains("1 picture made smaller and 2 kept"), "{t}");
        assert!(t.contains("charts and diagrams, something new"), "{t}");
        assert!(t.contains("has not been moved"), "{t}");
        assert!(!t.contains('\u{2014}'));
    }
}
