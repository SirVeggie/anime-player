# Migrations

Schema and persisted-data changes. Additive SQLite updates run at startup
from `ensure_schema_updates` in `src-tauri/src/db.rs`. There is no old-data
rewrite unless a change below says so.

## Audio / subtitle track preferences

- **What changed:** New `anime_track_prefs` and `episode_track_prefs` tables
  store the last explicit audio/subtitle choice. Episode rows are overrides;
  the anime row is the latest choice used for episodes without an override.
- **Why:** Track menus previously only changed the current mpv session.
  Returning to an episode or advancing to the next one forgot the selection.
- **Affected versions:** Existing installs have no prior track data. Tables
  are empty until the user picks tracks. No data conversion.
- **Where:** `CREATE TABLE IF NOT EXISTS` in `ensure_schema_updates`
  (`src-tauri/src/db.rs`). Read/write and matching live in
  `src-tauri/src/track_prefs.rs`.
- **Verify:** Pick audio/subs on episode 1, reopen it (same tracks), open
  episode 2 (closest match). Change episode 2 only; episode 1 still has its
  own choice. `J` / `#` after load should persist on leave / next episode.
- **When to remove:** Keep the `CREATE TABLE IF NOT EXISTS` statements; they
  are the schema definition for new and existing databases.
