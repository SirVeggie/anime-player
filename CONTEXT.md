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
`ToastStack.tsx`, `ContextMenu.tsx`, `PromptModal.tsx`, `icons.tsx`. The `pickQuickPlayEpisode` helper used by both `App.tsx` and
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
- **Search** (Ctrl+F) matches local title and linked AniList title; optional
  **Include filenames** checkbox adds episode file names (not paths); both search
  checkboxes persist in `localStorage`. Default mode:
  space-separated words are ANDed (any order); `|`, `||`, or uppercase `OR` split
  alternatives (lowercase `or` is a normal search word). **Regular expression** mode (off by default) replaces that syntax
  with a case-insensitive `RegExp` against the same fields. Results use the same
  grid sort dropdown (always visible; `animePlayer.searchGridSort` in
  `localStorage`, separate from category `animePlayer.animeGridSort`). Clear with the ×
  control or Esc (clears query first;
  Esc again leaves search). Index from `get_anime_search_index`; matching in
  `src/search.ts`.
- Bulk Edit can filter anime by source category and by a case-insensitive regex
  matched against full episode paths, then move the affected anime to another
  category. Its filename replacer tab scans all video files under configured
  root folders, so malformed files that are not yet detected as episodes can be
  renamed and then imported by the follow-up rescan.
- A **Jobs** sidebar page (between Settings and Rescan) lists queued/running
  background work and job history (two tabs, like Bulk Edit). The nav icon shows
  a badge with the count of queued + running jobs. **Max parallel jobs** (1–20,
  default 12, stored in SQLite `settings` as `jobs_max_parallel`) caps how many
  jobs may run at once for scheduling low/medium work (high-priority jobs count
  toward that cap but are never blocked by it—e.g. six low jobs at the limit can
  still be joined by a seventh high job). Jobs also have a **resource type**
  (`none` by default; scrub thumbnails use `ffmpeg`) with its own max-parallel
  limit (`jobs_max_parallel_type_ffmpeg`, default 1; `jobs_max_parallel_type_chroma`,
  default 12 for OP/ED pre-fingerprint jobs). On rotational volumes, new chroma jobs wait
  until `PercentDiskTime` for that drive is below 50% (one cached WMI sample for all drives,
  refreshed at most every 500ms on a background thread), with at least 500ms between starts
  on the same volume, and a 3s gap when busy cannot be read; the scheduler polls while deferred.
  SSD/NVMe paths skip deferral.
  Rotational classification uses `MSFT_PhysicalDisk.MediaType` via WMI (PowerShell fallback);
  unknown media type keeps deferral enabled. A job starts only when
  both caps allow it: low/medium respect `min(global, type)`; high bypasses the
  global cap but **not** the type cap. While a queued **medium** job can start,
  **low** jobs wait; a medium job that is blocked (prerequisites, caps, disk-busy
  deferral on HDD) does not hold low jobs back. Jobs may list **prerequisite job ids**;
  a queued job stays blocked until every prerequisite finishes successfully (failed
  or canceled prerequisites fail the dependent). While queued on prerequisites, the
  progress bar advances two steps per prerequisite (one when it starts, one when it
  completes). Each job has a short numeric `#id` in the UI. Jobs dedupe by `identity`,
  support cancel/cancel-all, progress steps, and emit `jobs://updated` (coalesced to
  at most ~4 per second) /
  `jobs://finished`. The root layout only subscribes to the active job count for the
  sidebar badge; the episode and jobs screens subscribe to the full snapshot (cached from
  the last `jobs://updated` when opening Jobs — no extra IPC). The Jobs page collapses
  large chroma queues into one summary row. Frontend
  helpers live in `src/jobs/jobClient.ts` (`subscribeJobsSnapshot`, `waitForJob`,
  `onJobIdentityFinished`). Workers run on `spawn_blocking` (off the WebView
  thread), but **must not** hold `AppDatabase::with_conn` across ffmpeg/fpcalc
  or other slow I/O—only brief SQLite reads/writes—so UI commands like
  `list_episodes` and `get_anilist_cover_image` stay responsive.
- The anime grid header includes a sort dropdown (alphabetical, most recent
  episode, last watched, episode/remaining counts); the choice is persisted in
  `localStorage` under `animePlayer.animeGridSort`. **Most recent** sorts by
  `anime.latest_episode_at` descending. That field is stored on `anime` and
  recomputed after every library rescan as `MAX(episodes.updated_at)` per show
  (`db::refresh_anime_latest_episode_at`). On each app start, if at least one
  root folder is configured, the app schedules `rescan_library` once after the
  initial `get_library_state` / auth / stats load and the first paint (idle
  callback with timeout via `scheduleAfterAppReady`) so the library UI is
  interactive before background scan work and job enqueue begin.
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
  the detected filesystem name. A **Hide AniList features** toggle
  (`hide_anilist_features` in SQLite `settings`) controls whether linking UI,
  the episode-page AniList banner, and score controls appear — independent of
  login state. Login uses the implicit OAuth flow with the
  `anime-player://anilist-auth` deep-link callback; `App.tsx` handles the
  callback, validates/stores the token through Rust, and refreshes auth state in
  the UI.
- Users can link titles and use AniList covers/metadata without logging in.
  Search, link, and public GraphQL reads (`episodes`, `status`, `meanScore`,
  `description`) work without a token; account features (progress sync, personal
  score, progress import) require login. Linked anime have search/link/unlink/open
  controls on the episode page when AniList features are not hidden.
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
- The AniList banner opens the linked AniList page from the title block only
  (cover + title + meta). When logged in it shows `Progress: current/total` and a
  debounced personal score input that writes back to AniList; when not logged in
  it shows `Episodes: total` and read-only community mean score instead. The
  synopsis from AniList fills the banner's middle column with a **Show more**
  control when clamped. AniList returns HTML markup (`<i>`, `<br>`, etc.); the UI
  sanitizes to a small tag whitelist before rendering. Cached media status includes
  `status` (e.g. `RELEASING`);
  **missing episode** counts and episode-list gap separators use AniList total
  episodes to fill holes in the local sequence, but skip trailing gaps while
  status is `RELEASING` so unreleased episodes are not flagged. Local watched progress syncs on EOF / near-end saves only
  when logged in and the adjusted local episode number is ahead of the viewer's
  current AniList progress. `persistProgress` in `PlayerView` awaits
  `syncAnilistEpisodeProgress` when leaving the player (Q/back, window close,
  flush-on-exit). EOF auto-advance, near-end **Next**, and other episode
  switches defer AniList to the background after the local SQLite save so
  `mpv_load` can start immediately (often in parallel with saving the next
  episode's position). Reopening an
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
- `html` / `body` / `#root` use **`var(--app-bg)`** by default (and a matching
  inline color in `index.html`) so the transparent Tauri window does not flash
  the desktop on startup. **`PlayerView`** toggles **`html.compositor-active`**
  only while the player is visible and **`videoCompositorRevealed`**, restoring
  transparent document roots for mpv DComp.
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
  auto-queues scrub jobs (low priority) on a background thread after the scan
  IPC returns (batched scheduler flush, same as episode-page scrub); opening an anime’s episode list uses one
  batched command (`jobs_enqueue_episode_page_scrub_sprites`, medium priority for
  uncached episodes) and downgrades them to **low** when leaving that page
  (`jobs_set_scrub_sprite_priority_for_paths`, one emit); opening
  the player upgrades the current file’s queued job to **high** (starts
  immediately). Queued priorities can be changed via `jobs_set_job_priority`.
  `get_scrub_sprite_if_ready` reads the
  cache synchronously; finished jobs emit `scrub-sprite-ready`. Generation uses
  ffmpeg with `-hwaccel auto` and `-skip_frame nokey`. The UI slices the sheet
  via CSS `background-position`.
- **OP/ED detection** is queued automatically like scrub sprites when
  `settings.auto_op_ed_detect` is enabled (default off): opening a title’s
  episode page may enqueue OP/ED work when needed. The setting applies only to
  titles with no matched skip timestamps yet; titles that already have matched
  OP/ED segments keep automatic follow-up (new episodes, staleness) even when
  the setting is off. Titles with manual skip templates use `anime_needs_manual_op_ed_rematch` instead
  of the auto-detect staleness check: rematch runs only when episodes lack segment
  rows (new imports) or custom templates changed without a follow-up rematch.
  Detect jobs (`op_ed_detect:{anime_id}`, resource type `none`) always enqueue at
  **high** priority so they start as soon as chroma prerequisites finish (including
  small-rescan auto-enqueue). A rescan that imports at most **20** new episodes
  (`RESCAN_AUTO_SCRUB_MAX`, shared with scrub) also enqueues detect per affected
  anime (detect high, chroma low). The worker (`jobs_enqueue_op_ed_detect`) scans
  cached Chromaprint fingerprints and writes templates plus per-episode OP/ED rows
  in SQLite (`op_ed_templates`, `episode_op_ed_segments`, `anime.no_op_ed`).
  Progress survives app restarts; re-running resumes from saved rows. Detect reuses
  cached fingerprints when present; otherwise it enqueues `op_ed_chroma` jobs
  (resource type `chroma`; ffmpeg + `fpcalc.exe`; fingerprints under
  `data/op-ed/fp-full`, `fp-part`, and `fp-custom`)
  as prerequisites. **Chroma** priority mirrors scrub: medium while browsing a
  title’s episode list (`jobs_set_op_ed_chroma_priority_for_anime`), low on leave,
  high for the episode open in the player (`jobs_enqueue_op_ed_chroma_for_episode`).
  Detect is
  For titles with **more than 12** episodes, detect runs in two jobs: a **preview**
  pass on the first 12 episodes (fast timestamps for early watching), then a **full**
  pass on every episode with the same block-detection logic as a single job on the
  full list (`rematch_matched` off; preview `matched` rows are demoted to `pending`
  first so discovery seeds from episode 1). Failed matches keep prior timestamps via
  SQL `COALESCE` on `start_sec`/`end_sec`. **12 or fewer** episodes
  use a single full pass. After `op_ed_analyzed_at` is set, newly imported episodes
  enqueue one full all-episode pass only. `op-ed://analysis-updated` fires after OP/ED
  detect or manual rematch jobs finish, coalesced to at most ~2 per second per title so
  parallel per-episode rematch work does not flood the WebView message queue. Auto-enqueue is skipped when analysis is already current
  (`op_ed_analyzed_at`, `ANALYSIS_VERSION`, segment rows for every episode).
  Chroma jobs that
  find an on-disk fingerprint at start finish immediately and release the HDD stagger
  gap (enqueue skips chroma when the cache file exists, using the same canonical path key as
  fingerprinting)
  gap so the next chroma can start without a 500ms wait. Chroma jobs fingerprint the full episode plus phase-1
  discovery windows (one ffmpeg decode per OP/ED search band, then isolated fpcalc per 15s window on that PCM). Detect
  loads cached windows for discovery; 90s templates use isolated segment fpcalc. Match candidates use the cached
  full-episode fingerprint. Phase-1 discovery must not slice from full Chromaprint (segment-boundary fpcalc differs);
  phase-2 refinement re-slices the 90s template from the cached reference fingerprint while sliding in unison. After coarse discovery, a **refinement pass**
  anchors the coarse template on phase-1 seed episodes (dropping failures), then slides ±8s at 0.5s steps and picks the shift with the highest average per-episode score; the refined start is
  kept only when its template matches at least as many batch episodes as the coarse template (and coarse already
  matches ≥2). Optimistic search windows run before a full-episode pass for
  failed episodes. Titles with multiple OP/ED sets in one folder (e.g. two seasons
  merged) use **block detection**: after three consecutive per-kind match failures,
  discovery re-runs on episodes not yet `matched` for that kind (`op_ed_templates.block_index`
  increments). `not_found` rows from an earlier block are retried; only `matched`/`skipped`
  episodes are excluded from seeds. At a season boundary, an episode with exactly one of
  OP/ED matched may get a **bridge** retro match for the other kind (search_pass `bridge` /
  `bridge_full`) without overriding an existing match. Matching
  is intentionally conservative: the best audio offset must clear both an
  average-score threshold and consistency checks (enough strong frames plus a
  lower-quartile floor) before it can mark an episode segment as matched, which
  reduces false-positive skips on episodes without OP/ED. Per-episode match
  fallbacks (after optimistic + full fail): trim ~3s from the template lead and
  retry (`trim_*` passes); for ED also trim ~3s from the tail and retry
  (`trim_both_*` passes); then for OP only a near-miss at offset ≤ ~2.5s with
  slightly relaxed gates (`edge_near`). The worker commits
  progress through many short `with_conn` calls while ffmpeg/fpcalc run unlocked.
  On completion coalesced `op-ed://analysis-updated` events reload that title
  via `list_episodes` plus `get_anime_op_ed_summary` only (not a full `get_library_state`).
  The episode page navigates immediately and shows a loading state while
  `list_episodes` runs; episode row thumbnails load with bounded concurrency.
  Title settings include **OP/ED analysis** with **Run analysis** (manual
  `jobs_enqueue_op_ed_detect`, not gated by `auto_op_ed_detect`) and **Reset**
  (clears auto-detected templates and segment rows; manual/custom templates and
  Chromaprint cache files are kept for faster rematch). Settings **Auto-detect anime openings/endings**
  toggles `settings.auto_op_ed_detect` for titles without matched skip timestamps;
  analyzed titles with existing timestamps are not gated by it. Global **Skip detected OP/ED**
  (`settings.skip_op_ed`, player **Skip OP/ED** toggle) seeks past matched
  segments on `mpv://time-pos`. **Don't skip first episode opening/ending**
  (`settings.dont_skip_first_episode_op_ed`) exempts display episode 1 from that
  skip logic.
- **Manual skip areas** (episode page header icon): navigates to a dedicated
  `manualSkip` view (`ManualSkipScreen`) that hides the library shell like the
  player so mpv can composite through the WebView. Users define per-title OP/ED
  templates by selecting a source episode and dragging range handles over an mpv
  preview (`mpv_set_preview_rect` / four `video-margin-ratio-*` options). Templates
  live in `op_ed_templates` with `source = 'manual'` (5–180s variable duration,
  Chromaprint fingerprint per template). While any manual template exists, skip
  matching uses **manual templates only** (first match in template list order per
  kind, tested in parallel; `search_pass = 'manual'`); auto templates remain in SQLite but are
  inactive. Leaving the screen after changes clears segment rows and enqueues one
  `manual_op_ed_rematch:{anime_id}:{episode_id}` job per episode (medium priority;
  each waits only on that episode's chroma prerequisite). Deleting all manual templates and leaving rematches against auto templates only.
  **Run analysis** with manual templates enqueues chroma for episodes missing a
  full-episode fingerprint, then one rematch job per episode plus a summary shell job
  (`manual_op_ed_rematch:{anime_id}`, type `manual_op_ed_rematch_summary`) that lists every
  episode rematch as a prerequisite for the episode-page progress bar; the shell finishes
  immediately when all episode jobs complete. Clears existing segment rows first so every
  episode is rematched. Results stream in per episode as each job finishes — no need to
  wait for all chroma jobs before the first timestamps appear.
  Missing custom template `.fp` files are regenerated from the stored source episode and range before matching. Chromaprint cache files
  live under `data/op-ed/fp-full` (full episodes), `fp-part` (discovery
  windows and auto templates), and `fp-custom` (manual template extracts). Opening the
  screen stops mpv playback; `PlayerView` sets `playbackSuspended` until the view
  closes. The list view has **Custom templates** and, when any episode lacks a
  `matched` segment, **Missing segments** (OP/ED columns). The source picker marks
  episodes missing that kind at lower opacity. New templates pre-fill the scrubber
  from auto-detected `matched` segments (`search_pass != 'manual'`) when present.
  **Escape** steps back (editor → list, picker → list, list → episodes). In the
  range editor, **ArrowLeft/ArrowRight** nudge the start handle by one frame
  (Shift: five frames); **Ctrl+ArrowLeft/ArrowRight** nudge the end handle the
  same way. **Enter** saves, **Space** runs Test, **Ctrl+Space** plays the
  selected area; scroll wheel adjusts volume like the player. Frame step buttons
  repeat after a 200ms hold delay. Stored volume is applied when mpv initializes
  and after each file load so preview playback does not briefly use mpv defaults.
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
  mpv and returns to the episode list. Natural EOF uses `mpv://eof-reached`.
  Seeks (hotkeys or scrubber release) advance proactively when the target
  lands within one second of EOF or past it — without waiting on mpv EOF
  properties. A short poll loop retries natural EOF advance if `mpv_load`
  is slow.
  When advancing to the next episode
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
  `AppDatabase::with_conn` exposes `&mut Connection` behind one process-wide
  `Mutex` (all Tauri commands and background jobs share it). Keep critical
  sections short; never run ffmpeg, shell thumbnails, or Chromaprint inside
  `with_conn`. Transactions (e.g. bulk rescan) still use `with_conn` but
  should not call into media tools while the lock is held.
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
  (new or updated paths), scrub and OP/ED auto-enqueue run on a background
  thread after the command returns (one batched scheduler flush). `rescan_library` commits one SQLite transaction per
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
  covers and scrub sprites (matched by episode path keys in sprite metadata).
  When `settings.clean_unused_scrub_sprites` is enabled (default on), cleanup
  also removes scrub sprites for titles whose last watch date — or date added
  when never watched — is more than three months ago,
  unused OP/ED fingerprint caches, and leftover OP/ED fpcalc staging files
  (`*.s16le` under `data/op-ed/`, older than one day). The episode page's `delete_anime_files`
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
    `hwdec=auto-safe`, `keep-open=yes`, `keep-open-pause=yes`, `osc=no`,
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
  `mpv_get_time_pos()`, `mpv_get_playback_end_state()`, `mpv_set_volume(volume)`, and `mpv_stop()`.
- `scrub_preview.rs` — sprite cache I/O and ffmpeg generation;
  `get_scrub_sprite_if_ready_cmd`, `scrub_sprite_is_cached_cmd`.
- `jobs/` — `JobManager` in `AppState`, scheduler (priority, parallel limit,
  dedupe), scrub-sprite worker; commands `jobs_get_snapshot`, `jobs_enqueue_scrub_sprite`,
  `jobs_enqueue_episode_page_scrub_sprites`, `jobs_set_job_priority`,
  `jobs_set_scrub_sprite_priority_for_paths`, `jobs_cancel`,
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
  `libs/mpv/libmpv-2.dll`, `libs/ffmpeg/ffmpeg.exe`,
  `libs/ffmpeg/ffprobe.exe`, and `libs/chromaprint/fpcalc.exe` next to
  the exe.

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

## Diagnostics

Startup issues, Rust panics, Windows native faults (e.g. libmpv / ffmpeg),
and frontend `window.onerror` / unhandled rejections append to a portable log
at **`data/diagnostic.log`** next to the executable (rotates to
`diagnostic.log.old` when it exceeds 1 MiB). Ask users to attach that file
when reporting crashes. The frontend writes breadcrumbs such as
`startup: rescan_library begin` so the last line often pinpoints how far boot
got. Set `RUST_BACKTRACE=1` before launch for richer panic stacks.

## Files to know

- `README.md` — user-facing setup + run + project layout.
- `TODO.md` — running list of follow-ups.
- `CONTEXT.md` — this file.
- `src/api.ts`, `src/types.ts`, `src/utils.ts` — frontend command
  bindings, shared DTOs, and formatting helpers.
- `src/components/PlayerView.tsx` — mpv-backed player view and controls.
- `src/components/ManualSkipScreen.tsx`, `src/components/TemplateRangeScrubber.tsx`
  — manual OP/ED template editor UI.
- `src/components/{CategoryScreen,AnimeGrid,EpisodeScreen,SettingsScreen,WindowTitleBar,ViewHeader,CustomDropdown,ContextMenu,PromptModal,ToastStack,icons}.tsx`
  — split-out view components composed by `App.tsx`.
- `src/quickPlay.ts` — Q-hotkey "next episode to play" picker.
- `src/volume.ts` — volume constants and clamping for
  mpv's native 0–130 log scale.
- `src-tauri/src/db.rs`, `src-tauri/src/library.rs`,
  `src-tauri/src/scanner.rs` — portable SQLite, library commands, and
  regex scanner.
- `src-tauri/src/op_ed.rs`, `src-tauri/src/media_tools.rs` — OP/ED detection
  and shared ffmpeg/ffprobe helpers.
- `src/opEd.ts` — OP/ED job identity and episode-page progress helpers.
- `src-tauri/src/anilist.rs` — AniList OAuth callback handling, GraphQL
  search/link commands, and local cover caching.
- `scripts/download-mpv-libs.mjs` — setup entry point that downloads
  ignored local libmpv artifacts into `src-tauri/libs/mpv/`.
- `scripts/update-mpv-libs.mjs` — compatibility updater for the same
  artifacts from the latest `shinchiro/mpv-winbuild-cmake` release.
- `scripts/download-ffmpeg.mjs`, `scripts/download-chromaprint.mjs` —
  setup entry points for ignored local `ffmpeg.exe`/`ffprobe.exe` and
  Chromaprint `fpcalc.exe` artifacts.
- `scripts/release-notes.mjs` and `scripts/publish-github-release.mjs` — automated
  GitHub release flow (run `npm run release:notes` then `npm run release:publish`).
- `scripts/update.bat` and `scripts/_update.ps1` — portable end-user
  updater (`update.bat` only; `_update.ps1` is internal). Shipped in the
  release zip; downloads `anime-player.exe` from GitHub `releases/latest`.
- `temp/` — gitignored scratch space for agent HTML UI mockups (see
  `.cursor/rules/design-documents.mdc`).
- `.cursor/rules/` — agent rules. `read-context.mdc` points new agents
  here; `commit-checkpoints.mdc` defines the per-change commit
  workflow; `design-documents.mdc` covers `temp/` HTML mockups.

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

# Download / refresh media helper tools
npm run setup:ffmpeg
npm run setup:chromaprint

# Regenerate Tauri icons from a square SVG or PNG
npm run icon:setup -- path\to\icon.svg

# Run the app (compiles Rust, launches Vite, opens the window)
npm run tauri dev

# Production build (installer in src-tauri/target/release/bundle/)
npm run tauri build

# Portable release: `tauri build` then versioned folder + zip under `releases/`
# (that directory is listed in `.gitignore` so build artifacts stay local).
# Package includes libmpv-2.dll, ffmpeg.exe, ffprobe.exe, fpcalc.exe,
# update.bat, _update.ps1, and VERSION.txt (release only). Scrub thumbnails
# and OP/ED detection prefer bundled tools beside the exe; PATH is a fallback
# if those files are removed.
# Publish GitHub release with the zip, standalone anime-player.exe, and
# anime-player.exe.sha256.
npm run release

# Local dev portable only (no tag, zip, or publish artifacts): `releases/dev/`
npm run portable
```

A full `cargo check` from cold is ~25s on the user's machine; warm is <2s.
