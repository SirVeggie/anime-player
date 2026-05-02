import { useEffect, useState } from "react";
import { getAnilistCoverImage } from "../api";
import type { AnimeSummary, Category } from "../types";
import { ViewHeader } from "./ViewHeader";

export function AnimeGrid(props: {
  category: Category | null;
  anime: AnimeSummary[];
  onBack: () => void;
  onOpenAnime: (anime: AnimeSummary) => void;
  onOpenSettings: () => void;
}) {
  const { category, anime, onBack, onOpenAnime, onOpenSettings } = props;
  const [covers, setCovers] = useState<Record<number, string>>({});

  useEffect(() => {
    let cancelled = false;
    setCovers({});
    void Promise.all(
      anime
        .filter((item) => item.anilist_cover_path)
        .map(async (item) => {
          try {
            const cover = await getAnilistCoverImage(item.id);
            return cover ? ([item.id, cover] as const) : null;
          } catch {
            return null;
          }
        }),
    ).then((entries) => {
      if (cancelled) return;
      setCovers(Object.fromEntries(entries.filter((entry): entry is readonly [number, string] => entry !== null)));
    });
    return () => {
      cancelled = true;
    };
  }, [anime]);

  return (
    <>
      <ViewHeader
        title={category?.name ?? "Anime"}
        subtitle={`${anime.length} title${anime.length === 1 ? "" : "s"} in this category.`}
        onBack={onBack}
      />
      {anime.length === 0 ? (
        <div className="empty empty--wide">
          <h2>No anime found here yet</h2>
          <p className="muted">Add root folders and rescan from settings, or move anime into this category later.</p>
          <button type="button" onClick={onOpenSettings}>
            Open settings
          </button>
        </div>
      ) : (
        <div className="anime-grid">
          {anime.map((item) => {
            const cover = covers[item.id];
            return (
              <button type="button" className="anime-card" key={item.id} onClick={() => onOpenAnime(item)}>
                <div className={`poster-placeholder${cover ? " poster-placeholder--image" : ""}`}>
                  {cover ? <img src={cover} alt="" loading="lazy" /> : item.title.slice(0, 2).toUpperCase()}
                </div>
                <div className="anime-card-body">
                  <div className="anime-card-title" title={item.title}>
                    {item.title}
                  </div>
                  <div className="anime-card-meta">
                    {item.episode_count} eps - {item.unwatched_count} unwatched
                    {item.anilist_id ? " - AniList" : ""}
                  </div>
                </div>
                <div className="anime-tooltip">{item.anilist_title ?? item.title}</div>
              </button>
            );
          })}
        </div>
      )}
    </>
  );
}
