import { useEffect, useMemo, useState } from "react";
import {
  filterAnimeByText,
  filterAnimeForGrid,
  readStoredGridFilter,
  readStoredGridTextFilter,
  storeGridFilter,
  storeGridTextFilter,
} from "../animeGridFilter";
import {
  GRID_SORT_OPTIONS,
  gridSortLabel,
  readStoredGridSort,
  sortAnimeForGrid,
  storeGridSort,
} from "../animeGridSort";
import {
  animePosterSourceKey,
  cachedThumbnailUrl,
  loadAnimePosterUrls,
  pruneThumbnailUrlCache,
  type ThumbnailUrlCache,
} from "../animePoster";
import type { AnimeSearchEntry, AnimeSummary, AnilistSearchResult, Category } from "../types";
import { useRovingListNavigation } from "../useRovingListNavigation";
import { animeDisplayTitle, animeTooltipTitle } from "../utils";
import { AnimeCardLabel } from "./AnimeCardLabel";
import { useAnimeContextMenu, type AnimeContextMenuHandlers } from "./animeContextMenu";
import { CustomDropdown } from "./CustomDropdown";
import { GridFilterBar, GridTextFilter } from "./GridFilterBar";
import { ViewHeader } from "./ViewHeader";

export function AnimeGrid(props: {
  category: Category | null;
  anime: AnimeSummary[];
  searchIndex: AnimeSearchEntry[];
  categories: Category[];
  preferAnilistDisplayTitle: boolean;
  onBack: () => void;
  onOpenAnime: (anime: AnimeSummary) => void;
  onOpenSettings: () => void;
  onDeleteAnime: (anime: AnimeSummary) => void;
  onMoveAnime: (anime: AnimeSummary, categoryId: number) => void;
  onOpenAnimeFolder: (anime: AnimeSummary) => void;
  onSetAnimeThumbnail: (anime: AnimeSummary) => void;
  anilistFeaturesEnabled: boolean;
  onSearchAnilist: (query: string) => Promise<AnilistSearchResult[]>;
  onLinkAnilist: (animeId: number, anilistId: number) => void;
}) {
  const {
    category,
    anime,
    searchIndex,
    categories,
    preferAnilistDisplayTitle,
    onBack,
    onOpenAnime,
    onOpenSettings,
    onDeleteAnime,
    onMoveAnime,
    onOpenAnimeFolder,
    onSetAnimeThumbnail,
    anilistFeaturesEnabled,
    onSearchAnilist,
    onLinkAnilist,
  } = props;
  const [filterValue, setFilterValue] = useState(readStoredGridFilter);
  const [textFilter, setTextFilter] = useState(readStoredGridTextFilter);
  const [sortValue, setSortValue] = useState(readStoredGridSort);

  const textFilteredAnime = useMemo(
    () => filterAnimeByText(anime, textFilter, searchIndex),
    [anime, searchIndex, textFilter],
  );
  const filteredAnime = useMemo(
    () => filterAnimeForGrid(textFilteredAnime, filterValue),
    [filterValue, textFilteredAnime],
  );
  const sortedAnime = useMemo(
    () => sortAnimeForGrid(filteredAnime, sortValue, preferAnilistDisplayTitle),
    [filteredAnime, preferAnilistDisplayTitle, sortValue],
  );

  const sortLabel = gridSortLabel(sortValue);
  const titleCountLabel = `${anime.length} title${anime.length === 1 ? "" : "s"}`;
  const filtersActive = filterValue !== 0 || Boolean(textFilter.trim());
  const subtitle =
    !filtersActive || anime.length === 0
      ? `${titleCountLabel} in this category.`
      : `${filteredAnime.length} of ${titleCountLabel} in this category.`;

  const handleFilterChange = (value: number) => {
    setFilterValue(value);
    storeGridFilter(value);
  };

  const handleTextFilterChange = (value: string) => {
    setTextFilter(value);
    storeGridTextFilter(value);
  };

  const clearGridFilters = () => {
    handleFilterChange(0);
    handleTextFilterChange("");
  };

  const handleSortChange = (value: number) => {
    setSortValue(value);
    storeGridSort(value);
  };

  return (
    <>
      <ViewHeader
        title={category?.name ?? "Titles"}
        subtitle={subtitle}
        onBack={onBack}
        action={
          anime.length > 0 ? (
            <>
              <GridTextFilter value={textFilter} onChange={handleTextFilterChange} />
              <GridFilterBar value={filterValue} onChange={handleFilterChange} />
              <CustomDropdown
                label={`Sort: ${sortLabel}`}
                options={[...GRID_SORT_OPTIONS]}
                value={sortValue}
                onChange={handleSortChange}
              />
            </>
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
      ) : filteredAnime.length === 0 ? (
        <div className="empty empty--wide">
          <h2>No titles match this filter</h2>
          <p className="muted">Try a different filter, or show every title in this category.</p>
          <button type="button" onClick={clearGridFilters}>
            Show all
          </button>
        </div>
      ) : (
        <AnimeCardGrid
          anime={sortedAnime}
          preferAnilistDisplayTitle={preferAnilistDisplayTitle}
          onOpenAnime={onOpenAnime}
          contextMenu={{
            categories,
            onDeleteAnime,
            onMoveAnime,
            onOpenAnimeFolder,
            onSetAnimeThumbnail,
            anilistFeaturesEnabled,
            onSearchAnilist,
            onLinkAnilist,
          }}
        />
      )}
    </>
  );
}

export type { AnimeContextMenuHandlers } from "./animeContextMenu";

export function AnimeCardGrid(props: {
  anime: AnimeSummary[];
  preferAnilistDisplayTitle: boolean;
  onOpenAnime: (anime: AnimeSummary) => void;
  contextMenu?: AnimeContextMenuHandlers | null;
}) {
  const { anime, preferAnilistDisplayTitle, onOpenAnime, contextMenu = null } = props;
  const [covers, setCovers] = useState<ThumbnailUrlCache>({});
  const getRovingItemProps = useRovingListNavigation(anime.length);
  const { enabled: contextMenuEnabled, openAnimeMenu, menuUi } = useAnimeContextMenu(contextMenu);

  useEffect(() => {
    let cancelled = false;
    const sourceKeys = new Map(anime.map((item) => [item.id, animePosterSourceKey(item)]));
    setCovers((current) => pruneThumbnailUrlCache(current, anime, animePosterSourceKey));
    void loadAnimePosterUrls(
      anime,
      (animeId, url) => {
        const sourceKey = sourceKeys.get(animeId);
        if (!sourceKey) return;
        setCovers((current) => (cancelled ? current : { ...current, [animeId]: { sourceKey, url } }));
      },
      () => !cancelled,
    );
    return () => {
      cancelled = true;
    };
  }, [anime]);

  return (
    <>
      <div className="anime-grid">
        {anime.map((item, index) => {
          const cover = cachedThumbnailUrl(covers, item, animePosterSourceKey);
          const displayTitle = animeDisplayTitle(item, preferAnilistDisplayTitle);
          const tooltipTitle = animeTooltipTitle(item);
          return (
            <button
              type="button"
              className="anime-card"
              key={item.id}
              onClick={() => onOpenAnime(item)}
              onContextMenu={
                contextMenuEnabled ? (event) => openAnimeMenu(event, item) : undefined
              }
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
      {menuUi}
    </>
  );
}
