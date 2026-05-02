import { useEffect, useMemo, useState } from "react";
import { getAnilistCoverImage, getFileThumbnail } from "../api";
import { pickQuickPlayEpisode } from "../quickPlay";
import type { AnimeSummary, AnilistSearchResult, Category, Episode } from "../types";
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
  onSearchAnilist: (query: string) => Promise<AnilistSearchResult[]>;
  onLinkAnilist: (animeId: number, anilistId: number) => void;
  onUnlinkAnilist: (animeId: number) => void;
  onOpenAnilist: (url: string) => void;
}) {
  const {
    anime,
    episodes,
    categories,
    onBack,
    onPlay,
    onMoveAnime,
    onSearchAnilist,
    onLinkAnilist,
    onUnlinkAnilist,
    onOpenAnilist,
  } = props;
  // Highlight whichever episode the Q hotkey would launch right now, so the
  // pill always points at the same target as the keybind.
  const quickPlayEpisodeId = useMemo(() => pickQuickPlayEpisode(episodes)?.id ?? null, [episodes]);
  const [episodeThumbnails, setEpisodeThumbnails] = useState<Record<number, string>>({});
  const [animeCover, setAnimeCover] = useState<string | null>(null);
  const [linkQuery, setLinkQuery] = useState(anime.title);
  const [linkResults, setLinkResults] = useState<AnilistSearchResult[]>([]);
  const [linkSearchOpen, setLinkSearchOpen] = useState(false);
  const [linkSearchBusy, setLinkSearchBusy] = useState(false);
  const [linkSearchError, setLinkSearchError] = useState<string | null>(null);
  const unwatchedCount = episodes.filter((episode) => !episode.watched).length;
  const selectedCategory = categories.find((category) => category.id === anime.category_id);

  useEffect(() => {
    setLinkQuery(anime.title);
    setLinkResults([]);
    setLinkSearchOpen(false);
    setLinkSearchError(null);
  }, [anime.id, anime.title]);

  useEffect(() => {
    let cancelled = false;
    setAnimeCover(null);
    if (!anime.anilist_cover_path) return;
    void getAnilistCoverImage(anime.id)
      .then((cover) => {
        if (!cancelled) setAnimeCover(cover);
      })
      .catch(() => {
        if (!cancelled) setAnimeCover(null);
      });
    return () => {
      cancelled = true;
    };
  }, [anime.anilist_cover_path, anime.id]);

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

  const runLinkSearch = async () => {
    const query = linkQuery.trim();
    if (!query) return;
    setLinkSearchBusy(true);
    setLinkSearchError(null);
    try {
      setLinkResults(await onSearchAnilist(query));
    } catch (e) {
      setLinkSearchError(e instanceof Error ? e.message : String(e));
    } finally {
      setLinkSearchBusy(false);
    }
  };

  return (
    <>
      <ViewHeader
        title={anime.title}
        subtitle={`${episodes.length} episode${episodes.length === 1 ? "" : "s"} - ${unwatchedCount} unwatched`}
        onBack={onBack}
        action={
          <>
            <CustomDropdown
              label={selectedCategory?.name ?? "Select category"}
              options={categories.map((category) => ({ value: category.id, label: category.name }))}
              value={anime.category_id}
              onChange={onMoveAnime}
            />
            {anime.anilist_site_url ? (
              <>
                <button type="button" onClick={() => onOpenAnilist(anime.anilist_site_url ?? "")}>
                  Open AniList
                </button>
                <button type="button" onClick={() => onUnlinkAnilist(anime.id)}>
                  Unlink
                </button>
              </>
            ) : (
              <button type="button" onClick={() => setLinkSearchOpen((open) => !open)}>
                Link AniList
              </button>
            )}
          </>
        }
      />

      <section className="anime-detail-panel">
        <div className={`anime-detail-cover${animeCover ? " anime-detail-cover--image" : ""}`}>
          {animeCover ? <img src={animeCover} alt="" /> : anime.title.slice(0, 2).toUpperCase()}
        </div>
        <div>
          <h2>{anime.anilist_title ?? anime.title}</h2>
          <p className="muted">
            {anime.anilist_id ? `Linked to AniList #${anime.anilist_id}` : "Not linked to AniList yet."}
          </p>
        </div>
      </section>

      {linkSearchOpen && !anime.anilist_id ? (
        <section className="panel anilist-link-panel">
          <div className="panel-heading">
            <h2>Link AniList</h2>
          </div>
          <form
            className="form-row"
            onSubmit={(e) => {
              e.preventDefault();
              void runLinkSearch();
            }}
          >
            <input type="text" value={linkQuery} onChange={(e) => setLinkQuery(e.currentTarget.value)} />
            <button type="submit" disabled={linkSearchBusy || !linkQuery.trim()}>
              {linkSearchBusy ? "Searching..." : "Search"}
            </button>
          </form>
          {linkSearchError ? <p className="error">{linkSearchError}</p> : null}
          <div className="anilist-results">
            {linkResults.map((result) => (
              <button
                type="button"
                className="anilist-result"
                key={result.id}
                onClick={() => onLinkAnilist(anime.id, result.id)}
              >
                {result.cover_image_url ? <img src={result.cover_image_url} alt="" loading="lazy" /> : null}
                <span>
                  <strong>{result.title}</strong>
                  <small>
                    {[result.season_year, result.format, result.episodes ? `${result.episodes} eps` : null]
                      .filter(Boolean)
                      .join(" - ")}
                  </small>
                </span>
              </button>
            ))}
          </div>
        </section>
      ) : null}

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
