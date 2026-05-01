# Project context for agents

A running summary of what this project is, what's been built, and the key
design decisions. New agents should read this first to avoid re-deriving
context that was already settled.

## What the project is

A local **lossless anime video player** for Windows, built with **Tauri v2 +
React + TypeScript**. The user types or browses to a folder; the app
recursively lists all video files inside; clicking a file plays it via an
embedded **libmpv** loaded in-process from `libmpv-2.dll`.

The user is on Windows (PowerShell). Rust 1.95 (stable, MSVC toolchain),
Node 22, and the bundled `src-tauri/libs/mpv/libmpv-2.dll` are the
runtime prerequisites. See `README.md` for the full setup steps.

## Architecture

### Frontend — `src/App.tsx`, `src/App.css`

- Visual design follows the local reference project
  (`reference/working-tauri-mpv-example-project` → Soia): `data-theme="dark"`,
  Roboto, near-black shell (`#0f0f0f`), blue seek accent (`#58a6ff`), and a
  bottom player chrome that matches Soia’s gradient + custom seek bar +
  transparent icon transport buttons.
- Two-pane layout: 360px sidebar (folder input, file list, filter) +
  main player pane.
- Sidebar talks to Rust via `invoke("scan_videos", { folder })`.
- The main grid (`.app`) stays **CSS-transparent** for compositing. The
  player column uses **`var(--app-bg)`** when no file is selected so the
  transparent Tauri window does not show the desktop behind an empty
  player; once a file is selected, **`.player--playback`** switches that
  column to transparent so the libmpv DirectComposition swap-chain
  (same Tauri HWND) shows through. mpv’s video region is still driven by
  `video-margin-ratio-left`, not child-window geometry.
- Window-resize layout tracking happens **natively** in Rust: the
  Tauri `WindowEvent::Resized` / `ScaleFactorChanged` handler re-issues
  `video-margin-ratio-left` directly on the UI thread on every WM_SIZE.
  An earlier JS-side `window.resize` + `invoke("mpv_set_layout")` path
  visibly stuttered the modal resize loop while a video was playing
  (each WM_SIZE was paying for a full IPC round-trip per animation
  frame). The frontend still calls `mpv_init` / `mpv_set_layout` once
  to register the initial sidebar width.
- libmpv is initialized lazily on the first file selection via
  `mpv_init`. After init, switching files is just `mpv_load`.
- A custom HTML controls bar (transport + scrubber + time + fullscreen)
  drives mpv via `mpv_cycle_pause`, `mpv_seek`, `mpv_seek_relative`, and
  reads state from `mpv://time-pos`, `mpv://duration`, `mpv://pause`,
  `mpv://eof-reached` events. The video pane uses Tauri
  `startDragging` (single left-click) and `setFullscreen` (double
  left-click or **F**); right-click toggles pause; Space and ArrowLeft/ArrowRight
  seek ±5s are handled in the frontend (capture-phase `keydown`) so they
  work while the WebView has focus without typing in sidebar fields.
  A **Close video** control (and `mpv_stop`) returns to the no-selection
  empty player pane.

### Backend — `src-tauri/src/lib.rs`, `src-tauri/src/mpv/`

- `scan_videos(folder)` uses `walkdir` to recursively find files matching
  the extensions in `VIDEO_EXTENSIONS` (`mkv`, `mp4`, `avi`, `webm`,
  `ts`, `m2ts`, etc.). Returns `{ path, name, relative_path, size }[]`,
  sorted by relative path.
- `mpv/mod.rs` re-exports `MpvHandle` from the in-process libmpv module.
  All Win32 and FFI code is gated behind `#[cfg(windows)]`.
  - `mpv/ffi.rs` — minimal `extern "C"` declarations for the libmpv
    symbols we use (`mpv_create`, `mpv_initialize`, `mpv_command`,
    `mpv_set_option_string`, `mpv_observe_property`, `mpv_wait_event`,
    `mpv_wakeup`, `mpv_terminate_destroy`, etc.).
  - `mpv/handle.rs` — `MpvHandle` owns the libmpv context plus the
    background event-loop thread. `MpvHandle::new(hwnd, app_handle)`
    sets the `wid` option to the Tauri main HWND **before**
    `mpv_initialize`, plus `vo=gpu-next`, `gpu-context=d3d11`,
    `hwdec=auto-safe`, `keep-open=yes`, `osc=no`,
    `input-default-bindings=yes`, `input-vo-keyboard=yes`.
  - `mpv/event_loop.rs` — observes `time-pos`, `duration`, `pause`,
    `eof-reached` and republishes each property change as a Tauri
    event named `mpv://<property>`.
- Exposed Tauri commands: `scan_videos`, `mpv_init(window_width,
  sidebar_px)`, `mpv_load(path)`, `mpv_cycle_pause()`, `mpv_seek(seconds)`,
  `mpv_seek_relative(delta)`, `mpv_set_layout(window_width, sidebar_px)`,
  `mpv_stop()`.
- `lib.rs` hooks the main window's `WindowEvent`:
  - `CloseRequested` drops `MpvHandle` (terminates the libmpv context
    and joins the event-loop thread) before the HWND becomes invalid.
  - `Resized` and `ScaleFactorChanged` re-issue
    `video-margin-ratio-left` based on the new logical width and the
    last sidebar-width the frontend registered (`AppState::sidebar_px`).

### Build / linkage — `src-tauri/build.rs`, `src-tauri/libs/mpv/`

- `libs/mpv/libmpv-2.dll` and `libs/mpv/mpv.lib` are committed to the
  repo (refresh with `node scripts/update-mpv-libs.mjs`, which
  downloads the latest dev bundle from
  `shinchiro/mpv-winbuild-cmake` and renames `libmpv.dll.a` → `mpv.lib`
  for MSVC).
- `build.rs` adds `cargo:rustc-link-search=native=libs/mpv` and
  `cargo:rustc-link-lib=dylib=mpv`, then copies `libmpv-2.dll` next to
  the dev binary so `cargo run` / `tauri dev` can load it.
- For installer builds, `tauri.conf.json` `bundle.resources` ships
  `libs/mpv/libmpv-2.dll` next to the exe.

### Tauri configuration — `src-tauri/tauri.conf.json`, `capabilities/default.json`

- The main window has **`transparent: true`**. Combined with libmpv's
  `vo=gpu-next` (DirectComposition swap-chain) under the same HWND,
  this is what makes the video composite cleanly with the WebView2
  surface in the same DWM tree. See the design-decision section below
  for why this works now when an earlier transparent attempt failed.
- Permissions: `core:default`, `opener:default`, `dialog:default`. The
  dialog plugin is used for the native folder picker.
- Window: `productName = "Anime Player"`, 1280x800 default, min 800x600.

## Critical design decision: in-process libmpv via FFI

The non-obvious part of the project. **mpv is loaded as a DLL in the
Tauri process, not spawned as a subprocess; its rendering target is the
Tauri main HWND itself.**

- WebView2 renders via **DirectComposition**. So does modern mpv when
  using `vo=gpu-next` / `gpu-context=d3d11`. Both create DComp visuals
  under the same parent HWND, and DWM composites them into one surface.
- `MpvHandle::new` sets `wid=<TauriHWND>` **before** `mpv_initialize`.
  libmpv then creates its own DComp swap-chain under that HWND during
  init.
- `transparent: true` on the Tauri window puts WebView2 onto the
  layered/DComp-only presentation path *without a redirection bitmap*.
  WebView2's own visual still composes (so the sidebar / chrome paint
  fine), and mpv's swap-chain composes alongside it. Anywhere CSS is
  transparent (the `.player` pane), the user sees mpv's pixels.
- Because libmpv runs in-process, **audio output belongs to the Tauri
  PID** — external audio mixers and per-window scripts treat the player
  as a single application. This was the user's primary motivation for
  the migration.
- The video must be confined to the right pane. mpv's canvas is the
  full HWND, so we set `video-margin-ratio-left = sidebar_px /
  window_width` to leave the left strip empty; the opaque sidebar
  visually covers it. The frontend re-issues `mpv_set_layout` on every
  window resize.
- libmpv input is fully wired (`input-default-bindings=yes`,
  `input-vo-keyboard=yes`) so Space pause, ←/→ seek, F fullscreen,
  J subs, # audio tracks all work inside the player area. We disabled
  the on-screen controller (`osc=no`) because we draw our own.

If video issues recur (black, ghosting, no compose), the most likely
suspects are:

- A non-transparent CSS layer landing on top of `.player` (occludes the
  swap-chain).
- mpv falling back from `vo=gpu-next` (check the dev console for mpv
  init errors).
- The HWND being torn down before `MpvHandle::drop` — the
  `CloseRequested` hook in `lib.rs` exists specifically to avoid that.

### Why a popup window was the previous answer (and isn't anymore)

Before this migration, mpv ran as `mpv.exe --wid=<popup_hwnd>` in an
owned `WS_POPUP` top-level window over the player div, driven by a
JSON IPC named pipe. It worked, but:

1. The popup is a separate HWND, so window movers / per-window audio
   mixers treated the video as a different app from the rest of the UI.
2. mpv.exe is a separate PID, so audio mixers couldn't tag it with the
   parent app's settings.
3. Drawing custom HTML controls *over* the video required keeping the
   WebView2 surface on top of a separate top-level HWND, which is hard.

The popup workaround existed because we had also tried `transparent:
true` + a `WS_CHILD` GDI host (commit 8b16dfe, reverted in fe8991a) and
mpv's pixels never reached final composition: GDI children of a
DComp-only window have nowhere to paint. The fix wasn't to give up on
transparency — it was to skip the GDI host entirely and let libmpv's
own DComp swap-chain be the surface, which only works if mpv is
in-process so we can pass the Tauri HWND directly to it. That's what
this migration is.

## Version control

The repo is a git repository on branch `main` with no remote. Every
logical agent change is committed as its own checkpoint so the user
can roll back individual steps. The full convention (when to commit,
what to stage, message style, what *not* to do — no `--amend`, no
push, no rebase without an explicit request) lives in
`.cursor/rules/commit-checkpoints.mdc`.

## Files to know

- `README.md` — user-facing setup + run + project layout.
- `TODO.md` — running list of follow-ups.
- `CONTEXT.md` — this file.
- `scripts/update-mpv-libs.mjs` — refreshes `src-tauri/libs/mpv/`
  from the latest `shinchiro/mpv-winbuild-cmake` release.
- `.cursor/rules/` — agent rules. `read-context.mdc` points new agents
  here; `commit-checkpoints.mdc` defines the per-change commit
  workflow.

## Conventions

- Communication: PowerShell-friendly commands (the user's shell is
  PowerShell). Use `Get-ChildItem`/`Read`/etc., not `cat`/`ls`.
- Don't add boilerplate comments that just describe what the code does;
  comment trade-offs and constraints (see `mpv/handle.rs` for the level
  of commentary that's expected).
- The user prefers iterative changes with verification — run
  `cargo check` and `npx tsc --noEmit` after backend/frontend edits.
- **Commit after every logical change** so each agent step is a
  checkpoint the user can roll back to. See
  `.cursor/rules/commit-checkpoints.mdc` for the full convention.

## Build/test commands

```powershell
# Frontend type-check
npx tsc --noEmit

# Rust type-check (fast, doesn't link)
cargo check --manifest-path src-tauri/Cargo.toml

# Refresh bundled libmpv
node scripts/update-mpv-libs.mjs

# Run the app (compiles Rust, launches Vite, opens the window)
npm run tauri dev

# Production build (installer in src-tauri/target/release/bundle/)
npm run tauri build
```

A full `cargo check` from cold is ~25s on the user's machine; warm is <2s.
