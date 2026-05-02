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
        db.migrate()?;
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

    fn migrate(&self) -> Result<(), String> {
        self.with_conn(|conn| {
            conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
            apply_schema_updates(conn)?;
            seed_defaults(conn)?;
            Ok(())
        })
    }
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

    conn.execute(
        "INSERT OR IGNORE INTO regex_rules
            (id, name, detection_regex, title_regex, enabled, priority)
         VALUES
            (1, 'Fansub release filename',
             '(?i)^\\[[^\\]]+\\]\\s*.+\\s+-\\s+\\d+(?:\\.\\d+)?(?:v\\d+)?\\s*(?:\\([^)]+\\))?\\s*(?:\\[[a-f0-9]{8}\\])?\\.[^.]+$',
             '^\\[[^\\]]+\\]\\s*(?P<title>.+?)\\s+-\\s*(?P<episode>\\d+(?:\\.\\d+)?)(?:v\\d+)?(?:\\s|\\(|\\[|\\.)',
             1, 0)",
        [],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

fn apply_schema_updates(conn: &Connection) -> Result<(), String> {
    ensure_column(conn, "anime", "anilist_id", "INTEGER")?;
    ensure_column(conn, "anime", "anilist_title", "TEXT")?;
    ensure_column(conn, "anime", "anilist_site_url", "TEXT")?;
    ensure_column(conn, "anime", "anilist_cover_path", "TEXT")?;
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_anime_anilist_id
         ON anime(anilist_id)
         WHERE anilist_id IS NOT NULL",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| e.to_string())?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if columns.iter().any(|existing| existing == column) {
        return Ok(());
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY
);

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
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  last_watched_at TEXT,
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
  date_added TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  last_watched_at TEXT,
  FOREIGN KEY(anime_id) REFERENCES anime(id) ON DELETE CASCADE,
  FOREIGN KEY(root_folder_id) REFERENCES root_folders(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_episodes_anime_id ON episodes(anime_id);
CREATE INDEX IF NOT EXISTS idx_episodes_last_watched_at ON episodes(last_watched_at);
CREATE INDEX IF NOT EXISTS idx_anime_category_id ON anime(category_id);
CREATE INDEX IF NOT EXISTS idx_anime_last_watched_at ON anime(last_watched_at);

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
