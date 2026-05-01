import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import "./App.css";

type VideoFile = {
  path: string;
  name: string;
  relative_path: string;
  size: number;
};

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

function App() {
  const [folder, setFolder] = useState<string>("");
  const [files, setFiles] = useState<VideoFile[]>([]);
  const [selected, setSelected] = useState<VideoFile | null>(null);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");

  const playerHostRef = useRef<HTMLDivElement | null>(null);
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

  // Spin up mpv (and load the chosen file) when a file is selected. This
  // runs again whenever the selection changes; mpv stays alive across
  // selections and we just send another `loadfile`.
  useEffect(() => {
    if (!selected) return;
    const host = playerHostRef.current;
    if (!host) return;
    let cancelled = false;

    (async () => {
      try {
        const r = host.getBoundingClientRect();
        if (!mpvReadyRef.current) {
          await invoke("mpv_init", {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
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

  // Keep the embedded mpv HWND glued to the host div when the layout
  // changes (window resize, sidebar reflow, etc.).
  useEffect(() => {
    const host = playerHostRef.current;
    if (!host) return;

    let frame = 0;
    const updateRect = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (!mpvReadyRef.current) return;
        const r = host.getBoundingClientRect();
        void invoke("mpv_set_rect", {
          x: r.x,
          y: r.y,
          width: r.width,
          height: r.height,
        });
      });
    };

    const ro = new ResizeObserver(updateRect);
    ro.observe(host);
    window.addEventListener("resize", updateRect);

    return () => {
      cancelAnimationFrame(frame);
      ro.disconnect();
      window.removeEventListener("resize", updateRect);
    };
  }, [selected]);

  const filtered = useMemo(() => {
    if (!filter.trim()) return files;
    const needle = filter.toLowerCase();
    return files.filter((f) => f.relative_path.toLowerCase().includes(needle));
  }, [files, filter]);

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
            {/* The mpv child HWND will be positioned over this div.
                We keep it empty; mpv draws everything itself. */}
            <div ref={playerHostRef} className="mpv-host" />
            <div className="now-playing" title={selected.path}>
              {selected.relative_path}
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
