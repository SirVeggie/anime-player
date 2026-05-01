# Project context for agents

A running summary of what this project is, what's been built, and the key
design decisions. New agents should read this first to avoid re-deriving
context that was already settled.

## What the project is

A local **lossless anime video player** for Windows, built with **Tauri v2 +
React + TypeScript**. The user types or browses to a folder; the app
recursively lists all video files inside; clicking a file plays it via an
embedded `mpv.exe` instance.

The user is on Windows (PowerShell). Rust 1.95 (stable, MSVC toolchain),
Node 22, and `mpv.exe` on PATH are the runtime prerequisites. See
`README.md` for the full setup steps.

## Architecture

### Frontend — `src/App.tsx`, `src/App.css`

- Two-pane layout: 360px sidebar (folder input, file list, filter) +
  main player pane.
- Sidebar talks to Rust via `invoke("scan_videos", { folder })`.
- The player pane renders an empty `<div ref={playerHostRef} class="mpv-host" />`
  whose `getBoundingClientRect()` is reported to Rust. mpv renders into a
  native `WS_CHILD` HWND positioned over this div.
- The chain `html`/`body`/`#root`/`.app`/`.player`/`.mpv-host` is all
  `background: transparent`. The sidebar, now-playing strip, and empty
  state carry their own opaque backgrounds. Any opaque pixel inside
  `.mpv-host` would make WebView2's DComp surface paint over the mpv
  child, so leave it transparent.
- A `ResizeObserver` on the host (rAF-debounced) calls `mpv_set_rect`
  whenever the layout changes. `window.resize` is also listened to.
- mpv is initialized lazily on first file selection. After init, switching
  files just sends another `mpv_load`.

### Backend — `src-tauri/src/lib.rs`, `src-tauri/src/mpv.rs`

- `scan_videos(folder)` uses `walkdir` to recursively find files matching
  the extensions in `VIDEO_EXTENSIONS` (`mkv`, `mp4`, `mkv`, `avi`,
  `webm`, `ts`, `m2ts`, etc.). Returns `{ path, name, relative_path, size }[]`,
  sorted by relative path.
- `mpv.rs` owns the mpv subprocess and its host window. All Win32 code is
  gated behind `#[cfg(windows)]`. mpv is driven over its **JSON IPC
  named pipe** (`\\.\pipe\anime-player-mpv`) — currently send-only.
- Exposed Tauri commands: `scan_videos`, `mpv_init`, `mpv_set_rect`,
  `mpv_load`, `mpv_play_pause`, `mpv_stop`.
- No `Moved`/`Resized` window-event hook is needed: the mpv HWND is a
  child of the Tauri main HWND and follows it automatically. Only
  in-page layout changes (sidebar reflow, etc.) trigger `mpv_set_rect`.

### Tauri configuration — `src-tauri/tauri.conf.json`, `capabilities/default.json`

- Asset protocol is enabled with scope `["**"]` (legacy from when we used
  `<video>` + `convertFileSrc`; left in place but no longer used by mpv).
- Permissions: `core:default`, `opener:default`, `dialog:default`. The
  dialog plugin is used for the native folder picker.
- Window: `productName = "Anime Player"`, 1280x800 default, min 800x600,
  **`transparent: true`** — required for the mpv hosting strategy below.
- Cargo: the `tauri` crate is built with the `unstable` feature so the
  schema accepts `transparent: true`.

## Critical design decision: mpv hosting on Windows

mpv runs as a `WS_CHILD` of the Tauri main HWND, z-ordered beneath
WebView2's child HWND. The whole top-level window is in compositing
mode (`transparent: true`), so DWM/DComp alpha-blends the WebView2
surface against its sibling — and a transparent CSS region in the
player pane reveals the mpv child window underneath.

- Earlier attempts used `WS_CHILD` against an opaque top-level window;
  that failed (audio played, video pane was black) because in
  non-compositing mode WebView2's DComp surface always paints over GDI
  siblings regardless of Win32 z-order. Transparency is the missing
  ingredient.
- A previous iteration worked around it by hosting mpv in an owned
  top-level `WS_POPUP` and re-projecting the host div's client rect to
  screen coordinates on every move/resize. That worked but required two
  windows. The current single-window approach replaces it.
- Implementation:
  - `CreateWindowExW(0, "STATIC", ..., WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS, x, y, w, h, parent, ...)`
    with the Tauri main HWND as the parent and `x,y` in client
    coordinates (already in physical pixels via `scale_factor`).
  - `SetWindowPos(hwnd, HWND_BOTTOM, ..., SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)`
    to push the mpv child to the bottom of the z-order so WebView2
    composes on top of it.
  - `set_rect` calls `SetWindowPos` with `SWP_NOZORDER` so subsequent
    layout updates don't accidentally raise mpv above WebView2.
- mpv is launched with `--wid=<hwnd>` plus `--idle=yes
  --force-window=yes --keep-open=yes --osc=yes
  --input-default-bindings=yes --input-ipc-server=\\.\pipe\anime-player-mpv`.
  The IPC pipe accepts JSON one-liners like
  `{"command":["loadfile","C:\\path"]}`, `{"command":["cycle","pause"]}`.
- Full mpv keyboard bindings work inside the player area (Space pause,
  ←/→ seek, F fullscreen, J subs, # audio tracks). The on-screen
  controller appears on hover.

If video issues recur (black, misaligned, ghosting), the most likely
suspects are:

- Some opaque CSS leaked back into the chain `html`/`body`/`#root`/
  `.app`/`.player`/`.mpv-host`. Any opaque pixel there breaks the
  see-through and WebView2 paints over mpv again.
- The Tauri window lost `transparent: true` (or the `unstable` Cargo
  feature was dropped, so the schema rejected it silently).
- The mpv HWND was raised above WebView2 in z-order (look for missing
  `SWP_NOZORDER` in `set_rect`).
- mpv failing to launch (check the dev terminal for stderr).

## Version control

The repo is a git repository on branch `main` with no remote. Every
logical agent change is committed as its own checkpoint so the user
can roll back individual steps. The full convention (when to commit,
what to stage, message style, what *not* to do — no `--amend`, no
push, no rebase without an explicit request) lives in
`.cursor/rules/commit-checkpoints.mdc`.

## Files to know

- `README.md` — user-facing setup + run + project layout, including a
  "How playback works" section.
- `TODO.md` — running list of follow-ups (custom HTML controls, IPC
  event reader, bundling mpv.exe, DPI scale-factor change handling,
  resume playback).
- `CONTEXT.md` — this file.
- `.cursor/rules/` — agent rules. `read-context.mdc` points new agents
  here; `commit-checkpoints.mdc` defines the per-change commit
  workflow.

## Conventions

- Communication: PowerShell-friendly commands (the user's shell is
  PowerShell). Use `Get-ChildItem`/`Read`/etc., not `cat`/`ls`.
- Don't add boilerplate comments that just describe what the code does;
  comment trade-offs and constraints (see `mpv.rs` for the level of
  commentary that's expected).
- The user prefers iterative changes with verification — run
  `cargo check` and `npx tsc --noEmit` after backend/frontend edits.
- **Commit after every logical change** so each agent step is a
  checkpoint the user can roll back to. See
  `.cursor/rules/commit-checkpoints.mdc` for the full convention
  (one logical change per commit, imperative subject lines, no
  `--amend` / push / rebase without an explicit request).

## Build/test commands

```powershell
# Frontend type-check
npx tsc --noEmit

# Rust type-check (fast, doesn't link)
cargo check --manifest-path src-tauri/Cargo.toml

# Run the app (compiles Rust, launches Vite, opens the window)
npm run tauri dev

# Production build (installer in src-tauri/target/release/bundle/)
npm run tauri build
```

A full `cargo check` from cold is ~25s on the user's machine; warm is <2s.
