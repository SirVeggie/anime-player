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
        conn.execute(
            "INSERT INTO regex_rules
                (id, name, detection_regex, title_regex, enabled, priority)
             VALUES
                (1, 'Fansub',
                 '^\\[(\\w+)\\] .*? - (\\w+ )?\\d+',
                 '^\\[(\\w+)\\] (?P<title>.*?) - (?P<episode>\\d+(\\.\\d+)?)',
                 1, 10),
                (2, 'Fansub (no ep)',
                 '^\\[(\\w+)\\] .*? - (\\w+ )?\\d+',
                 '^\\[(\\w+)\\] (?P<title>.*?) - (?P<episode>\\d+(\\.\\d+)?)?',
                 1, 9),
                (3, 'Simple',
                 '^([\\w, ]|\\w-\\w)+ (- \\w+|(- )?\\d+)',
                 '^(?P<title>([\\w, ]|\\w-\\w)+) (- )?(?P<episode>\\d+(\\.\\d+)?)',
                 1, 5),
                (4, 'Simple (no ep)',
                 '^([\\w, ]|\\w-\\w)+ (- \\w+|(- )?\\d+)',
                 '^(?P<title>([\\w, ]|\\w-\\w)+) (- )?(?P<episode>\\d+(\\.\\d+)?)?',
                 1, 4),
                (5, 'Generic',
                 '(?i)\\.(mp4|mkv|m4v|mov|avi|wmv|flv|webm|ts|m2ts|mts|ogv|ogm|vob|3gp|rm|rmvb|mpg|mpeg)$',
                 '(?P<title>[\\w\\s.\\-,]+)\\.\\w+',
                 1, 0)",
            [],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Sets `anime.latest_episode_at` to the latest available episode update.
pub fn refresh_anime_latest_episode_at(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "UPDATE anime SET latest_episode_at = (
            SELECT MAX(e.updated_at) FROM episodes e
            WHERE e.anime_id = anime.id AND e.missing = 0
        )
        WHERE latest_episode_at IS NOT (
            SELECT MAX(e.updated_at) FROM episodes e
            WHERE e.anime_id = anime.id AND e.missing = 0
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
  anilist_status_fetched_at INTEGER,
  tracker_offset INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  last_watched_at TEXT,
  latest_episode_at TEXT,
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
        assert_eq!(rule_count(&conn), 5);

        conn.execute("DELETE FROM regex_rules WHERE id = 1", []).unwrap();
        seed_defaults(&conn).unwrap();
        assert_eq!(rule_count(&conn), 4);
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
        assert_eq!(rule_count(&conn), 5);
    }

    fn rule_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM regex_rules", [], |row| row.get(0))
            .unwrap()
    }
}
