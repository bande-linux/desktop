#!/usr/bin/env python3
"""check-look.py [browser]: under scripts/session.sh: fails when the window's colours aren't Datum's, when
anything but empty title bar lies under the compositor's control well, or when the menu button isn't
beside it: light and dark, buttons on either side, changed while the browser runs, in both writing
directions, with vertical tabs and full screen."""
import os
import re
import subprocess
import sys
import time

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
from drive import Browser, shot

THEME = os.path.join(os.path.dirname(os.path.dirname(os.path.realpath(__file__))), "theme")
LOOK = """
const at = e => { const r = e.getBoundingClientRect(); return [r.left, r.top, r.right, r.bottom]; };
const box = document.createXULElement("box");
document.documentElement.append(box);
box.style.backgroundColor = "AccentColor";
const accent = getComputedStyle(box).backgroundColor;
box.remove();
const under = [];
for (let x = arguments[0]; x < arguments[1]; x += 3) {
  const hit = document.elementFromPoint(x, arguments[2] / 2);
  const held = hit?.closest("toolbarbutton, toolbaritem, tab, .toolbar-items, input");
  if (held) under.push(held.id || held.className);
}
const popup = document.getElementById("appMenu-popup");
const content = popup.shadowRoot?.querySelector("[part=content]");
const item = popup.querySelector(".subviewbutton");
return {
  accent,
  under: [...new Set(under)],
  menu: at(document.querySelector("#PanelUI-menu-button > .toolbarbutton-badge-stack")),
  window: getComputedStyle(document.body).backgroundColor,
  tab: getComputedStyle(gBrowser.selectedTab.querySelector(".tab-background")).backgroundColor,
  panel: content && getComputedStyle(content).backgroundColor,
  item: item && getComputedStyle(item).color,
};
"""
PAGE = "return getComputedStyle(document.documentElement).backgroundColor;"
failures = []


def declared(sheet):
    with open(os.path.join(THEME, sheet)) as source:
        text = source.read()
    light, _, rest = text.partition("@media (prefers-color-scheme: dark)")
    find = lambda part: dict(re.findall(r"--bande-([\w-]+): ([^;]+);", part))
    return find(light), {**find(light), **find(rest.partition("}")[0])}


def rgb(value):
    return "rgb({}, {}, {})".format(*(int(value[i : i + 2], 16) for i in (1, 3, 5)))


def settle(read, good):
    for _ in range(40):
        value = read()
        if good(value):
            return value
        time.sleep(0.25)
    return value


def expect(case, what, got, wanted):
    if got != wanted:
        failures.append(f"{case}: {what} is {got}, not {wanted}")


def dbus(name, path, method, *arguments):
    return subprocess.run(
        ["dbus-send", "--session", "--print-reply=literal", f"--dest={name}", path, method, *arguments],
        check=True, capture_output=True, text=True,
    ).stdout


def config(name, value):
    dbus("org.bande.Config", "/org/bande/Config", "org.freedesktop.DBus.Properties.Set",
         "string:org.bande.Config", f"string:{name}", f"variant:string:{value}")


def portal_accent():
    reply = dbus("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop",
                 "org.freedesktop.portal.Settings.ReadOne", "string:org.freedesktop.appearance", "string:accent-color")
    return "rgb({}, {}, {})".format(*(round(float(part) * 255) for part in re.findall(r"double ([\d.]+)", reply)))


def check_row(browser, case, side):
    width = browser.js("return innerWidth;")
    span, height, size, gap = (SIZES[name] for name in ("well-span", "row-height", "menu-size", "menu-gap"))
    well = (width - span, width) if side == "right" else (0, span)
    left = well[0] - gap - size if side == "right" else well[1] + gap
    top = (height - size) / 2
    wanted = [left, top, left + size, top + size]
    good = lambda look: all(abs(a - b) < 1 for a, b in zip(look["menu"], wanted)) and not look["under"]
    look = settle(lambda: browser.js(LOOK, *well, height), good)
    if not good(look):
        expect(case, "the menu button", look["menu"], wanted)
        expect(case, "what lies under the well", look["under"], [])
    return look


def check_colors(browser, case, scheme):
    tokens = declared("colors.css")[scheme == "dark"]
    wanted = {
        "window": rgb(tokens["chassis"]), "tab": rgb(tokens["well"]), "panel": rgb(tokens["overlay"]),
        "item": rgb(tokens["ink"]), "accent": portal_accent(),
    }
    read = lambda: browser.js(LOOK, 0, 0, 0)
    look = settle(read, lambda look: all(look[name] == value for name, value in wanted.items()))
    for name, value in wanted.items():
        expect(case, name, look[name], value)
    browser.content()
    expect(case, "the Settings page", browser.js(PAGE), rgb(tokens["sheet"]))
    browser.chrome()


def check(direction, shots):
    prefs = {"bande.search.asked": True}
    if direction == "rtl":
        prefs["intl.l10n.pseudo"] = "bidi"
    config("WindowButtons", "left")
    browser = Browser(BINARY, prefs)
    try:
        browser.go("about:preferences")
        browser.chrome()
        browser.js("for (let i = 0; i < 12; i++) gBrowser.addTrustedTab('about:blank');")
        for side in ("left", "right"):
            config("WindowButtons", side)
            for scheme in ("light", "dark"):
                config("Appearance", scheme)
                case = f"{direction}, buttons {side}, {scheme}"
                check_row(browser, case, side)
                browser.js("window.PanelUI.show();")
                check_colors(browser, case, scheme)
                if shots:
                    shot(os.path.join(shots, f"{direction}-{side}-{scheme}.png"))
                browser.js("window.PanelUI.hide();")
                time.sleep(1)
            browser.js("Services.prefs.setBoolPref('sidebar.verticalTabs', true);")
            check_row(browser, f"{direction}, buttons {side}, vertical tabs", side)
            browser.js("Services.prefs.setBoolPref('sidebar.verticalTabs', false);")
            browser.js("window.fullScreen = true;")
            space = "const s = getComputedStyle(document.getElementById('TabsToolbar')); return [s.paddingLeft, s.paddingRight];"
            expect(f"{direction}, buttons {side}, full screen", "the space left for the well",
                   settle(lambda: browser.js(space), lambda got: got == ["0px", "0px"]), ["0px", "0px"])
            browser.js("window.fullScreen = false;")
            time.sleep(1)
    finally:
        browser.quit()


BINARY = sys.argv[1] if len(sys.argv) > 1 else "/usr/lib/bande-browser/bande-browser"
SIZES = {name: float(value[:-2]) for name, value in declared("controls.css")[0].items() if value.endswith("px")}
for direction in ("ltr", "rtl"):
    check(direction, os.environ.get("LOOK_SHOTS"))
if failures:
    sys.exit("check-look: " + "\ncheck-look: ".join(failures))
