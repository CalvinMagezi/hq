-- What HQ remembers gets a way to be found by what is being talked about, and the facts a
-- reader needs to judge a row.
--
-- Columns, all with a default so every existing row stays valid:
--   status       active, or review (held back until the owner looks), or superseded (the owner
--                said it is out of date). Only active rows are recalled on their own.
--   origin       owner, or unknown_channel for a row from a shared channel that cannot be tied to
--                the owner. Discord is the one surface where someone else can speak (a family
--                guest), so its older rows are marked below; they are not recalled until the
--                owner keeps them.
--   provenance   owner_said, model_inferred or owner_edited. Rows written before this column are
--                summaries of both sides of a turn, so they start as model_inferred.
--   thread_id    the chat the row came from, so a row can be traced and removed with it.
--   pinned       the owner asked HQ to keep this; it never ages out.
--   seen_count   how many times the same fact was said (a repeat reinforces, it is not a new row).
--   content_hash a hash of the normalised summary, to find a repeat and to block a re-learn.
ALTER TABLE memories ADD COLUMN status TEXT NOT NULL DEFAULT 'active';
ALTER TABLE memories ADD COLUMN origin TEXT NOT NULL DEFAULT 'owner';
ALTER TABLE memories ADD COLUMN provenance TEXT NOT NULL DEFAULT 'model_inferred';
ALTER TABLE memories ADD COLUMN thread_id TEXT;
ALTER TABLE memories ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN seen_count INTEGER NOT NULL DEFAULT 1;
ALTER TABLE memories ADD COLUMN last_seen_at TEXT;
ALTER TABLE memories ADD COLUMN content_hash TEXT;

-- The insight note a consolidation wrote, recorded instead of rebuilt from a timestamp.
ALTER TABLE consolidations ADD COLUMN note_path TEXT;

UPDATE memories SET origin = 'unknown_channel' WHERE harness = 'discord';

CREATE INDEX IF NOT EXISTS idx_memories_content_hash ON memories(content_hash);

-- Keyword search over summaries, the people and places named, and the topics. The FTS rowid is
-- the memory id, so every query joins back on it and a row that is gone is never returned.
-- Entities are weighted above the summary at query time: names and places are what a question
-- most often shares with a memory, and they are the same in any language.
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    summary,
    entities,
    topics,
    tokenize = 'porter unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS memories_fts_insert AFTER INSERT ON memories BEGIN
    INSERT OR REPLACE INTO memories_fts (rowid, summary, entities, topics)
    VALUES (new.id, COALESCE(NULLIF(new.summary, ''), new.content, ''), new.entities, new.topics);
END;

CREATE TRIGGER IF NOT EXISTS memories_fts_delete AFTER DELETE ON memories BEGIN
    DELETE FROM memories_fts WHERE rowid = old.id;
END;

CREATE TRIGGER IF NOT EXISTS memories_fts_update AFTER UPDATE OF summary, entities, topics ON memories BEGIN
    DELETE FROM memories_fts WHERE rowid = old.id;
    INSERT OR REPLACE INTO memories_fts (rowid, summary, entities, topics)
    VALUES (new.id, COALESCE(NULLIF(new.summary, ''), new.content, ''), new.entities, new.topics);
END;

INSERT INTO memories_fts (rowid, summary, entities, topics)
    SELECT id, COALESCE(NULLIF(summary, ''), content, ''), entities, topics FROM memories;
