import { type ReactNode, useCallback, useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  addRootFolder,
  createCategory,
  createRegexRule,
  deleteCategory,
  deleteRegexRule,
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
import { errorMessage, formatEpisodeNumber, formatSize, formatTime, progressPercent } from "./utils";
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
          {fatalError ? <p className="muted">{fatalError}</p> : null}
        </div>
      </main>
    );
  }

  const showPlayer = view === "player" && selectedEpisode;

  return (
    <main
      className={`app${showPlayer ? " app--player-open" : ""}${videoOpening ? " app--video-opening" : ""}`}
    >
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
          onError={(message) => showToast("error", message)}
        />
      ) : (
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
      )}

      {videoOpening ? <div className="video-open-overlay" /> : null}
      <ToastStack toasts={toasts} onDismiss={(id) => setToasts((current) => current.filter((toast) => toast.id !== id))} />
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
              <div className="anime-card-title" title={item.title}>
                {item.title}
              </div>
              <div className="anime-card-meta">
                {item.episode_count} eps - {item.unwatched_count} unwatched
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
  const unwatchedCount = episodes.filter((episode) => !episode.watched).length;
  const selectedCategory = categories.find((category) => category.id === anime.category_id);

  return (
    <>
      <ViewHeader
        title={anime.title}
        subtitle={`${episodes.length} episode${episodes.length === 1 ? "" : "s"} - ${unwatchedCount} unwatched`}
        onBack={onBack}
      />

      <section className="panel episode-toolbar">
        <div className="toolbar-field">
          <span className="muted">Category</span>
          <CustomDropdown
            label={selectedCategory?.name ?? "Select category"}
            options={categories.map((category) => ({ value: category.id, label: category.name }))}
            value={anime.category_id}
            onChange={onMoveAnime}
          />
        </div>
        <div className="muted">Current progress is saved when you leave or switch episodes.</div>
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
            onChange={(e) => setDraft((current) => ({ ...current, enabled: e.currentTarget.checked }))}
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
            onChange={(e) => setDraft((current) => ({ ...current, name: e.currentTarget.value }))}
          />
        </label>
        <label>
          <span>Priority</span>
          <input
            type="number"
            value={draft.priority}
            onChange={(e) =>
              setDraft((current) => ({ ...current, priority: Number(e.currentTarget.value) || 0 }))
            }
          />
        </label>
      </div>
      <label className="stacked-field">
        <span>Detection regex</span>
        <textarea
          value={draft.detection_regex}
          onChange={(e) => setDraft((current) => ({ ...current, detection_regex: e.currentTarget.value }))}
          rows={2}
        />
      </label>
      <label className="stacked-field">
        <span>Title regex</span>
        <textarea
          value={draft.title_regex}
          onChange={(e) => setDraft((current) => ({ ...current, title_regex: e.currentTarget.value }))}
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
    <div className="custom-select">
      <button type="button" className="custom-select-trigger" onClick={() => setOpen((current) => !current)}>
        <span>{label}</span>
        <span aria-hidden>v</span>
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
