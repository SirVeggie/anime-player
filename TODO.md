# TODO

Running list of things to do / explore for the Anime Player app.

## Improved UI

Current UI is almost completely replaced with a new more comprehensive UI.
The new UI consists of multiple different views, and the idea is to be able to browse local animes and play them with a clean UI.

UI flow:
User opens app -> category selection -> display animes in category as a grid of thumbnails -> Anime episode selection and extras -> video player

Note:
When building the new interface, try to build it in a way that is relatively easy to modify later, with css variables at least for colors. The UI design is still volatile, but at least the video player style is quite nice already.

Example anime filename: [SubsPlease] Sousou no Frieren S2 - 09 (1080p) [A3A99C65].mkv

- [x] ~~Use the regex pairs mentioned below to filter out all anime files, then extract the anime titles, and display all the episodes of the anime with an identical name under the same page as a list~~
- [x] ~~Add a category screen, where user can select a category of anime (ongoing, completed, finished)~~
    - [x] ~~The user should be able to add or delete categories~~
    - [x] ~~One of the categories should be the default category for new anime~~
    - [x] ~~The user should be able to move anime to a new category~~
- [x] ~~Anime grid should have vertical orientation thumbnails, with anime titles (at least as far as they fit on one line)~~
    - [x] ~~Hovering shows the full title, episode progress and available episodes in a custom tooltip~~
- [x] ~~Custom window title bar for clean look, which can fade out when watching a video.~~
- [x] ~~Custom window border (clean look without the classic 1px border seen in windows 11, not a priority)~~
- [x] ~~Settings view~~
    - [x] ~~Ability to add multiple root folders for anime~~
    - [x] ~~Add anime detection pairs with regex. One regex checks if a filename is an anime file, the other regex extracts the anime title from the filename.~~
- [x] ~~Save data, settings, categories, etc. in SQLite database in the same folder as the executable.~~
- [x] ~~Below categories on the main screen, you can quickly continue where you left off for the last 5 animes watched~~
    - [ ] Select how many are displayed in settings
- [x] ~~Sorting options on the anime grid page: alphabetical, date added, last watched, total episodes, unwatched episodes~~

Episode selection page features
- [x] ~~Show anime title, current progress~~
- [x] ~~Add slight tint to already watched episodes~~
- [x] ~~Show thumbnail also on this page (if available)~~
- [x] ~~Delete anime files from the episode page, mark rows missing, and leave database cleanup to Settings~~
- [ ] Delete episode

## Page transitions

- [ ] Add a smooth transition between pages (sidebar tab change, opening a category / episode view).
    - Cross-fade or slight slide+fade on the main content when switching views.
    - Stretch goal: stagger grid items in (cards fade/slide in with a small delay between siblings) when entering a category or episode list.

## Video Player

- [x] ~~Video player view takes full size of the window~~
- [x] ~~Video controls fade in and out. Video controls only visible if mouse is moved, but not when hotkeys pressed~~
- [x] ~~Filename is displayed in the center of the top part instead of the left~~
- [x] ~~Exit video is moved to the top left with a back arrow symbol~~
- [x] ~~Audio track selection~~
- [x] ~~Subtitle track selection~~
- [x] ~~Video player remembers the progress of the last played episode for each anime~~
- [x] ~~Q key will toggle between the episode list and the video player~~
- [x] ~~The last played anime episode is highlighted on the episode select screen~~
- [x] ~~Fit aspect ratio button. Pressing this will resize the window to match the aspect ratio of the video, so the are no black bars.~~

## Keybindings

- [ ] Add keybind setting support (as a separate page)
- [ ] Ability to change the existing keyboard shortcuts
- [ ] Add keybind for skipping 28 seconds (forwards and backwards).
- [ ] Add keybind for skipping 85 seconds (forwards and backwards).

## AniList Support

This will be added last, once the local functionality is working well

- [x] ~~Login with anilist~~
- [x] ~~Link each anime with an ID from anilist using SQLite~~
- [x] ~~Fetch anime thumbnail from anilist and save it locally~~
- [x] ~~When an episode is finished, update the progress on anilist by parsing the episode number from the filename~~
    - Note: If the episode is lower than the current tracked episode, then do nothing.
- [x] ~~Add on an anime page to open the anilist page, or if it's not linked, start the linking process~~
- [x] ~~Add linking process, where the anime title is used to fetch the best matches from anilist, and the user can pick the correct one~~
- [x] ~~Add button to unlink the anime from anilist if linked~~