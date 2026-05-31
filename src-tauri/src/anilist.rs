use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose, Engine as _};
use reqwest::Client;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;
use url::{form_urlencoded, Url};

use crate::db::AppDatabase;

const GRAPHQL_URL: &str = "https://graphql.anilist.co";
const AUTHORIZE_URL: &str = "https://anilist.co/api/v2/oauth/authorize";
const REDIRECT_URI: &str = "anime-player://anilist-auth";
const CLIENT_ID_KEY: &str = "anilist.client_id";
const DEFAULT_CLIENT_ID: &str = "40455";
const TOKEN_KEY: &str = "anilist.access_token";
const VIEWER_ID_KEY: &str = "anilist.viewer_id";
const VIEWER_NAME_KEY: &str = "anilist.viewer_name";
const MEDIA_STATUS_CACHE_TTL_SECONDS: i64 = 5 * 60;
const ANILIST_COVER_DIR: &str = "anilist-covers";

#[derive(Debug, Serialize)]
pub struct AnilistAuthState {
    client_id: Option<String>,
    viewer_id: Option<i64>,
    viewer_name: Option<String>,
    authenticated: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnilistSearchResult {
    id: i64,
    title: String,
    native_title: Option<String>,
    format: Option<String>,
    status: Option<String>,
    episodes: Option<i64>,
    season_year: Option<i64>,
    cover_image_url: Option<String>,
    site_url: String,
}

#[derive(Debug, Serialize)]
pub struct AnilistProgressSyncResult {
    synced: bool,
    reason: Option<String>,
    remote_progress: Option<i64>,
    target_progress: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AnilistMediaStatus {
    progress: Option<i64>,
    episodes: Option<i64>,
    score: Option<f64>,
    /// AniList `MediaStatus` value, e.g. `RELEASING` or `FINISHED`.
    status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AnilistLocalProgressApplyResult {
    progress: i64,
    updated_episodes: i64,
}

#[tauri::command]
pub fn get_anilist_auth_state(db: State<'_, AppDatabase>) -> Result<AnilistAuthState, String> {
    db.with_conn(|conn| auth_state(conn))
}

#[tauri::command]
pub fn set_anilist_client_id(
    db: State<'_, AppDatabase>,
    client_id: String,
) -> Result<AnilistAuthState, String> {
    let trimmed = client_id.trim();
    db.with_conn(|conn| {
        if trimmed.is_empty() {
            delete_setting(conn, CLIENT_ID_KEY)?;
        } else {
            set_setting(conn, CLIENT_ID_KEY, trimmed)?;
        }
        auth_state(conn)
    })
}

#[tauri::command]
pub fn get_anilist_login_url(db: State<'_, AppDatabase>) -> Result<String, String> {
    let client_id = db.with_conn(|conn| get_setting(conn, CLIENT_ID_KEY))?;
    let client_id = client_id.unwrap_or_else(|| DEFAULT_CLIENT_ID.to_string());
    let mut url = Url::parse(AUTHORIZE_URL).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("response_type", "token");
    Ok(url.to_string())
}

#[tauri::command]
pub async fn complete_anilist_login(
    db: State<'_, AppDatabase>,
    callback_url: String,
) -> Result<AnilistAuthState, String> {
    let token = access_token_from_callback(&callback_url)?;
    let viewer = validate_token(&token).await?;
    db.with_conn(|conn| {
        set_setting(conn, TOKEN_KEY, &token)?;
        set_setting(conn, VIEWER_ID_KEY, &viewer.id.to_string())?;
        set_setting(conn, VIEWER_NAME_KEY, &viewer.name)?;
        auth_state(conn)
    })
}

#[tauri::command]
pub fn logout_anilist(db: State<'_, AppDatabase>) -> Result<AnilistAuthState, String> {
    db.with_conn(|conn| {
        delete_setting(conn, TOKEN_KEY)?;
        delete_setting(conn, VIEWER_ID_KEY)?;
        delete_setting(conn, VIEWER_NAME_KEY)?;
        auth_state(conn)
    })
}

#[tauri::command]
pub async fn search_anilist_anime(
    db: State<'_, AppDatabase>,
    query: String,
) -> Result<Vec<AnilistSearchResult>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let token = db.with_conn(|conn| get_setting(conn, TOKEN_KEY))?;
    search_anime(token.as_deref(), query).await
}

#[tauri::command]
pub async fn link_anime_anilist(
    db: State<'_, AppDatabase>,
    anime_id: i64,
    anilist_id: i64,
) -> Result<(), String> {
    let token = db.with_conn(|conn| get_setting(conn, TOKEN_KEY))?;
    let media = get_anime(token.as_deref(), anilist_id).await?;
    let cover_path = if let Some(url) = media.cover_image_url.as_deref() {
        download_cover(&db, media.id, url).await.ok()
    } else {
        None
    };
    let cover_path = cover_path
        .as_deref()
        .map(|path| cover_path_for_storage(&db, path))
        .transpose()?;

    db.with_conn(|conn| {
        let changed = conn
            .execute(
                "UPDATE anime
                 SET anilist_id = ?1,
                     anilist_title = ?2,
                     anilist_site_url = ?3,
                     anilist_cover_path = ?4,
                     anilist_cached_progress = NULL,
                     anilist_cached_episodes = NULL,
                     anilist_cached_score = NULL,
                     anilist_cached_status = NULL,
                     anilist_status_fetched_at = NULL,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?5",
                params![media.id, media.title, media.site_url, cover_path, anime_id],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("Title does not exist: {anime_id}"));
        }
        Ok(())
    })
}

#[tauri::command]
pub fn unlink_anime_anilist(db: State<'_, AppDatabase>, anime_id: i64) -> Result<(), String> {
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE anime
             SET anilist_id = NULL,
                 anilist_title = NULL,
                 anilist_site_url = NULL,
                 anilist_cover_path = NULL,
                 anilist_cached_progress = NULL,
                 anilist_cached_episodes = NULL,
                 anilist_cached_score = NULL,
                 anilist_cached_status = NULL,
                 anilist_status_fetched_at = NULL,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![anime_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub async fn get_anilist_cover_image(
    db: State<'_, AppDatabase>,
    anime_id: i64,
) -> Result<Option<String>, String> {
    let cover = db.with_conn(|conn| {
        conn.query_row(
            "SELECT anilist_cover_path, anilist_id FROM anime WHERE id = ?1",
            params![anime_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    })?;

    let Some((stored_path, anilist_id)) = cover else {
        return Ok(None);
    };

    if let Some(stored_path) = stored_path.as_deref() {
        let path = stored_cover_path(&db, stored_path)?;
        if path.exists() {
            update_anilist_cover_path(&db, anime_id, &path)?;
            return read_cover_data_url(&path).map(Some);
        }
    }

    if let Some(anilist_id) = anilist_id {
        if let Some(path) = existing_cover_for_anilist_id(&db, anilist_id)? {
            update_anilist_cover_path(&db, anime_id, &path)?;
            return read_cover_data_url(&path).map(Some);
        }

        let token = db.with_conn(|conn| get_setting(conn, TOKEN_KEY))?;
        let media = get_anime(token.as_deref(), anilist_id).await?;
        if let Some(url) = media.cover_image_url.as_deref() {
            let path = download_cover(&db, media.id, url).await?;
            update_anilist_cover_path(&db, anime_id, &path)?;
            return read_cover_data_url(&path).map(Some);
        }
    }

    Ok(None)
}

#[tauri::command]
pub async fn sync_anilist_episode_progress(
    db: State<'_, AppDatabase>,
    episode_id: i64,
) -> Result<AnilistProgressSyncResult, String> {
    let token = db.with_conn(|conn| get_setting(conn, TOKEN_KEY))?;
    let Some(token) = token else {
        return Ok(AnilistProgressSyncResult::skipped(
            "AniList is not logged in.",
            None,
            None,
        ));
    };
    let target = db.with_conn(|conn| progress_target_for_episode(conn, episode_id))?;
    let Some(target) = target else {
        return Ok(AnilistProgressSyncResult::skipped(
            "Episode is not linked to AniList or has no parsed episode number.",
            None,
            None,
        ));
    };

    let remote_progress = match fresh_cached_media_status(&db, target.anime_id)? {
        Some(status) => status.progress.unwrap_or(0),
        None => {
            let status = fetch_and_cache_media_status(&db, &token, target.anime_id, target.anilist_id).await?;
            status.progress.unwrap_or(0)
        }
    };
    if remote_progress >= target.progress {
        return Ok(AnilistProgressSyncResult::skipped(
            "AniList progress is already at or beyond this episode.",
            Some(remote_progress),
            Some(target.progress),
        ));
    }

    let saved_progress = save_remote_progress(&token, target.anilist_id, target.progress).await?;
    db.with_conn(|conn| cache_anilist_progress(conn, target.anime_id, saved_progress))?;
    Ok(AnilistProgressSyncResult {
        synced: true,
        reason: None,
        remote_progress: Some(saved_progress),
        target_progress: Some(target.progress),
    })
}

#[tauri::command]
pub async fn get_anilist_media_status(
    db: State<'_, AppDatabase>,
    anime_id: i64,
) -> Result<Option<AnilistMediaStatus>, String> {
    let Some((token, anilist_id)) = auth_and_media_id_for_anime(&db, anime_id)? else {
        return Ok(None);
    };
    cached_or_fetch_media_status(&db, &token, anime_id, anilist_id)
        .await
        .map(Some)
}

#[tauri::command]
pub async fn set_anilist_media_progress(
    db: State<'_, AppDatabase>,
    anime_id: i64,
    progress: i64,
) -> Result<AnilistMediaStatus, String> {
    let (token, anilist_id) = auth_and_media_id_for_anime(&db, anime_id)?
        .ok_or("Title is not linked to AniList or AniList is not logged in.")?;
    let progress = progress.max(0);
    let saved_progress = save_remote_progress(&token, anilist_id, progress).await?;
    db.with_conn(|conn| cache_anilist_progress(conn, anime_id, saved_progress))?;
    if let Some(status) = fresh_cached_media_status(&db, anime_id)? {
        return Ok(status);
    }
    Ok(AnilistMediaStatus {
        progress: Some(saved_progress),
        episodes: None,
        score: None,
        status: None,
    })
}

#[tauri::command]
pub async fn apply_anilist_progress_to_local(
    db: State<'_, AppDatabase>,
    anime_id: i64,
) -> Result<Option<AnilistLocalProgressApplyResult>, String> {
    let Some((token, anilist_id)) = auth_and_media_id_for_anime(&db, anime_id)? else {
        return Ok(None);
    };
    let status = cached_or_fetch_media_status(&db, &token, anime_id, anilist_id).await?;
    let progress = status.progress.unwrap_or(0).max(0);
    if progress == 0 {
        return Ok(Some(AnilistLocalProgressApplyResult {
            progress,
            updated_episodes: 0,
        }));
    }

    let updated_episodes = db.with_conn(|conn| {
        let tracker_offset = conn
            .query_row(
                "SELECT tracker_offset FROM anime WHERE id = ?1",
                params![anime_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE episodes
             SET watched = 1,
                 position_seconds = CASE
                     WHEN duration_seconds > 0 THEN duration_seconds
                     ELSE position_seconds
                 END,
                 updated_at = CURRENT_TIMESTAMP
             WHERE anime_id = ?1
               AND episode_number >= 1
               AND (CAST(episode_number AS INTEGER) - ?2) BETWEEN 1 AND ?3
               AND (
                   watched = 0
                   OR (duration_seconds > 0 AND position_seconds < duration_seconds)
               )",
            params![anime_id, tracker_offset, progress],
        )
        .map(|count| count as i64)
        .map_err(|e| e.to_string())
    })?;

    Ok(Some(AnilistLocalProgressApplyResult {
        progress,
        updated_episodes,
    }))
}

#[tauri::command]
pub async fn set_anilist_media_score(
    db: State<'_, AppDatabase>,
    anime_id: i64,
    score: Option<f64>,
) -> Result<AnilistMediaStatus, String> {
    let (token, anilist_id) = auth_and_media_id_for_anime(&db, anime_id)?
        .ok_or("Title is not linked to AniList or AniList is not logged in.")?;
    let status = save_remote_score(&token, anilist_id, score).await?;
    db.with_conn(|conn| cache_anilist_media_status(conn, anime_id, &status))?;
    Ok(status)
}

fn access_token_from_callback(callback_url: &str) -> Result<String, String> {
    let url = Url::parse(callback_url).map_err(|e| format!("Invalid AniList callback URL: {e}"))?;
    if url.scheme() != "anime-player" || url.host_str() != Some("anilist-auth") {
        return Err(format!(
            "Unexpected AniList callback URL. Expected {REDIRECT_URI}."
        ));
    }
    let fragment = url
        .fragment()
        .ok_or("AniList callback did not include an access token.")?;
    let pairs = form_urlencoded::parse(fragment.as_bytes());
    for (key, value) in pairs {
        if key == "access_token" && !value.is_empty() {
            return Ok(value.into_owned());
        }
    }
    Err("AniList callback did not include an access token.".to_string())
}

fn auth_state(conn: &Connection) -> Result<AnilistAuthState, String> {
    let client_id = Some(
        get_setting(conn, CLIENT_ID_KEY)?.unwrap_or_else(|| DEFAULT_CLIENT_ID.to_string()),
    );
    let viewer_id = get_setting(conn, VIEWER_ID_KEY)?.and_then(|value| value.parse().ok());
    let viewer_name = get_setting(conn, VIEWER_NAME_KEY)?;
    let authenticated = get_setting(conn, TOKEN_KEY)?.is_some();
    Ok(AnilistAuthState {
        client_id,
        viewer_id,
        viewer_name,
        authenticated,
    })
}

fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value)
         VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn delete_setting(conn: &Connection, key: &str) -> Result<(), String> {
    conn.execute("DELETE FROM settings WHERE key = ?1", params![key])
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn progress_target_for_episode(
    conn: &Connection,
    episode_id: i64,
) -> Result<Option<AnilistProgressTarget>, String> {
    conn.query_row(
        "SELECT e.anime_id, a.anilist_id, a.tracker_offset, e.episode_number
         FROM episodes e
         JOIN anime a ON a.id = e.anime_id
         WHERE e.id = ?1",
        params![episode_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<f64>>(3)?,
            ))
        },
    )
    .optional()
    .map_err(|e| e.to_string())
    .map(|row| {
        row.and_then(|(anime_id, anilist_id, tracker_offset, episode_number)| {
            let anilist_id = anilist_id?;
            let progress = episode_number?.floor() as i64 - tracker_offset;
            (progress > 0).then_some(AnilistProgressTarget {
                anime_id,
                anilist_id,
                progress,
            })
        })
    })
}

fn auth_and_media_id_for_anime(
    db: &AppDatabase,
    anime_id: i64,
) -> Result<Option<(String, i64)>, String> {
    db.with_conn(|conn| {
        let token = get_setting(conn, TOKEN_KEY)?;
        let anilist_id = conn
            .query_row(
                "SELECT anilist_id FROM anime WHERE id = ?1",
                params![anime_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .flatten();
        Ok(token.zip(anilist_id))
    })
}

fn now_seconds() -> Result<i64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|e| format!("system clock is before Unix epoch: {e}"))
}

fn fresh_cached_media_status(
    db: &AppDatabase,
    anime_id: i64,
) -> Result<Option<AnilistMediaStatus>, String> {
    let now = now_seconds()?;
    db.with_conn(|conn| cached_media_status(conn, anime_id, now))
}

fn cached_media_status(
    conn: &Connection,
    anime_id: i64,
    now: i64,
) -> Result<Option<AnilistMediaStatus>, String> {
    let row = conn
        .query_row(
            "SELECT anilist_cached_progress,
                    anilist_cached_episodes,
                    anilist_cached_score,
                    anilist_cached_status,
                    anilist_status_fetched_at
             FROM anime
             WHERE id = ?1",
            params![anime_id],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<f64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;

    let Some((progress, episodes, score, status, fetched_at)) = row else {
        return Ok(None);
    };
    let Some(fetched_at) = fetched_at else {
        return Ok(None);
    };
    if now.saturating_sub(fetched_at) >= MEDIA_STATUS_CACHE_TTL_SECONDS {
        return Ok(None);
    }
    Ok(Some(AnilistMediaStatus {
        progress,
        episodes,
        score,
        status,
    }))
}

async fn cached_or_fetch_media_status(
    db: &AppDatabase,
    token: &str,
    anime_id: i64,
    anilist_id: i64,
) -> Result<AnilistMediaStatus, String> {
    if let Some(status) = fresh_cached_media_status(db, anime_id)? {
        return Ok(status);
    }
    fetch_and_cache_media_status(db, token, anime_id, anilist_id).await
}

async fn fetch_and_cache_media_status(
    db: &AppDatabase,
    token: &str,
    anime_id: i64,
    anilist_id: i64,
) -> Result<AnilistMediaStatus, String> {
    let status = get_media_status(token, anilist_id).await?;
    db.with_conn(|conn| cache_anilist_media_status(conn, anime_id, &status))?;
    Ok(status)
}

fn cache_anilist_media_status(
    conn: &Connection,
    anime_id: i64,
    status: &AnilistMediaStatus,
) -> Result<(), String> {
    let fetched_at = now_seconds()?;
    conn.execute(
        "UPDATE anime
         SET anilist_cached_progress = ?1,
             anilist_cached_episodes = ?2,
             anilist_cached_score = ?3,
             anilist_cached_status = ?4,
             anilist_status_fetched_at = ?5
         WHERE id = ?6",
        params![
            status.progress,
            status.episodes,
            status.score,
            status.status,
            fetched_at,
            anime_id
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn cache_anilist_progress(
    conn: &Connection,
    anime_id: i64,
    progress: i64,
) -> Result<(), String> {
    let fetched_at = now_seconds()?;
    conn.execute(
        "UPDATE anime
         SET anilist_cached_progress = ?1,
             anilist_status_fetched_at = ?2
         WHERE id = ?3",
        params![progress, fetched_at, anime_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn validate_token(token: &str) -> Result<Viewer, String> {
    #[derive(Debug, Deserialize)]
    struct ViewerData {
        #[serde(rename = "Viewer")]
        viewer: Viewer,
    }

    let data: ViewerData = graphql(
        Some(token),
        "query ViewerName { Viewer { id name } }",
        json!({}),
    )
    .await?;
    Ok(data.viewer)
}

async fn get_media_status(token: &str, anilist_id: i64) -> Result<AnilistMediaStatus, String> {
    #[derive(Debug, Deserialize)]
    struct MediaData {
        #[serde(rename = "Media")]
        media: StatusMedia,
    }

    let data: MediaData = graphql(
        Some(token),
        r#"
        query MediaStatus($id: Int!) {
          Media(id: $id, type: ANIME) {
            status
            episodes
            mediaListEntry {
              progress
              score
            }
          }
        }
        "#,
        json!({ "id": anilist_id }),
    )
    .await?;
    Ok(data.media.into())
}

async fn save_remote_progress(token: &str, anilist_id: i64, progress: i64) -> Result<i64, String> {
    #[derive(Debug, Deserialize)]
    struct SaveData {
        #[serde(rename = "SaveMediaListEntry")]
        save_media_list_entry: SavedProgressEntry,
    }
    #[derive(Debug, Deserialize)]
    struct SavedProgressEntry {
        progress: Option<i64>,
    }

    let data: SaveData = graphql(
        Some(token),
        r#"
        mutation SyncProgress($mediaId: Int!, $progress: Int!) {
          SaveMediaListEntry(mediaId: $mediaId, progress: $progress) {
            progress
          }
        }
        "#,
        json!({ "mediaId": anilist_id, "progress": progress }),
    )
    .await?;
    Ok(data.save_media_list_entry.progress.unwrap_or(progress))
}

async fn save_remote_score(
    token: &str,
    anilist_id: i64,
    score: Option<f64>,
) -> Result<AnilistMediaStatus, String> {
    #[derive(Debug, Deserialize)]
    struct SaveData {
        #[serde(rename = "SaveMediaListEntry")]
        save_media_list_entry: SavedStatusEntry,
    }
    #[derive(Debug, Deserialize)]
    struct SavedStatusEntry {
        progress: Option<i64>,
        score: Option<f64>,
        media: Option<StatusMediaSummary>,
    }
    #[derive(Debug, Deserialize)]
    struct StatusMediaSummary {
        episodes: Option<i64>,
    }

    let data: SaveData = graphql(
        Some(token),
        r#"
        mutation SetScore($mediaId: Int!, $score: Float) {
          SaveMediaListEntry(mediaId: $mediaId, score: $score) {
            progress
            score
            media {
              episodes
            }
          }
        }
        "#,
        json!({ "mediaId": anilist_id, "score": score }),
    )
    .await?;
    Ok(AnilistMediaStatus {
        progress: data.save_media_list_entry.progress,
        episodes: data
            .save_media_list_entry
            .media
            .and_then(|media| media.episodes),
        score: data.save_media_list_entry.score,
        status: None,
    })
}

async fn search_anime(
    token: Option<&str>,
    search: &str,
) -> Result<Vec<AnilistSearchResult>, String> {
    #[derive(Debug, Deserialize)]
    struct SearchData {
        #[serde(rename = "Page")]
        page: SearchPage,
    }
    #[derive(Debug, Deserialize)]
    struct SearchPage {
        media: Vec<Media>,
    }

    let data: SearchData = graphql(
        token,
        r#"
        query SearchAnime($search: String!) {
          Page(page: 1, perPage: 8) {
            media(search: $search, type: ANIME, sort: SEARCH_MATCH) {
              id
              title { romaji english native }
              format
              status
              episodes
              seasonYear
              coverImage { large extraLarge }
              siteUrl
            }
          }
        }
        "#,
        json!({ "search": search }),
    )
    .await?;
    Ok(data.page.media.into_iter().map(Into::into).collect())
}

async fn get_anime(token: Option<&str>, id: i64) -> Result<AnilistSearchResult, String> {
    #[derive(Debug, Deserialize)]
    struct MediaData {
        #[serde(rename = "Media")]
        media: Media,
    }

    let data: MediaData = graphql(
        token,
        r#"
        query AnimeById($id: Int!) {
          Media(id: $id, type: ANIME) {
            id
            title { romaji english native }
            format
            status
            episodes
            seasonYear
            coverImage { large extraLarge }
            siteUrl
          }
        }
        "#,
        json!({ "id": id }),
    )
    .await?;
    Ok(data.media.into())
}

fn data_dir(db: &AppDatabase) -> Result<&Path, String> {
    db.path()
        .parent()
        .ok_or("failed to resolve database data directory".to_string())
}

fn cover_dir(db: &AppDatabase) -> Result<PathBuf, String> {
    Ok(data_dir(db)?.join(ANILIST_COVER_DIR))
}

fn stored_cover_path(db: &AppDatabase, stored_path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(stored_path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(data_dir(db)?.join(path))
    }
}

fn cover_path_for_storage(db: &AppDatabase, path: &Path) -> Result<String, String> {
    let data_dir = data_dir(db)?;
    let path = path
        .strip_prefix(data_dir)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf());
    Ok(path.to_string_lossy().replace('\\', "/"))
}

fn update_anilist_cover_path(db: &AppDatabase, anime_id: i64, path: &Path) -> Result<(), String> {
    let stored_path = cover_path_for_storage(db, path)?;
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE anime
             SET anilist_cover_path = ?1,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?2
               AND anilist_cover_path IS NOT ?1",
            params![stored_path, anime_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

fn existing_cover_for_anilist_id(
    db: &AppDatabase,
    anilist_id: i64,
) -> Result<Option<PathBuf>, String> {
    let cover_dir = cover_dir(db)?;
    for extension in ["jpg", "png", "jpeg", "webp"] {
        let path = cover_dir.join(format!("{anilist_id}.{extension}"));
        if path.exists() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn read_cover_data_url(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|e| format!("failed to read AniList cover {}: {e}", path.display()))?;
    let mime = match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    };
    Ok(format!(
        "data:{mime};base64,{}",
        general_purpose::STANDARD.encode(bytes)
    ))
}

async fn graphql<T: DeserializeOwned>(
    token: Option<&str>,
    query: &str,
    variables: Value,
) -> Result<T, String> {
    let client = Client::new();
    let mut request = client
        .post(GRAPHQL_URL)
        .header("Accept", "application/json")
        .json(&json!({ "query": query, "variables": variables }));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("AniList request failed: {e}"))?;
    let status = response.status();
    let body: GraphqlResponse<T> = response
        .json()
        .await
        .map_err(|e| format!("AniList returned an invalid response: {e}"))?;
    if !status.is_success() {
        return Err(body
            .error_message()
            .unwrap_or_else(|| format!("AniList request failed with status {status}")));
    }
    let error_message = body.error_message();
    body.data
        .ok_or_else(|| error_message.unwrap_or("AniList returned no data.".to_string()))
}

async fn download_cover(
    db: &AppDatabase,
    anilist_id: i64,
    cover_url: &str,
) -> Result<PathBuf, String> {
    let cover_dir = cover_dir(db)?;
    fs::create_dir_all(&cover_dir)
        .map_err(|e| format!("failed to create AniList cover directory: {e}"))?;

    let extension = if cover_url.to_ascii_lowercase().contains(".png") {
        "png"
    } else {
        "jpg"
    };
    let path = cover_dir.join(format!("{anilist_id}.{extension}"));
    let bytes = Client::new()
        .get(cover_url)
        .send()
        .await
        .map_err(|e| format!("failed to download AniList cover: {e}"))?
        .error_for_status()
        .map_err(|e| format!("failed to download AniList cover: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("failed to read AniList cover: {e}"))?;
    fs::write(&path, bytes).map_err(|e| format!("failed to save AniList cover: {e}"))?;
    Ok(path)
}

#[derive(Debug, Deserialize)]
struct Viewer {
    id: i64,
    name: String,
}

struct AnilistProgressTarget {
    anime_id: i64,
    anilist_id: i64,
    progress: i64,
}

#[derive(Debug, Deserialize)]
struct StatusMedia {
    status: Option<String>,
    episodes: Option<i64>,
    #[serde(rename = "mediaListEntry")]
    media_list_entry: Option<StatusMediaListEntry>,
}

#[derive(Debug, Deserialize)]
struct StatusMediaListEntry {
    progress: Option<i64>,
    score: Option<f64>,
}

impl From<StatusMedia> for AnilistMediaStatus {
    fn from(media: StatusMedia) -> Self {
        let entry = media.media_list_entry;
        Self {
            progress: entry.as_ref().and_then(|entry| entry.progress),
            episodes: media.episodes,
            score: entry.and_then(|entry| entry.score),
            status: media.status,
        }
    }
}

impl AnilistProgressSyncResult {
    fn skipped(
        reason: impl Into<String>,
        remote_progress: Option<i64>,
        target_progress: Option<i64>,
    ) -> Self {
        Self {
            synced: false,
            reason: Some(reason.into()),
            remote_progress,
            target_progress,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Media {
    id: i64,
    title: MediaTitle,
    format: Option<String>,
    status: Option<String>,
    episodes: Option<i64>,
    #[serde(rename = "seasonYear")]
    season_year: Option<i64>,
    #[serde(rename = "coverImage")]
    cover_image: Option<CoverImage>,
    #[serde(rename = "siteUrl")]
    site_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MediaTitle {
    romaji: Option<String>,
    english: Option<String>,
    native: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CoverImage {
    large: Option<String>,
    #[serde(rename = "extraLarge")]
    extra_large: Option<String>,
}

impl From<Media> for AnilistSearchResult {
    fn from(media: Media) -> Self {
        let title = media
            .title
            .english
            .clone()
            .or(media.title.romaji.clone())
            .or(media.title.native.clone())
            .unwrap_or_else(|| format!("AniList #{}", media.id));
        Self {
            id: media.id,
            title,
            native_title: media.title.native,
            format: media.format,
            status: media.status,
            episodes: media.episodes,
            season_year: media.season_year,
            cover_image_url: media
                .cover_image
                .and_then(|cover| cover.extra_large.or(cover.large)),
            site_url: media
                .site_url
                .unwrap_or_else(|| format!("https://anilist.co/anime/{}", media.id)),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GraphqlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphqlError>>,
}

impl<T> GraphqlResponse<T> {
    fn error_message(&self) -> Option<String> {
        self.errors.as_ref().map(|errors| {
            errors
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        })
    }
}

#[derive(Debug, Deserialize)]
struct GraphqlError {
    message: String,
}
