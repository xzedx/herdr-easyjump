//! herdr-easyjump: Vimium-style hint jump for Herdr.
//!
//! Two modes, both drawn inside Herdr's own terminal popup so the outer
//! terminal never loses focus:
//!
//!   sidebar (default)  Hint labels appear where the eye already is: next to
//!                      every space row and agent row in the sidebar (metadata
//!                      tokens), on the border of every pane of the current tab
//!                      (title override), and in the tab bar (a label prefix on
//!                      custom-named tabs; auto-numbered tabs use their number).
//!                      A tiny HUD popup captures the key and repeats the pane
//!                      and tab hints. When the sidebar is collapsed the HUD
//!                      lists the spaces and agents too.
//!   list               One big popup with a pane mini-map and full lists.
//!
//! Usage:
//!   herdr-easyjump                 sidebar-mode HUD (manifest pane "jump")
//!   herdr-easyjump --list          list-mode popup (manifest pane "list")
//!   herdr-easyjump --open          open the sidebar-mode popup, sized to fit
//!   herdr-easyjump --open-list     open the list-mode popup
//!   herdr-easyjump --back          jump back to the pane focused before the last hop
//!   herdr-easyjump --clear         clear every hint token (detached cleanup helper)
//!   herdr-easyjump --dump [--sidebar]   render one frame to stdout (debugging)
//!   herdr-easyjump --probe         write the popup size to the state dir (debugging)

mod model;
mod render;
mod rpc;
mod state;
mod term;

use model::{Context, Kind, Mode, Model, Snapshot, ALPHABET, REFRESH_MS};
use rpc::Client;
use serde_json::json;
use state::trace;
use std::sync::atomic::Ordering;
use term::Key;

fn plugin_id() -> String {
    std::env::var("HERDR_PLUGIN_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "xzedx.easyjump".into())
}

fn load_model(client: &mut Client, mode: Mode, ctx: Context) -> Result<Model, String> {
    let snap: Snapshot =
        serde_json::from_value(client.snapshot()?).map_err(|e| format!("snapshot: {e}"))?;
    Ok(Model::new(snap, ctx, mode))
}

/// After a relative move: re-read the session so the HUD and labels follow
/// the new focus, and refresh the sidebar tokens.
fn reload(model: &mut Model, client: &mut Client, mode: Mode, typed: &str) {
    if let Ok(m) = load_model(client, mode, Context::default()) {
        *model = m;
    }
    if mode == Mode::Sidebar {
        model.publish_hints(client, typed);
    }
}

enum Outcome {
    Jump(model::Dest),
    Back,
    /// Enter: keep whatever focus the relative moves produced.
    Confirm,
    /// Esc / q: undo relative moves, back to where the popup opened.
    Cancel,
    /// Herdr closed the popup from outside; keep the focus as it is.
    Quit,
}

/// The interactive loop. Returns what to do; the caller handles cleanup.
fn interact(model: &mut Model, client: &mut Client, mode: Mode) -> Outcome {
    let sidebar = mode == Mode::Sidebar;
    let mut typed = String::new();
    if sidebar {
        model.publish_hints(client, "");
        trace("popup:hints");
    }
    loop {
        if sidebar {
            let (cols, _) = term::size();
            term::draw(&render::hud_lines(model, &typed, cols));
        } else {
            let (cols, rows) = term::size();
            term::draw(&render::render_list_frame(model, &typed, cols, rows));
        }
        trace("popup:drawn");
        if term::INTERRUPTED.load(Ordering::SeqCst) {
            return Outcome::Quit;
        }
        let key = match term::read_key_timeout(REFRESH_MS) {
            Some(k) => k,
            None => {
                // Idle: refresh the tokens so their TTL never lapses while open.
                if sidebar {
                    model.publish_hints(client, &typed);
                }
                continue;
            }
        };
        trace(&format!("popup:key:{key:?}"));
        match key {
            Key::Eof => {
                // Herdr closed the popup from outside and will kill us within a
                // millisecond or two: hand the cleanup to a detached helper.
                if sidebar {
                    spawn_clear_helper();
                }
                return Outcome::Quit;
            }
            Key::Esc | Key::CtrlC | Key::Char('q') => return Outcome::Cancel,
            Key::Seq => {}
            Key::Backspace => {
                typed.pop();
                if sidebar {
                    model.publish_hints(client, &typed);
                }
            }
            Key::Char('`') | Key::Char('\'') => return Outcome::Back,
            // Relative movement, Vim style: panes with hjkl, spaces with J/K.
            Key::Char(c @ ('h' | 'j' | 'k' | 'l')) => {
                let dir = match c {
                    'h' => "left",
                    'j' => "down",
                    'k' => "up",
                    _ => "right",
                };
                let _ = client.call("pane.focus_direction", json!({ "direction": dir }));
                reload(model, client, mode, &typed);
            }
            Key::Char(c @ ('J' | 'K')) => {
                let step = if c == 'J' { 1 } else { -1 };
                if let Some(ws) = model.neighbour_workspace(step) {
                    let _ = client.call("workspace.focus", json!({ "workspace_id": ws }));
                    reload(model, client, mode, &typed);
                }
            }
            Key::Enter => {
                let matches = model.labels_with_prefix(&typed);
                if !typed.is_empty() && matches.len() == 1 {
                    if let Some(d) = model.dest_for(matches[0]) {
                        return Outcome::Jump(d.clone());
                    }
                }
                return Outcome::Confirm;
            }
            // Letters are labels; digits are the keys of auto-numbered tabs.
            Key::Char(c) if ALPHABET.contains(c.to_ascii_lowercase()) || c.is_ascii_digit() => {
                typed.push(c.to_ascii_lowercase());
                if let Some(d) = model.dest_for(&typed) {
                    return Outcome::Jump(d.clone());
                }
                if model.labels_with_prefix(&typed).is_empty() {
                    typed.clear();
                }
                if sidebar {
                    model.publish_hints(client, &typed);
                }
            }
            Key::Char(_) => {}
        }
    }
}

/// Start `herdr-easyjump --clear` in its own process group so it survives us.
fn spawn_clear_helper() {
    use std::os::unix::process::CommandExt;
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe)
            .arg("--clear")
            .process_group(0)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

fn clear_tokens() -> Result<(), String> {
    let mut client = Client::connect()?;
    let snap: Snapshot = serde_json::from_value(client.snapshot()?).map_err(|e| e.to_string())?;
    Model::clear_all(&mut client, &snap);
    trace("clear:done");
    Ok(())
}

fn run_popup(mode: Mode) -> Result<(), String> {
    trace("popup:start");
    let mut client = Client::connect()?;
    let mut model = load_model(&mut client, mode, Context::from_env())?;
    let origin = model.focused_pane.clone();
    trace("popup:model");
    term::install_signal_handlers();
    let raw = term::RawMode::enable();
    term::enter_alt_screen();

    let outcome = interact(&mut model, &mut client, mode);
    trace("popup:quit");

    if mode == Mode::Sidebar {
        model.clear_hints(&mut client);
        trace("popup:cleared");
    }
    drop(raw);
    term::leave_alt_screen();

    // "Previous" always means where we were when the popup opened, even
    // after a chain of relative moves. A jump-back or a cancel must not
    // overwrite it.
    let moved = model.focused_pane != origin;
    let remember = match &outcome {
        Outcome::Jump(_) => true,
        Outcome::Confirm | Outcome::Quit => moved,
        Outcome::Back | Outcome::Cancel => false,
    };
    if remember {
        state::remember_previous(origin.as_deref());
    }
    match outcome {
        Outcome::Jump(dest) => model.jump(&mut client, &dest),
        Outcome::Back => match state::read_previous() {
            Some(prev) if model.panes.contains_key(&prev) => {
                model.jump(&mut client, &(Kind::Pane, prev))
            }
            _ => Ok(()),
        },
        Outcome::Cancel => match origin {
            Some(o) if moved && model.panes.contains_key(&o) => {
                model.jump(&mut client, &(Kind::Pane, o))
            }
            _ => Ok(()),
        },
        Outcome::Confirm | Outcome::Quit => Ok(()),
    }
}

/// Ask Herdr to open the popup, sized to the HUD's content.
fn open_popup(mode: Mode) -> Result<(), String> {
    let mut client = Client::connect()?;
    let mut params = json!({ "plugin_id": plugin_id(), "placement": "popup" });
    match mode {
        Mode::Sidebar => {
            let model = load_model(&mut client, Mode::Sidebar, Context::from_env())?;
            // Fit the content, but stay inside the outer terminal when known.
            let max_w = if model.screen_cols > 12 {
                (model.screen_cols - 8).min(120)
            } else {
                120
            };
            let max_h = if model.screen_rows > 6 {
                model.screen_rows - 4
            } else {
                usize::MAX
            };
            let lines = render::hud_lines(&model, "", max_w - 4);
            let width = lines
                .iter()
                .map(|l| render::visible_len(l))
                .max()
                .unwrap_or(40)
                + 4;
            params["entrypoint"] = json!("jump");
            params["width"] = json!(width.min(max_w));
            params["height"] = json!((lines.len() + 2).min(max_h));
        }
        Mode::List => params["entrypoint"] = json!("list"),
    }
    client.call("plugin.pane.open", params).map(|_| ())
}

fn jump_back() -> Result<(), String> {
    let Some(prev) = state::read_previous() else {
        return Err("nothing to jump back to".into());
    };
    let mut client = Client::connect()?;
    let snap: Snapshot = serde_json::from_value(client.snapshot()?).map_err(|e| e.to_string())?;
    if !snap.panes.iter().any(|p| p.pane_id == prev) {
        return Err(format!("previous pane {prev} is gone"));
    }
    state::remember_previous(snap.focused_pane_id.as_deref());
    client
        .call("pane.focus", json!({ "pane_id": prev }))
        .map(|_| ())
}

fn dump(args: &[String]) -> Result<(), String> {
    let mode = if args.iter().any(|a| a == "--sidebar") {
        Mode::Sidebar
    } else {
        Mode::List
    };
    let mut client = Client::connect()?;
    let model = load_model(&mut client, mode, Context::from_env())?;
    let env = |k: &str, d: usize| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };
    let typed = std::env::var("EASYJUMP_TYPED").unwrap_or_default();
    let lines = match mode {
        Mode::Sidebar => render::hud_lines(&model, &typed, env("COLUMNS", 100)),
        Mode::List => {
            render::render_list_frame(&model, &typed, env("COLUMNS", 160), env("LINES", 30))
        }
    };
    println!("{}{}", lines.join("\n"), render::RESET);
    println!(
        "sidebar: {} cols ({}), screen: {}x{}",
        model.sidebar_cols,
        if model.sidebar_collapsed() { "collapsed" } else { "expanded" },
        model.screen_cols,
        model.screen_rows
    );
    let map: Vec<String> = model
        .labels
        .iter()
        .zip(&model.dests)
        .map(|(l, (k, id))| format!("{l}:{k:?}:{id}"))
        .collect();
    println!("labels: {}", map.join(" "));
    Ok(())
}

fn main() {
    trace("main:start");
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if let Ok(extra) = std::env::var("EASYJUMP_ARGS") {
        args.extend(extra.split_whitespace().map(String::from));
    }
    let has = |flag: &str| args.iter().any(|a| a == flag);

    let result = if has("--open") {
        open_popup(Mode::Sidebar)
    } else if has("--open-list") {
        open_popup(Mode::List)
    } else if has("--back") {
        jump_back()
    } else if has("--clear") {
        clear_tokens()
    } else if has("--dump") {
        dump(&args)
    } else if has("--probe") {
        let (c, r) = term::size();
        std::fs::write(
            state::state_dir().join("probe.txt"),
            format!("size=({c}, {r})\n"),
        )
        .map_err(|e| e.to_string())
    } else {
        let mode = if has("--list") {
            Mode::List
        } else {
            Mode::Sidebar
        };
        run_popup(mode)
    };

    if let Err(e) = result {
        term::leave_alt_screen();
        eprintln!("easyjump: {e}");
        std::process::exit(1);
    }
}
