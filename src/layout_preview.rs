//! Render a session-layout.kdl as a compact tree: tabs -> pane splits ->
//! leaf panes with name, command basename, size, and focus marker.
//!
//! Native port of the Python `zellij-layout-preview` (ressources/scripts/
//! zellij-layout-preview/main.py) — output format is golden-tested against
//! it; keep them byte-identical if the Python one is still installed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::util::home_dir;

// ---- Tokenizer ---------------------------------------------------------------

/// KDL for our purposes is quoted strings, single braces, newlines (node
/// terminators), and bareword/prop tokens. `//` comments are stripped
/// per-line first to avoid quoted "//" false positives.
fn strip_comments(src: &str) -> String {
    src.lines()
        .map(|line| {
            let cs: Vec<char> = line.chars().collect();
            let mut in_str = false;
            for i in 0..cs.len() {
                let c = cs[i];
                if c == '"' && (i == 0 || cs[i - 1] != '\\') {
                    in_str = !in_str;
                } else if !in_str && c == '/' && cs.get(i + 1) == Some(&'/') {
                    return cs[..i].iter().collect::<String>();
                }
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn tokenize(src: &str) -> Vec<String> {
    let cs: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '"' {
            let start = i;
            i += 1;
            while i < cs.len() {
                if cs[i] == '\\' {
                    i += 2;
                    continue;
                }
                if cs[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(cs[start..i.min(cs.len())].iter().collect());
        } else if c == '{' || c == '}' {
            out.push(c.to_string());
            i += 1;
        } else if c == '\n' {
            out.push("\n".to_string());
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else {
            let start = i;
            while i < cs.len()
                && !cs[i].is_whitespace()
                && cs[i] != '{'
                && cs[i] != '}'
                && cs[i] != '"'
            {
                i += 1;
            }
            out.push(cs[start..i].iter().collect());
        }
    }
    out
}

fn unquote(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let inner = &s[1..s.len() - 1];
        let mut out = String::new();
        let mut it = inner.chars();
        while let Some(c) = it.next() {
            if c == '\\' {
                match it.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('\\') => out.push('\\'),
                    Some('"') => out.push('"'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        out
    } else {
        s.to_string()
    }
}

// ---- Parser (minimal KDL) ----------------------------------------------------

#[derive(Debug, Default)]
struct Node {
    name: String,
    args: Vec<String>,
    props: HashMap<String, String>,
    children: Vec<Node>,
}

struct Cursor {
    toks: Vec<String>,
    i: usize,
}

impl Cursor {
    fn next(&mut self) -> Option<String> {
        let t = self.toks.get(self.i).cloned();
        self.i += 1;
        t
    }
}

fn parse_nodes(tokens: &mut Cursor, end_at_brace: bool) -> Vec<Node> {
    let mut nodes = Vec::new();
    while let Some(tok) = tokens.next() {
        match tok.as_str() {
            "\n" => continue,
            "}" => {
                if end_at_brace {
                    return nodes;
                }
                continue;
            }
            "{" => continue,
            _ => {}
        }
        let mut node = Node {
            name: tok,
            ..Default::default()
        };
        while let Some(arg) = tokens.next() {
            if arg == "\n" || arg == ";" {
                break; // end of this node (no children block)
            }
            if arg == "{" {
                node.children = parse_nodes(tokens, true);
                break;
            }
            if arg == "}" {
                nodes.push(node);
                return nodes;
            }
            // `name="zjp"` tokenizes as two: `name=` and `"zjp"`. Recombine.
            if arg.ends_with('=') && !arg.starts_with('"') {
                let key = arg[..arg.len() - 1].to_string();
                let mut val = String::new();
                while let Some(v) = tokens.next() {
                    if v == "\n" {
                        continue;
                    }
                    val = v;
                    break;
                }
                node.props.insert(key, unquote(&val));
            } else if arg.contains('=') && !arg.starts_with('"') {
                let (k, v) = arg.split_once('=').unwrap();
                node.props.insert(k.to_string(), unquote(v));
            } else {
                node.args.push(unquote(&arg));
            }
        }
        nodes.push(node);
    }
    nodes
}

fn parse(src: &str) -> Vec<Node> {
    let toks = tokenize(&strip_comments(src));
    parse_nodes(&mut Cursor { toks, i: 0 }, false)
}

// ---- Rendering ---------------------------------------------------------------

fn basename_cmd(cmd: &str) -> String {
    cmd.rsplit('/').next().unwrap_or("").to_string()
}

fn is_status_pane(p: &Node) -> bool {
    p.props.get("size").map(String::as_str) == Some("1")
        && p.children.iter().any(|c| c.name == "plugin")
}

fn plugin_name(p: &Node) -> String {
    for c in &p.children {
        if c.name == "plugin" {
            let loc = c.props.get("location").map(String::as_str).unwrap_or("");
            let leaf = loc.rsplit('/').next().unwrap_or("");
            let leaf = leaf.strip_suffix(".wasm").unwrap_or(leaf);
            return if leaf.is_empty() {
                "plugin".to_string()
            } else {
                leaf.to_string()
            };
        }
    }
    String::new()
}

fn render_pane(node: &Node, prefix: &str, is_last: bool) -> Vec<String> {
    let connector = if is_last { "└─ " } else { "├─ " };
    let child_prefix = format!("{prefix}{}", if is_last { "   " } else { "│  " });
    let kids: Vec<&Node> = node.children.iter().filter(|c| c.name == "pane").collect();
    let split = node
        .props
        .get("split_direction")
        .map(String::as_str)
        .unwrap_or("");
    if !kids.is_empty() {
        let axis = if !split.is_empty() {
            if split.to_lowercase().starts_with('h') {
                "H-split"
            } else {
                "V-split"
            }
        } else {
            "group"
        };
        let size = node.props.get("size").map(String::as_str).unwrap_or("");
        let label = if size.is_empty() {
            axis.to_string()
        } else {
            format!("{axis} [{size}]")
        };
        let mut lines = vec![format!("{prefix}{connector}{label}")];
        for (i, k) in kids.iter().enumerate() {
            lines.extend(render_pane(k, &child_prefix, i == kids.len() - 1));
        }
        return lines;
    }
    if is_status_pane(node) {
        let pl = plugin_name(node);
        let pl = if pl.is_empty() { "?".to_string() } else { pl };
        return vec![format!("{prefix}{connector}[status: {pl}]")];
    }
    let name = node.props.get("name").map(|s| s.trim()).unwrap_or("");
    let cmd = basename_cmd(node.props.get("command").map(String::as_str).unwrap_or(""));
    let size = node.props.get("size").map(String::as_str).unwrap_or("");
    let focus = node.props.get("focus").map(String::as_str) == Some("true");
    let mut parts: Vec<String> = Vec::new();
    if !name.is_empty() {
        parts.push(name.to_string());
    } else if cmd.is_empty() {
        parts.push("(shell)".to_string());
    }
    if !cmd.is_empty() {
        parts.push(format!("— {cmd}"));
    }
    if !size.is_empty() {
        parts.push(format!("[{size}]"));
    }
    if focus {
        parts.push("*".to_string());
    }
    let label = parts.join(" ");
    let label = if label.is_empty() {
        "(pane)".to_string()
    } else {
        label
    };
    vec![format!("{prefix}{connector}{label}")]
}

fn render_tab(tab: &Node, index: usize) -> Vec<String> {
    let default_name = format!("#{}", index + 1);
    let name = tab
        .props
        .get("name")
        .map(String::as_str)
        .unwrap_or(&default_name);
    let focus = tab.props.get("focus").map(String::as_str) == Some("true");
    let header = format!(
        "Tab {}: {}{}",
        index + 1,
        name,
        if focus { "  *" } else { "" }
    );
    let mut lines = vec![header];
    let top_panes: Vec<&Node> = tab.children.iter().filter(|c| c.name == "pane").collect();
    if top_panes.is_empty() {
        lines.push("  (empty)".to_string());
    } else {
        for (i, p) in top_panes.iter().enumerate() {
            lines.extend(render_pane(p, "  ", i == top_panes.len() - 1));
        }
    }
    if let Some(floats) = tab.children.iter().find(|c| c.name == "floating_panes") {
        let n = floats.children.iter().filter(|c| c.name == "pane").count();
        if n > 0 {
            lines.push(format!("  (+ {n} floating)"));
        }
    }
    lines
}

// ---- Entry points --------------------------------------------------------------

fn render_nodes(nodes: &[Node]) -> String {
    let Some(layout) = nodes.iter().find(|n| n.name == "layout") else {
        return "(no layout block)".to_string();
    };
    let cwd = layout
        .children
        .iter()
        .find(|c| c.name == "cwd" && !c.args.is_empty())
        .map(|c| c.args[0].clone())
        .unwrap_or_default();
    let tabs: Vec<&Node> = layout.children.iter().filter(|c| c.name == "tab").collect();
    let mut out: Vec<String> = Vec::new();
    if !cwd.is_empty() {
        out.push(format!("cwd: {cwd}"));
        out.push(String::new());
    }
    if tabs.is_empty() {
        out.push("(no tabs)".to_string());
    } else {
        for (i, t) in tabs.iter().enumerate() {
            if i > 0 {
                out.push(String::new());
            }
            out.extend(render_tab(t, i));
        }
    }
    out.join("\n")
}

pub fn render_str(src: &str) -> String {
    render_nodes(&parse(src))
}

pub fn render_file(path: &Path) -> String {
    let empty = std::fs::metadata(path)
        .map(|m| m.len() == 0)
        .unwrap_or(true);
    if empty {
        return "(no saved layout)".to_string();
    }
    match std::fs::read(path) {
        Ok(bytes) => render_str(&String::from_utf8_lossy(&bytes)),
        Err(e) => format!("(unable to read layout: {e})"),
    }
}

fn session_layout_file(name: &str) -> PathBuf {
    home_dir()
        .join(".cache/zellij/contract_version_1/session_info")
        .join(name)
        .join("session-layout.kdl")
}

pub fn render_session(name: &str) -> String {
    render_file(&session_layout_file(name))
}

// ---- Visual (box-art) rendering ------------------------------------------
// Draws the pane layout as nested boxes sized from the serialized layout —
// a blueprint of the actual session (pane arrangement + running programs).
// Reads ONLY the cached session-layout.kdl: previewing never talks to the
// zellij server, so a dead session can never be resurrected by hovering it.

enum SizeSpec {
    Pct(f64),
    Fixed(usize),
    Flex,
}

fn size_spec(node: &Node) -> SizeSpec {
    match node.props.get("size").map(String::as_str) {
        Some(s) if s.ends_with('%') => s[..s.len() - 1]
            .parse::<f64>()
            .map(SizeSpec::Pct)
            .unwrap_or(SizeSpec::Flex),
        Some(s) => s
            .parse::<usize>()
            .map(SizeSpec::Fixed)
            .unwrap_or(SizeSpec::Flex),
        None => SizeSpec::Flex,
    }
}

/// Split `total` cells among children: fixed sizes literal (clamped),
/// percentages of the total, the rest shared equally by flex panes.
fn allocate(total: usize, kids: &[&Node]) -> Vec<usize> {
    let mut sizes = vec![0usize; kids.len()];
    let mut flex = Vec::new();
    let mut used = 0usize;
    for (i, k) in kids.iter().enumerate() {
        match size_spec(k) {
            SizeSpec::Fixed(n) => {
                sizes[i] = n.min(total.saturating_sub(used));
                used += sizes[i];
            }
            SizeSpec::Pct(p) => {
                sizes[i] =
                    ((total as f64 * p / 100.0).round() as usize).min(total.saturating_sub(used));
                used += sizes[i];
            }
            SizeSpec::Flex => flex.push(i),
        }
    }
    if !flex.is_empty() {
        let rem = total.saturating_sub(used);
        let each = rem / flex.len();
        for (n, &i) in flex.iter().enumerate() {
            sizes[i] = if n == flex.len() - 1 {
                rem - each * (flex.len() - 1)
            } else {
                each
            };
        }
    } else if used < total {
        // No flex pane: hand the leftover to the last child so boxes tile.
        if let Some(last) = sizes.last_mut() {
            *last += total - used;
        }
    }
    sizes
}

struct Grid {
    w: usize,
    cells: Vec<Vec<char>>,
}

impl Grid {
    fn new(w: usize, h: usize) -> Self {
        Grid {
            w,
            cells: vec![vec![' '; w]; h],
        }
    }

    fn set(&mut self, x: usize, y: usize, c: char) {
        if y < self.cells.len() && x < self.w {
            self.cells[y][x] = c;
        }
    }

    fn text(&mut self, x: usize, y: usize, maxw: usize, s: &str) {
        for (i, c) in s.chars().take(maxw).enumerate() {
            self.set(x + i, y, c);
        }
    }

    fn draw_box(&mut self, x: usize, y: usize, w: usize, h: usize, heavy: bool) {
        if w < 2 || h < 2 {
            return;
        }
        let (tl, tr, bl, br, hz, vt) = if heavy {
            ('┏', '┓', '┗', '┛', '━', '┃')
        } else {
            ('┌', '┐', '└', '┘', '─', '│')
        };
        for cx in x + 1..x + w - 1 {
            self.set(cx, y, hz);
            self.set(cx, y + h - 1, hz);
        }
        for cy in y + 1..y + h - 1 {
            self.set(x, cy, vt);
            self.set(x + w - 1, cy, vt);
        }
        self.set(x, y, tl);
        self.set(x + w - 1, y, tr);
        self.set(x, y + h - 1, bl);
        self.set(x + w - 1, y + h - 1, br);
    }

    /// Emit lines with light box-drawing runs dimmed (heavy = focused pane
    /// border stays at normal intensity so it pops).
    fn lines(&self) -> Vec<String> {
        self.cells
            .iter()
            .map(|row| {
                let raw: String = row.iter().collect::<String>().trim_end().to_string();
                let mut out = String::new();
                let mut dim = false;
                for c in raw.chars() {
                    let is_light = matches!(c, '─' | '│' | '┌' | '┐' | '└' | '┘');
                    if is_light && !dim {
                        out.push_str("\x1b[2m");
                        dim = true;
                    } else if !is_light && dim {
                        out.push_str("\x1b[22m");
                        dim = false;
                    }
                    out.push(c);
                }
                if dim {
                    out.push_str("\x1b[22m");
                }
                out
            })
            .collect()
    }
}

fn visual_leaf_label(node: &Node) -> String {
    if is_status_pane(node) {
        let p = plugin_name(node);
        return format!("[{}]", if p.is_empty() { "status".into() } else { p });
    }
    let name = node
        .props
        .get("name")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let cmd = basename_cmd(node.props.get("command").map(String::as_str).unwrap_or(""));
    let base = if !name.is_empty() {
        name
    } else if !cmd.is_empty() {
        cmd
    } else {
        "shell".to_string()
    };
    if node.props.get("focus").map(String::as_str) == Some("true") {
        format!("{base} *")
    } else {
        base
    }
}

fn render_pane_rect(node: &Node, grid: &mut Grid, x: usize, y: usize, w: usize, h: usize) {
    if w == 0 || h == 0 {
        return;
    }
    let kids: Vec<&Node> = node.children.iter().filter(|c| c.name == "pane").collect();
    if kids.is_empty() {
        let label = visual_leaf_label(node);
        if is_status_pane(node) || h < 3 || w < 6 {
            grid.text(x, y, w, &label);
            return;
        }
        let focused = node.props.get("focus").map(String::as_str) == Some("true");
        grid.draw_box(x, y, w, h, focused);
        grid.text(x + 2, y + 1, w.saturating_sub(4), &label);
        return;
    }
    // split_direction="vertical" => vertical divider => children side by side;
    // default (horizontal) stacks them as rows.
    let side_by_side = node
        .props
        .get("split_direction")
        .map(|s| s.to_lowercase().starts_with('v'))
        .unwrap_or(false);
    if side_by_side {
        let sizes = allocate(w, &kids);
        let mut cx = x;
        for (k, sw) in kids.iter().zip(sizes) {
            render_pane_rect(k, grid, cx, y, sw, h);
            cx += sw;
        }
    } else {
        let sizes = allocate(h, &kids);
        let mut cy = y;
        for (k, sh) in kids.iter().zip(sizes) {
            render_pane_rect(k, grid, x, cy, w, sh);
            cy += sh;
        }
    }
}

fn render_tab_visual(tab: &Node, index: usize, width: usize, height: usize) -> Vec<String> {
    let default_name = format!("#{}", index + 1);
    let name = tab
        .props
        .get("name")
        .map(String::as_str)
        .unwrap_or(&default_name);
    let focus = tab.props.get("focus").map(String::as_str) == Some("true");
    let mut lines = vec![format!(
        "Tab {}: {}{}",
        index + 1,
        name,
        if focus { "  *" } else { "" }
    )];
    let panes: Vec<&Node> = tab.children.iter().filter(|c| c.name == "pane").collect();
    if panes.is_empty() {
        lines.push("  (empty)".to_string());
    } else {
        let mut grid = Grid::new(width, height);
        let sizes = allocate(height, &panes);
        let mut cy = 0;
        for (p, sh) in panes.iter().zip(sizes) {
            render_pane_rect(p, &mut grid, 0, cy, width, sh);
            cy += sh;
        }
        lines.extend(grid.lines());
    }
    if let Some(floats) = tab.children.iter().find(|c| c.name == "floating_panes") {
        let n = floats.children.iter().filter(|c| c.name == "pane").count();
        if n > 0 {
            lines.push(format!("(+ {n} floating)"));
        }
    }
    lines
}

/// Box-art render of a session's saved layout, sized for the fzf preview
/// pane. Returns None when a visual render isn't possible (missing/empty
/// layout, no tabs) — callers fall back to the text tree.
pub fn render_session_visual(name: &str, cols: usize, rows: usize) -> Option<String> {
    let path = session_layout_file(name);
    if std::fs::metadata(&path)
        .map(|m| m.len() == 0)
        .unwrap_or(true)
    {
        return None;
    }
    let src = std::fs::read(&path).ok()?;
    let nodes = parse(&String::from_utf8_lossy(&src));
    let layout = nodes.iter().find(|n| n.name == "layout")?;
    let tabs: Vec<&Node> = layout.children.iter().filter(|c| c.name == "tab").collect();
    if tabs.is_empty() {
        return None;
    }
    let cwd = layout
        .children
        .iter()
        .find(|c| c.name == "cwd" && !c.args.is_empty())
        .map(|c| c.args[0].clone())
        .unwrap_or_default();

    let width = cols.clamp(20, 200).saturating_sub(1);
    // fzf scrolls overflowing previews, so per-tab height just needs a
    // sane range, not an exact fit.
    let box_h = (rows.saturating_sub(4) / tabs.len()).clamp(5, 16);

    let mut out: Vec<String> = Vec::new();
    if !cwd.is_empty() {
        out.push(format!("cwd: {cwd}"));
        out.push(String::new());
    }
    for (i, t) in tabs.iter().enumerate() {
        if i > 0 {
            out.push(String::new());
        }
        out.extend(render_tab_visual(t, i, width, box_h));
    }
    Some(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden test: fixture rendered byte-identically to the Python
    /// zellij-layout-preview output captured on 2026-07-12.
    #[test]
    fn golden_render_matches_python_renderer() {
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/session-layout.kdl"
        );
        let out = render_file(Path::new(fixture));
        let expected = "\
cwd: /home/maike/.config/nixos

Tab 1: Tab #1  *
  ├─ V-split
  │  ├─ — claude [50%] *
  │  └─ — nvim [50%]
  └─ [status: zellij:compact-bar]
  (+ 1 floating)";
        assert_eq!(out, expected);
    }

    #[test]
    fn missing_file_and_empty_file() {
        assert_eq!(
            render_file(Path::new("/nope/nothing.kdl")),
            "(no saved layout)"
        );
    }

    #[test]
    fn no_layout_block() {
        assert_eq!(render_str("keybinds {\n}\n"), "(no layout block)");
    }

    #[test]
    fn empty_tab_and_unnamed_tab() {
        let out = render_str("layout {\n tab {\n }\n tab name=\"x\" {\n pane\n }\n}\n");
        assert_eq!(out, "Tab 1: #1\n  (empty)\n\nTab 2: x\n  └─ (shell)");
    }

    #[test]
    fn comments_are_stripped_but_not_inside_strings() {
        let out = render_str("layout { // trailing\n tab name=\"a//b\" { // c\n pane\n }\n}\n");
        assert_eq!(out, "Tab 1: a//b\n  └─ (shell)");
    }
}
