export type RootFolder = {
  id: number;
  path: string;
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
  episode_count: number;
  unwatched_count: number;
  last_watched_at: string | null;
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
  unmatched_count: number;
};

export type ScanSummary = {
  roots_scanned: number;
  episodes_imported: number;
  unmatched_files: number;
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
