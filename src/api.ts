import { invoke } from "@tauri-apps/api/core";
import type {
  Category,
  Episode,
  LibraryState,
  MpvTrack,
  MpvVideoGeometry,
  RegexRule,
  RegexRuleInput,
  RootFolder,
  ScanSummary,
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

export function createCategory(name: string): Promise<Category> {
  return invoke("create_category", { name });
}

export function deleteCategory(id: number): Promise<void> {
  return invoke("delete_category", { id });
}

export function setDefaultCategory(id: number): Promise<Category> {
  return invoke("set_default_category", { id });
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

export function getFileThumbnail(path: string, size: number): Promise<string | null> {
  return invoke("get_file_thumbnail", { path, size });
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

export function getMpvVideoGeometry(): Promise<MpvVideoGeometry | null> {
  return invoke("mpv_get_video_geometry");
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
