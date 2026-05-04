# TODO

Running list of things to do / explore for the Anime Player app.

## General UI

- [ ] Support custom thumbnail files (image with same name as parsed anime title)
    - [ ] Rename feature also renames custom thumbnail
- [ ] Combine detection regexes into one, we probably only need the second one?
    - Could have separate regex for title and episode number, then wouldn't need duplicate rules if episode doesn't match

Episode selection page features
- [x] ~~Show anime title, current progress~~
- [x] ~~Add slight tint to already watched episodes~~
- [x] ~~Show thumbnail also on this page (if available)~~
- [x] ~~Delete anime files from the episode page, mark rows missing, and leave database cleanup to Settings~~
- [ ] Delete episode (maybe context menu)

## Page transitions

- [ ] Add a smooth transition between pages (sidebar tab change, opening a category / episode view).
    - Cross-fade or slight slide+fade on the main content when switching views.
    - Stretch goal: stagger grid items in (cards fade/slide in with a small delay between siblings) when entering a category or episode list.

## Video Player

- [ ] Show preview when hovering the scrubber
- [ ] Continuously seek when dragging the scrubber handle (works great in mpv, here could have performance issues)

## Keybindings

- [ ] Add keybind setting support (as a separate page)
- [ ] Ability to change the existing keyboard shortcuts
- [x] Add keybind for skipping 28 seconds (forwards and backwards).
- [x] Add keybind for skipping 85 seconds (forwards and backwards).

# Performance

- [ ] Improve thumbnail loading speed
