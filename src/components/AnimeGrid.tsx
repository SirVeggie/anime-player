import { useCallback, useEffect, useMemo, useState } from "react";
import {
  GRID_SORT_OPTIONS,
  gridSortLabel,
  readStoredGridSort,
  sortAnimeForGrid,
  storeGridSort,
} from "../animeGridSort";
import { loadAnimePosterUrls } from "../animePoster";
import type { AnimeSummary, Category } from "../types";
import { useRovingListNavigation } from "../useRovingListNavigation";
import { animeDisplayTitle, animeTooltipTitle } from "../utils";
import { AnimeCardLabel } from "./AnimeCardLabel";
import { ConfirmModal } from "./ConfirmModal";
import { ContextMenu, useContextMenu, type ContextMenuItem } from "./ContextMenu";
import { CustomDropdown } from "./CustomDropdown";
import { ViewHeader } from "./ViewHeader";

export function AnimeGrid(props: {
  category: Category | null;
  anime: AnimeSummary[];
  categories: Category[];
  preferAnilistDisplayTitle: boolean;
  onBack: () => void;
  onOpenAnime: (anime: AnimeSummary) => void;
  onOpenSettings: () => void;
  onDeleteAnime: (anime: AnimeSummary) => void;
  onMoveAnime: (anime: AnimeSummary, categoryId: number) => void;
  onOpenAnimeFolder: (anime: AnimeSummary) => void;
  onSetAnimeThumbnail: (anime: AnimeSummary) => void;
}) {
  const {
    category,
    anime,
    categories,
    preferAnilistDisplayTitle,
    onBack,
    onOpenAnime,
    onOpenSettings,
    onDeleteAnime,
    onMoveAnime,
    onOpenAnimeFolder,
    onSetAnimeThumbnail,
  } = props;
  const [sortValue, setSortValue] = useState(readStoredGridSort);

  const sortedAnime = useMemo(
    () => sortAnimeForGrid(anime, sortValue, preferAnilistDisplayTitle),
    [anime, preferAnilistDisplayTitle, sortValue],
  );

  const sortLabel = gridSortLabel(sortValue);

  const handleSortChange = (value: number) => {
    setSortValue(value);
    storeGridSort(value);
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
          categories={categories}
          preferAnilistDisplayTitle={preferAnilistDisplayTitle}
          onOpenAnime={onOpenAnime}
          onDeleteAnime={onDeleteAnime}
          onMoveAnime={onMoveAnime}
          onOpenAnimeFolder={onOpenAnimeFolder}
          onSetAnimeThumbnail={onSetAnimeThumbnail}
        />
      )}
    </>
  );
}

export function AnimeCardGrid(props: {
  anime: AnimeSummary[];
  categories?: Category[];
  preferAnilistDisplayTitle: boolean;
  onOpenAnime: (anime: AnimeSummary) => void;
  onDeleteAnime?: (anime: AnimeSummary) => void;
  onMoveAnime?: (anime: AnimeSummary, categoryId: number) => void;
  onOpenAnimeFolder?: (anime: AnimeSummary) => void;
  onSetAnimeThumbnail?: (anime: AnimeSummary) => void;
}) {
  const {
    anime,
    categories = [],
    preferAnilistDisplayTitle,
    onOpenAnime,
    onDeleteAnime,
    onMoveAnime,
    onOpenAnimeFolder,
    onSetAnimeThumbnail,
  } = props;
  const [covers, setCovers] = useState<Record<number, string>>({});
  const [deleteAnime, setDeleteAnime] = useState<AnimeSummary | null>(null);
  const getRovingItemProps = useRovingListNavigation(anime.length);
  const { menu, openMenu, closeMenu } = useContextMenu();

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

  const contextMenuEnabled = Boolean(
    onDeleteAnime && onMoveAnime && onOpenAnimeFolder && onSetAnimeThumbnail && categories.length > 0,
  );

  const buildAnimeMenuItems = useCallback(
    (item: AnimeSummary): ContextMenuItem[] => [
      {
        type: "submenu",
        id: "move-to",
        label: "Move to",
        items: categories.map((category) => ({
          id: `category-${category.id}`,
          label: category.name,
          disabled: category.id === item.category_id,
          onSelect: () => onMoveAnime?.(item, category.id),
        })),
      },
      {
        type: "action",
        id: "open-folder",
        label: "Open folder",
        disabled: item.episode_count === 0,
        onSelect: () => onOpenAnimeFolder?.(item),
      },
      {
        type: "action",
        id: "set-thumbnail",
        label: "Set thumbnail",
        onSelect: () => onSetAnimeThumbnail?.(item),
      },
      { type: "separator", id: "delete-separator" },
      {
        type: "action",
        id: "delete",
        label: "Delete",
        danger: true,
        onSelect: () => setDeleteAnime(item),
      },
    ],
    [categories, onDeleteAnime, onMoveAnime, onOpenAnimeFolder, onSetAnimeThumbnail],
  );

  return (
    <>
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
              onContextMenu={
                contextMenuEnabled
                  ? (event) => openMenu(event, buildAnimeMenuItems(item))
                  : undefined
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
      {contextMenuEnabled ? <ContextMenu menu={menu} onClose={closeMenu} /> : null}

      {deleteAnime ? (
        <ConfirmModal
          title="Delete title files?"
          description={`Delete all ${deleteAnime.episode_count} episode file${deleteAnime.episode_count === 1 ? "" : "s"} for "${deleteAnime.title}"?`}
          warning="Files will be moved to the trash when possible. Library progress, cached covers, and scrub thumbnails for this title will also be removed."
          onConfirm={() => {
            onDeleteAnime?.(deleteAnime);
            setDeleteAnime(null);
          }}
          onClose={() => setDeleteAnime(null)}
        />
      ) : null}
    </>
  );
}
