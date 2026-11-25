# Photos

Domain: [photos](../../architecture/intents/domains/photos.md). Package A3, after D1's facts. Size
L.

**There is.** Decoding (JPEG, PNG, HEIF, JPEG XL, raw within limits), facts as display text,
confined import, thumbnails, the index service, Viewer.

**Work.** Facts as typed values (date with zone, dimensions, camera, place) in the confined importer
and the index; the app over plain folders, copying from a camera or card into folders by date when
asked: by date, album, favourite, and a map only with [Maps](maps.md)' installed regions; albums and
favourites in a small open file beside the library; edits through the same operations as the domain
(rotate, crop, copy without place); ratings and keywords in a sidecar (XMP); duplicates. Writing
metadata only to a copy.

**Not.** Recognising people or things: it needs a model the person chooses, and is decided apart.

**Done when** 50,000 pictures scroll without stutter, a picture added by another program appears,
and nothing is uploaded.
