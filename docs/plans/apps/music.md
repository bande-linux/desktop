# Music

Domain: [music](../../architecture/intents/domains/music.md). Package A4. Size L.

**There is.** Player, `bande-media` with its engine in a process of its own, MPRIS, `media-view`.
`lofty` is packaged for tags.

**Work.** A library in Player, or an app of its own sharing its engine: reading tags in a confined
process; artists, albums, songs, playlists (M3U); a queue; gapless where the engine allows; search
offers. Covers from the files; looking one up only when asked, from a server the person set.

**Decide first.** Player with a library, or a second app. Proposal: an app, so that Player stays a
player. Podcasts from [Feeds](feeds.md) play in its queue.

**Done when** 20,000 tracks are listed at once, and "play the new album" works with MPRIS and
intents alike.
