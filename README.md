# herdr-hop

Vimium-style hint jump for [Herdr](https://herdr.dev).

Press one key and every space and agent row **in the sidebar itself** gets a
yellow label. Type the label and you are there. Panes of the current tab and
tabs of the current workspace get their labels in a tiny HUD popup, because
the sidebar has no rows for them.

Everything is drawn by Herdr: the sidebar labels are metadata tokens, the HUD
is a Herdr terminal popup. There is no separate window, so the outer terminal
(Ghostty, iTerm2, WezTerm, Kitty, …) never loses focus, and it works over
`herdr --remote` / SSH.

```
 sidebar                          │  HUD popup (centered over the panes)
 ─────────────────────────────    │  ┌──────────────────────────────────────────┐
 ● [a] herdr                      │  │ hop  hints are in the sidebar · ` back   │
   ● [s] herdr 1                  │  │ panes [y] ● claude  [u] shell            │
     claude                       │  │ tabs  [i] 1  [o] 2                       │
 ○ [d] liberbot                   │  └──────────────────────────────────────────┘
   ○ [f] liberbot 1
     claude
   ○ [g] liberbot 2
     codex
 · [h] study
```

Labels are single letters while there are 25 or fewer targets, two letters
beyond that. The alphabet is home-row first (`asdfghjkl`, `wertyuiop`,
`zxcvbnm`); `q` is reserved for quitting.

## Requirements

- Herdr 0.8.2 or newer (plugin popup placement, metadata tokens)
- Python 3.8+, standard library only, no build step. The `hop` launcher
  prefers `/opt/homebrew/bin/python3`, `/usr/local/bin/python3`, then
  `/usr/bin/python3`, and only then `python3` from `PATH`, because pyenv,
  asdf, and mise shims add about 100 ms to every start. Override with
  `HOP_PYTHON=/path/to/python3` or by writing the path into
  `$(herdr plugin config-dir zed.hop)/python`.
- The expanded desktop sidebar (collapsed and mobile sidebars do not render
  custom tokens)

## Install

```bash
herdr plugin install <owner>/herdr-hop
```

or, for a local checkout:

```bash
herdr plugin link /path/to/herdr-hop
```

### Config

Two things in `~/.config/herdr/config.toml`: a key binding, and the `$hint`
token in the sidebar rows so Herdr has somewhere to draw the labels. The
token is empty whenever hop is not running, and empty tokens take no space.

```toml
[[keys.command]]
key = "prefix+f"
type = "plugin_action"
command = "zed.hop.open"
description = "hop: hint jump"

# Faster alternative to the binding above (about 60 ms to first frame instead
# of about 200 ms): let Herdr open the popup directly. Use the absolute path
# of the checkout, or the managed one from `herdr plugin list`.
# [[keys.command]]
# key = "prefix+f"
# type = "popup"
# command = "/path/to/herdr-hop/hop"
# width = 64
# height = 5
# description = "hop: hint jump"

# optional: jump back to where you were before the last hop
[[keys.command]]
key = "prefix+shift+f"
type = "plugin_action"
command = "zed.hop.back"
description = "hop: jump back"

# optional: the older all-in-one list popup with a pane mini-map
[[keys.command]]
key = "prefix+alt+f"
type = "plugin_action"
command = "zed.hop.open-list"
description = "hop: list popup"

# These are Herdr's default rows plus the $hint token. If you already
# customise rows, just add the token entry wherever you want the label.
[ui.sidebar.spaces]
rows = [["state_icon", { token = "$hint", fg = "#f9e2af", bold = true }, "workspace"], ["branch", "git_status"]]

[ui.sidebar.agents]
rows = [["state_icon", { token = "$hint", fg = "#f9e2af", bold = true }, "workspace", "tab"], ["agent"]]
```

```bash
herdr server reload-config
```

## Usage

Press `prefix+f`.

- Every space row and agent row in the sidebar shows `[x]`. Type `x` to jump
  there. A space label focuses that workspace's active tab; an agent label
  focuses that agent's pane, switching workspace and tab as needed.
- The HUD lists panes of the current tab (when there is more than one) and
  tabs of the current workspace (when there is more than one).
- With two-letter labels, typing the first letter hides every label that no
  longer matches, in the sidebar and in the HUD.
- `` ` `` (backtick) jumps back to the pane focused before the last hop.
- `Backspace` clears the typed prefix, `Enter` accepts a unique prefix.
- `q`, `Esc`, or `Ctrl-C` closes without jumping.

## How it works

1. The `open` action asks Herdr to open the `hop` popup (a fixed 64x5 cells);
   nothing else runs before the popup process itself.
2. The popup process reads the session snapshot over the Herdr socket,
   assigns labels in sidebar order, and publishes the labels as `hint` metadata tokens
   (`workspace.report_metadata` / `pane.report_metadata`) with a 30 second
   TTL, so labels disappear on their own even if the process is killed.
3. A keystroke becomes one `pane.focus`, `tab.focus`, or `workspace.focus`
   request. On exit the tokens are cleared explicitly.
4. The previous pane id is written to `HERDR_PLUGIN_STATE_DIR/last.json` for
   jump-back.

## Development

```bash
herdr plugin link "$PWD"
python3 hop.py --dump --sidebar               # HUD text + label map
COLUMNS=140 LINES=30 python3 hop.py --dump    # list-mode frame
herdr plugin action invoke zed.hop.open
HOP_TRACE=/tmp/hop-trace.txt ./hop          # per-stage timestamps
```

Latency on an M-series Mac: popup process starts about 45-70 ms after the
request and draws its first frame about 10 ms later.

## License

MIT
