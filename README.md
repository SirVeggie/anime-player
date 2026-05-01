# Anime Player

A minimal local video player built with Tauri v2 + React + TypeScript. Point it at a folder, browse all video files inside it (recursively), and click any file to play it.

## Features

- Type or browse a folder path; all video files inside (and inside its subfolders) are listed.
- Click a file to play it instantly via an embedded **mpv** process — true lossless decoding for any container/codec mpv supports (MKV with FLAC/Opus, HEVC, AV1, PGS/ASS subtitles, etc.).
- Standard mpv keyboard shortcuts work inside the player area (Space pause, ←/→ seek, F fullscreen, M mute, etc.) and the on-screen controller appears on hover.
- Quick filter to find a file in a large library.
- Recognized extensions: `mkv`, `mp4`, `m4v`, `mov`, `avi`, `wmv`, `flv`, `webm`, `ts`, `m2ts`, `mts`, `ogv`, `ogm`, `vob`, `3gp`, `rm`, `rmvb`, `mpg`, `mpeg`.

## Prerequisites

You need the following installed:

1. **Node.js 18+** (already detected: `npm` available).
2. **Rust** (stable toolchain) – install from <https://rustup.rs>.
3. **Microsoft Visual Studio C++ Build Tools** (Desktop development with C++ workload) – required by Tauri on Windows.
4. **WebView2 Runtime** – preinstalled on Windows 11 / recent Windows 10 builds.
5. **mpv** – the player shells out to `mpv.exe` on your `PATH`. Install one of:
   - `scoop install mpv`
   - `choco install mpv`
   - or download from <https://mpv.io> and add the folder to `PATH`.

See the official Tauri prerequisites: <https://tauri.app/start/prerequisites/>.

## Install

```powershell
npm install
```

## Run (development)

```powershell
npm run tauri dev
```

The first run will compile the Rust side, which takes a couple of minutes. Subsequent runs are fast.

## Build (production)

```powershell
npm run tauri build
```

Outputs an installer/executable in `src-tauri/target/release/bundle/`.

## Project layout

- `src/` – React + TypeScript frontend (Vite).
  - `App.tsx` – folder input, file list, and the mpv host pane.
- `src-tauri/` – Rust backend.
  - `src/lib.rs` – exposes the `scan_videos` command plus `mpv_*` commands.
  - `src/mpv.rs` – spawns `mpv.exe` into a Win32 child window and talks to it via JSON IPC over a named pipe.
  - `tauri.conf.json` – enables the asset protocol (`assetProtocol.scope = ["**"]`).
  - `capabilities/default.json` – grants `dialog` permission for the native folder picker.

## How playback works

The frontend renders an empty `<div class="mpv-host" />` where the video pane should be. On the first file selection, Rust creates an **owned top-level popup window** (`WS_POPUP`, owner = the Tauri main HWND) and positions it in screen coordinates over that div, then spawns:

```text
mpv.exe --wid=<popup_hwnd>
        --idle=yes --force-window=yes --no-terminal
        --keep-open=yes --osc=yes
        --input-default-bindings=yes
        --input-ipc-server=\\.\pipe\anime-player-mpv
```

Why an owned popup instead of a `WS_CHILD` window inside the Tauri HWND? WebView2 renders its content via DirectComposition, and a composited surface always paints on top of regular GDI child windows in the same parent — regardless of Win32 z-order. Hosting mpv in a top-level popup window sidesteps this: top-level windows compose at the desktop level, so the video sits cleanly above the WebView. The popup is "owned" by the main window, so it minimizes/restores with the owner and is destroyed when the owner closes.

The frontend reports CSS-pixel rects from a `ResizeObserver`; Rust scales them by the window's DPI factor, projects them into screen coordinates with `ClientToScreen`, and `SetWindowPos`'s the popup. The Tauri `Moved` and `Resized` window events also trigger a re-projection so the popup follows when the user drags or resizes the main window. Loading a new file is just a `{"command":["loadfile", path]}` JSON line written to the IPC pipe.

Subtitles, audio track switching, seeking, fullscreen, etc. all work via mpv's built-in OSC and key bindings. A custom HTML controls bar driven from React is on the TODO list.
