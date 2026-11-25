# Feeds

Domain: [feeds](../../architecture/intents/domains/feeds.md). Package A13. Size M.

**There is.** HTTPS, HTML reading, notifications.

**Work.** Atom (RFC 4287) and RSS the person subscribes to, OPML in and out; read confined, without
scripts and remote pictures until asked; podcasts go to [Music](music.md)'s queue; fetched when the
person asks or at the times they set.

**Done when** 500 feeds refresh without waking the computer between, and nothing is fetched the
person did not subscribe to.
