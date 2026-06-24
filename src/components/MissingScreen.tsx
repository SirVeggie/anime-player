import { useCallback, useEffect, useState, type MouseEvent as ReactMouseEvent } from "react";
import {
  animePosterSourceKey,
  cachedThumbnailUrl,
  loadAnimePosterUrls,
  pruneThumbnailUrlCache,
  type ThumbnailUrlCache,
} from "../animePoster";
import type { MissingAnimeSummary } from "../types";
import { animeDisplayTitle, animeTooltipTitle } from "../utils";
import { AnimeCardLabel } from "./AnimeCardLabel";
import { ConfirmModal } from "./ConfirmModal";
import { ContextMenu, useContextMenu, type ContextMenuItem } from "./ContextMenu";
import { ViewHeader } from "./ViewHeader";

export function MissingScreen(props: {
  anime: MissingAnimeSummary[];
  preferAnilistDisplayTitle: boolean;
  onClearLocalData: (anime: MissingAnimeSummary) => void;
}) {
  const { anime, preferAnilistDisplayTitle, onClearLocalData } = props;
  const [covers, setCovers] = useState<ThumbnailUrlCache>({});
  const { menu, openMenu, closeMenu } = useContextMenu();
  const [deleteAnime, setDeleteAnime] = useState<MissingAnimeSummary | null>(null);

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

  const buildMenuItems = useCallback(
    (item: MissingAnimeSummary): ContextMenuItem[] => [
      {
        type: "action",
        id: "delete",
        label: "Delete",
        danger: true,
        onSelect: () => setDeleteAnime(item),
      },
    ],
    [],
  );

  const openAnimeMenu = useCallback(
    (event: ReactMouseEvent, item: MissingAnimeSummary) => {
      openMenu(event, buildMenuItems(item));
    },
    [buildMenuItems, openMenu],
  );

  return (
    <>
      <ViewHeader
        title="Missing"
        subtitle={`${anime.length} title${anime.length === 1 ? "" : "s"} have episodes hidden from the main library.`}
      />
      {anime.length === 0 ? (
        <div className="empty empty--wide">
          <h2>No missing episodes</h2>
          <p className="muted">Everything in the database is currently matched by your root folders and detection rules.</p>
        </div>
      ) : (
        <div className="anime-grid">
          {anime.map((item) => {
            const cover = cachedThumbnailUrl(covers, item, animePosterSourceKey);
            const displayTitle = animeDisplayTitle(item, preferAnilistDisplayTitle);
            const tooltipTitle = animeTooltipTitle(item);
            return (
              <article
                className="anime-card missing-card"
                key={item.id}
                onContextMenu={(event) => openAnimeMenu(event, item)}
              >
                <div className={`poster-placeholder${cover ? " poster-placeholder--image" : ""}`}>
                  {cover ? <img src={cover} alt="" loading="lazy" /> : displayTitle.slice(0, 2).toUpperCase()}
                </div>
                <AnimeCardLabel
                  displayTitle={displayTitle}
                  tooltipTitle={tooltipTitle}
                  meta={<div className="anime-card-meta">{missingMeta(item)}</div>}
                />
              </article>
            );
          })}
        </div>
      )}

      <ContextMenu menu={menu} onClose={closeMenu} />

      {deleteAnime ? (
        <ConfirmModal
          title="Clear local data?"
          description={`Remove "${deleteAnime.title}" from the library database?`}
          warning="Saved progress, cached covers, scrub thumbnails, and episode rows for this title will be removed. Video files on disk are not deleted."
          onConfirm={() => {
            onClearLocalData(deleteAnime);
            setDeleteAnime(null);
          }}
          onClose={() => setDeleteAnime(null)}
        />
      ) : null}
    </>
  );
}

function missingMeta(anime: MissingAnimeSummary): string {
  if (anime.missing_episode_count >= anime.total_episode_count) return "missing";
  return `${anime.missing_episode_count}/${anime.total_episode_count} missing`;
}
