# Anime Player

Anime Player is a Windows desktop app for browsing and playing a local video library. It scans root folders recursively, groups matching filenames into titles and episodes, tracks local watch progress, and plays files through in-process **libmpv** so the UI and video behave like one app window.

![Home screen](assets/screenshots/homescreen.png)
![Category view](assets/screenshots/category.png)
![Episode view](assets/screenshots/episodes.png)
![Player](assets/screenshots/player.png)

## Features

- Local library scanner with editable categories, missing-file diagnostics, and regex-based filename detection rules.
- In-process libmpv playback for common containers/codecs including MKV, MP4, HEVC, AV1, FLAC/Opus audio, ASS subtitles, and more.
- Custom player controls for play/pause, seeking, audio/subtitle track selection, aspect fit, fullscreen, and episode navigation.
- Continue Watching, search, bulk category editing, bulk filename replacement, local progress tracking, and optional AniList linking/progress sync.
- Portable local data beside the app executable in `data/anime-player.db`.

Supported video extensions: `mkv`, `mp4`, `m4v`, `mov`, `avi`, `wmv`, `flv`, `webm`, `ts`, `m2ts`, `mts`, `ogv`, `ogm`, `vob`, `3gp`, `rm`, `rmvb`, `mpg`, `mpeg`.

## Installation

Prerequisites:

1. Node.js 18+.
2. Rust stable from <https://rustup.rs>.
3. Microsoft Visual Studio C++ Build Tools with the Desktop development with C++ workload.
4. WebView2 Runtime, which is preinstalled on current Windows 10/11 installs.
5. `7z` on PATH for the mpv setup script.

Install dependencies and download the local libmpv artifacts:

```powershell
npm install
npm run setup:mpv
```

You do not need a separate `mpv.exe` install. The setup script downloads `libmpv-2.dll` and `mpv.lib` into `src-tauri/libs/mpv/`; these generated files are intentionally not committed.

## Running

```powershell
npm run tauri dev
```

The first Rust build can take a few minutes. Later dev runs are much faster.

To build an installer:

```powershell
npm run tauri build
```

To build and package the portable release folder/zip:

```powershell
npm run release
```

## Basic Usage

1. Open Settings and add one or more root folders.
2. Rescan the library.
3. Open a category, pick a title, then choose an episode to play.
4. Adjust categories, detection rules, bulk edits, and cleanup from Settings/Bulk Edit as needed.

The default detection rules cover common fansub-style names, simple `Title - 01` names, and generic video filenames. Detection rules are evaluated by priority, highest first:

- `Fansub`, priority `10`
- `Fansub (no ep)`, priority `9`
- `Simple`, priority `5`
- `Simple (no ep)`, priority `4`
- `Generic`, priority `0`

## AniList

AniList support is optional. The app includes a default AniList OAuth client ID (`40455`), so most users can open Settings and press **Login with AniList** without changing anything.

If you want to use your own AniList API client, enter its client ID in Settings. Configure the AniList app redirect URL as:

```text
anime-player://anilist-auth
```

After login, you can link local titles to AniList entries, import watched progress, sync completed episodes, and adjust AniList scores from the title page.

## Keybindings

Global:

- `F11`: Toggle app fullscreen, unless typing in a text field.
- `Ctrl+F`: Open/focus Search.
- `Esc`: Go back from category, search, bulk edit, missing, episode, or settings pages.
- `Arrow keys`: Move focus through category/title card grids.

Episode page:

- `Q`: Quick play the current title. It resumes the most recently played episode, advances to the next unwatched episode if that one is watched, or starts the first unwatched episode.

Player:

- `Space`: Play/pause.
- Right click: Play/pause.
- `ArrowLeft` / `ArrowRight`: Seek backward/forward 5 seconds.
- `Numpad4` / `Numpad6`: Seek backward/forward 28 seconds.
- `Numpad7` / `Numpad9`: Seek backward/forward 85 seconds.
- `Ctrl+ArrowLeft` / `Ctrl+ArrowRight`: Previous/next episode.
- `F`: Toggle fullscreen.
- `F11`: Toggle app fullscreen.
- `C`: Toggle player controls.
- `Q` or `Esc`: Leave the player and return to the episode list.
- Double left click on the video: Toggle fullscreen.
- Single left click and drag on the video: Drag the window.

libmpv keyboard input is also enabled in the player area, so standard mpv bindings may work in addition to the custom shortcuts above.

## Development Notes

The frontend is React + TypeScript under `src/`. The backend is Rust/Tauri under `src-tauri/`. Playback is handled by libmpv loaded in the Tauri process and rendered into the main window via DirectComposition; see `CONTEXT.md` for the detailed architecture notes.

Useful checks:

```powershell
npx tsc --noEmit
cargo check --manifest-path src-tauri/Cargo.toml
```
