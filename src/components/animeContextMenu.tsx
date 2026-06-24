import { useCallback, useState, type MouseEvent as ReactMouseEvent } from "react";
import type { AnimeSummary, AnilistSearchResult, Category } from "../types";
import { AnilistLinkModal } from "./AnilistLinkModal";
import { ConfirmModal } from "./ConfirmModal";
import { ContextMenu, useContextMenu, type ContextMenuItem } from "./ContextMenu";

export type AnimeContextMenuHandlers = {
  categories: Category[];
  onDeleteAnime: (anime: AnimeSummary) => void;
  onMoveAnime: (anime: AnimeSummary, categoryId: number) => void;
  onOpenAnimeFolder: (anime: AnimeSummary) => void;
  onSetAnimeThumbnail: (anime: AnimeSummary) => void;
  anilistFeaturesEnabled?: boolean;
  onSearchAnilist?: (query: string) => Promise<AnilistSearchResult[]>;
  onLinkAnilist?: (animeId: number, anilistId: number) => void;
};

export function useAnimeContextMenu(handlers: AnimeContextMenuHandlers | null) {
  const { menu, openMenu, closeMenu } = useContextMenu();
  const [deleteAnime, setDeleteAnime] = useState<AnimeSummary | null>(null);
  const [linkAnime, setLinkAnime] = useState<AnimeSummary | null>(null);

  const enabled = Boolean(handlers && handlers.categories.length > 0);

  const buildMenuItems = useCallback(
    (item: AnimeSummary): ContextMenuItem[] => {
      if (!handlers) return [];
      const items: ContextMenuItem[] = [
        {
          type: "submenu",
          id: "move-to",
          label: "Move to",
          items: handlers.categories.map((category) => ({
            id: `category-${category.id}`,
            label: category.name,
            disabled: category.id === item.category_id,
            onSelect: () => handlers.onMoveAnime(item, category.id),
          })),
        },
        {
          type: "action",
          id: "open-folder",
          label: "Open folder",
          disabled: item.episode_count === 0,
          onSelect: () => handlers.onOpenAnimeFolder(item),
        },
        {
          type: "action",
          id: "set-thumbnail",
          label: "Set thumbnail",
          onSelect: () => handlers.onSetAnimeThumbnail(item),
        },
      ];

      if (
        handlers.anilistFeaturesEnabled &&
        !item.anilist_id &&
        handlers.onSearchAnilist &&
        handlers.onLinkAnilist
      ) {
        items.push({
          type: "action",
          id: "link-anilist",
          label: "Link AniList",
          onSelect: () => setLinkAnime(item),
        });
      }

      items.push(
        { type: "separator", id: "delete-separator" },
        {
          type: "action",
          id: "delete",
          label: "Delete",
          danger: true,
          onSelect: () => setDeleteAnime(item),
        },
      );

      return items;
    },
    [handlers],
  );

  const openAnimeMenu = useCallback(
    (event: ReactMouseEvent, item: AnimeSummary) => {
      if (!enabled) return;
      openMenu(event, buildMenuItems(item));
    },
    [buildMenuItems, enabled, openMenu],
  );

  const menuUi = enabled && handlers ? (
    <>
      <ContextMenu menu={menu} onClose={closeMenu} />

      {linkAnime && handlers.onSearchAnilist && handlers.onLinkAnilist ? (
        <AnilistLinkModal
          animeTitle={linkAnime.title}
          open
          onClose={() => setLinkAnime(null)}
          onSearch={handlers.onSearchAnilist}
          onSelect={(anilistId) => {
            handlers.onLinkAnilist?.(linkAnime.id, anilistId);
            setLinkAnime(null);
          }}
        />
      ) : null}

      {deleteAnime ? (
        <ConfirmModal
          title="Delete title files?"
          description={`Delete all ${deleteAnime.episode_count} episode file${deleteAnime.episode_count === 1 ? "" : "s"} for "${deleteAnime.title}"?`}
          warning="Files will be moved to the trash when possible. Library progress, cached covers, and scrub thumbnails for this title will also be removed."
          onConfirm={() => {
            handlers.onDeleteAnime(deleteAnime);
            setDeleteAnime(null);
          }}
          onClose={() => setDeleteAnime(null)}
        />
      ) : null}
    </>
  ) : null;

  return { enabled, openAnimeMenu, menuUi };
}
