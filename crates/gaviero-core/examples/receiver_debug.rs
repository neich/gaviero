//! Show how the resolver types the receivers of Rust method calls on one
//! line (receiver-typing plan; a debugging aid).
//!
//! ```text
//! receiver_debug <root> <graph-db> <file> <line> [<method>]
//! ```
//!
//! Builds (or updates) the graph of `<root>` in `<graph-db>` (a temp path),
//! then for each method call starting on `<line>` (1-based) of `<file>`
//! (repo-relative) prints the receiver expression, each evaluation step and
//! the final outcome.

use gaviero_core::repo_map::edges::{CallStyle, extract_references_for_extension};
use gaviero_core::repo_map::graph_builder::build_graph_at;
use gaviero_core::repo_map::resolve::{FileScope, Lang, ResolveContext};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, db, file, line, rest @ ..] = args.as_slice() else {
        anyhow::bail!("usage: receiver_debug <root> <graph-db> <file> <line> [<method>]");
    };
    let root = std::path::PathBuf::from(root);
    let line: usize = line.parse()?;
    let (store, _) = build_graph_at(&root, std::path::Path::new(db), &[])?;
    let ctx = ResolveContext::build(&store, &root)?;
    let content = std::fs::read_to_string(root.join(file))?;
    let refs = extract_references_for_extension("rs", &content);
    let scope = FileScope::build(Lang::Rust, &refs);
    for r in &refs {
        if r.style != CallStyle::Method || r.line + 1 != line {
            continue;
        }
        if let Some(m) = rest.first()
            && *m != r.target_name
        {
            continue;
        }
        println!("== .{}()", r.target_name);
        if let Some(e) = &r.receiver_expr {
            for l in ctx.explain_receiver(file, e) {
                println!("   {l}");
            }
        }
        println!("   outcome: {:?}", ctx.resolve(file, Lang::Rust, &scope, r));
    }
    Ok(())
}
