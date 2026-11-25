**Chat replies and mission lookups use indexes.** Two queries that scanned a whole table on every chat
reply and every mission lookup now use an index, so they stay fast as history grows.
