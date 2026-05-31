import type { AnilistAuthState, AnimeSummary, Episode } from "./types";

export function isAnilistConnected(auth: AnilistAuthState | null): boolean {
  return auth?.authenticated === true;
}

/** Library display name: detected title unless the user prefers AniList and the title is linked. */
export function animeDisplayTitle(
  anime: Pick<AnimeSummary, "title" | "anilist_id" | "anilist_title">,
  preferAnilistDisplayTitle: boolean,
): string {
  const anilistTitle = anime.anilist_title?.trim();
  if (preferAnilistDisplayTitle && anime.anilist_id != null && anilistTitle) {
    return anilistTitle;
  }
  return anime.title;
}

/** Grid hover tooltip: AniList name when linked, otherwise the detected filesystem title. */
export function animeTooltipTitle(
  anime: Pick<AnimeSummary, "title" | "anilist_id" | "anilist_title">,
): string {
  if (anime.anilist_id != null) {
    const anilistTitle = anime.anilist_title?.trim();
    if (anilistTitle) return anilistTitle;
  }
  return anime.title;
}

export function formatSize(bytes: number): string {
  if (bytes <= 0) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 100 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}

/** Case-insensitive path compare for matching episode paths to backend canonical paths. */
export function mediaPathsEqual(a: string, b: string): boolean {
  return normalizeMediaPath(a) === normalizeMediaPath(b);
}

function normalizeMediaPath(path: string): string {
  return path.trim().replace(/\//g, "\\").toLowerCase();
}

/** Formats a millisecond duration as M:SS or H:MM:SS (for job timers). */
export function formatDurationMs(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "0:00";
  return formatTime(ms / 1000);
}

export function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const total = Math.floor(seconds);
  const s = total % 60;
  const m = Math.floor(total / 60) % 60;
  const h = Math.floor(total / 3600);
  const ss = s.toString().padStart(2, "0");
  if (h > 0) {
    const mm = m.toString().padStart(2, "0");
    return `${h}:${mm}:${ss}`;
  }
  return `${m}:${ss}`;
}

export function isEpisodeNumberKnown(value: number | null): value is number {
  return value !== null && Number.isFinite(value);
}

export function formatEpisodeNumber(value: number | null): string {
  if (!isEpisodeNumberKnown(value)) return "Episode ?";
  return Number.isInteger(value) ? `Episode ${value}` : `Episode ${value.toFixed(1)}`;
}

/** Top overlay in the player when the episode number is known; otherwise the file name. */
export function playerNowPlayingLabel(
  episode: Pick<Episode, "episode_number" | "file_name">,
  anime: Pick<AnimeSummary, "title" | "anilist_id" | "anilist_title" | "tracker_offset">,
  preferAnilistDisplayTitle: boolean,
): string {
  if (!isEpisodeNumberKnown(episode.episode_number)) {
    return episode.file_name;
  }
  const displayTitle = animeDisplayTitle(anime, preferAnilistDisplayTitle);
  const displayEpisodeNumber = episode.episode_number - anime.tracker_offset;
  return `${displayTitle} — ${formatEpisodeNumber(displayEpisodeNumber)}`;
}

/** True when AniList reports the title is still airing (skip trailing gap count). */
export function isAnilistReleasing(status: string | null | undefined): boolean {
  return status?.toUpperCase() === "RELEASING";
}

function collectIntegerEpisodeNumbers(episodes: Episode[]): Set<number> {
  const intEpisodes = new Set<number>();
  for (const ep of episodes) {
    if (ep.episode_number !== null && Number.isInteger(ep.episode_number)) {
      intEpisodes.add(ep.episode_number);
    }
  }
  return intEpisodes;
}

function integerEpisodeRange(intEpisodes: Set<number>): { min: number; max: number } | null {
  if (intEpisodes.size === 0) return null;
  let min = Number.MAX_SAFE_INTEGER;
  let max = 0;
  for (const n of intEpisodes) {
    if (n < min) min = n;
    if (n > max) max = n;
  }
  return { min, max };
}

function effectiveMaxEpisodeNumber(
  maxLocal: number,
  anilistTotalEpisodes: number | null | undefined,
  anilistStatus: string | null | undefined,
  trackerOffset: number,
): number {
  if (isAnilistReleasing(anilistStatus)) return maxLocal;
  if (anilistTotalEpisodes != null && anilistTotalEpisodes > 0) {
    return Math.max(maxLocal, anilistTotalEpisodes + trackerOffset);
  }
  return maxLocal;
}

/**
 * Count integer episode numbers missing from the range [min_local .. effective_max].
 * Decimal episode numbers are excluded. If `anilistTotalEpisodes` is set and positive,
 * effective_max is extended to `anilistTotalEpisodes + trackerOffset` unless status is RELEASING.
 */
export function computeGapEpisodeCount(
  episodes: Episode[],
  anilistTotalEpisodes: number | null | undefined,
  trackerOffset: number,
  anilistStatus?: string | null,
): number {
  const intEpisodes = collectIntegerEpisodeNumbers(episodes);
  const range = integerEpisodeRange(intEpisodes);
  if (!range) return 0;
  const max = effectiveMaxEpisodeNumber(
    range.max,
    anilistTotalEpisodes,
    anilistStatus,
    trackerOffset,
  );
  return max - range.min + 1 - intEpisodes.size;
}

export type EpisodeListItem =
  | { kind: "episode"; episode: Episode; episodeIndex: number }
  | { kind: "gap"; missingCount: number; key: string };

export function formatMissingEpisodesLabel(missingCount: number): string {
  const noun = missingCount === 1 ? "episode" : "episodes";
  return `— missing ${missingCount} ${noun} —`;
}

/**
 * Episode rows interleaved with gap separators for missing integer episode numbers.
 */
export function buildEpisodeListItems(
  episodes: Episode[],
  options: {
    trackerOffset: number;
    anilistTotalEpisodes?: number | null;
    anilistStatus?: string | null;
  },
): EpisodeListItem[] {
  const { trackerOffset, anilistTotalEpisodes, anilistStatus } = options;
  const intEpisodes = collectIntegerEpisodeNumbers(episodes);
  const range = integerEpisodeRange(intEpisodes);
  if (!range) {
    return episodes.map((episode, episodeIndex) => ({ kind: "episode", episode, episodeIndex }));
  }

  const effectiveMax = effectiveMaxEpisodeNumber(
    range.max,
    anilistTotalEpisodes,
    anilistStatus,
    trackerOffset,
  );
  const items: EpisodeListItem[] = [];
  let lastInt: number | null = null;

  const pushGap = (fromExclusive: number, toInclusive: number) => {
    const missingCount = toInclusive - fromExclusive;
    if (missingCount <= 0) return;
    items.push({
      kind: "gap",
      missingCount,
      key: `gap-${fromExclusive}-${toInclusive}`,
    });
  };

  episodes.forEach((episode, episodeIndex) => {
    const n = episode.episode_number;
    if (n !== null && Number.isInteger(n)) {
      if (lastInt !== null && n > lastInt + 1) {
        pushGap(lastInt, n - 1);
      }
      lastInt = n;
    }
    items.push({ kind: "episode", episode, episodeIndex });
  });

  if (lastInt !== null && effectiveMax > lastInt) {
    pushGap(lastInt, effectiveMax);
  }

  return items;
}

export function progressPercent(position: number, duration: number): number {
  if (duration <= 0) return 0;
  return Math.min(100, Math.max(0, (position / duration) * 100));
}

export function errorMessage(error: unknown): string {
  return typeof error === "string" ? error : String(error);
}

/** Shown in the native window title (taskbar / Alt-Tab); keep OS limits in mind. */
export function shortenForOsTitle(text: string, maxChars = 42): string {
  const t = text.trim().replace(/\s+/g, " ");
  if (!t) return "Anime";
  if (t.length <= maxChars) return t;
  return `${t.slice(0, Math.max(1, maxChars - 1))}…`;
}

export const APP_WINDOW_TITLE = "Anime Player";

/** True when the event target is typing in a field that should keep window shortcuts from firing. */
export function isTextInputTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  return target.isContentEditable;
}
