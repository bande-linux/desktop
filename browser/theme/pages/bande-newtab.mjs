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

async function showSites() {
  const db = await PlacesUtils.promiseDBConnection();
  const rows = await db.executeCached(
    `SELECT url, title FROM moz_places
     WHERE hidden = 0 AND frecency > 0 AND url LIKE 'http%'
     ORDER BY frecency DESC LIMIT 8`
  );
  const list = document.getElementById("sites");
  for (const row of rows) {
    const url = row.getResultByName("url");
    const link = document.createElement("a");
    link.href = url;
    const icon = document.createElement("img");
    icon.alt = "";
    icon.src = `page-icon:${url}`;
    const label = document.createElement("span");
    label.textContent = row.getResultByName("title") || new URL(url).hostname;
    link.append(icon, label);
    link.addEventListener("click", event => {
      event.preventDefault();
      window.browsingContext.topChromeWindow.openTrustedLinkIn(url, "current");
    });
    const item = document.createElement("li");
    item.append(link);
    list.append(item);
  }
  document.getElementById("empty").hidden = rows.length > 0;
}

showTime();
showSites();
