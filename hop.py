#!/usr/bin/env python3
"""herdr-hop: Vimium-style hint jump for Herdr.

Two modes, both drawn inside Herdr's own terminal popup so the outer
terminal never loses focus:

  sidebar (default)  Hint labels appear *in the sidebar itself*, next to every
                     space row and agent row, via Herdr's metadata tokens.
                     A tiny HUD popup captures the key and lists the targets
                     the sidebar does not show (panes of the current tab,
                     tabs of the current workspace).
  list               One big popup with a pane mini-map and full lists.

Entrypoints:
  hop.py                 sidebar-mode HUD (manifest pane "hop")
  hop.py --list          list-mode popup (manifest pane "list")
  hop.py --open          open the sidebar-mode popup, sized to fit
  hop.py --open-list     open the list-mode popup
  hop.py --back          jump back to the pane focused before the last hop
  hop.py --dump          render one list-mode frame to stdout (debugging)
  hop.py --probe         write the popup size to the state dir (debugging)
"""

import json
import os
import select
import signal
import socket
import subprocess
import sys
import termios
import tty
import unicodedata
import time

_TRACE = os.environ.get("HOP_TRACE")


def trace(label):
    if _TRACE:
        with open(_TRACE, "a") as f:
            f.write(f"{label} {time.time():.4f}\n")

# Home-row first. "q" is deliberately absent so it can quit the popup.
ALPHABET = "asdfghjklwertyuiopzxcvbnm"
TOKEN = "hint"  # sidebar rows render this as $hint
SOURCE = "zed.hop"
TOKEN_TTL_MS = 30000  # backstop: labels vanish on their own if we die

RESET = "\x1b[0m"
BOLD = "\x1b[1m"
DIM = "\x1b[2m"
HINT = "\x1b[1;30;48;5;220m"  # black on yellow, like Vimium
HINT_TYPED = "\x1b[1;30;48;5;208m"  # darker orange for the typed prefix
TITLE = "\x1b[1;38;5;110m"
FOCUS = "\x1b[38;5;214m"
STATUS_COLORS = {
    "working": "\x1b[38;5;114m",
    "running": "\x1b[38;5;114m",
    "idle": "\x1b[38;5;245m",
    "waiting": "\x1b[38;5;214m",
    "blocked": "\x1b[38;5;203m",
    "error": "\x1b[38;5;203m",
    "done": "\x1b[38;5;75m",
}
STATUS_ICONS = {
    "working": "●",
    "running": "●",
    "idle": "○",
    "waiting": "◐",
    "blocked": "◆",
    "error": "✗",
    "done": "✓",
}


# --------------------------------------------------------------------------
# Herdr socket API
# --------------------------------------------------------------------------


def socket_path():
    return os.environ.get("HERDR_SOCKET_PATH") or os.path.expanduser(
        "~/.config/herdr/herdr.sock"
    )


def rpc(method, params=None):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(socket_path())
    try:
        payload = {"id": "hop", "method": method, "params": params or {}}
        s.sendall((json.dumps(payload) + "\n").encode())
        buf = b""
        while b"\n" not in buf:
            chunk = s.recv(1 << 16)
            if not chunk:
                break
            buf += chunk
    finally:
        s.close()
    resp = json.loads(buf.split(b"\n", 1)[0] or b"{}")
    if "error" in resp:
        raise RuntimeError(f"{method}: {resp['error'].get('message')}")
    return resp.get("result", {})


def snapshot():
    return rpc("session.snapshot")["snapshot"]


def plugin_context():
    raw = os.environ.get("HERDR_PLUGIN_CONTEXT_JSON")
    if not raw:
        return {}
    try:
        return json.loads(raw)
    except ValueError:
        return {}


def plugin_id():
    return os.environ.get("HERDR_PLUGIN_ID", "zed.hop")


def state_dir():
    d = os.environ.get("HERDR_PLUGIN_STATE_DIR")
    if not d:
        d = os.path.expanduser("~/.local/state/herdr/plugins/zed.hop")
    os.makedirs(d, exist_ok=True)
    return d


def remember_previous(pane_id):
    if not pane_id:
        return
    with open(os.path.join(state_dir(), "last.json"), "w") as f:
        json.dump({"pane_id": pane_id}, f)


def read_previous():
    try:
        with open(os.path.join(state_dir(), "last.json")) as f:
            return json.load(f).get("pane_id")
    except (OSError, ValueError):
        return None


# --------------------------------------------------------------------------
# Text helpers (CJK aware)
# --------------------------------------------------------------------------


def cwidth(ch):
    if unicodedata.combining(ch):
        return 0
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


def dwidth(s):
    return sum(cwidth(c) for c in s)


def truncate(s, width):
    if width <= 0:
        return ""
    if dwidth(s) <= width:
        return s
    out = ""
    w = 0
    for ch in s:
        cw = cwidth(ch)
        if w + cw > width - 1:
            break
        out += ch
        w += cw
    return out + "…"


def short_path(p):
    if not p:
        return ""
    home = os.path.expanduser("~")
    if p == home:
        return "~"
    if p.startswith(home + "/"):
        return "~" + p[len(home):]
    return p


def visible_len(s):
    """Display width of a string ignoring ANSI escapes."""
    out = 0
    i = 0
    while i < len(s):
        if s[i] == "\x1b":
            j = s.find("m", i)
            i = j + 1 if j != -1 else len(s)
            continue
        out += cwidth(s[i])
        i += 1
    return out


def clip_ansi(s, width):
    """Cut an ANSI string to a display width, keeping escapes intact."""
    out = ""
    w = 0
    i = 0
    while i < len(s):
        if s[i] == "\x1b":
            j = s.find("m", i)
            out += s[i : j + 1]
            i = j + 1
            continue
        cw = cwidth(s[i])
        if w + cw > width:
            break
        out += s[i]
        w += cw
        i += 1
    return out + RESET


# --------------------------------------------------------------------------
# Model
# --------------------------------------------------------------------------


def make_labels(n):
    if n <= len(ALPHABET):
        return list(ALPHABET[:n])
    labels = []
    for a in ALPHABET:
        for b in ALPHABET:
            labels.append(a + b)
            if len(labels) == n:
                return labels
    return labels


def status_str(status):
    status = status or "unknown"
    color = STATUS_COLORS.get(status, DIM)
    icon = STATUS_ICONS.get(status, "·")
    return f"{color}{icon} {status}{RESET}"


class Model:
    """Everything the UI needs, computed once from a session snapshot.

    mode="sidebar": labels follow the sidebar's visual order (each space,
    then the agents inside it), then panes of the current tab, then tabs of
    the current workspace.
    mode="list": current-tab panes first, then agents, then spaces/tabs.
    """

    def __init__(self, snap, ctx, mode="sidebar"):
        self.mode = mode
        self.focused_pane = snap.get("focused_pane_id")
        self.current_tab = ctx.get("tab_id") or snap.get("focused_tab_id")
        self.current_ws = ctx.get("workspace_id") or snap.get("focused_workspace_id")

        self.panes = {p["pane_id"]: p for p in snap.get("panes", [])}
        self.agents = {a["pane_id"]: a for a in snap.get("agents", [])}
        self.tabs = {t["tab_id"]: t for t in snap.get("tabs", [])}
        self.workspaces = {w["workspace_id"]: w for w in snap.get("workspaces", [])}
        self.layout = next(
            (l for l in snap.get("layouts", []) if l["tab_id"] == self.current_tab),
            None,
        )
        self.tab_panes = []
        if self.layout:
            self.tab_panes = sorted(
                self.layout.get("panes", []),
                key=lambda p: (p["rect"]["y"], p["rect"]["x"]),
            )
        self.ws_rows = []  # (workspace, [tabs]) in sidebar order
        for ws in sorted(self.workspaces.values(), key=lambda w: w.get("number", 0)):
            tabs = sorted(
                (t for t in self.tabs.values() if t["workspace_id"] == ws["workspace_id"]),
                key=lambda t: t.get("number", 0),
            )
            self.ws_rows.append((ws, tabs))
        self.current_ws_tabs = next(
            (tabs for ws, tabs in self.ws_rows if ws["workspace_id"] == self.current_ws), []
        )

        self.dests = []
        seen = set()

        def add(kind, ident):
            key = (kind, ident)
            if key not in seen:
                seen.add(key)
                self.dests.append(key)

        def tab_number(pane_id):
            t = self.tabs.get(self.panes.get(pane_id, {}).get("tab_id"), {})
            return t.get("number", 0)

        if mode == "sidebar":
            for ws, tabs in self.ws_rows:
                add("workspace", ws["workspace_id"])
                agents = [a for a in self.agents.values() if a["workspace_id"] == ws["workspace_id"]]
                for a in sorted(agents, key=lambda a: (tab_number(a["pane_id"]), a["pane_id"])):
                    add("pane", a["pane_id"])
            for p in self.tab_panes:
                add("pane", p["pane_id"])
            if len(self.current_ws_tabs) > 1:
                for t in self.current_ws_tabs:
                    add("tab", t["tab_id"])
        else:
            for p in self.tab_panes:
                add("pane", p["pane_id"])
            order = {"blocked": 0, "waiting": 1, "error": 2, "done": 3, "working": 4, "idle": 5}
            for a in sorted(
                self.agents.values(),
                key=lambda a: (order.get(a.get("agent_status"), 9), a["pane_id"]),
            ):
                add("pane", a["pane_id"])
            for ws, tabs in self.ws_rows:
                if len(tabs) == 1:
                    add("tab", tabs[0]["tab_id"])
                else:
                    add("workspace", ws["workspace_id"])
                    for t in tabs:
                        add("tab", t["tab_id"])

        labels = make_labels(len(self.dests))
        self.label_of = dict(zip(self.dests, labels))
        self.dest_of = dict(zip(labels, self.dests))

    def label(self, kind, ident):
        return self.label_of.get((kind, ident), "")

    def jump(self, dest):
        kind, ident = dest
        remember_previous(self.focused_pane)
        if kind == "pane":
            rpc("pane.focus", {"pane_id": ident})
        elif kind == "tab":
            rpc("tab.focus", {"tab_id": ident})
        elif kind == "workspace":
            rpc("workspace.focus", {"workspace_id": ident})

    def pane_summary(self, pane_id):
        """(primary, secondary) text for a pane."""
        p = self.panes.get(pane_id, {})
        a = self.agents.get(pane_id)
        if a:
            primary = a.get("name") or a.get("display_agent") or a.get("agent") or "agent"
            secondary = a.get("terminal_title_stripped") or short_path(a.get("cwd"))
        else:
            primary = p.get("label") or os.path.basename(p.get("cwd") or "") or "shell"
            secondary = p.get("terminal_title_stripped") or short_path(p.get("cwd"))
        return primary, secondary

    # Sidebar tokens --------------------------------------------------------

    def sidebar_targets(self):
        """(method, id_key, kind, id) for every destination the sidebar shows."""
        for kind, ident in self.dests:
            if kind == "workspace":
                yield "workspace.report_metadata", "workspace_id", kind, ident
            elif kind == "pane" and ident in self.agents:
                yield "pane.report_metadata", "pane_id", kind, ident

    def publish_hints(self, typed=""):
        """Push (or refresh) the $hint token on every sidebar row."""
        for method, key, kind, ident in self.sidebar_targets():
            label = self.label_of[(kind, ident)]
            value = f"[{label}]" if label.startswith(typed) else None
            params = {key: ident, "source": SOURCE, "tokens": {TOKEN: value}}
            if value is not None:
                params["ttl_ms"] = TOKEN_TTL_MS
            try:
                rpc(method, params)
            except (RuntimeError, OSError):
                pass

    def clear_hints(self):
        for method, key, _kind, ident in self.sidebar_targets():
            try:
                rpc(method, {key: ident, "source": SOURCE, "tokens": {TOKEN: None}})
            except (RuntimeError, OSError):
                pass


# --------------------------------------------------------------------------
# Rendering
# --------------------------------------------------------------------------


def render_hint(label, typed):
    """Hint box, dimmed when it no longer matches the typed prefix."""
    if typed and not label.startswith(typed):
        return f"{DIM}[{label}]{RESET}"
    if typed:
        return f"{HINT_TYPED}[{typed}{HINT}{label[len(typed):]}]{RESET}"
    return f"{HINT}[{label}]{RESET}"


def hud_lines(model, typed):
    """Sidebar-mode HUD: header plus whatever the sidebar cannot show."""
    header = f"{TITLE}{BOLD}hop{RESET} {DIM}hints are in the sidebar · ` back · esc{RESET}"
    if typed:
        header += f"  {HINT_TYPED} {typed} {RESET}"
    lines = [header]
    if len(model.tab_panes) > 1:
        parts = []
        for p in model.tab_panes:
            primary, _ = model.pane_summary(p["pane_id"])
            a = model.agents.get(p["pane_id"])
            icon = STATUS_ICONS.get(a.get("agent_status"), "·") + " " if a else ""
            cur = BOLD if p["pane_id"] == model.focused_pane else ""
            parts.append(f"{render_hint(model.label('pane', p['pane_id']), typed)} {cur}{icon}{truncate(primary, 14)}{RESET}")
        lines.append(f"{DIM}panes{RESET} " + "  ".join(parts))
    if len(model.current_ws_tabs) > 1:
        parts = []
        for t in model.current_ws_tabs:
            cur = BOLD if t["tab_id"] == model.current_tab else ""
            parts.append(f"{render_hint(model.label('tab', t['tab_id']), typed)} {cur}{truncate(t.get('label', ''), 12)}{RESET}")
        lines.append(f"{DIM}tabs {RESET} " + "  ".join(parts))
    return lines


def render_minimap(model, width, height, typed):
    """Scaled drawing of the current tab's pane layout as a char grid."""
    lines = [[" "] * width for _ in range(height)]
    styles = [[""] * width for _ in range(height)]
    if not model.layout or width < 8 or height < 3:
        return ["".join(row) for row in lines]

    area = model.layout["area"]
    aw, ah = max(1, area["width"]), max(1, area["height"])

    def put(y, x, ch, style=""):
        if 0 <= y < height and 0 <= x < width:
            lines[y][x] = ch
            styles[y][x] = style

    def put_text(y, x, text, style=""):
        for ch in text:
            put(y, x, ch, style)
            x += cwidth(ch)

    for p in model.tab_panes:
        r = p["rect"]
        x0 = round((r["x"] - area["x"]) / aw * width)
        x1 = round((r["x"] + r["width"] - area["x"]) / aw * width) - 1
        y0 = round((r["y"] - area["y"]) / ah * height)
        y1 = round((r["y"] + r["height"] - area["y"]) / ah * height) - 1
        x1 = min(max(x1, x0 + 5), width - 1)
        y1 = min(max(y1, y0 + 1), height - 1)
        focused = p["pane_id"] == model.focused_pane
        st = FOCUS if focused else DIM
        for x in range(x0, x1 + 1):
            put(y0, x, "─", st)
            put(y1, x, "─", st)
        for y in range(y0, y1 + 1):
            put(y, x0, "│", st)
            put(y, x1, "│", st)
        put(y0, x0, "┌", st)
        put(y0, x1, "┐", st)
        put(y1, x0, "└", st)
        put(y1, x1, "┘", st)

        inner_w = x1 - x0 - 1
        primary, secondary = model.pane_summary(p["pane_id"])
        label = model.label("pane", p["pane_id"])
        put(y0, x0 + 1, f"\x00{label}")  # marker, expanded below
        text_rows = y1 - y0 - 1
        if text_rows >= 1:
            a = model.agents.get(p["pane_id"])
            head = f"{STATUS_ICONS.get(a.get('agent_status'), '·')} {primary}" if a else primary
            put_text(y0 + 1, x0 + 1, truncate(head, inner_w), BOLD if focused else "")
        if text_rows >= 2 and secondary:
            put_text(y0 + 2, x0 + 1, truncate(secondary, inner_w), DIM)

    out = []
    for y in range(height):
        row = ""
        x = 0
        while x < width:
            ch = lines[y][x]
            if ch.startswith("\x00"):
                label = ch[1:]
                row += render_hint(label, typed)
                x += len(label) + 2
                continue
            st = styles[y][x]
            row += f"{st}{ch}{RESET}" if st else ch
            if cwidth(ch) == 2:
                x += 1
            x += 1
        out.append(row)
    return out


def render_lists(model, width, typed):
    """List-mode right column: workspaces/tabs then agents."""
    lines = [f"{TITLE}TABS{RESET}"]
    for ws, tabs in model.ws_rows:
        cur = ws["workspace_id"] == model.current_ws
        mark = f"{FOCUS}▸{RESET}" if cur else " "
        if len(tabs) == 1:
            t = tabs[0]
            hint = render_hint(model.label("tab", t["tab_id"]), typed)
            st = status_str(t.get("agent_status")) if t.get("agent_status") not in (None, "unknown") else ""
            lines.append(f"{mark}{hint} {BOLD if cur else ''}{truncate(ws.get('label', ''), 24)}{RESET} {st}")
        else:
            hint = render_hint(model.label("workspace", ws["workspace_id"]), typed)
            lines.append(f"{mark}{hint} {BOLD if cur else ''}{truncate(ws.get('label', ''), 24)}{RESET}")
            for t in tabs:
                th = render_hint(model.label("tab", t["tab_id"]), typed)
                tcur = t["tab_id"] == model.current_tab
                st = status_str(t.get("agent_status")) if t.get("agent_status") not in (None, "unknown") else ""
                lines.append(f"   {th} {BOLD if tcur else ''}{truncate(t.get('label', ''), 20)}{RESET} {DIM}{t.get('pane_count', 1)}p{RESET} {st}")

    if model.agents:
        lines.append("")
        lines.append(f"{TITLE}AGENTS{RESET}")
        for key in model.dests:
            if key[0] != "pane" or key[1] not in model.agents:
                continue
            a = model.agents[key[1]]
            hint = render_hint(model.label("pane", key[1]), typed)
            ws = model.workspaces.get(a["workspace_id"], {}).get("label", "")
            name = a.get("name") or a.get("display_agent") or a.get("agent") or "agent"
            title = a.get("terminal_title_stripped") or ""
            mark = f"{FOCUS}▸{RESET}" if key[1] == model.focused_pane else " "
            head = f"{mark}{hint} {BOLD}{truncate(name, 10)}{RESET} {status_str(a.get('agent_status'))}"
            tail = f"{DIM}{truncate(ws, 14)}{RESET}  {truncate(title, max(8, width - 46))}"
            lines.append(f"{head}  {tail}")
    return lines


def render_list_frame(model, typed, cols, rows):
    header = f"{TITLE}{BOLD} hop {RESET}{DIM}type a hint to jump · ` back · q/esc close{RESET}"
    if typed:
        header += f"  {HINT_TYPED} {typed} {RESET}"
    body_rows = rows - 2
    lines = [header, ""]
    side_by_side = cols >= 90 and model.layout and len(model.tab_panes) > 1
    if side_by_side:
        left_w = max(24, min(int(cols * 0.42), 70))
        right_w = cols - left_w - 3
        left = render_minimap(model, left_w, body_rows, typed)
        right = render_lists(model, right_w, typed)
        for i in range(body_rows):
            l = left[i] if i < len(left) else " " * left_w
            r = right[i] if i < len(right) else ""
            lines.append(f"{l}{RESET} {DIM}│{RESET} {clip_ansi(r, right_w)}")
    else:
        right = render_lists(model, cols, typed)
        for i in range(body_rows):
            r = right[i] if i < len(right) else ""
            lines.append(clip_ansi(r, cols))
    return lines[:rows]


def term_size():
    try:
        sz = os.get_terminal_size(sys.stdout.fileno())
        if sz.columns > 0 and sz.lines > 0:
            return sz.columns, sz.lines
    except OSError:
        pass
    return int(os.environ.get("COLUMNS", 120)), int(os.environ.get("LINES", 30))


def draw(lines):
    cols, rows = term_size()
    out = "\x1b[?25l\x1b[H"
    frame = [clip_ansi(l, cols) for l in lines[:rows]]
    for i, line in enumerate(frame):
        out += line + "\x1b[K"
        if i < len(frame) - 1:
            out += "\r\n"
    sys.stdout.write(out)
    sys.stdout.flush()


# --------------------------------------------------------------------------
# Input loop
# --------------------------------------------------------------------------


def read_key(fd):
    """Return one key: a printable char, 'esc', 'bs', 'ctrl-c', 'seq', or None."""
    ch = os.read(fd, 1)
    if not ch:
        return None
    if ch == b"\x1b":
        if select.select([fd], [], [], 0.03)[0]:
            while select.select([fd], [], [], 0.01)[0]:
                os.read(fd, 32)
            return "seq"
        return "esc"
    if ch == b"\x03":
        return "ctrl-c"
    if ch in (b"\x7f", b"\x08"):
        return "bs"
    try:
        return ch.decode()
    except UnicodeDecodeError:
        return None


def run_popup(mode):
    trace("popup:start")
    model = Model(snapshot(), plugin_context(), mode)
    trace("popup:model")
    sidebar = mode == "sidebar"
    fd = sys.stdin.fileno()
    old = termios.tcgetattr(fd)
    typed = ""

    def on_signal(signum, frame):
        raise SystemExit(0)

    for sig in (signal.SIGHUP, signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, on_signal)

    sys.stdout.write("\x1b[?1049h")
    try:
        tty.setraw(fd)
        if sidebar:
            model.publish_hints()
            trace("popup:hints")
        while True:
            if sidebar:
                draw(hud_lines(model, typed))
            else:
                cols, rows = term_size()
                draw(render_list_frame(model, typed, cols, rows))
            trace("popup:drawn")
            key = read_key(fd)
            if key in (None, "esc", "ctrl-c", "q"):
                return 0
            if key == "seq":
                continue
            if key == "bs":
                typed = typed[:-1]
                if sidebar:
                    model.publish_hints(typed)
                continue
            if key in ("`", "'"):
                prev = read_previous()
                if prev and prev in model.panes:
                    model.jump(("pane", prev))
                return 0
            if key in ("\r", "\n"):
                matches = [l for l in model.dest_of if l.startswith(typed)]
                if len(matches) == 1:
                    model.jump(model.dest_of[matches[0]])
                    return 0
                continue
            if len(key) == 1 and key.lower() in ALPHABET:
                typed += key.lower()
                if typed in model.dest_of:
                    model.jump(model.dest_of[typed])
                    return 0
                if not any(l.startswith(typed) for l in model.dest_of):
                    typed = ""
                if sidebar:
                    model.publish_hints(typed)
    finally:
        if sidebar:
            model.clear_hints()
        termios.tcsetattr(fd, termios.TCSADRAIN, old)
        sys.stdout.write("\x1b[?25h\x1b[?1049l")
        sys.stdout.flush()


# --------------------------------------------------------------------------
# Entrypoints
# --------------------------------------------------------------------------


def open_popup(mode):
    """Open the popup through the socket, sized to the HUD's content."""
    params = {"plugin_id": plugin_id(), "placement": "popup"}
    if mode == "sidebar":
        model = Model(snapshot(), plugin_context(), "sidebar")
        lines = hud_lines(model, "")
        width = max(visible_len(l) for l in lines) + 4
        params.update({"entrypoint": "hop", "width": min(width, 120), "height": len(lines) + 2})
    else:
        params.update({"entrypoint": "list"})
    try:
        rpc("plugin.pane.open", params)
    except RuntimeError as e:
        print(f"hop: {e}", file=sys.stderr)
        return 1
    return 0


def jump_back():
    prev = read_previous()
    if not prev:
        print("hop: nothing to jump back to", file=sys.stderr)
        return 1
    snap = snapshot()
    if prev not in {p["pane_id"] for p in snap.get("panes", [])}:
        print(f"hop: previous pane {prev} is gone", file=sys.stderr)
        return 1
    remember_previous(snap.get("focused_pane_id"))
    rpc("pane.focus", {"pane_id": prev})
    return 0


def dump(argv):
    mode = "sidebar" if "--sidebar" in argv else "list"
    model = Model(snapshot(), plugin_context(), mode)
    cols = int(os.environ.get("COLUMNS", 160))
    rows = int(os.environ.get("LINES", 30))
    typed = os.environ.get("HOP_TYPED", "")
    lines = hud_lines(model, typed) if mode == "sidebar" else render_list_frame(model, typed, cols, rows)
    print("\n".join(lines) + RESET)
    print("labels:", {k: f"{v[0]}:{v[1]}" for k, v in model.dest_of.items()})
    return 0


def main(argv):
    if "--open" in argv:
        return open_popup("sidebar")
    if "--open-list" in argv:
        return open_popup("list")
    if "--back" in argv:
        return jump_back()
    if "--dump" in argv:
        return dump(argv)
    if "--probe" in argv:
        with open(os.path.join(state_dir(), "probe.txt"), "w") as f:
            f.write(f"size={term_size()}\n")
        return 0
    mode = "list" if "--list" in argv else "sidebar"
    try:
        return run_popup(mode)
    except Exception as e:  # keep the popup from dying silently
        sys.stdout.write("\x1b[?25h\x1b[?1049l")
        sys.stderr.write(f"hop: {e}\n")
        sys.stderr.flush()
        try:
            os.read(sys.stdin.fileno(), 1)
        except OSError:
            pass
        return 1


if __name__ == "__main__":
    trace("main:start")
    sys.exit(main(sys.argv[1:] + os.environ.get("HOP_ARGS", "").split()))
