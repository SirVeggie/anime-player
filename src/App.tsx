import { useCallback, useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  addRootFolder,
  createCategory,
  deleteCategory,
  getLibraryState,
  listEpisodes,
  moveAnimeToCategory,
  removeRootFolder,
  rescanLibrary,
} from "./api";
import { PlayerView } from "./components/PlayerView";
import type { AnimeSummary, Category, Episode, LibraryState, RootFolder } from "./types";
import { errorMessage, formatEpisodeNumber, formatSize, formatTime, progressPercent } from "./utils";
import "./App.css";

type View = "categories" | "anime" | "episodes" | "settings" | "player";

function App() {
  const [library, setLibrary] = useState<LibraryState | null>(null);
  const [view, setView] = useState<View>("categories");
  const [selectedCategoryId, setSelectedCategoryId] = useState<number | null>(null);
  const [selectedAnime, setSelectedAnime] = useState<AnimeSummary | null>(null);
  const [episodes, setEpisodes] = useState<Episode[]>([]);
  const [selectedEpisode, setSelectedEpisode] = useState<Episode | null>(null);
  const [rootInput, setRootInput] = useState("");
  const [newCategoryName, setNewCategoryName] = useState("");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reloadLibrary = useCallback(async () => {
    const state = await getLibraryState();
    setLibrary(state);
    if (selectedCategoryId === null && state.categories.length > 0) {
      setSelectedCategoryId(state.categories[0].id);
    }
    return state;
  }, [selectedCategoryId]);

  useEffect(() => {
    void (async () => {
      try {
        await reloadLibrary();
      } catch (e) {
        setError(errorMessage(e));
      } finally {
        setLoading(false);
      }
    })();
  }, [reloadLibrary]);

  const selectedCategory = useMemo(() => {
    if (!library || selectedCategoryId === null) return null;
    return library.categories.find((category) => category.id === selectedCategoryId) ?? null;
  }, [library, selectedCategoryId]);

  const animeInCategory = useMemo(() => {
    if (!library || selectedCategoryId === null) return [];
    return library.anime.filter((anime) => anime.category_id === selectedCategoryId);
  }, [library, selectedCategoryId]);

  const runAction = useCallback(
    async (action: () => Promise<void>) => {
      setBusy(true);
      setError(null);
      setStatus(null);
      try {
        await action();
      } catch (e) {
        setError(errorMessage(e));
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  const openAnime = useCallback(
    async (anime: AnimeSummary) => {
      setSelectedAnime(anime);
      setError(null);
      try {
        const nextEpisodes = await listEpisodes(anime.id);
        setEpisodes(nextEpisodes);
        setView("episodes");
      } catch (e) {
        setError(errorMessage(e));
      }
    },
    [],
  );

  const handleAddRoot = useCallback(
    async (path: string) => {
      const trimmed = path.trim();
      if (!trimmed) {
        setError("Choose or paste a folder path first.");
        return;
      }
      await runAction(async () => {
        await addRootFolder(trimmed);
        setRootInput("");
        await reloadLibrary();
        setStatus("Root folder added.");
      });
    },
    [reloadLibrary, runAction],
  );

  const handlePickFolder = useCallback(async () => {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string" && picked) {
      setRootInput(picked);
      await handleAddRoot(picked);
    }
  }, [handleAddRoot]);

  const handleRemoveRoot = useCallback(
    async (root: RootFolder) => {
      await runAction(async () => {
        await removeRootFolder(root.id);
        await reloadLibrary();
        setStatus("Root folder removed. Existing library entries are preserved until rescanned.");
      });
    },
    [reloadLibrary, runAction],
  );

  const handleRescan = useCallback(async () => {
    await runAction(async () => {
      const summary = await rescanLibrary();
      await reloadLibrary();
      setStatus(
        `Scanned ${summary.roots_scanned} root folder${summary.roots_scanned === 1 ? "" : "s"}: ${summary.episodes_imported} episode${summary.episodes_imported === 1 ? "" : "s"} imported, ${summary.unmatched_files} unmatched.`,
      );
    });
  }, [reloadLibrary, runAction]);

  const handleCreateCategory = useCallback(async () => {
    const name = newCategoryName.trim();
    if (!name) return;
    await runAction(async () => {
      const category = await createCategory(name);
      setNewCategoryName("");
      setSelectedCategoryId(category.id);
      await reloadLibrary();
      setStatus("Category created.");
    });
  }, [newCategoryName, reloadLibrary, runAction]);

  const handleDeleteCategory = useCallback(
    async (category: Category) => {
      await runAction(async () => {
        await deleteCategory(category.id);
        const state = await reloadLibrary();
        setSelectedCategoryId(state.categories[0]?.id ?? null);
        setStatus("Category deleted. Anime were moved to the default category.");
      });
    },
    [reloadLibrary, runAction],
  );

  const handleMoveAnime = useCallback(
    async (categoryId: number) => {
      if (!selectedAnime) return;
      await runAction(async () => {
        await moveAnimeToCategory(selectedAnime.id, categoryId);
        const state = await reloadLibrary();
        const updated = state.anime.find((anime) => anime.id === selectedAnime.id);
        if (updated) setSelectedAnime(updated);
        setStatus("Anime moved.");
      });
    },
    [reloadLibrary, runAction, selectedAnime],
  );

  const handleProgressSaved = useCallback(
    (saved: Episode) => {
      setEpisodes((current) => current.map((episode) => (episode.id === saved.id ? saved : episode)));
      void reloadLibrary().catch((e) => setError(errorMessage(e)));
    },
    [reloadLibrary],
  );

  const navigateToCategory = useCallback((categoryId: number) => {
    setSelectedCategoryId(categoryId);
    setSelectedAnime(null);
    setSelectedEpisode(null);
    setEpisodes([]);
    setView("anime");
  }, []);

  if (loading) {
    return (
      <main className="app app--loading">
        <div className="empty">
          <h2>Loading library...</h2>
        </div>
      </main>
    );
  }

  if (!library) {
    return (
      <main className="app app--loading">
        <div className="empty">
          <h2>Library failed to load</h2>
          {error ? <p className="muted">{error}</p> : null}
        </div>
      </main>
    );
  }

  const showPlayer = view === "player" && selectedEpisode;

  return (
    <main className={`app${showPlayer ? " app--player-open" : ""}`}>
      <aside className="sidebar">
        <header className="sidebar-header">
          <h1>Anime Player</h1>
          <p className="muted">Portable local library</p>
        </header>

        <nav className="nav-list">
          <button
            type="button"
            className={view === "categories" ? "nav-item active" : "nav-item"}
            onClick={() => setView("categories")}
          >
            Library
          </button>
          {library.categories.map((category) => (
            <button
              type="button"
              key={category.id}
              className={
                view === "anime" && selectedCategoryId === category.id ? "nav-item active" : "nav-item"
              }
              onClick={() => navigateToCategory(category.id)}
            >
              <span>{category.name}</span>
              {category.is_default ? <span className="pill">Default</span> : null}
            </button>
          ))}
          <button
            type="button"
            className={view === "settings" ? "nav-item active" : "nav-item"}
            onClick={() => setView("settings")}
          >
            Settings
          </button>
        </nav>

        <div className="sidebar-footer">
          <div className="stat-line">
            <span>{library.anime.length} anime</span>
            <span>{library.unmatched_count} unmatched</span>
          </div>
          <button type="button" onClick={() => void handleRescan()} disabled={busy}>
            {busy ? "Working..." : "Rescan library"}
          </button>
        </div>
      </aside>

      {showPlayer ? (
        <PlayerView
          episode={selectedEpisode}
          playlist={episodes}
          onSelectEpisode={setSelectedEpisode}
          onBack={() => setView("episodes")}
          onProgressSaved={handleProgressSaved}
          onError={setError}
        />
      ) : (
        <section className="content">
          <div className="content-inner">
            {error ? <div className="error">{error}</div> : null}
            {status ? <div className="status">{status}</div> : null}

            {view === "categories" ? (
              <CategoryScreen
                library={library}
                onOpenCategory={navigateToCategory}
                onOpenAnime={openAnime}
                onOpenSettings={() => setView("settings")}
              />
            ) : null}

            {view === "anime" ? (
              <AnimeGrid
                category={selectedCategory}
                anime={animeInCategory}
                onOpenAnime={openAnime}
                onOpenSettings={() => setView("settings")}
              />
            ) : null}

            {view === "episodes" && selectedAnime ? (
              <EpisodeScreen
                anime={selectedAnime}
                episodes={episodes}
                categories={library.categories}
                onBack={() => setView("anime")}
                onPlay={(episode) => {
                  setSelectedEpisode(episode);
                  setView("player");
                }}
                onMoveAnime={(categoryId) => void handleMoveAnime(categoryId)}
              />
            ) : null}

            {view === "settings" ? (
              <SettingsScreen
                library={library}
                busy={busy}
                rootInput={rootInput}
                newCategoryName={newCategoryName}
                onRootInput={setRootInput}
                onPickFolder={() => void handlePickFolder()}
                onAddRoot={() => void handleAddRoot(rootInput)}
                onRemoveRoot={(root) => void handleRemoveRoot(root)}
                onRescan={() => void handleRescan()}
                onNewCategoryName={setNewCategoryName}
                onCreateCategory={() => void handleCreateCategory()}
                onDeleteCategory={(category) => void handleDeleteCategory(category)}
              />
            ) : null}
          </div>
        </section>
      )}
    </main>
  );
}

function CategoryScreen(props: {
  library: LibraryState;
  onOpenCategory: (categoryId: number) => void;
  onOpenAnime: (anime: AnimeSummary) => void;
  onOpenSettings: () => void;
}) {
  const { library, onOpenCategory, onOpenAnime, onOpenSettings } = props;
  const animeByCategory = useMemo(() => {
    const counts = new Map<number, number>();
    for (const anime of library.anime) {
      counts.set(anime.category_id, (counts.get(anime.category_id) ?? 0) + 1);
    }
    return counts;
  }, [library.anime]);

  return (
    <>
      <ViewHeader
        title="Library"
        subtitle="Browse your local anime by category, or continue where you left off."
        action={
          <button type="button" onClick={onOpenSettings}>
            Settings
          </button>
        }
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

      {library.recent_anime.length > 0 ? (
        <section className="panel">
          <div className="panel-heading">
            <h2>Continue Watching</h2>
            <span className="muted">Last {library.recent_anime.length}</span>
          </div>
          <div className="continue-row">
            {library.recent_anime.map((anime) => (
              <button type="button" className="continue-card" key={anime.id} onClick={() => onOpenAnime(anime)}>
                <strong>{anime.title}</strong>
                <span>{anime.episode_count} episodes</span>
              </button>
            ))}
          </div>
        </section>
      ) : null}

      <section className="category-grid">
        {library.categories.map((category) => (
          <button
            type="button"
            className="category-card"
            key={category.id}
            onClick={() => onOpenCategory(category.id)}
          >
            <span className="category-name">{category.name}</span>
            <span className="category-count">{animeByCategory.get(category.id) ?? 0} anime</span>
          </button>
        ))}
      </section>
    </>
  );
}

function AnimeGrid(props: {
  category: Category | null;
  anime: AnimeSummary[];
  onOpenAnime: (anime: AnimeSummary) => void;
  onOpenSettings: () => void;
}) {
  const { category, anime, onOpenAnime, onOpenSettings } = props;

  return (
    <>
      <ViewHeader
        title={category?.name ?? "Anime"}
        subtitle={`${anime.length} title${anime.length === 1 ? "" : "s"} in this category.`}
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
              <div className="anime-card-title" title={item.title}>
                {item.title}
              </div>
              <div className="anime-card-meta">
                {item.episode_count} eps · {item.unwatched_count} unwatched
              </div>
              <div className="anime-tooltip">
                <strong>{item.title}</strong>
                <span>{item.episode_count} available episodes</span>
                <span>{item.unwatched_count} unwatched</span>
              </div>
            </button>
          ))}
        </div>
      )}
    </>
  );
}

function EpisodeScreen(props: {
  anime: AnimeSummary;
  episodes: Episode[];
  categories: Category[];
  onBack: () => void;
  onPlay: (episode: Episode) => void;
  onMoveAnime: (categoryId: number) => void;
}) {
  const { anime, episodes, categories, onBack, onPlay, onMoveAnime } = props;
  const latestEpisodeId = useMemo(() => {
    return episodes
      .filter((episode) => episode.last_watched_at)
      .sort((a, b) => String(b.last_watched_at).localeCompare(String(a.last_watched_at)))[0]?.id;
  }, [episodes]);

  return (
    <>
      <ViewHeader
        title={anime.title}
        subtitle={`${episodes.length} episode${episodes.length === 1 ? "" : "s"} · ${anime.unwatched_count} unwatched`}
        action={
          <button type="button" onClick={onBack}>
            Back to grid
          </button>
        }
      />

      <section className="panel episode-toolbar">
        <div>
          <span className="muted">Category</span>
          <select value={anime.category_id} onChange={(e) => onMoveAnime(Number(e.currentTarget.value))}>
            {categories.map((category) => (
              <option key={category.id} value={category.id}>
                {category.name}
              </option>
            ))}
          </select>
        </div>
        <div className="muted">
          Current progress is saved when you leave or switch episodes.
        </div>
      </section>

      <section className="episode-list">
        {episodes.map((episode) => {
          const percent = progressPercent(episode.position_seconds, episode.duration_seconds);
          return (
            <button
              type="button"
              key={episode.id}
              className={`episode-row${episode.watched ? " episode-row--watched" : ""}${episode.id === latestEpisodeId ? " episode-row--last" : ""}`}
              onClick={() => onPlay(episode)}
              title={episode.path}
            >
              <div className="episode-thumb">{episode.file_type.toUpperCase()}</div>
              <div className="episode-main">
                <div className="episode-title">
                  <span>{formatEpisodeNumber(episode.episode_number)}</span>
                  {episode.id === latestEpisodeId ? <span className="pill">Last played</span> : null}
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

function SettingsScreen(props: {
  library: LibraryState;
  busy: boolean;
  rootInput: string;
  newCategoryName: string;
  onRootInput: (value: string) => void;
  onPickFolder: () => void;
  onAddRoot: () => void;
  onRemoveRoot: (root: RootFolder) => void;
  onRescan: () => void;
  onNewCategoryName: (value: string) => void;
  onCreateCategory: () => void;
  onDeleteCategory: (category: Category) => void;
}) {
  const {
    library,
    busy,
    rootInput,
    newCategoryName,
    onRootInput,
    onPickFolder,
    onAddRoot,
    onRemoveRoot,
    onRescan,
    onNewCategoryName,
    onCreateCategory,
    onDeleteCategory,
  } = props;

  return (
    <>
      <ViewHeader title="Settings" subtitle={`Portable database: ${library.db_path}`} />

      <section className="panel">
        <div className="panel-heading">
          <h2>Root Folders</h2>
          <button type="button" onClick={onRescan} disabled={busy || library.root_folders.length === 0}>
            Rescan
          </button>
        </div>
        <form
          className="form-row"
          onSubmit={(e) => {
            e.preventDefault();
            onAddRoot();
          }}
        >
          <input
            type="text"
            value={rootInput}
            onChange={(e) => onRootInput(e.currentTarget.value)}
            placeholder="Paste a root folder path..."
            spellCheck={false}
          />
          <button type="button" onClick={onPickFolder} disabled={busy}>
            Browse
          </button>
          <button type="submit" disabled={busy}>
            Add
          </button>
        </form>
        <div className="settings-list">
          {library.root_folders.map((root) => (
            <div className="settings-item" key={root.id}>
              <span title={root.path}>{root.path}</span>
              <button type="button" onClick={() => onRemoveRoot(root)} disabled={busy}>
                Remove
              </button>
            </div>
          ))}
          {library.root_folders.length === 0 ? <p className="muted">No root folders configured.</p> : null}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h2>Categories</h2>
        </div>
        <form
          className="form-row"
          onSubmit={(e) => {
            e.preventDefault();
            onCreateCategory();
          }}
        >
          <input
            type="text"
            value={newCategoryName}
            onChange={(e) => onNewCategoryName(e.currentTarget.value)}
            placeholder="New category name..."
          />
          <button type="submit" disabled={busy || !newCategoryName.trim()}>
            Add
          </button>
        </form>
        <div className="settings-list">
          {library.categories.map((category) => (
            <div className="settings-item" key={category.id}>
              <span>
                {category.name} {category.is_default ? <span className="pill">Default</span> : null}
              </span>
              <button
                type="button"
                onClick={() => onDeleteCategory(category)}
                disabled={busy || category.is_default}
              >
                Delete
              </button>
            </div>
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h2>Detection Rules</h2>
          <span className="muted">{library.regex_rules.length} configured</span>
        </div>
        <div className="settings-list">
          {library.regex_rules.map((rule) => (
            <div className="settings-item settings-item--stacked" key={rule.id}>
              <strong>{rule.name}</strong>
              <code>{rule.detection_regex}</code>
              <code>{rule.title_regex}</code>
            </div>
          ))}
        </div>
      </section>
    </>
  );
}

function ViewHeader(props: { title: string; subtitle: string; action?: React.ReactNode }) {
  const { title, subtitle, action } = props;
  return (
    <header className="view-header">
      <div>
        <h1>{title}</h1>
        <p className="muted">{subtitle}</p>
      </div>
      {action ? <div className="view-actions">{action}</div> : null}
    </header>
  );
}

export default App;
