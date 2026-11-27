-- `created_at` on memories and consolidations is declared INTEGER, but HQ has always written an
-- RFC 3339 string. A row an older build wrote as a number (seconds, or milliseconds from the
-- earlier TypeScript version) compared wrongly against every cutoff string and made the daily
-- forgetting pass fail on read. Rewrite only those rows, to the same text form everything else
-- uses. A row that is already text is left exactly as it is, so running this twice, or on a
-- database with no such rows, changes nothing.
UPDATE memories
   SET created_at = strftime(
           '%Y-%m-%dT%H:%M:%S',
           CASE WHEN created_at > 100000000000 THEN created_at / 1000 ELSE created_at END,
           'unixepoch') || '+00:00'
 WHERE typeof(created_at) = 'integer';

UPDATE consolidations
   SET created_at = strftime(
           '%Y-%m-%dT%H:%M:%S',
           CASE WHEN created_at > 100000000000 THEN created_at / 1000 ELSE created_at END,
           'unixepoch') || '+00:00'
 WHERE typeof(created_at) = 'integer';
