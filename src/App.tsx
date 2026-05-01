import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import "./App.css";

type VideoFile = {
  path: string;
  name: string;
  relative_path: string;
  size: number;
};

const SIDEBAR_PX = 360;
const appWindow = getCurrentWindow();

function formatSize(bytes: number): string {
  if (bytes <= 0) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 100 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const total = Math.floor(seconds);
  const s = total % 60;
  const m = Math.floor(total / 60) % 60;
  const h = Math.floor(total / 3600);
  const ss = s.toString().padStart(2, "0");
  if (h > 0) {
    const mm = m.toString().padStart(2, "0");
    return `${h}:${mm}:${ss}`;
  }
  return `${m}:${ss}`;
}

function isTextInputTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  return target.isContentEditable;
}

function IconSkipBack() {
  return (
    <svg className="control-glyph" viewBox="0 0 24 24" width="22" height="22" aria-hidden>
      <path
        fill="currentColor"
        d="M6 6h2v12H6V6zm3.5 6l8.5 6V6l-8.5 6z"
      />
    </svg>
  );
}

function IconSkipForward() {
  return (
    <svg className="control-glyph" viewBox="0 0 24 24" width="22" height="22" aria-hidden>
      <path
        fill="currentColor"
        d="M16 18h2V6h-2v12zM6 18l8.5-6L6 6v12z"
      />
    </svg>
  );
}

function IconPlay() {
  return (
    <svg className="control-glyph" viewBox="0 0 24 24" width="26" height="26" aria-hidden>
      <path fill="currentColor" d="M8 5v14l11-7z" />
    </svg>
  );
}

function IconPause() {
  return (
    <svg className="control-glyph" viewBox="0 0 24 24" width="26" height="26" aria-hidden>
      <path fill="currentColor" d="M6 19h4V5H6v14zm8-14v14h4V5h-4z" />
    </svg>
  );
}

function IconFullscreen() {
  return (
    <svg className="control-glyph" viewBox="0 0 24 24" width="20" height="20" aria-hidden>
      <path
        fill="currentColor"
        d="M7 14H5v5h5v-2H7v-3zm-2-4h2V7h3V5H5v5zm12 7h-3v2h5v-5h-2v3zM14 5v2h3v3h2V5h-5z"
      />
    </svg>
  );
}

function IconFullscreenExit() {
  return (
    <svg className="control-glyph" viewBox="0 0 24 24" width="20" height="20" aria-hidden>
      <path
        fill="currentColor"
        d="M5 16h3v3h2v-5H5v2zm3-8H5v2h5V5H8v3zm6 11h2v-3h3v-2h-5v5zm2-11V5h-2v5h5V8h-3z"
      />
    </svg>
  );
}

function App() {
  const [folder, setFolder] = useState<string>("");
  const [files, setFiles] = useState<VideoFile[]>([]);
  const [selected, setSelected] = useState<VideoFile | null>(null);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");

  const [paused, setPaused] = useState(true);
  const [position, setPosition] = useState(0);
  const [duration, setDuration] = useState(0);
  const [scrubbing, setScrubbing] = useState<number | null>(null);
  const [fullscreen, setFullscreen] = useState(false);

  const mpvReadyRef = useRef(false);

  async function handleScan(target?: string) {
    const path = (target ?? folder).trim();
    if (!path) {
      setError("Please enter or pick a folder path.");
      return;
    }
    setScanning(true);
    setError(null);
    try {
      const result = await invoke<VideoFile[]>("scan_videos", { folder: path });
      setFiles(result);
      if (result.length === 0) {
        setError("No video files found in this folder.");
      }
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      setFiles([]);
    } finally {
      setScanning(false);
    }
  }

  async function handlePickFolder() {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string" && picked) {
      setFolder(picked);
      void handleScan(picked);
    }
  }

  useEffect(() => {
    if (!selected) return;
    let cancelled = false;
    (async () => {
      try {
        if (!mpvReadyRef.current) {
          await invoke("mpv_init", {
            windowWidth: window.innerWidth,
            sidebarPx: SIDEBAR_PX,
          });
          mpvReadyRef.current = true;
        }
        if (cancelled) return;
        await invoke("mpv_load", { path: selected.path });
      } catch (e) {
        if (!cancelled) setError(typeof e === "string" ? e : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [selected]);

  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    let cancelled = false;

    (async () => {
      const subs: Array<[string, (e: { payload: unknown }) => void]> = [
        [
          "mpv://time-pos",
          (e) => {
            if (typeof e.payload === "number") setPosition(e.payload);
          },
        ],
        [
          "mpv://duration",
          (e) => {
            if (typeof e.payload === "number") setDuration(e.payload);
          },
        ],
        [
          "mpv://pause",
          (e) => {
            if (typeof e.payload === "boolean") setPaused(e.payload);
          },
        ],
        [
          "mpv://eof-reached",
          (e) => {
            if (e.payload === true) setPaused(true);
          },
        ],
      ];

      for (const [name, handler] of subs) {
        const fn = await listen(name, handler);
        if (cancelled) {
          fn();
          continue;
        }
        unlisteners.push(fn);
      }
    })();

    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
    };
  }, []);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    (async () => {
      try {
        setFullscreen(await appWindow.isFullscreen());
      } catch {
        /* ignore */
      }
      if (cancelled) return;
      unlisten = await appWindow.onResized(async () => {
        try {
          setFullscreen(await appWindow.isFullscreen());
        } catch {
          /* ignore */
        }
      });
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const onTogglePause = useCallback(() => {
    void invoke("mpv_cycle_pause").catch((e) =>
      setError(typeof e === "string" ? e : String(e))
    );
  }, []);

  const onScrubChange = useCallback((value: number) => {
    setScrubbing(value);
  }, []);

  const onScrubCommit = useCallback((value: number) => {
    setScrubbing(null);
    setPosition(value);
    void invoke("mpv_seek", { seconds: value }).catch((e) =>
      setError(typeof e === "string" ? e : String(e))
    );
  }, []);

  const toggleFullscreen = useCallback(async () => {
    try {
      const next = !(await appWindow.isFullscreen());
      await appWindow.setFullscreen(next);
      setFullscreen(next);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, []);

  const onCanvasMouseDown = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      if (e.button !== 0) return;
      if (e.detail === 2) {
        void toggleFullscreen();
        return;
      }
      void appWindow.startDragging();
    },
    [toggleFullscreen]
  );

  const onCanvasContextMenu = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      e.preventDefault();
      onTogglePause();
    },
    [onTogglePause]
  );

  useEffect(() => {
    if (!selected) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat) return;
      if (isTextInputTarget(e.target)) return;

      if (e.code === "Space") {
        e.preventDefault();
        void invoke("mpv_cycle_pause").catch((err) =>
          setError(typeof err === "string" ? err : String(err))
        );
        return;
      }
      if (e.code === "ArrowLeft") {
        e.preventDefault();
        void invoke("mpv_seek_relative", { delta: -5 }).catch((err) =>
          setError(typeof err === "string" ? err : String(err))
        );
        return;
      }
      if (e.code === "ArrowRight") {
        e.preventDefault();
        void invoke("mpv_seek_relative", { delta: 5 }).catch((err) =>
          setError(typeof err === "string" ? err : String(err))
        );
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [selected]);

  const filtered = useMemo(() => {
    if (!filter.trim()) return files;
    const needle = filter.toLowerCase();
    return files.filter((f) => f.relative_path.toLowerCase().includes(needle));
  }, [files, filter]);

  const selectedIndex = useMemo(
    () => (selected ? files.findIndex((f) => f.path === selected.path) : -1),
    [files, selected]
  );

  const loadSibling = useCallback(
    (delta: number) => {
      if (selectedIndex < 0 || files.length === 0) return;
      const j = selectedIndex + delta;
      if (j < 0 || j >= files.length) return;
      setSelected(files[j]);
    },
    [files, selectedIndex]
  );

  const displayedPosition = scrubbing ?? position;
  const safeDuration = duration > 0 ? duration : 0;
  const canPrev = selectedIndex > 0;
  const canNext = selectedIndex >= 0 && selectedIndex < files.length - 1;

  return (
    <main className="app">
      <aside className="sidebar">
        <header className="sidebar-header">
          <h1>Anime Player</h1>
          <p className="muted">Local lossless video library</p>
        </header>

        <form
          className="folder-row"
          onSubmit={(e) => {
            e.preventDefault();
            void handleScan();
          }}
        >
          <input
            type="text"
            value={folder}
            onChange={(e) => setFolder(e.currentTarget.value)}
            placeholder="Paste a folder path…"
            spellCheck={false}
          />
          <button type="button" onClick={handlePickFolder} title="Browse for folder">
            Browse
          </button>
          <button type="submit" disabled={scanning}>
            {scanning ? "Scanning…" : "Scan"}
          </button>
        </form>

        {files.length > 0 && (
          <input
            className="filter"
            type="text"
            value={filter}
            onChange={(e) => setFilter(e.currentTarget.value)}
            placeholder={`Filter ${files.length} file${files.length === 1 ? "" : "s"}…`}
          />
        )}

        {error && <div className="error">{error}</div>}

        <ul className="file-list">
          {filtered.map((file) => (
            <li
              key={file.path}
              className={selected?.path === file.path ? "active" : ""}
              onClick={() => setSelected(file)}
              title={file.path}
            >
              <span className="file-name">{file.relative_path}</span>
              <span className="file-size">{formatSize(file.size)}</span>
            </li>
          ))}
        </ul>
      </aside>

      <section className="player">
        {selected ? (
          <>
            <div
              className="player-canvas"
              onMouseDown={onCanvasMouseDown}
              onContextMenu={onCanvasContextMenu}
            />
            <div className="now-playing" title={selected.path}>
              {selected.relative_path}
            </div>
            <div className="controls">
              <div className="controls-cluster controls-cluster--transport">
                <button
                  type="button"
                  className="control-icon-btn"
                  disabled={!canPrev}
                  onClick={() => loadSibling(-1)}
                  title="Previous"
                >
                  <IconSkipBack />
                </button>
                <button
                  type="button"
                  className="control-icon-btn control-icon-btn--primary"
                  onClick={onTogglePause}
                  title={paused ? "Play" : "Pause"}
                >
                  {paused ? <IconPlay /> : <IconPause />}
                </button>
                <button
                  type="button"
                  className="control-icon-btn"
                  disabled={!canNext}
                  onClick={() => loadSibling(1)}
                  title="Next"
                >
                  <IconSkipForward />
                </button>
              </div>
              <div className="controls-scrub-wrap">
                <input
                  className="scrubber"
                  type="range"
                  min={0}
                  max={Math.max(safeDuration, 0.001)}
                  step={0.05}
                  value={Math.min(displayedPosition, safeDuration || displayedPosition)}
                  aria-label="Seek"
                  onInput={(e) => onScrubChange(Number(e.currentTarget.value))}
                  onChange={(e) => onScrubChange(Number(e.currentTarget.value))}
                  onPointerUp={(e) => onScrubCommit(Number(e.currentTarget.value))}
                  onPointerCancel={(e) => onScrubCommit(Number(e.currentTarget.value))}
                  onKeyUp={(e) => onScrubCommit(Number(e.currentTarget.value))}
                />
              </div>
              <span className="time">
                {formatTime(displayedPosition)} / {formatTime(safeDuration)}
              </span>
              <button
                type="button"
                className="control-icon-btn"
                onClick={() => void toggleFullscreen()}
                title={fullscreen ? "Exit fullscreen" : "Fullscreen"}
              >
                {fullscreen ? <IconFullscreenExit /> : <IconFullscreen />}
              </button>
            </div>
          </>
        ) : (
          <div className="empty">
            <h2>No video selected</h2>
            <p className="muted">
              {files.length === 0
                ? "Choose a folder to scan for video files."
                : "Pick a video from the list to start playing."}
            </p>
          </div>
        )}
      </section>
    </main>
  );
}

export default App;
