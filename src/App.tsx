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

/** Seek bar matching reference `SeekBar.vue` (custom track + thumb, pointer capture). */
function SeekBar(props: {
  duration: number;
  position: number;
  formatTime: (seconds: number) => string;
  onSeek: (seconds: number) => void;
}) {
  const { duration, position, formatTime, onSeek } = props;
  const areaRef = useRef<HTMLDivElement>(null);
  const durationRef = useRef(duration);
  const onSeekRef = useRef(onSeek);
  durationRef.current = duration;
  onSeekRef.current = onSeek;

  const [isDragging, setIsDragging] = useState(false);
  const isDraggingRef = useRef(false);
  const activePointerId = useRef<number | null>(null);
  const [dragRatio, setDragRatio] = useState<number | null>(null);
  const [showHoverTime, setShowHoverTime] = useState(false);
  const [hoverRatio, setHoverRatio] = useState(0);
  const [hoverTime, setHoverTime] = useState(0);
  const dragListenersCleanup = useRef<(() => void) | null>(null);

  const clampRatio = (v: number) => Math.min(1, Math.max(0, v));

  const getRatioFromClientX = (clientX: number) => {
    const container = areaRef.current;
    if (!container) return 0;
    const rect = container.getBoundingClientRect();
    if (rect.width <= 0) return 0;
    return clampRatio((clientX - rect.left) / rect.width);
  };

  const detachDragListeners = () => {
    dragListenersCleanup.current?.();
    dragListenersCleanup.current = null;
  };

  useEffect(() => {
    return () => {
      detachDragListeners();
    };
  }, []);

  useEffect(() => {
    const onKeyDown = () => {
      if (!isDraggingRef.current) setShowHoverTime(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const stopDragging = (event?: PointerEvent) => {
    if (event && areaRef.current?.hasPointerCapture(event.pointerId)) {
      areaRef.current.releasePointerCapture(event.pointerId);
    }
    activePointerId.current = null;
    isDraggingRef.current = false;
    setIsDragging(false);
    setDragRatio(null);
    detachDragListeners();
  };

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || duration <= 0) return;
    e.preventDefault();
    detachDragListeners();
    activePointerId.current = e.pointerId;
    isDraggingRef.current = true;
    setIsDragging(true);
    areaRef.current?.setPointerCapture(e.pointerId);
    const ratio = getRatioFromClientX(e.clientX);
    setDragRatio(ratio);
    setHoverRatio(ratio);
    setHoverTime(ratio * durationRef.current);
    setShowHoverTime(true);

    const onMove = (ev: PointerEvent) => {
      if (!isDraggingRef.current || ev.pointerId !== activePointerId.current) return;
      const r = getRatioFromClientX(ev.clientX);
      setDragRatio(r);
      setHoverRatio(r);
      setHoverTime(r * durationRef.current);
    };
    const onUp = (ev: PointerEvent) => {
      if (!isDraggingRef.current || ev.pointerId !== activePointerId.current) return;
      const r = getRatioFromClientX(ev.clientX);
      onSeekRef.current(r * durationRef.current);
      stopDragging(ev);
      setShowHoverTime(false);
    };
    const onCancel = (ev: PointerEvent) => {
      if (ev.pointerId !== activePointerId.current) return;
      stopDragging(ev);
      setShowHoverTime(false);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    dragListenersCleanup.current = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
    };
  };

  const updateHoverTime = (e: React.MouseEvent) => {
    if (isDraggingRef.current) return;
    if (duration <= 0) {
      setShowHoverTime(false);
      return;
    }
    const ratio = getRatioFromClientX(e.clientX);
    setHoverRatio(ratio);
    setHoverTime(ratio * duration);
    setShowHoverTime(true);
  };

  const hideHoverTime = () => {
    if (isDraggingRef.current) return;
    setShowHoverTime(false);
  };

  const progressPercent = duration > 0 ? (position / duration) * 100 : 0;
  const displayProgressPercent =
    isDragging && dragRatio !== null ? dragRatio * 100 : progressPercent;

  return (
    <div
      ref={areaRef}
      className={`progress-area${isDragging ? " is-dragging" : ""}`}
      role="slider"
      aria-label="Seek"
      aria-valuemin={0}
      aria-valuemax={Math.max(duration, 0)}
      aria-valuenow={Math.min(position, duration)}
      onPointerDown={onPointerDown}
      onMouseMove={updateHoverTime}
      onMouseLeave={hideHoverTime}
    >
      {showHoverTime ? (
        <div className="time-tooltip" style={{ left: `${hoverRatio * 100}%` }}>
          {formatTime(hoverTime)}
        </div>
      ) : null}
      <div className="progress-bg">
        <div className="progress-current" style={{ width: `${displayProgressPercent}%` }} />
      </div>
      <div
        className="scrubber-head"
        style={{ left: `${displayProgressPercent}%` }}
      />
    </div>
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

  const onSeekCommit = useCallback((seconds: number) => {
    setPosition(seconds);
    void invoke("mpv_seek", { seconds }).catch((e) =>
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

  const onCloseVideo = useCallback(() => {
    void (async () => {
      try {
        await invoke("mpv_stop");
      } catch (e) {
        setError(typeof e === "string" ? e : String(e));
      } finally {
        setSelected(null);
        setPosition(0);
        setDuration(0);
        setPaused(true);
      }
    })();
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
      if (e.target instanceof HTMLElement && e.target.closest("aside.sidebar")) {
        return;
      }

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
        return;
      }
      if (e.code === "KeyF") {
        e.preventDefault();
        void toggleFullscreen();
        return;
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [selected, toggleFullscreen]);

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

      <section className={selected ? "player player--playback" : "player"}>
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
            <div className="player-controls ui-surface">
              <div className="player-controls-content">
                <SeekBar
                  duration={safeDuration}
                  position={Math.min(position, safeDuration || position)}
                  formatTime={formatTime}
                  onSeek={onSeekCommit}
                />
                <div className="controls-main-viewport">
                  <div className="controls-main">
                    <div className="controls-left">
                      <button
                        type="button"
                        className={`icon-button icon-button--player icon-button--lg${canPrev ? "" : " icon-button--disabled"}`}
                        disabled={!canPrev}
                        onClick={() => loadSibling(-1)}
                        title="Previous"
                      >
                        <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                          <path d="M6 18V6h2v12H6zm3.5-6 8.5 6V6l-8.5 6z" />
                        </svg>
                      </button>
                      <button
                        type="button"
                        className="icon-button icon-button--player icon-button--lg"
                        onClick={onTogglePause}
                        title={paused ? "Play" : "Pause"}
                      >
                        {paused ? (
                          <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                            <path d="M8,5.14V19.14L19,12.14L8,5.14Z" />
                          </svg>
                        ) : (
                          <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                            <path d="M14,19H18V5H14M6,19H10V5H6V19Z" />
                          </svg>
                        )}
                      </button>
                      <button
                        type="button"
                        className={`icon-button icon-button--player icon-button--lg${canNext ? "" : " icon-button--disabled"}`}
                        disabled={!canNext}
                        onClick={() => loadSibling(1)}
                        title="Next"
                      >
                        <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                          <path d="M16 6v12h2V6h-2zm-1.5 6L6 18V6l8.5 6z" />
                        </svg>
                      </button>
                      <div className="time-display">
                        <span>{formatTime(Math.min(position, safeDuration || position))}</span>
                        <span className="separator">/</span>
                        <span>{formatTime(safeDuration)}</span>
                      </div>
                    </div>
                    <div className="controls-right">
                      <button
                        type="button"
                        className="icon-button icon-button--player icon-button--lg"
                        onClick={() => void toggleFullscreen()}
                        title={fullscreen ? "Exit fullscreen (F)" : "Fullscreen (F)"}
                      >
                        {fullscreen ? (
                          <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                            <path d="M5 16h3v3h2v-5H5v2zm3-8H5v2h5V5H8v3zm6 11h2v-3h3v-2h-5v5zm2-11V5h-2v5h5V8h-3z" />
                          </svg>
                        ) : (
                          <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                            <path d="M7 14H5v5h5v-2H7v-3zm-2-4h2V7h3V5H5v5zm12 7h-3v2h5v-5h-2v3zM14 5v2h3v3h2V5h-5z" />
                          </svg>
                        )}
                      </button>
                      <button
                        type="button"
                        className="icon-button icon-button--player icon-button--lg"
                        onClick={onCloseVideo}
                        title="Close video"
                        aria-label="Close video"
                      >
                        <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden>
                          <path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z" />
                        </svg>
                      </button>
                    </div>
                  </div>
                </div>
              </div>
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
