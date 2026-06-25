//! S2.1 / S2.2 — rustdoc JSON enrichment into the `symbol_docs` sidecar.
//!
//! Triggered explicitly via `gaviero-cli --graph --enrich` (never at
//! workspace-open). Parses `cargo +nightly rustdoc --output-format json`,
//! keys rows on the graph's existing `qualified_name` (`{rel_path}::{name}`),
//! and optionally embeds `signature + doc + role_summary` via the active
//! memory embedder.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use rustdoc_types::{Crate, Item, ItemEnum, Span};
use serde::Deserialize;

use crate::memory::{Embedder, build_embedder_by_name};

use super::store::{GraphStore, SymbolDoc};

/// Options for a symbol-enrichment pass.
#[derive(Debug, Clone)]
pub struct SymbolEnrichOpts {
    /// When true, compute and store embedding BLOBs (S2.2).
    pub embed: bool,
    /// Embedder alias (`nomic`, `gte-modernbert`, …). `None` = workspace default.
    pub embedder_name: Option<String>,
}

/// Outcome of [`enrich_graph`].
#[derive(Debug, Default)]
pub struct SymbolEnrichResult {
    pub crates_processed: usize,
    pub symbols_written: usize,
    pub symbols_unmatched: usize,
    pub symbols_skipped_hash: usize,
    pub rustdoc_failures: Vec<String>,
    pub fallback_files: usize,
}

/// Run rustdoc enrichment against an already-built [`GraphStore`].
pub async fn enrich_graph(
    store: &GraphStore,
    workspace: &Path,
    opts: &SymbolEnrichOpts,
) -> Result<SymbolEnrichResult> {
    let workspace = workspace
        .canonicalize()
        .with_context(|| format!("canonicalize {}", workspace.display()))?;

    let packages = workspace_lib_packages(&workspace)?;
    if packages.is_empty() {
        bail!("no library packages found in workspace metadata");
    }

    let embedder: Option<std::sync::Arc<dyn Embedder>> = if opts.embed {
        let name = opts.embedder_name.clone().unwrap_or_default();
        let built = tokio::task::spawn_blocking(move || build_embedder_by_name(&name))
            .await
            .context("spawn embedder for symbol enrichment")??;
        Some(built)
    } else {
        None
    };

    let mut result = SymbolEnrichResult::default();
    for package in packages {
        match enrich_crate(store, &workspace, &package, &embedder, &mut result).await {
            Ok(()) => result.crates_processed += 1,
            Err(e) => {
                result
                    .rustdoc_failures
                    .push(format!("{package}: {e:#}"));
            }
        }
    }

    if result.crates_processed == 0 && !result.rustdoc_failures.is_empty() {
        bail!(
            "rustdoc enrichment failed for all crates: {}",
            result.rustdoc_failures.join("; ")
        );
    }

    Ok(result)
}

async fn enrich_crate(
    store: &GraphStore,
    workspace: &Path,
    package: &str,
    embedder: &Option<std::sync::Arc<dyn Embedder>>,
    result: &mut SymbolEnrichResult,
) -> Result<()> {
    let json_path = run_rustdoc_json(workspace, package)?;
    let json = std::fs::read_to_string(&json_path)
        .with_context(|| format!("reading {}", json_path.display()))?;
    let krate: Crate = serde_json::from_str(&json)
        .with_context(|| format!("parsing rustdoc JSON for {package}"))?;

    if krate.format_version != rustdoc_types::FORMAT_VERSION {
        tracing::warn!(
            target: "symbol_enrichment",
            package,
            got = krate.format_version,
            expected = rustdoc_types::FORMAT_VERSION,
            "rustdoc format_version mismatch — parsing may fail"
        );
    }

    for item in krate.index.values() {
        ingest_rustdoc_item(store, workspace, item, embedder, result).await?;
    }
    Ok(())
}

async fn ingest_rustdoc_item(
    store: &GraphStore,
    workspace: &Path,
    item: &Item,
    embedder: &Option<std::sync::Arc<dyn Embedder>>,
    result: &mut SymbolEnrichResult,
) -> Result<()> {
    let Some(name) = item.name.as_deref() else {
        return Ok(());
    };
    let Some((qn, rel_path)) = graph_qn_from_span(workspace, name, item.span.as_ref()) else {
        return Ok(());
    };
    if !store.has_node(&qn)? {
        result.symbols_unmatched += 1;
        return Ok(());
    }

    let file_hash = store.get_file_hash(&rel_path)?;
    if let Some(existing) = store.symbol_doc(&qn)? {
        if existing.file_hash == file_hash && !existing.signature.is_empty() {
            result.symbols_skipped_hash += 1;
            return Ok(());
        }
    }

    let (signature, bounds, role_summary) = extract_item_fields(item);
    let doc = item.docs.clone().unwrap_or_default();
    let mut row = SymbolDoc {
        qualified_name: qn.clone(),
        file_path: rel_path,
        file_hash,
        signature,
        bounds,
        doc,
        role_summary,
        embedding: None,
    };

    if let Some(emb) = embedder {
        let text = embed_text_for_symbol(&row);
        if !text.trim().is_empty() {
            row.embedding = Some(emb.embed_document(&text).await?);
        }
    }

    store.upsert_symbol_doc(&row)?;
    result.symbols_written += 1;
    Ok(())
}

/// Fallback enrichment for one file when rustdoc is unavailable: syn
/// signatures for top-level items that exist in the graph.
pub async fn enrich_file_fallback(
    store: &GraphStore,
    workspace: &Path,
    rel_path: &str,
    embedder: &Option<std::sync::Arc<dyn Embedder>>,
    result: &mut SymbolEnrichResult,
) -> Result<()> {
    let abs = workspace.join(rel_path);
    let source = match std::fs::read_to_string(&abs) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };
    let file = match syn::parse_file(&source) {
        Ok(f) => f,
        Err(_) => return Ok(()),
    };
    let file_hash = store.get_file_hash(rel_path)?;

    for item in file.items {
        let (name, signature) = match syn_item_signature(&item) {
            Some(v) => v,
            None => continue,
        };
        let qn = format!("{rel_path}::{name}");
        if !store.has_node(&qn)? {
            continue;
        }
        let mut row = SymbolDoc {
            qualified_name: qn,
            file_path: rel_path.to_string(),
            file_hash: file_hash.clone(),
            signature,
            bounds: String::new(),
            doc: String::new(),
            role_summary: String::new(),
            embedding: None,
        };
        if let Some(emb) = embedder {
            let text = embed_text_for_symbol(&row);
            if !text.trim().is_empty() {
                row.embedding = Some(emb.embed_document(&text).await?);
            }
        }
        store.upsert_symbol_doc(&row)?;
        result.symbols_written += 1;
    }
    result.fallback_files += 1;
    Ok(())
}

fn graph_qn_from_span(
    workspace: &Path,
    name: &str,
    span: Option<&Span>,
) -> Option<(String, String)> {
    let span = span?;
    let rel = if span.filename.is_absolute() {
        span.filename
            .strip_prefix(workspace)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/")
    } else {
        span.filename.to_string_lossy().replace('\\', "/")
    };
    Some((format!("{rel}::{name}"), rel))
}

fn extract_item_fields(item: &Item) -> (String, String, String) {
    let name = item.name.as_deref().unwrap_or("?");
    match &item.inner {
        ItemEnum::Function(f) => {
            let bounds = format_generics(&f.generics);
            (
                format_function_sig(name, &f.sig),
                bounds,
                "function".to_string(),
            )
        }
        ItemEnum::Struct(s) => {
            let sig = format!("struct {name}");
            let bounds = format_generics(&s.generics);
            (sig, bounds, "struct".to_string())
        }
        ItemEnum::Enum(e) => {
            let sig = format!("enum {name}");
            let bounds = format_generics(&e.generics);
            (sig, bounds, "enum".to_string())
        }
        ItemEnum::Trait(t) => {
            let sig = format!("trait {name}");
            let bounds = format_generics(&t.generics);
            (sig, bounds, "trait".to_string())
        }
        ItemEnum::Constant { type_, .. } => {
            let sig = format!("const {name}: {}", format_rustdoc_type(type_));
            (sig, String::new(), "const".to_string())
        }
        ItemEnum::TypeAlias(ta) => {
            let sig = format!("type {name} = {}", format_rustdoc_type(&ta.type_));
            let bounds = format_generics(&ta.generics);
            (sig, bounds, "type".to_string())
        }
        _ => (
            item.name.clone().unwrap_or_default(),
            String::new(),
            "item".to_string(),
        ),
    }
}

fn format_function_sig(name: &str, sig: &rustdoc_types::FunctionSignature) -> String {
    let mut out = format!("fn {name}(");
    for (i, (arg_name, ty)) in sig.inputs.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(arg_name);
        out.push_str(": ");
        out.push_str(&format_rustdoc_type(ty));
    }
    out.push(')');
    if let Some(ret) = &sig.output {
        out.push_str(" -> ");
        out.push_str(&format_rustdoc_type(ret));
    }
    out
}

fn format_rustdoc_type(ty: &rustdoc_types::Type) -> String {
    use rustdoc_types::Type;
    match ty {
        Type::ResolvedPath(p) => p.path.clone(),
        Type::Generic(s) | Type::Primitive(s) => s.clone(),
        Type::Tuple(v) => {
            let inner: Vec<_> = v.iter().map(format_rustdoc_type).collect();
            format!("({})", inner.join(", "))
        }
        Type::Slice(t) => format!("[{}]", format_rustdoc_type(t)),
        Type::Array { type_, .. } => format!("[{}; _]", format_rustdoc_type(type_)),
        Type::RawPointer { is_mutable, type_ } => {
            let star = if *is_mutable { "*mut " } else { "*const " };
            format!("{star}{}", format_rustdoc_type(type_))
        }
        Type::BorrowedRef {
            is_mutable, type_, ..
        } => {
            let mut prefix = "&".to_string();
            if *is_mutable {
                prefix.push_str("mut ");
            }
            format!("{prefix}{}", format_rustdoc_type(type_))
        }
        _ => format!("{ty:?}"),
    }
}

fn format_generics(generics: &rustdoc_types::Generics) -> String {
    if generics.params.is_empty() {
        return String::new();
    }
    generics
        .params
        .iter()
        .map(|p| p.name.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

fn embed_text_for_symbol(doc: &SymbolDoc) -> String {
    let mut out = String::new();
    if !doc.signature.is_empty() {
        out.push_str(&doc.signature);
        out.push('\n');
    }
    if !doc.bounds.is_empty() {
        out.push_str(&doc.bounds);
        out.push('\n');
    }
    if !doc.role_summary.is_empty() {
        out.push_str(&doc.role_summary);
        out.push('\n');
    }
    if !doc.doc.is_empty() {
        out.push_str(&doc.doc);
    }
    out
}

fn syn_item_signature(item: &syn::Item) -> Option<(String, String)> {
    use syn::Item;
    match item {
        Item::Fn(f) => Some((f.sig.ident.to_string(), quote_sig(&f.sig))),
        Item::Struct(s) => Some((s.ident.to_string(), format!("struct {}", s.ident))),
        Item::Enum(e) => Some((e.ident.to_string(), format!("enum {}", e.ident))),
        Item::Trait(t) => Some((t.ident.to_string(), format!("trait {}", t.ident))),
        Item::Const(c) => Some((
            c.ident.to_string(),
            format!("const {}: {}", c.ident, type_to_string(&c.ty)),
        )),
        Item::Type(t) => Some((
            t.ident.to_string(),
            format!("type {} = {}", t.ident, type_to_string(&t.ty)),
        )),
        _ => None,
    }
}

fn quote_sig(sig: &syn::Signature) -> String {
    let mut out = String::new();
    if sig.constness.is_some() {
        out.push_str("const ");
    }
    if sig.asyncness.is_some() {
        out.push_str("async ");
    }
    if sig.unsafety.is_some() {
        out.push_str("unsafe ");
    }
    out.push_str("fn ");
    out.push_str(&sig.ident.to_string());
    out.push('(');
    for (i, arg) in sig.inputs.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        if let syn::FnArg::Typed(pat) = arg {
            out.push_str(&type_to_string(&pat.ty));
        }
    }
    out.push(')');
    if let syn::ReturnType::Type(_, ty) = &sig.output {
        out.push_str(" -> ");
        out.push_str(&type_to_string(ty));
    }
    out
}

fn type_to_string(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(p) => p
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        _ => "…".to_string(),
    }
}

fn run_rustdoc_json(workspace: &Path, package: &str) -> Result<PathBuf> {
    let status = Command::new("cargo")
        .current_dir(workspace)
        .args([
            "+nightly-2026-06-15",
            "rustdoc",
            "-p",
            package,
            "--lib",
            "--",
            "-Z",
            "unstable-options",
            "--output-format",
            "json",
        ])
        .status()
        .with_context(|| format!("spawning cargo rustdoc for {package}"))?;
    if !status.success() {
        bail!("cargo rustdoc failed for {package} (exit {status})");
    }

    let json_path = workspace.join("target/doc").join(format!("{package}.json"));
    if json_path.exists() {
        return Ok(json_path);
    }
    let doc_dir = workspace.join("target/doc");
    let mut matches = Vec::new();
    if doc_dir.is_dir() {
        for entry in std::fs::read_dir(&doc_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json")
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| !s.starts_with('.'))
            {
                matches.push(path);
            }
        }
    }
    if matches.len() == 1 {
        return Ok(matches.pop().expect("checked len"));
    }
    bail!(
        "rustdoc JSON not found at {} (found {} json files in target/doc)",
        json_path.display(),
        matches.len()
    );
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    name: String,
    targets: Vec<CargoTarget>,
}

#[derive(Debug, Deserialize)]
struct CargoTarget {
    kind: Vec<String>,
}

fn workspace_lib_packages(workspace: &Path) -> Result<Vec<String>> {
    let output = Command::new("cargo")
        .current_dir(workspace)
        .args(["metadata", "--format-version=1", "--no-deps"])
        .output()
        .context("cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let meta: CargoMetadata =
        serde_json::from_slice(&output.stdout).context("parse cargo metadata")?;
    Ok(meta
        .packages
        .into_iter()
        .filter(|p| p.targets.iter().any(|t| t.kind.iter().any(|k| k == "lib")))
        .map(|p| p.name)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_qn_from_workspace_relative_span() {
        let ws = std::env::current_dir().unwrap();
        let rel = "crates/gaviero-core/src/lib.rs";
        let span = Span {
            filename: PathBuf::from(rel),
            begin: (0, 0),
            end: (1, 0),
        };
        let (qn, path) = graph_qn_from_span(&ws, "foo", Some(&span)).unwrap();
        assert_eq!(path, rel);
        assert_eq!(qn, format!("{rel}::foo"));
    }
}
