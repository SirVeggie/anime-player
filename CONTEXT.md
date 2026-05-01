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
  separate native window that's positioned over this div.
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
- `lib.rs` hooks the main window's `Moved` and `Resized` events to call
  `Mpv::refresh_position`, so the popup stays glued to the host div when
  the user drags or resizes the main window.

### Tauri configuration — `src-tauri/tauri.conf.json`, `capabilities/default.json`

- Asset protocol is enabled with scope `["**"]` (legacy from when we used
  `<video>` + `convertFileSrc`; left in place but no longer used by mpv).
- Permissions: `core:default`, `opener:default`, `dialog:default`. The
  dialog plugin is used for the native folder picker.
- Window: `productName = "Anime Player"`, 1280x800 default, min 800x600.

## Critical design decision: mpv hosting on Windows

The non-obvious part of the project. **mpv must be hosted in an owned
top-level popup window, not a `WS_CHILD` of the Tauri HWND.**

- WebView2 renders its content via **DirectComposition**. A composited
  surface always paints on top of regular GDI child windows in the same
  parent regardless of Win32 z-order. We tried `WS_CHILD` first and the
  symptom was: audio plays, mpv.exe is alive, video pane is black.
- Fix: create a top-level window with `WS_POPUP | WS_VISIBLE`, ex-style
  `WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`, with the Tauri main HWND as the
  **owner** (passed as the `hWndParent` argument; without `WS_CHILD` the
  Win32 API treats it as the owner). Top-level windows compose at the
  desktop level, so they sit above the WebView.
- Trade-off: owned popups don't auto-follow when the owner moves, so
  `Mpv` caches the last client-area rect and re-projects it via
  `ClientToScreen` whenever the main window moves or resizes.
- mpv is launched with `--wid=<popup_hwnd>` plus `--idle=yes
  --force-window=yes --keep-open=yes --osc=yes
  --input-default-bindings=yes --input-ipc-server=\\.\pipe\anime-player-mpv`.
  The IPC pipe accepts JSON one-liners like
  `{"command":["loadfile","C:\\path"]}`, `{"command":["cycle","pause"]}`.
- Full mpv keyboard bindings work inside the player area (Space pause,
  ←/→ seek, F fullscreen, J subs, # audio tracks). The on-screen
  controller appears on hover.

If video issues recur (black, misaligned, ghosting), the most likely
suspects are: popup z-order vs another window, stale cached rect after
a DPI change, or mpv failing to launch (check the dev terminal for
stderr).

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
