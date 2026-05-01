# TODO

Running list of things to do / explore for the Anime Player app.

## Playback

- [x] ~~Embed mpv for true lossless playback.~~ Done via in-process `libmpv-2.dll` + the Tauri main HWND as the render target. See `src-tauri/src/mpv/`.
- [x] ~~Read mpv events from the IPC pipe.~~ Replaced by libmpv property observers; `time-pos`, `duration`, `pause`, `eof-reached` are emitted as `mpv://<property>` Tauri events.
- [x] ~~Bundle mpv with the app so users don't need to install it separately.~~ `libmpv-2.dll` is committed under `src-tauri/libs/mpv/` and shipped via `tauri.conf.json` `bundle.resources`. Refresh with `node scripts/update-mpv-libs.mjs`.
- [ ] Expand the custom HTML controls bar: audio/sub track menus, speed control, volume slider. Foundation (play/pause, scrubber, time) is in place.
- [ ] Persist last-played folder + last position per file (resume playback).
- [ ] Re-issue `mpv_set_layout` on `ScaleFactorChanged` (window dragged across monitors with different DPI).
- [ ] Confirm the libmpv DComp swap-chain composes correctly across all WebView2 versions / Windows builds the user runs.
