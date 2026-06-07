import { useCallback, useEffect, useRef, useState } from "react";
import type { AnilistSearchResult } from "../types";

export function AnilistLinkModal(props: {
  animeTitle: string;
  open: boolean;
  onClose: () => void;
  onSearch: (query: string) => Promise<AnilistSearchResult[]>;
  onSelect: (anilistId: number) => void;
}) {
  const { animeTitle, open, onClose, onSearch, onSelect } = props;
  const [linkQuery, setLinkQuery] = useState(animeTitle);
  const [linkResults, setLinkResults] = useState<AnilistSearchResult[]>([]);
  const [linkSearchBusy, setLinkSearchBusy] = useState(false);
  const [linkSearchError, setLinkSearchError] = useState<string | null>(null);
  const linkSearchRequestRef = useRef(0);

  const runLinkSearch = useCallback(
    async (queryOverride?: string) => {
      const query = (queryOverride ?? linkQuery).trim();
      if (!query) return;
      const requestId = linkSearchRequestRef.current + 1;
      linkSearchRequestRef.current = requestId;
      setLinkSearchBusy(true);
      setLinkSearchError(null);
      try {
        const results = await onSearch(query);
        if (linkSearchRequestRef.current === requestId) {
          setLinkResults(results);
        }
      } catch (e) {
        if (linkSearchRequestRef.current === requestId) {
          setLinkSearchError(e instanceof Error ? e.message : String(e));
        }
      } finally {
        if (linkSearchRequestRef.current === requestId) {
          setLinkSearchBusy(false);
        }
      }
    },
    [linkQuery, onSearch],
  );

  useEffect(() => {
    if (!open) return;
    setLinkQuery(animeTitle);
    setLinkResults([]);
    setLinkSearchError(null);
    void runLinkSearch(animeTitle);
  }, [animeTitle, open, runLinkSearch]);

  const close = useCallback(() => {
    linkSearchRequestRef.current += 1;
    setLinkSearchBusy(false);
    setLinkSearchError(null);
    onClose();
  }, [onClose]);

  if (!open) return null;

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <section
        className="modal anilist-link-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="anilist-link-title"
      >
        <div className="modal-heading">
          <div>
            <h2 id="anilist-link-title">Link AniList</h2>
            <p className="muted">Pick the AniList entry that matches "{animeTitle}".</p>
          </div>
          <button type="button" onClick={close} aria-label="Close AniList linking">
            Close
          </button>
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
        <div className="anilist-results" aria-busy={linkSearchBusy}>
          {linkResults.map((result) => (
            <button
              type="button"
              className="anilist-result"
              key={result.id}
              onClick={() => {
                close();
                onSelect(result.id);
              }}
            >
              {result.cover_image_url ? <img src={result.cover_image_url} alt="" loading="lazy" /> : null}
              <span>
                <strong>{result.title}</strong>
                {result.native_title ? <em>{result.native_title}</em> : null}
                <small>
                  {[result.season_year, result.format, result.episodes ? `${result.episodes} eps` : null]
                    .filter(Boolean)
                    .join(" - ")}
                </small>
              </span>
            </button>
          ))}
          {!linkSearchBusy && linkResults.length === 0 ? (
            <p className="muted">No matches yet. Try a different search title.</p>
          ) : null}
        </div>
      </section>
    </div>
  );
}
