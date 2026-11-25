**`hq memory graph` reads the same database as the rest of HQ.** It used to open a separate file under
`_embeddings`, so it could come back empty and leave a stray database behind.
