import type { Episode } from "./types";

/**
 * Pick the episode to launch when the user presses Q on the episodes screen:
 * - With no playback history, the first episode (so a fresh anime starts
 *   with one Q press).
 * - Otherwise, the most recently played episode — unless it is already
 *   watched, in which case the next episode in list order.
 * - Returns null only when there is nothing to play (empty list, or the
 *   watched candidate is the last episode and there is no successor).
 *
 * `episodes` is assumed to come from `list_episodes`, which orders by
 * episode_number then relative_path — so array index reflects in-anime order.
 */
export function pickQuickPlayEpisode(episodes: Episode[]): Episode | null {
  if (episodes.length === 0) return null;
  let lastIdx = -1;
  let lastTimestamp = "";
  for (let i = 0; i < episodes.length; i += 1) {
    const ts = episodes[i].last_watched_at;
    // SQLite CURRENT_TIMESTAMP is "YYYY-MM-DD HH:MM:SS" which sorts
    // lexicographically, so string comparison is correct.
    if (ts && ts > lastTimestamp) {
      lastTimestamp = ts;
      lastIdx = i;
    }
  }
  if (lastIdx === -1) return episodes[0];
  const last = episodes[lastIdx];
  if (!last.watched) return last;
  return episodes[lastIdx + 1] ?? null;
}
