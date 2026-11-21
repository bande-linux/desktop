/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

const { PlacesUtils } = ChromeUtils.importESModule(
  "resource://gre/modules/PlacesUtils.sys.mjs"
);

function showTime() {
  const now = new Date();
  document.getElementById("clock").textContent = now.toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
  document.getElementById("date").textContent = now.toLocaleDateString(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
  });
  setTimeout(showTime, 60000 - (now.getTime() % 60000));
}

function tile(url, title) {
  const link = document.createElement("a");
  link.href = url;
  const icon = document.createElement("img");
  icon.alt = "";
  icon.src = `page-icon:${url}`;
  const label = document.createElement("span");
  label.textContent = title || new URL(url).hostname;
  link.append(icon, label);
  link.addEventListener("click", event => {
    event.preventDefault();
    window.browsingContext.topChromeWindow.openTrustedLinkIn(url, "current");
  });
  const item = document.createElement("li");
  item.append(link);
  return item;
}

function fill(id, rows) {
  const list = document.getElementById(id);
  list.replaceChildren(...rows.map(row => tile(row.getResultByName("url"), row.getResultByName("title"))));
  list.closest("section").hidden = rows.length == 0;
}

async function showSites() {
  const db = await PlacesUtils.promiseDBConnection();
  const favorites = await db.executeCached(
    `SELECT h.url, b.title FROM moz_bookmarks b JOIN moz_places h ON h.id = b.fk
     WHERE b.parent = (SELECT id FROM moz_bookmarks WHERE guid = :toolbar)
       AND b.type = :url AND h.url LIKE :web
     ORDER BY b.position LIMIT 12`,
    { toolbar: PlacesUtils.bookmarks.toolbarGuid, url: PlacesUtils.bookmarks.TYPE_BOOKMARK, web: "http%" }
  );
  const frequent = await db.executeCached(
    `SELECT url, title FROM moz_places
     WHERE hidden = 0 AND visit_count > 0 AND frecency > 0 AND url LIKE :web
       AND id NOT IN (SELECT b.fk FROM moz_bookmarks b
                      WHERE b.parent = (SELECT id FROM moz_bookmarks WHERE guid = :toolbar) AND b.fk NOT NULL)
     ORDER BY frecency DESC LIMIT 8`,
    { toolbar: PlacesUtils.bookmarks.toolbarGuid, web: "http%" }
  );
  fill("favorites", favorites);
  fill("sites", frequent);
  document.getElementById("empty").hidden = favorites.length + frequent.length > 0;
}

showTime();
showSites();
