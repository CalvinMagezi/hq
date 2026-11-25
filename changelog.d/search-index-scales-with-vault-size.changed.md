**Saving and searching notes stays fast in a large vault.** Indexing a note used to read every other note
in the index, so a first index of a 5,000-note vault took about 18 seconds and each search result cost
another full read. Both now look the note up directly: the same first index takes under a second, and
fetching 200 results' titles went from 360 ms to under 2 ms.
