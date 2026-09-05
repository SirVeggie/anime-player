use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[cfg(windows)]
use crate::db::AppDatabase;
#[cfg(windows)]
use crate::mpv::MpvTrack;
#[cfg(windows)]
use crate::AppState;
#[cfg(windows)]
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrackPref {
    pub audio_lang: Option<String>,
    pub audio_title: Option<String>,
    pub subtitle_off: bool,
    pub subtitle_lang: Option<String>,
    pub subtitle_title: Option<String>,
    pub subtitle_external_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TrackCandidate {
    pub id: i64,
    pub lang: Option<String>,
    pub title: Option<String>,
}

const LANG_MATCH: i32 = 100;
const TITLE_EXACT: i32 = 50;
const TITLE_CONTAINS: i32 = 25;
const TOKEN_OVERLAP: i32 = 5;
const SIGNS_ALIGN: i32 = 20;
const SIGNS_MISMATCH: i32 = -40;
const SIGNS_PREF_ONLY: i32 = -20;
const MIN_SCORE: i32 = 50;

pub fn normalize_lang(lang: &str) -> String {
    match lang.trim().to_ascii_lowercase().as_str() {
        "jpn" | "jp" | "japanese" | "ja" => "ja".into(),
        "eng" | "en" | "english" => "en".into(),
        "spa" | "es" | "spanish" | "castilian" => "es".into(),
        "por" | "pt" | "portuguese" => "pt".into(),
        "fre" | "fra" | "fr" | "french" => "fr".into(),
        "ger" | "deu" | "de" | "german" => "de".into(),
        "ita" | "it" | "italian" => "it".into(),
        "kor" | "ko" | "korean" => "ko".into(),
        "chi" | "zho" | "zh" | "chinese" | "cmn" | "yue" => "zh".into(),
        "rus" | "ru" | "russian" => "ru".into(),
        "ara" | "ar" | "arabic" => "ar".into(),
        "hin" | "hi" | "hindi" => "hi".into(),
        "tha" | "th" | "thai" => "th".into(),
        "vie" | "vi" | "vietnamese" => "vi".into(),
        "pol" | "pl" | "polish" => "pl".into(),
        "und" | "unknown" | "" => String::new(),
        other => other.to_string(),
    }
}

pub fn title_key(title: &str) -> String {
    let mut out = String::new();
    let mut last_space = true;
    for ch in title.chars() {
        let next = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            ' '
        };
        if next == ' ' {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(next);
            last_space = false;
        }
    }
    out.trim().to_string()
}

pub fn is_signs_like(title: &str) -> bool {
    title_key(title)
        .split_whitespace()
        .any(|word| matches!(word, "signs" | "sign" | "songs" | "song" | "forced"))
}

fn tokens(title: &str) -> Vec<String> {
    title_key(title)
        .split_whitespace()
        .filter(|word| word.len() > 1)
        .map(str::to_string)
        .collect()
}

fn optional_norm_lang(lang: Option<&str>) -> Option<String> {
    let value = normalize_lang(lang.unwrap_or(""));
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub fn score_track(
    pref_lang: Option<&str>,
    pref_title: Option<&str>,
    candidate: &TrackCandidate,
) -> Option<i32> {
    let pref_lang = optional_norm_lang(pref_lang);
    let cand_lang = optional_norm_lang(candidate.lang.as_deref());
    if let (Some(preferred), Some(actual)) = (pref_lang.as_ref(), cand_lang.as_ref()) {
        if preferred != actual {
            return None;
        }
    }

    let mut score = 0;
    if pref_lang.is_some() && cand_lang.is_some() {
        score += LANG_MATCH;
    }

    let pref_title = pref_title.unwrap_or("").trim();
    let cand_title = candidate.title.as_deref().unwrap_or("").trim();
    let pref_key = title_key(pref_title);
    let cand_key = title_key(cand_title);
    if !pref_key.is_empty() && !cand_key.is_empty() {
        if pref_key == cand_key {
            score += TITLE_EXACT;
        } else if pref_key.contains(&cand_key) || cand_key.contains(&pref_key) {
            score += TITLE_CONTAINS;
        }
        let pref_tokens = tokens(pref_title);
        let cand_tokens = tokens(cand_title);
        let overlap = pref_tokens
            .iter()
            .filter(|token| cand_tokens.contains(token))
            .count() as i32;
        score += overlap * TOKEN_OVERLAP;
    }

    let pref_signs = is_signs_like(pref_title);
    let cand_signs = is_signs_like(cand_title);
    if pref_signs && cand_signs {
        score += SIGNS_ALIGN;
    } else if !pref_signs && cand_signs {
        score += SIGNS_MISMATCH;
    } else if pref_signs && !cand_signs {
        score += SIGNS_PREF_ONLY;
    }

    Some(score)
}

pub fn pick_closest_track(
    tracks: &[TrackCandidate],
    pref_lang: Option<&str>,
    pref_title: Option<&str>,
) -> Option<i64> {
    let mut best: Option<(i64, i32)> = None;
    for track in tracks {
        let Some(score) = score_track(pref_lang, pref_title, track) else {
            continue;
        };
        if score < MIN_SCORE {
            continue;
        }
        match best {
            Some((_, best_score)) if score <= best_score => {}
            _ => best = Some((track.id, score)),
        }
    }
    best.map(|(id, _)| id)
}

#[cfg(windows)]
fn empty_to_none(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub fn load_episode_pref(conn: &Connection, episode_id: i64) -> Result<Option<TrackPref>, String> {
    conn.query_row(
        "SELECT audio_lang, audio_title, subtitle_off, subtitle_lang, subtitle_title, subtitle_external_path
         FROM episode_track_prefs WHERE episode_id = ?1",
        params![episode_id],
        |row| {
            Ok(TrackPref {
                audio_lang: row.get(0)?,
                audio_title: row.get(1)?,
                subtitle_off: row.get::<_, i64>(2)? != 0,
                subtitle_lang: row.get(3)?,
                subtitle_title: row.get(4)?,
                subtitle_external_path: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn load_anime_pref(conn: &Connection, anime_id: i64) -> Result<Option<TrackPref>, String> {
    conn.query_row(
        "SELECT audio_lang, audio_title, subtitle_off, subtitle_lang, subtitle_title
         FROM anime_track_prefs WHERE anime_id = ?1",
        params![anime_id],
        |row| {
            Ok(TrackPref {
                audio_lang: row.get(0)?,
                audio_title: row.get(1)?,
                subtitle_off: row.get::<_, i64>(2)? != 0,
                subtitle_lang: row.get(3)?,
                subtitle_title: row.get(4)?,
                subtitle_external_path: None,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn upsert_episode_pref(
    conn: &Connection,
    episode_id: i64,
    pref: &TrackPref,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO episode_track_prefs (
            episode_id, audio_lang, audio_title, subtitle_off,
            subtitle_lang, subtitle_title, subtitle_external_path, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP)
         ON CONFLICT(episode_id) DO UPDATE SET
            audio_lang = excluded.audio_lang,
            audio_title = excluded.audio_title,
            subtitle_off = excluded.subtitle_off,
            subtitle_lang = excluded.subtitle_lang,
            subtitle_title = excluded.subtitle_title,
            subtitle_external_path = excluded.subtitle_external_path,
            updated_at = CURRENT_TIMESTAMP",
        params![
            episode_id,
            pref.audio_lang.as_deref(),
            pref.audio_title.as_deref(),
            if pref.subtitle_off { 1 } else { 0 },
            pref.subtitle_lang.as_deref(),
            pref.subtitle_title.as_deref(),
            pref.subtitle_external_path.as_deref(),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn upsert_anime_pref(conn: &Connection, anime_id: i64, pref: &TrackPref) -> Result<(), String> {
    conn.execute(
        "INSERT INTO anime_track_prefs (
            anime_id, audio_lang, audio_title, subtitle_off,
            subtitle_lang, subtitle_title, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP)
         ON CONFLICT(anime_id) DO UPDATE SET
            audio_lang = excluded.audio_lang,
            audio_title = excluded.audio_title,
            subtitle_off = excluded.subtitle_off,
            subtitle_lang = excluded.subtitle_lang,
            subtitle_title = excluded.subtitle_title,
            updated_at = CURRENT_TIMESTAMP",
        params![
            anime_id,
            pref.audio_lang.as_deref(),
            pref.audio_title.as_deref(),
            if pref.subtitle_off { 1 } else { 0 },
            pref.subtitle_lang.as_deref(),
            pref.subtitle_title.as_deref(),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn save_track_pref(
    conn: &Connection,
    anime_id: i64,
    episode_id: i64,
    pref: &TrackPref,
) -> Result<(), String> {
    upsert_episode_pref(conn, episode_id, pref)?;
    upsert_anime_pref(conn, anime_id, pref)?;
    Ok(())
}

#[cfg(windows)]
fn candidates_of_kind<'a>(tracks: &'a [MpvTrack], kind: &'a str) -> Vec<TrackCandidate> {
    tracks
        .iter()
        .filter(|track| track.kind == kind)
        .map(|track| TrackCandidate {
            id: track.id,
            lang: track.lang.clone(),
            title: track.title.clone(),
        })
        .collect()
}

#[cfg(windows)]
pub fn identity_from_tracks(tracks: &[MpvTrack]) -> TrackPref {
    let audio = tracks
        .iter()
        .find(|track| track.kind == "audio" && track.selected);
    let sub = tracks
        .iter()
        .find(|track| track.kind == "sub" && track.selected);
    TrackPref {
        audio_lang: empty_to_none(audio.and_then(|track| track.lang.clone())),
        audio_title: empty_to_none(audio.and_then(|track| track.title.clone())),
        subtitle_off: sub.is_none(),
        subtitle_lang: empty_to_none(sub.and_then(|track| track.lang.clone())),
        subtitle_title: empty_to_none(sub.and_then(|track| track.title.clone())),
        subtitle_external_path: sub.filter(|track| track.external).and_then(|track| {
            empty_to_none(track.external_filename.clone())
        }),
    }
}

#[cfg(windows)]
fn apply_pref_to_mpv(
    mpv: &crate::mpv::MpvHandle,
    pref: &TrackPref,
    tracks: &[MpvTrack],
) -> Result<(), String> {
    let mut restored_external_sub = false;
    if !pref.subtitle_off {
        if let Some(path) = pref.subtitle_external_path.as_deref() {
            if std::path::Path::new(path).is_file() {
                mpv.add_subtitle_file(path)?;
                restored_external_sub = true;
            }
        }
    }

    let audio_id = pick_closest_track(
        &candidates_of_kind(tracks, "audio"),
        pref.audio_lang.as_deref(),
        pref.audio_title.as_deref(),
    );
    if let Some(track_id) = audio_id {
        mpv.select_audio_track(track_id)?;
    }

    if pref.subtitle_off {
        mpv.select_subtitle_track(None)?;
    } else if !restored_external_sub {
        let sub_id = pick_closest_track(
            &candidates_of_kind(tracks, "sub"),
            pref.subtitle_lang.as_deref(),
            pref.subtitle_title.as_deref(),
        );
        if let Some(track_id) = sub_id {
            mpv.select_subtitle_track(Some(track_id))?;
        }
    }
    Ok(())
}

#[cfg(windows)]
#[tauri::command]
pub fn apply_saved_track_prefs(
    db: State<'_, AppDatabase>,
    state: State<'_, AppState>,
    anime_id: i64,
    episode_id: i64,
) -> Result<TrackPref, String> {
    let pref = db.with_conn(|conn| {
        if let Some(episode) = load_episode_pref(conn, episode_id)? {
            Ok(Some(episode))
        } else {
            load_anime_pref(conn, anime_id)
        }
    })?;

    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    let mpv = guard.as_ref().ok_or("mpv has not been initialized yet")?;
    let tracks = mpv.tracks()?;
    if let Some(pref) = pref.as_ref() {
        apply_pref_to_mpv(mpv, pref, &tracks)?;
    }
    Ok(identity_from_tracks(&mpv.tracks()?))
}

#[cfg(windows)]
#[tauri::command]
pub fn save_current_track_prefs(
    db: State<'_, AppDatabase>,
    state: State<'_, AppState>,
    anime_id: i64,
    episode_id: i64,
) -> Result<TrackPref, String> {
    let guard = state.mpv.lock().map_err(|e| e.to_string())?;
    let mpv = guard.as_ref().ok_or("mpv has not been initialized yet")?;
    let pref = identity_from_tracks(&mpv.tracks()?);
    drop(guard);
    db.with_conn(|conn| save_track_pref(conn, anime_id, episode_id, &pref))?;
    Ok(pref)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: i64, lang: &str, title: &str) -> TrackCandidate {
        TrackCandidate {
            id,
            lang: if lang.is_empty() {
                None
            } else {
                Some(lang.to_string())
            },
            title: if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            },
        }
    }

    #[test]
    fn normalize_lang_aliases() {
        assert_eq!(normalize_lang("jpn"), "ja");
        assert_eq!(normalize_lang("Japanese"), "ja");
        assert_eq!(normalize_lang("ENG"), "en");
        assert_eq!(normalize_lang("und"), "");
    }

    #[test]
    fn title_key_strips_wrappers() {
        assert_eq!(title_key("English [Signs]"), "english signs");
        assert_eq!(title_key("  English  (Full) "), "english full");
    }

    #[test]
    fn exact_lang_and_title_wins() {
        let tracks = [
            track(1, "jpn", "Japanese"),
            track(2, "eng", "English"),
            track(3, "eng", "English [Signs]"),
        ];
        assert_eq!(
            pick_closest_track(&tracks, Some("en"), Some("English")),
            Some(2)
        );
    }

    #[test]
    fn does_not_cross_languages() {
        let tracks = [
            track(1, "jpn", "Commentary"),
            track(2, "eng", "English"),
        ];
        assert_eq!(
            pick_closest_track(&tracks, Some("ja"), Some("Japanese")),
            Some(1)
        );
        assert_eq!(
            pick_closest_track(&tracks, Some("es"), Some("Spanish")),
            None
        );
    }

    #[test]
    fn prefers_signs_when_saved_title_is_signs() {
        let tracks = [
            track(1, "eng", "English"),
            track(2, "eng", "English [Signs]"),
        ];
        assert_eq!(
            pick_closest_track(&tracks, Some("en"), Some("English [Signs]")),
            Some(2)
        );
        assert_eq!(
            pick_closest_track(&tracks, Some("en"), Some("English")),
            Some(1)
        );
    }

    #[test]
    fn untagged_needs_strong_title() {
        let tracks = [track(1, "", "Japanese 2.0"), track(2, "", "Commentary")];
        assert_eq!(
            pick_closest_track(&tracks, Some("ja"), Some("Japanese 2.0")),
            Some(1)
        );
        assert_eq!(
            pick_closest_track(&tracks, Some("ja"), Some("Japanese")),
            None
        );
    }

    #[test]
    fn below_minimum_score_is_none() {
        let tracks = [track(1, "eng", "English")];
        assert_eq!(pick_closest_track(&tracks, None, Some("spa")), None);
        assert!(pick_closest_track(&[], Some("en"), Some("English")).is_none());
    }
}
