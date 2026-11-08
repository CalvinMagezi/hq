-- Keyword search over chat messages and fold summaries (docs/plans/context-fold.md), replacing
-- the LIKE scan behind session_search. Each FTS row carries its message or fold id, and every
-- query joins back on that id, so a row whose source is gone is never returned. The FTS rowid
-- follows the source rowid only to make deletes cheap; INSERT OR REPLACE keeps an insert from
-- failing if a VACUUM ever renumbered the source.
CREATE VIRTUAL TABLE IF NOT EXISTS chat_fts USING fts5(
    message_id UNINDEXED,
    thread_id UNINDEXED,
    content,
    tokenize = 'porter unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS chat_fts_insert AFTER INSERT ON chat_messages BEGIN
    INSERT OR REPLACE INTO chat_fts (rowid, message_id, thread_id, content)
    VALUES (new.rowid, new.message_id, new.thread_id, new.content);
END;

CREATE TRIGGER IF NOT EXISTS chat_fts_delete AFTER DELETE ON chat_messages BEGIN
    DELETE FROM chat_fts WHERE rowid = old.rowid AND message_id = old.message_id;
END;

CREATE TRIGGER IF NOT EXISTS chat_fts_update AFTER UPDATE OF content ON chat_messages BEGIN
    DELETE FROM chat_fts WHERE rowid = old.rowid AND message_id = old.message_id;
    INSERT OR REPLACE INTO chat_fts (rowid, message_id, thread_id, content)
    VALUES (new.rowid, new.message_id, new.thread_id, new.content);
END;

INSERT INTO chat_fts (rowid, message_id, thread_id, content)
    SELECT rowid, message_id, thread_id, content FROM chat_messages;

CREATE VIRTUAL TABLE IF NOT EXISTS fold_fts USING fts5(
    fold_id UNINDEXED,
    thread_id UNINDEXED,
    summary,
    tokenize = 'porter unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS fold_fts_insert AFTER INSERT ON thread_folds BEGIN
    INSERT OR REPLACE INTO fold_fts (rowid, fold_id, thread_id, summary)
    VALUES (new.rowid, new.id, new.thread_id, new.summary);
END;

CREATE TRIGGER IF NOT EXISTS fold_fts_delete AFTER DELETE ON thread_folds BEGIN
    DELETE FROM fold_fts WHERE rowid = old.rowid AND fold_id = old.id;
END;

-- Fold summaries embedded on the device (the local Model2Vec model), for meaning-based search
-- that works offline. Only summaries are embedded: they are few and dense, and embedding every
-- chat message on a phone would cost far more than it finds.
CREATE TABLE IF NOT EXISTS thread_fold_embeddings (
    fold_id   TEXT PRIMARY KEY REFERENCES thread_folds(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL,
    model     TEXT NOT NULL,
    vector    BLOB NOT NULL
);
