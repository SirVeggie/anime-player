import type { Episode } from "./types";
import { isEpisodeNumberKnown } from "./utils";

export function displayEpisodeNumber(episode: Episode, trackerOffset: number): number | null {
  if (!isEpisodeNumberKnown(episode.episode_number)) return null;
  return episode.episode_number - trackerOffset;
}

export function maxWatchedDisplayEpisode(episodes: Episode[], trackerOffset: number): number {
  let max = 0;
  for (const episode of episodes) {
    if (!episode.watched) continue;
    const displayNumber = displayEpisodeNumber(episode, trackerOffset);
    if (displayNumber !== null && displayNumber > max) {
      max = displayNumber;
    }
  }
  return max;
}

export function isLatestWatchedEpisode(
  episode: Episode,
  episodes: Episode[],
  trackerOffset: number,
): boolean {
  const displayNumber = displayEpisodeNumber(episode, trackerOffset);
  if (displayNumber === null || !episode.watched) return false;
  return displayNumber === maxWatchedDisplayEpisode(episodes, trackerOffset);
}
