import { invoke } from "@tauri-apps/api/core";
import type {
  AnilistAuthState,
  AnilistLocalProgressApplyResult,
  AnilistMediaStatus,
  AnilistProgressSyncResult,
  AnilistSearchResult,
  Category,
  DeleteAnimeFilesSummary,
  Episode,
  LibraryState,
  LocalDataCleanupSummary,
  LocalDataStats,
  MpvTrack,
  MpvVideoGeometry,
  ScrubSpriteStatus,
  ProgressOverrideSummary,
  RegexRule,
  RegexRuleInput,
  RenameAnimeSummary,
  RenameFileRequest,
  RenameFilesSummary,
  RootFolder,
  ScanSummary,
  VideoFile,
} from "./types";

export function getLibraryState(): Promise<LibraryState> {
  return invoke("get_library_state");
}

export function addRootFolder(path: string): Promise<RootFolder> {
  return invoke("add_root_folder", { path });
}

export function removeRootFolder(id: number): Promise<void> {
  return invoke("remove_root_folder", { id });
}

export function rescanLibrary(): Promise<ScanSummary> {
  return invoke("rescan_library");
}

export function getLocalDataStats(): Promise<LocalDataStats> {
  return invoke("get_local_data_stats");
}

export function cleanLocalData(): Promise<LocalDataCleanupSummary> {
  return invoke("clean_local_data");
}

export function createCategory(name: string): Promise<Category> {
  return invoke("create_category", { name });
}

export function deleteCategory(id: number): Promise<void> {
  return invoke("delete_category", { id });
}

export function setDefaultCategory(id: number): Promise<Category> {
  return invoke("set_default_category", { id });
}

export function reorderCategories(categoryIds: number[]): Promise<void> {
  return invoke("reorder_categories", { categoryIds });
}

export function moveAnimeToCategory(animeId: number, categoryId: number): Promise<void> {
  return invoke("move_anime_to_category", { animeId, categoryId });
}

export function createRegexRule(input: RegexRuleInput): Promise<RegexRule> {
  return invoke("create_regex_rule", { input });
}

export function updateRegexRule(id: number, input: RegexRuleInput): Promise<RegexRule> {
  return invoke("update_regex_rule", { id, input });
}

export function deleteRegexRule(id: number): Promise<void> {
  return invoke("delete_regex_rule", { id });
}

export function listEpisodes(animeId: number): Promise<Episode[]> {
  return invoke("list_episodes", { animeId });
}

export function listRootVideoFiles(): Promise<VideoFile[]> {
  return invoke("list_root_video_files");
}

export function deleteAnimeFiles(animeId: number): Promise<DeleteAnimeFilesSummary> {
  return invoke("delete_anime_files", { animeId });
}

export function validateFileRenames(renames: RenameFileRequest[]): Promise<void> {
  return invoke("validate_file_renames", { renames });
}

export function renameFiles(renames: RenameFileRequest[]): Promise<RenameFilesSummary> {
  return invoke("rename_files", { renames });
}

export function renameAnime(animeId: number, newTitle: string): Promise<RenameAnimeSummary> {
  return invoke("rename_anime", { animeId, newTitle });
}

export function openAnimeEpisodeFolder(animeId: number): Promise<void> {
  return invoke("open_anime_episode_folder", { animeId });
}

/** Which enabled detection rule matches this anime's files (same logic as rescan; not persisted). */
export function getMatchingDetectionRuleName(animeId: number): Promise<string | null> {
  return invoke("get_matching_detection_rule_name", { animeId });
}

export function setAnimeTrackerOffset(animeId: number, trackerOffset: number): Promise<void> {
  return invoke("set_anime_tracker_offset", { animeId, trackerOffset });
}

export function setAnimeCustomThumbnailPath(animeId: number, customThumbnailPath: string | null): Promise<void> {
  return invoke("set_anime_custom_thumbnail_path", { animeId, customThumbnailPath });
}

export function overrideAnimeProgress(animeId: number, progress: number): Promise<ProgressOverrideSummary> {
  return invoke("override_anime_progress", { animeId, progress });
}

export function getFileThumbnail(path: string, size: number): Promise<string | null> {
  return invoke("get_file_thumbnail", { path, size });
}

export function ensureScrubSprite(path: string): Promise<ScrubSpriteStatus> {
  return invoke("ensure_scrub_sprite", { path });
}

export function getMpvTracks(): Promise<MpvTrack[]> {
  return invoke("mpv_get_tracks");
}

export function selectMpvAudioTrack(trackId: number): Promise<void> {
  return invoke("mpv_select_audio_track", { trackId });
}

export function selectMpvSubtitleTrack(trackId: number | null): Promise<void> {
  return invoke("mpv_select_subtitle_track", { trackId });
}

export function addMpvSubtitleFile(path: string): Promise<void> {
  return invoke("mpv_add_subtitle_file", { path });
}

export function setMpvVolume(volume: number): Promise<void> {
  return invoke("mpv_set_volume", { volume });
}

export function getMpvVideoGeometry(): Promise<MpvVideoGeometry | null> {
  return invoke("mpv_get_video_geometry");
}

export function getMpvTimePos(): Promise<number> {
  return invoke("mpv_get_time_pos");
}

export function stopMpv(): Promise<void> {
  return invoke("mpv_stop");
}

let minPositionSecondsToPersistCache: number | null = null;
let minPositionSecondsToPersistPromise: Promise<number> | null = null;

/** Mirrors `MIN_POSITION_SECONDS_TO_PERSIST` in `library.rs` (cached after first call). */
export function getMinPositionSecondsToPersist(): Promise<number> {
  if (minPositionSecondsToPersistCache !== null) {
    return Promise.resolve(minPositionSecondsToPersistCache);
  }
  if (!minPositionSecondsToPersistPromise) {
    minPositionSecondsToPersistPromise = invoke<number>("get_min_position_seconds_to_persist").then(
      (value) => {
        minPositionSecondsToPersistCache = value;
        return value;
      },
    );
  }
  return minPositionSecondsToPersistPromise;
}

export function saveEpisodeProgress(
  episodeId: number,
  positionSeconds: number,
  durationSeconds: number,
  watched: boolean,
): Promise<Episode> {
  return invoke("save_episode_progress", {
    episodeId,
    positionSeconds,
    durationSeconds,
    watched,
  });
}

export function getAnilistAuthState(): Promise<AnilistAuthState> {
  return invoke("get_anilist_auth_state");
}

export function setAnilistClientId(clientId: string): Promise<AnilistAuthState> {
  return invoke("set_anilist_client_id", { clientId });
}

export function getAnilistLoginUrl(): Promise<string> {
  return invoke("get_anilist_login_url");
}

export function completeAnilistLogin(callbackUrl: string): Promise<AnilistAuthState> {
  return invoke("complete_anilist_login", { callbackUrl });
}

export function logoutAnilist(): Promise<AnilistAuthState> {
  return invoke("logout_anilist");
}

export function searchAnilistAnime(query: string): Promise<AnilistSearchResult[]> {
  return invoke("search_anilist_anime", { query });
}

export function linkAnimeAnilist(animeId: number, anilistId: number): Promise<void> {
  return invoke("link_anime_anilist", { animeId, anilistId });
}

export function unlinkAnimeAnilist(animeId: number): Promise<void> {
  return invoke("unlink_anime_anilist", { animeId });
}

export function getAnilistCoverImage(animeId: number): Promise<string | null> {
  return invoke("get_anilist_cover_image", { animeId });
}

export function syncAnilistEpisodeProgress(episodeId: number): Promise<AnilistProgressSyncResult> {
  return invoke("sync_anilist_episode_progress", { episodeId });
}

export function getAnilistMediaStatus(animeId: number): Promise<AnilistMediaStatus | null> {
  return invoke("get_anilist_media_status", { animeId });
}

export function setAnilistMediaProgress(animeId: number, progress: number): Promise<AnilistMediaStatus> {
  return invoke("set_anilist_media_progress", { animeId, progress });
}

export function applyAnilistProgressToLocal(animeId: number): Promise<AnilistLocalProgressApplyResult | null> {
  return invoke("apply_anilist_progress_to_local", { animeId });
}

export function setAnilistMediaScore(animeId: number, score: number | null): Promise<AnilistMediaStatus> {
  return invoke("set_anilist_media_score", { animeId, score });
}
