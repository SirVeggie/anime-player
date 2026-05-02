import { type MouseEvent as ReactMouseEvent, type ReactNode, useCallback, useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  addRootFolder,
  createCategory,
  createRegexRule,
  deleteCategory,
  deleteRegexRule,
  getFileThumbnail,
  getLibraryState,
  listEpisodes,
  moveAnimeToCategory,
  removeRootFolder,
  rescanLibrary,
  setDefaultCategory,
  updateRegexRule,
} from "./api";
import { PlayerView } from "./components/PlayerView";
import type {
  AnimeSummary,
  Category,
  Episode,
  LibraryState,
  RegexRule,
  RegexRuleInput,
  RootFolder,
} from "./types";
import {
  errorMessage,
  formatEpisodeNumber,
  formatSize,
  formatTime,
  isTextInputTarget,
  progressPercent,
} from "./utils";
import "./App.css";

type View = "categories" | "anime" | "episodes" | "settings" | "player";
type Toast = { id: number; kind: "success" | "error"; message: string };

const EMPTY_RULE: RegexRuleInput = {
  name: "",
  detection_regex: "",
  title_regex: "",
  enabled: true,
  priority: 0,
};

const VIDEO_OPEN_FADE_MS = 180;
const appWindow = getCurrentWindow();

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
  const [fatalError, setFatalError] = useState<string | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [videoOpening, setVideoOpening] = useState(false);
  const [playerControlsChromeVisible, setPlayerControlsChromeVisible] = useState(true);

  const showToast = useCallback((kind: Toast["kind"], message: string) => {
    const id = Date.now() + Math.random();
    setToasts((current) => [...current, { id, kind, message }]);
    window.setTimeout(() => {
      setToasts((current) => current.filter((toast) => toast.id !== id));
    }, 4200);
  }, []);

  const reloadLibrary = useCallback(async () => {
    const state = await getLibraryState();
    setLibrary(state);
    setSelectedCategoryId((current) => current ?? state.categories[0]?.id ?? null);
    return state;
  }, []);

  useEffect(() => {
    void (async () => {
      try {
        await reloadLibrary();
      } catch (e) {
        setFatalError(errorMessage(e));
      } finally {
        setLoading(false);
      }
    })();
  }, [reloadLibrary]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat) return;
      if (e.code !== "F11") return;
      if (isTextInputTarget(e.target)) return;
      e.preventDefault();
      void (async () => {
        try {
          const next = !(await appWindow.isFullscreen());
          await appWindow.setFullscreen(next);
        } catch {
          /* ignore */
        }
      })();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, []);

  useEffect(() => {
    if (!(view === "player" && selectedEpisode)) {
      setPlayerControlsChromeVisible(true);
    }
  }, [view, selectedEpisode]);

  const selectedCategory = useMemo(() => {
    if (!library || selectedCategoryId === null) return null;
    return library.categories.find((category) => category.id === selectedCategoryId) ?? null;
  }, [library, selectedCategoryId]);

  const animeInCategory = useMemo(() => {
    if (!library || selectedCategoryId === null) return [];
    return library.anime.filter((anime) => anime.category_id === selectedCategoryId);
  }, [library, selectedCategoryId]);

  const runAction = useCallback(
    async (action: () => Promise<string | void>) => {
      setBusy(true);
      try {
        const message = await action();
        if (message) showToast("success", message);
      } catch (e) {
        showToast("error", errorMessage(e));
      } finally {
        setBusy(false);
      }
    },
    [showToast],
  );

  const openAnime = useCallback(
    async (anime: AnimeSummary) => {
      setSelectedAnime(anime);
      try {
        const nextEpisodes = await listEpisodes(anime.id);
        setEpisodes(nextEpisodes);
        setView("episodes");
      } catch (e) {
        showToast("error", errorMessage(e));
      }
    },
    [showToast],
  );

  const handleAddRoot = useCallback(
    async (path: string) => {
      const trimmed = path.trim();
      if (!trimmed) {
        showToast("error", "Choose or paste a folder path first.");
        return;
      }
      await runAction(async () => {
        await addRootFolder(trimmed);
        setRootInput("");
        await reloadLibrary();
        return "Root folder added.";
      });
    },
    [reloadLibrary, runAction, showToast],
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
        return "Root folder removed. Existing library entries are preserved until rescanned.";
      });
    },
    [reloadLibrary, runAction],
  );

  const handleRescan = useCallback(async () => {
    await runAction(async () => {
      const summary = await rescanLibrary();
      await reloadLibrary();
      return `Scanned ${summary.roots_scanned} root folder${summary.roots_scanned === 1 ? "" : "s"}: ${summary.episodes_imported} episode${summary.episodes_imported === 1 ? "" : "s"} imported, ${summary.unmatched_files} unmatched.`;
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
      return "Category created.";
    });
  }, [newCategoryName, reloadLibrary, runAction]);

  const handleDeleteCategory = useCallback(
    async (category: Category) => {
      await runAction(async () => {
        await deleteCategory(category.id);
        const state = await reloadLibrary();
        setSelectedCategoryId(state.categories[0]?.id ?? null);
        return "Category deleted. Anime were moved to the default category.";
      });
    },
    [reloadLibrary, runAction],
  );

  const handleSetDefaultCategory = useCallback(
    async (category: Category) => {
      await runAction(async () => {
        await setDefaultCategory(category.id);
        await reloadLibrary();
        return `"${category.name}" is now the default category.`;
      });
    },
    [reloadLibrary, runAction],
  );

  const handleCreateRule = useCallback(
    async (input: RegexRuleInput) => {
      await runAction(async () => {
        await createRegexRule(input);
        await reloadLibrary();
        return "Detection rule added.";
      });
    },
    [reloadLibrary, runAction],
  );

  const handleUpdateRule = useCallback(
    async (id: number, input: RegexRuleInput) => {
      await runAction(async () => {
        await updateRegexRule(id, input);
        await reloadLibrary();
        return "Detection rule updated.";
      });
    },
    [reloadLibrary, runAction],
  );

  const handleDeleteRule = useCallback(
    async (rule: RegexRule) => {
      await runAction(async () => {
        await deleteRegexRule(rule.id);
        await reloadLibrary();
        return "Detection rule deleted.";
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
        return "Anime moved.";
      });
    },
    [reloadLibrary, runAction, selectedAnime],
  );

  const handleProgressSaved = useCallback(
    (saved: Episode) => {
      setEpisodes((current) => {
        const next = current.map((episode) => (episode.id === saved.id ? saved : episode));
        setSelectedAnime((anime) =>
          anime && anime.id === saved.anime_id
            ? { ...anime, unwatched_count: next.filter((episode) => !episode.watched).length }
            : anime,
        );
        return next;
      });
      void reloadLibrary().catch((e) => showToast("error", errorMessage(e)));
    },
    [reloadLibrary, showToast],
  );

  const navigateToCategory = useCallback((categoryId: number) => {
    setSelectedCategoryId(categoryId);
    setSelectedAnime(null);
    setSelectedEpisode(null);
    setEpisodes([]);
    setView("anime");
  }, []);

  const openEpisode = useCallback((episode: Episode) => {
    setVideoOpening(true);
    window.setTimeout(() => {
      setSelectedEpisode(episode);
      setView("player");
      setVideoOpening(false);
    }, VIDEO_OPEN_FADE_MS);
  }, []);

  // Q on the episodes screen jumps into the current anime's last-played episode
  // (or the next one if that episode is already watched). Scoped to the
  // episodes view so PlayerView keeps owning Q while playback is visible.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat || e.code !== "KeyQ") return;
      if (isTextInputTarget(e.target)) return;
      if (view !== "episodes" || !selectedAnime) return;
      const target = pickQuickPlayEpisode(episodes);
      if (!target) return;
      e.preventDefault();
      if (selectedEpisode && selectedEpisode.id === target.id) {
        // Same file already loaded in the hidden player; reveal it without
        // the open-fade. PlayerView's `visible` effect handles the unpause.
        setView("player");
      } else {
        openEpisode(target);
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [episodes, openEpisode, selectedAnime, selectedEpisode, view]);

  if (loading) {
    return (
      <main className="app app--loading">
        <WindowTitleBar playerOpen={false} playerControlsChromeVisible />
        <div className="empty">
          <h2>Loading library...</h2>
        </div>
      </main>
    );
  }

  if (!library) {
    return (
      <main className="app app--loading">
        <WindowTitleBar playerOpen={false} playerControlsChromeVisible />
        <div className="empty">
          <h2>Library failed to load</h2>
          {fatalError ? <p className="muted">{fatalError}</p> : null}
        </div>
      </main>
    );
  }

  const showPlayer = view === "player" && selectedEpisode;
  const playerLoadedInBackground = selectedEpisode && !showPlayer;
  const libraryViewActive = view === "categories" || view === "anime" || view === "episodes";

  return (
    <main
      className={`app${showPlayer ? " app--player-open" : ""}${
        playerLoadedInBackground ? " app--player-background" : ""
      }${videoOpening ? " app--video-opening" : ""}`}
    >
      <WindowTitleBar
        playerOpen={Boolean(showPlayer)}
        playerControlsChromeVisible={playerControlsChromeVisible}
      />
      <aside className="sidebar">
        <nav className="nav-list" aria-label="Primary navigation">
          <button
            type="button"
            className={libraryViewActive ? "nav-item active" : "nav-item"}
            onClick={() => setView("categories")}
            aria-label="Library"
            title="Library"
          >
            <LibraryIcon />
            <span className="nav-label">Library</span>
          </button>
          <button
            type="button"
            className={view === "settings" ? "nav-item active" : "nav-item"}
            onClick={() => setView("settings")}
            aria-label="Settings"
            title="Settings"
          >
            <SettingsIcon />
            <span className="nav-label">Settings</span>
          </button>
          <button
            type="button"
            className="nav-item nav-item--action"
            onClick={() => void handleRescan()}
            disabled={busy}
            aria-label={busy ? "Rescanning library" : "Rescan library"}
            title={busy ? "Rescanning library" : "Rescan library"}
          >
            <RescanIcon />
            <span className="nav-label">{busy ? "Working..." : "Rescan library"}</span>
          </button>
        </nav>
      </aside>

      {selectedEpisode ? (
        <PlayerView
          episode={selectedEpisode}
          playlist={episodes}
          visible={Boolean(showPlayer)}
          onSelectEpisode={setSelectedEpisode}
          onBack={() => setView("episodes")}
          onClose={() => {
            setSelectedEpisode(null);
            setView("episodes");
          }}
          onProgressSaved={handleProgressSaved}
          onError={(message) => showToast("error", message)}
          onControlsVisibilityChange={setPlayerControlsChromeVisible}
        />
      ) : null}

      <section className="content">
        <div className="content-inner">
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
              onBack={() => setView("categories")}
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
              onPlay={openEpisode}
              onMoveAnime={(categoryId) => void handleMoveAnime(categoryId)}
            />
          ) : null}

          {view === "settings" ? (
            <SettingsScreen
              library={library}
              busy={busy}
              rootInput={rootInput}
              newCategoryName={newCategoryName}
              onBack={() => setView("categories")}
              onRootInput={setRootInput}
              onPickFolder={() => void handlePickFolder()}
              onAddRoot={() => void handleAddRoot(rootInput)}
              onRemoveRoot={(root) => void handleRemoveRoot(root)}
              onRescan={() => void handleRescan()}
              onNewCategoryName={setNewCategoryName}
              onCreateCategory={() => void handleCreateCategory()}
              onDeleteCategory={(category) => void handleDeleteCategory(category)}
              onSetDefaultCategory={(category) => void handleSetDefaultCategory(category)}
              onCreateRule={(input) => void handleCreateRule(input)}
              onUpdateRule={(id, input) => void handleUpdateRule(id, input)}
              onDeleteRule={(rule) => void handleDeleteRule(rule)}
            />
          ) : null}
        </div>
      </section>

      {videoOpening ? <div className="video-open-overlay" /> : null}
      <ToastStack toasts={toasts} onDismiss={(id) => setToasts((current) => current.filter((toast) => toast.id !== id))} />
    </main>
  );
}

function WindowTitleBar(props: { playerOpen: boolean; playerControlsChromeVisible: boolean }) {
  const { playerOpen, playerControlsChromeVisible } = props;
  const [maximized, setMaximized] = useState(false);

  const refreshMaximized = useCallback(async () => {
    try {
      setMaximized(await appWindow.isMaximized());
    } catch {
      /* The title bar should never block the rest of the UI. */
    }
  }, []);

  useEffect(() => {
    void refreshMaximized();
  }, [refreshMaximized]);

  const toggleMaximized = useCallback(async () => {
    try {
      await appWindow.toggleMaximize();
      await refreshMaximized();
    } catch {
      /* ignore */
    }
  }, [refreshMaximized]);

  const startDragOrMaximize = useCallback(
    (e: ReactMouseEvent<HTMLElement>) => {
      if (e.button !== 0) return;
      if (e.detail === 2) {
        e.preventDefault();
        void toggleMaximized();
        return;
      }
      void appWindow.startDragging().catch(() => undefined);
    },
    [toggleMaximized],
  );

  return (
    <header
      className={`window-titlebar${playerOpen ? " window-titlebar--player" : ""}${
        playerOpen && playerControlsChromeVisible ? " window-titlebar--player-visible" : ""
      }`}
      onMouseDown={startDragOrMaximize}
    >
      <div className="window-titlebar-title">Anime Player</div>
      <div className="window-controls" onMouseDown={(e) => e.stopPropagation()}>
        <button type="button" className="window-control" onClick={() => void appWindow.minimize()} aria-label="Minimize">
          <svg viewBox="0 0 12 12" aria-hidden>
            <path d="M2 6.5h8v1H2z" />
          </svg>
        </button>
        <button type="button" className="window-control" onClick={() => void toggleMaximized()} aria-label={maximized ? "Restore" : "Maximize"}>
          {maximized ? (
            <svg viewBox="0 0 12 12" aria-hidden>
              <path d="M3 2h7v7H8V8h1V3H4v1H3V2zm-1 3h6v5H2V5zm1 1v3h4V6H3z" />
            </svg>
          ) : (
            <svg viewBox="0 0 12 12" aria-hidden>
              <path d="M2 2h8v8H2V2zm1 1v6h6V3H3z" />
            </svg>
          )}
        </button>
        <button type="button" className="window-control window-control--close" onClick={() => void appWindow.close()} aria-label="Close">
          <svg viewBox="0 0 12 12" aria-hidden>
            <path d="m3.1 2.4 2.9 2.9 2.9-2.9.7.7L6.7 6l2.9 2.9-.7.7L6 6.7 3.1 9.6l-.7-.7L5.3 6 2.4 3.1l.7-.7z" />
          </svg>
        </button>
      </div>
    </header>
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

      {library.recent_anime.length > 0 ? (
        <>
          <div className="section-heading">
            <h2>Continue Watching</h2>
          </div>
          <div className="continue-grid">
            {library.recent_anime.map((anime) => (
              <button type="button" className="continue-card" key={anime.id} onClick={() => onOpenAnime(anime)}>
                <strong>{anime.title}</strong>
                <span>{anime.episode_count} episodes</span>
              </button>
            ))}
          </div>
        </>
      ) : null}
    </>
  );
}

function AnimeGrid(props: {
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

function EpisodeScreen(props: {
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

function SettingsScreen(props: {
  library: LibraryState;
  busy: boolean;
  rootInput: string;
  newCategoryName: string;
  onBack: () => void;
  onRootInput: (value: string) => void;
  onPickFolder: () => void;
  onAddRoot: () => void;
  onRemoveRoot: (root: RootFolder) => void;
  onRescan: () => void;
  onNewCategoryName: (value: string) => void;
  onCreateCategory: () => void;
  onDeleteCategory: (category: Category) => void;
  onSetDefaultCategory: (category: Category) => void;
  onCreateRule: (input: RegexRuleInput) => void;
  onUpdateRule: (id: number, input: RegexRuleInput) => void;
  onDeleteRule: (rule: RegexRule) => void;
}) {
  const {
    library,
    busy,
    rootInput,
    newCategoryName,
    onBack,
    onRootInput,
    onPickFolder,
    onAddRoot,
    onRemoveRoot,
    onRescan,
    onNewCategoryName,
    onCreateCategory,
    onDeleteCategory,
    onSetDefaultCategory,
    onCreateRule,
    onUpdateRule,
    onDeleteRule,
  } = props;

  return (
    <>
      <ViewHeader title="Settings" subtitle={`Portable database: ${library.db_path}`} onBack={onBack} />

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
              <div className="settings-actions">
                <button
                  type="button"
                  onClick={() => onSetDefaultCategory(category)}
                  disabled={busy || category.is_default}
                >
                  Make default
                </button>
                <button
                  type="button"
                  onClick={() => onDeleteCategory(category)}
                  disabled={busy || category.is_default}
                >
                  Delete
                </button>
              </div>
            </div>
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h2>Detection Rules</h2>
          <span className="muted">{library.regex_rules.length} configured</span>
        </div>
        <RuleEditor
          title="Add rule"
          busy={busy}
          initial={EMPTY_RULE}
          submitLabel="Add rule"
          onSubmit={onCreateRule}
        />
        <div className="settings-list">
          {library.regex_rules.map((rule) => (
            <RuleEditor
              key={rule.id}
              title={rule.name}
              busy={busy}
              initial={ruleToInput(rule)}
              submitLabel="Save"
              onSubmit={(input) => onUpdateRule(rule.id, input)}
              onDelete={() => onDeleteRule(rule)}
            />
          ))}
        </div>
      </section>
    </>
  );
}

function RuleEditor(props: {
  title: string;
  busy: boolean;
  initial: RegexRuleInput;
  submitLabel: string;
  onSubmit: (input: RegexRuleInput) => void;
  onDelete?: () => void;
}) {
  const { title, busy, initial, submitLabel, onSubmit, onDelete } = props;
  const [draft, setDraft] = useState(initial);

  useEffect(() => {
    setDraft(initial);
  }, [initial]);

  return (
    <form
      className="rule-editor"
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit(draft);
      }}
    >
      <div className="rule-editor-heading">
        <strong>{title}</strong>
        <label>
          <input
            type="checkbox"
            checked={draft.enabled}
            onChange={(e) => {
              // Read the event field synchronously: React's functional updater
              // can run twice under StrictMode, and by the second pass `e.currentTarget`
              // has been nulled, throwing inside RuleEditor and unmounting the whole tree.
              const enabled = e.currentTarget.checked;
              setDraft((current) => ({ ...current, enabled }));
            }}
          />
          Enabled
        </label>
      </div>
      <div className="form-grid">
        <label>
          <span>Name</span>
          <input
            type="text"
            value={draft.name}
            onChange={(e) => {
              const name = e.currentTarget.value;
              setDraft((current) => ({ ...current, name }));
            }}
          />
        </label>
        <label>
          <span>Priority</span>
          <input
            type="number"
            value={draft.priority}
            onChange={(e) => {
              const priority = Number(e.currentTarget.value) || 0;
              setDraft((current) => ({ ...current, priority }));
            }}
          />
        </label>
      </div>
      <label className="stacked-field">
        <span>Detection regex</span>
        <textarea
          value={draft.detection_regex}
          onChange={(e) => {
            const detection_regex = e.currentTarget.value;
            setDraft((current) => ({ ...current, detection_regex }));
          }}
          rows={2}
        />
      </label>
      <label className="stacked-field">
        <span>Title regex</span>
        <textarea
          value={draft.title_regex}
          onChange={(e) => {
            const title_regex = e.currentTarget.value;
            setDraft((current) => ({ ...current, title_regex }));
          }}
          rows={2}
        />
      </label>
      <div className="settings-actions">
        <button type="submit" disabled={busy}>
          {submitLabel}
        </button>
        {onDelete ? (
          <button type="button" onClick={onDelete} disabled={busy}>
            Delete
          </button>
        ) : null}
      </div>
    </form>
  );
}

function CustomDropdown(props: {
  label: string;
  value: number;
  options: Array<{ value: number; label: string }>;
  onChange: (value: number) => void;
}) {
  const { label, value, options, onChange } = props;
  const [open, setOpen] = useState(false);

  return (
    <div className={`custom-select${open ? " custom-select--open" : ""}`}>
      <button type="button" className="custom-select-trigger" onClick={() => setOpen((current) => !current)}>
        <span>{label}</span>
        <span className="chevron" aria-hidden>
          ▾
        </span>
      </button>
      {open ? (
        <div className="custom-select-menu">
          {options.map((option) => (
            <button
              type="button"
              key={option.value}
              className={option.value === value ? "custom-select-option active" : "custom-select-option"}
              onClick={() => {
                onChange(option.value);
                setOpen(false);
              }}
            >
              {option.label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function ViewHeader(props: {
  title: string;
  subtitle: string;
  action?: ReactNode;
  onBack?: () => void;
}) {
  const { title, subtitle, action, onBack } = props;
  return (
    <header className="view-header">
      <div className="view-title-row">
        {onBack ? (
          <button type="button" className="back-button" onClick={onBack} aria-label="Back">
            <ArrowLeftIcon />
          </button>
        ) : null}
        <div>
          <h1>{title}</h1>
          <p className="muted">{subtitle}</p>
        </div>
      </div>
      {action ? <div className="view-actions">{action}</div> : null}
    </header>
  );
}

function ToastStack(props: { toasts: Toast[]; onDismiss: (id: number) => void }) {
  const { toasts, onDismiss } = props;
  return (
    <div className="toast-stack" aria-live="polite" aria-atomic="true">
      {toasts.map((toast) => (
        <button
          type="button"
          key={toast.id}
          className={`toast toast--${toast.kind}`}
          onClick={() => onDismiss(toast.id)}
        >
          {toast.message}
        </button>
      ))}
    </div>
  );
}

function ArrowLeftIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <path d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.42-1.41L7.83 13H20v-2z" />
    </svg>
  );
}

function LibraryIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <path d="M4 5.5A2.5 2.5 0 0 1 6.5 3h11A2.5 2.5 0 0 1 20 5.5v13A2.5 2.5 0 0 1 17.5 21h-11A2.5 2.5 0 0 1 4 18.5v-13zm2.5-.75a.75.75 0 0 0-.75.75v13c0 .41.34.75.75.75h11c.41 0 .75-.34.75-.75v-13a.75.75 0 0 0-.75-.75h-11zM8 7h8v1.5H8V7zm0 3.5h8V12H8v-1.5zm0 3.5h5v1.5H8V14z" />
    </svg>
  );
}

function SettingsIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <path d="M19.43 12.98c.04-.32.07-.65.07-.98s-.02-.66-.07-.98l2.1-1.64a.5.5 0 0 0 .12-.64l-2-3.46a.5.5 0 0 0-.6-.22l-2.47 1a7.42 7.42 0 0 0-1.7-.98L14.5 2.45A.5.5 0 0 0 14 2h-4a.5.5 0 0 0-.5.45L9.12 5.08c-.62.24-1.19.56-1.7.98l-2.47-1a.5.5 0 0 0-.6.22l-2 3.46a.5.5 0 0 0 .12.64l2.1 1.64c-.04.32-.07.65-.07.98s.02.66.07.98l-2.1 1.64a.5.5 0 0 0-.12.64l2 3.46c.13.22.39.31.62.22l2.45-1c.51.4 1.08.73 1.7.98l.38 2.63c.04.25.25.45.5.45h4c.25 0 .46-.2.5-.45l.38-2.63c.62-.24 1.19-.56 1.7-.98l2.45 1c.23.09.49 0 .62-.22l2-3.46a.5.5 0 0 0-.12-.64l-2.1-1.64zM12 15.5A3.5 3.5 0 1 1 12 8a3.5 3.5 0 0 1 0 7.5z" />
    </svg>
  );
}

function RescanIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <path d="M17.65 6.35A7.95 7.95 0 0 0 12 4a8 8 0 1 0 7.75 10h-2.1A6 6 0 1 1 12 6c1.66 0 3.14.69 4.22 1.78L13 11h8V3l-3.35 3.35z" />
    </svg>
  );
}

/**
 * Pick the episode to launch when the user presses Q on the episodes screen:
 * - With no playback history, the first episode (so a fresh anime starts
 *   with one Q press).
 * - Otherwise, the most recently played episode — unless it is already
 *   watched, in which case the next episode in list order.
 * - Returns null only when there is nothing to play (empty list, or the
 *   watched candidate is the last episode and there is no successor).
 *
 * `episodes` is assumed to come from `list_episodes`, which orders by
 * episode_number then relative_path — so array index reflects in-anime order.
 */
function pickQuickPlayEpisode(episodes: Episode[]): Episode | null {
  if (episodes.length === 0) return null;
  let lastIdx = -1;
  let lastTimestamp = "";
  for (let i = 0; i < episodes.length; i += 1) {
    const ts = episodes[i].last_watched_at;
    // SQLite CURRENT_TIMESTAMP is "YYYY-MM-DD HH:MM:SS" which sorts
    // lexicographically, so string comparison is correct.
    if (ts && ts > lastTimestamp) {
      lastTimestamp = ts;
      lastIdx = i;
    }
  }
  if (lastIdx === -1) return episodes[0];
  const last = episodes[lastIdx];
  if (!last.watched) return last;
  return episodes[lastIdx + 1] ?? null;
}

function ruleToInput(rule: RegexRule): RegexRuleInput {
  return {
    name: rule.name,
    detection_regex: rule.detection_regex,
    title_regex: rule.title_regex,
    enabled: rule.enabled,
    priority: rule.priority,
  };
}

export default App;
