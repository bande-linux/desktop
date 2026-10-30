import os, sys

OUT = sys.argv[1]
os.makedirs(OUT, exist_ok=True)

ACCENT = "#2F6FEB"
LABEL = "#1A1C20"
MUTED = "#5F646D"
SEP = "rgba(26,28,32,0.10)"
FONT = "Inter"


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def text(x, y, s, size=13, weight=400, fill=LABEL, anchor="start", extra=""):
    return (f'<text x="{x}" y="{y}" font-family="{FONT}" font-size="{size}" font-weight="{weight}" '
            f'fill="{fill}" text-anchor="{anchor}" {extra}>{esc(s)}</text>')


DEFS = f"""
<defs>
 <linearGradient id="wall" x1="0" y1="0" x2="1" y2="1">
  <stop offset="0" stop-color="#C9D6EA"/><stop offset="0.45" stop-color="#E7DCEB"/><stop offset="1" stop-color="#F3E3D3"/>
 </linearGradient>
 <radialGradient id="wallglow" cx="0.72" cy="0.3" r="0.6">
  <stop offset="0" stop-color="#FFFFFF" stop-opacity="0.55"/><stop offset="1" stop-color="#FFFFFF" stop-opacity="0"/>
 </radialGradient>
 <filter id="shadow" x="-20%" y="-20%" width="140%" height="150%">
  <feDropShadow dx="0" dy="10" stdDeviation="16" flood-color="#0B1020" flood-opacity="0.20"/>
 </filter>
 <filter id="pop" x="-20%" y="-20%" width="140%" height="150%">
  <feDropShadow dx="0" dy="6" stdDeviation="10" flood-color="#0B1020" flood-opacity="0.22"/>
 </filter>
 <filter id="rail" x="-50%" y="-5%" width="200%" height="110%">
  <feDropShadow dx="0" dy="0" stdDeviation="6" flood-color="#0B1020" flood-opacity="0.12"/>
 </filter>
 <linearGradient id="g-files" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#5AA2FF"/><stop offset="1" stop-color="#2F6FEB"/></linearGradient>
 <linearGradient id="g-web" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#FF9F5A"/><stop offset="1" stop-color="#F0563A"/></linearGradient>
 <linearGradient id="g-mail" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#5BD3F5"/><stop offset="1" stop-color="#1E9BD7"/></linearGradient>
 <linearGradient id="g-term" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#3A3F48"/><stop offset="1" stop-color="#1D2026"/></linearGradient>
 <linearGradient id="g-text" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#FFE27A"/><stop offset="1" stop-color="#F5B82E"/></linearGradient>
 <linearGradient id="g-music" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#FF7A9A"/><stop offset="1" stop-color="#E8335F"/></linearGradient>
 <linearGradient id="g-set" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#A9B0BC"/><stop offset="1" stop-color="#6E7684"/></linearGradient>
 <linearGradient id="g-cal" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#FFFFFF"/><stop offset="1" stop-color="#EDEEF2"/></linearGradient>
 <linearGradient id="g-photo" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#9C7CFF"/><stop offset="1" stop-color="#6A45E8"/></linearGradient>
 <linearGradient id="g-mon" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#6EE7A8"/><stop offset="1" stop-color="#22B573"/></linearGradient>
 <linearGradient id="g-chat" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#7C8CFF"/><stop offset="1" stop-color="#4A55D8"/></linearGradient>
 <linearGradient id="g-trash" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#F4F5F7"/><stop offset="1" stop-color="#DADDE3"/></linearGradient>
</defs>
"""

APPS = [
    ("files", "Files", True, 0),
    ("web", "Web Browser", True, 0),
    ("mail", "Mail", True, 3),
    ("cal", "Calendar", False, 0),
    ("music", "Music", True, 0),
    ("text", "Text Editor", True, 0),
    ("term", "Terminal", True, 0),
    ("set", "Settings", False, 0),
]
RUNNING_EXTRA = [("chat", "Chat", True, 12), ("mon", "Activity Monitor", True, 0)]
ACTIVE = "files"
MENUS = ["File", "Edit", "View", "Go", "Window", "Help"]


def icon(kind, x, y, s):
    r = s * 0.23
    o = [f'<g transform="translate({x},{y}) scale({s/40})">']
    o.append(f'<rect x="0" y="0" width="40" height="40" rx="{r*40/s}" fill="url(#g-{kind})"/>')
    if kind == "files":
        o.append('<path d="M8 13 h9 l3 3 h12 v14 a2 2 0 0 1 -2 2 h-20 a2 2 0 0 1 -2 -2 z" fill="#fff" opacity="0.95"/>')
        o.append('<rect x="8" y="18" width="24" height="1.6" fill="#2F6FEB" opacity="0.35"/>')
    elif kind == "web":
        o.append('<circle cx="20" cy="20" r="11" fill="none" stroke="#fff" stroke-width="2.2"/>')
        o.append('<ellipse cx="20" cy="20" rx="4.8" ry="11" fill="none" stroke="#fff" stroke-width="1.8"/>')
        o.append('<path d="M9 20 h22 M11 14 h18 M11 26 h18" stroke="#fff" stroke-width="1.6"/>')
    elif kind == "mail":
        o.append('<rect x="8" y="12" width="24" height="17" rx="2.5" fill="#fff"/>')
        o.append('<path d="M9 13.5 l11 8 l11 -8" fill="none" stroke="#1E9BD7" stroke-width="1.8"/>')
    elif kind == "term":
        o.append('<path d="M10 14 l6 5 l-6 5" fill="none" stroke="#7CF0A0" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/>')
        o.append('<rect x="18" y="24" width="10" height="2.4" rx="1" fill="#E6E8EC"/>')
    elif kind == "text":
        for i, w in enumerate([22, 22, 16, 20]):
            o.append(f'<rect x="9" y="{11+i*5.2}" width="{w}" height="2.2" rx="1" fill="#7A5A00" opacity="0.55"/>')
    elif kind == "music":
        o.append('<path d="M17 27 v-14 l11 -3 v13" fill="none" stroke="#fff" stroke-width="2.4" stroke-linejoin="round"/>')
        o.append('<circle cx="14.5" cy="27.5" r="3.2" fill="#fff"/><circle cx="25.5" cy="23.5" r="3.2" fill="#fff"/>')
    elif kind == "set":
        o.append('<circle cx="20" cy="20" r="9.5" fill="none" stroke="#fff" stroke-width="4.5" stroke-dasharray="3.2 2.2"/>')
        o.append('<circle cx="20" cy="20" r="6" fill="none" stroke="#fff" stroke-width="2.4"/>')
    elif kind == "cal":
        o.append('<rect x="0" y="0" width="40" height="11" rx="0" fill="#F0453A" clip-path="none"/>')
        o.append(f'<rect x="0" y="0" width="40" height="40" rx="{r*40/s}" fill="none" stroke="#D5D8DE" stroke-width="0.8"/>')
        o.append(f'<text x="20" y="8.2" font-family="{FONT}" font-size="6.5" font-weight="700" fill="#fff" text-anchor="middle">THU</text>')
        o.append(f'<text x="20" y="32" font-family="{FONT}" font-size="18" font-weight="600" fill="#1A1C20" text-anchor="middle">30</text>')
    elif kind == "photo":
        for i, c in enumerate(["#FFD166", "#06D6A0", "#EF476F", "#FFFFFF"]):
            dx = [-4, 4, 0, 0][i]; dy = [0, 0, -4, 4][i]
            o.append(f'<circle cx="{20+dx}" cy="{20+dy}" r="6" fill="{c}" opacity="0.8"/>')
    elif kind == "mon":
        o.append('<path d="M8 25 l6 -6 l5 4 l7 -9 l6 5" fill="none" stroke="#fff" stroke-width="2.4" stroke-linejoin="round" stroke-linecap="round"/>')
    elif kind == "chat":
        o.append('<path d="M10 12 h20 a3 3 0 0 1 3 3 v9 a3 3 0 0 1 -3 3 h-11 l-5 4 v-4 h-4 a3 3 0 0 1 -3 -3 v-9 a3 3 0 0 1 3 -3 z" fill="#fff"/>')
    elif kind == "trash":
        o.append(f'<rect x="0" y="0" width="40" height="40" rx="{r*40/s}" fill="none" stroke="#C9CDD4" stroke-width="0.8"/>')
        o.append('<path d="M13 14 h14 l-1.3 15 a2 2 0 0 1 -2 1.8 h-7.4 a2 2 0 0 1 -2 -1.8 z" fill="none" stroke="#6E7684" stroke-width="1.8" stroke-linejoin="round"/>')
        o.append('<path d="M11.5 13 h17 M17.5 13 v-2 h5 v2 M18 18 v9 M22 18 v9" stroke="#6E7684" stroke-width="1.6" fill="none" stroke-linecap="round"/>')
    o.append('</g>')
    return "".join(o)


def mark(cx, cy, s=22, fill=LABEL):
    k = s / 22
    return (f'<g transform="translate({cx-11*k},{cy-11*k}) scale({k})">'
            f'<path d="M4 3 v8.5 a7 7 0 0 0 14 0 v-8.5" fill="none" stroke="{fill}" stroke-width="3.2" stroke-linecap="round"/>'
            f'<circle cx="11" cy="11.5" r="2.4" fill="{ACCENT}"/></g>')


def glyph(kind, cx, cy, s=16, c=LABEL):
    k = s / 16
    g = [f'<g transform="translate({cx-8*k},{cy-8*k}) scale({k})" fill="none" stroke="{c}" stroke-width="1.6" stroke-linecap="round">']
    if kind == "wifi":
        g.append('<path d="M1.5 6.2 a9.5 9.5 0 0 1 13 0 M3.9 8.9 a6 6 0 0 1 8.2 0 M6.3 11.5 a2.6 2.6 0 0 1 3.4 0"/>')
        g.append(f'<circle cx="8" cy="13.6" r="1.1" fill="{c}" stroke="none"/>')
    elif kind == "sound":
        g.append(f'<path d="M2 6 h3 l4 -3.4 v10.8 l-4 -3.4 h-3 z" fill="{c}" stroke="none"/>')
        g.append('<path d="M11 5.5 a3.6 3.6 0 0 1 0 5 M12.8 3.6 a6.2 6.2 0 0 1 0 8.8"/>')
    elif kind == "bt":
        g.append('<path d="M4.5 4.8 l7 6.2 l-3.5 3 v-12 l3.5 3 l-7 6.2" stroke-linejoin="round"/>')
    elif kind == "battery":
        g.append(f'<rect x="0.8" y="4.5" width="12.4" height="7" rx="2" stroke-width="1.2"/>')
        g.append(f'<rect x="2.2" y="5.9" width="8.2" height="4.2" rx="1" fill="{c}" stroke="none"/>')
        g.append(f'<path d="M14.6 6.8 v2.4" stroke-width="1.4"/>')
    elif kind == "cc":
        g.append(f'<rect x="1.5" y="2.5" width="13" height="4.6" rx="2.3" stroke-width="1.3"/><circle cx="11.9" cy="4.8" r="1.5" fill="{c}" stroke="none"/>')
        g.append(f'<rect x="1.5" y="8.9" width="13" height="4.6" rx="2.3" stroke-width="1.3"/><circle cx="4.1" cy="11.2" r="1.5" fill="{c}" stroke="none"/>')
    elif kind == "cloud":
        g.append('<path d="M4.5 12.5 h7.5 a3 3 0 0 0 0 -6 a4.3 4.3 0 0 0 -8.2 1.2 a2.5 2.5 0 0 0 0.7 4.8 z" stroke-linejoin="round"/>')
    elif kind == "drop":
        g.append(f'<path d="M8 2.5 l4.5 3 l-4.5 3 l-4.5 -3 z M8 8.5 l4.5 3 l-4.5 3 l-4.5 -3 z" fill="{c}" stroke="none" opacity="0.85"/>')
    elif kind == "kbd":
        g.append(f'<text x="8" y="11.5" font-family="{FONT}" font-size="9" font-weight="600" fill="{c}" stroke="none" text-anchor="middle">EN</text>')
        g.append('<rect x="0.8" y="2.8" width="14.4" height="10.4" rx="2.4" stroke-width="1.2"/>')
    elif kind == "search":
        g.append('<circle cx="7" cy="7" r="4.6"/><path d="M10.4 10.4 l3.8 3.8"/>')
    elif kind == "chev":
        g.append('<path d="M6 3.5 l4.5 4.5 l-4.5 4.5"/>')
    elif kind == "chevdown":
        g.append('<path d="M3.5 6 l4.5 4.5 l4.5 -4.5"/>')
    elif kind == "more":
        for i in (3, 8, 13):
            g.append(f'<circle cx="{i}" cy="8" r="1.4" fill="{c}" stroke="none"/>')
    elif kind == "bell":
        g.append('<path d="M4 11 v-4 a4 4 0 0 1 8 0 v4 l1.3 1.5 h-10.6 z M6.6 14 a1.6 1.6 0 0 0 2.8 0" stroke-linejoin="round"/>')
    elif kind == "sun":
        g.append(f'<circle cx="8" cy="8" r="3" fill="{c}" stroke="none"/><path d="M8 1.5 v1.6 M8 12.9 v1.6 M1.5 8 h1.6 M12.9 8 h1.6 M3.4 3.4 l1.1 1.1 M11.5 11.5 l1.1 1.1 M3.4 12.6 l1.1 -1.1 M11.5 4.5 l1.1 -1.1"/>')
    elif kind == "menu":
        g.append('<path d="M3 4.5 h10 M3 8 h10 M3 11.5 h10"/>')
    g.append('</g>')
    return "".join(g)


def window(x, y, w, h, title, kind="files", active=True):
    o = [f'<g filter="url(#shadow)"><rect x="{x}" y="{y}" width="{w}" height="{h}" rx="12" fill="#FFFFFF"/></g>']
    tb = 44
    o.append(f'<rect x="{x}" y="{y}" width="{w}" height="{tb}" rx="12" fill="{"#F4F5F7" if active else "#F8F8FA"}"/>')
    o.append(f'<rect x="{x}" y="{y+tb-12}" width="{w}" height="12" fill="{"#F4F5F7" if active else "#F8F8FA"}"/>')
    o.append(f'<rect x="{x}" y="{y+tb}" width="{w}" height="1" fill="#E3E5EA"/>')
    cols = ["#FF5F57", "#FEBC2E", "#28C840"] if active else ["#D5D8DE"] * 3
    for i, c in enumerate(cols):
        o.append(f'<circle cx="{x+22+i*20}" cy="{y+22}" r="6.5" fill="{c}"/>')
    o.append(text(x + w / 2, y + 27, title, 13, 600 if active else 500, LABEL if active else MUTED, "middle"))
    if kind == "files":
        sw = min(200, w * 0.25)
        o.append(f'<rect x="{x}" y="{y+tb+1}" width="{sw}" height="{h-tb-1}" fill="#F7F8FA"/>')
        o.append(f'<rect x="{x}" y="{y+h-12}" width="12" height="12" fill="#F7F8FA"/>')
        for i, n in enumerate(["Favourites", "Home", "Documents", "Downloads", "Pictures", "Music", "", "Locations", "Macintosh", "Network"]):
            if not n:
                continue
            head = n in ("Favourites", "Locations")
            if n == "Documents":
                o.append(f'<rect x="{x+10}" y="{y+tb+16+i*30}" width="{sw-20}" height="26" rx="7" fill="{ACCENT}" opacity="0.14"/>')
            o.append(text(x + 22, y + tb + 34 + i * 30, n, 11 if head else 13, 600 if head else 400, MUTED if head else LABEL))
        gx = x + sw + 28
        names = ["Budget 2026.ods", "Holiday", "Notes.txt", "Invoices", "Plan.pdf", "Photos", "Report.odt", "Scans", "Taxes", "Recipes.md", "Fonts", "Archive.zip"]
        per = max(1, int((w - sw - 40) // 120))
        for i, n in enumerate(names):
            r, c = divmod(i, per)
            if y + tb + 30 + r * 120 + 100 > y + h:
                break
            ix = gx + c * 120 + 22
            iy = y + tb + 30 + r * 120
            if "." in n:
                o.append(f'<path d="M{ix+12} {iy} h28 l14 14 v46 h-42 z" fill="#FFFFFF" stroke="#C9CDD4"/>')
                o.append(f'<rect x="{ix+18}" y="{iy+24}" width="28" height="2" fill="#C9CDD4"/><rect x="{ix+18}" y="{iy+32}" width="22" height="2" fill="#C9CDD4"/>')
            else:
                o.append(f'<path d="M{ix} {iy+8} h24 l6 6 h36 v42 a4 4 0 0 1 -4 4 h-58 a4 4 0 0 1 -4 -4 z" fill="#7FB2FF"/>')
                o.append(f'<rect x="{ix}" y="{iy+20}" width="66" height="40" rx="4" fill="#5A9BFF"/>')
            o.append(text(ix + 33, iy + 82, n, 12, 400, LABEL, "middle"))
    elif kind == "text":
        lines = ["# Combined bar", "", "One bar on the left edge holds the launcher, the active", "app and its menus, the apps, the status items and the clock.", "", "- Fitts: the edge is infinitely deep.", "- Wide screens have width to spare, not height.", "- Menus stay visible and searchable."]
        for i, l in enumerate(lines):
            if y + tb + 40 + i * 24 > y + h - 10:
                break
            o.append(text(x + 28, y + tb + 36 + i * 24, l, 18 if l.startswith("#") else 14, 600 if l.startswith("#") else 400, LABEL if active else MUTED))
    elif kind == "web":
        o.append(f'<rect x="{x+120}" y="{y+9}" width="{w-240}" height="26" rx="8" fill="#FFFFFF" stroke="#E3E5EA"/>')
        o.append(text(x + w / 2, y + 27, "unbound.example/docs/bar", 12, 400, MUTED, "middle"))
        o.append(f'<rect x="{x+40}" y="{y+tb+40}" width="{w*0.5}" height="26" rx="4" fill="#E9ECF2"/>')
        for i in range(9):
            if y + tb + 100 + i * 26 > y + h - 20:
                break
            o.append(f'<rect x="{x+40}" y="{y+tb+96+i*26}" width="{(w-80)*(0.92 if i%4 != 3 else 0.6)}" height="10" rx="4" fill="#EEF0F4"/>')
    elif kind == "term":
        o.append(f'<rect x="{x}" y="{y+tb+1}" width="{w}" height="{h-tb-1}" fill="#FFFFFF"/>')
        for i, l in enumerate(["mikael@unbound ~ % cargo test -p unbound-bar", "   Compiling unbound-bar v0.1.0", "    Finished test profile in 4.2s", "     Running 38 tests", "test result: ok. 38 passed; 0 failed", "mikael@unbound ~ % "]):
            if y + tb + 30 + i * 20 > y + h - 10:
                break
            o.append(f'<text x="{x+16}" y="{y+tb+26+i*20}" font-family="DejaVu Sans Mono" font-size="12" fill="{LABEL}">{esc(l)}</text>')
    return "".join(o)


def popover(x, y, w, rows, highlight=None, header=None):
    h = 8 + sum(9 if r == "-" else 24 for r in rows) + 8 + (40 if header else 0)
    o = [f'<g filter="url(#pop)"><rect x="{x}" y="{y}" width="{w}" height="{h}" rx="10" fill="#FCFCFD" fill-opacity="0.97"/></g>',
         f'<rect x="{x+0.5}" y="{y+0.5}" width="{w-1}" height="{h-1}" rx="10" fill="none" stroke="rgba(0,0,0,0.08)"/>']
    cy = y + 8
    if header:
        o.append(f'<rect x="{x+8}" y="{cy}" width="{w-16}" height="30" rx="8" fill="#EDEFF3"/>')
        o.append(glyph("search", x + 24, cy + 15, 13, MUTED))
        o.append(text(x + 38, cy + 20, header, 13, 400, MUTED))
        cy += 40
    rects = {}
    for r in rows:
        if r == "-":
            o.append(f'<rect x="{x+12}" y="{cy+4}" width="{w-24}" height="1" fill="rgba(0,0,0,0.09)"/>')
            cy += 9
            continue
        label, sc, sub, bold = r if len(r) == 4 else (*r, False)
        hi = label == highlight
        if hi:
            o.append(f'<rect x="{x+5}" y="{cy}" width="{w-10}" height="24" rx="6" fill="{ACCENT}"/>')
        col = "#FFFFFF" if hi else LABEL
        o.append(text(x + 16, cy + 16.5, label, 13, 600 if bold else 400, col))
        if sc:
            o.append(text(x + w - 16 - (12 if sub else 0), cy + 16.5, sc, 12, 400, "#FFFFFF" if hi else MUTED, "end"))
        if sub:
            o.append(glyph("chev", x + w - 16, cy + 12, 11, col))
        rects[label] = cy
        cy += 24
    return "".join(o), rects, h


def material(x, y, w, h, rx=0, edge=None):
    o = [f'<g filter="url(#rail)"><rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" fill="#F6F7F9" fill-opacity="0.80"/></g>']
    o.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" fill="#FFFFFF" fill-opacity="0.35"/>')
    if edge == "right":
        o.append(f'<rect x="{x+w-1}" y="{y}" width="1" height="{h}" fill="rgba(255,255,255,0.7)"/>')
    if edge == "top":
        o.append(f'<rect x="{x}" y="{y}" width="{w}" height="1" fill="rgba(255,255,255,0.7)"/>')
    return "".join(o)


def rail_collapsed(H, open_item=None, small=False, apps=None, extra=None, ws=3, tray=3, x0=0):
    W = 64
    apps = APPS if apps is None else apps
    extra = RUNNING_EXTRA if extra is None else extra
    o = [material(x0, 0, W, H, 0, "right")]
    cx = x0 + W / 2
    o.append(mark(cx, 26, 24))
    if open_item == "launcher":
        o.insert(1, f'<rect x="{x0+8}" y="6" width="48" height="40" rx="10" fill="rgba(26,28,32,0.08)"/>')
    hy = 56
    hb = "rgba(47,111,235,0.14)" if open_item == "menu" else "rgba(26,28,32,0.05)"
    o.append(f'<rect x="{x0+6}" y="{hy}" width="52" height="58" rx="11" fill="{hb}"/>')
    o.append(icon(ACTIVE, cx - 12, hy + 7, 24))
    o.append(text(cx - 2, hy + 47, "Files", 11, 600, LABEL, "middle"))
    o.append(glyph("chev", cx + 17, hy + 43, 9, MUTED))
    o.append(f'<rect x="{x0+16}" y="{hy+67}" width="32" height="1" fill="{SEP}"/>')
    cell = 48 if not small else 44
    isz = 40 if not small else 36
    y = hy + 76
    rects = {}
    def app_cell(a, y):
        k, n, running, badge = a
        r = [icon(k, cx - isz / 2, y + (cell - isz) / 2, isz)]
        if k == ACTIVE:
            r.append(f'<rect x="{x0}" y="{y+cell/2-10}" width="3" height="20" rx="1.5" fill="{LABEL}"/>')
        elif running:
            r.append(f'<circle cx="{x0+4}" cy="{y+cell/2}" r="2" fill="{LABEL}"/>')
        if badge:
            bx = cx + isz / 2 - 4
            bw = 18 if badge < 10 else 22
            r.append(f'<rect x="{bx-bw/2}" y="{y+(cell-isz)/2-5}" width="{bw}" height="18" rx="9" fill="#FF3B30" stroke="#F6F7F9" stroke-width="1.5"/>')
            r.append(text(bx, y + (cell - isz) / 2 + 8, str(badge), 11, 600, "#FFFFFF", "middle"))
        return "".join(r)
    bottom = []
    clock_h = 52
    by = H - clock_h
    bottom.append(text(cx, by + 22, "14:32", 15, 600, LABEL, "middle", 'font-feature-settings="tnum"'))
    bottom.append(text(cx, by + 39, "Thu 30", 11, 500, MUTED, "middle"))
    bottom.append(f'<circle cx="{cx+22}" cy="{by+10}" r="3.5" fill="{ACCENT}"/>')
    if open_item == "clock":
        bottom.insert(0, f'<rect x="{x0+6}" y="{by+2}" width="52" height="46" rx="10" fill="rgba(26,28,32,0.08)"/>')
    sy = by - 66
    sb = "rgba(47,111,235,0.14)" if open_item == "cc" else "rgba(26,28,32,0.05)"
    bottom.append(f'<rect x="{x0+6}" y="{sy}" width="52" height="58" rx="11" fill="{sb}"/>')
    for i, g in enumerate(["wifi", "sound", "bt", "battery"]):
        r, c = divmod(i, 2)
        bottom.append(glyph(g, x0 + 19 + c * 26, sy + 16 + r * 26, 15))
    ty = sy - 8
    titems = ["drop", "cloud", "kbd"][:tray] if tray <= 3 else ["drop", "cloud", "kbd"]
    rows = (len(titems) + (1 if tray > 3 else 0) + 1) // 2
    ty -= rows * 26
    tl = titems + (["more"] if tray > 3 else [])
    for i, g in enumerate(tl):
        r, c = divmod(i, 2)
        bottom.append(glyph(g, x0 + 19 + c * 26, ty + 13 + r * 26, 15, LABEL if g != "more" else MUTED))
    wy = ty - 8
    if ws:
        wy -= ws * 12 + 8
        for i in range(ws):
            if i == 0:
                bottom.append(f'<rect x="{cx-3}" y="{wy+6+i*12}" width="6" height="12" rx="3" fill="{LABEL}"/>')
            else:
                bottom.append(f'<circle cx="{cx}" cy="{wy+12+i*12+3}" r="3" fill="rgba(26,28,32,0.30)"/>')
        wy -= 4
    bottom.append(f'<rect x="{x0+16}" y="{wy-4}" width="32" height="1" fill="{SEP}"/>')
    tr = wy - 8 - 48
    bottom.append(icon("trash", cx - isz / 2, tr + (48 - isz) / 2, isz))
    allapps = list(apps) + ([None] + list(extra) if extra else [])
    need = sum(9 if a is None else cell for a in allapps)
    room = tr - 12 - y
    if need > room:
        shown, used = [], 0
        for a in allapps:
            h = 9 if a is None else cell
            if used + h + cell > room:
                break
            shown.append(a); used += h
        hidden = sum(1 for a in allapps if a is not None) - sum(1 for a in shown if a is not None)
        allapps = shown + [("more", f"+{hidden}", True, 0)]
    for a in allapps:
        if a is None:
            o.append(f'<rect x="{x0+16}" y="{y+4}" width="32" height="1" fill="{SEP}"/>'); y += 9; continue
        if a[0] == "more":
            o.append(f'<rect x="{x0+12}" y="{y+6}" width="40" height="{cell-12}" rx="10" fill="rgba(26,28,32,0.06)"/>')
            o.append(text(cx, y + cell / 2 + 4.5, a[1], 12, 600, MUTED, "middle"))
            y += cell; continue
        o.append(app_cell(a, y)); rects[a[0]] = y; y += cell
    o.extend(bottom)
    return "".join(o), rects, {"header": hy, "cc": sy, "clock": by, "trash": tr, "apps_end": y}


def rail_expanded(H, W=248, menus_inline=True, hover_menu=None, windows=True, x0=0):
    o = [material(x0, 0, W, H, 0, "right")]
    o.append(mark(x0 + 32, 26, 24))
    o.append(f'<rect x="{x0+56}" y="10" width="{W-68}" height="32" rx="9" fill="rgba(26,28,32,0.06)"/>')
    o.append(glyph("search", x0 + 72, 26, 13, MUTED))
    o.append(text(x0 + 86, 30.5, "Search", 13, 400, MUTED))
    o.append(f'<rect x="{x0+W-70}" y="17" width="52" height="18" rx="5" fill="rgba(26,28,32,0.06)"/>')
    o.append(text(x0 + W - 44, 30, "Super", 10.5, 500, MUTED, "middle"))
    y = 54
    o.append(f'<rect x="{x0+8}" y="{y}" width="{W-16}" height="{40 + (len(MENUS)*28+6 if menus_inline else 0)}" rx="11" fill="rgba(26,28,32,0.05)"/>')
    o.append(icon(ACTIVE, x0 + 18, y + 8, 24))
    o.append(text(x0 + 52, y + 25, "Files", 14, 700))
    o.append(glyph("menu", x0 + W - 32, y + 20, 14, MUTED))
    y += 40
    mrects = {}
    if menus_inline:
        for m in MENUS:
            if m == hover_menu:
                o.append(f'<rect x="{x0+14}" y="{y}" width="{W-28}" height="26" rx="7" fill="{ACCENT}"/>')
            col = "#FFFFFF" if m == hover_menu else LABEL
            o.append(text(x0 + 52, y + 17.5, m, 13, 400, col))
            o.append(glyph("chev", x0 + W - 30, y + 13, 10, "#FFFFFF" if m == hover_menu else MUTED))
            mrects[m] = y
            y += 28
        y += 6
    y += 8
    o.append(f'<rect x="{x0+20}" y="{y}" width="{W-40}" height="1" fill="{SEP}"/>')
    y += 8
    def row(a, y, h=40):
        k, n, running, badge = a
        r = []
        if k == ACTIVE:
            r.append(f'<rect x="{x0+8}" y="{y+2}" width="{W-16}" height="{h-4}" rx="9" fill="rgba(26,28,32,0.07)"/>')
            r.append(f'<rect x="{x0}" y="{y+h/2-10}" width="3" height="20" rx="1.5" fill="{LABEL}"/>')
        elif running:
            r.append(f'<circle cx="{x0+4}" cy="{y+h/2}" r="2" fill="{LABEL}"/>')
        r.append(icon(k, x0 + 16, y + (h - 30) / 2, 30))
        r.append(text(x0 + 56, y + h / 2 + 4.5, n, 13, 500 if k == ACTIVE else 400))
        if badge:
            r.append(f'<rect x="{x0+W-46}" y="{y+h/2-9}" width="28" height="18" rx="9" fill="#FF3B30"/>')
            r.append(text(x0 + W - 32, y + h / 2 + 4, str(badge), 11, 600, "#FFFFFF", "middle"))
        return "".join(r)
    for a in APPS:
        o.append(row(a, y)); y += 40
        if a[0] == ACTIVE and windows:
            for wn, cur in [("Documents", True), ("Downloads", False)]:
                o.append(text(x0 + 56, y + 15, wn, 12, 500 if cur else 400, LABEL if cur else MUTED))
                if cur:
                    o.append(f'<circle cx="{x0+47}" cy="{y+11}" r="2.5" fill="{ACCENT}"/>')
                y += 22
            y += 4
    o.append(f'<rect x="{x0+20}" y="{y+4}" width="{W-40}" height="1" fill="{SEP}"/>'); y += 9
    for a in RUNNING_EXTRA:
        o.append(row(a, y)); y += 40
    by = H - 64
    o.append(f'<rect x="{x0+8}" y="{by}" width="{W-16}" height="56" rx="11" fill="rgba(26,28,32,0.0)"/>')
    o.append(text(x0 + 20, by + 30, "14:32", 22, 600, LABEL, "start", 'font-feature-settings="tnum"'))
    o.append(text(x0 + 20, by + 48, "Thursday 30 October", 12, 400, MUTED))
    o.append(glyph("bell", x0 + W - 34, by + 26, 16, LABEL))
    o.append(f'<circle cx="{x0+W-27}" cy="{by+19}" r="3.5" fill="{ACCENT}"/>')
    sy = by - 56
    o.append(f'<rect x="{x0+8}" y="{sy}" width="{W-16}" height="48" rx="11" fill="rgba(26,28,32,0.05)"/>')
    for i, g in enumerate(["wifi", "sound", "bt", "battery"]):
        o.append(glyph(g, x0 + 30 + i * 30, sy + 17, 15))
    o.append(text(x0 + 21, sy + 41, "Home-5G · 84%", 11, 400, MUTED))
    o.append(glyph("cc", x0 + W - 34, sy + 24, 16))
    ty = sy - 36
    for i, g in enumerate(["drop", "cloud", "kbd"]):
        o.append(glyph(g, x0 + 30 + i * 30, ty + 16, 15))
    wy = ty - 34
    o.append(text(x0 + 20, wy + 20, "Desktops", 12, 500, MUTED))
    for i in range(3):
        fill = LABEL if i == 0 else "rgba(26,28,32,0.10)"
        o.append(f'<rect x="{x0+92+i*36}" y="{wy+6}" width="30" height="20" rx="6" fill="{fill}"/>')
        o.append(text(x0 + 107 + i * 36, wy + 20, str(i + 1), 11, 600, "#FFFFFF" if i == 0 else LABEL, "middle"))
    o.append(f'<rect x="{x0+20}" y="{wy-6}" width="{W-40}" height="1" fill="{SEP}"/>')
    tr = wy - 50
    o.append(icon("trash", x0 + 16, tr + 5, 30))
    o.append(text(x0 + 56, tr + 24.5, "Trash", 13, 400))
    return "".join(o), mrects


def wallpaper(w, h):
    return f'<rect width="{w}" height="{h}" fill="url(#wall)"/><rect width="{w}" height="{h}" fill="url(#wallglow)"/>'


def svg(w, h, body, name):
    s = f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">{DEFS}{body}</svg>'
    with open(os.path.join(OUT, name), "w") as f:
        f.write(s)


def caption(x, y, title, sub=None, w=None):
    o = [text(x, y, title, 22, 700)]
    if sub:
        o.append(text(x, y + 26, sub, 14, 400, MUTED))
    return "".join(o)


def app_menu_rows():
    return [("Files", None, True, True), "-", ("File", None, True), ("Edit", None, True), ("View", None, True), ("Go", None, True), ("Window", None, True), ("Help", None, True)]


EDIT = [("Undo Rename", "Super Z", False), ("Redo", "Shift Super Z", False), "-", ("Cut", "Super X", False), ("Copy", "Super C", False), ("Paste", "Super V", False), ("Select All", "Super A", False), "-", ("Find", None, True), ("Rename…", "Return", False)]

W1, H1 = 1920, 1080
r, rects, pos = rail_collapsed(H1)
b = wallpaper(W1, H1) + window(380, 120, 1080, 660, "Documents", "files", True) + window(980, 430, 760, 520, "Combined bar.md", "text", False)
b = wallpaper(W1, H1) + window(1000, 300, 760, 520, "Combined bar.md", "text", False) + window(300, 110, 1060, 680, "Documents", "files", True)
svg(W1, H1, b + r, "01-desktop-left-collapsed.svg")

r2, _ = rail_expanded(H1)
b = wallpaper(W1, H1) + window(1100, 300, 700, 520, "Combined bar.md", "text", False) + window(420, 110, 1060, 680, "Documents", "files", True)
svg(W1, H1, b + r2, "02-desktop-left-expanded.svg")

PW, PH = 720, 760
A = [wallpaper(PW, PH)]
A.append(window(150, 70, 700, 640, "Documents", "files", True))
ra, _, pa = rail_collapsed(PH - 74, open_item="menu", ws=0, tray=0, apps=APPS[:6], extra=[])
A.append(ra)
p1, rr, h1 = popover(72, pa["header"], 220, app_menu_rows(), "Edit", header="Search menus")
A.append(p1)
p2, _, _ = popover(72 + 220 - 4, rr["Edit"] - 8, 250, EDIT, "Find")
A.append(p2)
A.append(f'<rect x="0" y="{PH-74}" width="{PW}" height="74" fill="#FFFFFF"/>')
A.append(text(20, PH - 44, "A. Cascade from the app header  (recommended)", 17, 700))
A.append(text(20, PH - 20, "One menu for the active app: search field, app items, then each menu as a row.", 13, 400, MUTED))

B = [wallpaper(PW, PH)]
B.append(window(330, 70, 600, 640, "Documents", "files", True))
rb, mr = rail_expanded(PH + 400, W=248, hover_menu="Edit", windows=False)
B.append(f'<svg x="0" y="0" width="{PW}" height="{PH}" viewBox="0 0 {PW} {PH}">{rb}</svg>')
p3, _, _ = popover(248 - 6, mr["Edit"] - 8, 250, EDIT)
B.append(p3)
B.append(f'<rect x="0" y="{PH-74}" width="{PW}" height="74" fill="#FFFFFF"/>')
B.append(text(20, PH - 44, "B. Menus as rows in the expanded bar", 17, 700))
B.append(text(20, PH - 20, "Visible titles like a sidebar; each row opens its menu beside the bar.", 13, 400, MUTED))

C = [wallpaper(PW, PH)]
C.append(window(150, 70, 700, 640, "Documents", "files", True))
rc, _, _ = rail_collapsed(PH - 74, ws=0, tray=0, apps=APPS[:6], extra=[])
C.append(rc)
C.append(f'<rect x="64" y="0" width="{PW-64}" height="{PH}" fill="#0B1020" fill-opacity="0.10"/>')
cx0, cy0, cw = 120, 150, 560
C.append(f'<g filter="url(#pop)"><rect x="{cx0}" y="{cy0}" width="{cw}" height="330" rx="16" fill="#FCFCFD" fill-opacity="0.98"/></g>')
C.append(glyph("menu", cx0 + 28, cy0 + 30, 18, MUTED))
C.append(text(cx0 + 48, cy0 + 37, "fin", 20, 400))
C.append(f'<rect x="{cx0+80}" y="{cy0+18}" width="1.5" height="24" fill="{ACCENT}"/>')
C.append(text(cx0 + cw - 20, cy0 + 36, "Files", 13, 600, MUTED, "end"))
C.append(f'<rect x="{cx0}" y="{cy0+58}" width="{cw}" height="1" fill="rgba(0,0,0,0.08)"/>')
res = [("Find…", "Edit › Find", "Super F"), ("Find Next", "Edit › Find", "Super G"), ("Find Previous", "Edit › Find", "Shift Super G"), ("Use Selection for Find", "Edit › Find", "Super E"), ("Show Info", "File", "Super I"), ("Window: Downloads", "Window", "")]
for i, (l, path, sc) in enumerate(res):
    yy = cy0 + 70 + i * 40
    if i == 0:
        C.append(f'<rect x="{cx0+8}" y="{yy}" width="{cw-16}" height="36" rx="8" fill="{ACCENT}"/>')
    col = "#FFFFFF" if i == 0 else LABEL
    C.append(text(cx0 + 24, yy + 16, l, 14, 500, col))
    C.append(text(cx0 + 24, yy + 30, path, 11, 400, "#DCE6FB" if i == 0 else MUTED))
    C.append(text(cx0 + cw - 24, yy + 23, sc, 12, 400, "#FFFFFF" if i == 0 else MUTED, "end"))
C.append(f'<rect x="0" y="{PH-74}" width="{PW}" height="74" fill="#FFFFFF"/>')
C.append(text(20, PH - 44, "C. Menu search (HUD), Super+/", 17, 700))
C.append(text(20, PH - 20, "Every menu item by name, with its path and shortcut. A layer on A, not a substitute.", 13, 400, MUTED))

sheet = []
for i, part in enumerate([A, B, C]):
    sheet.append(f'<svg x="{24+i*(PW+24)}" y="24" width="{PW}" height="{PH}" viewBox="0 0 {PW} {PH}"><g>{"".join(part)}</g></svg>')
    sheet.append(f'<rect x="{24+i*(PW+24)+0.5}" y="24.5" width="{PW-1}" height="{PH-1}" rx="2" fill="none" stroke="rgba(0,0,0,0.12)"/>')
SW = 24 + 3 * (PW + 24)
svg(SW, PH + 48, f'<rect width="{SW}" height="{PH+48}" fill="#ECEEF2"/>' + "".join(sheet), "03-app-menu-alternatives.svg")


def bar_bottom(Wd, Hd, H=56):
    y0 = Hd - H
    o = [material(0, y0, Wd, H, 0, "top")]
    cy = y0 + H / 2
    o.append(mark(30, cy, 22))
    o.append(f'<rect x="58" y="{y0+8}" width="384" height="40" rx="10" fill="rgba(26,28,32,0.05)"/>')
    o.append(icon(ACTIVE, 68, cy - 11, 22))
    o.append(text(98, cy + 5, "Files", 13, 700))
    x = 146
    widths = {"File": 24, "Edit": 26, "View": 32, "Go": 20, "Window": 51, "Help": 30}
    for m in MENUS:
        o.append(text(x, cy + 5, m, 13, 400))
        x += widths[m] + 18
    x = 490
    o.append(f'<rect x="{x}" y="{y0+16}" width="1" height="24" fill="{SEP}"/>')
    x += 12
    for a in APPS + [None] + RUNNING_EXTRA:
        if a is None:
            o.append(f'<rect x="{x+4}" y="{y0+16}" width="1" height="24" fill="{SEP}"/>'); x += 9; continue
        k, n, running, badge = a
        o.append(icon(k, x + 4, y0 + 6, 40))
        if k == ACTIVE:
            o.append(f'<rect x="{x+14}" y="{Hd-3}" width="20" height="3" rx="1.5" fill="{LABEL}"/>')
        elif running:
            o.append(f'<circle cx="{x+24}" cy="{Hd-4}" r="2" fill="{LABEL}"/>')
        if badge:
            bw = 18 if badge < 10 else 22
            o.append(f'<rect x="{x+40-bw/2}" y="{y0+1}" width="{bw}" height="18" rx="9" fill="#FF3B30" stroke="#F6F7F9" stroke-width="1.5"/>')
            o.append(text(x + 40, y0 + 14, str(badge), 11, 600, "#FFFFFF", "middle"))
        x += 48
    o.append(f'<rect x="{x+4}" y="{y0+16}" width="1" height="24" fill="{SEP}"/>'); x += 9
    o.append(icon("trash", x + 4, y0 + 6, 40))
    rx = Wd - 16
    o.append(text(rx, cy - 1, "14:32", 14, 600, LABEL, "end"))
    o.append(text(rx, cy + 15, "Thu 30 Oct", 11, 400, MUTED, "end"))
    o.append(f'<circle cx="{rx-74}" cy="{cy-6}" r="3.5" fill="{ACCENT}"/>')
    sx = rx - 96 - 132
    o.append(f'<rect x="{sx}" y="{y0+10}" width="128" height="36" rx="10" fill="rgba(26,28,32,0.05)"/>')
    for i, g in enumerate(["wifi", "sound", "bt", "battery"]):
        o.append(glyph(g, sx + 20 + i * 29, cy, 15))
    tx = sx - 100
    for i, g in enumerate(["drop", "cloud", "kbd"]):
        o.append(glyph(g, tx + 14 + i * 28, cy, 15))
    wx = tx - 60
    for i in range(3):
        if i == 0:
            o.append(f'<rect x="{wx+i*12}" y="{cy-3}" width="12" height="6" rx="3" fill="{LABEL}"/>')
        else:
            o.append(f'<circle cx="{wx+12+i*12}" cy="{cy}" r="3" fill="rgba(26,28,32,0.30)"/>')
    return "".join(o)


b = wallpaper(W1, H1) + window(1040, 140, 760, 540, "Combined bar.md", "text", False) + window(220, 60, 1060, 720, "Documents", "files", True)
p, rr, _ = popover(58, 0, 220, [("Undo Rename", "Super Z", False), ("Redo", "Shift Super Z", False), "-", ("Cut", "Super X", False), ("Copy", "Super C", False), ("Paste", "Super V", False), ("Select All", "Super A", False), "-", ("Find", None, True)])
svg(W1, H1, b + bar_bottom(W1, H1), "04-desktop-bottom.svg")


def status_detail():
    w, h = 1000, 620
    o = [f'<rect width="{w}" height="{h}" fill="#DCE3EE"/>', wallpaper(w, h)]
    o.append(text(24, 40, "Status area: one Control Center button, apps' tray items above it, clock and notifications at the corner", 16, 700))
    sub = f'<svg x="40" y="70" width="64" height="520" viewBox="0 1560 64 520">{rail_collapsed(2080, open_item="cc", tray=4)[0]}</svg>'
    o.append(sub)
    ann = [(70 + 300, "Trash"), (70 + 345, "Workspaces (only when there are two or more)"), (70 + 390, "Apps' tray items, 2 per row; ••• when more than 3"), (70 + 450, "Wi-Fi, sound, Bluetooth, battery: one button = Control Center"), (70 + 492, "14:32 · Thu 30, accent dot = unread notifications")]
    ann = [(70 + 340 - 1560 + 1560, a) for _, a in []]
    rows = [(70 + 520 - 270, "Trash"), (70 + 520 - 214, "Workspaces (shown with 2 or more)"), (70 + 520 - 160, "Tray items of apps, two per row; ••• beyond three"), (70 + 520 - 98, "Wi-Fi · sound · Bluetooth · battery = one Control Center button"), (70 + 520 - 30, "Clock (opens Notification Center); dot = unread")]
    for yy, s in rows:
        o.append(f'<path d="M110 {yy} h40" stroke="{MUTED}" stroke-width="1"/>')
        o.append(text(156, yy + 4, s, 13, 400, LABEL))
    ex = f'<svg x="560" y="70" width="248" height="520" viewBox="0 1560 248 520">{rail_expanded(2080)[0]}</svg>'
    o.append(ex)
    o.append(text(560, 610, "Expanded: same order, with names, network and charge, full date", 13, 400, MUTED))
    svg(w, h, "".join(o), "05-status-detail.svg")


status_detail()


def cc_panel(x, y):
    w, h = 320, 420
    o = [f'<g filter="url(#pop)"><rect x="{x}" y="{y}" width="{w}" height="{h}" rx="16" fill="#FCFCFD" fill-opacity="0.96"/></g>']
    o.append(f'<circle cx="{x+30}" cy="{y+30}" r="14" fill="#C9D6EA"/>')
    o.append(text(x + 30, y + 35, "M", 13, 700, LABEL, "middle"))
    o.append(text(x + 52, y + 35, "Mikael", 14, 600))
    for i, g in enumerate(["lock", "power", "gear"]):
        cx = x + w - 96 + i * 32
        o.append(f'<circle cx="{cx}" cy="{y+30}" r="13" fill="rgba(26,28,32,0.06)"/>')
        if g == "lock":
            o.append(f'<rect x="{cx-5}" y="{y+29}" width="10" height="7" rx="1.5" fill="{LABEL}"/><path d="M{cx-3} {y+29} v-3 a3 3 0 0 1 6 0 v3" fill="none" stroke="{LABEL}" stroke-width="1.5"/>')
        elif g == "power":
            o.append(f'<path d="M{cx-4} {y+25.5} a6 6 0 1 0 8 0 M{cx} {y+23} v6" fill="none" stroke="{LABEL}" stroke-width="1.6" stroke-linecap="round"/>')
        else:
            o.append(f'<circle cx="{cx}" cy="{y+30}" r="5" fill="none" stroke="{LABEL}" stroke-width="3" stroke-dasharray="2 1.6"/>')
    tiles = [("wifi", "Wi-Fi", "Home-5G", True), ("bt", "Bluetooth", "AirPods", True), ("bell", "Do Not Disturb", "Off", False), ("kbd", "Input", "English", False)]
    for i, (g, n, s, on) in enumerate(tiles):
        r, c = divmod(i, 2)
        tx, ty = x + 14 + c * 150, y + 60 + r * 64
        o.append(f'<rect x="{tx}" y="{ty}" width="142" height="56" rx="12" fill="{"rgba(47,111,235,0.12)" if on else "rgba(26,28,32,0.05)"}"/>')
        o.append(f'<circle cx="{tx+22}" cy="{ty+28}" r="14" fill="{ACCENT if on else "rgba(26,28,32,0.10)"}"/>')
        o.append(glyph(g, tx + 22, ty + 28, 14, "#FFFFFF" if on else LABEL))
        o.append(text(tx + 44, ty + 25, n, 12.5, 600))
        o.append(text(tx + 44, ty + 41, s, 11.5, 400, MUTED))
    for i, (n, v) in enumerate([("Display", 0.7), ("Sound", 0.45)]):
        sy = y + 200 + i * 62
        o.append(text(x + 16, sy, n, 12.5, 600))
        o.append(f'<rect x="{x+14}" y="{sy+10}" width="{w-28}" height="26" rx="13" fill="rgba(26,28,32,0.08)"/>')
        o.append(f'<rect x="{x+14}" y="{sy+10}" width="{(w-28)*v}" height="26" rx="13" fill="#FFFFFF" stroke="rgba(0,0,0,0.06)"/>')
        o.append(glyph("sound" if n == "Sound" else "sun", x + 32, sy + 23, 13, MUTED))
    o.append(f'<rect x="{x+14}" y="{y+330}" width="{w-28}" height="76" rx="12" fill="rgba(26,28,32,0.05)"/>')
    o.append(f'<rect x="{x+26}" y="{y+342}" width="52" height="52" rx="8" fill="url(#g-music)"/>')
    o.append(text(x + 90, y + 362, "Now Playing", 12.5, 600))
    o.append(text(x + 90, y + 380, "Music · Track 4", 11.5, 400, MUTED))
    return "".join(o), h


Ws, Hs = 1366, 768
apps_small = APPS
extra_small = RUNNING_EXTRA + [("photo", "Photos", True, 0)]
rs, _, ps = rail_collapsed(Hs, open_item="cc", small=True, apps=apps_small, extra=extra_small, ws=2, tray=4)
b = wallpaper(Ws, Hs) + window(96, 18, 1250, 730, "Documents", "files", True)
ccp, cch = cc_panel(72, ps["cc"] + 58 - 420)
svg(Ws, Hs, b + rs + ccp, "06-small-1366x768.svg")

Ww, Hw = 3440, 1440
rw, _ = rail_expanded(Hw)
b = wallpaper(Ww, Hw)
gap = 12
x0 = 248 + gap
avail = Ww - x0 - gap
w1 = int(avail * 0.30); w2 = int(avail * 0.40); w3 = avail - w1 - w2 - 2 * gap
b += window(x0, gap, w1, Hw - 2 * gap, "Documents", "files", True)
b += window(x0 + w1 + gap, gap, w2, Hw - 2 * gap, "Unbound docs", "web", False)
b += window(x0 + w1 + w2 + 2 * gap, gap, w3, (Hw - 3 * gap) // 2, "Combined bar.md", "text", False)
b += window(x0 + w1 + w2 + 2 * gap, gap + (Hw - 3 * gap) // 2 + gap, w3, (Hw - 3 * gap) // 2, "Terminal", "term", False)
svg(Ww, Hw, b + rw, "07-wide-3440x1440.svg")

nb = wallpaper(W1, H1) + window(300, 110, 1060, 680, "Documents", "files", True)
r, rects, pos = rail_collapsed(H1, open_item="clock")
npanel = [f'<g filter="url(#pop)"><rect x="72" y="8" width="360" height="{H1-16}" rx="16" fill="#FCFCFD" fill-opacity="0.985"/></g>']
npanel.append(text(92, 46, "Notifications", 17, 700))
npanel.append(text(412, 46, "Clear All", 12.5, 500, ACCENT, "end"))
cards = [("mail", "Mail", "Anna Berg", "Draft of the bar spec attached", "2 min"), ("chat", "Chat", "Design team · 12 new", "“Left by default looks right on 16:10”", "9 min"), ("cal", "Calendar", "Review at 15:00", "Combined bar walkthrough", "1 h")]
for i, (k, a, t1, t2, age) in enumerate(cards):
    cy = 70 + i * 92
    npanel.append(f'<rect x="84" y="{cy}" width="336" height="80" rx="12" fill="#FFFFFF" stroke="rgba(0,0,0,0.06)"/>')
    npanel.append(icon(k, 96, cy + 12, 28))
    npanel.append(text(134, cy + 24, t1, 13, 600))
    npanel.append(text(134, cy + 43, t2, 12.5, 400))
    npanel.append(text(134, cy + 62, a, 11, 400, MUTED))
    npanel.append(text(408, cy + 24, age, 11, 400, MUTED, "end"))
svg(W1, H1, nb + r + "".join(npanel), "08-notification-center.svg")
