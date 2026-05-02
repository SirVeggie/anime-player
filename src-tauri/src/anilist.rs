use std::fs;
use std::path::PathBuf;

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
const TOKEN_KEY: &str = "anilist.access_token";
const VIEWER_ID_KEY: &str = "anilist.viewer_id";
const VIEWER_NAME_KEY: &str = "anilist.viewer_name";

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

#[tauri::command]
pub fn get_anilist_auth_state(db: State<'_, AppDatabase>) -> Result<AnilistAuthState, String> {
    db.with_conn(auth_state)
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
    let client_id = client_id.ok_or("Set an AniList client ID before logging in.")?;
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

    db.with_conn(|conn| {
        let changed = conn
            .execute(
                "UPDATE anime
                 SET anilist_id = ?1,
                     anilist_title = ?2,
                     anilist_site_url = ?3,
                     anilist_cover_path = ?4,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?5",
                params![
                    media.id,
                    media.title,
                    media.site_url,
                    cover_path.map(|path| path.to_string_lossy().to_string()),
                    anime_id
                ],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("Anime does not exist: {anime_id}"));
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
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![anime_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn get_anilist_cover_image(
    db: State<'_, AppDatabase>,
    anime_id: i64,
) -> Result<Option<String>, String> {
    let path = db.with_conn(|conn| {
        conn.query_row(
            "SELECT anilist_cover_path FROM anime WHERE id = ?1",
            params![anime_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(|path| path.flatten())
        .map_err(|e| e.to_string())
    })?;

    let Some(path) = path else {
        return Ok(None);
    };
    let bytes = fs::read(&path).map_err(|e| format!("failed to read AniList cover {path}: {e}"))?;
    let mime = if path.to_ascii_lowercase().ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    };
    Ok(Some(format!(
        "data:{mime};base64,{}",
        general_purpose::STANDARD.encode(bytes)
    )))
}

fn access_token_from_callback(callback_url: &str) -> Result<String, String> {
    let url = Url::parse(callback_url).map_err(|e| format!("Invalid AniList callback URL: {e}"))?;
    if url.scheme() != "anime-player" || url.host_str() != Some("anilist-auth") {
        return Err(format!("Unexpected AniList callback URL. Expected {REDIRECT_URI}."));
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
    let client_id = get_setting(conn, CLIENT_ID_KEY)?;
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
    let cover_dir = db
        .path()
        .parent()
        .ok_or("failed to resolve database data directory")?
        .join("anilist-covers");
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
