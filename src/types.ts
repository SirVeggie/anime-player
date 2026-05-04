export type RootFolder = {
  id: number;
  path: string;
};

export type VideoFile = {
  path: string;
  name: string;
  relative_path: string;
  size: number;
};

export type RegexRule = {
  id: number;
  name: string;
  detection_regex: string;
  title_regex: string;
  enabled: boolean;
  priority: number;
};

export type RegexRuleInput = {
  name: string;
  detection_regex: string;
  title_regex: string;
  enabled: boolean;
  priority: number;
};

export type Category = {
  id: number;
  name: string;
  is_default: boolean;
  sort_order: number;
};

export type AnimeSummary = {
  id: number;
  title: string;
  category_id: number;
  anilist_id: number | null;
  anilist_title: string | null;
  anilist_site_url: string | null;
  anilist_cover_path: string | null;
  custom_thumbnail_path: string | null;
  tracker_offset: number;
  episode_count: number;
  unwatched_count: number;
  last_watched_at: string | null;
  /** ISO-ish timestamp from DB when the anime row was first created */
  created_at: string;
  /** Latest `episodes.updated_at` for this show; refreshed on library rescan */
  latest_episode_at: string | null;
  /** First episode path in list order; used when AniList cover is missing */
  first_episode_path: string | null;
};

export type MissingAnimeSummary = AnimeSummary & {
  missing_episode_count: number;
  total_episode_count: number;
};

export type Episode = {
  id: number;
  anime_id: number;
  path: string;
  relative_path: string;
  file_name: string;
  file_type: string;
  episode_number: number | null;
  size: number;
  duration_seconds: number;
  position_seconds: number;
  watched: boolean;
  last_watched_at: string | null;
};

export type MpvTrack = {
  id: number;
  kind: "audio" | "sub";
  title: string | null;
  lang: string | null;
  selected: boolean;
};

export type MpvVideoGeometry = {
  width: number;
  height: number;
};

export type LibraryState = {
  db_path: string;
  root_folders: RootFolder[];
  regex_rules: RegexRule[];
  categories: Category[];
  anime: AnimeSummary[];
  recent_anime: AnimeSummary[];
  missing_anime: MissingAnimeSummary[];
  unmatched_count: number;
};

export type ScanSummary = {
  roots_scanned: number;
  episodes_imported: number;
  episodes_removed: number;
  unmatched_files: number;
};

export type ProgressOverrideSummary = {
  progress: number;
  updated_episodes: number;
};

export type LocalDataStats = {
  database_bytes: number;
  thumbnails_bytes: number;
  total_bytes: number;
};

export type LocalDataCleanupSummary = {
  roots_scanned: number;
  stale_episodes_removed: number;
  empty_anime_removed: number;
  unmatched_files_removed: number;
  thumbnails_removed: number;
  bytes_removed: number;
};

export type DeleteAnimeFilesSummary = {
  episodes_deleted: number;
  episodes_failed: number;
  bytes_deleted: number;
  cover_deleted: boolean;
  cover_failed: boolean;
  permanent_delete_used: boolean;
};

export type RenameFileRequest = {
  old_path: string;
  new_path: string;
};

export type RenameFilesSummary = {
  files_renamed: number;
};

export type RenameAnimeSummary = {
  files_renamed: number;
};

export type AnilistAuthState = {
  client_id: string | null;
  viewer_id: number | null;
  viewer_name: string | null;
  authenticated: boolean;
};

export type AnilistSearchResult = {
  id: number;
  title: string;
  native_title: string | null;
  format: string | null;
  status: string | null;
  episodes: number | null;
  season_year: number | null;
  cover_image_url: string | null;
  site_url: string;
};

export type AnilistProgressSyncResult = {
  synced: boolean;
  reason: string | null;
  remote_progress: number | null;
  target_progress: number | null;
};

export type AnilistMediaStatus = {
  progress: number | null;
  episodes: number | null;
  score: number | null;
};

export type AnilistLocalProgressApplyResult = {
  progress: number;
  updated_episodes: number;
};
