"""Sleek redesign of the Organise screens, applied to the open Sketchframe project through its MCP server.

    python3 build2.py && python3 mcp.py ../organise.sketch.json calls2.json .

Clears and redraws every screen in place (keeps the file and its version history), renames them into order,
adds the 11th screen if missing, then exports PNGs + the HTML prototype and saves a version.
"""
import json

# ---- palette ----
ACC = "#3b6fd8"
WORK, PERS, SIDE = "#3b6fd8", "#2f9e62", "#8b5cf6"
TXT, TXT2, TXT3 = "#1f2328", "#6b7280", "#9ca3af"
SIDEBAR, HOVER, SEL, CHIP, LINE = "#f6f6f7", "#f5f7fa", "#eaf1fd", "#f1f3f5", "#e5e7eb"
AMBER, AMBER_BG, GREEN = "#b45309", "#fdf3e1", "#2f9e62"

def rect(x, y, w, h, fill, border=None, **kw):
    return dict(type="rect", x=x, y=y, w=w, h=h, fill=fill, color=border or fill, **kw)

def label(x, y, text, size=14, color=TXT, w=None, bold=False, **kw):
    return dict(type="label", x=x, y=y, w=w or int(len(text) * size * 0.56 + 24), h=size + 8,
                text=text, font_size=size, color=color, bold=bold, **kw)

def icon(x, y, name, size=18, color=TXT2, **kw):
    return dict(type="icon", x=x, y=y, w=size, h=size, text=name, color=color, **kw)

def dot(x, y, color, size=8):
    return dict(type="circle", x=x, y=y, w=size, h=size, fill=color, color=color)

def pill(x, y, text, ic=None, fill=CHIP, color="#374151", size=12, h=24, opacity=None):
    """Borderless rounded chip with an optional icon. Returns (controls, next_x)."""
    w = int(len(text) * size * 0.56 + (34 if ic else 18))
    extra = {"opacity": opacity} if opacity else {}
    out = [rect(x, y, w, h, fill, **extra)]
    tx = x + 9
    if ic:
        out.append(icon(x + 8, y + (h - 14) / 2, ic, 14, color, **extra))
        tx = x + 26
    out.append(dict(type="label", x=tx, y=y + (h - size - 8) / 2, w=w - (tx - x), h=size + 8, text=text,
                    font_size=size, color=color, **extra))
    return out, x + w + 6

def softbutton(x, y, w, h, text, ic=None, size=13, fill=CHIP, color=TXT):
    out = [rect(x, y, w, h, fill)]
    tw = len(text) * size * 0.56 + (22 if ic else 0)
    sx = x + (w - tw) / 2
    if ic:
        out.append(icon(sx, y + (h - 16) / 2, ic, 16, color))
        sx += 22
    out.append(dict(type="label", x=sx, y=y + (h - size - 8) / 2, w=tw + 10, h=size + 8, text=text, font_size=size, color=color))
    return out

def primary(x, y, w, h, text, size=13):
    return [dict(type="button_primary", x=x, y=y, w=w, h=h, text=text, font_size=size)]

def section(x, y, text):
    return label(x, y, text.upper(), 11, TXT3, bold=True)

# ---------------- desktop sidebar ----------------
def sidebar(active="lists", list_sel="Q4 budget review", attention=True):
    c = [rect(0, 0, 248, 900, SIDEBAR, name="sidebar")]
    c += [icon(18, 20, "check", 20, ACC), label(44, 17, "Organise", 17, TXT, bold=True, link="1 Lists")]
    c += [rect(12, 56, 224, 34, "#ececee"), icon(22, 64, "search", 16, TXT3), label(44, 61, "Search", 13, TXT3),
          label(178, 61, "Ctrl K", 12, TXT3, w=52)]
    def nav(y, ic, text, key, link, count=None):
        out = []
        if active == key:
            out.append(rect(8, y - 5, 232, 32, "#e9eaee"))
        out += [icon(20, y + 1, ic, 18, TXT if active == key else TXT2),
                label(46, y - 1, text, 14, TXT, bold=(active == key), link=link)]
        if count:
            out.append(label(200, y, count, 12, TXT3, w=30, align="right"))
        return out
    c += nav(108, "list", "Lists", "lists", "1 Lists")
    c += nav(140, "grid", "Board", "board", "4 Board", "5")
    groups = [("Work", WORK, ["Q4 budget review", "Hiring – backend dev", "Hybris upgrade"]),
              ("Personal", PERS, ["House move", "Tax return 2026", "Car service"]),
              ("Side projects", SIDE, ["Garden shed plans"])]
    y = 194
    for g, col, items in groups:
        c += [dot(20, y + 5, col), section(32, y - 2, g)]
        y += 26
        for it in items:
            if it == list_sel and active == "lists":
                c.append(rect(8, y - 5, 232, 30, "#e9eaee"))
            c.append(label(32, y - 1, it, 14, TXT, bold=(it == list_sel and active == "lists")))
            y += 30
        y += 12
    c += [icon(20, y + 1, "plus", 16, TXT3), label(42, y - 1, "New list", 13, TXT3)]
    if attention:
        p, _ = pill(14, 812, "1 link needs attention", "warning", AMBER_BG, AMBER, 12, 28)
        for e in p:
            e["link"] = "5 Moved & missing"
        c += p
    c += nav(856, "settings", "Settings", "settings", "6 Settings")
    return c

# ---------------- list rows ----------------
ROWS = [
    (0, "Collect department forecasts", True, [("Forecasts_Q4.xlsx", "file"), ("HR headcount.csv", "file")]),
    (0, "Reconcile with actuals", False, []),
    (1, "Pull SAP export", True, [("sap_export_oct.csv", "file")]),
    (1, "Check travel spend", False, [("RE: Travel policy changes", "mail"), ("travel_q3.xlsx", "file")]),
    (1, "Ask Dana about licence costs", False, [("Hiring – backend dev", "link")]),
    (0, "Draft budget pack", False, []),
    (1, "Slides for exec meeting", False, [("Budget_pack_v3.pptx", "file")]),
    (1, "Appendix tables", False, [("Budget / Appendix", "folder")]),
    (0, "Review with finance", False, [("FIN-212", "globe")]),
    (0, "Submit to CFO", False, []),
]

def list_body(x0, max_x, hover="Check travel spend", drop=None):
    c, y, rows_y = [], 168, {}
    for ind, text, done, chips in ROWS:
        x = x0 + ind * 30
        rows_y[text] = y
        if text == hover:
            c += [rect(x0 - 34, y - 7, max_x - x0 + 44, 38, HOVER),
                  icon(x0 - 28 + ind * 30, y + 2, "more_v", 16, TXT3),
                  icon(max_x - 18, y + 2, "chevron_right", 18, TXT3)]
        if text == drop:
            c += [rect(x0 - 34, y - 7, max_x - x0 + 44, 38, SEL), rect(x0 - 34, y - 7, 3, 38, ACC)]
        tw = int(len(text) * 8.0 + 34)
        c.append(dict(type="checkbox", x=x, y=y, w=tw, h=24, text=text, state="selected" if done else "normal",
                      font_size=15, **({"color": TXT3} if done else {})))
        cx = x + tw + 8
        for i, (t, ic) in enumerate(chips):
            p, nx = pill(cx, y, t, ic)
            if nx > max_x - 30:
                p, nx = pill(cx, y, f"+{len(chips) - i}")
                c += p
                break
            c += p
            cx = nx
        y += 42
    c += [icon(x0 + 3, y + 2, "plus", 16, TXT3), label(x0 + 28, y, "New item", 14, TXT3)]
    return c, rows_y

def list_header(x0, right):
    c = [dot(x0, 54, WORK), label(x0 + 12, 46, "Work", 12, TXT2),
         dict(type="heading", x=x0 - 4, y=66, w=520, h=44, text="Q4 budget review", font_size=30, color=TXT),
         label(x0, 116, "In progress  ·  Due 12 Dec  ·  4 of 9 done", 13, TXT2, w=300),
         dict(type="progress", x=x0 + 300, y=124, w=120, h=6, value=44, color=ACC)]
    c += [icon(right - 64, 20, "info", 20, TXT2), icon(right - 30, 20, "more", 20, TXT2)]
    return c

def lists_screen():
    c = sidebar()
    c += list_header(340, 1440)
    body, _ = list_body(340, 1200)
    return c + body

# ---------------- 2 item details ----------------
def details_screen():
    c = sidebar()
    c += list_header(310, 1030)
    body, _ = list_body(310, 1000)
    c += body
    px = 1050
    c += [rect(1030, 0, 410, 900, "#ffffff", LINE, name="details panel"),
          icon(1400, 18, "close", 18, TXT2),
          label(px, 24, "Q4 budget review  ›  Reconcile with actuals", 12, TXT3, w=340),
          dict(type="subheading", x=px - 2, y=46, w=360, h=34, text="Check travel spend", font_size=22, color=TXT),
          section(px, 104, "Links"), label(1368, 101, "+ Add", 12, ACC, w=50)]
    # email link card
    c += [rect(px, 128, 370, 64, "#f7f8fa"), icon(px + 14, 140, "mail", 20, AMBER),
          label(px + 46, 136, "RE: Travel policy changes", 14, TXT, bold=True, w=300),
          label(px + 46, 160, "Outlook  ·  Dana Smith  ·  3 Nov", 12, TXT2, w=300),
          icon(px + 340, 150, "more", 16, TXT3)]
    c += [rect(px, 200, 370, 64, "#f7f8fa"), icon(px + 14, 212, "file", 20, ACC),
          label(px + 46, 208, "travel_q3.xlsx", 14, TXT, bold=True, w=300),
          label(px + 46, 232, "~/OneDrive/Finance/2026  ·  moved 2 days ago", 12, TXT2, w=300),
          icon(px + 340, 222, "more", 16, TXT3)]
    c += [section(px, 290, "Notes"),
          rect(px, 314, 370, 120, "#f7f8fa"),
          dict(type="paragraph", x=px + 12, y=324, w=346, h=100, font_size=14, color=TXT,
               text="Dana says the new per-diem rates start in January.")]
    c += [rect(px, 456, 370, 1, LINE), label(px, 470, "More details", 14, TXT, bold=True, w=150),
          icon(px + 346, 472, "chevron_down", 18, TXT2),
          label(px, 494, "Category, board column, due date, linked from", 12, TXT3, w=340)]
    return c

# ---------------- 3 drag in + right-click ----------------
def drop_screen():
    c = sidebar()
    c += list_header(340, 1440)
    body, ry = list_body(340, 1200, hover=None, drop="Draft budget pack")
    c += body
    y = ry["Draft budget pack"]
    c += [rect(600, y - 2, 220, 32, "#ffffff", "#c7d2fe", opacity=0.95, name="drag ghost"),
          icon(612, y + 6, "file", 16, ACC), label(634, y + 2, "Contract_signed.pdf", 13, TXT, w=180)]
    t, _ = pill(828, y + 2, "Link here", "link", ACC, "#ffffff", 12)
    c += t
    y2 = ry["Slides for exec meeting"]
    c += [dict(type="menu", x=620, y=y2 + 30, w=220, h=196,
               text=":file Open\tEnter\n:folder Show in folder\n:link Copy path\n---\n:more More…\n:trash Remove link")]
    c += [dict(type="toast", x=600, y=820, w=340, h=46, text="Linked Contract_signed.pdf\tUndo")]
    return c

# ---------------- 4 board ----------------
def card(x, y, title, cat, done, total, links, w=196, ghost=False, note=None):
    col = {"Work": WORK, "Personal": PERS, "Side": SIDE}[cat]
    o = {"opacity": 0.75} if ghost else {}
    h = 96 if note else 78
    c = [rect(x, y, w, h, "#ffffff", LINE, **o),
         label(x + 14, y + 10, title, 14, TXT, bold=True, w=w - 24, **o),
         dot(x + 15, y + 41, col, 7), label(x + 27, y + 34, cat if cat != "Side" else "Side projects", 12, TXT2, w=110, **o),
         icon(x + w - 50, y + 36, "link", 14, TXT3), label(x + w - 34, y + 33, str(links), 12, TXT3, w=30),
         dict(type="progress", x=x + 14, y=y + 60, w=w - 64, h=5, value=int(100 * done / max(total, 1)), color=col, **o),
         label(x + w - 44, y + 53, f"{done}/{total}", 11, TXT3, w=36, align="right")]
    if note:
        c.append(label(x + 14, y + 72, note, 12, AMBER, w=w - 24))
    return c

def board_screen():
    c = sidebar(active="board", attention=True)
    c += [dict(type="heading", x=276, y=24, w=200, h=44, text="Board", font_size=30, color=TXT)]
    fx = 400
    for i, t in enumerate(["All", "Work", "Personal", "Side projects"]):
        w = int(len(t) * 7.4 + 24)
        if i == 0:
            c.append(rect(fx, 34, w, 28, "#e9eaee"))
        c.append(label(fx + 12, 37, t, 13, TXT if i == 0 else TXT2, bold=(i == 0), w=w))
        fx += w + 6
    c += softbutton(1250, 30, 120, 34, "Add list", "plus")
    c += [icon(1392, 38, "more", 20, TXT2)]
    cols = [("To do", 2, TXT3), ("In progress", 3, ACC), ("Blocked", 1, "#dc2626"), ("Under review", 1, "#d97706"), ("Done", 1, GREEN)]
    xs = [276, 500, 724, 948, 1172]
    for (name, n, col), x in zip(cols, xs):
        c += [rect(x, 84, 212, 800, "#f7f7f8"), dot(x + 14, 103, col), label(x + 28, 95, name, 14, TXT, bold=True),
              label(x + 28 + len(name) * 8.2 + 10, 96, str(n), 13, TXT3, w=24)]
    c += card(284, 128, "Tax return 2026", "Personal", 1, 6, 4)
    c += card(284, 216, "Car service", "Personal", 0, 3, 1)
    c += card(508, 128, "Q4 budget review", "Work", 4, 9, 7)
    c += card(508, 216, "Hiring – backend dev", "Work", 5, 6, 3)
    c += [rect(508, 304, 196, 78, SEL, "#c7d2fe")]
    c += card(560, 360, "House move", "Personal", 7, 15, 12, ghost=True)
    c += card(732, 128, "Hybris upgrade", "Work", 2, 8, 9, note="Waiting on vendor patch")
    c += card(956, 128, "Kitchen quotes", "Personal", 3, 3, 5)
    c += card(1180, 128, "Passport renewal", "Personal", 4, 4, 2)
    c += [label(1186, 216, "+ 6 more this month", 12, TXT3, w=180)]
    # column menu (hover reveals ⋯ on the column header)
    c += [icon(914, 96, "more", 18, TXT), dict(type="menu", x=760, y=122, w=180, h=168,
          text=":edit Rename\n:arrow_left Move left\n:arrow_right Move right\n---\n:more More…\n-:trash Delete column"),
          dict(type="tooltip", x=740, y=292, w=240, h=40, text="Move its 1 list out first")]
    c += [rect(1396, 84, 36, 36, "#f7f7f8"), icon(1405, 93, "plus", 18, TXT3)]
    return c

# ---------------- 5 moved & missing ----------------
def health_screen():
    c = sidebar(active="none", list_sel=None)
    x = 340
    c += [dict(type="heading", x=x - 4, y=40, w=500, h=44, text="Links", font_size=30, color=TXT),
          label(x, 90, "1,284 linked files and emails. Moves are followed automatically.", 14, TXT2, w=600)]
    c += [section(x, 140, "Needs attention")]
    c += [rect(x, 164, 780, 150, "#ffffff", LINE), icon(x + 18, 182, "warning", 20, AMBER),
          label(x + 50, 178, "Budget_pack_v2.pptx can't be found", 15, TXT, bold=True, w=400),
          label(x + 50, 202, "Linked from Q4 budget review  ·  last seen 21 Nov in ~/Documents/Budget", 12, TXT2, w=600),
          icon(x + 50, 234, "search", 16, TXT2),
          label(x + 74, 230, "Likely match: ~/Archive/2026/Budget_pack_v2.pptx (same size and content)", 13, TXT, w=600)]
    c += primary(x + 50, 264, 110, 34, "Use this")
    c += softbutton(x + 170, 264, 130, 34, "Choose file…")
    c += [icon(x + 316, 272, "more", 18, TXT2)]
    c += [section(x, 350, "Recently moved"), label(x + 700, 347, "Show all", 12, ACC, w=80)]
    rows = [("file", "travel_q3.xlsx", "~/Downloads  →  ~/OneDrive/Finance/2026", "Today"),
            ("file", "Contract_signed.pdf", "renamed from Contract_final.pdf", "Yesterday"),
            ("mail", "RE: Travel policy changes", "Inbox  →  Inbox / Finance", "Mon"),
            ("folder", "Hybris", "~/Projects/old  →  ~/Projects/clients", "Fri")]
    y = 378
    for ic, name, how, when in rows:
        c += [icon(x + 4, y + 10, ic, 18, TXT2), label(x + 34, y + 2, name, 14, TXT, w=300),
              label(x + 34, y + 22, how, 12, TXT2, w=500), label(x + 690, y + 10, when, 12, TXT3, w=90, align="right"),
              rect(x, y + 50, 780, 1, LINE)]
        y += 60
    return c

# ---------------- 6 settings / 7 advanced ----------------
def settings_common():
    c = sidebar(active="settings", list_sel=None)
    x = 340
    c += [dict(type="heading", x=x - 4, y=40, w=400, h=44, text="Settings", font_size=30, color=TXT)]
    c += [section(x, 110, "Your data"),
          label(x, 136, "Data folder", 14, TXT, w=200), label(x, 158, "~/Organise  ·  plain text, Markdown + YAML", 12, TXT2, w=400)]
    c += softbutton(x + 540, 138, 110, 32, "Open", "folder")
    c += softbutton(x + 660, 138, 120, 32, "Change…")
    c += [rect(x, 190, 780, 1, LINE), label(x, 204, "Backup", 14, TXT, w=200),
          label(x, 226, "A .zip of the data folder. Restoring keeps a copy of what's there now.", 12, TXT2, w=500)]
    c += primary(x + 540, 206, 110, 32, "Export")
    c += softbutton(x + 660, 206, 120, 32, "Restore…")
    c += [section(x, 280, "Categories")]
    cx = x
    for t, col in [("Work", WORK), ("Personal", PERS), ("Side projects", SIDE)]:
        w = int(len(t) * 7.3 + 40)
        c += [rect(cx, 304, w, 30, CHIP), dot(cx + 12, 315, col, 8), label(cx + 26, 307, t, 13, TXT, w=w - 26)]
        cx += w + 6
    p, _ = pill(cx, 304, "Add", "plus", "#ffffff", TXT2, 13, 30)
    c += p
    c += [section(x, 360, "Board columns")]
    cx = x
    for t in ["To do", "In progress", "Blocked", "Under review", "Done"]:
        p, cx = pill(cx, 384, t, None, CHIP, TXT, 13, 30)
        c += p
    p, _ = pill(cx, 384, "Add", "plus", "#ffffff", TXT2, 13, 30)
    c += p
    c += [section(x, 440, "Email"), icon(x, 466, "mail", 18, TXT2),
          label(x + 28, 463, "Outlook on this PC", 14, TXT, w=200),
          label(x + 28, 485, "Drag emails onto items. No sign-in needed.", 12, TXT2, w=400)]
    t, _ = pill(x + 690, 466, "Connected", "check", "#e7f6ee", GREEN, 12)
    c += t
    c += [rect(x, 520, 780, 1, LINE)]
    return c

def settings_screen():
    x = 340
    c = settings_common()
    c += [label(x, 536, "Advanced", 15, TXT, bold=True, w=120), icon(x + 756, 538, "chevron_right", 18, TXT2),
          label(x, 560, "Watched folders, file tracking, file format, devices", 12, TXT3, w=400)]
    return c

def advanced_screen():
    x = 340
    c = settings_common()
    c += [label(x, 536, "Advanced", 15, TXT, bold=True, w=120), icon(x + 756, 538, "chevron_down", 18, TXT2)]
    c += [label(x, 572, "Watched folders", 14, TXT, w=200),
          label(x, 594, "Moves inside these are seen instantly. Others are found by fingerprint.", 12, TXT2, w=500)]
    y = 624
    for f, n in [("~ (home)", "1,102 links"), ("/data/projects", "160 links"), ("/mnt/nas/shared", "22 links · checked every 15 min")]:
        c += [rect(x, y, 780, 36, "#f7f8fa"), icon(x + 12, y + 9, "folder", 18, TXT2), label(x + 40, y + 6, f, 14, TXT, w=260),
              label(x + 330, y + 8, n, 12, TXT2, w=300), icon(x + 750, y + 10, "close", 16, TXT3)]
        y += 42
    c += [icon(x + 4, y + 6, "plus", 16, ACC), label(x + 26, y + 2, "Add folder", 13, ACC, w=100)]
    y += 44
    c += [dict(type="checkbox", x=x, y=y, w=420, h=24, text="Skip hidden folders, node_modules, caches", state="selected", font_size=14),
          label(x, y + 40, "If a linked file is missing", 14, TXT, w=240),
          dict(type="dropdown", x=x + 540, y=y + 34, w=240, h=34, text="Search, then ask me", font_size=13),
          label(x, y + 84, "File format", 14, TXT, w=200), label(x + 680, y + 84, "Show example", 13, ACC, w=110),
          label(x, y + 120, "Sync & devices", 14, TXT3, w=200), label(x + 700, y + 120, "Coming later", 12, TXT3, w=90)]
    return c

# ---------------- phones ----------------
def phone_top(title, left="menu", right="more_v"):
    return [dict(type="status_bar", x=0, y=0, w=412, h=40, text="9:41"),
            icon(18, 56, left, 22, TXT), icon(372, 56, right, 22, TXT)]

def phone_nav(active):
    items = [(":list Lists", "list"), (":grid Board", "board"), (":search Search", "search"), (":settings Settings", "settings")]
    txt = ",".join(("*" if k == active else "") + t for t, k in items)
    return [dict(type="bottom_nav", x=0, y=841, w=412, h=74, text=txt)]

def phone_list(dim_pc=True):
    c = phone_top("")
    c += [dot(20, 104, WORK), label(32, 96, "Work", 12, TXT2),
          dict(type="heading", x=14, y=114, w=380, h=40, text="Q4 budget review", font_size=26, color=TXT),
          label(18, 158, "In progress  ·  4 of 9 done", 13, TXT2, w=300)]
    y = 200
    rows = [(0, "Collect department forecasts", True, [("Forecasts_Q4.xlsx", "file", True)]),
            (0, "Reconcile with actuals", False, []),
            (1, "Pull SAP export", True, []),
            (1, "Check travel spend", False, [("RE: Travel policy", "mail", False), ("travel_q3.xlsx", "file", False)]),
            (1, "Ask Dana about licence costs", False, []),
            (0, "Draft budget pack", False, []),
            (1, "Slides for exec meeting", False, [("Budget_pack_v3.pptx", "file", True)]),
            (0, "Review with finance", False, [("FIN-212", "globe", False)])]
    for ind, t, done, chips in rows:
        x = 18 + ind * 26
        c.append(dict(type="checkbox", x=x, y=y, w=380 - x, h=26, text=t, state="selected" if done else "normal",
                      font_size=15, **({"color": TXT3} if done else {})))
        y += 32
        if chips:
            cx = x + 30
            for name, ic, pc_only in chips:
                p, nx = pill(cx, y, name, ic, CHIP, "#374151", 12, 24, 0.5 if (pc_only and dim_pc) else None)
                c += p
                cx = nx
            y += 34
        else:
            y += 6
    c += [dict(type="fab", x=336, y=762, w=56, h=56, text="plus")]
    return c + phone_nav("list")

def phone_fetch():
    c = phone_list()
    c += [rect(0, 0, 412, 915, "#000000", opacity=0.35, name="scrim"),
          rect(0, 470, 412, 445, "#ffffff", name="sheet"), rect(176, 480, 60, 5, "#d1d5db"),
          icon(20, 506, "file", 28, ACC),
          label(58, 502, "Budget_pack_v3.pptx", 18, TXT, bold=True, w=330),
          label(58, 530, "PowerPoint  ·  2.4 MB  ·  changed 21 Nov", 12, TXT2, w=330),
          dot(22, 576, GREEN, 9), label(38, 568, "On OMAR-PC  ·  online now", 14, TXT, w=300)]
    c += primary(18, 610, 376, 48, ":download Get a copy now", 15)
    c += [rect(18, 676, 376, 1, LINE), label(18, 690, "More options", 14, TXT, w=200), icon(374, 692, "chevron_down", 18, TXT2),
          label(18, 714, "Keep on this phone, send when the PC is online", 12, TXT3, w=376)]
    return c

def phone_board():
    c = phone_top("")
    c += [dict(type="heading", x=14, y=96, w=300, h=40, text="Board", font_size=26, color=TXT)]
    x = 18
    for i, t in enumerate(["To do 2", "In progress 3", "Blocked 1", "Review 1"]):
        w = int(len(t) * 8.2 + 20)
        c.append(label(x, 150, t, 14, TXT if i == 1 else TXT2, bold=(i == 1), w=w))
        if i == 1:
            c.append(rect(x, 176, w - 14, 3, ACC))
        x += w + 4
    c += [rect(0, 182, 412, 1, LINE)]
    c += card(18, 204, "Q4 budget review", "Work", 4, 9, 7, w=376)
    c += card(18, 296, "Hiring – backend dev", "Work", 5, 6, 3, w=376)
    c += card(18, 388, "House move", "Personal", 7, 15, 12, w=376)
    c += [dict(type="page_dots", x=166, y=490, w=80, h=16, value=2)]
    return c + phone_nav("board")

def phone_share():
    c = [dict(type="status_bar", x=0, y=0, w=412, h=40, text="9:41"),
         icon(18, 56, "chevron_left", 22, TXT), label(50, 54, "Gmail", 17, TXT, bold=True),
         label(18, 110, "RE: Garage door quote", 16, TXT, bold=True, w=380),
         dict(type="paragraph", x=18, y=140, w=376, h=80, color=TXT2, font_size=14, text="Hi Omar, please find the quote attached…"),
         rect(0, 0, 412, 915, "#000000", opacity=0.35, name="scrim"),
         rect(0, 430, 412, 485, "#ffffff", name="sheet"), rect(176, 440, 60, 5, "#d1d5db"),
         label(18, 462, "Add to Organise", 18, TXT, bold=True, w=300)]
    p, _ = pill(18, 494, "RE: Garage door quote", "mail")
    c += p
    c += [rect(18, 532, 376, 40, "#f1f3f5"), icon(30, 543, "search", 18, TXT3), label(56, 540, "Find a list or item", 14, TXT3, w=200)]
    y = 588
    for t, sel in [("Inbox", False), ("House move  ›  Garage", True), ("House move  ›  Quotes", False), ("Car service", False)]:
        if sel:
            c.append(rect(10, y - 6, 392, 40, SEL))
        c += [icon(24, y + 4, "list", 18, TXT2), label(54, y + 1, t, 15, TXT, bold=sel, w=320)]
        if sel:
            c.append(icon(372, y + 5, "check", 18, ACC))
        y += 44
    c += primary(18, 780, 376, 48, "Add link", 15)
    return c

# ---------------- screen table: (existing id or None, name, device, w, h, notes, builder) ----------------
SCREENS = [
    ("s2", "1 Lists", "window", 1440, 900, "Main view. Sidebar: search, Lists/Board, lists grouped by category, a status pill only when a link needs attention. Hovering a row shows the drag handle and the open-details arrow.", lists_screen),
    ("s3", "2 Item details", "window", 1440, 900, "Details open on demand (click the arrow or press Space). Shows links and notes. Category, board column, due date and backlinks sit under More details.", details_screen),
    ("s4", "3 Drag in & right-click", "window", 1440, 900, "Drag a file from the file manager onto an item: only a link is saved. Dropping between rows makes a new item. Right-click a link: Open, Show in folder (file pre-selected), Copy path; More… holds Re-link and Open with.", drop_screen),
    ("s5", "4 Board", "window", 1440, 900, "Cards are lists. Column menu appears on hover; Delete is disabled while a column holds cards. Board ⋯ holds rarely used options (show sub-lists as cards, lists not on the board).", board_screen),
    ("s6", "5 Moved & missing", "window", 1440, 900, "Only reached from the sidebar pill when something needs attention. Moves are followed automatically; this page lists what couldn't be.", health_screen),
    ("s7", "6 Settings", "window", 1440, 900, "Everyday settings only. Advanced is collapsed.", settings_screen),
    ("s8", "7 Settings – advanced", "window", 1440, 1000, "Advanced expanded: watched folders, missing-file behaviour, file format, devices.", advanced_screen),
    ("s11", "8 Phone – list", "android", 412, 915, "Links to files that only exist on a PC are dimmed.", phone_list),
    ("s9", "9 Phone – file on another PC", "android", 412, 915, "Tapping a PC-only file. Get a copy fetches it peer-to-peer; the rarer options are under More options.", phone_fetch),
    ("s10", "10 Phone – board", "android", 412, 915, "One column at a time; tabs switch columns.", phone_board),
    ("s12", "11 Phone – share in", "android", 412, 915, "The phone equivalent of drag-and-drop: Share › Organise.", phone_share),
]

if __name__ == "__main__":
    D = "/home/omar/work/AI/organise/design"
    calls = []
    for sid, name, dev, w, h, notes, _ in SCREENS:
        if sid:
            calls.append({"name": "update_screen", "arguments": {"screen": sid, "name": name, "device": dev, "width": w, "height": h, "notes": notes}})
            calls.append({"name": "clear_screen", "arguments": {"screen": sid}})
        else:
            calls.append({"name": "create_screen", "arguments": {"name": name, "device": dev, "width": w, "height": h, "notes": notes}})
    for sid, name, dev, w, h, notes, fn in SCREENS:
        calls.append({"name": "add_controls", "arguments": {"screen": name, "controls": fn()}})
    for sid, name, *_ in SCREENS:
        fn = name.replace(" ", "_").replace("–", "-").replace("&", "and")
        calls.append({"name": "export", "arguments": {"format": "png", "screen": name, "path": f"{D}/screens/{fn}.png", "sketch": False}})
    calls.append({"name": "export", "arguments": {"format": "html", "path": D + "/prototype.html", "sketch": False}})
    calls.append({"name": "save_version", "arguments": {"name": "Sleek redesign"}})
    json.dump(calls, open("calls2.json", "w"))
