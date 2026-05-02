import { useEffect, useState } from "react";
import { loadAnimePosterUrls } from "../animePoster";
import type { MissingAnimeSummary } from "../types";
import { ViewHeader } from "./ViewHeader";

export function MissingScreen(props: {
  anime: MissingAnimeSummary[];
}) {
  const { anime } = props;
  const [covers, setCovers] = useState<Record<number, string>>({});

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
            const cover = covers[item.id];
            return (
              <article className="anime-card missing-card" key={item.id}>
                <div className={`poster-placeholder${cover ? " poster-placeholder--image" : ""}`}>
                  {cover ? <img src={cover} alt="" loading="lazy" /> : item.title.slice(0, 2).toUpperCase()}
                </div>
                <div className="anime-card-body">
                  <div className="anime-card-title" title={item.title}>
                    {item.title}
                  </div>
                  <div className="anime-card-meta">{missingMeta(item)}</div>
                </div>
                <div className="anime-tooltip">{item.anilist_title ?? item.title}</div>
              </article>
            );
          })}
        </div>
      )}
    </>
  );
}

function missingMeta(anime: MissingAnimeSummary): string {
  if (anime.missing_episode_count >= anime.total_episode_count) return "missing";
  return `${anime.missing_episode_count}/${anime.total_episode_count} missing`;
}
