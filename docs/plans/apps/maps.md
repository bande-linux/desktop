# Maps

Domain: [maps](../../architecture/intents/domains/maps.md). Package A9. Size XL.

**There is.** Vector drawing, search, HTTPS. No tile, routing or location code.

**Work.** Vector tiles from region files the person downloads (PMTiles, MVT); places and addresses
searched in them; routes on foot, by bicycle and by car offline; `geo:` links and GPX; the person's
location only when asked, through the location portal; a server only if the person enters one.

**Rule.** No default server, no download and no request the person did not start.

**Done when** a country's file pans smoothly and routes without a network.
