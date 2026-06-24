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
  gap_episode_count: number;
  last_watched_at: string | null;
  /** ISO-ish timestamp from DB when the anime row was first created */
  created_at: string;
  /** Latest `episodes.updated_at` for this show; refreshed on library rescan */
  latest_episode_at: string | null;
  /** First episode path in list order; used when AniList cover is missing */
  first_episode_path: string | null;
  no_op_ed: boolean;
};

/** Per-title text used by library search (titles + episode file names, not paths). */
export type AnimeSearchEntry = {
  id: number;
  title: string;
  anilist_title: string | null;
  file_names: string[];
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
  op_ed_segments: OpEdSegmentInfo[];
};

export type OpEdSegmentInfo = {
  kind: string;
  status: string;
  startSec: number | null;
  endSec: number | null;
  confidence: number | null;
  searchPass: string;
  errorText: string | null;
};

export type AnimeOpEdAnalysisSummary = {
  animeId: number;
  noOpEd: boolean;
  analysisVersion: number;
  analyzedAt: string | null;
  episodeCount: number;
  opMatched: number;
  opPending: number;
  edMatched: number;
  edPending: number;
  templatesCount: number;
};

export type ManualOpEdTemplate = {
  id: number;
  kind: string;
  kindIndex: number;
  startSec: number;
  durationSec: number;
  sourceEpisodeId: number;
  sourceEpisodeLabel: string;
};

export type PrepareManualOpEdRematchResult = {
  jobId: string | null;
  usedManualTemplates: boolean;
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

export type MpvPlaybackEndState = {
  time_pos: number;
  duration: number;
  eof_reached: boolean;
  paused: boolean;
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
  /** When true, linked titles use AniList name in library UI; unlinked titles always use the detected name. */
  prefer_anilist_display_title: boolean;
  /** When true, hide AniList linking UI, episode banners, and score controls. */
  hide_anilist_features: boolean;
  skip_op_ed: boolean;
  auto_op_ed_detect: boolean;
  dont_skip_first_episode_op_ed: boolean;
  clean_unused_scrub_sprites: boolean;
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
  scrub_sprites_bytes: number;
  op_ed_fingerprints_bytes: number;
  total_bytes: number;
};

export type LocalDataCleanupSummary = {
  roots_scanned: number;
  stale_episodes_removed: number;
  empty_anime_removed: number;
  unmatched_files_removed: number;
  thumbnails_removed: number;
  scrub_sprites_removed: number;
  op_ed_fingerprints_removed: number;
  op_ed_temp_pcm_removed: number;
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

export type ClearAnimeLocalDataSummary = {
  episodes_removed: number;
  bytes_removed: number;
};

export type LibraryOperationType =
  | "delete_anime"
  | "delete_episode"
  | "clean_local_data"
  | "rescan_library"
  | "local_data_stats";

export type LibraryOperationStatus = "queued" | "running" | "done" | "failed" | "canceled";

export type LibraryOperationRecord = {
  id: number;
  operationType: LibraryOperationType;
  status: LibraryOperationStatus;
  phase: string;
  targetAnimeId: number | null;
  targetEpisodeId: number | null;
  progressCurrent: number;
  progressTotal: number;
  summaryJson: string | null;
  errorText: string | null;
  createdAt: string;
  startedAt: string | null;
  finishedAt: string | null;
  updatedAt: string;
};

export type LibraryOpsSnapshot = {
  active: LibraryOperationRecord[];
  history: LibraryOperationRecord[];
  activeCount: number;
};

export type LibraryOperationFinishedEvent = {
  operationId: number;
  operationType: LibraryOperationType;
  status: LibraryOperationStatus;
  targetAnimeId: number | null;
  targetEpisodeId: number | null;
  summaryJson: string | null;
  errorText: string | null;
};

export type LibraryUpdatedEvent = {
  reason: string;
  operationId: number | null;
  statsChanged: boolean;
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
  /** AniList MediaStatus, e.g. RELEASING or FINISHED */
  status: string | null;
  /** Community mean score from AniList (available without login). */
  mean_score: number | null;
  /** Synopsis text from AniList (available without login). */
  description: string | null;
};

export type AnilistLocalProgressApplyResult = {
  progress: number;
  updated_episodes: number;
};

export type ScrubSpriteReady = {
  path: string;
  dataUrl: string;
  cols: number;
  rows: number;
  thumbWidth: number;
  thumbHeight: number;
  thumbCount: number;
  intervalSec: number;
};

export type ScrubSpriteStatus =
  | ({ status: "ready" } & ScrubSpriteReady)
  | { status: "unavailable"; path: string };

export type JobPriority = "low" | "medium" | "high";

export type JobStatus = "queued" | "running" | "done" | "failed" | "canceled";

export type JobProgress = {
  currentStep: number;
  totalSteps: number;
};

export type JobResourceType = "none" | "ffmpeg" | "chroma";

export type JobPrerequisiteView = {
  jobId: string;
  shortId: number;
};

export type JobRecord = {
  id: string;
  shortId: number;
  name: string;
  desc: string;
  identity: string;
  jobType: string;
  resourceType: JobResourceType;
  priority: JobPriority;
  status: JobStatus;
  cancelable: boolean;
  progress: JobProgress;
  stepLabel: string;
  completionMessage: string | null;
  createdAt: number;
  startedAt: number | null;
  finishedAt: number | null;
  prerequisiteTotal: number;
  /** Still queued or running; use for "+N more" (not capped like `waitingFor`). */
  prerequisitePending: number;
  /** Queued-job progress: two steps per prerequisite (start + complete). */
  prerequisiteProgressCurrent: number;
  prerequisiteProgressTotal: number;
  waitingFor: JobPrerequisiteView[];
};

export type EnqueueOpEdChromaAnimeJob = {
  animeId: number;
  priority: JobPriority;
  animeTitle?: string | null;
};

export type EnqueueOpEdChromaEpisodeJob = {
  episodeId: number;
  priority: JobPriority;
  animeTitle?: string | null;
};

export type TypeMaxParallel = {
  resourceType: string;
  maxParallel: number;
};

export type JobsSnapshot = {
  active: JobRecord[];
  history: JobRecord[];
  maxParallel: number;
  typeMaxParallel: TypeMaxParallel[];
  activeCount: number;
};

export type EnqueueJobResult = {
  jobId: string | null;
  skipped: boolean;
  chromaOnly?: boolean;
};

export type EnqueueScrubSpriteJob = {
  path: string;
  priority: JobPriority;
  animeTitle?: string | null;
  episodeLabel?: string | null;
  followUp?: EnqueueScrubSpriteJob[];
};

export type EpisodePageScrubItem = {
  path: string;
  episodeLabel?: string | null;
};

export type EnqueueEpisodePageScrubSprites = {
  priority: JobPriority;
  animeTitle?: string | null;
  episodes: EpisodePageScrubItem[];
};

export type EnqueueOpEdDetectJob = {
  animeId: number;
  priority: JobPriority;
  animeTitle?: string | null;
};

export type JobFinishedEvent = {
  jobId: string;
  identity: string;
  jobType: string;
  status: JobStatus;
};

export type OpEdAnalysisUpdatedEvent = {
  animeId: number;
};
