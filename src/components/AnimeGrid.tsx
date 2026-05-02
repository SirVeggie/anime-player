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
          {anime.map((item) => (
            <button type="button" className="anime-card" key={item.id} onClick={() => onOpenAnime(item)}>
              <div className="poster-placeholder">{item.title.slice(0, 2).toUpperCase()}</div>
              <div className="anime-card-body">
                <div className="anime-card-title" title={item.title}>
                  {item.title}
                </div>
                <div className="anime-card-meta">
                  {item.episode_count} eps - {item.unwatched_count} unwatched
                </div>
              </div>
              <div className="anime-tooltip">{item.title}</div>
            </button>
          ))}
        </div>
      )}
    </>
  );
}
