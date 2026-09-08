//! Session snapshot types and the label model.

use crate::rpc::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// Home-row first. "h", "j", "k", "l" are reserved for relative movement and
/// "q" for quitting, so they never appear in a label.
pub const ALPHABET: &str = "asdfgwertyuiopzxcvbnm";
pub const TOKEN: &str = "hint"; // sidebar rows render this as $hint
pub const SOURCE: &str = "xzedx.easyjump";
pub const TOKEN_TTL_MS: u64 = 15_000; // backstop: labels vanish on their own if we die
pub const REFRESH_MS: i32 = 5_000; // re-publish while open so the TTL never expires under us
/// Herdr does not report whether the sidebar is open. The pane area of every
/// tab starts to the right of it, so its x offset is the sidebar's width: about
/// 30 columns when expanded, 4 when collapsed to the icon rail, 0 when hidden.
/// Anything narrower than this cannot show a `[xx]` label next to a name.
pub const SIDEBAR_MIN_COLS: i64 = 12;

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct LayoutPane {
    pub pane_id: String,
    pub rect: Rect,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Layout {
    pub tab_id: String,
    #[serde(default)]
    pub area: Rect,
    #[serde(default)]
    pub panes: Vec<LayoutPane>,
}

#[derive(Deserialize, Clone, Default, Debug)]
pub struct Workspace {
    pub workspace_id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub number: i64,
}

#[derive(Deserialize, Clone, Default, Debug)]
pub struct Tab {
    pub tab_id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub number: i64,
    #[serde(default)]
    pub pane_count: i64,
    #[serde(default)]
    pub agent_status: Option<String>,
}

#[derive(Deserialize, Clone, Default, Debug)]
pub struct Pane {
    pub pane_id: String,
    #[serde(default)]
    pub tab_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub terminal_title_stripped: Option<String>,
}

#[derive(Deserialize, Clone, Default, Debug)]
pub struct Agent {
    pub pane_id: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub display_agent: Option<String>,
    #[serde(default)]
    pub agent_status: Option<String>,
    #[serde(default)]
    pub terminal_title_stripped: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Deserialize, Default, Debug)]
pub struct Snapshot {
    #[serde(default)]
    pub focused_pane_id: Option<String>,
    #[serde(default)]
    pub focused_tab_id: Option<String>,
    #[serde(default)]
    pub focused_workspace_id: Option<String>,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub tabs: Vec<Tab>,
    #[serde(default)]
    pub panes: Vec<Pane>,
    #[serde(default)]
    pub agents: Vec<Agent>,
    #[serde(default)]
    pub layouts: Vec<Layout>,
}

/// The bits of HERDR_PLUGIN_CONTEXT_JSON we care about.
#[derive(Deserialize, Default, Debug)]
pub struct Context {
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub tab_id: Option<String>,
}

impl Context {
    pub fn from_env() -> Self {
        std::env::var("HERDR_PLUGIN_CONTEXT_JSON")
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Sidebar,
    List,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Pane,
    Tab,
    Workspace,
}

pub type Dest = (Kind, String);

pub fn make_labels(n: usize) -> Vec<String> {
    let letters: Vec<char> = ALPHABET.chars().collect();
    if n <= letters.len() {
        return letters[..n].iter().map(|c| c.to_string()).collect();
    }
    let mut out = Vec::with_capacity(n);
    for a in &letters {
        for b in &letters {
            out.push(format!("{a}{b}"));
            if out.len() == n {
                return out;
            }
        }
    }
    out
}

pub fn short_path(p: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        if p == home {
            return "~".into();
        }
        if let Some(rest) = p.strip_prefix(&format!("{home}/")) {
            return format!("~/{rest}");
        }
    }
    p.to_string()
}

/// Strip a leading "[xx] " hint we put there ourselves, so a reload while the
/// popup is open (or the `--clear` helper) sees the original label.
pub fn strip_hint(s: &str) -> &str {
    let Some(rest) = s.strip_prefix('[') else {
        return s;
    };
    let Some(end) = rest.find("] ") else {
        return s;
    };
    let tag = &rest[..end];
    let ok = (1..=2).contains(&tag.len())
        && tag
            .chars()
            .all(|c| ALPHABET.contains(c) || c.is_ascii_digit());
    if ok {
        &rest[end + 2..]
    } else {
        s
    }
}

/// A tab that still carries Herdr's automatic label (its number). The tab bar
/// already shows that number, so it doubles as the hint; renaming such a tab
/// would pin the label and stop it from renumbering.
pub fn auto_tab_key(t: &Tab) -> Option<String> {
    if t.label == t.number.to_string() && (1..=9).contains(&t.number) {
        Some(t.label.clone())
    } else {
        None
    }
}

fn status_rank(s: Option<&str>) -> i32 {
    match s {
        Some("blocked") => 0,
        Some("waiting") => 1,
        Some("error") => 2,
        Some("done") => 3,
        Some("working") => 4,
        Some("idle") => 5,
        _ => 9,
    }
}

pub struct Model {
    pub focused_pane: Option<String>,
    pub current_tab: Option<String>,
    pub current_ws: Option<String>,
    pub panes: HashMap<String, Pane>,
    pub agents: HashMap<String, Agent>,
    pub workspaces: HashMap<String, Workspace>,
    pub layout: Option<Layout>,
    pub tab_panes: Vec<LayoutPane>,
    pub ws_rows: Vec<(Workspace, Vec<Tab>)>,
    pub current_ws_tabs: Vec<Tab>,
    /// Agent panes per workspace, in sidebar order (sidebar mode only).
    pub ws_agents: Vec<(String, Vec<String>)>,
    /// Columns left of the pane area, i.e. the sidebar's width. -1 if unknown.
    pub sidebar_cols: i64,
    /// Outer terminal size, derived from the pane area. (0, 0) if unknown.
    pub screen_cols: usize,
    pub screen_rows: usize,
    pub dests: Vec<Dest>,
    pub labels: Vec<String>, // parallel to dests
    label_index: HashMap<Dest, usize>,
}

impl Model {
    pub fn new(snap: Snapshot, ctx: Context, mode: Mode) -> Self {
        let current_tab = ctx.tab_id.or(snap.focused_tab_id);
        let current_ws = ctx.workspace_id.or(snap.focused_workspace_id);

        let panes: HashMap<_, _> = snap
            .panes
            .into_iter()
            .map(|p| (p.pane_id.clone(), p))
            .collect();
        let agents: HashMap<_, _> = snap
            .agents
            .into_iter()
            .map(|a| (a.pane_id.clone(), a))
            .collect();
        let tabs: HashMap<_, _> = snap
            .tabs
            .into_iter()
            .map(|mut t| {
                t.label = strip_hint(&t.label).to_string();
                (t.tab_id.clone(), t)
            })
            .collect();
        let workspaces: HashMap<_, _> = snap
            .workspaces
            .into_iter()
            .map(|w| (w.workspace_id.clone(), w))
            .collect();

        let layout = snap
            .layouts
            .iter()
            .find(|l| Some(&l.tab_id) == current_tab.as_ref())
            .cloned();
        let probe = layout.as_ref().or(snap.layouts.first());
        let (sidebar_cols, screen_cols, screen_rows) = match probe {
            Some(l) => (
                l.area.x,
                (l.area.x + l.area.width).max(0) as usize,
                (l.area.y + l.area.height).max(0) as usize,
            ),
            None => (-1, 0, 0),
        };
        // EASYJUMP_SIDEBAR=collapsed|expanded overrides the guess (debugging).
        let sidebar_cols = match std::env::var("EASYJUMP_SIDEBAR").as_deref() {
            Ok("collapsed") => 0,
            Ok("expanded") => SIDEBAR_MIN_COLS.max(sidebar_cols),
            _ => sidebar_cols,
        };
        let mut tab_panes = layout.as_ref().map(|l| l.panes.clone()).unwrap_or_default();
        tab_panes.sort_by_key(|p| (p.rect.y, p.rect.x));

        let mut ws_sorted: Vec<Workspace> = workspaces.values().cloned().collect();
        ws_sorted.sort_by_key(|w| w.number);
        let mut ws_rows = Vec::new();
        for ws in ws_sorted {
            let mut wtabs: Vec<Tab> = tabs
                .values()
                .filter(|t| t.workspace_id == ws.workspace_id)
                .cloned()
                .collect();
            wtabs.sort_by_key(|t| t.number);
            ws_rows.push((ws, wtabs));
        }
        let current_ws_tabs = ws_rows
            .iter()
            .find(|(w, _)| Some(&w.workspace_id) == current_ws.as_ref())
            .map(|(_, t)| t.clone())
            .unwrap_or_default();

        let mut dests: Vec<Dest> = Vec::new();
        let mut fixed: Vec<Option<String>> = Vec::new(); // parallel to dests
        let mut seen: HashSet<Dest> = HashSet::new();
        let mut ws_agents: Vec<(String, Vec<String>)> = Vec::new();
        let mut add = |kind: Kind, id: &str, key: Option<String>| {
            let d = (kind, id.to_string());
            if seen.insert(d.clone()) {
                dests.push(d);
                fixed.push(key);
            }
        };
        let tab_number = |pane_id: &str| -> i64 {
            panes
                .get(pane_id)
                .and_then(|p| tabs.get(&p.tab_id))
                .map(|t| t.number)
                .unwrap_or(0)
        };

        match mode {
            Mode::Sidebar => {
                for (ws, _) in &ws_rows {
                    add(Kind::Workspace, &ws.workspace_id, None);
                    let mut here: Vec<&Agent> = agents
                        .values()
                        .filter(|a| a.workspace_id == ws.workspace_id)
                        .collect();
                    here.sort_by_key(|a| (tab_number(&a.pane_id), a.pane_id.clone()));
                    let ids: Vec<String> = here.iter().map(|a| a.pane_id.clone()).collect();
                    for id in &ids {
                        add(Kind::Pane, id, None);
                    }
                    ws_agents.push((ws.workspace_id.clone(), ids));
                }
                for p in &tab_panes {
                    add(Kind::Pane, &p.pane_id, None);
                }
                if current_ws_tabs.len() > 1 {
                    for t in &current_ws_tabs {
                        add(Kind::Tab, &t.tab_id, auto_tab_key(t));
                    }
                }
            }
            Mode::List => {
                for p in &tab_panes {
                    add(Kind::Pane, &p.pane_id, None);
                }
                let mut all: Vec<&Agent> = agents.values().collect();
                all.sort_by_key(|a| (status_rank(a.agent_status.as_deref()), a.pane_id.clone()));
                for a in all {
                    add(Kind::Pane, &a.pane_id, None);
                }
                for (ws, wtabs) in &ws_rows {
                    if wtabs.len() == 1 {
                        add(Kind::Tab, &wtabs[0].tab_id, None);
                    } else {
                        add(Kind::Workspace, &ws.workspace_id, None);
                        for t in wtabs {
                            add(Kind::Tab, &t.tab_id, None);
                        }
                    }
                }
            }
        }

        // Letters go to every destination without a fixed key, in order.
        let free = fixed.iter().filter(|f| f.is_none()).count();
        let mut letters = make_labels(free).into_iter();
        let labels: Vec<String> = fixed
            .into_iter()
            .map(|f| f.unwrap_or_else(|| letters.next().unwrap_or_default()))
            .collect();
        let label_index = dests
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, d)| (d, i))
            .collect();

        Model {
            focused_pane: snap.focused_pane_id,
            current_tab,
            current_ws,
            panes,
            agents,
            workspaces,
            layout,
            tab_panes,
            ws_rows,
            current_ws_tabs,
            ws_agents,
            sidebar_cols,
            screen_cols,
            screen_rows,
            dests,
            labels,
            label_index,
        }
    }

    /// True when the sidebar is too narrow to show the hint tokens, so the
    /// HUD has to list spaces and agents itself. Unknown counts as expanded.
    pub fn sidebar_collapsed(&self) -> bool {
        self.sidebar_cols >= 0 && self.sidebar_cols < SIDEBAR_MIN_COLS
    }

    /// Workspace before/after the current one in sidebar order, wrapping.
    pub fn neighbour_workspace(&self, step: i64) -> Option<String> {
        let n = self.ws_rows.len() as i64;
        if n == 0 {
            return None;
        }
        let i = self
            .ws_rows
            .iter()
            .position(|(w, _)| Some(&w.workspace_id) == self.current_ws.as_ref())
            .unwrap_or(0) as i64;
        let j = (i + step).rem_euclid(n) as usize;
        Some(self.ws_rows[j].0.workspace_id.clone())
    }

    /// Tab before/after the current one in the current workspace, wrapping.
    pub fn neighbour_tab(&self, step: i64) -> Option<String> {
        let n = self.current_ws_tabs.len() as i64;
        if n < 2 {
            return None;
        }
        let i = self
            .current_ws_tabs
            .iter()
            .position(|t| Some(&t.tab_id) == self.current_tab.as_ref())
            .unwrap_or(0) as i64;
        let j = (i + step).rem_euclid(n) as usize;
        Some(self.current_ws_tabs[j].tab_id.clone())
    }

    pub fn label(&self, kind: Kind, id: &str) -> &str {
        self.label_index
            .get(&(kind, id.to_string()))
            .map(|&i| self.labels[i].as_str())
            .unwrap_or("")
    }

    pub fn dest_for(&self, label: &str) -> Option<&Dest> {
        self.labels
            .iter()
            .position(|l| l == label)
            .map(|i| &self.dests[i])
    }

    pub fn labels_with_prefix(&self, prefix: &str) -> Vec<&str> {
        self.labels
            .iter()
            .filter(|l| l.starts_with(prefix))
            .map(String::as_str)
            .collect()
    }

    pub fn jump(&self, client: &mut Client, dest: &Dest) -> Result<(), String> {
        let (method, key) = match dest.0 {
            Kind::Pane => ("pane.focus", "pane_id"),
            Kind::Tab => ("tab.focus", "tab_id"),
            Kind::Workspace => ("workspace.focus", "workspace_id"),
        };
        client.call(method, json!({ key: dest.1 })).map(|_| ())
    }

    /// (primary, secondary) text for a pane.
    pub fn pane_summary(&self, pane_id: &str) -> (String, String) {
        if let Some(a) = self.agents.get(pane_id) {
            let primary = a
                .name
                .clone()
                .or_else(|| a.display_agent.clone())
                .or_else(|| a.agent.clone())
                .unwrap_or_else(|| "agent".into());
            let secondary = a
                .terminal_title_stripped
                .clone()
                .unwrap_or_else(|| short_path(a.cwd.as_deref().unwrap_or("")));
            return (primary, secondary);
        }
        let p = self.panes.get(pane_id);
        let cwd = p.and_then(|p| p.cwd.clone()).unwrap_or_default();
        let primary = p
            .and_then(|p| p.label.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                cwd.rsplit('/')
                    .next()
                    .filter(|s| !s.is_empty())
                    .map(String::from)
            })
            .unwrap_or_else(|| "shell".into());
        let secondary = p
            .and_then(|p| p.terminal_title_stripped.clone())
            .unwrap_or_else(|| short_path(&cwd));
        (primary, secondary)
    }

    pub fn agent_status(&self, pane_id: &str) -> Option<&str> {
        self.agents
            .get(pane_id)
            .and_then(|a| a.agent_status.as_deref())
    }

    /// (method, id_key, dest) for every destination the sidebar shows.
    fn sidebar_targets(&self) -> Vec<(&'static str, &'static str, &Dest)> {
        self.dests
            .iter()
            .filter_map(|d| match d.0 {
                Kind::Workspace => Some(("workspace.report_metadata", "workspace_id", d)),
                Kind::Pane if self.agents.contains_key(&d.1) => {
                    Some(("pane.report_metadata", "pane_id", d))
                }
                _ => None,
            })
            .collect()
    }

    /// Text the pane border shows behind the hint: agent name, or the
    /// terminal title / cwd of a plain shell.
    fn border_text(&self, pane_id: &str) -> String {
        let (primary, secondary) = self.pane_summary(pane_id);
        if self.agents.contains_key(pane_id) {
            return primary;
        }
        let title = self
            .panes
            .get(pane_id)
            .and_then(|p| p.terminal_title_stripped.clone())
            .unwrap_or_default();
        if title.is_empty() {
            secondary
        } else {
            title
        }
    }

    /// Tabs of the current workspace that need a rename to show their hint
    /// (custom-named ones; auto-numbered tabs use their number as the key).
    fn renamed_tabs(&self) -> Vec<&Tab> {
        self.current_ws_tabs
            .iter()
            .filter(|t| {
                auto_tab_key(t).is_none()
                    && self.label_index.contains_key(&(Kind::Tab, t.tab_id.clone()))
            })
            .collect()
    }

    /// Push (or refresh) every visible hint: the $hint token on sidebar rows,
    /// the title override on the borders of the current tab's panes, and a
    /// label prefix on custom-named tabs. Hints that no longer match `typed`
    /// are withdrawn.
    pub fn publish_hints(&self, client: &mut Client, typed: &str) {
        // One report_metadata call per pane, even when it is both an agent
        // row in the sidebar and a pane of the current tab.
        let mut pane_params: HashMap<String, Value> = HashMap::new();
        for (method, key, dest) in self.sidebar_targets() {
            let label = self.label(dest.0.clone(), &dest.1);
            let mut params = json!({ key: dest.1, "source": SOURCE, "ttl_ms": TOKEN_TTL_MS });
            if label.starts_with(typed) {
                params["tokens"] = json!({ TOKEN: format!("[{label}]") });
            } else {
                params["tokens"] = json!({ TOKEN: Value::Null });
            }
            if method == "workspace.report_metadata" {
                let _ = client.call(method, params);
            } else {
                pane_params.insert(dest.1.clone(), params);
            }
        }
        for p in &self.tab_panes {
            let label = self.label(Kind::Pane, &p.pane_id);
            let entry = pane_params.entry(p.pane_id.clone()).or_insert_with(|| {
                json!({ "pane_id": p.pane_id, "source": SOURCE, "ttl_ms": TOKEN_TTL_MS })
            });
            if label.starts_with(typed) {
                entry["title"] = json!(format!("[{label}] {}", self.border_text(&p.pane_id)));
            } else {
                entry["clear_title"] = json!(true);
            }
        }
        for params in pane_params.into_values() {
            let _ = client.call("pane.report_metadata", params);
        }
        for t in self.renamed_tabs() {
            let label = self.label(Kind::Tab, &t.tab_id);
            let new = if label.starts_with(typed) {
                format!("[{label}] {}", t.label)
            } else {
                t.label.clone()
            };
            let _ = client.call("tab.rename", json!({ "tab_id": t.tab_id, "label": new }));
        }
    }

    /// Withdraw every hint this model published.
    pub fn clear_hints(&self, client: &mut Client) {
        let mut pane_ids: HashSet<&str> = HashSet::new();
        for (method, key, dest) in self.sidebar_targets() {
            if method == "workspace.report_metadata" {
                let _ = client.call(
                    method,
                    json!({ key: dest.1, "source": SOURCE, "tokens": { TOKEN: Value::Null } }),
                );
            } else {
                pane_ids.insert(&dest.1);
            }
        }
        for p in &self.tab_panes {
            pane_ids.insert(&p.pane_id);
        }
        for id in pane_ids {
            let _ = client.call(
                "pane.report_metadata",
                json!({ "pane_id": id, "source": SOURCE, "tokens": { TOKEN: Value::Null }, "clear_title": true }),
            );
        }
        for t in self.renamed_tabs() {
            let _ = client.call("tab.rename", json!({ "tab_id": t.tab_id, "label": t.label }));
        }
    }

    /// Clear every hint on every workspace, pane and tab, whether or not this
    /// process published it. Used by the detached `--clear` helper, which
    /// only has a fresh snapshot to go on.
    pub fn clear_all(client: &mut Client, snap: &Snapshot) {
        for w in &snap.workspaces {
            let _ = client.call(
                "workspace.report_metadata",
                json!({ "workspace_id": w.workspace_id, "source": SOURCE, "tokens": { TOKEN: Value::Null } }),
            );
        }
        for p in &snap.panes {
            let _ = client.call(
                "pane.report_metadata",
                json!({ "pane_id": p.pane_id, "source": SOURCE, "tokens": { TOKEN: Value::Null }, "clear_title": true }),
            );
        }
        for t in &snap.tabs {
            let orig = strip_hint(&t.label);
            if orig != t.label {
                let _ = client.call("tab.rename", json!({ "tab_id": t.tab_id, "label": orig }));
            }
        }
    }

}
