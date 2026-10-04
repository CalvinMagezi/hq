-- vault_health stopped writing wikilink rows, and get_related_paths never read
-- them: their source is an absolute path and their target a bare link title.
DELETE FROM graph_links WHERE link_type = 'wikilink';
