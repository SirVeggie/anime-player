use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

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
pub struct MissingAnimeSummary {
    id: i64,
    title: String,
    category_id: i64,
    anilist_id: Option<i64>,
    anilist_title: Option<String>,
    anilist_site_url: Option<String>,
    anilist_cover_path: Option<String>,
    episode_count: i64,
    unwatched_count: i64,
    missing_episode_count: i64,
    total_episode_count: i64,
    last_watched_at: Option<String>,
    created_at: String,
    latest_episode_at: Option<String>,
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
    missing_anime: Vec<MissingAnimeSummary>,
    unmatched_count: i64,
}

#[derive(Debug, Serialize)]
pub struct ScanSummary {
    roots_scanned: i64,
    episodes_imported: i64,
    episodes_removed: i64,
    unmatched_files: i64,
}

#[derive(Debug, Serialize)]
pub struct LocalDataStats {
    database_bytes: u64,
    thumbnails_bytes: u64,
    total_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct LocalDataCleanupSummary {
    roots_scanned: i64,
    stale_episodes_removed: i64,
    empty_anime_removed: i64,
    unmatched_files_removed: i64,
    thumbnails_removed: i64,
    bytes_removed: u64,
}

#[derive(Debug, Serialize)]
pub struct DeleteAnimeFilesSummary {
    episodes_deleted: i64,
    episodes_failed: i64,
    bytes_deleted: u64,
    cover_deleted: bool,
    cover_failed: bool,
    permanent_delete_used: bool,
}

#[derive(Debug)]
struct DeletableEpisode {
    id: i64,
    path: String,
    size: i64,
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
            missing_anime: list_missing_anime(conn)?,
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

fn delete_anime_with_no_episodes(conn: &Connection) -> Result<usize, String> {
    conn.execute(
        "DELETE FROM anime WHERE NOT EXISTS (
            SELECT 1 FROM episodes e WHERE e.anime_id = anime.id
        )",
        [],
    )
    .map_err(|e| e.to_string())
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

#[tauri::command]
pub fn delete_anime_files(
    db: State<'_, AppDatabase>,
    anime_id: i64,
) -> Result<DeleteAnimeFilesSummary, String> {
    let (episodes, cover_path) = db.with_conn(|conn| {
        let episodes = list_deletable_episodes_for_anime(conn, anime_id)?;
        let cover_path = conn
            .query_row(
                "SELECT anilist_cover_path FROM anime WHERE id = ?1",
                params![anime_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .flatten();
        Ok((episodes, cover_path))
    })?;

    let mut deleted_episode_ids = Vec::new();
    let mut bytes_deleted = 0_u64;
    let mut episodes_failed = 0_i64;
    let mut permanent_delete_used = false;

    for episode in episodes {
        let path = PathBuf::from(&episode.path);
        if !path.exists() {
            deleted_episode_ids.push(episode.id);
            continue;
        }

        let size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or_else(|_| episode.size.max(0) as u64);
        match move_path_to_trash_or_delete(&path) {
            Ok(permanent) => {
                deleted_episode_ids.push(episode.id);
                bytes_deleted += size;
                permanent_delete_used |= permanent;
            }
            Err(_) => {
                episodes_failed += 1;
            }
        }
    }

    let mut cover_deleted = false;
    let mut cover_failed = false;
    let mut clear_cover_path = false;
    if let Some(cover_path) = cover_path {
        let path = PathBuf::from(&cover_path);
        if path.exists() {
            let size = fs::metadata(&path)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            match move_path_to_trash_or_delete(&path) {
                Ok(permanent) => {
                    bytes_deleted += size;
                    cover_deleted = true;
                    clear_cover_path = true;
                    permanent_delete_used |= permanent;
                }
                Err(_) => {
                    cover_failed = true;
                }
            }
        } else {
            clear_cover_path = true;
        }
    }

    db.with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for episode_id in &deleted_episode_ids {
            tx.execute(
                "UPDATE episodes
                 SET missing = 1,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?1",
                params![episode_id],
            )
            .map_err(|e| e.to_string())?;
        }
        if clear_cover_path {
            tx.execute(
                "UPDATE anime
                 SET anilist_cover_path = NULL,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?1",
                params![anime_id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        refresh_anime_latest_episode_at(conn)?;
        Ok(())
    })?;

    Ok(DeleteAnimeFilesSummary {
        episodes_deleted: deleted_episode_ids.len() as i64,
        episodes_failed,
        bytes_deleted,
        cover_deleted,
        cover_failed,
        permanent_delete_used,
    })
}

#[tauri::command]
pub fn open_anime_episode_folder(
    app: AppHandle,
    db: State<'_, AppDatabase>,
    anime_id: i64,
) -> Result<(), String> {
    let folder = db.with_conn(|conn| shortest_episode_folder_for_anime(conn, anime_id))?;
    let Some(folder) = folder else {
        return Err("No episode folder is available.".to_string());
    };
    app.opener()
        .open_path(folder.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
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
            summary.unmatched_files += scan.unmatched.len() as i64;

            let tx = conn.transaction().map_err(|e| e.to_string())?;
            let mut anime_cache: HashMap<String, i64> = HashMap::new();

            mark_episodes_missing_not_in_scan(&tx, root.id, &scan.episodes)?;
            delete_unmatched_files_now_matched(&tx, root.id, &scan.episodes)?;

            for episode in scan.episodes {
                let anime_id = cached_anime_id(
                    &tx,
                    &mut anime_cache,
                    &episode.title,
                    &episode.title_key,
                    default_category,
                )?;
                if upsert_episode(&tx, root.id, anime_id, &episode)? {
                    summary.episodes_imported += 1;
                }
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
                        detected_at = CURRENT_TIMESTAMP
                     WHERE unmatched_files.root_folder_id IS NOT excluded.root_folder_id
                        OR unmatched_files.relative_path IS NOT excluded.relative_path
                        OR unmatched_files.file_name IS NOT excluded.file_name
                        OR unmatched_files.reason IS NOT excluded.reason",
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

#[tauri::command]
pub fn get_local_data_stats(db: State<'_, AppDatabase>) -> Result<LocalDataStats, String> {
    local_data_stats(&db)
}

#[tauri::command]
pub fn clean_local_data(db: State<'_, AppDatabase>) -> Result<LocalDataCleanupSummary, String> {
    let database_bytes_before = fs::metadata(db.path()).map(|m| m.len()).unwrap_or(0);
    let mut summary = db.with_conn(|conn| {
        let roots = list_root_folders(conn)?;
        let rules = list_enabled_detection_rules(conn)?;
        let mut summary = LocalDataCleanupSummary {
            roots_scanned: 0,
            stale_episodes_removed: 0,
            empty_anime_removed: 0,
            unmatched_files_removed: 0,
            thumbnails_removed: 0,
            bytes_removed: 0,
        };

        for root in roots {
            let scan = scanner::scan_root(Path::new(&root.path), &rules)?;
            summary.roots_scanned += 1;
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            summary.stale_episodes_removed +=
                delete_episodes_not_in_scan(&tx, root.id, &scan.episodes)? as i64;
            summary.unmatched_files_removed += delete_unmatched_files_not_in_scan(
                &tx,
                root.id,
                &scan.unmatched,
            )? as i64;
            tx.commit().map_err(|e| e.to_string())?;
        }

        summary.stale_episodes_removed += conn
            .execute("DELETE FROM episodes WHERE root_folder_id IS NULL", [])
            .map_err(|e| e.to_string())? as i64;
        summary.empty_anime_removed = delete_anime_with_no_episodes(conn)? as i64;
        refresh_anime_latest_episode_at(conn)?;
        conn.execute_batch("VACUUM;").map_err(|e| e.to_string())?;
        Ok(summary)
    })?;

    let database_bytes_after = fs::metadata(db.path()).map(|m| m.len()).unwrap_or(0);
    let (removed, thumbnail_bytes_removed) = delete_unreferenced_thumbnails(&db)?;
    summary.thumbnails_removed = removed as i64;
    summary.bytes_removed =
        database_bytes_before.saturating_sub(database_bytes_after) + thumbnail_bytes_removed;
    Ok(summary)
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
             ORDER BY priority DESC, id",
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
             ORDER BY priority DESC, id",
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
             ORDER BY priority DESC, id",
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
               AND missing = 0
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
                 WHERE e2.anime_id = a.id AND e2.missing = 0
                 ORDER BY e2.episode_number IS NULL, e2.episode_number, e2.relative_path COLLATE NOCASE
                 LIMIT 1) AS first_episode_path
         FROM anime a
         LEFT JOIN episodes e ON e.anime_id = a.id AND e.missing = 0",
    );
    if category_id.is_some() {
        sql.push_str(
            " WHERE a.category_id = ?1
              AND EXISTS (SELECT 1 FROM episodes ae WHERE ae.anime_id = a.id AND ae.missing = 0)",
        );
    } else if recent_only {
        sql.push_str(
            " WHERE a.last_watched_at IS NOT NULL
              AND EXISTS (SELECT 1 FROM episodes ae WHERE ae.anime_id = a.id AND ae.missing = 0)",
        );
    } else {
        sql.push_str(
            " WHERE EXISTS (SELECT 1 FROM episodes ae WHERE ae.anime_id = a.id AND ae.missing = 0)",
        );
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

fn list_missing_anime(conn: &Connection) -> Result<Vec<MissingAnimeSummary>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id,
                    a.title,
                    a.category_id,
                    a.anilist_id,
                    a.anilist_title,
                    a.anilist_site_url,
                    a.anilist_cover_path,
                    SUM(CASE WHEN e.missing = 0 THEN 1 ELSE 0 END) AS available_count,
                    SUM(CASE WHEN e.missing = 0 AND e.watched = 0 THEN 1 ELSE 0 END) AS unwatched_count,
                    SUM(CASE WHEN e.missing != 0 THEN 1 ELSE 0 END) AS missing_count,
                    COUNT(e.id) AS total_count,
                    a.last_watched_at,
                    a.created_at,
                    a.latest_episode_at,
                    (SELECT e2.path FROM episodes e2
                     WHERE e2.anime_id = a.id
                     ORDER BY e2.missing, e2.episode_number IS NULL, e2.episode_number, e2.relative_path COLLATE NOCASE
                     LIMIT 1) AS first_episode_path
             FROM anime a
             JOIN episodes e ON e.anime_id = a.id
             GROUP BY a.id
             HAVING SUM(CASE WHEN e.missing != 0 THEN 1 ELSE 0 END) > 0
             ORDER BY a.title COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(MissingAnimeSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                category_id: row.get(2)?,
                anilist_id: row.get(3)?,
                anilist_title: row.get(4)?,
                anilist_site_url: row.get(5)?,
                anilist_cover_path: row.get(6)?,
                episode_count: row.get::<_, Option<i64>>(7)?.unwrap_or(0),
                unwatched_count: row.get::<_, Option<i64>>(8)?.unwrap_or(0),
                missing_episode_count: row.get::<_, Option<i64>>(9)?.unwrap_or(0),
                total_episode_count: row.get(10)?,
                last_watched_at: row.get(11)?,
                created_at: row.get(12)?,
                latest_episode_at: row.get(13)?,
                first_episode_path: row.get(14)?,
            })
        })
        .map_err(|e| e.to_string())?;
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
               AND missing = 0
             ORDER BY episode_number IS NULL, episode_number, relative_path COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![anime_id], episode_from_row)
        .map_err(|e| e.to_string())?;
    collect_rows(rows)
}

fn list_deletable_episodes_for_anime(
    conn: &Connection,
    anime_id: i64,
) -> Result<Vec<DeletableEpisode>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path, size
             FROM episodes
             WHERE anime_id = ?1
               AND missing = 0
             ORDER BY episode_number IS NULL, episode_number, relative_path COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![anime_id], |row| {
            Ok(DeletableEpisode {
                id: row.get(0)?,
                path: row.get(1)?,
                size: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    collect_rows(rows)
}

fn shortest_episode_folder_for_anime(
    conn: &Connection,
    anime_id: i64,
) -> Result<Option<PathBuf>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT path
             FROM episodes
             WHERE anime_id = ?1
               AND missing = 0",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![anime_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;

    let mut shortest: Option<PathBuf> = None;
    for row in rows {
        let path = PathBuf::from(row.map_err(|e| e.to_string())?);
        let Some(parent) = path.parent() else {
            continue;
        };
        let parent = parent.to_path_buf();
        let replace = shortest.as_ref().is_none_or(|current| {
            let parent_len = parent.as_os_str().len();
            let current_len = current.as_os_str().len();
            parent_len < current_len
                || (parent_len == current_len
                    && parent.to_string_lossy() < current.to_string_lossy())
        });
        if replace {
            shortest = Some(parent);
        }
    }

    Ok(shortest)
}

fn move_path_to_trash_or_delete(path: &Path) -> Result<bool, String> {
    match trash::delete(path) {
        Ok(()) => Ok(false),
        Err(trash_error) => {
            fs::remove_file(path).map_err(|remove_error| {
                format!(
                    "failed to move {} to trash ({trash_error}) or delete it permanently ({remove_error})",
                    path.display()
                )
            })?;
            Ok(true)
        }
    }
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
        "UPDATE anime
         SET title = ?1,
             updated_at = CURRENT_TIMESTAMP
         WHERE title_key = ?2
           AND title IS NOT ?1",
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

fn mark_episodes_missing_not_in_scan(
    tx: &rusqlite::Transaction<'_>,
    root_folder_id: i64,
    matched: &[scanner::ScannedEpisode],
) -> Result<usize, String> {
    tx.execute("DROP TABLE IF EXISTS rescan_matched_paths", [])
        .map_err(|e| e.to_string())?;
    tx.execute(
        "CREATE TEMP TABLE rescan_matched_paths (path TEXT PRIMARY KEY)",
        [],
    )
    .map_err(|e| e.to_string())?;
    for ep in matched {
        tx.execute(
            "INSERT INTO rescan_matched_paths (path) VALUES (?1)",
            params![ep.path],
        )
        .map_err(|e| e.to_string())?;
    }
    let marked = tx
        .execute(
            "UPDATE episodes
             SET missing = 1,
                 updated_at = CURRENT_TIMESTAMP
             WHERE root_folder_id = ?1
               AND missing = 0
               AND NOT EXISTS (
                   SELECT 1 FROM rescan_matched_paths k WHERE k.path = episodes.path
               )",
            params![root_folder_id],
        )
        .map_err(|e| e.to_string())?;
    Ok(marked)
}

fn delete_unmatched_files_now_matched(
    tx: &rusqlite::Transaction<'_>,
    root_folder_id: i64,
    matched: &[scanner::ScannedEpisode],
) -> Result<usize, String> {
    ensure_rescan_matched_paths(tx, matched)?;
    let removed = tx
        .execute(
            "DELETE FROM unmatched_files
             WHERE root_folder_id = ?1
             AND EXISTS (
                 SELECT 1 FROM rescan_matched_paths k WHERE k.path = unmatched_files.path
             )",
            params![root_folder_id],
        )
        .map_err(|e| e.to_string())?;
    Ok(removed)
}

fn ensure_rescan_matched_paths(
    tx: &rusqlite::Transaction<'_>,
    matched: &[scanner::ScannedEpisode],
) -> Result<(), String> {
    tx.execute("DROP TABLE IF EXISTS rescan_matched_paths", [])
        .map_err(|e| e.to_string())?;
    tx.execute(
        "CREATE TEMP TABLE rescan_matched_paths (path TEXT PRIMARY KEY)",
        [],
    )
    .map_err(|e| e.to_string())?;
    for ep in matched {
        tx.execute(
            "INSERT INTO rescan_matched_paths (path) VALUES (?1)",
            params![ep.path],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn delete_unmatched_files_not_in_scan(
    tx: &rusqlite::Transaction<'_>,
    root_folder_id: i64,
    kept: &[scanner::UnmatchedFile],
) -> Result<usize, String> {
    tx.execute("DROP TABLE IF EXISTS cleanup_keep_unmatched_paths", [])
        .map_err(|e| e.to_string())?;
    tx.execute(
        "CREATE TEMP TABLE cleanup_keep_unmatched_paths (path TEXT PRIMARY KEY)",
        [],
    )
    .map_err(|e| e.to_string())?;
    for file in kept {
        tx.execute(
            "INSERT INTO cleanup_keep_unmatched_paths (path) VALUES (?1)",
            params![file.path],
        )
        .map_err(|e| e.to_string())?;
    }
    let removed = tx
        .execute(
            "DELETE FROM unmatched_files
             WHERE root_folder_id = ?1
             AND NOT EXISTS (
                 SELECT 1 FROM cleanup_keep_unmatched_paths k WHERE k.path = unmatched_files.path
             )",
            params![root_folder_id],
        )
        .map_err(|e| e.to_string())?;
    Ok(removed)
}

fn local_data_stats(db: &AppDatabase) -> Result<LocalDataStats, String> {
    let database_bytes = fs::metadata(db.path()).map(|m| m.len()).unwrap_or(0);
    let thumbnails_bytes = db
        .path()
        .parent()
        .map(|data_dir| directory_size(&data_dir.join("anilist-covers")))
        .transpose()?
        .unwrap_or(0);
    Ok(LocalDataStats {
        database_bytes,
        thumbnails_bytes,
        total_bytes: database_bytes + thumbnails_bytes,
    })
}

fn directory_size(path: &Path) -> Result<u64, String> {
    if !path.exists() {
        return Ok(0);
    }

    let mut total = 0_u64;
    let entries = fs::read_dir(path)
        .map_err(|e| format!("failed to read directory {}: {e}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let metadata = entry.metadata().map_err(|e| e.to_string())?;
        if metadata.is_dir() {
            total += directory_size(&entry.path())?;
        } else {
            total += metadata.len();
        }
    }
    Ok(total)
}

fn delete_unreferenced_thumbnails(db: &AppDatabase) -> Result<(usize, u64), String> {
    let Some(data_dir) = db.path().parent() else {
        return Ok((0, 0));
    };
    let cover_dir = data_dir.join("anilist-covers");
    if !cover_dir.exists() {
        return Ok((0, 0));
    }

    let referenced_paths = db.with_conn(list_referenced_thumbnail_paths)?;
    let mut removed_count = 0_usize;
    let mut removed_bytes = 0_u64;
    for file in list_files_recursive(&cover_dir)? {
        if referenced_paths.contains(&file) {
            continue;
        }
        let size = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        fs::remove_file(&file)
            .map_err(|e| format!("failed to remove thumbnail {}: {e}", file.display()))?;
        removed_count += 1;
        removed_bytes += size;
    }

    Ok((removed_count, removed_bytes))
}

fn list_referenced_thumbnail_paths(conn: &mut Connection) -> Result<HashSet<PathBuf>, String> {
    let mut stmt = conn
        .prepare("SELECT anilist_cover_path FROM anime WHERE anilist_cover_path IS NOT NULL")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let paths = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(PathBuf::from)
        .collect();
    Ok(paths)
}

fn list_files_recursive(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if !path.exists() {
        return Ok(files);
    }

    let entries = fs::read_dir(path)
        .map_err(|e| format!("failed to read directory {}: {e}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let entry_path = entry.path();
        let metadata = entry.metadata().map_err(|e| e.to_string())?;
        if metadata.is_dir() {
            files.extend(list_files_recursive(&entry_path)?);
        } else {
            files.push(entry_path);
        }
    }
    Ok(files)
}

fn upsert_episode(
    conn: &Connection,
    root_folder_id: i64,
    anime_id: i64,
    episode: &scanner::ScannedEpisode,
) -> Result<bool, String> {
    let changed = conn.execute(
        "INSERT INTO episodes
            (anime_id, root_folder_id, path, relative_path, file_name, file_type,
             episode_number, size, missing)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0)
         ON CONFLICT(path) DO UPDATE SET
            anime_id = excluded.anime_id,
            root_folder_id = excluded.root_folder_id,
            relative_path = excluded.relative_path,
            file_name = excluded.file_name,
            file_type = excluded.file_type,
            episode_number = excluded.episode_number,
            size = excluded.size,
            missing = 0,
            updated_at = CURRENT_TIMESTAMP
         WHERE episodes.anime_id IS NOT excluded.anime_id
            OR episodes.root_folder_id IS NOT excluded.root_folder_id
            OR episodes.relative_path IS NOT excluded.relative_path
            OR episodes.file_name IS NOT excluded.file_name
            OR episodes.file_type IS NOT excluded.file_type
            OR episodes.episode_number IS NOT excluded.episode_number
            OR episodes.size IS NOT excluded.size
            OR episodes.missing != 0",
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
    Ok(changed > 0)
}

fn collect_rows<T>(rows: impl Iterator<Item = rusqlite::Result<T>>) -> Result<Vec<T>, String> {
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}
