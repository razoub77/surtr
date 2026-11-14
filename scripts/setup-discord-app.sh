Configure a Discord application for Surtr, and confirm it renders a card

.Prerequisites: Discord desktop app running on Windows, and an anime episode
under one of your Anime Roots.

Steps you only can do:

  [ ] 1. Open https://discord.com/developers/applications and click "New Application"
  [ ] 2. Name it exactly:  Surtr
  [ ] 3. Copy its "Application ID" (a long number on the General Information page)
  [ ] 4. Open script-opts/surtr.conf and set client_id to that Application ID
  [ ] 5. Open Discord -> User Settings -> Activity Privacy and enable
          "Share your detected activities with others"
  [ ] 6. Fully quit Discord from the system tray and start it again
  [ ] 7. Restart mpv (it re-reads surtr.conf)
  [ ] 8. Play an episode under one of your Anime Roots, with the mpv window focused
  [ ] 9. Look at your own Discord profile: a "Watching Surtr" card should show
          cover art, the anime title, "Episode X of Y" and a progress bar
  [ ] 10. Press Ctrl+Shift+A in mpv: the card disappears and the OSD says
          "Surtr: presence off"; press it again: the card comes back
  [ ] 11. Quit mpv: the card disappears

If no card appears, open mpv's console with ` and read the "[surtr]" lines:
the most common cause is step 5 (activity sharing off), then a file outside
every Anime Root, then a title AniList cannot match.
