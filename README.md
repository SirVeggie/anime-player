# Anime Player

A minimal local video player built with Tauri v2 + React + TypeScript. Point it at a folder, browse all video files inside it (recursively), and click any file to play it. Playback is provided by **libmpv** loaded in-process from `libmpv-2.dll` — there is no separate `mpv.exe` and no popup window.

## Features

- Type or browse a folder path; all video files inside (and inside its subfolders) are listed.
- Click a file to play it instantly via in-process libmpv — true lossless decoding for any container/codec mpv supports (MKV with FLAC/Opus, HEVC, AV1, PGS/ASS subtitles, etc.).
- The video composites with the rest of the UI inside a single window: window-mover scripts and per-window audio mixers see the player as one app.
- Custom HTML controls bar (play/pause, scrubber, time) on top of the video, plus standard mpv keyboard shortcuts (Space pause, ←/→ seek, F fullscreen, M mute, etc.).
- Quick filter to find a file in a large library.
- Recognized extensions: `mkv`, `mp4`, `m4v`, `mov`, `avi`, `wmv`, `flv`, `webm`, `ts`, `m2ts`, `mts`, `ogv`, `ogm`, `vob`, `3gp`, `rm`, `rmvb`, `mpg`, `mpeg`.

## Prerequisites

You need the following installed:

1. **Node.js 18+** (already detected: `npm` available).
2. **Rust** (stable toolchain) – install from <https://rustup.rs>.
3. **Microsoft Visual Studio C++ Build Tools** (Desktop development with C++ workload) – required by Tauri on Windows.
4. **WebView2 Runtime** – preinstalled on Windows 11 / recent Windows 10 builds.

You do **not** need a separate `mpv.exe` install. `libmpv-2.dll` is downloaded into `src-tauri/libs/mpv/` and bundled with the installer.

See the official Tauri prerequisites: <https://tauri.app/start/prerequisites/>.

## Install

```powershell
npm install
npm run setup:mpv
```

## Run (development)

```powershell
npm run tauri dev
```

The first run will compile the Rust side, which takes a couple of minutes. Subsequent runs are fast. The build script copies `libmpv-2.dll` next to the dev binary automatically.

## Build (production)

```powershell
npm run tauri build
```

Outputs an installer/executable in `src-tauri/target/release/bundle/`. `libmpv-2.dll` is shipped alongside via `tauri.conf.json`'s `bundle.resources`.

## Downloading / updating libmpv

The generated `libmpv-2.dll` and `mpv.lib` come from `shinchiro/mpv-winbuild-cmake` releases. They are intentionally ignored by git, so run this after cloning or whenever you want to refresh them:

```powershell
npm run setup:mpv
```

This downloads the latest dev bundle, extracts `libmpv-2.dll` and `libmpv.dll.a` (renamed to `mpv.lib` for MSVC), and writes a `VERSION.txt` recording which release was installed. Requires `7z` on PATH (e.g. `scoop install 7zip`).

## Project layout

- `src/` – React + TypeScript frontend (Vite).
  - `App.tsx` – folder input, file list, transparent player pane, custom controls.
- `src-tauri/` – Rust backend.
  - `src/lib.rs` – exposes `scan_videos` plus `mpv_*` commands.
  - `src/mpv/` – in-process libmpv module: `ffi.rs` (FFI declarations), `handle.rs` (`MpvHandle` and lifecycle), `event_loop.rs` (property observers → `mpv://*` Tauri events).
  - `libs/mpv/` – local generated `libmpv-2.dll` + `mpv.lib`.
  - `build.rs` – tells Cargo to link against `mpv` and copies the DLL beside the dev binary.
  - `tauri.conf.json` – sets `transparent: true` on the main window and ships `libmpv-2.dll` as a bundle resource.
- `scripts/download-mpv-libs.mjs` – setup entry point for local libmpv artifacts.
- `scripts/update-mpv-libs.mjs` – compatibility updater for the same artifacts.

## How playback works

The Tauri main window has `"transparent": true`. On the first file selection the frontend calls `invoke("mpv_init", ...)`, which:

1. `mpv_create()`s a libmpv context in the Tauri process.
2. Sets the `wid` option to the Tauri main `HWND` (so libmpv embeds into our window) plus `vo=gpu-next`, `gpu-context=d3d11`, `hwdec=auto-safe`, `osc=no`.
3. `mpv_initialize()`s the context. libmpv creates its own DirectComposition swap-chain under our HWND.
4. Spawns a background thread that observes `time-pos`, `duration`, `pause`, `eof-reached` and republishes each property change as a Tauri event (`mpv://time-pos`, etc.).

Because libmpv's swap-chain and WebView2's visual both compose under the same HWND in the same DWM tree, the video shows through anywhere the React UI is CSS-transparent (the `.player` pane). The sidebar paints opaquely on top. We confine the video to the right pane by setting `video-margin-ratio-left = sidebar_px / window_width`, which we re-issue on every window resize.

Loading another file is just `mpv_load(path)` (which fires `loadfile`). Switching to a custom controls UI was a primary motivator for this approach: drawing HTML over a transparent CSS region is much easier than drawing it over a separate top-level popup HWND.

For the historical popup-window architecture and why the in-process libmpv approach replaced it, see `CONTEXT.md`.
