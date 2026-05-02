import { getAnilistCoverImage, getFileThumbnail } from "./api";
import type { AnimeSummary } from "./types";

/** Shell thumbnail size; matches episode list rows */
const POSTER_THUMB_PX = 184;

/**
 * Prefer cached AniList cover art; if missing or unavailable, use a Windows
 * shell thumbnail from the first local episode (same ordering as the episode list).
 */
export async function resolveAnimePosterUrl(anime: AnimeSummary): Promise<string | null> {
  if (anime.anilist_cover_path) {
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
