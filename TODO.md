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

- [ ] Use the regex pairs mentioned below to filter out all anime files, then extract the anime titles, and display all the episodes of the anime with an identical name under the same page as a list, with each item looking like this [thumbnail from file | Episode x | filetype]
- [x] ~~Add a category screen, where user can select a category of anime (ongoing, completed, finished)~~
    - [x] ~~The user should be able to add or delete categories~~
    - [x] ~~One of the categories should be the default category for new anime~~
    - [x] ~~The user should be able to move anime to a new category~~
- [x] ~~Anime grid should have vertical orientation thumbnails, with anime titles (at least as far as they fit on one line)~~
    - [x] ~~Hovering shows the full title, episode progress and available episodes in a custom tooltip~~
- [ ] Custom window title bar for clean look, which can fade out when watching a video.
- [ ] Custom window border (clean look without the classic 1px border seen in windows 11, not a priority)
- [x] ~~Settings view~~
    - [x] ~~Ability to add multiple root folders for anime~~
    - [ ] Add anime detection pairs with regex. One regex checks if a filename is an anime file, the other regex extracts the anime title from the filename.
- [x] ~~Save data, settings, categories, etc. in SQLite database in the same folder as the executable.~~
- [x] ~~Below categories on the main screen, you can quickly continue where you left off for the last 5 animes watched~~
    - [ ] Select how many are displayed in settings
- [ ] Sorting options on the anime grid page: alphabetical, date added, last watched, total episodes, unwatched episodes

Episode selection page features
- [x] ~~Show anime title, current progress~~
- [x] ~~Add slight tint to already watched episodes~~
- [ ] Show thumbnail also on this page (if available)
- [ ] Delete episode
- [ ] Delete anime (aka delete all episodes and database entries)
- [ ] Unset watched status on episode

## Video Player

- [ ] Video player view takes full size of the window
- [ ] Video controls fade in and out. Video controls only visible if mouse is moved, but not when hotkeys pressed
- [x] ~~Filename is displayed in the center of the top part instead of the left~~
- [ ] Exit video is moved to the top left with a back arrow symbol
- [ ] Audio track selection
- [ ] Subtitle track selection
- [x] ~~Video player remembers the progress of the last played episode for each anime~~
- [ ] Q key will toggle between the episode list and the video player
- [x] ~~The last played anime episode is highlighted on the episode select screen~~

## AniList Support

This will be added last, once the local functionality is working well

- [ ] Login with anilist
- [ ] Link each anime with an ID from anilist using SQLite
- [ ] Fetch anime thumbnail from anilist and save it locally
- [ ] When an episode is finished, update the progress on anilist by parsing the episode number from the filename
- [ ] Add on an anime page to open the anilist page, or if it's not linked, start the linking process
- [ ] Add linking process, where the anime title is used to fetch the best matches from anilist, and the user can pick the correct one
- [ ] Add button to unlink the anime from anilist if linked