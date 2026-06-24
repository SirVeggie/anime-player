use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::Connection;

pub struct AppDatabase {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl AppDatabase {
    pub fn open_portable() -> Result<Self, String> {
        let data_dir = portable_data_dir()?;
        fs::create_dir_all(&data_dir)
            .map_err(|e| format!("failed to create data directory {data_dir:?}: {e}"))?;

        let path = data_dir.join("anime-player.db");
        let conn = Connection::open(&path)
            .map_err(|e| format!("failed to open database {path:?}: {e}"))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| e.to_string())?;

        let db = Self {
            conn: Mutex::new(conn),
            path,
        };
        db.initialize()?;
        Ok(db)
    }

    pub fn with_conn<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut guard = self.conn.lock().map_err(|e| e.to_string())?;
        f(&mut guard)
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    fn initialize(&self) -> Result<(), String> {
        self.with_conn(|conn| {
            conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
            ensure_schema_updates(conn)?;
            seed_defaults(conn)?;
            Ok(())
        })
    }
}

fn ensure_schema_updates(conn: &Connection) -> Result<(), String> {
    if !table_has_column(conn, "anime", "custom_thumbnail_path")? {
        conn.execute("ALTER TABLE anime ADD COLUMN custom_thumbnail_path TEXT", [])
            .map_err(|e| e.to_string())?;
    }
    if !table_has_column(conn, "anime", "anilist_cached_status")? {
        conn.execute("ALTER TABLE anime ADD COLUMN anilist_cached_status TEXT", [])
            .map_err(|e| e.to_string())?;
    }
    if !table_has_column(conn, "anime", "no_op_ed")? {
        conn.execute(
            "ALTER TABLE anime ADD COLUMN no_op_ed INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    if !table_has_column(conn, "anime", "op_ed_analysis_version")? {
        conn.execute(
            "ALTER TABLE anime ADD COLUMN op_ed_analysis_version INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    if !table_has_column(conn, "anime", "op_ed_analyzed_at")? {
        conn.execute("ALTER TABLE anime ADD COLUMN op_ed_analyzed_at TEXT", [])
            .map_err(|e| e.to_string())?;
    }
    if !table_has_column(conn, "anime", "anilist_cached_mean_score")? {
        conn.execute("ALTER TABLE anime ADD COLUMN anilist_cached_mean_score REAL", [])
            .map_err(|e| e.to_string())?;
    }
    if !table_has_column(conn, "anime", "anilist_cached_description")? {
        conn.execute("ALTER TABLE anime ADD COLUMN anilist_cached_description TEXT", [])
            .map_err(|e| e.to_string())?;
    }
    if !table_has_column(conn, "anime", "pending_delete")? {
        conn.execute(
            "ALTER TABLE anime ADD COLUMN pending_delete INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    if !table_has_column(conn, "episodes", "pending_delete")? {
        conn.execute(
            "ALTER TABLE episodes ADD COLUMN pending_delete INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| e.to_string())?;
    }

    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS op_ed_templates (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  anime_id INTEGER NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('op', 'ed')),
  block_index INTEGER NOT NULL DEFAULT 0,
  start_sec REAL NOT NULL,
  duration_sec REAL NOT NULL,
  confidence REAL NOT NULL DEFAULT 0,
  fingerprint_cache_key TEXT NOT NULL,
  source_episode_ids TEXT,
  source TEXT NOT NULL DEFAULT 'auto' CHECK(source IN ('auto', 'manual')),
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY(anime_id) REFERENCES anime(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS episode_op_ed_segments (
  episode_id INTEGER NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('op', 'ed')),
  status TEXT NOT NULL DEFAULT 'pending',
  start_sec REAL,
  end_sec REAL,
  confidence REAL,
  template_id INTEGER,
  search_pass TEXT NOT NULL DEFAULT 'none',
  fingerprint_cache_key TEXT,
  error_text TEXT,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (episode_id, kind),
  FOREIGN KEY(episode_id) REFERENCES episodes(id) ON DELETE CASCADE,
  FOREIGN KEY(template_id) REFERENCES op_ed_templates(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_op_ed_templates_anime ON op_ed_templates(anime_id);
CREATE INDEX IF NOT EXISTS idx_episode_op_ed_status ON episode_op_ed_segments(episode_id, status);

CREATE TABLE IF NOT EXISTS library_operations (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  operation_type TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('queued', 'running', 'done', 'failed', 'canceled')),
  phase TEXT NOT NULL DEFAULT 'queued',
  target_anime_id INTEGER,
  target_episode_id INTEGER,
  payload_json TEXT NOT NULL DEFAULT '{}',
  progress_current INTEGER NOT NULL DEFAULT 0,
  progress_total INTEGER NOT NULL DEFAULT 0,
  summary_json TEXT,
  error_text TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  started_at TEXT,
  finished_at TEXT,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY(target_anime_id) REFERENCES anime(id) ON DELETE SET NULL,
  FOREIGN KEY(target_episode_id) REFERENCES episodes(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_library_operations_status ON library_operations(status, id);
CREATE INDEX IF NOT EXISTS idx_library_operations_type ON library_operations(operation_type, id);
CREATE INDEX IF NOT EXISTS idx_library_operations_target_anime ON library_operations(target_anime_id);
CREATE INDEX IF NOT EXISTS idx_library_operations_target_episode ON library_operations(target_episode_id);
"#,
    )
    .map_err(|e| e.to_string())?;

    if !table_has_column(conn, "op_ed_templates", "source")? {
        conn.execute(
            "ALTER TABLE op_ed_templates ADD COLUMN source TEXT NOT NULL DEFAULT 'auto'",
            [],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&pragma).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?;
    for name in rows {
        if name.map_err(|e| e.to_string())? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn portable_data_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("failed to resolve exe path: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| format!("failed to resolve parent directory for {exe:?}"))?;
    Ok(dir.join("data"))
}

/// Default detection rules when `regex_rules` is empty. Use parameterized inserts
/// so regex literals (e.g. `.'!` in Generic) never break SQL quoting.
const DEFAULT_REGEX_RULES: &[(
    i64,
    &str,
    &str,
    &str,
    i64,
    i64,
)] = &[
    (
        1,
        "Fansub",
        r"^\[(\w+)\] .*? - (\w+ )?\d+",
        r"^\[(\w+)\] (?P<title>.*?) - (?P<episode>\d+(\.\d+)?)",
        1,
        10,
    ),
    (
        2,
        "Fansub (no ep)",
        r"^\[(\w+)\] .*? - (\w+ )?\d+",
        r"^\[(\w+)\] (?P<title>.*?) - (?P<episode>\d+(\.\d+)?)?",
        1,
        9,
    ),
    (
        3,
        "Series",
        ".",
        r"(?i)^(\[\w+\])?(?P<title>.*?(S\d+|[.\- ]))E(?P<episode>\d+)",
        1,
        8,
    ),
    (
        4,
        "Simple",
        r"^([\w\s,.!]|\w-\w)+ (- \w+|(- )?\d+)",
        r"^(?P<title>([\w\s,.!]|\w-\w)+) (- )?(?P<episode>\d+(\.\d+)?)",
        1,
        5,
    ),
    (
        5,
        "Simple (no ep)",
        r"^([\w\s,.!]|\w-\w)+ (- \w+|(- )?\d+)",
        r"^(?P<title>([\w\s,.!]|\w-\w)+) (- )?(?P<episode>\d+(\.\d+)?)?",
        1,
        4,
    ),
    (
        6,
        "Generic",
        ".",
        r"(\[\w+\])?(?P<title>[\w\s\-,.'!]+).*\.\w+",
        1,
        0,
    ),
];

fn seed_defaults(conn: &Connection) -> Result<(), String> {
    let category_count = conn
        .query_row("SELECT COUNT(*) FROM categories", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|e| e.to_string())?;

    if category_count == 0 {
        conn.execute(
            "INSERT INTO categories (id, name, is_default, sort_order)
             VALUES (1, 'Ongoing', 1, 0)",
            [],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO categories (id, name, is_default, sort_order)
             VALUES (2, 'Completed', 0, 1)",
            [],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO categories (id, name, is_default, sort_order)
             VALUES (3, 'Finished', 0, 2)",
            [],
        )
        .map_err(|e| e.to_string())?;
    }

    let rule_count = conn
        .query_row("SELECT COUNT(*) FROM regex_rules", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|e| e.to_string())?;

    if rule_count == 0 {
        insert_default_regex_rules(conn)?;
    }

    Ok(())
}

pub fn insert_default_regex_rules(conn: &Connection) -> Result<(), String> {
    for (id, name, detection, title, enabled, priority) in DEFAULT_REGEX_RULES {
        conn.execute(
            "INSERT INTO regex_rules
                (id, name, detection_regex, title_regex, enabled, priority)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![id, name, detection, title, enabled, priority],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn reset_regex_rules_to_defaults(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM regex_rules", [])
        .map_err(|e| e.to_string())?;
    insert_default_regex_rules(conn)
}

/// Sets `anime.latest_episode_at` to the latest available episode update.
pub fn refresh_anime_latest_episode_at(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "UPDATE anime SET latest_episode_at = (
            SELECT MAX(e.updated_at) FROM episodes e
            WHERE e.anime_id = anime.id AND e.missing = 0 AND e.pending_delete = 0
        )
        WHERE latest_episode_at IS NOT (
            SELECT MAX(e.updated_at) FROM episodes e
            WHERE e.anime_id = anime.id AND e.missing = 0 AND e.pending_delete = 0
        )",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS root_folders (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  path TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS regex_rules (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  detection_regex TEXT NOT NULL,
  title_regex TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  priority INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS categories (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL UNIQUE,
  is_default INTEGER NOT NULL DEFAULT 0,
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS anime (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  title TEXT NOT NULL,
  title_key TEXT NOT NULL UNIQUE,
  category_id INTEGER NOT NULL,
  anilist_id INTEGER UNIQUE,
  anilist_title TEXT,
  anilist_site_url TEXT,
  anilist_cover_path TEXT,
  custom_thumbnail_path TEXT,
  anilist_cached_progress INTEGER,
  anilist_cached_episodes INTEGER,
  anilist_cached_score REAL,
  anilist_cached_status TEXT,
  anilist_status_fetched_at INTEGER,
  tracker_offset INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  last_watched_at TEXT,
  latest_episode_at TEXT,
  pending_delete INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY(category_id) REFERENCES categories(id)
);

CREATE TABLE IF NOT EXISTS episodes (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  anime_id INTEGER NOT NULL,
  root_folder_id INTEGER,
  path TEXT NOT NULL UNIQUE,
  relative_path TEXT NOT NULL,
  file_name TEXT NOT NULL,
  file_type TEXT NOT NULL,
  episode_number REAL,
  size INTEGER NOT NULL DEFAULT 0,
  duration_seconds REAL NOT NULL DEFAULT 0,
  position_seconds REAL NOT NULL DEFAULT 0,
  watched INTEGER NOT NULL DEFAULT 0,
  missing INTEGER NOT NULL DEFAULT 0,
  date_added TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  last_watched_at TEXT,
  pending_delete INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY(anime_id) REFERENCES anime(id) ON DELETE CASCADE,
  FOREIGN KEY(root_folder_id) REFERENCES root_folders(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_episodes_anime_id ON episodes(anime_id);
CREATE INDEX IF NOT EXISTS idx_episodes_last_watched_at ON episodes(last_watched_at);
CREATE INDEX IF NOT EXISTS idx_episodes_missing ON episodes(missing);
CREATE INDEX IF NOT EXISTS idx_anime_category_id ON anime(category_id);
CREATE INDEX IF NOT EXISTS idx_anime_last_watched_at ON anime(last_watched_at);
CREATE UNIQUE INDEX IF NOT EXISTS idx_anime_anilist_id ON anime(anilist_id) WHERE anilist_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS unmatched_files (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  root_folder_id INTEGER,
  path TEXT NOT NULL UNIQUE,
  relative_path TEXT NOT NULL,
  file_name TEXT NOT NULL,
  reason TEXT NOT NULL,
  detected_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY(root_folder_id) REFERENCES root_folders(id) ON DELETE CASCADE
);
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;

    #[test]
    fn seed_defaults_restores_rules_only_when_table_empty() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();

        seed_defaults(&conn).unwrap();
        assert_eq!(rule_count(&conn), DEFAULT_REGEX_RULES.len() as i64);
        assert_generic_rule_stored(&conn);

        conn.execute("DELETE FROM regex_rules WHERE id = 1", []).unwrap();
        seed_defaults(&conn).unwrap();
        assert_eq!(rule_count(&conn), DEFAULT_REGEX_RULES.len() as i64 - 1);
        assert!(
            conn.query_row(
                "SELECT 1 FROM regex_rules WHERE id = 1",
                [],
                |_| Ok(()),
            )
            .optional()
            .unwrap()
            .is_none()
        );

        conn.execute("DELETE FROM regex_rules", []).unwrap();
        seed_defaults(&conn).unwrap();
        assert_eq!(rule_count(&conn), DEFAULT_REGEX_RULES.len() as i64);
        assert_generic_rule_stored(&conn);

        conn.execute(
            "UPDATE regex_rules SET name = 'Broken' WHERE id = 6",
            [],
        )
        .unwrap();
        reset_regex_rules_to_defaults(&conn).unwrap();
        assert_eq!(rule_count(&conn), DEFAULT_REGEX_RULES.len() as i64);
        assert_generic_rule_stored(&conn);
    }

    fn assert_generic_rule_stored(conn: &Connection) {
        let title_regex: String = conn
            .query_row(
                "SELECT title_regex FROM regex_rules WHERE id = 6",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            title_regex.contains("'!"),
            "Generic rule should keep apostrophe in character class: {title_regex:?}"
        );
    }

    fn rule_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM regex_rules", [], |row| row.get(0))
            .unwrap()
    }
}
