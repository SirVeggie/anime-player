import { useEffect, useMemo, useState } from "react";
import { resolveAnimePosterUrl } from "../animePoster";
import type { AnimeSummary, LibraryState } from "../types";
import { useRovingListNavigation } from "../useRovingListNavigation";
import { ViewHeader } from "./ViewHeader";

export function CategoryScreen(props: {
  library: LibraryState;
  onOpenCategory: (categoryId: number) => void;
  onOpenAnime: (anime: AnimeSummary) => void;
  onOpenSettings: () => void;
}) {
  const { library, onOpenCategory, onOpenAnime, onOpenSettings } = props;
  const getRovingItemProps = useRovingListNavigation(library.categories.length + library.recent_anime.length);
  const [recentCovers, setRecentCovers] = useState<Record<number, string>>({});
  const animeByCategory = useMemo(() => {
    const counts = new Map<number, number>();
    for (const anime of library.anime) {
      counts.set(anime.category_id, (counts.get(anime.category_id) ?? 0) + 1);
    }
    return counts;
  }, [library.anime]);

  useEffect(() => {
    let cancelled = false;
    setRecentCovers({});
    void Promise.all(
      library.recent_anime.map(async (anime) => {
        const url = await resolveAnimePosterUrl(anime);
        return url ? ([anime.id, url] as const) : null;
      }),
    ).then((entries) => {
      if (cancelled) return;
      setRecentCovers(Object.fromEntries(entries.filter((entry): entry is readonly [number, string] => entry !== null)));
    });
    return () => {
      cancelled = true;
    };
  }, [library.recent_anime]);

  return (
    <>
      <ViewHeader
        title="Library"
        subtitle="Browse your local anime by category, or continue where you left off."
      />

      {library.root_folders.length === 0 ? (
        <div className="empty empty--wide">
          <h2>Add a root folder to begin</h2>
          <p className="muted">The library scanner will group matching anime filenames into shows and episodes.</p>
          <button type="button" onClick={onOpenSettings}>
            Open settings
          </button>
        </div>
      ) : null}

      <section className="category-grid">
        {library.categories.map((category, index) => (
          <button
            type="button"
            className="category-card"
            key={category.id}
            onClick={() => onOpenCategory(category.id)}
            {...getRovingItemProps(index)}
          >
            <span className="category-name">{category.name}</span>
            <span className="category-count">{animeByCategory.get(category.id) ?? 0} anime</span>
          </button>
        ))}
      </section>

      {library.recent_anime.length > 0 ? (
        <>
          <div className="section-heading">
            <h2>Continue Watching</h2>
          </div>
          <div className="continue-grid">
            {library.recent_anime.map((anime, index) => {
              const cover = recentCovers[anime.id];
              return (
                <button
                  type="button"
                  className={`continue-card${cover ? " continue-card--with-cover" : ""}`}
                  key={anime.id}
                  onClick={() => onOpenAnime(anime)}
                  {...getRovingItemProps(library.categories.length + index)}
                >
                  {cover ? (
                    <img className="continue-card-cover" src={cover} alt="" loading="lazy" />
                  ) : null}
                  <div className="continue-card-body">
                    <strong>{anime.title}</strong>
                    <span>{anime.episode_count} episodes</span>
                  </div>
                </button>
              );
            })}
          </div>
        </>
      ) : null}
    </>
  );
}
