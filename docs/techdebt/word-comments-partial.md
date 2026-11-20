# Word comments in headers, footers and notes, and reply threads, are not carried

**Reason skipped:** the first version places only comments whose range is in the document body, which covers review of the text people actually edit.
**Date:** 2026-11-20
**Where:** `crates/hq-convert/src/ingest/docx_comments.rs`, `crates/hq-convert/src/ingest/docx.rs`, `crates/suite/hq-suite-ingest/src/comments.rs`
**Remove when:** a header, footer or note comment has a block to sit on in a note (they have none today), or replies get a threaded form in `comment` blocks.

A comment on a header, footer, footnote or endnote is counted as unanchored in the loss report and
dropped. A reply is written as its own comment on the block of the comment it answers; the thread
link (`paraIdParent`) is not kept. The exact span the comment covered inside a paragraph is lost, as
the anchor is the whole block.
