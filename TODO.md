# TODO

Running list of things to do / explore for the Anime Player app.

## Playback

- [x] ~~Embed mpv for true lossless playback.~~ Done via `mpv.exe` subprocess + Win32 child HWND + JSON IPC. See `src-tauri/src/mpv.rs`.
- [ ] Bundle a pinned `mpv.exe` with the installer so users don't have to install mpv separately. (Today the app shells out to `mpv` on PATH.)
- [ ] Build a custom HTML controls bar (play/pause, scrubber, volume, audio/sub track menus) that drives mpv via the existing IPC pipe instead of relying on mpv's OSC.
- [ ] Read mpv events from the IPC pipe (`time-pos`, `pause`, `playlist-pos`, `track-list`) so the React UI can display state.
- [ ] Persist last-played folder + last position per file (resume playback).
- [ ] Re-issue `mpv_set_rect` on `ScaleFactorChanged` (window dragged across monitors with different DPI).
