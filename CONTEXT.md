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
Node 22, 7-Zip on PATH, and the local generated
`src-tauri/libs/mpv/libmpv-2.dll` are the runtime prerequisites. See
`README.md` for the full setup steps.

## Architecture

### Frontend — `src/App.tsx`, `src/App.css` (barrel `@import` of `src/styles/*.css`), `src/api.ts`, `src/types.ts`, `src/components/*`

The frontend is split so `App.tsx` only owns top-level state (library data,
view selection, async handlers, toasts, F11 / Esc / Q hotkeys) and composes the
view components. Per-screen UI lives in `src/components/`:
`WindowTitleBar.tsx`, `CategoryScreen.tsx`, `AnimeGrid.tsx`,
`EpisodeScreen.tsx`, `BulkEditScreen.tsx`, `SettingsScreen.tsx` (plus the local
`RuleEditor`), `PlayerView.tsx`, `ViewHeader.tsx`, `CustomDropdown.tsx`,
`ToastStack.tsx`, `icons.tsx`. The `pickQuickPlayEpisode` helper used by both `App.tsx` and
`EpisodeScreen.tsx` lives in `src/quickPlay.ts`.

- Visual design follows the local reference project
  (`reference/working-tauri-mpv-example-project` → Soia): `data-theme="dark"`,
  Roboto, near-black shell (`#0f0f0f`), blue seek accent (`#58a6ff`), and a
  bottom player chrome that matches Soia’s gradient + custom seek bar +
  transparent icon transport buttons. The sidebar and **idle / pending**
  player pane share **`--ui-bg`** with **`backdrop-filter` blur** so the
  desktop shows through slightly frosted; during **`.player--playback`** blur
  is disabled so mpv stays sharp.
- Two-pane layout: 280px sidebar (Library, Search, Bulk Edit, Settings, rescan,
  stats) + main content pane. Category navigation lives on the library page rather
  than in the sidebar. Bulk Edit can filter anime by source category and by a
  case-insensitive regex matched against full episode paths, then move the
  affected anime to another category. A **Missing** sidebar page appears only when the
  database has episodes that the latest scan could not currently match; normal
  library/search/episode views hide those missing rows so rule mistakes are
  obvious without deleting saved metadata. The anime grid header includes a sort dropdown
  (alphabetical, most recent episode, last watched, episode/remaining counts);
  the choice is persisted in `localStorage` under `animePlayer.animeGridSort`.
  **Most recent** sorts by `anime.latest_episode_at` descending (shows with
  the freshest episode activity first). That field is **stored on `anime`**
  and recomputed after every library rescan as `MAX(episodes.updated_at)`
  per show (`db::refresh_anime_latest_episode_at`). On each app start, if at
  least one root folder is configured, the app runs `rescan_library` once
  (after the initial `get_library_state`) so the library and this timestamp
  stay current without a manual rescan.
- The MVP file list has been replaced by a SQLite-backed local library UI:
  category screen, anime grid, episode list, settings, and player view.
  `src/api.ts` contains the Tauri command bindings and `src/types.ts`
  mirrors the serialized Rust DTOs.
- Settings talks to Rust via library commands such as `get_library_state`,
  `add_root_folder`, `rescan_library`, `get_local_data_stats`,
  `clean_local_data`, `list_episodes`, `delete_anime_files`,
  `move_anime_to_category`, `set_default_category`, editable
  `*_regex_rule` commands, `save_episode_progress`, and the Windows-only
  `get_file_thumbnail` Shell thumbnail helper. The legacy `scan_videos`
  command still exists for compatibility.
- AniList integration is a first-slice metadata/linking feature. Settings
  stores an AniList OAuth client ID and opens the implicit OAuth flow with
  `anime-player://anilist-auth` as the custom URI callback. `App.tsx`
  listens for Tauri deep-link callbacks, validates/stores the token through
  Rust, and exposes search/link/unlink/open controls on the anime episode
  page. Linked anime prefer cached AniList cover art over placeholder
  initials in the grid and episode header; if no cover is available (unlinked,
  missing file, or load failure), the grid and Continue Watching use a
  Windows shell thumbnail from the first local episode (`first_episode_path`
  in `AnimeSummary`, same ordering as the episode list). Grid/Continue poster
  loading is intentionally phased: cached AniList covers are loaded and shown
  first, then local video thumbnail extraction runs with limited concurrency so
  large categories do not flood the backend. The linked info card opens
  AniList, shows remote `Progress: current/total`, and has a debounced score
  input that writes to AniList. When a local progress save marks an episode
  watched (EOF or the near-end threshold used by hide/next), the player asks
  Rust to sync AniList progress for the linked anime if the parsed local
  episode number is ahead of the viewer's current AniList progress. Opening a
  linked anime with no local episode progress imports AniList progress by
  marking matching local episodes watched; linking an anime does the same
  watched-only import without clearing any later local progress.
- Page-level status/error banners have been replaced with transient
  toast notifications rendered by `src/App.tsx`; toasts slide in from
  above and dismiss automatically.
- The app window is frameless (`decorations: false`) and draws its own
  React title bar in `src/App.tsx`. The title bar owns dragging,
  double-click maximize, and minimize / maximize / close controls. While
  the window is **fullscreen** (F11 or the player control), that title bar
  is not rendered so those controls stay hidden. On
  library/settings views it stays visible when not fullscreen; **in the video player** it
  shows and hides with the player chrome (see `onControlsVisibilityChange`
  in `PlayerView`), uses no extra top gradient (the player UI already
  shades the top/bottom), and the **“Anime Player”** label stays hidden
  so only the system buttons appear. **F11** (globally, unless typing in a
  field) toggles Tauri window fullscreen like a browser. **Escape** matches
  the per-screen back control (library → up a level, settings → library);
  in the player it uses the same path as **Q** / the back arrow (pause +
  persist + return to episodes). Those actions
  require explicit `core:window:allow-*` entries in
  `src-tauri/capabilities/default.json` (Tauri v2 ACL).
  Even though the custom title bar hides the **Anime Player** label in the
  player, **`App.tsx` updates the native OS window title** via Tauri
  `set_title` when the player view is visible: `Playing - [shortened anime
  title] - Anime Player` while playback is unpaused and `Paused - …`
  while paused (shortening uses `shortenForOsTitle` in `src/utils.ts`);
  otherwise the title is reset to `Anime Player`. This needs
  **`core:window:allow-set-title`** in the default capability file.
- The main grid (`.app`) stays **CSS-transparent** for compositing. The
  player column is **`var(--app-bg)`** when idle or while a new file is
  opening (**`.player--playback-pending`**); only after mpv emits
  **`mpv://playback-restart`** (decoder/first-frame restart) does
  **`.player--playback`** make that column transparent so the DComp
  swap-chain shows through—avoiding a desktop flash before the first
  frame. Opening an episode briefly fades the selection UI into black,
  then `PlayerView` fades that black cover out after playback restart so
  the video appears smoothly. mpv’s video region is still driven by
  `video-margin-ratio-left`, not child-window geometry.
- Window-resize layout tracking happens **natively** in Rust: the
  Tauri `WindowEvent::Resized` / `ScaleFactorChanged` handler re-issues
  `video-margin-ratio-left` directly on the UI thread on every WM_SIZE.
  An earlier JS-side `window.resize` + `invoke("mpv_set_layout")` path
  visibly stuttered the modal resize loop while a video was playing
  (each WM_SIZE was paying for a full IPC round-trip per animation
  frame). The frontend still calls `mpv_init` / `mpv_set_layout` once
  to register the initial sidebar width.
- `src/components/PlayerView.tsx` owns mpv playback UI/state. libmpv is
  initialized lazily on the first episode selection via `mpv_init` and
  then remains alive for the app session. Opening or switching files is
  `mpv_load`, which runs `loadfile` then **`set pause no`** so each new
  file autoplays instead of inheriting the previous pause state. The
  player view is mounted as the loaded playback session even when hidden
  behind the episode list.
  Player-internal reset for a new file keys off `episode.id` only (not
  saved progress fields) so `save_episode_progress` updates do not clear
  `videoCompositorRevealed` and paint opaque pending chrome over mpv.
  When the same path is already loaded, showing the player again runs
  unpause without a fresh `mpv_load` (matching Q-to-resume behavior).
- The video player is full-window while visible: `.app--player-open`
  hides the sidebar/content and passes `sidebar_px = 0` to mpv so
  `video-margin-ratio-left` is cleared. When **Q**, **Escape**, or the back arrow
  returns to the episode list, playback is paused via `mpv_set_pause`
  and mpv is pushed fully off-screen with an oversized stored sidebar
  margin, so native resize handling cannot reveal a stale video strip
  without unloading mpv.
- A custom HTML controls bar (transport + scrubber + time + track menus
  + aspect fit + fullscreen) drives mpv via `mpv_cycle_pause`,
  `mpv_set_pause`, `mpv_seek`, `mpv_seek_relative`, track selection
  commands, and reads state from `mpv://time-pos`, `mpv://duration`,
  `mpv://pause`, `mpv://eof-reached`, `mpv://file-loaded`, and
  `mpv://playback-restart` events. Controls fade out after pointer idle,
  hide immediately when the pointer leaves the window (unless a seek or
  track menu is active), and are revealed by mouse movement, active
  menu/seek interaction, or **C** to toggle visibility.
- The video pane uses Tauri `startDragging` (single left-click) and
  `setFullscreen` (double left-click canvas, **F** on the player control,
  or **F11** app-wide); right-click toggles
  pause; **C** toggles player chrome visibility; Space and ArrowLeft/ArrowRight seek ±5s; Ctrl+ArrowLeft/ArrowRight
  loads the previous/next episode in the list when available; Numpad 4/6 seek ±28s and
  Numpad 7/9 seek ±85s; **Q**, **Escape**, or the back
  control returns to the episode list (without unloading mpv). On the
  episode list, **Q** is owned by `App.tsx`'s `pickQuickPlayEpisode`
  helper: it plays the current anime's most recently played episode, the
  next unwatched episode in list order if that one is already watched, or the
  first unwatched episode when the anime has no playback history yet. Returns
  null (Q no-op) only when no unwatched candidate remains.
  When the chosen target is the same file already loaded in the hidden
  player, App skips the open-fade and just flips `view` to `"player"`
  so PlayerView's `visible` effect handles the unpause.   At EOF the
  frontend saves progress, loads the next episode if one exists, or stops
  mpv and returns to the episode list. When advancing to the next episode
  from EOF or via **Next** while the current episode is past the near-end
  threshold (same 90% as the watched flag), the next episode’s saved
  position is cleared so playback starts from the beginning.

### Backend — `src-tauri/src/lib.rs`, `src-tauri/src/db.rs`,
`src-tauri/src/library.rs`, `src-tauri/src/scanner.rs`, `src-tauri/src/mpv/`

- `db.rs` opens a portable SQLite database at
  `<current-exe>/data/anime-player.db`, creates the first schema, and
  seeds default categories (`Ongoing`, `Completed`, `Finished`) plus a
  fansub-style regex rule. `AppDatabase::with_conn` exposes `&mut Connection`
  so callers can start transactions (e.g. bulk rescan writes).
- `scanner.rs` owns recursive video discovery, extension filtering, and
  regex-based anime title / episode extraction. When multiple enabled
  detection rules match a filename, the highest numeric `priority` wins
  (`ORDER BY priority DESC, id`). Unmatched video files are preserved in
  SQLite for diagnostics instead of silently disappearing.
- `library.rs` exposes the local-library Tauri commands and keeps the
  legacy `scan_videos(folder)` command. Current commands include:
  `get_library_state`, `add_root_folder`, `remove_root_folder` (deletes that
  root’s episodes and any anime that no longer has files, so the library does
  not keep stale entries from removed roots),
  `create_category`, `delete_category`, `set_default_category`,
  `create_regex_rule`, `update_regex_rule`, `delete_regex_rule`,
  `move_anime_to_category`, `list_episodes`, `get_matching_detection_rule_name`
  (re-runs filename matching against enabled rules for the episode list; the
  episode page shows the resulting rule name without persisting it),
  `save_episode_progress`, `get_local_data_stats`, `clean_local_data`,
  and `rescan_library`. `rescan_library` commits one SQLite transaction per
  root folder and caches `title_key` → `anime_id` while importing so each
  series is upserted once per scan instead of once per file. The upserts are
  idempotent: unchanged anime, episode, and unmatched-file rows are not
  rewritten, so a no-op rescan does not refresh `updated_at`/`detected_at` or
  reorder the "Most recent" grid. Rescans are intentionally nondestructive:
  episode rows that are missing or no longer match the current detection rules
  stay in SQLite so a temporary rule mistake does not lose links, categories,
  or watch progress. Rescans mark those rows with `episodes.missing = 1`;
  regular library summaries and `list_episodes` filter to `missing = 0`, while
  `LibraryState.missing_anime` drives the Missing sidebar page with
  `missing/total` counts. Settings shows database and saved AniList cover sizes via
  `get_local_data_stats`; the explicit `clean_local_data` action prunes stale
  episodes/unmatched rows, removes anime with no episodes, vacuums SQLite, and
  deletes unreferenced saved covers. The episode page's `delete_anime_files`
  command deletes/trashes the currently visible episode files for an anime,
  marks those episode rows missing, and removes that anime's cached AniList
  cover path/file while leaving the database rows for Settings cleanup.
  `save_episode_progress` stores `position_seconds`
  as 0 when the reported position is under 60 seconds so brief opens do
  not leave a resume point; when `watched` is true (end-of-episode
  threshold), it stores `position_seconds` at full duration (100%).
- `anilist.rs` owns AniList GraphQL/OAuth work. It stores the OAuth
  client ID, access token, and viewer metadata in `settings`, adds link
  metadata on `anime` rows (`anilist_id`, title, site URL, cached cover
  path), searches AniList via `https://graphql.anilist.co`, validates
  login tokens with `Viewer`, and downloads covers under the portable
  `data/anilist-covers/` directory. Linked media status (`progress`,
  `episodes`, `score`) is cached on the `anime` row for five minutes so normal
  navigation does not repeatedly hit AniList; app-originated score/progress
  writes update that cache immediately. `sync_anilist_episode_progress` uses
  the fresh cache when possible and only sends `SaveMediaListEntry` when the
  finished local episode number is greater than the known remote progress.
  `apply_anilist_progress_to_local` uses the same cached-or-fetched media
  status to mark local episodes watched without changing unwatched rows.
  Network requests are kept outside the SQLite mutex.
- `mpv/mod.rs` re-exports `MpvHandle` plus typed mpv DTOs from the
  in-process libmpv module. All Win32 and FFI code is gated behind
  `#[cfg(windows)]`.
  - `mpv/ffi.rs` — minimal `extern "C"` declarations for the libmpv
    symbols we use (`mpv_create`, `mpv_initialize`, `mpv_command`,
    `mpv_set_option_string`, `mpv_observe_property`, `mpv_wait_event`,
    `mpv_get_property`, `mpv_free_node_contents`, `mpv_wakeup`,
    `mpv_terminate_destroy`, etc.).
  - `mpv/handle.rs` — `MpvHandle` owns the libmpv context plus the
    background event-loop thread. `MpvHandle::new(hwnd, app_handle)`
    sets the `wid` option to the Tauri main HWND **before**
    `mpv_initialize`, plus `vo=gpu-next`, `gpu-context=d3d11`,
    `hwdec=auto-safe`, `keep-open=yes`, `osc=no`,
    `input-default-bindings=yes`, `input-vo-keyboard=yes`.
  - `mpv/event_loop.rs` — observes `time-pos`, `duration`, `pause`,
    `eof-reached` and republishes each property change as a Tauri
    event named `mpv://<property>`.
- Exposed mpv Tauri commands: `mpv_init(window_width, sidebar_px)`,
  `mpv_load(path)`, `mpv_cycle_pause()`, `mpv_set_pause(paused)`,
  `mpv_seek(seconds)`, `mpv_seek_relative(delta)`,
  `mpv_set_layout(window_width, sidebar_px)`, `mpv_get_tracks()`,
  `mpv_select_audio_track(track_id)`,
  `mpv_select_subtitle_track(track_id)`, `mpv_get_video_geometry()`,
  and `mpv_stop()`.
- `lib.rs` hooks the main window's `WindowEvent`:
  - `CloseRequested` drops `MpvHandle` (terminates the libmpv context
    and joins the event-loop thread) before the HWND becomes invalid.
  - `Resized` and `ScaleFactorChanged` re-issue
    `video-margin-ratio-left` based on the new logical width and the
    last sidebar-width the frontend registered (`AppState::sidebar_px`).

### Build / linkage — `src-tauri/build.rs`, `src-tauri/libs/mpv/`

- `libs/mpv/libmpv-2.dll`, `libs/mpv/mpv.lib`, and `libs/mpv/VERSION.txt`
  are generated local artifacts ignored by git. Install or refresh them
  with `npm run setup:mpv`, which downloads the latest dev bundle from
  `shinchiro/mpv-winbuild-cmake` and renames `libmpv.dll.a` → `mpv.lib`
  for MSVC.
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
- The main window also has **`decorations: false`** so Windows does not
  draw the native title bar or 1px frame; the frontend supplies the
  custom title bar and window controls.
- Permissions: `core:default`, `opener:default`, `opener:allow-open-path`,
  `dialog:default`, and `deep-link:default`. The opener plugin opens AniList
  URLs and episode folders; the dialog plugin is used for the native folder
  picker; the deep-link and single-instance plugins support the AniList
  `anime-player://anilist-auth` OAuth callback.
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
- `src/api.ts`, `src/types.ts`, `src/utils.ts` — frontend command
  bindings, shared DTOs, and formatting helpers.
- `src/components/PlayerView.tsx` — mpv-backed player view and controls.
- `src/components/{CategoryScreen,AnimeGrid,EpisodeScreen,SettingsScreen,WindowTitleBar,ViewHeader,CustomDropdown,ToastStack,icons}.tsx`
  — split-out view components composed by `App.tsx`.
- `src/quickPlay.ts` — Q-hotkey "next episode to play" picker.
- `src-tauri/src/db.rs`, `src-tauri/src/library.rs`,
  `src-tauri/src/scanner.rs` — portable SQLite, library commands, and
  regex scanner.
- `src-tauri/src/anilist.rs` — AniList OAuth callback handling, GraphQL
  search/link commands, and local cover caching.
- `scripts/download-mpv-libs.mjs` — setup entry point that downloads
  ignored local libmpv artifacts into `src-tauri/libs/mpv/`.
- `scripts/update-mpv-libs.mjs` — compatibility updater for the same
  artifacts from the latest `shinchiro/mpv-winbuild-cmake` release.
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
  checkpoint the user can roll back to. Before a final response after
  verified edits, check whether the change has been committed; if not,
  commit it or explain why it is not ready. See
  `.cursor/rules/commit-checkpoints.mdc` for the full convention.

## Build/test commands

```powershell
# Frontend type-check
npx tsc --noEmit

# Rust type-check (fast, doesn't link)
cargo check --manifest-path src-tauri/Cargo.toml

# Download / refresh local libmpv
npm run setup:mpv

# Run the app (compiles Rust, launches Vite, opens the window)
npm run tauri dev

# Production build (installer in src-tauri/target/release/bundle/)
npm run tauri build
```

A full `cargo check` from cold is ~25s on the user's machine; warm is <2s.
