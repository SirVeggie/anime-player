use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::db::{refresh_anime_latest_episode_at, AppDatabase};
use crate::op_ed::{self, OpEdSegmentInfo};
use crate::scanner::{self, DetectionRule};

#[cfg(windows)]
use crate::AppState;

/// Saved positions below this are stored as 0 (avoid sticky resume after brief opens).
const MIN_POSITION_SECONDS_TO_PERSIST: f64 = 60.0;

const PREFER_ANILIST_DISPLAY_TITLE_KEY: &str = "prefer_anilist_display_title";
const HIDE_ANILIST_FEATURES_KEY: &str = "hide_anilist_features";
const CLEAN_UNUSED_SCRUB_SPRITES_KEY: &str = "clean_unused_scrub_sprites";
const LOCAL_DATA_STATS_CACHE_KEY: &str = "local_data_stats_cache";

/// Gaps in the integer episode-number sequence, optionally extended to AniList total.
/// When AniList status is `RELEASING`, trailing unreleased episodes are not counted.
fn compute_gap_episode_count(
    min_int_ep: i64,
    max_int_ep: i64,
    int_ep_count: i64,
    anilist_cached_episodes: Option<i64>,
    anilist_cached_status: Option<&str>,
    tracker_offset: i64,
) -> i64 {
    let extend_to_anilist_total = !matches!(
        anilist_cached_status,
        Some(status) if status.eq_ignore_ascii_case("RELEASING")
    );
    let effective_max = if extend_to_anilist_total {
        match anilist_cached_episodes {
            Some(ae) if ae > 0 => max_int_ep.max(ae + tracker_offset),
            _ => max_int_ep,
        }
    } else {
        max_int_ep
    };
    effective_max - min_int_ep + 1 - int_ep_count
}

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
    custom_thumbnail_path: Option<String>,
    tracker_offset: i64,
    episode_count: i64,
    unwatched_count: i64,
    /// Gaps in the integer episode-number sequence, optionally extended to AniList total.
    gap_episode_count: i64,
    last_watched_at: Option<String>,
    created_at: String,
    /// Latest `episodes.updated_at` for this anime (refreshed on rescan); used for "Most recent" sort.
    latest_episode_at: Option<String>,
    /// Path of the first episode in list order (same as `list_episodes`); grid thumbnail fallback.
    first_episode_path: Option<String>,
    no_op_ed: bool,
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
    custom_thumbnail_path: Option<String>,
    tracker_offset: i64,
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
    op_ed_segments: Vec<OpEdSegmentInfo>,
}

#[derive(Debug, Serialize)]
pub struct AnimeSearchEntry {
    id: i64,
    title: String,
    anilist_title: Option<String>,
    file_names: Vec<String>,
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
    prefer_anilist_display_title: bool,
    hide_anilist_features: bool,
    skip_op_ed: bool,
    auto_op_ed_detect: bool,
    dont_skip_first_episode_op_ed: bool,
    clean_unused_scrub_sprites: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanSummary {
    pub roots_scanned: i64,
    pub episodes_imported: i64,
    pub episodes_removed: i64,
    pub unmatched_files: i64,
}

#[derive(Debug, Serialize)]
pub struct ProgressOverrideSummary {
    progress: i64,
    updated_episodes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalDataStats {
    pub database_bytes: u64,
    pub thumbnails_bytes: u64,
    pub scrub_sprites_bytes: u64,
    pub op_ed_fingerprints_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalDataCleanupSummary {
    pub roots_scanned: i64,
    pub stale_episodes_removed: i64,
    pub empty_anime_removed: i64,
    pub unmatched_files_removed: i64,
    pub thumbnails_removed: i64,
    pub scrub_sprites_removed: i64,
    pub op_ed_fingerprints_removed: i64,
    pub op_ed_temp_pcm_removed: i64,
    pub bytes_removed: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteAnimeFilesSummary {
    pub episodes_deleted: i64,
    pub episodes_failed: i64,
    pub bytes_deleted: u64,
    pub cover_deleted: bool,
    pub cover_failed: bool,
    pub permanent_delete_used: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClearAnimeLocalDataSummary {
    pub episodes_removed: i64,
    pub bytes_removed: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RenameFileRequest {
    old_path: String,
    new_path: String,
}

#[derive(Debug, Serialize)]
pub struct RenameFilesSummary {
    files_renamed: i64,
}

#[derive(Debug, Serialize)]
pub struct RenameAnimeSummary {
    files_renamed: i64,
}

#[derive(Debug)]
struct DeletableEpisode {
    id: i64,
    path: String,
    size: i64,
}

#[derive(Debug)]
struct RenameEpisodePlan {
    old_path: PathBuf,
    old_path_string: String,
    new_path: PathBuf,
    new_path_string: String,
    root_folder: PathBuf,
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
    scan_video_files_in_root(root)
}

#[tauri::command]
pub fn list_root_video_files(db: State<'_, AppDatabase>) -> Result<Vec<VideoFile>, String> {
    let roots = db.with_conn(|conn| list_root_folders(conn))?;
    let mut files = Vec::new();
    for root in roots {
        files.extend(scan_video_files_in_root(Path::new(&root.path))?);
    }
    files.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
    Ok(files)
}

fn scan_video_files_in_root(root: &Path) -> Result<Vec<VideoFile>, String> {
    if !root.exists() {
        return Err(format!("Folder does not exist: {}", root.display()));
    }
    if !root.is_dir() {
        return Err(format!("Path is not a directory: {}", root.display()));
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

fn read_prefer_anilist_display_title(conn: &Connection) -> Result<bool, String> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![PREFER_ANILIST_DISPLAY_TITLE_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(matches!(value.as_deref(), Some("1" | "true" | "yes")))
}

fn write_prefer_anilist_display_title(conn: &Connection, enabled: bool) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![
            PREFER_ANILIST_DISPLAY_TITLE_KEY,
            if enabled { "1" } else { "0" }
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn read_hide_anilist_features(conn: &Connection) -> Result<bool, String> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![HIDE_ANILIST_FEATURES_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(matches!(value.as_deref(), Some("1" | "true" | "yes")))
}

fn write_hide_anilist_features(conn: &Connection, enabled: bool) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![HIDE_ANILIST_FEATURES_KEY, if enabled { "1" } else { "0" }],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn read_clean_unused_scrub_sprites(conn: &Connection) -> Result<bool, String> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![CLEAN_UNUSED_SCRUB_SPRITES_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(match value.as_deref() {
        None => true,
        Some("0" | "false" | "no") => false,
        _ => true,
    })
}

fn write_clean_unused_scrub_sprites(conn: &Connection, enabled: bool) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![
            CLEAN_UNUSED_SCRUB_SPRITES_KEY,
            if enabled { "1" } else { "0" }
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_library_state(db: State<'_, AppDatabase>) -> Result<LibraryState, String> {
    db.with_conn(|conn| build_library_state(conn, &db))
}

#[tauri::command]
pub fn get_anime_search_index(db: State<'_, AppDatabase>) -> Result<Vec<AnimeSearchEntry>, String> {
    db.with_conn(|conn| list_anime_search_index(conn))
}

fn build_library_state(conn: &Connection, db: &AppDatabase) -> Result<LibraryState, String> {
    Ok(LibraryState {
        db_path: db.path().to_string_lossy().to_string(),
        root_folders: list_root_folders(conn)?,
        regex_rules: list_regex_rules(conn)?,
        categories: list_categories(conn)?,
        anime: list_anime(conn, None, false)?,
        recent_anime: list_anime(conn, None, true)?,
        missing_anime: list_missing_anime(conn)?,
        unmatched_count: count_unmatched(conn)?,
        prefer_anilist_display_title: read_prefer_anilist_display_title(conn)?,
        hide_anilist_features: read_hide_anilist_features(conn)?,
        skip_op_ed: op_ed::read_skip_op_ed(conn)?,
        auto_op_ed_detect: op_ed::read_auto_op_ed_detect(conn)?,
        dont_skip_first_episode_op_ed: op_ed::read_dont_skip_first_episode_op_ed(conn)?,
        clean_unused_scrub_sprites: read_clean_unused_scrub_sprites(conn)?,
    })
}

#[tauri::command]
pub fn set_skip_op_ed(db: State<'_, AppDatabase>, enabled: bool) -> Result<LibraryState, String> {
    db.with_conn(|conn| {
        op_ed::write_skip_op_ed(conn, enabled)?;
        build_library_state(conn, &db)
    })
}

#[tauri::command]
pub fn set_auto_op_ed_detect(db: State<'_, AppDatabase>, enabled: bool) -> Result<LibraryState, String> {
    db.with_conn(|conn| {
        op_ed::write_auto_op_ed_detect(conn, enabled)?;
        build_library_state(conn, &db)
    })
}

#[tauri::command]
pub fn set_dont_skip_first_episode_op_ed(
    db: State<'_, AppDatabase>,
    enabled: bool,
) -> Result<LibraryState, String> {
    db.with_conn(|conn| {
        op_ed::write_dont_skip_first_episode_op_ed(conn, enabled)?;
        build_library_state(conn, &db)
    })
}

#[tauri::command]
pub fn set_prefer_anilist_display_title(
    db: State<'_, AppDatabase>,
    enabled: bool,
) -> Result<LibraryState, String> {
    db.with_conn(|conn| {
        write_prefer_anilist_display_title(conn, enabled)?;
        build_library_state(conn, &db)
    })
}

#[tauri::command]
pub fn set_hide_anilist_features(
    db: State<'_, AppDatabase>,
    enabled: bool,
) -> Result<LibraryState, String> {
    db.with_conn(|conn| {
        write_hide_anilist_features(conn, enabled)?;
        build_library_state(conn, &db)
    })
}

#[tauri::command]
pub fn set_clean_unused_scrub_sprites(
    db: State<'_, AppDatabase>,
    enabled: bool,
) -> Result<LibraryState, String> {
    db.with_conn(|conn| {
        write_clean_unused_scrub_sprites(conn, enabled)?;
        build_library_state(conn, &db)
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
pub fn reorder_categories(db: State<'_, AppDatabase>, category_ids: Vec<i64>) -> Result<(), String> {
    db.with_conn(|conn| {
        if category_ids.is_empty() {
            return Err("Category order cannot be empty.".to_string());
        }

        let existing = list_categories(conn)?;
        if category_ids.len() != existing.len() {
            return Err("Category order must include every category exactly once.".to_string());
        }

        let existing_ids = existing.into_iter().map(|category| category.id).collect::<HashSet<_>>();
        let mut seen = HashSet::new();
        for id in &category_ids {
            if !existing_ids.contains(id) {
                return Err(format!("Category does not exist: {id}"));
            }
            if !seen.insert(*id) {
                return Err(format!("Category appears more than once: {id}"));
            }
        }
        if seen.len() != existing_ids.len() {
            return Err("Category order must include every category exactly once.".to_string());
        }

        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (sort_order, id) in category_ids.iter().enumerate() {
            tx.execute(
                "UPDATE categories SET sort_order = ?1 WHERE id = ?2",
                params![sort_order as i64, id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
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
pub fn reset_regex_rules_to_defaults(db: State<'_, AppDatabase>) -> Result<Vec<RegexRule>, String> {
    db.with_conn(|conn| {
        crate::db::reset_regex_rules_to_defaults(conn)?;
        list_regex_rules(conn)
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
    delete_anime_files_impl(&db, anime_id, false)
}

pub(crate) fn delete_anime_files_for_operation(
    db: &AppDatabase,
    anime_id: i64,
) -> Result<DeleteAnimeFilesSummary, String> {
    delete_anime_files_impl(db, anime_id, true)
}

fn delete_anime_files_impl(
    db: &AppDatabase,
    anime_id: i64,
    include_pending_delete: bool,
) -> Result<DeleteAnimeFilesSummary, String> {
    let (episodes, cover_path, custom_thumbnail_path, root_folders) = db.with_conn(|conn| {
        let episodes = list_deletable_episodes_for_anime(conn, anime_id, include_pending_delete)?;
        let (cover_path, custom_thumbnail_path) = conn
            .query_row(
                "SELECT anilist_cover_path, custom_thumbnail_path FROM anime WHERE id = ?1",
                params![anime_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or((None, None));
        let root_folders = list_root_folders(conn)?
            .into_iter()
            .map(|root| PathBuf::from(root.path))
            .collect::<Vec<_>>();
        Ok((episodes, cover_path, custom_thumbnail_path, root_folders))
    })?;

    let deletable_episode_count = episodes.len();
    let mut deleted_episode_ids = Vec::new();
    let mut bytes_deleted = 0_u64;
    let mut episodes_failed = 0_i64;
    let mut permanent_delete_used = false;
    let mut dirs_to_cleanup = HashSet::new();

    for episode in episodes {
        let path = PathBuf::from(&episode.path);
        bytes_deleted += crate::scrub_preview::remove_scrub_sprite_cache(&episode.path)
            .unwrap_or(0);

        if !path.exists() {
            deleted_episode_ids.push(episode.id);
            if let Some(parent) = path.parent() {
                dirs_to_cleanup.insert(parent.to_path_buf());
            }
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
                if let Some(parent) = path.parent() {
                    dirs_to_cleanup.insert(parent.to_path_buf());
                }
            }
            Err(_) => {
                episodes_failed += 1;
            }
        }
    }

    cleanup_empty_dirs_after_deletions(dirs_to_cleanup, &root_folders);

    let mut cover_deleted = false;
    let mut cover_failed = false;
    let mut clear_cover_path = false;
    if let Some(cover_path) = cover_path {
        let path = data_file_path(db, &cover_path);
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

    let remove_anime_from_library = episodes_failed == 0
        && deleted_episode_ids.len() == deletable_episode_count;

    let mut remove_anime_row = remove_anime_from_library;

    let _ = db.with_conn(|conn| op_ed::reset_anime_op_ed_analysis(conn, anime_id));

    db.with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        if remove_anime_from_library {
            tx.execute("DELETE FROM anime WHERE id = ?1", params![anime_id])
                .map_err(|e| e.to_string())?;
        } else {
            for episode_id in &deleted_episode_ids {
                tx.execute("DELETE FROM episodes WHERE id = ?1", params![episode_id])
                    .map_err(|e| e.to_string())?;
            }
            let remaining: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM episodes WHERE anime_id = ?1",
                    params![anime_id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if remaining == 0 {
                remove_anime_row = true;
                tx.execute("DELETE FROM anime WHERE id = ?1", params![anime_id])
                    .map_err(|e| e.to_string())?;
            } else if clear_cover_path {
                tx.execute(
                    "UPDATE anime
                     SET anilist_cover_path = NULL,
                         updated_at = CURRENT_TIMESTAMP
                     WHERE id = ?1",
                    params![anime_id],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        refresh_anime_latest_episode_at(conn)?;
        Ok(())
    })?;

    if remove_anime_row {
        if let Some(custom_thumbnail_path) = custom_thumbnail_path {
            let path = PathBuf::from(&custom_thumbnail_path);
            if path.exists() {
                let size = fs::metadata(&path).map(|metadata| metadata.len()).unwrap_or(0);
                if move_path_to_trash_or_delete(&path).is_ok() {
                    bytes_deleted += size;
                }
            }
        }
    }

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
pub fn delete_episode_files(
    db: State<'_, AppDatabase>,
    episode_id: i64,
) -> Result<DeleteAnimeFilesSummary, String> {
    delete_episode_files_impl(&db, episode_id, false)
}

pub(crate) fn delete_episode_files_for_operation(
    db: &AppDatabase,
    episode_id: i64,
) -> Result<DeleteAnimeFilesSummary, String> {
    delete_episode_files_impl(db, episode_id, true)
}

fn delete_episode_files_impl(
    db: &AppDatabase,
    episode_id: i64,
    include_pending_delete: bool,
) -> Result<DeleteAnimeFilesSummary, String> {
    let (episode, anime_id, cover_path, custom_thumbnail_path, root_folders) = db.with_conn(|conn| {
        let episode = conn
            .query_row(
                "SELECT id, path, size, anime_id
                 FROM episodes
                 WHERE id = ?1
                   AND missing = 0
                   AND (pending_delete = 0 OR ?2 != 0)",
                params![episode_id, if include_pending_delete { 1 } else { 0 }],
                |row| {
                    Ok(DeletableEpisode {
                        id: row.get(0)?,
                        path: row.get(1)?,
                        size: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Episode does not exist: {episode_id}"))?;
        let anime_id: i64 = conn
            .query_row(
                "SELECT anime_id FROM episodes WHERE id = ?1",
                params![episode_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let (cover_path, custom_thumbnail_path) = conn
            .query_row(
                "SELECT anilist_cover_path, custom_thumbnail_path FROM anime WHERE id = ?1",
                params![anime_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or((None, None));
        let root_folders = list_root_folders(conn)?
            .into_iter()
            .map(|root| PathBuf::from(root.path))
            .collect::<Vec<_>>();
        Ok((episode, anime_id, cover_path, custom_thumbnail_path, root_folders))
    })?;

    let mut deleted_episode_ids = Vec::new();
    let mut bytes_deleted = 0_u64;
    let mut episodes_failed = 0_i64;
    let mut permanent_delete_used = false;
    let mut dirs_to_cleanup = HashSet::new();

    let path = PathBuf::from(&episode.path);
    bytes_deleted += crate::scrub_preview::remove_scrub_sprite_cache(&episode.path).unwrap_or(0);

    if !path.exists() {
        deleted_episode_ids.push(episode.id);
        if let Some(parent) = path.parent() {
            dirs_to_cleanup.insert(parent.to_path_buf());
        }
    } else {
        let size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or_else(|_| episode.size.max(0) as u64);
        match move_path_to_trash_or_delete(&path) {
            Ok(permanent) => {
                deleted_episode_ids.push(episode.id);
                bytes_deleted += size;
                permanent_delete_used |= permanent;
                if let Some(parent) = path.parent() {
                    dirs_to_cleanup.insert(parent.to_path_buf());
                }
            }
            Err(_) => {
                episodes_failed = 1;
            }
        }
    }

    cleanup_empty_dirs_after_deletions(dirs_to_cleanup, &root_folders);

    let mut cover_deleted = false;
    let mut cover_failed = false;
    let mut clear_cover_path = false;
    let episode_deleted = episodes_failed == 0 && !deleted_episode_ids.is_empty();

    if episode_deleted {
        let remaining = db.with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM episodes
                 WHERE anime_id = ?1 AND missing = 0 AND pending_delete = 0 AND id != ?2",
                params![anime_id, episode_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())
        })?;
        if remaining == 0 {
            if let Some(cover_path) = cover_path {
                let cover_file = data_file_path(db, &cover_path);
                if cover_file.exists() {
                    let size = fs::metadata(&cover_file)
                        .map(|metadata| metadata.len())
                        .unwrap_or(0);
                    match move_path_to_trash_or_delete(&cover_file) {
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
        }
    }

    let mut remove_anime_row = false;

    db.with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        if episodes_failed > 0 {
            tx.commit().map_err(|e| e.to_string())?;
            return Ok(());
        }

        tx.execute("DELETE FROM episodes WHERE id = ?1", params![episode_id])
            .map_err(|e| e.to_string())?;

        let remaining: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM episodes WHERE anime_id = ?1",
                params![anime_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if remaining == 0 {
            remove_anime_row = true;
            op_ed::reset_anime_op_ed_analysis(&tx, anime_id)?;
            tx.execute("DELETE FROM anime WHERE id = ?1", params![anime_id])
                .map_err(|e| e.to_string())?;
        } else if clear_cover_path {
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

    if remove_anime_row {
        if let Some(custom_thumbnail_path) = custom_thumbnail_path {
            let path = PathBuf::from(&custom_thumbnail_path);
            if path.exists() {
                let size = fs::metadata(&path).map(|metadata| metadata.len()).unwrap_or(0);
                if move_path_to_trash_or_delete(&path).is_ok() {
                    bytes_deleted += size;
                }
            }
        }
    }

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
pub fn clear_anime_local_data(
    db: State<'_, AppDatabase>,
    anime_id: i64,
) -> Result<ClearAnimeLocalDataSummary, String> {
    clear_anime_local_data_impl(&db, anime_id)
}

fn clear_anime_local_data_impl(
    db: &AppDatabase,
    anime_id: i64,
) -> Result<ClearAnimeLocalDataSummary, String> {
    let (episode_paths, cover_path, custom_thumbnail_path, episode_count) = db.with_conn(|conn| {
        let (cover_path, custom_thumbnail_path) = conn
            .query_row(
                "SELECT anilist_cover_path, custom_thumbnail_path
                 FROM anime
                 WHERE id = ?1 AND pending_delete = 0",
                params![anime_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Anime does not exist: {anime_id}"))?;

        let mut stmt = conn
            .prepare("SELECT path FROM episodes WHERE anime_id = ?1 AND pending_delete = 0")
            .map_err(|e| e.to_string())?;
        let paths = stmt
            .query_map(params![anime_id], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|e| e.to_string())?;
        let episode_count = paths.len() as i64;

        Ok((paths, cover_path, custom_thumbnail_path, episode_count))
    })?;

    let mut bytes_removed = 0_u64;
    for path in &episode_paths {
        bytes_removed += crate::scrub_preview::remove_scrub_sprite_cache(path).unwrap_or(0);
    }

    let _ = db.with_conn(|conn| op_ed::reset_anime_op_ed_analysis(conn, anime_id));

    if let Some(cover_path) = cover_path {
        let path = data_file_path(db, &cover_path);
        if path.exists() {
            bytes_removed += fs::metadata(&path).map(|metadata| metadata.len()).unwrap_or(0);
            let _ = fs::remove_file(&path);
        }
    }

    if let Some(custom_thumbnail_path) = custom_thumbnail_path {
        let path = PathBuf::from(&custom_thumbnail_path);
        if path.exists() {
            bytes_removed += fs::metadata(&path).map(|metadata| metadata.len()).unwrap_or(0);
            let _ = fs::remove_file(&path);
        }
    }

    db.with_conn(|conn| {
        conn.execute("DELETE FROM anime WHERE id = ?1", params![anime_id])
            .map_err(|e| e.to_string())?;
        refresh_anime_latest_episode_at(conn)?;
        Ok(())
    })?;

    Ok(ClearAnimeLocalDataSummary {
        episodes_removed: episode_count,
        bytes_removed,
    })
}

#[tauri::command]
pub fn validate_file_renames(
    db: State<'_, AppDatabase>,
    renames: Vec<RenameFileRequest>,
) -> Result<(), String> {
    db.with_conn(|conn| build_rename_episode_plan(conn, &renames).map(|_| ()))
}

#[tauri::command]
pub fn rename_files(
    db: State<'_, AppDatabase>,
    renames: Vec<RenameFileRequest>,
) -> Result<RenameFilesSummary, String> {
    let plans = db.with_conn(|conn| build_rename_episode_plan(conn, &renames))?;
    let temp_moves = move_episode_sources_to_temps(&plans)?;

    if let Err(error) = move_episode_temps_to_destinations(&temp_moves) {
        rollback_episode_renames(&temp_moves);
        return Err(error);
    }

    if let Err(error) = db.with_conn(|conn| persist_episode_renames(conn, &plans)) {
        rollback_episode_rename_destinations(&temp_moves);
        return Err(error);
    }

    Ok(RenameFilesSummary {
        files_renamed: plans.len() as i64,
    })
}

#[tauri::command]
pub fn rename_anime(
    #[cfg(windows)] mpv_state: State<'_, AppState>,
    db: State<'_, AppDatabase>,
    anime_id: i64,
    new_title: String,
) -> Result<RenameAnimeSummary, String> {
    let new_title = normalize_anime_rename_title(&new_title)?;
    let plans = db.with_conn(|conn| build_anime_rename_plan(conn, anime_id, &new_title))?;
    #[cfg(windows)]
    {
        let paths = plans
            .iter()
            .map(|plan| plan.old_path_string.clone())
            .collect::<Vec<_>>();
        let guard = mpv_state.mpv.lock().map_err(|e| e.to_string())?;
        crate::mpv::unload_if_loading_any_of(guard.as_ref(), &paths)?;
    }
    let temp_moves = move_episode_sources_to_temps(&plans)?;

    if let Err(error) = move_episode_temps_to_destinations(&temp_moves) {
        rollback_episode_renames(&temp_moves);
        return Err(error);
    }

    if let Err(error) =
        db.with_conn(|conn| persist_anime_rename(conn, anime_id, &new_title, &plans))
    {
        rollback_episode_rename_destinations(&temp_moves);
        return Err(error);
    }

    Ok(RenameAnimeSummary {
        files_renamed: plans.len() as i64,
    })
}

#[tauri::command]
pub fn open_anime_episode_folder(
    app: AppHandle,
    db: State<'_, AppDatabase>,
    anime_id: i64,
) -> Result<(), String> {
    let folder = db.with_conn(|conn| preferred_episode_folder_for_anime(conn, anime_id))?;
    let Some(folder) = folder else {
        return Err("No episode folder is available.".to_string());
    };
    open_folder_in_shell(&app, &folder)
}

#[tauri::command]
pub fn open_episode_folder(
    app: AppHandle,
    db: State<'_, AppDatabase>,
    episode_id: i64,
) -> Result<(), String> {
    let path = db.with_conn(|conn| {
        conn.query_row(
            "SELECT path FROM episodes WHERE id = ?1 AND missing = 0 AND pending_delete = 0",
            params![episode_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })?;
    let Some(path) = path else {
        return Err("Episode not found.".to_string());
    };
    let path_buf = PathBuf::from(path);
    let Some(folder) = path_buf.parent() else {
        return Err("No episode folder is available.".to_string());
    };
    open_folder_in_shell(&app, folder)
}

fn open_folder_in_shell(app: &AppHandle, folder: &Path) -> Result<(), String> {
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
pub fn set_anime_tracker_offset(
    db: State<'_, AppDatabase>,
    anime_id: i64,
    tracker_offset: i64,
) -> Result<(), String> {
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE anime
             SET tracker_offset = ?1,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?2",
            params![tracker_offset, anime_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn set_anime_custom_thumbnail_path(
    db: State<'_, AppDatabase>,
    anime_id: i64,
    custom_thumbnail_path: Option<String>,
) -> Result<(), String> {
    let normalized = custom_thumbnail_path.and_then(|path| {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE anime
             SET custom_thumbnail_path = ?1,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?2",
            params![normalized, anime_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn override_anime_progress(
    db: State<'_, AppDatabase>,
    anime_id: i64,
    progress: i64,
) -> Result<ProgressOverrideSummary, String> {
    let progress = progress.max(0);
    db.with_conn(|conn| {
        let tracker_offset = conn
            .query_row(
                "SELECT tracker_offset FROM anime WHERE id = ?1",
                params![anime_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())?;
        let updated_episodes = conn
            .execute(
                "UPDATE episodes
                 SET watched = CASE
                         WHEN episode_number IS NOT NULL
                          AND (CAST(episode_number AS INTEGER) - ?2) BETWEEN 1 AND ?3 THEN 1
                         ELSE 0
                     END,
                     position_seconds = CASE
                         WHEN episode_number IS NOT NULL
                          AND (CAST(episode_number AS INTEGER) - ?2) BETWEEN 1 AND ?3
                          AND duration_seconds > 0 THEN duration_seconds
                         ELSE 0
                     END,
                     last_watched_at = CASE
                         WHEN episode_number IS NOT NULL
                          AND (CAST(episode_number AS INTEGER) - ?2) BETWEEN 1 AND ?3
                         THEN CURRENT_TIMESTAMP
                         ELSE NULL
                     END,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE anime_id = ?1
                   AND missing = 0
                   AND pending_delete = 0",
                params![anime_id, tracker_offset, progress],
            )
            .map(|count| count as i64)
            .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE anime
             SET last_watched_at = CASE WHEN ?1 > 0 THEN CURRENT_TIMESTAMP ELSE NULL END,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?2",
            params![progress, anime_id],
        )
        .map_err(|e| e.to_string())?;
        refresh_anime_latest_episode_at(conn)?;
        Ok(ProgressOverrideSummary {
            progress,
            updated_episodes,
        })
    })
}

#[tauri::command]
pub fn get_min_position_seconds_to_persist() -> f64 {
    MIN_POSITION_SECONDS_TO_PERSIST
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
                 last_watched_at = CASE WHEN ?3 = 1 THEN CURRENT_TIMESTAMP ELSE NULL END,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?4",
            params![position_seconds, duration_seconds, watched_flag, episode_id],
        )
        .map_err(|e| e.to_string())?;
        let anime_id: i64 = conn
            .query_row(
                "SELECT anime_id FROM episodes WHERE id = ?1",
                params![episode_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE anime
             SET last_watched_at = (
                     SELECT MAX(last_watched_at)
                     FROM episodes
                     WHERE anime_id = ?1
                       AND missing = 0
                       AND pending_delete = 0
                 ),
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![anime_id],
        )
        .map_err(|e| e.to_string())?;
        get_episode(conn, episode_id)
    })
}

#[derive(Debug, Clone)]
struct RescanImportedEpisode {
    anime_id: i64,
    path: String,
    anime_title: String,
    episode_label: String,
}

fn scrub_episode_label(episode: &scanner::ScannedEpisode) -> String {
    match episode.episode_number {
        Some(n) if n.is_finite() => {
            if n.fract() == 0.0 {
                format!("Episode {}", n as i64)
            } else {
                format!("Episode {n:.1}")
            }
        }
        _ => episode.file_name.clone(),
    }
}

fn rescan_library_in_conn(conn: &mut Connection) -> Result<(ScanSummary, Vec<RescanImportedEpisode>), String> {
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
    let mut new_imports = Vec::new();

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
                new_imports.push(RescanImportedEpisode {
                    anime_id,
                    path: episode.path.clone(),
                    anime_title: episode.title.clone(),
                    episode_label: scrub_episode_label(&episode),
                });
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

    Ok((summary, new_imports))
}

#[tauri::command]
#[cfg(windows)]
pub fn rescan_library(
    app: AppHandle,
    db: State<'_, AppDatabase>,
) -> Result<ScanSummary, String> {
    rescan_library_for_operation(app, &db)
}

#[cfg(windows)]
pub(crate) fn rescan_library_for_operation(
    app: AppHandle,
    db: &AppDatabase,
) -> Result<ScanSummary, String> {
    let (summary, new_imports) = db.with_conn(rescan_library_in_conn)?;
    let import_count = new_imports.len();
    let op_ed_imports: Vec<crate::jobs::RescanOpEdImport> = new_imports
        .iter()
        .map(|item| crate::jobs::RescanOpEdImport {
            anime_id: item.anime_id,
            anime_title: item.anime_title.clone(),
        })
        .collect();
    let scrub_imports: Vec<crate::jobs::RescanScrubImport> = new_imports
        .into_iter()
        .map(|item| crate::jobs::RescanScrubImport {
            path: item.path,
            anime_title: item.anime_title,
            episode_label: item.episode_label,
        })
        .collect();
    if import_count > 0 && import_count <= crate::jobs::RESCAN_AUTO_SCRUB_MAX {
        crate::jobs::schedule_rescan_job_enqueue(app, scrub_imports, op_ed_imports);
    }
    Ok(summary)
}

#[tauri::command]
#[cfg(not(windows))]
pub fn rescan_library(db: State<'_, AppDatabase>) -> Result<ScanSummary, String> {
    rescan_library_for_operation(&db)
}

#[cfg(not(windows))]
pub(crate) fn rescan_library_for_operation(db: &AppDatabase) -> Result<ScanSummary, String> {
    db.with_conn(|conn| rescan_library_in_conn(conn).map(|(summary, _)| summary))
}

#[tauri::command]
pub fn get_local_data_stats(db: State<'_, AppDatabase>) -> Result<LocalDataStats, String> {
    cached_local_data_stats(&db)
}

pub(crate) fn refresh_local_data_stats_cache(db: &AppDatabase) -> Result<LocalDataStats, String> {
    let stats = local_data_stats(db)?;
    let json = serde_json::to_string(&stats).map_err(|e| e.to_string())?;
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![LOCAL_DATA_STATS_CACHE_KEY, json],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(stats)
}

#[tauri::command]
pub fn clean_local_data(db: State<'_, AppDatabase>) -> Result<LocalDataCleanupSummary, String> {
    clean_local_data_for_operation(&db)
}

pub(crate) fn clean_local_data_for_operation(
    db: &AppDatabase,
) -> Result<LocalDataCleanupSummary, String> {
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
            scrub_sprites_removed: 0,
            op_ed_fingerprints_removed: 0,
            op_ed_temp_pcm_removed: 0,
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
    let referenced_scrub_paths = db.with_conn(|conn| list_referenced_scrub_paths(conn))?;
    let (removed, thumbnail_bytes_removed) = delete_unreferenced_thumbnails(&db)?;
    summary.thumbnails_removed = removed as i64;
    let (scrub_removed, scrub_bytes_removed) =
        crate::scrub_preview::delete_unreferenced_scrub_sprites(&referenced_scrub_paths)?;
    summary.scrub_sprites_removed = scrub_removed as i64;
    let mut scrub_bytes_removed = scrub_bytes_removed;
    let clean_stale_scrub_sprites = db.with_conn(|conn| read_clean_unused_scrub_sprites(conn))?;
    if clean_stale_scrub_sprites {
        let stale_paths = db.with_conn(|conn| list_stale_anime_episode_paths(conn))?;
        let (stale_removed, stale_bytes) =
            crate::scrub_preview::delete_scrub_sprites_for_paths(&stale_paths)?;
        summary.scrub_sprites_removed += stale_removed as i64;
        scrub_bytes_removed += stale_bytes;
    }
    let referenced_op_ed = db.with_conn(|conn| op_ed::list_referenced_op_ed_fingerprint_keys(conn))?;
    let (op_ed_removed, op_ed_bytes_removed) =
        op_ed::delete_unreferenced_op_ed_fingerprints(&referenced_op_ed)?;
    summary.op_ed_fingerprints_removed = op_ed_removed as i64;
    let (temp_pcm_removed, temp_pcm_bytes_removed) = op_ed::delete_stale_op_ed_temp_pcm_files()?;
    summary.op_ed_temp_pcm_removed = temp_pcm_removed as i64;
    summary.bytes_removed = database_bytes_before.saturating_sub(database_bytes_after)
        + thumbnail_bytes_removed
        + scrub_bytes_removed
        + op_ed_bytes_removed
        + temp_pcm_bytes_removed;
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
               AND pending_delete = 0
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

fn list_anime_search_index(conn: &mut Connection) -> Result<Vec<AnimeSearchEntry>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.title, a.anilist_title, e.file_name
             FROM anime a
             JOIN episodes e ON e.anime_id = a.id AND e.missing = 0 AND e.pending_delete = 0
             WHERE a.pending_delete = 0
               AND EXISTS (
                 SELECT 1 FROM episodes ae
                 WHERE ae.anime_id = a.id AND ae.missing = 0 AND ae.pending_delete = 0
               )
             ORDER BY a.id, e.file_name COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut entries: Vec<AnimeSearchEntry> = Vec::new();
    for row in rows {
        let (id, title, anilist_title, file_name) = row.map_err(|e| e.to_string())?;
        if let Some(entry) = entries.last_mut() {
            if entry.id == id {
                entry.file_names.push(file_name);
                continue;
            }
        }
        entries.push(AnimeSearchEntry {
            id,
            title,
            anilist_title,
            file_names: vec![file_name],
        });
    }
    Ok(entries)
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
                a.custom_thumbnail_path,
                a.tracker_offset,
                COUNT(e.id) AS episode_count,
                SUM(CASE WHEN e.watched = 0 THEN 1 ELSE 0 END) AS unwatched_count,
                MIN(CASE WHEN e.episode_number IS NOT NULL
                         AND e.episode_number = CAST(e.episode_number AS INTEGER)
                    THEN CAST(e.episode_number AS INTEGER) END) AS min_int_ep,
                MAX(CASE WHEN e.episode_number IS NOT NULL
                         AND e.episode_number = CAST(e.episode_number AS INTEGER)
                    THEN CAST(e.episode_number AS INTEGER) END) AS max_int_ep,
                COUNT(DISTINCT CASE WHEN e.episode_number IS NOT NULL
                                     AND e.episode_number = CAST(e.episode_number AS INTEGER)
                                THEN CAST(e.episode_number AS INTEGER) END) AS int_ep_count,
                a.anilist_cached_episodes,
                a.anilist_cached_status,
                a.last_watched_at,
                a.created_at,
                a.latest_episode_at,
                (SELECT e2.path FROM episodes e2
                 WHERE e2.anime_id = a.id AND e2.missing = 0 AND e2.pending_delete = 0
                 ORDER BY e2.episode_number IS NULL, e2.episode_number, e2.relative_path COLLATE NOCASE
                 LIMIT 1) AS first_episode_path,
                a.no_op_ed
         FROM anime a
         LEFT JOIN episodes e ON e.anime_id = a.id AND e.missing = 0 AND e.pending_delete = 0",
    );
    if category_id.is_some() {
        sql.push_str(
            " WHERE a.pending_delete = 0
              AND a.category_id = ?1
              AND EXISTS (
                SELECT 1 FROM episodes ae
                WHERE ae.anime_id = a.id AND ae.missing = 0 AND ae.pending_delete = 0
              )",
        );
    } else if recent_only {
        sql.push_str(
            " WHERE a.pending_delete = 0
              AND a.last_watched_at IS NOT NULL
              AND EXISTS (
                SELECT 1 FROM episodes ae
                WHERE ae.anime_id = a.id AND ae.missing = 0 AND ae.pending_delete = 0
              )",
        );
    } else {
        sql.push_str(
            " WHERE a.pending_delete = 0
              AND EXISTS (
                SELECT 1 FROM episodes ae
                WHERE ae.anime_id = a.id AND ae.missing = 0 AND ae.pending_delete = 0
              )",
        );
    }
    sql.push_str(" GROUP BY a.id");
    if recent_only {
        sql.push_str(" ORDER BY a.last_watched_at DESC LIMIT 12");
    } else {
        sql.push_str(" ORDER BY a.title COLLATE NOCASE");
    }

    let map_row = |row: &rusqlite::Row<'_>| {
        let tracker_offset: i64 = row.get(8)?;
        let min_int_ep: Option<i64> = row.get(11)?;
        let max_int_ep: Option<i64> = row.get(12)?;
        let int_ep_count: i64 = row.get::<_, Option<i64>>(13)?.unwrap_or(0);
        let anilist_cached_episodes: Option<i64> = row.get(14)?;
        let anilist_cached_status: Option<String> = row.get(15)?;
        let gap_episode_count = match (min_int_ep, max_int_ep) {
            (Some(lo), Some(hi)) => compute_gap_episode_count(
                lo,
                hi,
                int_ep_count,
                anilist_cached_episodes,
                anilist_cached_status.as_deref(),
                tracker_offset,
            ),
            _ => 0,
        };
        Ok(AnimeSummary {
            id: row.get(0)?,
            title: row.get(1)?,
            category_id: row.get(2)?,
            anilist_id: row.get(3)?,
            anilist_title: row.get(4)?,
            anilist_site_url: row.get(5)?,
            anilist_cover_path: row.get(6)?,
            custom_thumbnail_path: row.get(7)?,
            tracker_offset,
            episode_count: row.get(9)?,
            unwatched_count: row.get::<_, Option<i64>>(10)?.unwrap_or(0),
            gap_episode_count,
            last_watched_at: row.get(16)?,
            created_at: row.get(17)?,
            latest_episode_at: row.get(18)?,
            first_episode_path: row.get(19)?,
            no_op_ed: row.get::<_, i64>(20)? != 0,
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
                    a.custom_thumbnail_path,
                    a.tracker_offset,
                    SUM(CASE WHEN e.missing = 0 AND e.pending_delete = 0 THEN 1 ELSE 0 END) AS available_count,
                    SUM(CASE WHEN e.missing = 0 AND e.pending_delete = 0 AND e.watched = 0 THEN 1 ELSE 0 END) AS unwatched_count,
                    SUM(CASE WHEN e.missing != 0 AND e.pending_delete = 0 THEN 1 ELSE 0 END) AS missing_count,
                    COUNT(e.id) AS total_count,
                    a.last_watched_at,
                    a.created_at,
                    a.latest_episode_at,
                    (SELECT e2.path FROM episodes e2
                     WHERE e2.anime_id = a.id AND e2.pending_delete = 0
                     ORDER BY e2.missing, e2.episode_number IS NULL, e2.episode_number, e2.relative_path COLLATE NOCASE
                     LIMIT 1) AS first_episode_path
             FROM anime a
             JOIN episodes e ON e.anime_id = a.id AND e.pending_delete = 0
             WHERE a.pending_delete = 0
             GROUP BY a.id
             HAVING SUM(CASE WHEN e.missing != 0 AND e.pending_delete = 0 THEN 1 ELSE 0 END) > 0
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
                custom_thumbnail_path: row.get(7)?,
                tracker_offset: row.get(8)?,
                episode_count: row.get::<_, Option<i64>>(9)?.unwrap_or(0),
                unwatched_count: row.get::<_, Option<i64>>(10)?.unwrap_or(0),
                missing_episode_count: row.get::<_, Option<i64>>(11)?.unwrap_or(0),
                total_episode_count: row.get(12)?,
                last_watched_at: row.get(13)?,
                created_at: row.get(14)?,
                latest_episode_at: row.get(15)?,
                first_episode_path: row.get(16)?,
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
               AND pending_delete = 0
             ORDER BY episode_number IS NULL, episode_number, relative_path COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![anime_id], episode_from_row)
        .map_err(|e| e.to_string())?;
    let mut episodes = collect_rows(rows)?;
    for episode in &mut episodes {
        episode.op_ed_segments = op_ed::load_episode_op_ed_segments(conn, episode.id)?;
    }
    Ok(episodes)
}

fn list_deletable_episodes_for_anime(
    conn: &Connection,
    anime_id: i64,
    include_pending_delete: bool,
) -> Result<Vec<DeletableEpisode>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path, size
             FROM episodes
             WHERE anime_id = ?1
               AND missing = 0
               AND (pending_delete = 0 OR ?2 != 0)
             ORDER BY episode_number IS NULL, episode_number, relative_path COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![anime_id, if include_pending_delete { 1 } else { 0 }], |row| {
            Ok(DeletableEpisode {
                id: row.get(0)?,
                path: row.get(1)?,
                size: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    collect_rows(rows)
}

fn preferred_episode_folder_for_anime(
    conn: &Connection,
    anime_id: i64,
) -> Result<Option<PathBuf>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT path
             FROM episodes
             WHERE anime_id = ?1
               AND missing = 0
               AND pending_delete = 0",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![anime_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;

    let episode_paths: Vec<PathBuf> = collect_rows(rows)?
        .into_iter()
        .map(PathBuf::from)
        .collect();

    Ok(pick_preferred_episode_folder(&episode_paths))
}

/// Among each episode's parent directory, pick the one that contains the most
/// episode files recursively; break ties by shortest path, then lexicographic.
fn pick_preferred_episode_folder(episode_paths: &[PathBuf]) -> Option<PathBuf> {
    if episode_paths.is_empty() {
        return None;
    }

    let mut candidates = HashSet::new();
    for path in episode_paths {
        if let Some(parent) = path.parent() {
            candidates.insert(parent.to_path_buf());
        }
    }

    let mut best: Option<PathBuf> = None;
    let mut best_count = 0usize;
    for candidate in candidates {
        let count = count_episodes_under_folder(&candidate, episode_paths);
        let replace = best.as_ref().is_none_or(|current| {
            count > best_count || (count == best_count && is_shorter_path(&candidate, current))
        });
        if replace {
            best = Some(candidate);
            best_count = count;
        }
    }

    best
}

fn count_episodes_under_folder(folder: &Path, episode_paths: &[PathBuf]) -> usize {
    episode_paths
        .iter()
        .filter(|path| path.starts_with(folder))
        .count()
}

fn is_shorter_path(a: &Path, b: &Path) -> bool {
    let a_len = a.as_os_str().len();
    let b_len = b.as_os_str().len();
    a_len < b_len || (a_len == b_len && a.to_string_lossy() < b.to_string_lossy())
}

#[cfg(test)]
mod episode_folder_tests {
    use super::*;

    fn frieren_example_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for i in 1..=8 {
            paths.push(PathBuf::from(format!(
                r"C:\anime\ongoing\frieren\watched\{i:02}.mkv"
            )));
        }
        for i in 9..=10 {
            paths.push(PathBuf::from(format!(
                r"C:\anime\ongoing\frieren\{i:02}.mkv"
            )));
        }
        for i in 1..=3 {
            paths.push(PathBuf::from(format!(r"V:\frieren-special\{i}.mkv")));
        }
        paths
    }

    #[test]
    fn pick_preferred_episode_folder_prefers_most_episodes_recursively() {
        let picked = pick_preferred_episode_folder(&frieren_example_paths()).expect("folder");
        assert!(paths_equal(
            &picked,
            Path::new(r"C:\anime\ongoing\frieren"),
        ));
    }

    #[test]
    fn pick_preferred_episode_folder_breaks_ties_by_shortest_path() {
        let paths = vec![
            PathBuf::from(r"D:\show\season 1\01.mkv"),
            PathBuf::from(r"D:\show\season 1\02.mkv"),
            PathBuf::from(r"D:\show\season 2\01.mkv"),
            PathBuf::from(r"D:\show\season 2\02.mkv"),
        ];
        let picked = pick_preferred_episode_folder(&paths).expect("folder");
        assert!(paths_equal(&picked, Path::new(r"D:\show\season 1")));
        assert_eq!(count_episodes_under_folder(&picked, &paths), 2);
    }

    #[test]
    fn pick_preferred_episode_folder_does_not_match_similar_prefix() {
        let paths = vec![PathBuf::from(
            r"C:\anime\ongoing\frieren-extra\01.mkv",
        )];
        let picked = pick_preferred_episode_folder(&paths).expect("folder");
        assert!(paths_equal(
            &picked,
            Path::new(r"C:\anime\ongoing\frieren-extra"),
        ));
        assert_eq!(count_episodes_under_folder(&picked, &paths), 1);
    }
}

/// Retries help when Windows briefly holds a file (mpv, indexer, antivirus).
const FILE_DELETE_MAX_ATTEMPTS: usize = 3;
const FILE_DELETE_RETRY_DELAY_MS: u64 = 1500;

fn move_path_to_trash_or_delete(path: &Path) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }

    let mut last_error: Option<String> = None;
    for attempt in 0..FILE_DELETE_MAX_ATTEMPTS {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(FILE_DELETE_RETRY_DELAY_MS));
            if !path.exists() {
                return Ok(false);
            }
        }
        match move_path_to_trash_or_delete_once(path) {
            Ok(permanent) => return Ok(permanent),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| format!("failed to delete {}", path.display())))
}

fn move_path_to_trash_or_delete_once(path: &Path) -> Result<bool, String> {
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

fn cleanup_empty_dirs_after_deletions(dirs: HashSet<PathBuf>, roots: &[PathBuf]) {
    if dirs.is_empty() || roots.is_empty() {
        return;
    }

    let mut ordered: Vec<PathBuf> = dirs.into_iter().collect();
    ordered.sort_by(|a, b| {
        b.components()
            .count()
            .cmp(&a.components().count())
            .then_with(|| b.to_string_lossy().cmp(&a.to_string_lossy()))
    });

    for dir in ordered {
        remove_empty_parent_dirs(&dir, roots);
    }
}

fn remove_empty_parent_dirs(start: &Path, roots: &[PathBuf]) {
    let mut dir = start.to_path_buf();
    let Some(root) = matching_root_folder(&dir, roots) else {
        return;
    };

    while !paths_equal(&dir, root) {
        remove_empty_folder_children_when_only_empty_folders_remain(&dir);

        if !is_directory_empty(&dir) {
            break;
        }
        if fs::remove_dir(&dir).is_err() {
            break;
        }
        dir = match dir.parent() {
            Some(parent) => parent.to_path_buf(),
            None => break,
        };
    }
}

/// True when `path` has no files and every subdirectory is empty or file-free.
fn directory_only_contains_empty_folders(path: &Path) -> bool {
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        if entry_path.is_file() {
            return false;
        }
        if entry_path.is_dir() && directory_tree_has_files(&entry_path) {
            return false;
        }
    }
    true
}

fn directory_tree_has_files(path: &Path) -> bool {
    let Ok(entries) = fs::read_dir(path) else {
        return true;
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        if entry_path.is_file() {
            return true;
        }
        if entry_path.is_dir() && directory_tree_has_files(&entry_path) {
            return true;
        }
    }
    false
}

/// Removes child directories when the parent has no files and every child is an empty folder tree.
fn remove_empty_folder_children_when_only_empty_folders_remain(parent: &Path) {
    if !directory_only_contains_empty_folders(parent) {
        return;
    }

    let child_dirs: Vec<PathBuf> = fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            path.is_dir().then_some(path)
        })
        .collect();

    for child in child_dirs {
        remove_empty_folder_children_when_only_empty_folders_remain(&child);
        let _ = fs::remove_dir(&child);
    }
}

fn matching_root_folder<'a>(path: &Path, roots: &'a [PathBuf]) -> Option<&'a PathBuf> {
    roots
        .iter()
        .filter(|root| path.starts_with(root.as_path()))
        .max_by_key(|root| root.as_os_str().len())
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    windows_path_key(&a.to_string_lossy()) == windows_path_key(&b.to_string_lossy())
}

fn is_directory_empty(path: &Path) -> bool {
    let Ok(mut entries) = fs::read_dir(path) else {
        return false;
    };
    entries.next().is_none()
}

fn get_episode(conn: &Connection, episode_id: i64) -> Result<Episode, String> {
    let mut episode = conn
        .query_row(
            "SELECT id, anime_id, path, relative_path, file_name, file_type,
                    episode_number, size, duration_seconds, position_seconds,
                    watched, last_watched_at
             FROM episodes
             WHERE id = ?1",
            params![episode_id],
            episode_from_row,
        )
        .map_err(|e| e.to_string())?;
    episode.op_ed_segments = op_ed::load_episode_op_ed_segments(conn, episode.id)?;
    Ok(episode)
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
        op_ed_segments: Vec::new(),
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

fn cached_local_data_stats(db: &AppDatabase) -> Result<LocalDataStats, String> {
    let database_bytes = fs::metadata(db.path()).map(|m| m.len()).unwrap_or(0);
    let cached = db.with_conn(|conn| {
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![LOCAL_DATA_STATS_CACHE_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })?;
    let Some(cached) = cached else {
        return Ok(LocalDataStats {
            database_bytes,
            thumbnails_bytes: 0,
            scrub_sprites_bytes: 0,
            op_ed_fingerprints_bytes: 0,
            total_bytes: database_bytes,
        });
    };
    let mut stats: LocalDataStats = serde_json::from_str(&cached).map_err(|e| e.to_string())?;
    stats.database_bytes = database_bytes;
    stats.total_bytes = stats.database_bytes
        + stats.thumbnails_bytes
        + stats.scrub_sprites_bytes
        + stats.op_ed_fingerprints_bytes;
    Ok(stats)
}

fn local_data_stats(db: &AppDatabase) -> Result<LocalDataStats, String> {
    let database_bytes = fs::metadata(db.path()).map(|m| m.len()).unwrap_or(0);
    let (thumbnails_bytes, scrub_sprites_bytes, op_ed_fingerprints_bytes) = match db.path().parent() {
        Some(data_dir) => (
            directory_size(&data_dir.join("anilist-covers"))?,
            directory_size(&data_dir.join("scrub-sprites"))?,
            op_ed::op_ed_cache_directory_size()?,
        ),
        None => (0, 0, 0),
    };
    Ok(LocalDataStats {
        database_bytes,
        thumbnails_bytes,
        scrub_sprites_bytes,
        op_ed_fingerprints_bytes,
        total_bytes: database_bytes
            + thumbnails_bytes
            + scrub_sprites_bytes
            + op_ed_fingerprints_bytes,
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

    let referenced_paths = db.with_conn(|conn| list_referenced_thumbnail_paths(conn, data_dir))?;
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

fn data_file_path(db: &AppDatabase, stored_path: &str) -> PathBuf {
    let path = PathBuf::from(stored_path);
    if path.is_absolute() {
        path
    } else if let Some(data_dir) = db.path().parent() {
        data_dir.join(path)
    } else {
        path
    }
}

fn list_stale_anime_episode_paths(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT e.path
             FROM episodes e
             INNER JOIN anime a ON a.id = e.anime_id
             WHERE datetime(COALESCE(a.last_watched_at, a.created_at)) < datetime('now', '-3 months')",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    collect_rows(rows)
}

fn list_referenced_scrub_paths(conn: &mut Connection) -> Result<HashSet<String>, String> {
    let mut stmt = conn
        .prepare("SELECT path FROM episodes")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let paths = collect_rows(rows)?;

    let mut keys = HashSet::new();
    for path in paths {
        keys.insert(crate::scrub_preview::normalized_video_path_key(&path));
        let path_buf = PathBuf::from(&path);
        if path_buf.is_file() {
            if let Ok(canonical) = path_buf.canonicalize() {
                keys.insert(crate::scrub_preview::normalized_video_path_key(
                    &canonical.to_string_lossy(),
                ));
            }
        }
    }
    Ok(keys)
}

fn list_referenced_thumbnail_paths(
    conn: &mut Connection,
    data_dir: &Path,
) -> Result<HashSet<PathBuf>, String> {
    let mut stmt = conn
        .prepare("SELECT anilist_cover_path, anilist_id FROM anime WHERE anilist_cover_path IS NOT NULL OR anilist_id IS NOT NULL")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<i64>>(1)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let rows = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let mut paths = HashSet::new();
    for (stored_path, anilist_id) in rows {
        if let Some(stored_path) = stored_path {
            let path = PathBuf::from(stored_path);
            paths.insert(if path.is_absolute() {
                path
            } else {
                data_dir.join(path)
            });
        }
        if let Some(anilist_id) = anilist_id {
            for extension in ["jpg", "png", "jpeg", "webp"] {
                paths.insert(
                    data_dir
                        .join("anilist-covers")
                        .join(format!("{anilist_id}.{extension}")),
                );
            }
        }
    }
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
         WHERE episodes.pending_delete = 0
           AND (
              episodes.anime_id IS NOT excluded.anime_id
              OR episodes.root_folder_id IS NOT excluded.root_folder_id
              OR episodes.relative_path IS NOT excluded.relative_path
              OR episodes.file_name IS NOT excluded.file_name
              OR episodes.file_type IS NOT excluded.file_type
              OR episodes.episode_number IS NOT excluded.episode_number
              OR episodes.size IS NOT excluded.size
              OR episodes.missing != 0
           )",
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

fn build_rename_episode_plan(
    conn: &Connection,
    renames: &[RenameFileRequest],
) -> Result<Vec<RenameEpisodePlan>, String> {
    if renames.is_empty() {
        return Err("No files were selected for renaming.".to_string());
    }

    let roots = list_root_folders(conn)?;
    if roots.is_empty() {
        return Err("No root folders are configured.".to_string());
    }

    let mut source_keys = HashSet::new();
    let mut destination_keys = HashSet::new();
    let mut plans = Vec::with_capacity(renames.len());

    for request in renames {
        if request.old_path == request.new_path {
            return Err(format!("Rename request does not change the path: {}", request.old_path));
        }

        let old_path = PathBuf::from(&request.old_path);
        let new_path = PathBuf::from(&request.new_path);
        if !old_path.is_absolute() {
            return Err(format!("Source path is not absolute: {}", request.old_path));
        }
        if !new_path.is_absolute() {
            return Err(format!("Destination path is not absolute: {}", request.new_path));
        }

        let Some(root_folder) = roots
            .iter()
            .map(|root| PathBuf::from(&root.path))
            .find(|root| old_path.starts_with(root))
        else {
            return Err(format!(
                "Source file is not inside a configured root folder: {}",
                request.old_path
            ));
        };

        let old_key = windows_path_key(&request.old_path);
        let new_key = windows_path_key(&request.new_path);
        if old_key == new_key && request.old_path != request.new_path {
            return Err(format!(
                "Case-only renames are not supported in bulk mode: {}",
                request.old_path
            ));
        }
        if !source_keys.insert(old_key) {
            return Err(format!("Duplicate source path in rename request: {}", request.old_path));
        }
        if !destination_keys.insert(new_key) {
            return Err(format!(
                "Multiple files would be renamed to the same destination: {}",
                request.new_path
            ));
        }

        let source_metadata = fs::metadata(&old_path)
            .map_err(|e| format!("Source file is not available ({}): {e}", request.old_path))?;
        if !source_metadata.is_file() {
            return Err(format!("Source path is not a file: {}", request.old_path));
        }

        let Some(destination_parent) = new_path.parent() else {
            return Err(format!("Destination has no parent folder: {}", request.new_path));
        };
        if !destination_parent.exists() {
            return Err(format!(
                "Destination parent folder does not exist: {}",
                destination_parent.to_string_lossy()
            ));
        }
        if new_path.is_dir() {
            return Err(format!("Destination path is a folder: {}", request.new_path));
        }

        plans.push(RenameEpisodePlan {
            old_path,
            old_path_string: request.old_path.clone(),
            new_path,
            new_path_string: request.new_path.clone(),
            root_folder,
        });
    }

    for plan in &plans {
        ensure_rename_destination_not_in_database(conn, plan, &source_keys)?;
        if plan.new_path.exists() && !source_keys.contains(&windows_path_key(&plan.new_path_string)) {
            return Err(format!(
                "Destination already exists outside the rename set: {}",
                plan.new_path_string
            ));
        }
    }

    Ok(plans)
}

fn ensure_rename_destination_not_in_database(
    conn: &Connection,
    plan: &RenameEpisodePlan,
    source_keys: &HashSet<String>,
) -> Result<(), String> {
    let episode_path = conn
        .query_row(
            "SELECT path FROM episodes WHERE path = ?1",
            params![plan.new_path_string],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(path) = episode_path {
        if !source_keys.contains(&windows_path_key(&path)) {
            return Err(format!("Destination already belongs to another episode: {path}"));
        }
    }

    let unmatched_path = conn
        .query_row(
            "SELECT path FROM unmatched_files WHERE path = ?1",
            params![plan.new_path_string],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(path) = unmatched_path {
        if !source_keys.contains(&windows_path_key(&path)) {
            return Err(format!("Destination already belongs to another unmatched file: {path}"));
        }
    }

    Ok(())
}

fn normalize_anime_rename_title(title: &str) -> Result<String, String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err("Anime title cannot be empty.".to_string());
    }
    if trimmed.chars().any(|c| {
        c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
    }) {
        return Err(
            r#"Anime title cannot contain Windows filename characters: < > : " / \ | ? *"#
                .to_string(),
        );
    }
    Ok(trimmed.to_string())
}

fn build_anime_rename_plan(
    conn: &Connection,
    anime_id: i64,
    new_title: &str,
) -> Result<Vec<RenameEpisodePlan>, String> {
    let (current_title, current_title_key) = conn
        .query_row(
            "SELECT title, title_key FROM anime WHERE id = ?1",
            params![anime_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Anime does not exist: {anime_id}"))?;

    let new_title_key = scanner::title_key(new_title);
    if new_title_key.is_empty() {
        return Err("Anime title cannot be empty.".to_string());
    }

    if current_title == new_title && current_title_key == new_title_key {
        return Ok(Vec::new());
    }

    let existing_anime_id = conn
        .query_row(
            "SELECT id FROM anime WHERE title_key = ?1 AND id != ?2",
            params![new_title_key, anime_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(existing_anime_id) = existing_anime_id {
        return Err(format!(
            "Another anime already uses the title \"{new_title}\" (id {existing_anime_id})."
        ));
    }

    let roots = list_root_folders(conn)?;
    if roots.is_empty() {
        return Err("No root folders are configured.".to_string());
    }
    let rules = list_enabled_detection_rules(conn)?;
    if rules.is_empty() {
        return Err("No enabled anime detection rules are configured.".to_string());
    }

    let episodes = list_episodes_for_anime(conn, anime_id)?;
    if episodes.is_empty() {
        return Err("This anime has no available episode files to rename.".to_string());
    }

    let root_paths = roots
        .into_iter()
        .map(|root| PathBuf::from(root.path))
        .collect::<Vec<_>>();
    let mut plans = Vec::new();
    let mut destination_keys = HashSet::new();

    for episode in episodes {
        let old_path = PathBuf::from(&episode.path);
        if !old_path.is_absolute() {
            return Err(format!("Episode path is not absolute: {}", episode.path));
        }

        let Some(root_folder) = root_paths
            .iter()
            .find(|root| old_path.starts_with(root))
            .cloned()
        else {
            return Err(format!(
                "Episode file is not inside a configured root folder: {}",
                episode.path
            ));
        };

        let new_file_name =
            scanner::renamed_file_name_for_title(&episode.file_name, &rules, new_title)?
                .ok_or_else(|| {
                    format!(
                        "Episode filename no longer matches an enabled detection rule: {}",
                        episode.file_name
                    )
                })?;
        let reparsed_title_key = scanner::title_key_for_file_name(&new_file_name, &rules)?
            .ok_or_else(|| {
                format!(
                    "Renamed filename would not match an enabled detection rule: {new_file_name}"
                )
            })?;
        if reparsed_title_key != new_title_key {
            return Err(format!(
                "Renamed filename would scan as a different anime title: {new_file_name}"
            ));
        }

        let new_path = old_path.with_file_name(new_file_name);
        let new_path_string = new_path.to_string_lossy().to_string();
        if episode.path == new_path_string {
            continue;
        }
        if !destination_keys.insert(windows_path_key(&new_path_string)) {
            return Err(format!(
                "Multiple episodes would be renamed to the same destination: {new_path_string}"
            ));
        }

        plans.push(RenameEpisodePlan {
            old_path,
            old_path_string: episode.path,
            new_path,
            new_path_string,
            root_folder,
        });
    }

    let source_keys = plans
        .iter()
        .map(|plan| windows_path_key(&plan.old_path_string))
        .collect::<HashSet<_>>();
    for plan in &plans {
        ensure_rename_destination_not_in_database(conn, plan, &source_keys)?;
        if plan.new_path.exists() && !source_keys.contains(&windows_path_key(&plan.new_path_string))
        {
            return Err(format!(
                "Destination already exists outside the rename set: {}",
                plan.new_path_string
            ));
        }
    }

    Ok(plans)
}

#[derive(Debug)]
struct EpisodeTempMove {
    old_path: PathBuf,
    temp_path: PathBuf,
    new_path: PathBuf,
}

fn move_episode_sources_to_temps(plans: &[RenameEpisodePlan]) -> Result<Vec<EpisodeTempMove>, String> {
    let mut temp_moves = Vec::with_capacity(plans.len());

    for (index, plan) in plans.iter().enumerate() {
        let temp_path = unique_rename_temp_path(&plan.old_path, index)?;
        if let Err(error) = fs::rename(&plan.old_path, &temp_path) {
            rollback_episode_renames(&temp_moves);
            return Err(format!(
                "Failed to move {} to temporary rename path: {error}",
                plan.old_path_string
            ));
        }

        temp_moves.push(EpisodeTempMove {
            old_path: plan.old_path.clone(),
            temp_path,
            new_path: plan.new_path.clone(),
        });
    }

    Ok(temp_moves)
}

fn move_episode_temps_to_destinations(temp_moves: &[EpisodeTempMove]) -> Result<(), String> {
    for temp_move in temp_moves {
        if let Err(error) = fs::rename(&temp_move.temp_path, &temp_move.new_path) {
            return Err(format!(
                "Failed to move temporary rename path to {}: {error}",
                temp_move.new_path.to_string_lossy()
            ));
        }
    }

    Ok(())
}

fn rollback_episode_renames(temp_moves: &[EpisodeTempMove]) {
    for temp_move in temp_moves.iter().rev() {
        if temp_move.temp_path.exists() {
            let _ = fs::rename(&temp_move.temp_path, &temp_move.old_path);
        } else if temp_move.new_path.exists() {
            let _ = fs::rename(&temp_move.new_path, &temp_move.old_path);
        }
    }
}

fn rollback_episode_rename_destinations(temp_moves: &[EpisodeTempMove]) {
    for temp_move in temp_moves.iter().rev() {
        if temp_move.new_path.exists() {
            let _ = fs::rename(&temp_move.new_path, &temp_move.old_path);
        }
    }
}

fn persist_episode_renames(conn: &mut Connection, plans: &[RenameEpisodePlan]) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let temp_db_paths = plans
        .iter()
        .enumerate()
        .map(|(index, plan)| {
            format!(
                "__anime_player_rename_temp__:{}:{index}:{}",
                std::process::id(),
                plan.old_path_string
            )
        })
        .collect::<Vec<_>>();

    for (plan, temp_db_path) in plans.iter().zip(temp_db_paths.iter()) {
        tx.execute(
            "UPDATE episodes
             SET path = ?1
             WHERE path = ?2",
            params![temp_db_path, plan.old_path_string],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE unmatched_files
             SET path = ?1
             WHERE path = ?2",
            params![temp_db_path, plan.old_path_string],
        )
        .map_err(|e| e.to_string())?;
    }

    for (plan, temp_db_path) in plans.iter().zip(temp_db_paths.iter()) {
        let metadata = fs::metadata(&plan.new_path)
            .map_err(|e| format!("Failed to read renamed file metadata ({}): {e}", plan.new_path_string))?;
        let file_name = plan
            .new_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let file_type = plan
            .new_path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let relative_path = renamed_relative_path(&plan.new_path, &plan.root_folder, &plan.new_path_string);

        tx.execute(
            "UPDATE episodes
             SET path = ?1,
                 relative_path = ?2,
                 file_name = ?3,
                 file_type = ?4,
                 size = ?5,
                 updated_at = CURRENT_TIMESTAMP
             WHERE path = ?6",
            params![
                plan.new_path_string,
                relative_path,
                file_name,
                file_type,
                metadata.len() as i64,
                temp_db_path
            ],
        )
        .map_err(|e| e.to_string())?;

        tx.execute(
            "UPDATE unmatched_files
             SET path = ?1,
                 relative_path = ?2,
                 file_name = ?3,
                 detected_at = CURRENT_TIMESTAMP
             WHERE path = ?4",
            params![plan.new_path_string, relative_path, file_name, temp_db_path],
        )
        .map_err(|e| e.to_string())?;
    }

    tx.commit().map_err(|e| e.to_string())?;
    refresh_anime_latest_episode_at(conn)?;
    Ok(())
}

fn persist_anime_rename(
    conn: &mut Connection,
    anime_id: i64,
    new_title: &str,
    plans: &[RenameEpisodePlan],
) -> Result<(), String> {
    let new_title_key = scanner::title_key(new_title);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let temp_db_paths = plans
        .iter()
        .enumerate()
        .map(|(index, plan)| {
            format!(
                "__anime_player_rename_temp__:{}:{index}:{}",
                std::process::id(),
                plan.old_path_string
            )
        })
        .collect::<Vec<_>>();

    for (plan, temp_db_path) in plans.iter().zip(temp_db_paths.iter()) {
        tx.execute(
            "UPDATE episodes
             SET path = ?1
             WHERE path = ?2",
            params![temp_db_path, plan.old_path_string],
        )
        .map_err(|e| e.to_string())?;
    }

    for (plan, temp_db_path) in plans.iter().zip(temp_db_paths.iter()) {
        let metadata = fs::metadata(&plan.new_path).map_err(|e| {
            format!(
                "Failed to read renamed file metadata ({}): {e}",
                plan.new_path_string
            )
        })?;
        let file_name = plan
            .new_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let file_type = plan
            .new_path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let relative_path =
            renamed_relative_path(&plan.new_path, &plan.root_folder, &plan.new_path_string);

        tx.execute(
            "UPDATE episodes
             SET path = ?1,
                 relative_path = ?2,
                 file_name = ?3,
                 file_type = ?4,
                 size = ?5,
                 updated_at = CURRENT_TIMESTAMP
             WHERE path = ?6",
            params![
                plan.new_path_string,
                relative_path,
                file_name,
                file_type,
                metadata.len() as i64,
                temp_db_path
            ],
        )
        .map_err(|e| e.to_string())?;
    }

    let changed = tx
        .execute(
            "UPDATE anime
             SET title = ?1,
                 title_key = ?2,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?3",
            params![new_title, new_title_key, anime_id],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("Anime does not exist: {anime_id}"));
    }

    tx.commit().map_err(|e| e.to_string())?;
    refresh_anime_latest_episode_at(conn)?;
    Ok(())
}

fn unique_rename_temp_path(source: &Path, index: usize) -> Result<PathBuf, String> {
    let parent = source
        .parent()
        .ok_or_else(|| format!("Source path has no parent folder: {}", source.to_string_lossy()))?;
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("episode");

    for attempt in 0..1000 {
        let candidate = parent.join(format!(
            ".anime-player-rename-{}-{index}-{attempt}-{file_name}",
            std::process::id()
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "Could not create a unique temporary rename path for {}",
        source.to_string_lossy()
    ))
}

fn renamed_relative_path(path: &Path, root_folder: &Path, fallback: &str) -> String {
    if let Ok(relative) = path.strip_prefix(root_folder) {
        return relative.to_string_lossy().replace('\\', "/");
    }

    fallback.replace('\\', "/")
}

fn windows_path_key(path: &str) -> String {
    path.replace('/', "\\").to_ascii_lowercase()
}

fn collect_rows<T>(rows: impl Iterator<Item = rusqlite::Result<T>>) -> Result<Vec<T>, String> {
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}
