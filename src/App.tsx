import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import "./App.css";

type VideoFile = {
  path: string;
  name: string;
  relative_path: string;
  size: number;
};

const SIDEBAR_PX = 360;

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

function App() {
  const [folder, setFolder] = useState<string>("");
  const [files, setFiles] = useState<VideoFile[]>([]);
  const [selected, setSelected] = useState<VideoFile | null>(null);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");

  // Playback state observed from libmpv via mpv:// events.
  const [paused, setPaused] = useState(true);
  const [position, setPosition] = useState(0);
  const [duration, setDuration] = useState(0);
  // While the user drags the scrubber we want the thumb to follow the
  // pointer rather than mpv's reported time-pos (which lags by a frame
  // and would otherwise fight the drag).
  const [scrubbing, setScrubbing] = useState<number | null>(null);

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

  // Boot libmpv on first selection. Subsequent selections just send
  // another loadfile.
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

  // Keep mpv's video-margin-ratio-left in sync with the actual sidebar
  // fraction whenever the window resizes. The sidebar width itself is
  // fixed by CSS, but the *ratio* changes with window width.
  useEffect(() => {
    let frame = 0;
    const update = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (!mpvReadyRef.current) return;
        void invoke("mpv_set_layout", {
          windowWidth: window.innerWidth,
          sidebarPx: SIDEBAR_PX,
        });
      });
    };
    window.addEventListener("resize", update);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("resize", update);
    };
  }, []);

  // Wire libmpv property-change events into local React state.
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
            // EOF is reported as boolean; on false we leave state alone.
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

  const filtered = useMemo(() => {
    if (!filter.trim()) return files;
    const needle = filter.toLowerCase();
    return files.filter((f) => f.relative_path.toLowerCase().includes(needle));
  }, [files, filter]);

  const displayedPosition = scrubbing ?? position;
  const safeDuration = duration > 0 ? duration : 0;

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
            {/* The video shows through this transparent region: libmpv
                renders a DComp swap-chain under the Tauri main HWND
                and we leave the right pane's CSS fully transparent so
                that swap-chain composes through to the user. */}
            <div className="player-canvas" />
            <div className="now-playing" title={selected.path}>
              {selected.relative_path}
            </div>
            <div className="controls">
              <button
                type="button"
                className="play-toggle"
                onClick={onTogglePause}
                title={paused ? "Play" : "Pause"}
              >
                {paused ? "Play" : "Pause"}
              </button>
              <input
                className="scrubber"
                type="range"
                min={0}
                max={Math.max(safeDuration, 0.001)}
                step={0.05}
                value={Math.min(displayedPosition, safeDuration || displayedPosition)}
                onChange={(e) => onScrubChange(Number(e.currentTarget.value))}
                onMouseUp={(e) => onScrubCommit(Number(e.currentTarget.value))}
                onKeyUp={(e) => onScrubCommit(Number(e.currentTarget.value))}
              />
              <span className="time">
                {formatTime(displayedPosition)} / {formatTime(safeDuration)}
              </span>
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
