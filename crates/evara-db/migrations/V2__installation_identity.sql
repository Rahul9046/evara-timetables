-- Correct the identity model: a project records who *created* it, not who is using it.
--
-- V1 stored `site_id` in the project, which conflated two different things. `site_id`
-- identifies an Evara *installation*, so it belongs in application settings; a project
-- that carries it would attribute your edits to whoever created the file when you open
-- someone else's project.
--
-- After this migration a project holds:
--   project_id           stable identity of the school project, travels with the file
--   created_by_site_id   the installation that created it, historical fact, never updated
--
-- The installation doing the writing is supplied per connection by the application-defined
-- `evara_site_id()` SQL function, which the change-log triggers call. See
-- crates/evara-db/src/change_log.rs.
--
-- V1 is left untouched: a released migration is immutable, because refinery checksums
-- applied migrations and editing one would make every existing project refuse to open.

-- Rename in place, preserving the value: whoever's id was recorded in V1 is, by
-- definition, the installation that created the project.
UPDATE app_meta
   SET key = 'created_by_site_id',
       updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
 WHERE key = 'site_id'
   AND NOT EXISTS (SELECT 1 FROM app_meta WHERE key = 'created_by_site_id');

-- A project created before V1 recorded anything, or one whose key was already renamed,
-- must not be left without provenance. Unknown is honest; a fabricated id is not.
INSERT INTO app_meta (key, value, updated_at)
SELECT 'created_by_site_id', 'unknown', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
 WHERE NOT EXISTS (SELECT 1 FROM app_meta WHERE key = 'created_by_site_id');

-- Remove any leftover, so nothing can read project-level `site_id` by accident.
DELETE FROM app_meta WHERE key = 'site_id';
