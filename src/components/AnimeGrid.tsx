import { useEffect, useMemo, useState } from "react";
import { loadAnimePosterUrls } from "../animePoster";
import type { AnimeSummary, Category } from "../types";
import { useRovingListNavigation } from "../useRovingListNavigation";
import { animeDisplayTitle, animeTooltipTitle } from "../utils";
import { AnimeCardLabel } from "./AnimeCardLabel";
import { CustomDropdown } from "./CustomDropdown";
import { ViewHeader } from "./ViewHeader";

const GRID_SORT_STORAGE_KEY = "animePlayer.animeGridSort";

const GRID_SORT_OPTIONS = [
  { value: 0, label: "Alphabetical" },
  { value: 1, label: "Most recent" },
  { value: 2, label: "Last watched" },
  { value: 3, label: "Total episodes" },
  { value: 4, label: "Remaining episodes" },
  { value: 5, label: "Watch progress" },
] as const;

function readStoredGridSort(): number {
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

function sortAnimeForGrid(
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
        // Newest first: `latest_episode_at` (max episode updated_at), refreshed on rescan.
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

export function AnimeGrid(props: {
  category: Category | null;
  anime: AnimeSummary[];
  preferAnilistDisplayTitle: boolean;
  onBack: () => void;
  onOpenAnime: (anime: AnimeSummary) => void;
  onOpenSettings: () => void;
}) {
  const { category, anime, preferAnilistDisplayTitle, onBack, onOpenAnime, onOpenSettings } = props;
  const [sortValue, setSortValue] = useState(readStoredGridSort);

  const sortedAnime = useMemo(
    () => sortAnimeForGrid(anime, sortValue, preferAnilistDisplayTitle),
    [anime, preferAnilistDisplayTitle, sortValue],
  );

  const sortLabel = GRID_SORT_OPTIONS.find((o) => o.value === sortValue)?.label ?? "Alphabetical";

  const handleSortChange = (value: number) => {
    setSortValue(value);
    try {
      localStorage.setItem(GRID_SORT_STORAGE_KEY, String(value));
    } catch {
      /* ignore */
    }
  };

  return (
    <>
      <ViewHeader
        title={category?.name ?? "Titles"}
        subtitle={`${anime.length} title${anime.length === 1 ? "" : "s"} in this category.`}
        onBack={onBack}
        action={
          anime.length > 0 ? (
            <CustomDropdown
              label={`Sort: ${sortLabel}`}
              options={[...GRID_SORT_OPTIONS]}
              value={sortValue}
              onChange={handleSortChange}
            />
          ) : null
        }
      />
      {anime.length === 0 ? (
        <div className="empty empty--wide">
          <h2>No titles found here yet</h2>
          <p className="muted">Add root folders and rescan from settings, or move titles into this category later.</p>
          <button type="button" onClick={onOpenSettings}>
            Open settings
          </button>
        </div>
      ) : (
        <AnimeCardGrid
          anime={sortedAnime}
          preferAnilistDisplayTitle={preferAnilistDisplayTitle}
          onOpenAnime={onOpenAnime}
        />
      )}
    </>
  );
}

export function AnimeCardGrid(props: {
  anime: AnimeSummary[];
  preferAnilistDisplayTitle: boolean;
  onOpenAnime: (anime: AnimeSummary) => void;
}) {
  const { anime, preferAnilistDisplayTitle, onOpenAnime } = props;
  const [covers, setCovers] = useState<Record<number, string>>({});
  const getRovingItemProps = useRovingListNavigation(anime.length);

  useEffect(() => {
    let cancelled = false;
    setCovers({});
    void loadAnimePosterUrls(
      anime,
      (animeId, url) => {
        setCovers((current) => (cancelled ? current : { ...current, [animeId]: url }));
      },
      () => !cancelled,
    );
    return () => {
      cancelled = true;
    };
  }, [anime]);

  return (
    <div className="anime-grid">
      {anime.map((item, index) => {
        const cover = covers[item.id];
        const displayTitle = animeDisplayTitle(item, preferAnilistDisplayTitle);
        const tooltipTitle = animeTooltipTitle(item);
        return (
          <button
            type="button"
            className="anime-card"
            key={item.id}
            onClick={() => onOpenAnime(item)}
            {...getRovingItemProps(index)}
          >
            <div className={`poster-placeholder${cover ? " poster-placeholder--image" : ""}`}>
              {cover ? <img src={cover} alt="" loading="lazy" /> : displayTitle.slice(0, 2).toUpperCase()}
            </div>
            <AnimeCardLabel
              displayTitle={displayTitle}
              tooltipTitle={tooltipTitle}
              meta={
                <div className="anime-card-meta">
                  {item.unwatched_count > 0
                    ? `${item.episode_count} eps · ${item.unwatched_count} remaining`
                    : item.gap_episode_count > 0
                      ? (
                          <>
                            {item.episode_count} eps ·{" "}
                            <span className="stat-warning">{item.gap_episode_count} missing</span>
                          </>
                        )
                      : `${item.episode_count} eps`}
                </div>
              }
            />
          </button>
        );
      })}
    </div>
  );
}
