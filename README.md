# herdr-easyjump

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
 ● [a] herdr                      │  │ easyjump  hints in the sidebar · ` back  │
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
- A Rust toolchain (`cargo`); `herdr plugin install` runs
  `cargo build --release`. Four small dependencies: serde, serde_json, libc,
  unicode-width. No TUI framework.
- The expanded desktop sidebar (collapsed and mobile sidebars do not render
  custom tokens)

## Install

```bash
herdr plugin install <owner>/herdr-easyjump
```

or, for a local checkout:

```bash
cargo build --release
herdr plugin link /path/to/herdr-easyjump
```

### Config

Two things in `~/.config/herdr/config.toml`: a key binding, and the `$hint`
token in the sidebar rows so Herdr has somewhere to draw the labels. The
token is empty whenever easyjump is not running, and empty tokens take no space.

```toml
[[keys.command]]
key = "prefix+f"
type = "plugin_action"
command = "xzedx.easyjump.open"
description = "easyjump: hint jump"

# Faster alternative to the binding above (about 20 ms to first frame instead
# of about 150 ms): let Herdr open the popup directly. Use the absolute path
# of the checkout, or the managed one from `herdr plugin list`.
# [[keys.command]]
# key = "prefix+f"
# type = "popup"
# command = "/path/to/herdr-easyjump/target/release/herdr-easyjump"
# width = 64
# height = 5
# description = "easyjump: hint jump"

# optional: jump back to where you were before the last hop
[[keys.command]]
key = "prefix+shift+f"
type = "plugin_action"
command = "xzedx.easyjump.back"
description = "easyjump: jump back"

# optional: the older all-in-one list popup with a pane mini-map
[[keys.command]]
key = "prefix+alt+f"
type = "plugin_action"
command = "xzedx.easyjump.open-list"
description = "easyjump: list popup"

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

1. The `open` action asks Herdr to open the `jump` popup (a fixed 64x5 cells);
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
cargo build --release
herdr plugin link "$PWD"
./target/release/herdr-easyjump --dump --sidebar               # HUD text + label map
COLUMNS=140 LINES=30 ./target/release/herdr-easyjump --dump    # list-mode frame
herdr plugin action invoke xzedx.easyjump.open
EASYJUMP_TRACE=/tmp/easyjump-trace.txt ./target/release/herdr-easyjump  # per-stage timestamps
```

Latency on an M-series Mac, measured from the `plugin.pane.open` request:
the popup process starts after about 10-15 ms and has drawn its first frame
and published every sidebar label after about 15-25 ms. The binary itself
starts in about 3 ms; the rest is Herdr spawning the popup.

The first version was Python (see git history). It worked the same way but
needed 60-80 ms, most of it interpreter start-up.

## License

MIT
