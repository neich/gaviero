//! The CODE GRAPH side panel.
//!
//! This module owns the document format, the panel state, and the render; it
//! never opens the graph. [`crate::app::side_panel`] reads the graph and hands
//! this module plain owned rows, so rendering touches no database and no
//! filesystem.

use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::panels::chat_markdown::{self, ChatLine};
use crate::theme;
use crate::widgets::scroll_state::ScrollState;

/// Symbol rows a document lists before it prints a truncation note.
pub const MAX_SYMBOL_ROWS: usize = 200;
/// Edge rows per "Depends on" / "Used by" section.
pub const MAX_EDGE_ROWS: usize = 200;

const FOOTER_HELP: &str =
    "j/k scroll  n/p link  Enter open  r refresh  1/2 depth  x export  click open";

/// One indexed symbol of the active file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRow {
    /// 0-based.
    pub line: usize,
    /// tree-sitter kind, e.g. `"function_item"`.
    pub kind: String,
    pub name: String,
    pub owner: Option<String>,
    pub test: bool,
}

/// One file reached by the dependency or dependent walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeRow {
    /// Workspace-relative, `/`-separated.
    pub file_path: String,
    /// 0-based, only when it belongs to `file_path`.
    pub line: Option<usize>,
    /// `"Calls"` / `"Imports"` / …
    pub kind: String,
    /// Pre-rendered `src/a.rs:8 → src/b.rs`.
    pub via: String,
}

/// One symbol reference of the file's selected symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefRow {
    pub kind: String,
    pub file_path: String,
    /// 0-based.
    pub line: Option<usize>,
    /// Short name of the other end.
    pub name: String,
}

/// Where a document's links point.
#[derive(Debug, Clone)]
pub enum LinkStyle {
    /// `path#Lline`, relative to the graph root (in-panel).
    Repo,
    /// `file:///abs/path#Lline` (exported documents).
    Absolute(PathBuf),
}

/// The owned rows a markdown document is rendered from.
pub struct GraphDoc<'a> {
    pub rel: &'a str,
    pub symbol_name: Option<&'a str>,
    pub symbols: &'a [SymbolRow],
    pub total_symbols: usize,
    pub dependencies: &'a [EdgeRow],
    pub total_dependencies: usize,
    pub dependents: &'a [EdgeRow],
    pub total_dependents: usize,
    pub referenced_by: &'a [RefRow],
    pub references: &'a [RefRow],
    pub depth: usize,
    pub graph_ready: bool,
}

/// A click target: the first rendered line carrying `href`.
pub struct GraphLink {
    pub line: usize,
    pub href: String,
}

/// A code-graph link's destination for `path` and a 0-based `line`.
fn link_dest(style: &LinkStyle, path: &str, line: Option<usize>) -> String {
    match style {
        LinkStyle::Repo => match line {
            Some(line) => format!("{path}#L{}", line + 1),
            None => path.to_string(),
        },
        LinkStyle::Absolute(root) => {
            let abs = root.join(path).to_string_lossy().replace('\\', "/");
            let mut href = format!("file:///{}", percent_encode_path(abs.trim_start_matches('/')));
            if let Some(line) = line {
                href.push_str(&format!("#L{}", line + 1));
            }
            href
        }
    }
}

/// Percent-encode the characters that break a markdown destination. `%` is
/// encoded first (a single pass over the chars, so it can never re-encode).
fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for ch in path.chars() {
        match ch {
            '%' => out.push_str("%25"),
            ' ' => out.push_str("%20"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '[' => out.push_str("%5B"),
            ']' => out.push_str("%5D"),
            other => out.push(other),
        }
    }
    out
}

fn label(path: &str, line: Option<usize>) -> String {
    match line {
        Some(line) => format!("{path}:{}", line + 1),
        None => path.to_string(),
    }
}

/// Markdown escapes only `]` in a label; names never sit inside the label.
fn escape_label(text: &str) -> String {
    text.replace(']', "\\]")
}

fn link(style: &LinkStyle, path: &str, line: Option<usize>) -> String {
    format!(
        "[{}]({})",
        escape_label(&label(path, line)),
        link_dest(style, path, line)
    )
}

/// Render the panel document. Pure: every row is owned by the caller.
pub fn markdown_doc(doc: &GraphDoc<'_>, style: LinkStyle) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", doc.rel));

    if !doc.graph_ready {
        out.push_str(
            "_The code graph for this folder is not built yet. Open the folder in gaviero, or run gaviero-cli --graph._\n",
        );
        return out;
    }

    out.push_str(&format!(
        "`{}` symbols · `{}` depends on · `{}` used by · depth `{}`\n\n",
        doc.total_symbols, doc.total_dependencies, doc.total_dependents, doc.depth
    ));

    // ── Symbols ─────────────────────────────────────────────────
    out.push_str("## Symbols\n\n");
    if doc.symbols.is_empty() {
        out.push_str("_No indexed symbols for this file._\n\n");
    } else {
        for symbol in doc.symbols {
            out.push_str(&format!(
                "- [L{}]({}) — `{}` `{}`",
                symbol.line + 1,
                link_dest(&style, doc.rel, Some(symbol.line)),
                symbol.kind,
                symbol.name
            ));
            if let Some(owner) = &symbol.owner {
                out.push_str(&format!(" · impl `{owner}`"));
            }
            if symbol.test {
                out.push_str(" · test");
            }
            out.push('\n');
        }
        if doc.symbols.len() < doc.total_symbols {
            out.push_str(&format!(
                "\n_(showing {} of {})_\n",
                doc.symbols.len(),
                doc.total_symbols
            ));
        }
        out.push('\n');
    }

    // ── References ──────────────────────────────────────────────
    if let Some(name) = doc.symbol_name {
        out.push_str(&format!("## References for `{name}`\n\n"));

        out.push_str("### Referenced by\n\n");
        if doc.referenced_by.is_empty() {
            out.push_str("_None indexed._\n\n");
        } else {
            for row in doc.referenced_by {
                out.push_str(&format!(
                    "- {} — `{}` `{}`\n",
                    link(&style, &row.file_path, row.line),
                    row.kind,
                    row.name
                ));
            }
            out.push('\n');
        }

        out.push_str("### References\n\n");
        if doc.references.is_empty() {
            out.push_str("_None indexed._\n\n");
        } else {
            for row in doc.references {
                out.push_str(&format!(
                    "- {} — `{}` `{}`\n",
                    link(&style, &row.file_path, row.line),
                    row.kind,
                    row.name
                ));
            }
            out.push('\n');
        }
    }

    // ── Depends on / Used by ────────────────────────────────────
    out.push_str(&format!("## Depends on (depth {})\n\n", doc.depth));
    if doc.dependencies.is_empty() {
        out.push_str(&format!("_None at depth {}._\n\n", doc.depth));
    } else {
        for row in doc.dependencies {
            out.push_str(&format!(
                "- {} — `{}` · via `{}`\n",
                link(&style, &row.file_path, row.line),
                row.kind,
                row.via
            ));
        }
        if doc.dependencies.len() < doc.total_dependencies {
            out.push_str(&format!(
                "\n_(showing {} of {})_\n",
                doc.dependencies.len(),
                doc.total_dependencies
            ));
        }
        out.push('\n');
    }

    out.push_str(&format!("## Used by (depth {})\n\n", doc.depth));
    if doc.dependents.is_empty() {
        out.push_str(&format!("_None at depth {}._\n", doc.depth));
    } else {
        for row in doc.dependents {
            out.push_str(&format!(
                "- {} — `{}` · via `{}`\n",
                link(&style, &row.file_path, row.line),
                row.kind,
                row.via
            ));
        }
        if doc.dependents.len() < doc.total_dependents {
            out.push_str(&format!(
                "\n_(showing {} of {})_\n",
                doc.dependents.len(),
                doc.total_dependents
            ));
        }
    }

    out
}

/// The panel rects, derived from its outer area once so render and click
/// hit-testing cannot drift.
pub struct PanelGeometry {
    pub body: Rect,
    pub footer: Rect,
}

/// The CODE GRAPH panel's state.
pub struct CodeGraphPanel {
    pub markdown: String,
    pub lines: Vec<ChatLine>,
    /// First rendered line per distinct href, in document order.
    pub links: Vec<GraphLink>,
    pub rendered_width: u16,
    pub root: PathBuf,
    pub source_file: String,
    pub depth: usize,
    pub status: String,
    pub graph_ready: bool,
    /// Owned document rows. `markdown` is derived from these; export
    /// re-renders them with [`LinkStyle::Absolute`] instead of re-querying.
    pub symbols: Vec<SymbolRow>,
    pub total_symbols: usize,
    pub dependencies: Vec<EdgeRow>,
    pub total_dependencies: usize,
    pub dependents: Vec<EdgeRow>,
    pub total_dependents: usize,
    pub referenced_by: Vec<RefRow>,
    pub references: Vec<RefRow>,
    pub symbol_name: Option<String>,
    /// `(batches_applied, full_scans)`. `full_scans` matters because the
    /// startup scan publishes without incrementing `batches_applied`.
    pub witnesses_generation: (u64, u64),
    /// Root the cached witnesses belong to; counters are per service, so a
    /// multi-folder switch must invalidate even at equal counts.
    pub witnesses_root: PathBuf,
    pub witnesses: Vec<gaviero_core::repo_map::store::FileEdgeWitness>,
    pub selected_link: Option<usize>,
    pub scroll: ScrollState,
}

impl Default for CodeGraphPanel {
    fn default() -> Self {
        Self {
            markdown: String::new(),
            lines: Vec::new(),
            links: Vec::new(),
            rendered_width: 0,
            root: PathBuf::new(),
            source_file: String::new(),
            depth: 1,
            status: String::new(),
            graph_ready: false,
            symbols: Vec::new(),
            total_symbols: 0,
            dependencies: Vec::new(),
            total_dependencies: 0,
            dependents: Vec::new(),
            total_dependents: 0,
            referenced_by: Vec::new(),
            references: Vec::new(),
            symbol_name: None,
            // Not `(0, 0)`: the first refresh must always run.
            witnesses_generation: (u64::MAX, u64::MAX),
            witnesses_root: PathBuf::new(),
            witnesses: Vec::new(),
            selected_link: None,
            scroll: ScrollState::new(),
        }
    }
}

impl CodeGraphPanel {
    /// The only place that derives body/footer from the outer rect; render
    /// and click hit-testing both call it so they cannot drift.
    pub fn geometry(area: Rect) -> PanelGeometry {
        if area.height <= 1 {
            return PanelGeometry {
                body: area,
                footer: Rect::default(),
            };
        }
        PanelGeometry {
            body: Rect {
                height: area.height - 1,
                ..area
            },
            footer: Rect {
                y: area.y + area.height - 1,
                height: 1,
                ..area
            },
        }
    }

    /// Re-wrap `markdown` at `width` and rebuild `links`.
    pub fn rebuild_cache(&mut self, width: u16, base: Style) {
        // `render_lines_to_buffer` writes at `body.x + 1`, so the usable text
        // width is one column narrower than the area.
        let text_width = width.saturating_sub(1).max(1) as usize;
        self.lines = chat_markdown::format_chat_markdown(&self.markdown, text_width, base);
        self.links.clear();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (index, line) in self.lines.iter().enumerate() {
            for segment in &line.segments {
                let Some(href) = &segment.href else {
                    continue;
                };
                if seen.insert(href.clone()) {
                    self.links.push(GraphLink {
                        line: index,
                        href: href.clone(),
                    });
                }
            }
        }
        self.rendered_width = width;
    }

    /// Paint the panel body and footer. No graph or filesystem access.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, base: Style) {
        let geo = Self::geometry(area);
        if geo.body.width != self.rendered_width {
            self.rebuild_cache(geo.body.width, base);
        }
        self.scroll.set_viewport(geo.body.height as usize);
        self.scroll.clamp(self.lines.len());
        chat_markdown::render_lines_to_buffer(
            &self.lines,
            geo.body,
            buf,
            self.scroll.offset,
            base,
            None,
        );

        if geo.footer.height == 0 || geo.footer.width == 0 {
            return;
        }
        // ratatui does not clear between frames; a shorter status would leave
        // stale cells from the previous mode.
        for col in 0..geo.footer.width {
            let cx = geo.footer.x + col;
            if cx < buf.area().right() && geo.footer.y < buf.area().bottom() {
                buf[(cx, geo.footer.y)].set_char(' ').set_style(base);
            }
        }
        let text = if self.status.is_empty() {
            FOOTER_HELP
        } else {
            self.status.as_str()
        };
        crate::widgets::render_utils::write_text(
            buf,
            geo.footer.x + 1,
            geo.footer.y,
            geo.footer.right(),
            text,
            Style::default().fg(theme::TOOL_DIM),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn symbol(line: usize, kind: &str, name: &str) -> SymbolRow {
        SymbolRow {
            line,
            kind: kind.to_string(),
            name: name.to_string(),
            owner: None,
            test: false,
        }
    }

    fn doc<'a>(
        rel: &'a str,
        symbols: &'a [SymbolRow],
        dependencies: &'a [EdgeRow],
        dependents: &'a [EdgeRow],
    ) -> GraphDoc<'a> {
        GraphDoc {
            rel,
            symbol_name: None,
            symbols,
            total_symbols: symbols.len(),
            dependencies,
            total_dependencies: dependencies.len(),
            dependents,
            total_dependents: dependents.len(),
            referenced_by: &[],
            references: &[],
            depth: 1,
            graph_ready: true,
        }
    }

    #[test]
    fn code_graph_markdown_lists_symbols_as_links() {
        // 0-based rows; the document shows them 1-based.
        let symbols = vec![
            symbol(119, "function_item", "open_file"),
            SymbolRow {
                line: 239,
                kind: "function_item".into(),
                name: "open_file_at".into(),
                owner: Some("App".into()),
                test: false,
            },
        ];
        let out = markdown_doc(&doc("src/app.rs", &symbols, &[], &[]), LinkStyle::Repo);
        assert!(
            out.contains("- [L120](src/app.rs#L120) — `function_item` `open_file`"),
            "{out}"
        );
        assert!(out.contains("impl `App`"), "{out}");
    }

    #[test]
    fn code_graph_markdown_marks_truncation() {
        let symbols = vec![
            symbol(0, "function_item", "a"),
            symbol(1, "function_item", "b"),
        ];
        let mut d = doc("src/app.rs", &symbols, &[], &[]);
        d.total_symbols = 250;
        let out = markdown_doc(&d, LinkStyle::Repo);
        assert!(out.contains("_(showing 2 of 250)_"), "{out}");
    }

    #[test]
    fn code_graph_markdown_dependents_link_the_call_site() {
        let dependents = vec![EdgeRow {
            file_path: "src/baz.rs".into(),
            line: Some(88),
            kind: "Calls".into(),
            via: "src/baz.rs:89 → src/app.rs".into(),
        }];
        let out = markdown_doc(&doc("src/app.rs", &[], &[], &dependents), LinkStyle::Repo);
        assert!(out.contains("[src/baz.rs:89](src/baz.rs#L89)"), "{out}");
    }

    #[test]
    fn code_graph_markdown_dependencies_do_not_link_a_foreign_line() {
        let dependencies = vec![EdgeRow {
            file_path: "src/bar.rs".into(),
            line: None,
            kind: "Imports".into(),
            via: "src/app.rs:7 → src/bar.rs".into(),
        }];
        let out = markdown_doc(&doc("src/app.rs", &[], &dependencies, &[]), LinkStyle::Repo);
        assert!(out.contains("[src/bar.rs](src/bar.rs)"), "{out}");
        assert!(!out.contains("src/bar.rs#L"), "{out}");
    }

    #[test]
    fn code_graph_markdown_absolute_links_use_file_urls() {
        let symbols = vec![symbol(120, "function_item", "open_file")];
        let style = LinkStyle::Absolute(PathBuf::from("/work/repo"));
        let out = markdown_doc(&doc("src/app.rs", &symbols, &[], &[]), style);
        assert!(out.contains("(file:///work/repo/src/app.rs#L121)"), "{out}");
    }

    #[test]
    fn code_graph_markdown_percent_encodes_spaces() {
        let symbols = vec![symbol(0, "function_item", "f")];
        let style = LinkStyle::Absolute(PathBuf::from("/work/repo"));
        let out = markdown_doc(&doc("src/a b.rs", &symbols, &[], &[]), style);
        assert!(out.contains("src/a%20b.rs"), "{out}");
    }

    #[test]
    fn code_graph_markdown_says_when_the_graph_is_not_ready() {
        let symbols: Vec<SymbolRow> = Vec::new();
        let mut d = doc("src/app.rs", &symbols, &[], &[]);
        d.graph_ready = false;
        let out = markdown_doc(&d, LinkStyle::Repo);
        assert!(out.contains("not built yet"), "{out}");
        assert!(!out.contains("## Symbols"), "{out}");
    }

    // ── Panel state / render ────────────────────────────────────

    #[test]
    fn code_graph_panel_renders_markdown() {
        let mut panel = CodeGraphPanel {
            markdown: "# Title\n\n- [L1](src/a.rs#L1) — `f` `g`\n".to_string(),
            ..CodeGraphPanel::default()
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 20));
        panel.render(Rect::new(0, 0, 80, 20), &mut buf, Style::default());
        let rendered: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(rendered.contains("Title"), "{rendered}");
        assert!(rendered.contains("L1"), "{rendered}");
        assert!(rendered.contains('g'), "{rendered}");
    }

    #[test]
    fn code_graph_panel_rebuilds_cache_on_width_change() {
        let mut panel = CodeGraphPanel {
            markdown: "# Title\n\nbody text\n".to_string(),
            ..CodeGraphPanel::default()
        };
        let mut wide = Buffer::empty(Rect::new(0, 0, 80, 20));
        panel.render(Rect::new(0, 0, 80, 20), &mut wide, Style::default());
        assert_eq!(panel.rendered_width, 80);
        assert!(!panel.lines.is_empty());

        let mut narrow = Buffer::empty(Rect::new(0, 0, 40, 20));
        panel.render(Rect::new(0, 0, 40, 20), &mut narrow, Style::default());
        assert_eq!(panel.rendered_width, 40);
        assert!(!panel.lines.is_empty());
    }

    #[test]
    fn code_graph_panel_collects_one_link_per_href() {
        let mut panel = CodeGraphPanel {
            markdown: "[a very long link label that must wrap](src/a.rs#L1)\n".to_string(),
            ..CodeGraphPanel::default()
        };
        let area = Rect::new(0, 0, 24, 12);
        let mut buf = Buffer::empty(area);
        panel.render(area, &mut buf, Style::default());
        assert!(panel.lines.len() > 1, "the label must wrap");
        assert_eq!(panel.links.len(), 1, "wrapped pieces share one link");
        assert_eq!(panel.links[0].href, "src/a.rs#L1");
    }

    #[test]
    fn code_graph_panel_geometry_reserves_the_footer() {
        let geo = CodeGraphPanel::geometry(Rect::new(5, 3, 40, 10));
        assert_eq!(geo.body.height, 9);
        assert_eq!(geo.body.y, 3);
        assert_eq!(geo.footer.y, 12);
        assert_eq!(geo.footer.height, 1);
        let tiny = CodeGraphPanel::geometry(Rect::new(0, 0, 10, 1));
        assert_eq!(tiny.body.height, 1);
        assert_eq!(tiny.footer, Rect::default());
    }
}
