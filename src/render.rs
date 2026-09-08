//! ANSI rendering for the HUD and the list popup. CJK aware.

use crate::model::{Kind, Model};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const HINT: &str = "\x1b[1;30;48;5;220m"; // black on yellow, like Vimium
pub const HINT_TYPED: &str = "\x1b[1;30;48;5;208m"; // darker orange for the typed prefix
pub const TITLE: &str = "\x1b[1;38;5;110m";
pub const FOCUS: &str = "\x1b[38;5;214m";

pub fn status_icon(status: Option<&str>) -> &'static str {
    match status {
        Some("working") | Some("running") => "●",
        Some("idle") => "○",
        Some("waiting") => "◐",
        Some("blocked") => "◆",
        Some("error") => "✗",
        Some("done") => "✓",
        _ => "·",
    }
}

fn status_color(status: Option<&str>) -> &'static str {
    match status {
        Some("working") | Some("running") => "\x1b[38;5;114m",
        Some("idle") => "\x1b[38;5;245m",
        Some("waiting") => "\x1b[38;5;214m",
        Some("blocked") | Some("error") => "\x1b[38;5;203m",
        Some("done") => "\x1b[38;5;75m",
        _ => DIM,
    }
}

pub fn status_str(status: Option<&str>) -> String {
    let s = status.unwrap_or("unknown");
    format!(
        "{}{} {}{}",
        status_color(status),
        status_icon(status),
        s,
        RESET
    )
}

fn cw(c: char) -> usize {
    c.width().unwrap_or(0)
}

pub fn truncate(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let c = cw(ch);
        if w + c > width.saturating_sub(1) {
            break;
        }
        out.push(ch);
        w += c;
    }
    out.push('…');
    out
}

/// Display width of a string ignoring ANSI escapes.
pub fn visible_len(s: &str) -> usize {
    let mut out = 0;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for n in chars.by_ref() {
                if n == 'm' {
                    break;
                }
            }
            continue;
        }
        out += cw(c);
    }
    out
}

/// Cut an ANSI string to a display width, keeping escapes intact.
pub fn clip_ansi(s: &str, width: usize) -> String {
    let mut out = String::new();
    let mut w = 0;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            out.push(c);
            for n in chars.by_ref() {
                out.push(n);
                if n == 'm' {
                    break;
                }
            }
            continue;
        }
        let c_w = cw(c);
        if w + c_w > width {
            break;
        }
        out.push(c);
        w += c_w;
    }
    out.push_str(RESET);
    out
}

/// Hint box, dimmed when it no longer matches the typed prefix.
pub fn render_hint(label: &str, typed: &str) -> String {
    if !typed.is_empty() && !label.starts_with(typed) {
        return format!("{DIM}[{label}]{RESET}");
    }
    if !typed.is_empty() {
        return format!(
            "{HINT_TYPED}[{typed}{HINT}{}]{RESET}",
            &label[typed.len()..]
        );
    }
    format!("{HINT}[{label}]{RESET}")
}

/// Lay `items` out after `head`, two spaces apart, wrapping at `width`.
/// Continuation lines are indented to the first item.
fn flow(head: &str, items: &[String], width: usize) -> Vec<String> {
    let indent = " ".repeat(visible_len(head) + 1);
    let mut lines = Vec::new();
    let mut cur = format!("{head} ");
    let mut cur_w = visible_len(&cur);
    let mut empty = true;
    for item in items {
        let w = visible_len(item);
        if !empty && cur_w + 2 + w > width {
            lines.push(cur);
            cur = indent.clone();
            cur_w = indent.len();
            empty = true;
        }
        if !empty {
            cur.push_str("  ");
            cur_w += 2;
        }
        cur.push_str(item);
        cur_w += w;
        empty = false;
    }
    lines.push(cur);
    lines
}

/// Sidebar-mode HUD: header plus whatever the sidebar cannot show. With the
/// sidebar collapsed that includes the spaces and agents themselves, flowed
/// into as many lines as `width` needs.
pub fn hud_lines(m: &Model, typed: &str, width: usize) -> Vec<String> {
    let mut header =
        format!("{TITLE}{BOLD}easyjump{RESET} {DIM}hjkl/JK move · ⏎ keep · esc undo · ` back{RESET}");
    if !typed.is_empty() {
        header.push_str(&format!("  {HINT_TYPED} {typed} {RESET}"));
    }
    let mut lines = vec![header];
    if m.sidebar_collapsed() {
        let spaces: Vec<String> = m
            .ws_rows
            .iter()
            .map(|(ws, _)| {
                let cur = if Some(&ws.workspace_id) == m.current_ws.as_ref() {
                    BOLD
                } else {
                    ""
                };
                format!(
                    "{} {cur}{}{RESET}",
                    render_hint(m.label(Kind::Workspace, &ws.workspace_id), typed),
                    truncate(&ws.label, 14)
                )
            })
            .collect();
        if !spaces.is_empty() {
            lines.extend(flow(&format!("{DIM}spaces{RESET}"), &spaces, width));
        }
        let agents: Vec<String> = m
            .ws_agents
            .iter()
            .flat_map(|(ws_id, ids)| ids.iter().map(move |id| (ws_id, id)))
            .map(|(ws_id, id)| {
                let (name, _) = m.pane_summary(id);
                let ws = m
                    .workspaces
                    .get(ws_id)
                    .map(|w| w.label.as_str())
                    .unwrap_or("");
                let cur = if Some(id) == m.focused_pane.as_ref() {
                    BOLD
                } else {
                    ""
                };
                format!(
                    "{} {cur}{} {}{RESET}{DIM}·{}{RESET}",
                    render_hint(m.label(Kind::Pane, id), typed),
                    status_icon(m.agent_status(id)),
                    truncate(&name, 10),
                    truncate(ws, 10)
                )
            })
            .collect();
        if !agents.is_empty() {
            lines.extend(flow(&format!("{DIM}agents{RESET}"), &agents, width));
        }
    }
    if m.tab_panes.len() > 1 {
        let parts: Vec<String> = m
            .tab_panes
            .iter()
            .map(|p| {
                let (primary, _) = m.pane_summary(&p.pane_id);
                let icon = if m.agents.contains_key(&p.pane_id) {
                    format!("{} ", status_icon(m.agent_status(&p.pane_id)))
                } else {
                    String::new()
                };
                let cur = if Some(&p.pane_id) == m.focused_pane.as_ref() {
                    BOLD
                } else {
                    ""
                };
                format!(
                    "{} {cur}{icon}{}{RESET}",
                    render_hint(m.label(Kind::Pane, &p.pane_id), typed),
                    truncate(&primary, 14)
                )
            })
            .collect();
        lines.push(format!("{DIM}panes{RESET} {}", parts.join("  ")));
    }
    if m.current_ws_tabs.len() > 1 {
        let parts: Vec<String> = m
            .current_ws_tabs
            .iter()
            .map(|t| {
                let cur = if Some(&t.tab_id) == m.current_tab.as_ref() {
                    BOLD
                } else {
                    ""
                };
                format!(
                    "{} {cur}{}{RESET}",
                    render_hint(m.label(Kind::Tab, &t.tab_id), typed),
                    truncate(&t.label, 12)
                )
            })
            .collect();
        lines.push(format!("{DIM}tabs {RESET} {}", parts.join("  ")));
    }
    lines
}

#[derive(Clone)]
enum Cell {
    Blank,
    Ch(char, &'static str),
    Hint(String),
    Covered, // right half of a wide char, or under a hint
}

/// Scaled drawing of the current tab's pane layout as a char grid.
pub fn render_minimap(m: &Model, width: usize, height: usize, typed: &str) -> Vec<String> {
    let mut grid = vec![vec![Cell::Blank; width]; height];
    let layout = match &m.layout {
        Some(l) if width >= 8 && height >= 3 => l,
        _ => return vec![" ".repeat(width); height],
    };
    let area = &layout.area;
    let aw = area.width.max(1) as f64;
    let ah = area.height.max(1) as f64;

    let put = |grid: &mut Vec<Vec<Cell>>, y: i64, x: i64, cell: Cell| {
        if y >= 0 && (y as usize) < height && x >= 0 && (x as usize) < width {
            grid[y as usize][x as usize] = cell;
        }
    };
    let put_text =
        |grid: &mut Vec<Vec<Cell>>, y: i64, mut x: i64, text: &str, style: &'static str| {
            for ch in text.chars() {
                let w = cw(ch);
                put(grid, y, x, Cell::Ch(ch, style));
                if w == 2 {
                    put(grid, y, x + 1, Cell::Covered);
                }
                x += w.max(1) as i64;
            }
        };

    for p in &m.tab_panes {
        let r = &p.rect;
        let x0 = (((r.x - area.x) as f64) / aw * width as f64).round() as i64;
        let mut x1 = (((r.x + r.width - area.x) as f64) / aw * width as f64).round() as i64 - 1;
        let y0 = (((r.y - area.y) as f64) / ah * height as f64).round() as i64;
        let mut y1 = (((r.y + r.height - area.y) as f64) / ah * height as f64).round() as i64 - 1;
        x1 = x1.max(x0 + 5).min(width as i64 - 1);
        y1 = y1.max(y0 + 1).min(height as i64 - 1);
        let focused = Some(&p.pane_id) == m.focused_pane.as_ref();
        let st = if focused { FOCUS } else { DIM };
        for x in x0..=x1 {
            put(&mut grid, y0, x, Cell::Ch('─', st));
            put(&mut grid, y1, x, Cell::Ch('─', st));
        }
        for y in y0..=y1 {
            put(&mut grid, y, x0, Cell::Ch('│', st));
            put(&mut grid, y, x1, Cell::Ch('│', st));
        }
        put(&mut grid, y0, x0, Cell::Ch('┌', st));
        put(&mut grid, y0, x1, Cell::Ch('┐', st));
        put(&mut grid, y1, x0, Cell::Ch('└', st));
        put(&mut grid, y1, x1, Cell::Ch('┘', st));

        let inner_w = (x1 - x0 - 1).max(0) as usize;
        let (primary, secondary) = m.pane_summary(&p.pane_id);
        let label = m.label(Kind::Pane, &p.pane_id).to_string();
        let hint_w = label.chars().count() + 2;
        put(&mut grid, y0, x0 + 1, Cell::Hint(label));
        for i in 1..hint_w as i64 {
            put(&mut grid, y0, x0 + 1 + i, Cell::Covered);
        }
        let text_rows = y1 - y0 - 1;
        if text_rows >= 1 {
            let head = if m.agents.contains_key(&p.pane_id) {
                format!("{} {primary}", status_icon(m.agent_status(&p.pane_id)))
            } else {
                primary
            };
            put_text(
                &mut grid,
                y0 + 1,
                x0 + 1,
                &truncate(&head, inner_w),
                if focused { BOLD } else { "" },
            );
        }
        if text_rows >= 2 && !secondary.is_empty() {
            put_text(
                &mut grid,
                y0 + 2,
                x0 + 1,
                &truncate(&secondary, inner_w),
                DIM,
            );
        }
    }

    grid.iter()
        .map(|row| {
            let mut s = String::new();
            for cell in row {
                match cell {
                    Cell::Blank => s.push(' '),
                    Cell::Covered => {}
                    Cell::Hint(label) => s.push_str(&render_hint(label, typed)),
                    Cell::Ch(c, st) => {
                        if st.is_empty() {
                            s.push(*c);
                        } else {
                            s.push_str(st);
                            s.push(*c);
                            s.push_str(RESET);
                        }
                    }
                }
            }
            s
        })
        .collect()
}

fn tab_status(t: &crate::model::Tab) -> String {
    match t.agent_status.as_deref() {
        None | Some("unknown") => String::new(),
        s => status_str(s),
    }
}

/// List-mode right column: workspaces/tabs then agents.
pub fn render_lists(m: &Model, width: usize, typed: &str) -> Vec<String> {
    let mut lines = vec![format!("{TITLE}TABS{RESET}")];
    for (ws, tabs) in &m.ws_rows {
        let cur = Some(&ws.workspace_id) == m.current_ws.as_ref();
        let mark = if cur {
            format!("{FOCUS}▸{RESET}")
        } else {
            " ".into()
        };
        let bold = if cur { BOLD } else { "" };
        if tabs.len() == 1 {
            let t = &tabs[0];
            lines.push(format!(
                "{mark}{} {bold}{}{RESET} {}",
                render_hint(m.label(Kind::Tab, &t.tab_id), typed),
                truncate(&ws.label, 24),
                tab_status(t)
            ));
        } else {
            lines.push(format!(
                "{mark}{} {bold}{}{RESET}",
                render_hint(m.label(Kind::Workspace, &ws.workspace_id), typed),
                truncate(&ws.label, 24)
            ));
            for t in tabs {
                let tbold = if Some(&t.tab_id) == m.current_tab.as_ref() {
                    BOLD
                } else {
                    ""
                };
                lines.push(format!(
                    "   {} {tbold}{}{RESET} {DIM}{}p{RESET} {}",
                    render_hint(m.label(Kind::Tab, &t.tab_id), typed),
                    truncate(&t.label, 20),
                    t.pane_count.max(1),
                    tab_status(t)
                ));
            }
        }
    }

    if !m.agents.is_empty() {
        lines.push(String::new());
        lines.push(format!("{TITLE}AGENTS{RESET}"));
        for (kind, id) in &m.dests {
            if *kind != Kind::Pane {
                continue;
            }
            let Some(a) = m.agents.get(id) else { continue };
            let ws = m
                .workspaces
                .get(&a.workspace_id)
                .map(|w| w.label.as_str())
                .unwrap_or("");
            let name = a
                .name
                .clone()
                .or_else(|| a.display_agent.clone())
                .or_else(|| a.agent.clone())
                .unwrap_or_else(|| "agent".into());
            let title = a.terminal_title_stripped.clone().unwrap_or_default();
            let mark = if Some(id) == m.focused_pane.as_ref() {
                format!("{FOCUS}▸{RESET}")
            } else {
                " ".into()
            };
            let head = format!(
                "{mark}{} {BOLD}{}{RESET} {}",
                render_hint(m.label(Kind::Pane, id), typed),
                truncate(&name, 10),
                status_str(a.agent_status.as_deref())
            );
            let tail = format!(
                "{DIM}{}{RESET}  {}",
                truncate(ws, 14),
                truncate(&title, width.saturating_sub(46).max(8))
            );
            lines.push(format!("{head}  {tail}"));
        }
    }
    lines
}

pub fn render_list_frame(m: &Model, typed: &str, cols: usize, rows: usize) -> Vec<String> {
    let mut header =
        format!("{TITLE}{BOLD} easyjump {RESET}{DIM}type a hint · hjkl/JK move · ⏎ keep · esc undo · ` back{RESET}");
    if !typed.is_empty() {
        header.push_str(&format!("  {HINT_TYPED} {typed} {RESET}"));
    }
    let body_rows = rows.saturating_sub(2);
    let mut lines = vec![header, String::new()];
    let side_by_side = cols >= 90 && m.layout.is_some() && m.tab_panes.len() > 1;
    if side_by_side {
        let left_w = ((cols as f64 * 0.42) as usize).clamp(24, 70);
        let right_w = cols.saturating_sub(left_w + 3);
        let left = render_minimap(m, left_w, body_rows, typed);
        let right = render_lists(m, right_w, typed);
        for i in 0..body_rows {
            let l = left.get(i).cloned().unwrap_or_else(|| " ".repeat(left_w));
            let r = right.get(i).map(String::as_str).unwrap_or("");
            lines.push(format!(
                "{l}{RESET} {DIM}│{RESET} {}",
                clip_ansi(r, right_w)
            ));
        }
    } else {
        let right = render_lists(m, cols, typed);
        for i in 0..body_rows {
            let r = right.get(i).map(String::as_str).unwrap_or("");
            lines.push(clip_ansi(r, cols));
        }
    }
    lines.truncate(rows);
    lines
}
