# A link to a note in the Trash is reported but not offered a restore

**Reason skipped:** the plan offered "restore the note" as a likely fix for a link whose note was deleted recently. Restoring puts a note the owner deleted back without them asking, which is a bigger step than rewriting a link, and the Trash screen already restores one note at a time.
**Date:** 2026-11-27
**Where:** `crates/hq-vault/src/fixes.rs` (`choose`)
**Remove when:** the Review page can show the deleted note beside the link and ask, so a restore is a choice and not a side effect of a repair.
