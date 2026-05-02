use std::collections::HashMap;
use std::path::Path;

use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::{refresh_anime_latest_episode_at, AppDatabase};
use crate::scanner::{self, DetectionRule};

/// Saved positions below this are stored as 0 (avoid sticky resume after brief opens).
const MIN_POSITION_SECONDS_TO_PERSIST: f64 = 60.0;

#[derive(Debug, Serialize)]
pub struct VideoFile {
    path: String,
    name: String,
    relative_path: String,
    size: u64,
}

#[derive(Debug, Serialize)]
pub struct RootFolder {
    id: i64,
    path: String,
}

#[derive(Debug, Serialize)]
pub struct RegexRule {
    id: i64,
    name: String,
    detection_regex: String,
    title_regex: String,
    enabled: bool,
    priority: i64,
}

#[derive(Debug, Serialize)]
pub struct Category {
    id: i64,
    name: String,
    is_default: bool,
    sort_order: i64,
}

#[derive(Debug, Serialize)]
pub struct AnimeSummary {
    id: i64,
    title: String,
    category_id: i64,
    anilist_id: Option<i64>,
    anilist_title: Option<String>,
    anilist_site_url: Option<String>,
    anilist_cover_path: Option<String>,
    episode_count: i64,
    unwatched_count: i64,
    last_watched_at: Option<String>,
    created_at: String,
    /// Latest `episodes.updated_at` for this anime (refreshed on rescan); used for "Most recent" sort.
    latest_episode_at: Option<String>,
    /// Path of the first episode in list order (same as `list_episodes`); grid thumbnail fallback.
    first_episode_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Episode {
    id: i64,
    anime_id: i64,
    path: String,
    relative_path: String,
    file_name: String,
    file_type: String,
    episode_number: Option<f64>,
    size: i64,
    duration_seconds: f64,
    position_seconds: f64,
    watched: bool,
    last_watched_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LibraryState {
    db_path: String,
    root_folders: Vec<RootFolder>,
    regex_rules: Vec<RegexRule>,
    categories: Vec<Category>,
    anime: Vec<AnimeSummary>,
    recent_anime: Vec<AnimeSummary>,
    unmatched_count: i64,
}

#[derive(Debug, Serialize)]
pub struct ScanSummary {
    roots_scanned: i64,
    episodes_imported: i64,
    episodes_removed: i64,
    unmatched_files: i64,
}

#[derive(Debug, Deserialize)]
pub struct RegexRuleInput {
    name: String,
    detection_regex: String,
    title_regex: String,
    enabled: bool,
    priority: i64,
}

#[tauri::command]
pub fn scan_videos(folder: String) -> Result<Vec<VideoFile>, String> {
    let root = Path::new(&folder);
    if !root.exists() {
        return Err(format!("Folder does not exist: {folder}"));
    }
    if !root.is_dir() {
        return Err(format!("Path is not a directory: {folder}"));
    }

    let mut results: Vec<VideoFile> = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !entry.file_type().is_file() || !scanner::is_video_file(path) {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let relative_path = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

        results.push(VideoFile {
            path: path.to_string_lossy().to_string(),
            name,
            relative_path,
            size,
        });
    }

    results.sort_by(|a, b| {
        a.relative_path
            .to_lowercase()
            .cmp(&b.relative_path.to_lowercase())
    });
    Ok(results)
}

#[tauri::command]
pub fn get_library_state(db: State<'_, AppDatabase>) -> Result<LibraryState, String> {
    db.with_conn(|conn| {
        Ok(LibraryState {
            db_path: db.path().to_string_lossy().to_string(),
            root_folders: list_root_folders(conn)?,
            regex_rules: list_regex_rules(conn)?,
            categories: list_categories(conn)?,
            anime: list_anime(conn, None, false)?,
            recent_anime: list_anime(conn, None, true)?,
            unmatched_count: count_unmatched(conn)?,
        })
    })
}

#[tauri::command]
pub fn add_root_folder(db: State<'_, AppDatabase>, path: String) -> Result<RootFolder, String> {
    let root = Path::new(&path);
    if !root.exists() {
        return Err(format!("Folder does not exist: {path}"));
    }
    if !root.is_dir() {
        return Err(format!("Path is not a directory: {path}"));
    }

    db.with_conn(|conn| {
        conn.execute(
            "INSERT OR IGNORE INTO root_folders (path) VALUES (?1)",
            params![path],
        )
        .map_err(|e| e.to_string())?;
        get_root_folder_by_path(conn, &path)
    })
}

#[tauri::command]
pub fn remove_root_folder(db: State<'_, AppDatabase>, id: i64) -> Result<(), String> {
    db.with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        // Episodes FK uses ON DELETE SET NULL; deleting the root would orphan rows that
        // rescan never prunes (delete_episodes_not_in_scan filters by root_folder_id).
        tx.execute(
            "DELETE FROM episodes WHERE root_folder_id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        delete_anime_with_no_episodes(&tx)?;
        tx.execute("DELETE FROM root_folders WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        refresh_anime_latest_episode_at(conn)?;
        Ok(())
    })
}

fn delete_anime_with_no_episodes(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "DELETE FROM anime WHERE NOT EXISTS (
            SELECT 1 FROM episodes e WHERE e.anime_id = anime.id
        )",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn create_category(db: State<'_, AppDatabase>, name: String) -> Result<Category, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Category name cannot be empty.".to_string());
    }

    db.with_conn(|conn| {
        let next_sort = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM categories",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO categories (name, sort_order) VALUES (?1, ?2)",
            params![trimmed, next_sort],
        )
        .map_err(|e| e.to_string())?;
        get_category(conn, conn.last_insert_rowid())
    })
}

#[tauri::command]
pub fn delete_category(db: State<'_, AppDatabase>, id: i64) -> Result<(), String> {
    db.with_conn(|conn| {
        let default_id = default_category_id(conn)?;
        if id == default_id {
            return Err("Default category cannot be deleted.".to_string());
        }
        conn.execute(
            "UPDATE anime SET category_id = ?1 WHERE category_id = ?2",
            params![default_id, id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM categories WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn set_default_category(db: State<'_, AppDatabase>, id: i64) -> Result<Category, String> {
    db.with_conn(|conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let exists: Option<i64> = tx
            .query_row(
                "SELECT id FROM categories WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if exists.is_none() {
            return Err(format!("Category does not exist: {id}"));
        }
        tx.execute("UPDATE categories SET is_default = 0", [])
            .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE categories SET is_default = 1 WHERE id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        get_category(conn, id)
    })
}

#[tauri::command]
pub fn create_regex_rule(
    db: State<'_, AppDatabase>,
    input: RegexRuleInput,
) -> Result<RegexRule, String> {
    validate_regex_rule_input(&input)?;
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO regex_rules
                (name, detection_regex, title_regex, enabled, priority)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                input.name.trim(),
                input.detection_regex.trim(),
                input.title_regex.trim(),
                if input.enabled { 1 } else { 0 },
                input.priority
            ],
        )
        .map_err(|e| e.to_string())?;
        get_regex_rule(conn, conn.last_insert_rowid())
    })
}

#[tauri::command]
pub fn update_regex_rule(
    db: State<'_, AppDatabase>,
    id: i64,
    input: RegexRuleInput,
) -> Result<RegexRule, String> {
    validate_regex_rule_input(&input)?;
    db.with_conn(|conn| {
        let changed = conn
            .execute(
                "UPDATE regex_rules
                 SET name = ?1,
                     detection_regex = ?2,
                     title_regex = ?3,
                     enabled = ?4,
                     priority = ?5
                 WHERE id = ?6",
                params![
                    input.name.trim(),
                    input.detection_regex.trim(),
                    input.title_regex.trim(),
                    if input.enabled { 1 } else { 0 },
                    input.priority,
                    id
                ],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("Detection rule does not exist: {id}"));
        }
        get_regex_rule(conn, id)
    })
}

#[tauri::command]
pub fn delete_regex_rule(db: State<'_, AppDatabase>, id: i64) -> Result<(), String> {
    db.with_conn(|conn| {
        conn.execute("DELETE FROM regex_rules WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn move_anime_to_category(
    db: State<'_, AppDatabase>,
    anime_id: i64,
    category_id: i64,
) -> Result<(), String> {
    db.with_conn(|conn| {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM categories WHERE id = ?1",
                params![category_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if exists.is_none() {
            return Err(format!("Category does not exist: {category_id}"));
        }
        conn.execute(
            "UPDATE anime SET category_id = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            params![category_id, anime_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn list_episodes(db: State<'_, AppDatabase>, anime_id: i64) -> Result<Vec<Episode>, String> {
    db.with_conn(|conn| list_episodes_for_anime(conn, anime_id))
}

/// Resolves which enabled detection rule matches this anime's episode files, using the same
/// rule order and filename logic as a rescan — computed on demand, not stored in the DB.
#[tauri::command]
pub fn get_matching_detection_rule_name(
    db: State<'_, AppDatabase>,
    anime_id: i64,
) -> Result<Option<String>, String> {
    db.with_conn(|conn| {
        let rules = list_enabled_detection_rules_named(conn)?;
        if rules.is_empty() {
            return Ok(None);
        }
        let file_names = list_episode_file_names_for_anime(conn, anime_id)?;
        for file_name in file_names {
            if let Some(name) = scanner::match_rule_name_for_file_name(&file_name, &rules)? {
                return Ok(Some(name));
            }
        }
        Ok(None)
    })
}

#[tauri::command]
pub fn save_episode_progress(
    db: State<'_, AppDatabase>,
    episode_id: i64,
    position_seconds: f64,
    duration_seconds: f64,
    watched: bool,
) -> Result<Episode, String> {
    db.with_conn(|conn| {
        let watched_flag = if watched { 1 } else { 0 };
        let duration_seconds = duration_seconds.max(0.0);
        let mut position_seconds = position_seconds.max(0.0);
        if watched {
            position_seconds = duration_seconds;
        } else if position_seconds < MIN_POSITION_SECONDS_TO_PERSIST {
            position_seconds = 0.0;
        }
        conn.execute(
            "UPDATE episodes
             SET position_seconds = ?1,
                 duration_seconds = ?2,
                 watched = ?3,
                 last_watched_at = CURRENT_TIMESTAMP,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?4",
            params![position_seconds, duration_seconds, watched_flag, episode_id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE anime
             SET last_watched_at = CURRENT_TIMESTAMP,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = (SELECT anime_id FROM episodes WHERE id = ?1)",
            params![episode_id],
        )
        .map_err(|e| e.to_string())?;
        get_episode(conn, episode_id)
    })
}

#[tauri::command]
pub fn rescan_library(db: State<'_, AppDatabase>) -> Result<ScanSummary, String> {
    db.with_conn(|conn| {
        // Repair DBs from before remove_root_folder deleted episodes: SET NULL orphans.
        conn.execute("DELETE FROM episodes WHERE root_folder_id IS NULL", [])
            .map_err(|e| e.to_string())?;
        delete_anime_with_no_episodes(conn)?;
        refresh_anime_latest_episode_at(conn)?;

        let roots = list_root_folders(conn)?;
        let rules = list_enabled_detection_rules(conn)?;
        let default_category = default_category_id(conn)?;

        let mut summary = ScanSummary {
            roots_scanned: 0,
            episodes_imported: 0,
            episodes_removed: 0,
            unmatched_files: 0,
        };

        for root in roots {
            let scan = scanner::scan_root(Path::new(&root.path), &rules)?;
            summary.roots_scanned += 1;
            summary.episodes_imported += scan.episodes.len() as i64;
            summary.unmatched_files += scan.unmatched.len() as i64;

            let tx = conn.transaction().map_err(|e| e.to_string())?;
            let mut anime_cache: HashMap<String, i64> = HashMap::new();

            let removed = delete_episodes_not_in_scan(&tx, root.id, &scan.episodes)?;
            summary.episodes_removed += removed as i64;

            tx.execute(
                "DELETE FROM unmatched_files WHERE root_folder_id = ?1",
                params![root.id],
            )
            .map_err(|e| e.to_string())?;

            for episode in scan.episodes {
                let anime_id = cached_anime_id(
                    &tx,
                    &mut anime_cache,
                    &episode.title,
                    &episode.title_key,
                    default_category,
                )?;
                upsert_episode(&tx, root.id, anime_id, &episode)?;
            }

            for file in scan.unmatched {
                tx.execute(
                    "INSERT INTO unmatched_files
                        (root_folder_id, path, relative_path, file_name, reason, detected_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)
                     ON CONFLICT(path) DO UPDATE SET
                        root_folder_id = excluded.root_folder_id,
                        relative_path = excluded.relative_path,
                        file_name = excluded.file_name,
                        reason = excluded.reason,
                        detected_at = CURRENT_TIMESTAMP",
                    params![
                        root.id,
                        file.path,
                        file.relative_path,
                        file.file_name,
                        file.reason
                    ],
                )
                .map_err(|e| e.to_string())?;
            }

            tx.commit().map_err(|e| e.to_string())?;
        }

        refresh_anime_latest_episode_at(conn)?;

        Ok(summary)
    })
}

fn list_root_folders(conn: &Connection) -> Result<Vec<RootFolder>, String> {
    let mut stmt = conn
        .prepare("SELECT id, path FROM root_folders ORDER BY path COLLATE NOCASE")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(RootFolder {
                id: row.get(0)?,
                path: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?;
    collect_rows(rows)
}

fn get_root_folder_by_path(conn: &Connection, path: &str) -> Result<RootFolder, String> {
    conn.query_row(
        "SELECT id, path FROM root_folders WHERE path = ?1",
        params![path],
        |row| {
            Ok(RootFolder {
                id: row.get(0)?,
                path: row.get(1)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

fn list_regex_rules(conn: &Connection) -> Result<Vec<RegexRule>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, detection_regex, title_regex, enabled, priority
             FROM regex_rules
             ORDER BY priority, id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(RegexRule {
                id: row.get(0)?,
                name: row.get(1)?,
                detection_regex: row.get(2)?,
                title_regex: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
                priority: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    collect_rows(rows)
}

fn get_regex_rule(conn: &Connection, id: i64) -> Result<RegexRule, String> {
    conn.query_row(
        "SELECT id, name, detection_regex, title_regex, enabled, priority
         FROM regex_rules
         WHERE id = ?1",
        params![id],
        |row| {
            Ok(RegexRule {
                id: row.get(0)?,
                name: row.get(1)?,
                detection_regex: row.get(2)?,
                title_regex: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
                priority: row.get(5)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

fn validate_regex_rule_input(input: &RegexRuleInput) -> Result<(), String> {
    if input.name.trim().is_empty() {
        return Err("Detection rule name cannot be empty.".to_string());
    }
    let detection_regex = input.detection_regex.trim();
    let title_regex = input.title_regex.trim();
    if detection_regex.is_empty() || title_regex.is_empty() {
        return Err("Detection and title regexes are required.".to_string());
    }
    Regex::new(detection_regex).map_err(|e| format!("Invalid detection regex: {e}"))?;
    let title = Regex::new(title_regex).map_err(|e| format!("Invalid title regex: {e}"))?;
    if title.capture_names().flatten().all(|name| name != "title") {
        return Err("Title regex must include a named capture group called `title`.".to_string());
    }
    Ok(())
}

fn list_enabled_detection_rules(conn: &Connection) -> Result<Vec<DetectionRule>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT detection_regex, title_regex
             FROM regex_rules
             WHERE enabled = 1
             ORDER BY priority, id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(DetectionRule {
                detection_regex: row.get(0)?,
                title_regex: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?;
    collect_rows(rows)
}

fn list_enabled_detection_rules_named(
    conn: &Connection,
) -> Result<Vec<(String, DetectionRule)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT name, detection_regex, title_regex
             FROM regex_rules
             WHERE enabled = 1
             ORDER BY priority, id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                DetectionRule {
                    detection_regex: row.get(1)?,
                    title_regex: row.get(2)?,
                },
            ))
        })
        .map_err(|e| e.to_string())?;
    collect_rows(rows)
}

fn list_episode_file_names_for_anime(conn: &Connection, anime_id: i64) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT file_name FROM episodes
             WHERE anime_id = ?1
             ORDER BY episode_number IS NULL, episode_number, relative_path COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![anime_id], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    collect_rows(rows)
}

fn list_categories(conn: &Connection) -> Result<Vec<Category>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, is_default, sort_order FROM categories ORDER BY sort_order, name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Category {
                id: row.get(0)?,
                name: row.get(1)?,
                is_default: row.get::<_, i64>(2)? != 0,
                sort_order: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    collect_rows(rows)
}

fn get_category(conn: &Connection, id: i64) -> Result<Category, String> {
    conn.query_row(
        "SELECT id, name, is_default, sort_order FROM categories WHERE id = ?1",
        params![id],
        |row| {
            Ok(Category {
                id: row.get(0)?,
                name: row.get(1)?,
                is_default: row.get::<_, i64>(2)? != 0,
                sort_order: row.get(3)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

fn list_anime(
    conn: &Connection,
    category_id: Option<i64>,
    recent_only: bool,
) -> Result<Vec<AnimeSummary>, String> {
    let mut sql = String::from(
        "SELECT a.id,
                a.title,
                a.category_id,
                a.anilist_id,
                a.anilist_title,
                a.anilist_site_url,
                a.anilist_cover_path,
                COUNT(e.id) AS episode_count,
                SUM(CASE WHEN e.watched = 0 THEN 1 ELSE 0 END) AS unwatched_count,
                a.last_watched_at,
                a.created_at,
                a.latest_episode_at,
                (SELECT e2.path FROM episodes e2
                 WHERE e2.anime_id = a.id
                 ORDER BY e2.episode_number IS NULL, e2.episode_number, e2.relative_path COLLATE NOCASE
                 LIMIT 1) AS first_episode_path
         FROM anime a
         LEFT JOIN episodes e ON e.anime_id = a.id",
    );
    if category_id.is_some() {
        sql.push_str(" WHERE a.category_id = ?1");
    } else if recent_only {
        sql.push_str(" WHERE a.last_watched_at IS NOT NULL");
    }
    sql.push_str(" GROUP BY a.id");
    if recent_only {
        sql.push_str(" ORDER BY a.last_watched_at DESC LIMIT 10");
    } else {
        sql.push_str(" ORDER BY a.title COLLATE NOCASE");
    }

    let map_row = |row: &rusqlite::Row<'_>| {
        Ok(AnimeSummary {
            id: row.get(0)?,
            title: row.get(1)?,
            category_id: row.get(2)?,
            anilist_id: row.get(3)?,
            anilist_title: row.get(4)?,
            anilist_site_url: row.get(5)?,
            anilist_cover_path: row.get(6)?,
            episode_count: row.get(7)?,
            unwatched_count: row.get::<_, Option<i64>>(8)?.unwrap_or(0),
            last_watched_at: row.get(9)?,
            created_at: row.get(10)?,
            latest_episode_at: row.get(11)?,
            first_episode_path: row.get(12)?,
        })
    };

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = if let Some(category_id) = category_id {
        stmt.query_map(params![category_id], map_row)
            .map_err(|e| e.to_string())?
    } else {
        stmt.query_map([], map_row).map_err(|e| e.to_string())?
    };
    collect_rows(rows)
}

fn list_episodes_for_anime(conn: &Connection, anime_id: i64) -> Result<Vec<Episode>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, anime_id, path, relative_path, file_name, file_type,
                    episode_number, size, duration_seconds, position_seconds,
                    watched, last_watched_at
             FROM episodes
             WHERE anime_id = ?1
             ORDER BY episode_number IS NULL, episode_number, relative_path COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![anime_id], episode_from_row)
        .map_err(|e| e.to_string())?;
    collect_rows(rows)
}

fn get_episode(conn: &Connection, episode_id: i64) -> Result<Episode, String> {
    conn.query_row(
        "SELECT id, anime_id, path, relative_path, file_name, file_type,
                episode_number, size, duration_seconds, position_seconds,
                watched, last_watched_at
         FROM episodes
         WHERE id = ?1",
        params![episode_id],
        episode_from_row,
    )
    .map_err(|e| e.to_string())
}

fn episode_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Episode> {
    Ok(Episode {
        id: row.get(0)?,
        anime_id: row.get(1)?,
        path: row.get(2)?,
        relative_path: row.get(3)?,
        file_name: row.get(4)?,
        file_type: row.get(5)?,
        episode_number: row.get(6)?,
        size: row.get(7)?,
        duration_seconds: row.get(8)?,
        position_seconds: row.get(9)?,
        watched: row.get::<_, i64>(10)? != 0,
        last_watched_at: row.get(11)?,
    })
}

fn count_unmatched(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COUNT(*) FROM unmatched_files", [], |row| row.get(0))
        .map_err(|e| e.to_string())
}

fn default_category_id(conn: &Connection) -> Result<i64, String> {
    conn.query_row(
        "SELECT id FROM categories WHERE is_default = 1 ORDER BY id LIMIT 1",
        [],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

fn cached_anime_id(
    conn: &Connection,
    cache: &mut HashMap<String, i64>,
    title: &str,
    title_key: &str,
    default_category_id: i64,
) -> Result<i64, String> {
    if let Some(&id) = cache.get(title_key) {
        return Ok(id);
    }
    let id = upsert_anime(conn, title, title_key, default_category_id)?;
    cache.insert(title_key.to_string(), id);
    Ok(id)
}

fn upsert_anime(
    conn: &Connection,
    title: &str,
    title_key: &str,
    default_category_id: i64,
) -> Result<i64, String> {
    conn.execute(
        "INSERT OR IGNORE INTO anime (title, title_key, category_id)
         VALUES (?1, ?2, ?3)",
        params![title, title_key, default_category_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE anime SET title = ?1, updated_at = CURRENT_TIMESTAMP WHERE title_key = ?2",
        params![title, title_key],
    )
    .map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT id FROM anime WHERE title_key = ?1",
        params![title_key],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

/// Removes episode rows for this root whose files are no longer present in the scan
/// (deleted, moved, or no longer matched by detection rules).
fn delete_episodes_not_in_scan(
    tx: &rusqlite::Transaction<'_>,
    root_folder_id: i64,
    kept: &[scanner::ScannedEpisode],
) -> Result<usize, String> {
    tx.execute("DROP TABLE IF EXISTS rescan_keep_paths", [])
        .map_err(|e| e.to_string())?;
    tx.execute(
        "CREATE TEMP TABLE rescan_keep_paths (path TEXT PRIMARY KEY)",
        [],
    )
    .map_err(|e| e.to_string())?;
    for ep in kept {
        tx.execute(
            "INSERT INTO rescan_keep_paths (path) VALUES (?1)",
            params![ep.path],
        )
        .map_err(|e| e.to_string())?;
    }
    let removed = tx
        .execute(
            "DELETE FROM episodes
             WHERE root_folder_id = ?1
             AND NOT EXISTS (
                 SELECT 1 FROM rescan_keep_paths k WHERE k.path = episodes.path
             )",
            params![root_folder_id],
        )
        .map_err(|e| e.to_string())?;
    Ok(removed)
}

fn upsert_episode(
    conn: &Connection,
    root_folder_id: i64,
    anime_id: i64,
    episode: &scanner::ScannedEpisode,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO episodes
            (anime_id, root_folder_id, path, relative_path, file_name, file_type,
             episode_number, size)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(path) DO UPDATE SET
            anime_id = excluded.anime_id,
            root_folder_id = excluded.root_folder_id,
            relative_path = excluded.relative_path,
            file_name = excluded.file_name,
            file_type = excluded.file_type,
            episode_number = excluded.episode_number,
            size = excluded.size,
            updated_at = CURRENT_TIMESTAMP",
        params![
            anime_id,
            root_folder_id,
            episode.path,
            episode.relative_path,
            episode.file_name,
            episode.file_type,
            episode.episode_number,
            episode.size
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn collect_rows<T>(rows: impl Iterator<Item = rusqlite::Result<T>>) -> Result<Vec<T>, String> {
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}
