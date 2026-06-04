import { getAnilistCoverImage, getFileThumbnail } from "./api";
import type { AnimeSummary } from "./types";

/** Shell thumbnail size; matches episode list rows */
const POSTER_THUMB_PX = 184;
const ANILIST_COVER_CONCURRENCY = 12;
const FILE_THUMBNAIL_CONCURRENCY = 4;

async function runLimited<T>(items: T[], limit: number, worker: (item: T) => Promise<void>): Promise<void> {
  let nextIndex = 0;
  const workers = Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (nextIndex < items.length) {
      const item = items[nextIndex];
      nextIndex += 1;
      await worker(item);
    }
  });
  await Promise.all(workers);
}

/**
 * Prefer cached AniList cover art; if missing or unavailable, use a Windows
 * shell thumbnail from the first local episode (same ordering as the episode list).
 */
export async function resolveAnimePosterUrl(anime: AnimeSummary): Promise<string | null> {
  if (anime.custom_thumbnail_path) {
    try {
      const custom = await getFileThumbnail(anime.custom_thumbnail_path, POSTER_THUMB_PX);
      if (custom) return custom;
    } catch {
      /* fall through to AniList or episode thumbnail */
    }
  }
  if (anime.anilist_cover_path || anime.anilist_id) {
    try {
      const cover = await getAnilistCoverImage(anime.id);
      if (cover) return cover;
    } catch {
      /* fall through to file thumbnail */
    }
  }
  if (anime.first_episode_path) {
    try {
      return await getFileThumbnail(anime.first_episode_path, POSTER_THUMB_PX);
    } catch {
      return null;
    }
  }
  return null;
}

/**
 * Load poster URLs in priority order. AniList covers are local cached files and
 * should appear before slower video thumbnail extraction starts.
 */
export async function loadAnimePosterUrls(
  anime: AnimeSummary[],
  onPoster: (animeId: number, url: string) => void,
  shouldContinue: () => boolean,
): Promise<void> {
  const resolved = new Set<number>();
  const customCandidates = anime.filter((item) => item.custom_thumbnail_path);

  await runLimited(customCandidates, FILE_THUMBNAIL_CONCURRENCY, async (item) => {
    if (!shouldContinue() || !item.custom_thumbnail_path) return;
    try {
      const url = await getFileThumbnail(item.custom_thumbnail_path, POSTER_THUMB_PX);
      if (!url || !shouldContinue()) return;
      resolved.add(item.id);
      onPoster(item.id, url);
    } catch {
      /* fall through to AniList and episode thumbnail phases */
    }
  });

  if (!shouldContinue()) return;

  const anilistCandidates = anime.filter(
    (item) => !resolved.has(item.id) && (item.anilist_cover_path || item.anilist_id),
  );

  await runLimited(anilistCandidates, ANILIST_COVER_CONCURRENCY, async (item) => {
    if (!shouldContinue()) return;
    try {
      const url = await getAnilistCoverImage(item.id);
      if (!url || !shouldContinue()) return;
      resolved.add(item.id);
      onPoster(item.id, url);
    } catch {
      /* fall through to file thumbnail phase */
    }
  });

  if (!shouldContinue()) return;

  const fileCandidates = anime.filter((item) => !resolved.has(item.id) && item.first_episode_path);
  await runLimited(fileCandidates, FILE_THUMBNAIL_CONCURRENCY, async (item) => {
    if (!shouldContinue() || !item.first_episode_path) return;
    try {
      const url = await getFileThumbnail(item.first_episode_path, POSTER_THUMB_PX);
      if (!url || !shouldContinue()) return;
      onPoster(item.id, url);
    } catch {
      /* ignore missing or unsupported file thumbnails */
    }
  });
}

/** Load episode row thumbnails with limited concurrency (shell extraction, no DB). */
export async function loadEpisodeThumbnailUrls(
  episodes: { id: number; path: string }[],
  size: number,
  onThumbnail: (episodeId: number, url: string) => void,
  shouldContinue: () => boolean,
): Promise<void> {
  await runLimited(episodes, FILE_THUMBNAIL_CONCURRENCY, async (episode) => {
    if (!shouldContinue()) return;
    try {
      const url = await getFileThumbnail(episode.path, size);
      if (!url || !shouldContinue()) return;
      onThumbnail(episode.id, url);
    } catch {
      /* ignore missing or unsupported file thumbnails */
    }
  });
}
