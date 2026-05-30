# Project context for agents

A running summary of what this project is, what's been built, and the key
design decisions. New agents should read this first to avoid re-deriving
context that was already settled.

## What the project is

A local **lossless anime video player** for Windows, built with **Tauri v2 +
React + TypeScript**. The app scans configured root folders into a
SQLite-backed library, groups files into anime/episode views, tracks local
progress, links titles to AniList, and plays video through **libmpv** loaded
in-process from `libmpv-2.dll`.

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
`EpisodeScreen.tsx`, `BulkEditScreen.tsx`, `JobsScreen.tsx`, `SettingsScreen.tsx` (plus the local
`RuleEditor`), `PlayerView.tsx`, `ViewHeader.tsx`, `CustomDropdown.tsx`,
`ToastStack.tsx`, `icons.tsx`. The `pickQuickPlayEpisode` helper used by both `App.tsx` and
`EpisodeScreen.tsx` lives in `src/quickPlay.ts`.

- The library/settings UI is an original dark design. Only the video player
  chrome intentionally follows the local Soia reference project
  (`reference/working-tauri-mpv-example-project`): the bottom controls gradient,
  custom seek bar, and transparent icon transport buttons. During playback, the
  player surface stays transparent where mpv should show through; opaque CSS
  layers are only used for idle/pending states and transition covers.
- App layout is a narrow icon sidebar (`--sidebar-width`, currently 56px) plus
  a centered main content pane. The sidebar owns top-level navigation and quick
  actions; category navigation lives on the library page. A **Missing** sidebar
  page appears only when the database has episodes that the latest scan could
  not currently match. Normal library/search/episode views hide missing rows so
  rule mistakes are obvious without deleting saved metadata.
- Bulk Edit can filter anime by source category and by a case-insensitive regex
  matched against full episode paths, then move the affected anime to another
  category. Its filename replacer tab scans all video files under configured
  root folders, so malformed files that are not yet detected as episodes can be
  renamed and then imported by the follow-up rescan.
- A **Jobs** sidebar page (between Settings and Rescan) lists queued/running
  background work and job history (two tabs, like Bulk Edit). The nav icon shows
  a badge with the count of queued + running jobs. **Max parallel jobs** (1–8,
  default 5, stored in SQLite `settings` as `jobs_max_parallel`) caps how many
  jobs may run at once for scheduling low/medium work (high-priority jobs count
  toward that cap but are never blocked by it—e.g. six low jobs at the limit can
  still be joined by a seventh high job). Jobs also have a **resource type**
  (`none` by default; scrub thumbnails use `ffmpeg`) with its own max-parallel
  limit (`jobs_max_parallel_type_ffmpeg`, default 1). A job starts only when
  both caps allow it: low/medium respect `min(global, type)`; high bypasses the
  global cap but **not** the type cap. Jobs dedupe by `identity`, support cancel/cancel-all,
  progress steps, and emit `jobs://updated` / `jobs://finished`. Frontend
  helpers live in `src/jobs/jobClient.ts` (`subscribeJobsSnapshot`, `waitForJob`,
  `onJobIdentityFinished`).
- The anime grid header includes a sort dropdown (alphabetical, most recent
  episode, last watched, episode/remaining counts); the choice is persisted in
  `localStorage` under `animePlayer.animeGridSort`. **Most recent** sorts by
  `anime.latest_episode_at` descending. That field is stored on `anime` and
  recomputed after every library rescan as `MAX(episodes.updated_at)` per show
  (`db::refresh_anime_latest_episode_at`). On each app start, if at least one
  root folder is configured, the app runs `rescan_library` once after the
  initial `get_library_state` so the library and this timestamp stay current.
- `src/api.ts` contains the Tauri command bindings and `src/types.ts` mirrors
  the serialized Rust DTOs.
- Settings talks to Rust via library commands such as `get_library_state`,
  `add_root_folder`, `rescan_library`, `get_local_data_stats`,
  `clean_local_data`, `list_episodes`, `list_root_video_files`, `delete_anime_files`,
  `move_anime_to_category`, `set_default_category`, editable
  `*_regex_rule` commands, `rename_files`, `rename_anime`,
  `save_episode_progress`, and the Windows-only
  `get_file_thumbnail` Shell thumbnail helper. The legacy `scan_videos`
  command still exists for compatibility.
- AniList integration is a mature optional tracker layer. Settings exposes a
  default OAuth client ID (`40455`) that works without custom setup, plus an
  override field for users with their own AniList app. A **Use AniList title in
  the library when linked** toggle (`prefer_anilist_display_title` in SQLite
  `settings`) switches grids, search, continue watching, episode headers, and
  the player window title to the linked AniList name; unlinked titles always use
  the detected filesystem name. Login uses the implicit
  OAuth flow with the `anime-player://anilist-auth` deep-link callback;
  `App.tsx` handles the callback, validates/stores the token through Rust, and
  refreshes auth state in the UI.
- Linked anime have search/link/unlink/open controls on the episode page.
  Title Settings also exposes an optional per-anime `custom_thumbnail_path`
  (with browse/clear controls). Poster loading in grids and Continue Watching
  prefers that custom thumbnail first, then cached AniList cover art, then
  Windows shell thumbnails from `AnimeSummary.first_episode_path`.
  AniList cover paths are stored relative to the portable `data` directory when
  possible (`anilist-covers/<id>.<ext>`), so moving the app folder does not break
  them. Poster loading is phased so cached covers appear first and local thumbnail
  extraction runs with limited concurrency; linked titles with a missing/legacy
  cover path can recover by checking `data/anilist-covers/<anilist_id>.*` and then
  re-downloading the cover from AniList.
- The AniList card opens the linked AniList page, shows remote
  `Progress: current/total`, and includes a debounced score input that writes
  back to AniList. Local watched progress syncs on EOF / near-end saves only
  when the adjusted local episode number is ahead of the viewer's current
  AniList progress; `persistProgress` in `PlayerView` awaits
  `syncAnilistEpisodeProgress` in that case (Q/back, EOF advance, episode
  switches, and natural window close all use the same path). Reopening an
  episode that was already watched does not write new progress if the player
  session lasts under five minutes, unless the user seeks into the start
  period (`MIN_POSITION_SECONDS_TO_PERSIST`, exposed to the UI via
  `get_min_position_seconds_to_persist`), which clears watched state through
  the normal save path. `tracker_offset` is per-title and is subtracted from parsed
  episode numbers for display, AniList progress sync, and watched-progress
  import.
- AniList progress import is conservative: opening or linking a title can mark
  matching local episodes watched from remote progress, but it does not clear
  later local progress. The title settings popup also has a force progress
  override for intentionally rewriting all local episode progress; when a title
  is AniList-linked the override field prefills from remote progress and save
  skips the override path if the value still matches that progress; save also
  skips tracker/thumbnail IPC and library refresh when nothing changed, and
  filesystem-backed anime rename keeps categories, progress, tracker offset,
  and AniList links attached to the same anime ID while renaming files on disk.
- Transient toast notifications are rendered by `src/App.tsx` for operation
  status and errors.
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
  field) toggles Tauri window fullscreen like a browser. Fullscreen entered
  only during a player session (F11, F, or double-click on the video) is
  reverted to windowed when leaving the player; if the window was already
  fullscreen when playback started, it stays fullscreen after exit. **Escape** matches
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
  The title bar uses the generated app icon at `src-tauri/icons/icon.png`,
  matching the Tauri-generated executable icons.
  When the user closes the window while an episode session is loaded,
  `App.tsx` listens with `getCurrentWindow().onCloseRequested`, calls
  `PlayerView`'s `persistProgress` through `playbackProgressFlushRef`, then
  `destroy()` after that promise settles (SQLite plus any awaited watched
  AniList sync—the same as Q/back except pause is skipped on exit so mpv may
  already be torn down in Rust). Abrupt process death still skips this. That path needs
  **`core:window:allow-destroy`** in the default capability file in addition
  to `allow-close`.
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
  + volume + aspect fit + fullscreen) drives mpv via `mpv_cycle_pause`,
  `mpv_set_pause`, `mpv_seek`, `mpv_seek_relative`, `mpv_set_volume`,
  track selection commands, and reads state from `mpv://time-pos`,
  `mpv://duration`, `mpv://pause`, `mpv://eof-reached`,
  `mpv://file-loaded`, and `mpv://playback-restart` events. Controls
  fade out after pointer idle, hide immediately when the pointer leaves
  the window (unless a seek, track menu, or volume popup is active), and
  are revealed by mouse movement, active menu/seek interaction, or **C**
  to toggle visibility.
- Scrubber hover shows a floating thumbnail plus timestamp. Sprite sheets live
  under `data/scrub-sprites/` (bundled ffmpeg/ffprobe, 160×90 cells, ~one frame
  every five seconds capped at 120). **Scrub thumbnail** work runs as background
  jobs (`jobs_enqueue_scrub_sprite`): a rescan that imports ≤20 episodes
  auto-queues scrub jobs (low priority); opening an anime’s episode list queues
  **medium**-priority jobs for uncached episodes and downgrades them to **low**
  when leaving that page (`jobs_set_scrub_sprite_priority_for_paths`); opening
  the player upgrades the current file’s queued job to **high** (starts
  immediately). Queued priorities can be changed via `jobs_set_job_priority`.
  `get_scrub_sprite_if_ready` reads the
  cache synchronously; finished jobs emit `scrub-sprite-ready`. Generation uses
  ffmpeg with `-hwaccel auto` and `-skip_frame nokey`. The UI slices the sheet
  via CSS `background-position`.
- Scrubber drag pauses playback (if it was playing), issues throttled keyframe
  `mpv_seek` preview seeks (`keyframe: true` / `absolute+keyframes`), then on
  release one final keyframe seek. After the seek settles, the UI reads
  `mpv_get_time_pos` so the bar, clock, and saved progress match the decoded
  keyframe (not the pre-snap scrub target). Resumes only when playback was
  active before the drag. `mpv://time-pos` updates are ignored while scrubbing
  so the bar follows the drag target until release.
- The video pane uses Tauri `startDragging` (single left-click; skipped when
  the window is maximized or fullscreen) and
  `setFullscreen` (double left-click canvas, **F** on the player control,
  or **F11** app-wide); right-click toggles
  pause; **C** toggles player chrome visibility; Space and ArrowLeft/ArrowRight seek ±5s; Ctrl+ArrowLeft/ArrowRight
  loads the previous/next episode in the list when available; Numpad 4/6 seek ±28s and
  Numpad 7/9 seek ±85s; **W**/**S** bump volume ±2 steps (mpv's native
  0–130 range); mouse scroll wheel uses the same step;
  **M** or a click on the volume icon toggles mute (saved level is restored
  on unmute); adjusting volume with **W**/**S**, the wheel, or the slider
  clears mute;
  **Q**, **Escape**, or the back
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
  seeds default categories (`Ongoing`, `Completed`, `Finished`) and
  fansub/simple-title/generic video regex rules only when those tables are
  empty (deleting a default rule does not restore it on restart).
  `AppDatabase::with_conn` exposes `&mut Connection`
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
  `save_episode_progress`, `rename_anime`, `get_local_data_stats`, `clean_local_data`,
  and `rescan_library`. On Windows, when a rescan imports at most 20 episodes
  (new or updated paths), it enqueues low-priority scrub-thumbnail jobs for
  those files. `rescan_library` commits one SQLite transaction per
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
  `get_local_data_stats` (database, AniList covers, and scrub-sprite cache sizes);
  the explicit `clean_local_data` action prunes stale episodes/unmatched rows,
  removes anime with no episodes, vacuums SQLite, and deletes unreferenced saved
  covers and scrub sprites (matched by episode path keys in sprite metadata). The episode page's `delete_anime_files`
  command deletes/trashes the currently visible episode files for an anime,
  removes any now-empty parent folders up to (but not including) the matching
  configured root folder (including sibling empty folders such as an unused
  `Watched` directory when the season folder has no files and no other content),
  removes scrub sprite cache and that anime's cached
  AniList cover (and custom thumbnail when the title is removed), and deletes
  the corresponding SQLite rows (whole title when every targeted file succeeds,
  otherwise only the removed episode rows).
  `set_anime_custom_thumbnail_path` updates/clears the optional per-title
  custom poster file path on `anime.custom_thumbnail_path`.
  `save_episode_progress` stores `position_seconds`
  as 0 when the reported position is under `MIN_POSITION_SECONDS_TO_PERSIST`
  (60 seconds) so brief opens do not leave a resume point; when `watched` is
  true (end-of-episode threshold), it stores `position_seconds` at full
  duration (100%). `get_min_position_seconds_to_persist` exposes that cutoff
  to the frontend for start-reset detection in `PlayerView`.
- `anilist.rs` owns the AniList OAuth, GraphQL, cover-cache, and progress/score
  sync layer. It stores an optional custom OAuth client ID, access token, and
  viewer metadata in `settings`; stores link metadata on `anime` rows
  (`anilist_id`, title, site URL, cached cover path); searches AniList through
  `https://graphql.anilist.co`; validates login tokens with `Viewer`; and
  downloads covers under the portable `data/anilist-covers/` directory. Linked
  media status (`progress`, `episodes`, `score`) is cached on the `anime` row
  for five minutes, and app-originated score/progress writes update that cache
  immediately. Progress sync/import uses `tracker_offset`, only advances
  AniList when local progress is ahead, and only marks local episodes watched
  when importing remote progress. Network requests are kept outside the SQLite
  mutex.
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
  `mpv_seek(seconds, keyframe?)` (`keyframe: true` → `absolute+keyframes`;
  default exact), `mpv_seek_relative(delta)`,
  `mpv_set_layout(window_width, sidebar_px)`, `mpv_get_tracks()`,
  `mpv_select_audio_track(track_id)`,
  `mpv_select_subtitle_track(track_id)`,
  `mpv_add_subtitle_file(path)`, `mpv_get_video_geometry()`,
  `mpv_get_time_pos()`, `mpv_set_volume(volume)`, and `mpv_stop()`.
- `scrub_preview.rs` — sprite cache I/O and ffmpeg generation;
  `get_scrub_sprite_if_ready_cmd`, `scrub_sprite_is_cached_cmd`.
- `jobs/` — `JobManager` in `AppState`, scheduler (priority, parallel limit,
  dedupe), scrub-sprite worker; commands `jobs_get_snapshot`, `jobs_enqueue_scrub_sprite`,
  `jobs_set_job_priority`, `jobs_set_scrub_sprite_priority_for_paths`, `jobs_cancel`,
  `jobs_cancel_all`, `jobs_set_max_parallel`.
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
  the dev binary so `cargo run` / `tauri dev` can load it. It also
  declares rerun triggers for `icons/icon.ico` and `icons/icon.png` so
  Windows executable resources are rebuilt
  after icon changes.
- For installer builds, `tauri.conf.json` `bundle.resources` ships
  `libs/mpv/libmpv-2.dll` next to the exe.

### Tauri configuration — `src-tauri/tauri.conf.json`, `capabilities/default.json`

- The main window has **`transparent: true`**. Combined with libmpv's
  `vo=gpu-next` (DirectComposition swap-chain) under the same HWND,
  this is what makes the video composite cleanly with the WebView2
  surface in the same DWM tree.
- The main window also has **`decorations: false`** so Windows does not
  draw the native title bar or 1px frame; the frontend supplies the
  custom title bar and window controls.
- Permissions: `core:default`, `opener:default`, `dialog:default`, and
  `deep-link:default`. The opener plugin opens AniList URLs from the
  frontend and episode folders from the Rust `open_anime_episode_folder`
  command, which derives the folder from database episode paths instead of
  requiring a broad frontend filesystem scope. The dialog plugin is used for
  the native folder picker; the deep-link and single-instance plugins support
  the AniList `anime-player://anilist-auth` OAuth callback.
- Window: `productName = "Anime Player"`, 1280x800 default, min 800x600.

## Video architecture: in-process libmpv via FFI

mpv is loaded from `libmpv-2.dll` inside the Tauri process. It is not spawned as
`mpv.exe`; `MpvHandle::new` creates the libmpv context, sets
`wid=<TauriHWND>` before `mpv_initialize`, and targets the Tauri main HWND.

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
  PID** and external audio mixers / per-window scripts treat the player
  as one application.
- When the library UI is visible, mpv's full-HWND canvas is confined with
  `video-margin-ratio-left = sidebar_px / window_width`. The frontend registers
  the current sidebar width once, and native `WindowEvent::Resized` /
  `ScaleFactorChanged` handling in `lib.rs` re-applies the margin during window
  changes without a JS IPC round-trip per resize event.
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

## Version control

The repo is a git repository on branch `main` with an `origin` remote. Every
logical agent change is committed locally as its own checkpoint so the user can
roll back individual steps. Agents should **never push** unless the user
explicitly requests it. The full convention (when to commit, what to stage,
message style, what *not* to do — no `--amend`, no push, no rebase without an
explicit request) lives in `.cursor/rules/commit-checkpoints.mdc`.

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
- `src/volume.ts` — volume constants and clamping for
  mpv's native 0–130 log scale.
- `src-tauri/src/db.rs`, `src-tauri/src/library.rs`,
  `src-tauri/src/scanner.rs` — portable SQLite, library commands, and
  regex scanner.
- `src-tauri/src/anilist.rs` — AniList OAuth callback handling, GraphQL
  search/link commands, and local cover caching.
- `scripts/download-mpv-libs.mjs` — setup entry point that downloads
  ignored local libmpv artifacts into `src-tauri/libs/mpv/`.
- `scripts/update-mpv-libs.mjs` — compatibility updater for the same
  artifacts from the latest `shinchiro/mpv-winbuild-cmake` release.
- `scripts/release-notes.mjs` and `scripts/publish-github-release.mjs` — automated
  GitHub release flow (run `npm run release:notes` then `npm run release:publish`).
- `scripts/update.bat` and `scripts/_update.ps1` — portable end-user
  updater (`update.bat` only; `_update.ps1` is internal). Shipped in the
  release zip; downloads `anime-player.exe` from GitHub `releases/latest`.
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

# Regenerate Tauri icons from a square SVG or PNG
npm run icon:setup -- path\to\icon.svg

# Run the app (compiles Rust, launches Vite, opens the window)
npm run tauri dev

# Production build (installer in src-tauri/target/release/bundle/)
npm run tauri build

# Portable release: `tauri build` then versioned folder + zip under `releases/`
# (that directory is listed in `.gitignore` so build artifacts stay local).
# Package includes libmpv-2.dll, ffmpeg.exe, ffprobe.exe, update.bat,
# _update.ps1, and VERSION.txt (release only). Scrub thumbnails prefer
# ffmpeg beside the exe; PATH is a fallback if those files are removed.
# Publish GitHub release with the zip, standalone anime-player.exe, and
# anime-player.exe.sha256.
npm run release

# Local dev portable only (no tag, zip, or publish artifacts): `releases/dev/`
npm run portable
```

A full `cargo check` from cold is ~25s on the user's machine; warm is <2s.
