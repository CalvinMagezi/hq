-- How many times the owner answered "Allow once" to a fetch of a site, in any chat. Kept apart from
-- fetch_site_grants so "Allow all" and a decline (which clear a chat's grant) never change it.
CREATE TABLE IF NOT EXISTS site_allowed_once (
    host TEXT PRIMARY KEY,
    count INTEGER NOT NULL DEFAULT 0
);
