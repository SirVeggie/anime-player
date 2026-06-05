# TODO

Running list of things to do / explore for the Anime Player app.

## General UI

- [ ] Custom tooltips (replace native `title` tooltips on settings checkboxes and elsewhere)
- [ ] Add button on home page to add a new category (popup)
- [ ] Reorder categories on home page by dragging
- [ ] Combine detection regexes into one, we probably only need the second one?
    - Could have separate regex for title and episode number, then wouldn't need duplicate rules if episode doesn't match
- [x] Natural window close saves current episode progress to SQLite (`onCloseRequested`); crashes / kill still skip save
- [ ] Background gradient effect does not scale with window size, so it looks weird above around 1280x800

Episode selection page features
- [x] ~~Show anime title, current progress~~
- [x] ~~Add slight tint to already watched episodes~~
- [x] ~~Show thumbnail also on this page (if available)~~
- [x] ~~Delete anime files from the episode page and clear related local data~~
- [ ] Delete episode (maybe context menu)

## Page transitions

- [ ] Add a smooth transition between pages (sidebar tab change, opening a category / episode view).
    - Cross-fade or slight slide+fade on the main content when switching views.
    - Stretch goal: stagger grid items in (cards fade/slide in with a small delay between siblings) when entering a category or episode list.

## Library / scanning

- [ ] Automatically detect new files via filesystem events (instead of manual rescan only)
- [ ] Group seasons of the same show (subfolders? or fuzzy title match — low relative edit distance vs title length)
- [x] Auto-detect OP/ED by finding common audio signatures across episodes (manual **Detect OP/ED** per title)
    - [x] Skip detected regions automatically (global **Skip OP/ED** setting + player toggle)
    - [ ] **Full-episode OP/ED discovery fallback** — discovery only scans the first/last 180s (same as optimistic matching); matching can search the full file, but if seed discovery fails there is no full-timeline retry. Add a fallback pass (e.g. slide 15s windows across the whole episode) for titles that change OP/ED often or put OP in unusual places (e.g. end of episode). Not implemented yet.
    - [x] Option in settings to not skip on the first episode, so you get to watch them once
- [ ] View that lists all titles at once

## Video Player

- [x] ~~Show preview when hovering the scrubber~~
- [x] ~~Continuously seek when dragging the scrubber handle~~

## Keybindings

- [ ] Add keybind setting support (as a separate page)
    - [ ] Ability to change the existing keyboard shortcuts
- [x] Add keybind for skipping 28 seconds (forwards and backwards).
- [x] Add keybind for skipping 85 seconds (forwards and backwards).

# Performance

- [ ] Improve thumbnail loading speed

## Reliability

- [x] Portable `data/diagnostic.log` for startup breadcrumbs, panics, and native/JS faults
