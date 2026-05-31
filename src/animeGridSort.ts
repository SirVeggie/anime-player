import type { AnimeSummary } from "./types";
import { animeDisplayTitle } from "./utils";

export const GRID_SORT_STORAGE_KEY = "animePlayer.animeGridSort";

export const GRID_SORT_OPTIONS = [
  { value: 0, label: "Alphabetical" },
  { value: 1, label: "Most recent" },
  { value: 2, label: "Last watched" },
  { value: 3, label: "Total episodes" },
  { value: 4, label: "Remaining episodes" },
  { value: 5, label: "Watch progress" },
] as const;

export function readStoredGridSort(): number {
  try {
    const raw = localStorage.getItem(GRID_SORT_STORAGE_KEY);
    if (raw === null) return 0;
    const n = Number.parseInt(raw, 10);
    if (!Number.isFinite(n) || n < 0 || n > 5) return 0;
    return n;
  } catch {
    return 0;
  }
}

export function storeGridSort(value: number): void {
  try {
    localStorage.setItem(GRID_SORT_STORAGE_KEY, String(value));
  } catch {
    /* ignore */
  }
}

export function gridSortLabel(sortValue: number): string {
  return GRID_SORT_OPTIONS.find((o) => o.value === sortValue)?.label ?? "Alphabetical";
}

export function sortAnimeForGrid(
  anime: AnimeSummary[],
  sortValue: number,
  preferAnilistDisplayTitle: boolean,
): AnimeSummary[] {
  const copy = [...anime];
  const byTitle = (a: AnimeSummary, b: AnimeSummary) =>
    animeDisplayTitle(a, preferAnilistDisplayTitle).localeCompare(
      animeDisplayTitle(b, preferAnilistDisplayTitle),
      undefined,
      { sensitivity: "base" },
    );

  copy.sort((a, b) => {
    let cmp = 0;
    switch (sortValue) {
      case 0:
        cmp = byTitle(a, b);
        break;
      case 1: {
        const aw = a.latest_episode_at;
        const bw = b.latest_episode_at;
        if (aw === null && bw === null) cmp = byTitle(a, b);
        else if (aw === null) cmp = 1;
        else if (bw === null) cmp = -1;
        else {
          const ta = Date.parse(aw.replace(" ", "T"));
          const tb = Date.parse(bw.replace(" ", "T"));
          if (Number.isFinite(ta) && Number.isFinite(tb)) {
            cmp = tb - ta;
          } else {
            cmp = bw.localeCompare(aw);
          }
        }
        if (cmp === 0) cmp = byTitle(a, b);
        break;
      }
      case 2: {
        const aw = a.last_watched_at;
        const bw = b.last_watched_at;
        if (aw === null && bw === null) cmp = byTitle(a, b);
        else if (aw === null) cmp = 1;
        else if (bw === null) cmp = -1;
        else cmp = bw.localeCompare(aw);
        if (cmp === 0) cmp = byTitle(a, b);
        break;
      }
      case 3:
        cmp = b.episode_count - a.episode_count;
        if (cmp === 0) cmp = byTitle(a, b);
        break;
      case 4:
        cmp = b.unwatched_count - a.unwatched_count;
        if (cmp === 0) cmp = byTitle(a, b);
        break;
      case 5: {
        const aCompleted = a.unwatched_count === 0 && a.gap_episode_count === 0;
        const bCompleted = b.unwatched_count === 0 && b.gap_episode_count === 0;
        if (aCompleted !== bCompleted) {
          cmp = aCompleted ? 1 : -1;
        } else {
          const aTotal = a.episode_count + a.gap_episode_count;
          const bTotal = b.episode_count + b.gap_episode_count;
          const aProgress = aTotal > 0 ? (a.episode_count - a.unwatched_count) / aTotal : 0;
          const bProgress = bTotal > 0 ? (b.episode_count - b.unwatched_count) / bTotal : 0;
          cmp = bProgress - aProgress;
        }
        if (cmp === 0) cmp = byTitle(a, b);
        break;
      }
      default:
        cmp = byTitle(a, b);
    }
    return cmp;
  });
  return copy;
}
