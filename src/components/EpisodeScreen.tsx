import { useEffect, useMemo, useState } from "react";
import { getFileThumbnail } from "../api";
import { pickQuickPlayEpisode } from "../quickPlay";
import type { AnimeSummary, Category, Episode } from "../types";
import { formatEpisodeNumber, formatSize, formatTime, progressPercent } from "../utils";
import { CustomDropdown } from "./CustomDropdown";
import { ViewHeader } from "./ViewHeader";

export function EpisodeScreen(props: {
  anime: AnimeSummary;
  episodes: Episode[];
  categories: Category[];
  onBack: () => void;
  onPlay: (episode: Episode) => void;
  onMoveAnime: (categoryId: number) => void;
}) {
  const { anime, episodes, categories, onBack, onPlay, onMoveAnime } = props;
  // Highlight whichever episode the Q hotkey would launch right now, so the
  // pill always points at the same target as the keybind.
  const quickPlayEpisodeId = useMemo(() => pickQuickPlayEpisode(episodes)?.id ?? null, [episodes]);
  const [episodeThumbnails, setEpisodeThumbnails] = useState<Record<number, string>>({});
  const unwatchedCount = episodes.filter((episode) => !episode.watched).length;
  const selectedCategory = categories.find((category) => category.id === anime.category_id);

  useEffect(() => {
    let cancelled = false;
    setEpisodeThumbnails({});

    void Promise.all(
      episodes.map(async (episode) => {
        try {
          const thumbnail = await getFileThumbnail(episode.path, 184);
          return thumbnail ? ([episode.id, thumbnail] as const) : null;
        } catch {
          return null;
        }
      }),
    ).then((entries) => {
      if (cancelled) return;

      setEpisodeThumbnails(
        Object.fromEntries(entries.filter((entry): entry is readonly [number, string] => entry !== null)),
      );
    });

    return () => {
      cancelled = true;
    };
  }, [episodes]);

  return (
    <>
      <ViewHeader
        title={anime.title}
        subtitle={`${episodes.length} episode${episodes.length === 1 ? "" : "s"} - ${unwatchedCount} unwatched`}
        onBack={onBack}
        action={
          <CustomDropdown
            label={selectedCategory?.name ?? "Select category"}
            options={categories.map((category) => ({ value: category.id, label: category.name }))}
            value={anime.category_id}
            onChange={onMoveAnime}
          />
        }
      />

      <section className="episode-list">
        {episodes.map((episode) => {
          const percent = progressPercent(episode.position_seconds, episode.duration_seconds);
          const thumbnail = episodeThumbnails[episode.id];
          return (
            <button
              type="button"
              key={episode.id}
              className={`episode-row${episode.watched ? " episode-row--watched" : ""}${episode.id === quickPlayEpisodeId ? " episode-row--last" : ""}`}
              onClick={() => onPlay(episode)}
              title={episode.path}
            >
              <div className={`episode-thumb${thumbnail ? " episode-thumb--image" : ""}`}>
                {thumbnail ? <img src={thumbnail} alt="" loading="lazy" /> : episode.file_type.toUpperCase()}
              </div>
              <div className="episode-main">
                <div className="episode-title">
                  <span>{formatEpisodeNumber(episode.episode_number)}</span>
                  {episode.id === quickPlayEpisodeId ? <span className="pill">Up next</span> : null}
                </div>
                <div className="episode-meta">
                  <span>{episode.file_name}</span>
                  <span>{formatSize(episode.size)}</span>
                  {episode.duration_seconds > 0 ? <span>{formatTime(episode.duration_seconds)}</span> : null}
                </div>
                <div className="episode-progress">
                  <span style={{ width: `${percent}%` }} />
                </div>
              </div>
            </button>
          );
        })}
      </section>
    </>
  );
}
