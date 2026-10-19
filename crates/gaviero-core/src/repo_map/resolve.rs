//! Qualified, import-aware reference resolution (edge-resolution plan
//! E2–E5).
//!
//! Bare-name resolution linked a call `Widget::new()` to *every* symbol
//! named `new`. Here a reference is resolved through what the source
//! actually says, and each step only narrows:
//!
//! - **Rust:** the written path (`crate::` / `self::` / `super::` / crate
//!   names / child modules), the file's `use` bindings and globs, `pub use`
//!   re-exports (followed up to [`MAX_REEXPORT_HOPS`]), and symbol owners
//!   (`impl:Type`, `trait:Name`) for `Type::method()` and `.method()` calls.
//!   The crate/module map comes from `Cargo.toml` files and the `src/` tree.
//! - **TypeScript / Python:** see [`ResolveContext::resolve_ts`] and
//!   [`ResolveContext::resolve_py`].
//!
//! The last fallback everywhere is the name (`global_name`): definitions in
//! files of the same language, free items only for `foo()` and methods only
//! for `.foo()`. Production files never target test files. Whatever is
//! still ambiguous — candidates spanning more than
//! [`RESOLVE_MAX_TARGET_FILES`] files — is **dropped and counted**, not
//! fanned out.
//!
//! Resolution is file-level by design: `blast_radius` asks which *files*
//! are affected, so a target only has to land in the right file.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use super::edges::{CallStyle, RawReference, RefKind, is_test_file};
use super::store::{Definition, EdgeKind, GraphStore};

mod receiver;

/// Candidates spanning more files than this are dropped as ambiguous.
///
/// Chosen by measurement (edge-resolution plan E7, 2026-10-18): on a
/// hand-labelled, stratified sample of 200 references from this repository
/// (seed 20261018), cap 1 scored pair precision 0.93 / relative recall
/// 0.95 / F1 0.94, against F1 0.88 for cap 2 and 0.87 for cap 3. Almost
/// every two- and three-file candidate set came from the by-name fallback
/// on a common method name (`lock`, `take`, `is_none`, …) that the receiver
/// actually got from std. Scored with `research/edge_resolution_score.py`.
///
/// Re-checked after receiver typing (receiver-typing plan R7, 2026-10-18)
/// on a fresh stratified sample (seed 20261020, 200 references): cap 1
/// precision 1.000 / relative recall 0.984 / F1 0.992, cap 2 F1 0.990,
/// cap 3 F1 0.989. Typing removed most multi-file candidate sets (the
/// two-file population fell from 1,057 to 154), so a wider cap buys almost
/// no recall and still costs precision.
pub const RESOLVE_MAX_TARGET_FILES: usize = 1;

/// Re-export chains longer than this are not followed (cycle-safe).
pub const MAX_REEXPORT_HOPS: usize = 4;

/// Per-build resolution counters (reported in `BuildResult`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolveStats {
    /// References resolved, by the step that decided them.
    pub resolved: BTreeMap<String, usize>,
    /// References that resolve inside their own file (no edge needed).
    pub local: usize,
    /// References with no candidate (external crates/packages, std, macros).
    pub unresolved: usize,
    /// References whose candidates spanned too many files.
    pub dropped_ambiguous: usize,
    /// Dropped references by name (for the debug log / report).
    pub dropped_names: BTreeMap<String, usize>,
    /// Method calls (`.foo()`) linked by the by-name fallback, by name.
    pub method_global_names: BTreeMap<String, usize>,
    /// Outcomes that are not edges but were decided on purpose, by reason
    /// (e.g. a method call on an external receiver).
    pub no_edge: BTreeMap<String, usize>,
    /// Rust method calls by how their receiver was written
    /// ([`super::types::ReceiverExpr::shape`]).
    pub receiver_shapes: BTreeMap<String, usize>,
}

impl ResolveStats {
    pub fn record(&mut self, outcome: &Outcome, r: &RawReference) {
        let name = r.target_name.as_str();
        if let Some(e) = &r.receiver_expr {
            *self.receiver_shapes.entry(e.shape().to_string()).or_default() += 1;
        }
        match outcome {
            Outcome::Files { step, .. } => {
                *self.resolved.entry((*step).to_string()).or_default() += 1;
                if *step == "global_name" && r.style == CallStyle::Method {
                    *self.method_global_names.entry(name.to_string()).or_default() += 1;
                }
            }
            Outcome::Local => self.local += 1,
            Outcome::Unresolved => self.unresolved += 1,
            Outcome::NoEdge { reason } => *self.no_edge.entry((*reason).to_string()).or_default() += 1,
            Outcome::Ambiguous { .. } => {
                self.dropped_ambiguous += 1;
                *self.dropped_names.entry(name.to_string()).or_default() += 1;
            }
        }
    }
}

/// Languages with structured resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    TypeScript,
    Python,
}

/// The resolution language of a path, if it has one.
pub fn lang_of(path: &str) -> Option<Lang> {
    let ext = path.rsplit('.').next()?;
    match ext {
        "rs" => Some(Lang::Rust),
        "ts" | "tsx" => Some(Lang::TypeScript),
        "py" | "pyi" => Some(Lang::Python),
        _ => None,
    }
}

/// What one reference resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Target `(file, item name)` pairs, all outside the source file.
    /// A `None` name targets the file itself (a module import).
    Files {
        step: &'static str,
        targets: BTreeSet<(String, Option<String>)>,
    },
    Local,
    Unresolved,
    /// Decided on purpose that there is no workspace target (e.g. a method
    /// call whose receiver has an external type). Counted by `reason`.
    NoEdge { reason: &'static str },
    /// Candidates in more files than the cap: dropped, no edge.
    Ambiguous { step: &'static str, files: Vec<String> },
}

// ── Rust crate / module map (E3) ─────────────────────────────────────

/// File ↔ module path for every Rust file under a crate's `src/`.
///
/// Module paths are `crate_name::a::b` (`-` in package names becomes `_`;
/// `[lib] name` wins). `lib.rs` (or, without one, `main.rs`) is the crate
/// root; `a/b.rs` and `a/b/mod.rs` are `crate::a::b`. Files under
/// `src/bin/`, `tests/`, `examples/` and `benches/` are their own roots
/// (`crate#path`), so they can name the crate but are not part of it.
/// Inline `mod x { … }` blocks and `#[path]` attributes are not modelled.
#[derive(Debug, Default, Clone)]
pub struct RustModuleMap {
    module_file: HashMap<String, String>,
    file_module: HashMap<String, String>,
    /// Crate name → its root module file.
    crate_roots: HashMap<String, String>,
}

impl RustModuleMap {
    /// Build from the indexed file list and the `Cargo.toml`s under `root`.
    pub fn build(root: &Path, files: &[String]) -> Self {
        let mut map = Self::default();
        let mut crate_of_dir: HashMap<String, Option<(String, String)>> = HashMap::new();
        let mut rs: Vec<&String> = files.iter().filter(|f| f.ends_with(".rs")).collect();
        rs.sort();
        for file in rs {
            let Some((crate_dir, crate_name)) = find_crate(root, file, &mut crate_of_dir) else {
                continue;
            };
            let in_crate = if crate_dir.is_empty() {
                file.as_str()
            } else {
                match file.strip_prefix(&format!("{crate_dir}/")) {
                    Some(r) => r,
                    None => continue,
                }
            };
            let module = match in_crate.strip_prefix("src/") {
                Some(src_rel) if !src_rel.starts_with("bin/") => {
                    let mut segs: Vec<&str> = src_rel.trim_end_matches(".rs").split('/').collect();
                    if segs.last() == Some(&"mod") {
                        segs.pop();
                    }
                    if segs == ["lib"] || segs == ["main"] {
                        crate_name.clone()
                    } else {
                        let mut m = crate_name.clone();
                        for s in segs {
                            m.push_str("::");
                            m.push_str(s);
                        }
                        m
                    }
                }
                _ => format!("{crate_name}#{in_crate}"),
            };
            let is_lib = in_crate == "src/lib.rs";
            if module == crate_name {
                // Prefer lib.rs as the crate's external root.
                if is_lib || !map.crate_roots.contains_key(&crate_name) {
                    map.crate_roots.insert(crate_name.clone(), file.clone());
                    map.module_file.insert(module.clone(), file.clone());
                }
            } else {
                map.module_file.entry(module.clone()).or_insert_with(|| file.clone());
            }
            map.file_module.insert(file.clone(), module);
        }
        map
    }

    pub fn module_of(&self, file: &str) -> Option<&str> {
        self.file_module.get(file).map(String::as_str)
    }

    pub fn file_of(&self, module: &str) -> Option<&str> {
        self.module_file.get(module).map(String::as_str)
    }

    pub fn is_module(&self, module: &str) -> bool {
        self.module_file.contains_key(module)
    }

    pub fn crate_root(&self, name: &str) -> Option<&str> {
        self.crate_roots.get(name).map(String::as_str)
    }

    /// The crate a module belongs to (`alpha::engine` → `alpha`;
    /// `alpha#tests/x.rs` → `alpha`).
    pub fn crate_of(module: &str) -> &str {
        let head = module.split("::").next().unwrap_or(module);
        head.split('#').next().unwrap_or(head)
    }
}

fn parent_module(module: &str) -> Option<String> {
    module.rfind("::").map(|i| module[..i].to_string())
}

/// The crate owning `file`: nearest ancestor directory (within `root`)
/// whose `Cargo.toml` has a `[package]`. Cached per directory.
fn find_crate(
    root: &Path,
    file: &str,
    cache: &mut HashMap<String, Option<(String, String)>>,
) -> Option<(String, String)> {
    let mut dir = file.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
    let mut visited = Vec::new();
    let found = loop {
        if let Some(hit) = cache.get(&dir) {
            break hit.clone();
        }
        visited.push(dir.clone());
        let manifest = if dir.is_empty() {
            root.join("Cargo.toml")
        } else {
            root.join(&dir).join("Cargo.toml")
        };
        if let Some(name) = package_crate_name(&manifest) {
            break Some((dir.clone(), name));
        }
        match dir.rsplit_once('/') {
            Some((up, _)) => dir = up.to_string(),
            None if !dir.is_empty() => dir.clear(),
            None => break None,
        }
    };
    for d in visited {
        cache.insert(d, found.clone());
    }
    found
}

/// `[lib] name`, else `[package] name` with `-` → `_`. `None` for a
/// workspace-only manifest or an unreadable file.
fn package_crate_name(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let value: toml::Value = toml::from_str(&text).ok()?;
    let package = value.get("package")?.get("name")?.as_str()?;
    let lib = value
        .get("lib")
        .and_then(|l| l.get("name"))
        .and_then(|n| n.as_str());
    Some(lib.unwrap_or(package).replace('-', "_"))
}

// ── Rust import tables (E2) ──────────────────────────────────────────

/// A Rust file's `use` / `extern crate` bindings.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RustImports {
    /// Local name → written path (`h` → `beta_core::util::helper`).
    pub bindings: HashMap<String, Vec<String>>,
    /// Glob-imported module paths (`use engine::*`).
    pub globs: Vec<Vec<String>>,
}

impl RustImports {
    pub fn from_refs(refs: &[RawReference]) -> Self {
        let mut out = Self::default();
        for r in refs.iter().filter(|r| r.kind == RefKind::Import && !r.path.is_empty()) {
            if r.glob {
                out.globs.push(r.path.clone());
                continue;
            }
            let local = r.alias.clone().unwrap_or_else(|| r.target_name.clone());
            if local == "_" || local == "self" {
                continue;
            }
            out.bindings.insert(local, r.path.clone());
        }
        out
    }
}

/// `pub use` re-exports of a Rust file as `(name, target, glob)` rows for
/// the store (`target` = the written path joined with `::`).
pub fn rust_reexports(refs: &[RawReference]) -> Vec<(String, String, bool)> {
    refs.iter()
        .filter(|r| r.kind == RefKind::Import && r.public && !r.path.is_empty())
        .map(|r| {
            let name = if r.glob {
                "*".to_string()
            } else {
                r.alias.clone().unwrap_or_else(|| r.target_name.clone())
            };
            (name, r.path.join("::"), r.glob)
        })
        .collect()
}

/// A resolved Rust path.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RustRes {
    /// A module (its path).
    Module(String),
    /// An item defined in `file`.
    Item { file: String, name: String },
    /// `Type::name`: a member of type `ty` defined in `file`.
    Member { ty: String, file: String, name: String },
}

// ── Context ──────────────────────────────────────────────────────────

/// Workspace-wide indexes for one resolution pass, built after every
/// changed file's symbols and re-exports are in the store.
pub struct ResolveContext {
    defs_by_file: HashMap<String, HashMap<String, Vec<Definition>>>,
    defs_by_name: HashMap<String, Vec<Definition>>,
    /// `(type, method)` → files with that method in an `impl` of the type.
    owner_methods: HashMap<(String, String), BTreeSet<String>>,
    /// method → `(trait, file)` for trait-declared methods.
    trait_methods: HashMap<String, Vec<(String, String)>>,
    /// file → `(name, target, glob)` re-exports.
    reexports: HashMap<String, Vec<(String, String, bool)>>,
    // ── Receiver typing (receiver/R4) ──
    /// Every Rust file's `use` table, from `rust_imports`.
    file_imports: HashMap<String, RustImports>,
    /// `(file, type)` → field → type text.
    type_fields: HashMap<(String, String), HashMap<String, String>>,
    /// fn name → `(file, owner, return type text)`.
    fn_returns: HashMap<String, Vec<(String, String, String)>>,
    /// Written type path → `(file, trait path or "")` per impl header.
    type_impls: HashMap<String, Vec<(String, String)>>,
    /// `(file, alias)` → (type parameters, aliased type text).
    aliases: HashMap<(String, String), (Vec<String>, String)>,
    /// `(written type path, method)` → `(file, implemented trait)` (`None`: inherent).
    impl_methods: HashMap<(String, String), Vec<(String, Option<String>)>>,
    max_target_files: usize,
    files: HashSet<String>,
    rust: RustModuleMap,
    py: PyModuleMap,
}

impl ResolveContext {
    pub fn build(store: &GraphStore, root: &Path) -> anyhow::Result<Self> {
        let files = store.all_file_paths()?;
        let mut defs_by_file: HashMap<String, HashMap<String, Vec<Definition>>> = HashMap::new();
        let mut defs_by_name: HashMap<String, Vec<Definition>> = HashMap::new();
        let mut owner_methods: HashMap<(String, String), BTreeSet<String>> = HashMap::new();
        let mut trait_methods: HashMap<String, Vec<(String, String)>> = HashMap::new();
        let mut impl_methods: HashMap<(String, String), Vec<(String, Option<String>)>> = HashMap::new();
        for d in store.all_definitions()? {
            if let Some(owner) = &d.owner {
                if let Some(rest) = owner.strip_prefix("impl:") {
                    let mut parts = rest.splitn(2, ':');
                    let ty = parts.next().unwrap_or(rest).to_string();
                    let tr = parts.next().map(str::to_string);
                    impl_methods
                        .entry((ty.clone(), d.name.clone()))
                        .or_default()
                        .push((d.file_path.clone(), tr));
                    owner_methods
                        .entry((ty, d.name.clone()))
                        .or_default()
                        .insert(d.file_path.clone());
                } else if let Some(tr) = owner.strip_prefix("trait:") {
                    trait_methods
                        .entry(d.name.clone())
                        .or_default()
                        .push((tr.to_string(), d.file_path.clone()));
                }
            }
            defs_by_file
                .entry(d.file_path.clone())
                .or_default()
                .entry(d.name.clone())
                .or_default()
                .push(d.clone());
            defs_by_name.entry(d.name.clone()).or_default().push(d);
        }
        let mut reexports: HashMap<String, Vec<(String, String, bool)>> = HashMap::new();
        for (file, name, target, glob) in store.all_reexports()? {
            reexports.entry(file).or_default().push((name, target, glob));
        }
        let mut file_imports: HashMap<String, RustImports> = HashMap::new();
        let mut type_fields: HashMap<(String, String), HashMap<String, String>> = HashMap::new();
        let mut fn_returns: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
        let mut type_impls: HashMap<String, Vec<(String, String)>> = HashMap::new();
        let mut aliases: HashMap<(String, String), (Vec<String>, String)> = HashMap::new();
        let type_facts = store.all_type_facts()?;
        let fact_files: HashSet<&str> = type_facts.iter().map(|(f, _)| f.as_str()).collect();
        // Persisted facts retain impl paths and every method owner; nodes do not.
        for rows in impl_methods.values_mut() {
            rows.retain(|(file, _)| !fact_files.contains(file.as_str()));
        }
        for (file, facts) in type_facts {
            let imports = file_imports.entry(file.clone()).or_default();
            for (local, path, glob) in &facts.imports {
                let segs: Vec<String> = path.split("::").map(str::to_string).collect();
                if *glob {
                    imports.globs.push(segs);
                } else {
                    imports.bindings.insert(local.clone(), segs);
                }
            }
            for (ty, field, text) in facts.fields {
                type_fields.entry((file.clone(), ty)).or_default().insert(field, text);
            }
            for (owner, name, ret, _) in facts.returns {
                fn_returns.entry(name).or_default().push((file.clone(), owner, ret));
            }
            for (ty, tr) in facts.impls {
                type_impls.entry(ty).or_default().push((file.clone(), tr));
            }
            for (name, params, text) in facts.aliases {
                let params = params.split(',').filter(|p| !p.is_empty()).map(str::to_string).collect();
                aliases.insert((file.clone(), name), (params, text));
            }
            // Methods as written per impl / trait block: they survive the
            // node collapse of same-named methods in one file.
            for (owner, name) in facts.methods {
                if let Some(rest) = owner.strip_prefix("impl:") {
                    let (ty, tr) = receiver::split_impl_owner(rest);
                    let ty = ty.to_string();
                    let tr = tr.map(str::to_string);
                    let entry = impl_methods.entry((ty, name)).or_default();
                    if !entry.iter().any(|(f, t)| *f == file && *t == tr) {
                        entry.push((file.clone(), tr));
                    }
                } else if let Some(tr) = owner.strip_prefix("trait:") {
                    let entry = trait_methods.entry(name).or_default();
                    if !entry.iter().any(|(t, f)| t == tr && *f == file) {
                        entry.push((tr.to_string(), file.clone()));
                    }
                }
            }
        }
        Ok(Self {
            file_imports,
            type_fields,
            fn_returns,
            type_impls,
            aliases,
            impl_methods,
            rust: RustModuleMap::build(root, &files),
            py: PyModuleMap::build(&files),
            files: files.into_iter().collect(),
            defs_by_file,
            defs_by_name,
            owner_methods,
            trait_methods,
            reexports,
            max_target_files: RESOLVE_MAX_TARGET_FILES,
        })
    }

    fn defines(&self, file: &str, name: &str) -> bool {
        self.defs_by_file
            .get(file)
            .is_some_and(|m| m.contains_key(name))
    }

    /// `file` defines `name` as an item a path can name: not a method
    /// inside an `impl` / `trait` block.
    fn defines_item(&self, file: &str, name: &str) -> bool {
        self.defs_by_file
            .get(file)
            .and_then(|m| m.get(name))
            .is_some_and(|ds| ds.iter().any(|d| d.owner.is_none()))
    }

    /// The file of the single definition of `name`, if there is exactly
    /// one in the workspace (`Some(None)` would be ambiguous: returns None).
    fn unique_definition(&self, name: &str) -> Option<&str> {
        match self.defs_by_name.get(name).map(Vec::as_slice) {
            Some([only]) if lang_of(&only.file_path) == Some(Lang::Rust) => Some(only.file_path.as_str()),
            _ => None,
        }
    }

    /// Resolve one reference made in `file`.
    pub fn resolve(&self, file: &str, lang: Lang, scope: &FileScope, r: &RawReference) -> Outcome {
        let outcome = match lang {
            Lang::Rust => self.resolve_rust(file, scope, r),
            Lang::TypeScript => self.resolve_ts(file, scope, r),
            Lang::Python => self.resolve_py(file, scope, r),
        };
        finish(file, outcome, self.max_target_files, is_test_file(file))
    }

    /// Override the ambiguity cap ([`RESOLVE_MAX_TARGET_FILES`]); for
    /// measuring the cap, not for builds.
    pub fn with_max_target_files(mut self, max: usize) -> Self {
        self.max_target_files = max;
        self
    }
}

/// Per-file resolution input built from the file's own references.
#[derive(Debug, Default, Clone)]
pub struct FileScope {
    pub rust: RustImports,
    pub js: JsImports,
    pub py: PyImports,
}

impl FileScope {
    pub fn build(lang: Lang, refs: &[RawReference]) -> Self {
        let mut s = Self::default();
        match lang {
            Lang::Rust => s.rust = RustImports::from_refs(refs),
            Lang::TypeScript => s.js = JsImports::from_refs(refs),
            Lang::Python => s.py = PyImports::from_refs(refs),
        }
        s
    }
}

/// Drop same-file targets and, for non-test sources, targets in test files
/// (production code cannot reach them), then apply the ambiguity cap.
fn finish(file: &str, outcome: Outcome, max_files: usize, source_is_test: bool) -> Outcome {
    match outcome {
        Outcome::Files { step, targets } => {
            let local = targets.iter().any(|(f, _)| f == file);
            let targets: BTreeSet<_> = targets
                .into_iter()
                .filter(|(f, _)| f != file && (source_is_test || !is_test_file(f)))
                .collect();
            let files: BTreeSet<&String> = targets.iter().map(|(f, _)| f).collect();
            if targets.is_empty() {
                if local { Outcome::Local } else { Outcome::Unresolved }
            } else if files.len() > max_files {
                Outcome::Ambiguous {
                    step,
                    files: files.into_iter().cloned().collect(),
                }
            } else {
                Outcome::Files { step, targets }
            }
        }
        other => other,
    }
}

fn files_of(
    step: &'static str,
    it: impl IntoIterator<Item = (String, Option<String>)>,
) -> Option<Outcome> {
    let targets: BTreeSet<_> = it.into_iter().collect();
    (!targets.is_empty()).then_some(Outcome::Files { step, targets })
}

// ── Rust resolution (E4) ─────────────────────────────────────────────

impl ResolveContext {
    /// Resolve a Rust path written in `from_file`. `imports` is that file's
    /// table, or `None` when following a path written elsewhere (a
    /// re-export target) or an import's own path.
    fn resolve_rust_path(
        &self,
        from_file: &str,
        segs: &[String],
        imports: Option<&RustImports>,
        depth: usize,
    ) -> Option<RustRes> {
        if depth > MAX_REEXPORT_HOPS || segs.is_empty() {
            return None;
        }
        let module = self.rust.module_of(from_file)?.to_string();
        let first = segs[0].as_str();
        let (base, rest): (String, &[String]) = match first {
            "crate" => (RustModuleMap::crate_of(&module).to_string(), &segs[1..]),
            "self" => (module.clone(), &segs[1..]),
            "super" => {
                let mut m = parent_module(&module)?;
                let mut rest = &segs[1..];
                while rest.first().is_some_and(|s| s == "super") {
                    m = parent_module(&m)?;
                    rest = &rest[1..];
                }
                (m, rest)
            }
            _ => {
                if let Some(imp) = imports
                    && let Some(bound) = imp.bindings.get(first)
                {
                    let mut full = bound.clone();
                    full.extend(segs[1..].iter().cloned());
                    return self.resolve_rust_path(from_file, &full, None, depth + 1);
                }
                let child = format!("{module}::{first}");
                if self.rust.is_module(&child) {
                    (child, &segs[1..])
                } else if self.rust.crate_root(first).is_some() {
                    (first.to_string(), &segs[1..])
                } else {
                    return None;
                }
            }
        };
        let mut base = base;
        let mut i = 0;
        while i < rest.len() {
            let cand = format!("{base}::{}", rest[i]);
            if self.rust.is_module(&cand) {
                base = cand;
                i += 1;
            } else {
                break;
            }
        }
        let file = self.rust.file_of(&base)?.to_string();
        match rest.len() - i {
            0 => Some(RustRes::Module(base)),
            1 => self.rust_lookup_item(&file, &rest[i], depth),
            _ => {
                let ty = &rest[i];
                match self.rust_lookup_item(&file, ty, depth)? {
                    RustRes::Item { file: tfile, name: canonical } => Some(RustRes::Member {
                        ty: canonical,
                        file: tfile,
                        name: rest[i + 1].clone(),
                    }),
                    other => Some(other),
                }
            }
        }
    }

    /// Item `name` as seen from module file `file`: defined there, or
    /// reached through one of its `pub use` re-exports.
    fn rust_lookup_item(&self, file: &str, name: &str, depth: usize) -> Option<RustRes> {
        if self.defines_item(file, name) {
            return Some(RustRes::Item {
                file: file.to_string(),
                name: name.to_string(),
            });
        }
        if depth >= MAX_REEXPORT_HOPS {
            return None;
        }
        let rows = self.reexports.get(file)?;
        for (n, target, glob) in rows {
            if !*glob && n == name {
                let segs: Vec<String> = target.split("::").map(str::to_string).collect();
                if let Some(res) = self.resolve_rust_path(file, &segs, None, depth + 1) {
                    return Some(match res {
                        RustRes::Module(m) => RustRes::Module(m),
                        other => other,
                    });
                }
            }
        }
        for (_, target, glob) in rows {
            if *glob {
                let segs: Vec<String> = target.split("::").map(str::to_string).collect();
                if let Some(RustRes::Module(m)) = self.resolve_rust_path(file, &segs, None, depth + 1)
                    && let Some(mfile) = self.rust.file_of(&m)
                    && let Some(hit) = self.rust_lookup_item(mfile, name, depth + 1)
                {
                    return Some(hit);
                }
            }
        }
        None
    }

    fn res_target(&self, res: &RustRes) -> Option<(String, Option<String>)> {
        match res {
            RustRes::Module(m) => self.rust.file_of(m).map(|f| (f.to_string(), None)),
            RustRes::Item { file, name } => Some((file.clone(), Some(name.clone()))),
            RustRes::Member { file, .. } => Some((file.clone(), None)),
        }
    }

    /// Files implementing method `name` of type `ty` (defined in `tfile`):
    /// the type's own file when its `impl` has the method (same-named types
    /// elsewhere are other types), else methods in `impl` blocks of the type
    /// within the type's crate (any crate if none there), else a same-named
    /// item in the type's file.
    fn member_targets(&self, ty: &str, tfile: &str, name: &str) -> BTreeSet<(String, Option<String>)> {
        let mut out = BTreeSet::new();
        if let Some(files) = self.owner_methods.get(&(ty.to_string(), name.to_string())) {
            if files.contains(tfile) {
                out.insert((tfile.to_string(), Some(name.to_string())));
                return out;
            }
            let crate_of = |f: &str| self.rust.module_of(f).map(RustModuleMap::crate_of);
            let home = crate_of(tfile);
            let same: Vec<&String> = files.iter().filter(|f| crate_of(f) == home).collect();
            let pick: Vec<&String> = if same.is_empty() { files.iter().collect() } else { same };
            out.extend(pick.into_iter().map(|f| (f.clone(), Some(name.to_string()))));
        }
        if let Some(entries) = self.trait_methods.get(name) {
            for (tr, f) in entries {
                if tr == ty {
                    out.insert((f.clone(), Some(name.to_string())));
                }
            }
        }
        if out.is_empty() && self.defines(tfile, name) {
            out.insert((tfile.to_string(), Some(name.to_string())));
        }
        out
    }

    /// A type named by a bare identifier in `file`: imported, defined in
    /// the file, glob-imported, or unique in the workspace.
    fn rust_bare_type(&self, file: &str, imports: &RustImports, ty: &str) -> Option<String> {
        self.rust_bare_type_identity(file, imports, ty).map(|(file, _)| file)
    }

    /// Preserve the definition's name as well as its file across renamed imports.
    fn rust_bare_type_identity(&self, file: &str, imports: &RustImports, ty: &str) -> Option<(String, String)> {
        if let Some(bound) = imports.bindings.get(ty) {
            // An explicit binding owns this name even when its target is
            // external or missing from the graph. Do not substitute a
            // same-named workspace definition via the fallback below.
            return match self.resolve_rust_path(file, bound, None, 1) {
                Some(RustRes::Item { file, name }) => Some((file, name)),
                _ => None,
            };
        }
        if self.defines(file, ty) {
            return Some((file.to_string(), ty.to_string()));
        }
        for g in &imports.globs {
            if let Some(RustRes::Module(m)) = self.resolve_rust_path(file, g, None, 1)
                && let Some(mfile) = self.rust.file_of(&m)
                && let Some(RustRes::Item { file, name }) = self.rust_lookup_item(mfile, ty, 1)
            {
                return Some((file, name));
            }
        }
        self.unique_definition(ty).map(|file| (file.to_string(), ty.to_string()))
    }

    fn resolve_rust(&self, file: &str, scope: &FileScope, r: &RawReference) -> Outcome {
        let imports = &scope.rust;
        match r.kind {
            RefKind::Import => {
                let res = self.resolve_rust_path(file, &r.path, None, 0);
                match res.as_ref().and_then(|res| self.res_target(res)) {
                    Some(t) => Outcome::Files {
                        step: "rust_import",
                        targets: [t].into(),
                    },
                    None => Outcome::Unresolved,
                }
            }
            RefKind::Implements => {
                let target = if r.path.len() > 1 {
                    self.resolve_rust_path(file, &r.path, Some(imports), 0)
                        .and_then(|res| self.res_target(&res))
                } else {
                    self.rust_bare_type(file, imports, &r.target_name)
                        .map(|f| (f, Some(r.target_name.clone())))
                };
                match target {
                    Some(t) => Outcome::Files {
                        step: "rust_implements",
                        targets: [t].into(),
                    },
                    None => Outcome::Unresolved,
                }
            }
            RefKind::Call => match r.style {
                CallStyle::Scoped => self.rust_scoped_call(file, imports, r),
                CallStyle::Bare => self.rust_bare_call(file, imports, &r.target_name),
                CallStyle::Method => {
                    // A receiver whose type is known decides; only an
                    // unknown one falls back to the name-based steps.
                    if let Some(e) = &r.receiver_expr {
                        let ty = self.eval_receiver(file, e);
                        if let Some(out) = self.typed_method(file, &ty, &r.target_name) {
                            return out;
                        }
                    }
                    self.rust_method_call(file, imports, &r.target_name)
                }
            },
        }
    }

    fn rust_scoped_call(&self, file: &str, imports: &RustImports, r: &RawReference) -> Outcome {
        match self.resolve_rust_path(file, &r.path, Some(imports), 0) {
            Some(RustRes::Item { file: f, name }) => Outcome::Files {
                step: "rust_scoped_path",
                targets: [(f, Some(name))].into(),
            },
            Some(RustRes::Member { ty, file: tfile, name }) => {
                files_of("rust_type_member", self.member_targets(&ty, &tfile, &name))
                    .unwrap_or(Outcome::Unresolved)
            }
            Some(RustRes::Module(_)) => Outcome::Unresolved,
            None if r.path.len() == 2 => {
                let (ty, name) = (&r.path[0], &r.path[1]);
                if ty == "Self" {
                    return Outcome::Local;
                }
                match self.rust_bare_type(file, imports, ty) {
                    Some(tfile) => files_of("rust_type_member", self.member_targets(ty, &tfile, name))
                        .unwrap_or(Outcome::Unresolved),
                    None => Outcome::Unresolved,
                }
            }
            None => Outcome::Unresolved,
        }
    }

    fn rust_bare_call(&self, file: &str, imports: &RustImports, name: &str) -> Outcome {
        if let Some(bound) = imports.bindings.get(name) {
            return match self.resolve_rust_path(file, bound, None, 1) {
                Some(RustRes::Item { file: f, name }) => Outcome::Files {
                    step: "rust_import_binding",
                    targets: [(f, Some(name))].into(),
                },
                _ => Outcome::Unresolved,
            };
        }
        // A local item shadows glob imports.
        if self.defines(file, name) {
            return Outcome::Local;
        }
        let mut glob_hits = BTreeSet::new();
        for g in &imports.globs {
            if let Some(RustRes::Module(m)) = self.resolve_rust_path(file, g, None, 1)
                && let Some(mfile) = self.rust.file_of(&m)
                && let Some(RustRes::Item { file: f, name: n }) = self.rust_lookup_item(mfile, name, 1)
            {
                glob_hits.insert((f, Some(n)));
            }
        }
        if let Some(out) = files_of("rust_glob", glob_hits) {
            return out;
        }
        // `foo()` cannot call a method.
        self.local_or_global(file, name, DefFilter::Free)
    }

    fn rust_method_call(&self, file: &str, imports: &RustImports, name: &str) -> Outcome {
        let my_crate = self.rust.module_of(file).map(RustModuleMap::crate_of);
        // 1. A method declared by a trait in scope: the trait's file stands
        //    for every implementation (dynamic dispatch).
        if let Some(entries) = self.trait_methods.get(name) {
            let in_scope: BTreeSet<_> = entries
                .iter()
                .filter(|(tr, tfile)| {
                    tfile == file
                        || imports.bindings.contains_key(tr)
                        || self.rust.module_of(tfile).map(RustModuleMap::crate_of) == my_crate
                })
                .map(|(_, tfile)| (tfile.clone(), Some(name.to_string())))
                .collect();
            if let Some(out) = files_of("rust_trait_method", in_scope) {
                return out;
            }
        }
        // 2. A method of a type in scope (imported or defined here).
        let mut types: BTreeSet<String> = imports.bindings.keys().cloned().collect();
        if let Some(defs) = self.defs_by_file.get(file) {
            for (n, ds) in defs {
                if ds.iter().any(|d| matches!(d.kind.as_str(), "Struct" | "Enum" | "Trait")) {
                    types.insert(n.clone());
                }
            }
        }
        let mut hits = BTreeSet::new();
        for ty in &types {
            if let Some(files) = self.owner_methods.get(&(ty.clone(), name.to_string())) {
                hits.extend(files.iter().map(|f| (f.clone(), Some(name.to_string()))));
            }
        }
        if let Some(out) = files_of("rust_type_in_scope", hits) {
            return out;
        }
        // 3. Defined in this file (a method of a local type) / by name;
        //    `x.foo()` cannot call a free function.
        self.local_or_global(file, name, DefFilter::Owned)
    }
}

// ── Measurement helpers (labelled samples) ───────────────────────────

/// Seeded FNV-1a over `parts`, the ordering key of the labelled samples
/// (`examples/edge_resolution_sample.rs`). Stable across platforms and
/// runs, so a seed names one sample.
pub fn sample_key(seed: u64, parts: &[&str]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for p in parts {
        for b in p.bytes().chain(std::iter::once(0xff)) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// The `n` items with the smallest keys: a uniform draw without
/// replacement, deterministic for given keys. Ties keep input order.
pub fn take_by_key<T>(mut items: Vec<(u64, T)>, n: usize) -> Vec<T> {
    items.sort_by_key(|(k, _)| *k);
    items.into_iter().take(n).map(|(_, t)| t).collect()
}

// ── Edge insertion ───────────────────────────────────────────────────

/// Resolve every reference of `file` and insert its edges. Returns the
/// number of edges inserted.
pub fn resolve_and_insert(
    store: &GraphStore,
    ctx: &ResolveContext,
    file: &str,
    lang: Lang,
    refs: &[RawReference],
    stats: &mut ResolveStats,
) -> anyhow::Result<usize> {
    let scope = FileScope::build(lang, refs);
    let source_is_test = is_test_file(file);
    let mut edges = 0;
    for r in refs {
        let outcome = ctx.resolve(file, lang, &scope, r);
        stats.record(&outcome, r);
        let Outcome::Files { targets, .. } = outcome else {
            continue;
        };
        let kind = match r.kind {
            RefKind::Import => EdgeKind::Imports,
            RefKind::Call => EdgeKind::Calls,
            RefKind::Implements => EdgeKind::Implements,
        };
        // `import a.b` has no item name; the module stands in for it.
        let name = match (&r.module, r.target_name.is_empty()) {
            (Some(m), true) => m.as_str(),
            _ => r.target_name.as_str(),
        };
        let source_qn = format!("{file}::{name}");
        for (target_file, name) in targets {
            let target_qn = match &name {
                Some(n) if ctx.defines(&target_file, n) => ctx.defs_by_file[&target_file][n][0]
                    .qualified_name
                    .clone(),
                _ => target_file.clone(),
            };
            store.insert_edge(kind, &source_qn, &target_qn, file, r.line as i64)?;
            edges += 1;
            if source_is_test && !is_test_file(&target_file) {
                store.insert_edge(EdgeKind::TestOf, &source_qn, &target_qn, file, r.line as i64)?;
                edges += 1;
            }
        }
    }
    Ok(edges)
}

/// Re-export rows of a file for the store.
pub fn reexports_of(lang: Lang, refs: &[RawReference]) -> Vec<(String, String, bool)> {
    match lang {
        Lang::Rust => rust_reexports(refs),
        Lang::TypeScript => js_reexports(refs),
        Lang::Python => py_reexports(refs),
    }
}

// ── TypeScript (E5) ──────────────────────────────────────────────────

/// A TypeScript file's import bindings.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct JsImports {
    /// Local name → (module specifier, imported name; `default` / `*`).
    pub bindings: HashMap<String, (String, String)>,
}

impl JsImports {
    pub fn from_refs(refs: &[RawReference]) -> Self {
        let mut out = Self::default();
        for r in refs.iter().filter(|r| r.kind == RefKind::Import && !r.public) {
            let Some(module) = &r.module else { continue };
            // A side-effect import (`import './x'`) binds nothing.
            if r.target_name.is_empty() || (r.alias.is_none() && &r.target_name == module) {
                continue;
            }
            let local = r.alias.clone().unwrap_or_else(|| r.target_name.clone());
            out.bindings
                .insert(local, (module.clone(), r.target_name.clone()));
        }
        out
    }
}

fn js_reexports(refs: &[RawReference]) -> Vec<(String, String, bool)> {
    refs.iter()
        .filter(|r| r.kind == RefKind::Import && r.public)
        .filter_map(|r| {
            let module = r.module.clone()?;
            Some(if r.glob {
                ("*".to_string(), module, true)
            } else {
                (
                    r.alias.clone().unwrap_or_else(|| r.target_name.clone()),
                    format!("{module}#{}", r.target_name),
                    false,
                )
            })
        })
        .collect()
}

impl ResolveContext {
    /// The file a relative specifier names from `from_file`, trying the
    /// usual extensions and `index` files. Package specifiers → `None`.
    fn js_module_file(&self, from_file: &str, spec: &str) -> Option<String> {
        if !(spec.starts_with("./") || spec.starts_with("../")) {
            return None;
        }
        let dir = from_file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let joined = normalize_path(&if dir.is_empty() {
            spec.to_string()
        } else {
            format!("{dir}/{spec}")
        })?;
        let mut candidates = vec![joined.clone()];
        for ext in ["ts", "tsx", "js", "jsx"] {
            candidates.push(format!("{joined}.{ext}"));
        }
        for ext in ["ts", "tsx", "js", "jsx"] {
            candidates.push(format!("{joined}/index.{ext}"));
        }
        candidates.into_iter().find(|c| self.files.contains(c))
    }

    /// Where `name` (or `default`) exported by module file `file` is
    /// defined: in the file, or through its `export … from` re-exports.
    fn js_lookup(&self, file: &str, name: &str, depth: usize) -> Option<String> {
        if name == "*" || self.defines(file, name) {
            return Some(file.to_string());
        }
        if depth < MAX_REEXPORT_HOPS {
            for (n, target, glob) in self.reexports.get(file).into_iter().flatten() {
                if *glob {
                    // `export * from` never re-exports a module's default.
                    if name != "default"
                        && let Some(mfile) = self.js_module_file(file, target)
                        && let Some(hit) = self.js_lookup(&mfile, name, depth + 1)
                    {
                        return Some(hit);
                    }
                } else if n == name
                    && let Some((spec, orig)) = target.split_once('#')
                    && let Some(mfile) = self.js_module_file(file, spec)
                {
                    return self.js_lookup(&mfile, orig, depth + 1);
                }
            }
        }
        // A default export that is not re-exported from elsewhere is the
        // file's own (default exports are not tracked as named symbols).
        (name == "default").then(|| file.to_string())
    }

    fn resolve_ts(&self, file: &str, scope: &FileScope, r: &RawReference) -> Outcome {
        let imports = &scope.js;
        match r.kind {
            RefKind::Import => match r.module.as_deref().and_then(|m| self.js_module_file(file, m)) {
                Some(f) => Outcome::Files {
                    step: "js_import",
                    targets: [(f, None)].into(),
                },
                None => Outcome::Unresolved,
            },
            RefKind::Call | RefKind::Implements => {
                // `ns.f()` through a namespace import / `X.f()` on an import.
                if let Some(recv) = &r.receiver
                    && let Some((spec, imported)) = imports.bindings.get(recv)
                {
                    let Some(mfile) = self.js_module_file(file, spec) else {
                        return Outcome::Unresolved;
                    };
                    let looked = if imported == "*" {
                        self.js_lookup(&mfile, &r.target_name, 0)
                    } else {
                        self.js_lookup(&mfile, imported, 0)
                    };
                    return match looked {
                        Some(f) => Outcome::Files {
                            step: "js_receiver_import",
                            targets: [(f, Some(r.target_name.clone()))].into(),
                        },
                        None => Outcome::Unresolved,
                    };
                }
                if r.style != CallStyle::Method
                    && let Some((spec, imported)) = imports.bindings.get(&r.target_name)
                {
                    return match self
                        .js_module_file(file, spec)
                        .and_then(|m| self.js_lookup(&m, imported, 0))
                    {
                        Some(f) => Outcome::Files {
                            step: "js_import_binding",
                            targets: [(f, Some(imported.clone()))].into(),
                        },
                        None => Outcome::Unresolved,
                    };
                }
                self.local_or_global(file, &r.target_name, DefFilter::Any)
            }
        }
    }

    /// Shared last steps: defined in this file → local; else every file
    /// with a matching definition of the name (the cap in [`finish`] keeps
    /// this to a handful).
    fn local_or_global(&self, file: &str, name: &str, filter: DefFilter) -> Outcome {
        if self.defines(file, name) {
            return Outcome::Local;
        }
        let defs = self.defs_by_name.get(name).map(Vec::as_slice).unwrap_or_default();
        let lang = lang_of(file);
        let found = files_of(
            "global_name",
            defs.iter()
                .filter(|d| lang_of(&d.file_path) == lang)
                .filter(|d| match filter {
                    DefFilter::Any => true,
                    DefFilter::Free => d.owner.is_none(),
                    DefFilter::Owned => d.owner.is_some(),
                })
                .map(|d| (d.file_path.clone(), Some(name.to_string()))),
        );
        // A `.foo()` whose receiver type is unknown and whose name std also
        // defines is far more often the std method than the workspace's
        // `foo` (receiver-typing plan R1: `ok`, `send`, `elapsed`, …).
        match found {
            Some(_) if matches!(filter, DefFilter::Owned) && is_std_method_name(name) => {
                Outcome::NoEdge { reason: "std_name_unknown_receiver" }
            }
            Some(out) => out,
            None => Outcome::Unresolved,
        }
    }
}

/// Method names defined by std / core / alloc, generated from the
/// toolchain's rustdoc HTML by `research/gen_std_methods.py` (header lines
/// start with `#`).
static STD_METHOD_NAMES: std::sync::LazyLock<HashSet<&'static str>> = std::sync::LazyLock::new(|| {
    include_str!("std_methods.txt")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
});

/// Whether std (or a listed dependency) defines a method of this name.
pub fn is_std_method_name(name: &str) -> bool {
    STD_METHOD_NAMES.contains(name)
}

/// Which definitions a by-name fallback may land on. Rust knows owners;
/// TypeScript / Python symbols carry none, so they use `Any`.
#[derive(Debug, Clone, Copy)]
enum DefFilter {
    Any,
    /// Not inside an `impl` / `trait` (a bare `foo()` call).
    Free,
    /// Inside an `impl` / `trait` (a `.foo()` call).
    Owned,
}

/// Collapse `.` / `..` in a `/`-separated path; `None` if it escapes.
fn normalize_path(p: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            s => out.push(s),
        }
    }
    Some(out.join("/"))
}

// ── Python (E5) ──────────────────────────────────────────────────────

/// Python module name ↔ file.
///
/// A file's module name starts at its topmost enclosing package (a chain
/// of directories with `__init__.py`): `src/pkg/core.py` with
/// `src/pkg/__init__.py` (but no `src/__init__.py`) is `pkg.core`; a
/// script outside any package is its stem (`app`).
#[derive(Debug, Default, Clone)]
pub struct PyModuleMap {
    module_file: HashMap<String, String>,
    file_module: HashMap<String, String>,
}

impl PyModuleMap {
    pub fn build(files: &[String]) -> Self {
        let set: HashSet<&str> = files.iter().map(String::as_str).collect();
        let is_pkg = |dir: &str| {
            let init = if dir.is_empty() {
                "__init__.py".to_string()
            } else {
                format!("{dir}/__init__.py")
            };
            !dir.is_empty() && set.contains(init.as_str())
        };
        let mut map = Self::default();
        for f in files.iter().filter(|f| f.ends_with(".py") || f.ends_with(".pyi")) {
            let (dir, stem) = match f.rsplit_once('/') {
                Some((d, s)) => (d.to_string(), s),
                None => (String::new(), f.as_str()),
            };
            let stem = stem.trim_end_matches(".pyi").trim_end_matches(".py");
            let mut parts: Vec<String> = Vec::new();
            let mut d = dir.clone();
            while is_pkg(&d) {
                let (up, name) = match d.rsplit_once('/') {
                    Some((u, n)) => (u.to_string(), n.to_string()),
                    None => (String::new(), d.clone()),
                };
                parts.push(name);
                d = up;
            }
            parts.reverse();
            if stem != "__init__" {
                parts.push(stem.to_string());
            }
            if parts.is_empty() {
                continue;
            }
            let module = parts.join(".");
            map.module_file.entry(module.clone()).or_insert_with(|| f.clone());
            map.file_module.insert(f.clone(), module);
        }
        map
    }

    pub fn file_of(&self, module: &str) -> Option<&str> {
        self.module_file.get(module).map(String::as_str)
    }

    /// The package a module's relative imports start from: the module
    /// itself for an `__init__.py`, else its parent.
    fn package_of(&self, file: &str) -> Option<String> {
        let module = self.file_module.get(file)?;
        if file.ends_with("__init__.py") {
            Some(module.clone())
        } else {
            Some(module.rsplit_once('.').map(|(p, _)| p.to_string()).unwrap_or_default())
        }
    }
}

/// A Python file's import bindings.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PyImports {
    /// Local name → (module as written, imported name or `None` for a
    /// module binding).
    pub bindings: HashMap<String, (String, Option<String>)>,
    /// Modules imported with `from m import *`.
    pub globs: Vec<String>,
}

impl PyImports {
    pub fn from_refs(refs: &[RawReference]) -> Self {
        let mut out = Self::default();
        for r in refs.iter().filter(|r| r.kind == RefKind::Import) {
            let Some(module) = &r.module else { continue };
            if r.glob {
                out.globs.push(module.clone());
            } else if r.target_name.is_empty() {
                // `import a.b` binds `a`; `import a.b as x` binds `x` → a.b.
                let local = r
                    .alias
                    .clone()
                    .unwrap_or_else(|| module.split('.').next().unwrap_or(module).to_string());
                let bound = if r.alias.is_some() {
                    module.clone()
                } else {
                    local.clone()
                };
                out.bindings.insert(local, (bound, None));
            } else {
                let local = r.alias.clone().unwrap_or_else(|| r.target_name.clone());
                out.bindings
                    .insert(local, (module.clone(), Some(r.target_name.clone())));
            }
        }
        out
    }
}

fn py_reexports(refs: &[RawReference]) -> Vec<(String, String, bool)> {
    // Every name a module imports with `from m import x` is an attribute of
    // that module, so other modules can import it from there.
    refs.iter()
        .filter(|r| r.kind == RefKind::Import && !r.target_name.is_empty())
        .filter_map(|r| {
            let module = r.module.clone()?;
            Some(if r.glob {
                ("*".to_string(), module, true)
            } else {
                (
                    r.alias.clone().unwrap_or_else(|| r.target_name.clone()),
                    format!("{module}#{}", r.target_name),
                    false,
                )
            })
        })
        .collect()
}

impl ResolveContext {
    /// Absolute module name for `module` written in `from_file`
    /// (handles leading dots of relative imports).
    fn py_absolute(&self, from_file: &str, module: &str) -> Option<String> {
        let dots = module.chars().take_while(|c| *c == '.').count();
        if dots == 0 {
            return Some(module.to_string());
        }
        let mut pkg = self.py.package_of(from_file)?;
        for _ in 1..dots {
            pkg = pkg.rsplit_once('.').map(|(p, _)| p.to_string()).unwrap_or_default();
        }
        let rest = &module[dots..];
        Some(match (pkg.is_empty(), rest.is_empty()) {
            (true, _) => rest.to_string(),
            (false, true) => pkg,
            (false, false) => format!("{pkg}.{rest}"),
        })
    }

    fn py_module_file(&self, from_file: &str, module: &str) -> Option<String> {
        let abs = self.py_absolute(from_file, module)?;
        self.py.file_of(&abs).map(str::to_string)
    }

    /// Where attribute `name` of module file `file` is defined: in the
    /// file, a submodule, or through its from-imports.
    fn py_lookup(&self, file: &str, name: &str, depth: usize) -> Option<String> {
        if self.defines(file, name) {
            return Some(file.to_string());
        }
        if let Some(module) = self.py.file_module.get(file) {
            let sub = format!("{module}.{name}");
            if let Some(f) = self.py.file_of(&sub) {
                return Some(f.to_string());
            }
        }
        if depth >= MAX_REEXPORT_HOPS {
            return None;
        }
        for (n, target, glob) in self.reexports.get(file)? {
            if *glob {
                if let Some(mfile) = self.py_module_file(file, target)
                    && let Some(hit) = self.py_lookup(&mfile, name, depth + 1)
                {
                    return Some(hit);
                }
            } else if n == name
                && let Some((m, orig)) = target.split_once('#')
                && let Some(mfile) = self.py_module_file(file, m)
            {
                return self.py_lookup(&mfile, orig, depth + 1);
            }
        }
        None
    }

    fn resolve_py(&self, file: &str, scope: &FileScope, r: &RawReference) -> Outcome {
        let imports = &scope.py;
        match r.kind {
            RefKind::Import => {
                let Some(module) = &r.module else {
                    return Outcome::Unresolved;
                };
                // `from pkg import core` names the submodule when there is one.
                if !r.glob && !r.target_name.is_empty() {
                    let sub = if module.ends_with('.') {
                        format!("{module}{}", r.target_name)
                    } else {
                        format!("{module}.{}", r.target_name)
                    };
                    if let Some(f) = self.py_module_file(file, &sub) {
                        return Outcome::Files {
                            step: "py_import",
                            targets: [(f, None)].into(),
                        };
                    }
                }
                match self.py_module_file(file, module) {
                    Some(f) => Outcome::Files {
                        step: "py_import",
                        targets: [(f, None)].into(),
                    },
                    None => Outcome::Unresolved,
                }
            }
            RefKind::Call | RefKind::Implements => {
                if let Some(recv) = &r.receiver {
                    // `pkg.core.compute()` / `alias.f()`: resolve the receiver
                    // as a module through the bindings.
                    let first = recv.split('.').next().unwrap_or(recv);
                    if let Some((bound, None)) = imports.bindings.get(first) {
                        let tail = &recv[first.len()..];
                        let module = format!("{bound}{tail}");
                        return match self
                            .py_module_file(file, &module)
                            .and_then(|m| self.py_lookup(&m, &r.target_name, 0))
                        {
                            Some(f) => Outcome::Files {
                                step: "py_module_attribute",
                                targets: [(f, Some(r.target_name.clone()))].into(),
                            },
                            None => Outcome::Unresolved,
                        };
                    }
                    if let Some((m, Some(orig))) = imports.bindings.get(first)
                        && let Some(f) = self
                            .py_module_file(file, m)
                            .and_then(|mf| self.py_lookup(&mf, orig, 0))
                    {
                        // `Cls.method()` on an imported class.
                        return Outcome::Files {
                            step: "py_receiver_import",
                            targets: [(f, Some(r.target_name.clone()))].into(),
                        };
                    }
                    return self.local_or_global(file, &r.target_name, DefFilter::Any);
                }
                if let Some((m, Some(orig))) = imports.bindings.get(&r.target_name) {
                    return match self
                        .py_module_file(file, m)
                        .and_then(|mf| self.py_lookup(&mf, orig, 0))
                    {
                        Some(f) => Outcome::Files {
                            step: "py_import_binding",
                            targets: [(f, Some(orig.clone()))].into(),
                        },
                        None => Outcome::Unresolved,
                    };
                }
                let mut glob_hits = BTreeSet::new();
                for g in &imports.globs {
                    if let Some(f) = self
                        .py_module_file(file, g)
                        .and_then(|mf| self.py_lookup(&mf, &r.target_name, 0))
                    {
                        glob_hits.insert((f, Some(r.target_name.clone())));
                    }
                }
                if let Some(out) = files_of("py_glob", glob_hits) {
                    return out;
                }
                self.local_or_global(file, &r.target_name, DefFilter::Any)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Vec<String> {
        v.split("::").map(str::to_string).collect()
    }

    fn import(path: &str, alias: Option<&str>, glob: bool) -> RawReference {
        let segs = s(path);
        RawReference {
            target_name: segs.last().cloned().unwrap(),
            kind: RefKind::Import,
            path: segs,
            alias: alias.map(str::to_string),
            glob,
            ..Default::default()
        }
    }

    #[test]
    fn rust_import_table_binds_aliases_selfs_and_globs() {
        let refs = vec![
            import("beta_core::shapes::widget::Widget", None, false),
            import("beta_core::util", None, false),
            import("beta_core::util::helper", Some("h"), false),
            import("engine", None, true),
            import("super::widget::Widget", Some("W2"), false),
            import("std::fmt", Some("_"), false),
            import("beta_core", None, false),
        ];
        let t = RustImports::from_refs(&refs);
        assert_eq!(t.bindings["Widget"], s("beta_core::shapes::widget::Widget"));
        assert_eq!(t.bindings["util"], s("beta_core::util"));
        assert_eq!(t.bindings["h"], s("beta_core::util::helper"));
        assert_eq!(t.bindings["W2"], s("super::widget::Widget"));
        assert_eq!(t.bindings["beta_core"], s("beta_core"));
        assert!(!t.bindings.contains_key("_"));
        assert_eq!(t.globs, vec![s("engine")]);
    }

    fn fixture_files() -> (tempfile::TempDir, Vec<String>) {
        let dir = tempfile::tempdir().unwrap();
        let files = [
            "crates/alpha/src/main.rs",
            "crates/alpha/src/engine/mod.rs",
            "crates/alpha/src/engine/parts.rs",
            "crates/alpha/src/bin/tool.rs",
            "crates/alpha/tests/it.rs",
            "crates/beta-core/src/lib.rs",
            "crates/beta-core/src/util.rs",
            "crates/beta-core/src/shapes/mod.rs",
            "crates/beta-core/src/shapes/widget.rs",
            "loose/orphan.rs",
        ];
        for f in files {
            let p = dir.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "").unwrap();
        }
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        std::fs::write(
            dir.path().join("crates/alpha/Cargo.toml"),
            "[package]\nname = \"alpha\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("crates/beta-core/Cargo.toml"),
            "[package]\nname = \"beta-core\"\n[lib]\nname = \"beta_core\"\n",
        )
        .unwrap();
        (dir, files.iter().map(|f| f.to_string()).collect())
    }

    #[test]
    fn rust_module_map_follows_cargo_and_the_src_tree() {
        let (dir, files) = fixture_files();
        let m = RustModuleMap::build(dir.path(), &files);
        assert_eq!(m.module_of("crates/alpha/src/main.rs"), Some("alpha"));
        assert_eq!(m.module_of("crates/alpha/src/engine/mod.rs"), Some("alpha::engine"));
        assert_eq!(m.module_of("crates/alpha/src/engine/parts.rs"), Some("alpha::engine::parts"));
        assert_eq!(m.module_of("crates/beta-core/src/lib.rs"), Some("beta_core"));
        assert_eq!(m.module_of("crates/beta-core/src/shapes/mod.rs"), Some("beta_core::shapes"));
        assert_eq!(
            m.module_of("crates/beta-core/src/shapes/widget.rs"),
            Some("beta_core::shapes::widget")
        );
        assert_eq!(m.module_of("crates/alpha/src/bin/tool.rs"), Some("alpha#src/bin/tool.rs"));
        assert_eq!(m.module_of("crates/alpha/tests/it.rs"), Some("alpha#tests/it.rs"));
        assert_eq!(m.module_of("loose/orphan.rs"), None, "no crate, no module");
        assert_eq!(m.crate_root("beta_core"), Some("crates/beta-core/src/lib.rs"));
        assert_eq!(m.crate_root("alpha"), Some("crates/alpha/src/main.rs"));
        assert_eq!(RustModuleMap::crate_of("alpha#tests/it.rs"), "alpha");
        assert_eq!(m.file_of("alpha::engine::parts"), Some("crates/alpha/src/engine/parts.rs"));
    }

    fn ctx_with(
        files: &[String],
        root: &Path,
        defs: &[(&str, &str, &str, Option<&str>)],
        reexports: &[(&str, &str, &str, bool)],
    ) -> ResolveContext {
        let store = GraphStore::open_memory().unwrap();
        for f in files {
            store
                .upsert_node(super::super::store::NodeKind::File, f, f, f, None, None, None, None)
                .unwrap();
        }
        for (file, name, kind, owner) in defs {
            let qn = format!("{file}::{name}");
            let k = super::super::store::NodeKind::parse(kind).unwrap();
            store
                .upsert_node(k, name, &qn, file, Some(0), None, None, None)
                .unwrap();
            store.set_node_owner(&qn, *owner).unwrap();
        }
        let mut by_file: HashMap<&str, Vec<(String, String, bool)>> = HashMap::new();
        for (file, name, target, glob) in reexports {
            by_file
                .entry(file)
                .or_default()
                .push((name.to_string(), target.to_string(), *glob));
        }
        for (file, rows) in by_file {
            store.replace_reexports(file, &rows).unwrap();
        }
        ResolveContext::build(&store, root).unwrap()
    }

    #[test]
    fn rust_paths_resolve_crate_self_super_children_and_reexports() {
        let (dir, files) = fixture_files();
        let ctx = ctx_with(
            &files,
            dir.path(),
            &[
                ("crates/beta-core/src/shapes/widget.rs", "Widget", "Struct", None),
                ("crates/beta-core/src/util.rs", "helper", "Function", None),
                ("crates/alpha/src/engine/parts.rs", "Part", "Struct", None),
            ],
            &[
                ("crates/beta-core/src/lib.rs", "Widget", "shapes::widget::Widget", false),
                ("crates/beta-core/src/lib.rs", "Loop", "self::Loop", false),
            ],
        );
        let main = "crates/alpha/src/main.rs";
        let parts = "crates/alpha/src/engine/parts.rs";
        let item = |file: &str, name: &str| {
            Some(RustRes::Item {
                file: file.to_string(),
                name: name.to_string(),
            })
        };
        assert_eq!(
            ctx.resolve_rust_path(main, &s("beta_core::util::helper"), None, 0),
            item("crates/beta-core/src/util.rs", "helper"),
            "external crate name"
        );
        assert_eq!(
            ctx.resolve_rust_path(main, &s("beta_core::Widget"), None, 0),
            item("crates/beta-core/src/shapes/widget.rs", "Widget"),
            "pub use re-export followed"
        );
        assert_eq!(
            ctx.resolve_rust_path(main, &s("engine::parts::Part"), None, 0),
            item(parts, "Part"),
            "child module from the crate root"
        );
        assert_eq!(
            ctx.resolve_rust_path(parts, &s("crate::engine::parts::Part"), None, 0),
            item(parts, "Part")
        );
        assert_eq!(
            ctx.resolve_rust_path(parts, &s("super::parts::Part"), None, 0),
            item(parts, "Part")
        );
        assert_eq!(
            ctx.resolve_rust_path(parts, &s("self::Part"), None, 0),
            item(parts, "Part")
        );
        assert_eq!(
            ctx.resolve_rust_path(main, &s("beta_core::shapes"), None, 0),
            Some(RustRes::Module("beta_core::shapes".into()))
        );
        assert_eq!(
            ctx.resolve_rust_path(main, &s("beta_core::shapes::widget::Widget::new"), None, 0),
            Some(RustRes::Member {
                ty: "Widget".into(),
                file: "crates/beta-core/src/shapes/widget.rs".into(),
                name: "new".into()
            })
        );
        assert_eq!(ctx.resolve_rust_path(main, &s("std::cmp::max"), None, 0), None, "external");
        assert_eq!(
            ctx.resolve_rust_path(main, &s("beta_core::Loop"), None, 0),
            None,
            "a re-export cycle ends without a result"
        );
    }

    fn call(name: &str, style: CallStyle, path: &str) -> RawReference {
        RawReference {
            target_name: name.into(),
            kind: RefKind::Call,
            style,
            path: if path.is_empty() { Vec::new() } else { s(path) },
            ..Default::default()
        }
    }

    fn files(o: &Outcome) -> Vec<&str> {
        match o {
            Outcome::Files { targets, .. } => targets.iter().map(|(f, _)| f.as_str()).collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn rust_calls_resolve_by_step_and_the_cap_drops_wide_fanout() {
        let (dir, files_list) = fixture_files();
        let widget = "crates/beta-core/src/shapes/widget.rs";
        let util = "crates/beta-core/src/util.rs";
        let shapes = "crates/beta-core/src/shapes/mod.rs";
        let parts = "crates/alpha/src/engine/parts.rs";
        let main = "crates/alpha/src/main.rs";
        let ctx = ctx_with(
            &files_list,
            dir.path(),
            &[
                (widget, "Widget", "Struct", None),
                (widget, "new", "Function", Some("impl:Widget")),
                (widget, "spin", "Function", Some("impl:Widget")),
                (parts, "Part", "Struct", None),
                (parts, "new", "Function", Some("impl:Part")),
                (util, "helper", "Function", None),
                (util, "Observer", "Trait", None),
                (util, "notify", "Function", Some("trait:Observer")),
                (shapes, "only_shapes", "Function", None),
                // `common` is defined in three files: too wide.
                (widget, "common", "Function", None),
                (util, "common", "Function", None),
                (parts, "common", "Function", None),
            ],
            &[],
        );
        let refs = vec![
            import("beta_core::shapes::widget::Widget", None, false),
            import("beta_core::util::helper", Some("h"), false),
            import("beta_core::util::Observer", None, false),
            import("beta_core::shapes", None, true),
        ];
        let scope = FileScope::build(Lang::Rust, &refs);
        let r = |c: RawReference| ctx.resolve(main, Lang::Rust, &scope, &c);

        assert_eq!(files(&r(call("new", CallStyle::Scoped, "Widget::new"))), [widget], "type member via import");
        assert_eq!(files(&r(call("new", CallStyle::Scoped, "engine::parts::Part::new"))), [parts]);
        assert_eq!(files(&r(call("h", CallStyle::Bare, ""))), [util], "aliased binding");
        assert_eq!(files(&r(call("only_shapes", CallStyle::Bare, ""))), [shapes], "glob import");
        assert_eq!(files(&r(call("notify", CallStyle::Method, ""))), [util], "trait in scope");
        assert_eq!(files(&r(call("spin", CallStyle::Method, ""))), [widget], "method of an imported type");
        assert!(
            matches!(r(call("common", CallStyle::Bare, "")), Outcome::Ambiguous { ref files, .. } if files.len() == 3),
            "three files > cap"
        );
        assert_eq!(r(call("max", CallStyle::Scoped, "std::cmp::max")), Outcome::Unresolved);
        assert_eq!(r(call("nowhere", CallStyle::Bare, "")), Outcome::Unresolved);
        assert_eq!(
            ctx.resolve(parts, Lang::Rust, &FileScope::default(), &call("new", CallStyle::Scoped, "Self::new")),
            Outcome::Local
        );
        let mut stats = ResolveStats::default();
        stats.record(
            &Outcome::Ambiguous { step: "global_name", files: Vec::new() },
            &call("common", CallStyle::Bare, ""),
        );
        assert_eq!(stats.dropped_names["common"], 1);
        stats.record(
            &Outcome::Files { step: "global_name", targets: [("a.rs".to_string(), None)].into() },
            &call("spin", CallStyle::Method, ""),
        );
        stats.record(&Outcome::NoEdge { reason: "external_receiver" }, &call("lock", CallStyle::Method, ""));
        assert_eq!(stats.method_global_names["spin"], 1);
        assert_eq!(stats.no_edge["external_receiver"], 1);
    }

    #[test]
    fn name_fallbacks_respect_call_style_locals_and_test_files() {
        let (dir, mut files_list) = fixture_files();
        files_list.push("crates/alpha/tests/support.rs".into());
        files_list.push("scripts/tool.py".into());
        let app = "crates/alpha/src/engine/mod.rs";
        let parts = "crates/alpha/src/engine/parts.rs";
        let util = "crates/beta-core/src/util.rs";
        let support = "crates/alpha/tests/support.rs";
        let ctx = ctx_with(
            &files_list,
            dir.path(),
            &[
                // `regear` is a method in parts.rs and a free fn in util.rs.
                (parts, "regear", "Function", Some("impl:Part")),
                (util, "regear", "Function", None),
                (util, "shared", "Function", None),
                (app, "shared", "Function", None),
                (support, "fixture_only", "Function", None),
                // Two unrelated types named `Part`, each with `new`.
                (parts, "Part", "Struct", None),
                (parts, "new", "Function", Some("impl:Part")),
                (util, "Part", "Struct", None),
                (util, "new", "Function", Some("impl:Part")),
                ("scripts/tool.py", "py_only", "Function", None),
            ],
            &[],
        );
        assert_eq!(
            ctx.resolve(parts, Lang::Rust, &FileScope::default(), &call("new", CallStyle::Scoped, "Part::new")),
            Outcome::Local,
            "a type defined here: its own impl, not the homonym's"
        );
        assert_eq!(
            ctx.resolve("crates/alpha/src/main.rs", Lang::Rust, &FileScope::default(), &call("py_only", CallStyle::Bare, "")),
            Outcome::Unresolved,
            "the name fallback stays within the language"
        );
        let main = "crates/alpha/src/main.rs";
        let none = FileScope::default();
        let r = |file: &str, c: RawReference| ctx.resolve(file, Lang::Rust, &none, &c);
        assert_eq!(files(&r(main, call("regear", CallStyle::Bare, ""))), [util], "bare → free fn");
        assert_eq!(files(&r(main, call("regear", CallStyle::Method, ""))), [parts], "method → impl method");

        // A local item shadows a glob import of the same name.
        let glob = FileScope::build(Lang::Rust, &[import("beta_core::util", None, true)]);
        assert_eq!(ctx.resolve(app, Lang::Rust, &glob, &call("shared", CallStyle::Bare, "")), Outcome::Local);

        assert_eq!(
            r(main, call("fixture_only", CallStyle::Bare, "")),
            Outcome::Unresolved,
            "production code never targets a test file"
        );
        assert_eq!(
            files(&r("crates/alpha/tests/it.rs", call("fixture_only", CallStyle::Bare, ""))),
            [support]
        );
    }

    #[test]
    fn std_method_names_skip_the_unknown_receiver_fallback() {
        for n in ["ok", "send", "elapsed", "clamp", "lock", "take"] {
            assert!(is_std_method_name(n), "{n} should be listed");
        }
        assert!(!is_std_method_name("refresh_git_panel"));
        assert!(!is_std_method_name("# Toolchain"), "header lines are not names");

        let (dir, files_list) = fixture_files();
        let util = "crates/beta-core/src/util.rs";
        let ctx = ctx_with(
            &files_list,
            dir.path(),
            &[
                (util, "elapsed", "Function", Some("impl:Clock")),
                (util, "regear", "Function", Some("impl:Clock")),
            ],
            &[],
        );
        let main = "crates/alpha/src/main.rs";
        let none = FileScope::default();
        let r = |c: RawReference| ctx.resolve(main, Lang::Rust, &none, &c);
        assert_eq!(
            r(call("elapsed", CallStyle::Method, "")),
            Outcome::NoEdge { reason: "std_name_unknown_receiver" },
            "the workspace's only `elapsed` is not assumed"
        );
        assert_eq!(files(&r(call("regear", CallStyle::Method, ""))), [util], "a non-std name keeps the fallback");

        let free = ctx_with(&files_list, dir.path(), &[(util, "elapsed", "Function", None)], &[]);
        assert_eq!(
            files(&free.resolve(main, Lang::Rust, &none, &call("elapsed", CallStyle::Bare, ""))),
            [util],
            "a bare call is unaffected"
        );
    }

    #[test]
    fn python_module_names_start_at_the_topmost_package() {
        let files: Vec<String> = [
            "src/pkg/__init__.py",
            "src/pkg/core.py",
            "src/pkg/util/__init__.py",
            "src/pkg/util/tools.py",
            "scripts/app.py",
        ]
        .iter()
        .map(|f| f.to_string())
        .collect();
        let m = PyModuleMap::build(&files);
        assert_eq!(m.file_of("pkg"), Some("src/pkg/__init__.py"));
        assert_eq!(m.file_of("pkg.core"), Some("src/pkg/core.py"));
        assert_eq!(m.file_of("pkg.util.tools"), Some("src/pkg/util/tools.py"));
        assert_eq!(m.file_of("app"), Some("scripts/app.py"));
        assert_eq!(m.package_of("src/pkg/util/__init__.py").as_deref(), Some("pkg.util"));
        assert_eq!(m.package_of("src/pkg/util/tools.py").as_deref(), Some("pkg.util"));
    }

    #[test]
    fn sample_draws_are_deterministic_for_a_seed() {
        let items = |seed| -> Vec<(u64, usize)> {
            (0..1000usize)
                .map(|i| (sample_key(seed, &["f.rs", &i.to_string(), "m"]), i))
                .collect()
        };
        let a = take_by_key(items(20261019), 150);
        assert_eq!(a, take_by_key(items(20261019), 150), "same seed, same draw");
        assert_eq!(a.len(), 150);
        assert_ne!(a, take_by_key(items(20261021), 150), "another seed, another draw");
        let distinct: HashSet<_> = a.iter().collect();
        assert_eq!(distinct.len(), 150, "without replacement");
        assert_ne!(sample_key(0, &["a", "b"]), sample_key(0, &["ab"]), "parts are separated");
    }

    // ── Receiver typing (receiver-typing plan R4/R5) ────────────────

    /// A one-crate Cargo workspace from `(path under src/, source)` pairs,
    /// indexed into an in-memory store.
    fn crate_ws(files: &[(&str, &str)]) -> (tempfile::TempDir, GraphStore, ResolveContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        for (p, src) in files {
            let path = dir.path().join("src").join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, src).unwrap();
        }
        let store = GraphStore::open_memory().unwrap();
        super::super::graph_builder::build_graph_into(&store, dir.path(), &[]).unwrap();
        let ctx = ResolveContext::build(&store, dir.path()).unwrap();
        (dir, store, ctx)
    }

    /// The outcome of the first `.method()` call in `file` (repo-relative).
    fn method_outcome(dir: &tempfile::TempDir, ctx: &ResolveContext, file: &str, method: &str) -> Outcome {
        let src = std::fs::read_to_string(dir.path().join(file)).unwrap();
        let refs = super::super::edges::extract_references_for_extension("rs", &src);
        let scope = FileScope::build(Lang::Rust, &refs);
        let r = refs
            .iter()
            .find(|r| r.style == CallStyle::Method && r.target_name == method)
            .unwrap_or_else(|| panic!("no .{method}() in {file}"));
        ctx.resolve(file, Lang::Rust, &scope, r)
    }

    /// The type a `let probe: <ty>` written in `file` resolves to.
    fn probe_type(ctx: &ResolveContext, file: &str, ty: &str) -> receiver::Ty {
        ctx.resolve_type(file, &super::super::types::TypeRef::parse(ty), None, 0)
    }

    #[test]
    fn renamed_imports_preserve_receiver_type_identity() {
        let (dir, _store, ctx) = crate_ws(&[
            ("lib.rs", "mod a; mod b; mod barrel; mod caller; mod glob;"),
            ("a.rs", "pub struct Widget(pub u8); impl Widget { pub fn frobnicate(&self) {} pub fn new() -> Self { Self(0) } } pub trait Shape { fn area(&self); }"),
            ("b.rs", "pub struct Other; impl Other { pub fn frobnicate(&self) {} }"),
            ("barrel.rs", "pub use crate::a::Widget as Renamed; pub use crate::a::Shape as RenamedTrait;"),
            ("caller.rs", "use crate::a::Widget as W; use crate::barrel::Renamed as R; use crate::barrel::RenamedTrait as T; fn run(x: W) { x.frobnicate(); }"),
            ("glob.rs", "use crate::barrel::*; fn run(x: Renamed) { x.frobnicate(); }"),
        ]);
        let expected = receiver::Ty::Workspace { name: "Widget".into(), file: "src/a.rs".into() };
        for ty in ["W", "R", "crate::barrel::Renamed"] {
            assert_eq!(probe_type(&ctx, "src/caller.rs", ty), expected, "{ty}");
        }
        assert_eq!(probe_type(&ctx, "src/glob.rs", "Renamed"), expected);
        for ty in ["dyn T", "impl T", "dyn crate::barrel::RenamedTrait"] {
            assert_eq!(probe_type(&ctx, "src/caller.rs", ty), receiver::Ty::Traits(vec![("Shape".into(), "src/a.rs".into())]));
        }
        for path in ["W", "R", "W::new", "crate::barrel::Renamed::new"] {
            let expr = super::super::types::ReceiverExpr {
                root: super::super::types::Root::CallPath(path.split("::").map(str::to_string).collect()),
                steps: Vec::new(),
                origin: super::super::types::Origin::Direct,
            };
            assert_eq!(ctx.eval_receiver("src/caller.rs", &expr), expected, "{path}");
        }
        for file in ["src/caller.rs", "src/glob.rs"] {
            assert_eq!(method_outcome(&dir, &ctx, file, "frobnicate"), Outcome::Files {
                step: "rust_receiver_inherent",
                targets: [("src/a.rs".into(), Some("frobnicate".into()))].into(),
            });
        }
    }

    #[test]
    fn types_resolve_with_the_imports_of_the_file_that_wrote_them() {
        let (dir, _store, ctx) = crate_ws(&[
            ("lib.rs", "pub mod git; pub mod app; pub mod caller; pub mod types; pub mod shadow;\n"),
            ("git.rs", "pub struct GitPanel; impl GitPanel { pub fn refresh(&self) {} }\n"),
            (
                "app.rs",
                "use crate::git::GitPanel;\npub struct App { pub git_panel: GitPanel }\n\
                 impl App { pub fn me(&self) -> Self { todo!() } pub fn panel(&self) -> &GitPanel { &self.git_panel } }\n",
            ),
            // The caller does not import GitPanel: the field's type is
            // resolved with app.rs's imports.
            (
                "caller.rs",
                "use crate::app::App;\npub fn f(app: &App) { app.git_panel.refresh(); app.me().panel().refresh(); }\n",
            ),
            (
                "types.rs",
                "use tokio::sync::mpsc::Sender;\npub type Res<T> = std::result::Result<T, Error>;\n\
                 pub type Loop = Loop;\npub type Panel = crate::git::GitPanel;\npub type P2 = Panel;\n",
            ),
            ("shadow.rs", "pub struct Result; impl Result { pub fn mine(&self) {} }\n"),
        ]);
        let caller = "src/caller.rs";
        assert_eq!(files(&method_outcome(&dir, &ctx, caller, "refresh")), ["src/git.rs"]);
        let types = "src/types.rs";
        assert!(matches!(probe_type(&ctx, types, "Sender<u8>"), receiver::Ty::External { ref name, .. } if name == "Sender"));
        assert!(matches!(probe_type(&ctx, types, "Option<u8>"), receiver::Ty::External { ref name, .. } if name == "Option"));
        assert!(matches!(probe_type(&ctx, types, "Res<u8>"), receiver::Ty::External { ref name, .. } if name == "Result"), "alias expanded");
        assert!(matches!(probe_type(&ctx, types, "P2"), receiver::Ty::Workspace { ref name, .. } if name == "GitPanel"), "alias chain");
        assert_eq!(probe_type(&ctx, types, "Loop"), receiver::Ty::Unknown, "an alias cycle ends");
        assert!(
            matches!(probe_type(&ctx, "src/shadow.rs", "Result"), receiver::Ty::Workspace { ref name, .. } if name == "Result"),
            "a local type shadows the std name"
        );
        assert!(matches!(probe_type(&ctx, types, "Box<Arc<crate::git::GitPanel>>"), receiver::Ty::Workspace { .. }), "wrappers unwrap");
    }

    #[test]
    fn same_named_types_keep_their_own_impls_and_returns() {
        for a_impl in ["crate::a::Widget", "AW"] {
            let implementation = format!(
                "use crate::a::Widget as AW; impl {a_impl} {{
                    pub fn direct(&self) {{}}
                    pub fn get(&self) -> crate::a::Widget {{ todo!() }}
                }}
                impl crate::b::Widget {{
                    pub fn get(&self) -> crate::b::Widget {{ todo!() }}
                }}"
            );
            let (dir, _store, ctx) = crate_ws(&[
                ("lib.rs", "mod a; mod b; mod imp; mod tr; mod caller;"),
                ("a.rs", "pub struct Widget; impl Widget { pub fn finish(&self) {} }"),
                ("b.rs", "pub struct Widget; impl Widget { pub fn direct(&self) {} pub fn finish(&self) {} pub fn foreign(&self) {} } impl crate::tr::Tr for Widget {}"),
                ("imp.rs", &implementation),
                ("tr.rs", "pub trait Tr { fn default_method(&self) {} }"),
                ("caller.rs", "fn run(x: crate::a::Widget) { x.direct(); x.get().finish(); x.foreign(); x.default_method(); }"),
            ]);
            assert_eq!(files(&method_outcome(&dir, &ctx, "src/caller.rs", "direct")), ["src/imp.rs"]);
            assert_eq!(files(&method_outcome(&dir, &ctx, "src/caller.rs", "finish")), ["src/a.rs"]);
            for method in ["foreign", "default_method"] {
                assert_eq!(method_outcome(&dir, &ctx, "src/caller.rs", method), Outcome::NoEdge { reason: "typed_no_method" });
            }
        }
    }

    #[test]
    fn typed_lookup_order_inherent_trait_impl_default_and_no_method() {
        let (dir, _store, ctx) = crate_ws(&[
            ("lib.rs", "pub mod tr; pub mod ty; pub mod imp; pub mod caller;\n"),
            ("tr.rs", "pub trait Shape { fn area(&self) -> u32; fn label(&self) -> String { String::new() } }\n"),
            ("ty.rs", "#[derive(Clone)]\npub struct Sq;\nimpl Sq { pub fn side(&self) {} }\n"),
            ("imp.rs", "use crate::tr::Shape;\nuse crate::ty::Sq;\nimpl Shape for Sq { fn area(&self) -> u32 { 0 } }\n"),
            (
                "caller.rs",
                "use crate::ty::Sq;\nuse crate::tr::Shape;\n\
                 pub fn f(s: Sq, d: &dyn Shape) { s.side(); s.area(); s.label(); s.cmp_all(); d.area(); }\n",
            ),
        ]);
        let c = "src/caller.rs";
        assert_eq!(method_outcome(&dir, &ctx, c, "side"), Outcome::Files {
            step: "rust_receiver_inherent",
            targets: [("src/ty.rs".to_string(), Some("side".to_string()))].into()
        });
        assert!(matches!(method_outcome(&dir, &ctx, c, "area"), Outcome::Files { step: "rust_receiver_trait_impl", ref targets } if targets.iter().all(|(f, _)| f == "src/imp.rs")));
        assert!(matches!(method_outcome(&dir, &ctx, c, "label"), Outcome::Files { step: "rust_receiver_trait_default", ref targets } if targets.iter().all(|(f, _)| f == "src/tr.rs")));
        assert_eq!(method_outcome(&dir, &ctx, c, "cmp_all"), Outcome::NoEdge { reason: "typed_no_method" });
        let src = std::fs::read_to_string(dir.path().join(c)).unwrap();
        let refs = super::super::edges::extract_references_for_extension("rs", &src);
        let scope = FileScope::build(Lang::Rust, &refs);
        let dyn_area = refs.iter().filter(|r| r.target_name == "area").nth(1).unwrap();
        assert!(matches!(ctx.resolve(c, Lang::Rust, &scope, dyn_area), Outcome::Files { step: "rust_receiver_bound", ref targets } if targets.iter().all(|(f, _)| f == "src/tr.rs")));
    }

    #[test]
    fn method_returns_follow_inherent_trait_impl_default_precedence() {
        for separate in [false, true] {
            let inherent = "impl Holder { pub fn get(&self) -> crate::a::A { todo!() } pub fn empty(&self) {} }";
            let holder = format!("pub struct Holder; impl crate::tr::Tr for Holder {{ fn get(&self) -> crate::b::B {{ todo!() }} fn empty(&self) -> crate::b::B {{ todo!() }} }} {}", if separate { "" } else { inherent });
            let imp = if separate { format!("use crate::holder::Holder; {inherent}") } else { String::new() };
            let (dir, _store, ctx) = crate_ws(&[
                ("lib.rs", "mod holder; mod imp; mod tr; mod a; mod b; mod caller;"),
                ("a.rs", "pub struct A; impl A { pub fn frobnicate(&self) {} }"),
                ("b.rs", "pub struct B; impl B { pub fn frobnicate(&self) {} }"),
                ("tr.rs", "pub trait Tr { fn get(&self) -> crate::b::B; fn empty(&self) -> crate::b::B; fn defaulted(&self) -> crate::a::A { todo!() } }"),
                ("holder.rs", &holder),
                ("imp.rs", &imp),
                ("caller.rs", "use crate::holder::Holder; fn run(x: Holder) { x.get().frobnicate(); }"),
            ]);
            assert_eq!(files(&method_outcome(&dir, &ctx, "src/caller.rs", "frobnicate")), ["src/a.rs"]);
            let returned = |method: &str| {
                use super::super::types::{Origin, ReceiverExpr, Root, Step, TypeRef};
                ctx.eval_receiver("src/caller.rs", &ReceiverExpr {
                    root: Root::Typed(TypeRef::parse("Holder")),
                    steps: vec![Step::Method(method.into())],
                    origin: Origin::Direct,
                })
            };
            assert_eq!(returned("get"), probe_type(&ctx, "src/a.rs", "A"));
            assert_eq!(returned("empty"), receiver::Ty::Unknown);
            assert_eq!(returned("defaulted"), probe_type(&ctx, "src/a.rs", "A"));
        }
    }

    #[test]
    fn self_inside_a_wrapped_return_survives_try_unwrap_and_ok() {
        // `Result<Self, E>` / `Option<Self>` keep the type `Self` stood for
        // through `?`, `unwrap()` and `ok()`; `b.rs` defines the same
        // method name, so a lost receiver would be ambiguous.
        let (dir, _store, ctx) = crate_ws(&[
            ("lib.rs", "pub mod a; pub mod b; pub mod caller;\n"),
            (
                "a.rs",
                "pub struct A;\nimpl A {\n\
                   pub fn open() -> Result<Self, ()> { Ok(Self) }\n\
                   pub fn find() -> Option<Self> { None }\n\
                   pub fn frobnicate(&self) {}\n\
                   pub fn tweak(&self) {}\n\
                   pub fn poke(&self) {}\n\
                 }\n",
            ),
            ("b.rs", "pub struct B;\nimpl B { pub fn frobnicate(&self) {} pub fn tweak(&self) {} pub fn poke(&self) {} }\n"),
            (
                "caller.rs",
                "fn run() -> Result<(), ()> {\n\
                   let x = crate::a::A::open()?;\n\
                   x.frobnicate();\n\
                   crate::a::A::find().unwrap().tweak();\n\
                   crate::a::A::open().ok().unwrap().poke();\n\
                   Ok(())\n\
                 }\n",
            ),
        ]);
        for m in ["frobnicate", "tweak", "poke"] {
            assert_eq!(
                method_outcome(&dir, &ctx, "src/caller.rs", m),
                Outcome::Files {
                    step: "rust_receiver_inherent",
                    targets: [("src/a.rs".to_string(), Some(m.to_string()))].into()
                },
                "{m}"
            );
        }
    }

    #[test]
    fn typescript_default_reexports_reach_the_defining_file() {
        let dir = tempfile::tempdir().unwrap();
        for (file, text) in [
            ("impl.ts", "export default function work() {}\nexport function named() {}\n"),
            ("barrel.ts", "export { default } from './impl';\nexport * from './impl';\n"),
            ("renamed.ts", "export { named as default } from './impl';\n"),
            ("own.ts", "export default function own() {}\n"),
        ] {
            std::fs::write(dir.path().join(file), text).unwrap();
        }
        let store = GraphStore::open_memory().unwrap();
        super::super::graph_builder::build_graph_into(&store, dir.path(), &[]).unwrap();
        let ctx = ResolveContext::build(&store, dir.path()).unwrap();
        let call_target = |src: &str| {
            let refs = super::super::edges::extract_references_for_extension("ts", src);
            let scope = FileScope::build(Lang::TypeScript, &refs);
            let r = refs.iter().find(|r| r.kind == RefKind::Call).unwrap();
            ctx.resolve("caller.ts", Lang::TypeScript, &scope, r)
        };
        assert_eq!(files(&call_target("import work from './barrel'; work();")), ["impl.ts"], "explicit default re-export");
        assert_eq!(files(&call_target("import n from './renamed'; n();")), ["impl.ts"], "`named as default`");
        assert_eq!(files(&call_target("import o from './own'; o();")), ["own.ts"], "a file's own default");
        assert_eq!(files(&call_target("import { named } from './barrel'; named();")), ["impl.ts"], "`export *` still forwards names");
    }

    #[test]
    fn two_impls_of_one_trait_in_one_file_keep_their_own_overrides() {
        // Both `on` methods share one node (`imps.rs::on`); the per-impl
        // method facts still say A overrides the default.
        let (dir, _store, ctx) = crate_ws(&[
            ("lib.rs", "pub mod tr; pub mod imps; pub mod caller;\n"),
            ("tr.rs", "pub trait Tr { fn on(&self) {} }\n"),
            (
                "imps.rs",
                "use crate::tr::Tr;\npub struct A; pub struct B;\nimpl Tr for A { fn on(&self) {} }\nimpl Tr for B { fn on(&self) {} }\n",
            ),
            ("caller.rs", "use crate::imps::A;\nuse crate::tr::Tr;\npub fn f(a: A) { a.on(); }\n"),
        ]);
        assert_eq!(
            method_outcome(&dir, &ctx, "src/caller.rs", "on"),
            Outcome::Files {
                step: "rust_receiver_trait_impl",
                targets: [("src/imps.rs".to_string(), Some("on".to_string()))].into()
            }
        );
    }

    #[test]
    fn external_receivers_have_no_edge_unless_an_extension_trait_applies() {
        let (dir, _store, ctx) = crate_ws(&[
            ("lib.rs", "pub mod ext; pub mod decoy; pub mod caller;\n"),
            ("ext.rs", "pub trait Ext { fn shout(&self); }\nimpl Ext for String { fn shout(&self) {} }\n"),
            ("decoy.rs", "pub struct D; impl D { pub fn recv_many(&self) {} pub fn shout(&self) {} }\n"),
            (
                "caller.rs",
                "use crate::ext::Ext;\npub fn f(rx: tokio::sync::mpsc::Receiver<u8>, s: String, n: u32) { rx.recv_many(); s.shout(); n.shout(); }\n",
            ),
        ]);
        let c = "src/caller.rs";
        assert_eq!(method_outcome(&dir, &ctx, c, "recv_many"), Outcome::NoEdge { reason: "external_receiver" });
        let src = std::fs::read_to_string(dir.path().join(c)).unwrap();
        let refs = super::super::edges::extract_references_for_extension("rs", &src);
        let scope = FileScope::build(Lang::Rust, &refs);
        let shouts: Vec<Outcome> = refs
            .iter()
            .filter(|r| r.target_name == "shout")
            .map(|r| ctx.resolve(c, Lang::Rust, &scope, r))
            .collect();
        assert_eq!(files(&shouts[0]), ["src/ext.rs"], "String implements Ext");
        assert_eq!(shouts[1], Outcome::NoEdge { reason: "external_receiver" }, "u32 does not");
    }

    #[test]
    fn explicit_type_imports_do_not_fall_back_to_workspace_homonyms() {
        let (dir, _store, ctx) = crate_ws(&[
            ("lib.rs", "pub mod local; pub mod caller; pub mod glob; pub mod missing; pub mod valid;"),
            ("local.rs", "pub struct Instant; impl Instant { pub fn elapsed(&self) {} }"),
            ("caller.rs", "use std::time::Instant; fn run(x: Instant) { x.elapsed(); }"),
            ("glob.rs", "use crate::local::*; use std::time::Instant; fn run(x: Instant) { x.elapsed(); }"),
            ("missing.rs", "use crate::missing_module::Instant; fn run(x: Instant) { x.elapsed(); }"),
            ("valid.rs", "use crate::local::Instant; fn run(x: Instant) { x.elapsed(); }"),
        ]);
        for file in ["src/caller.rs", "src/glob.rs"] {
            assert_eq!(method_outcome(&dir, &ctx, file, "elapsed"), Outcome::NoEdge { reason: "external_receiver" });
        }
        let missing = ctx.imports_of("src/missing.rs");
        assert_eq!(ctx.rust_bare_type("src/missing.rs", missing, "Instant"), None);
        assert_eq!(files(&method_outcome(&dir, &ctx, "src/valid.rs", "elapsed")), ["src/local.rs"]);
    }

    #[test]
    fn std_transforms_reach_the_guarded_and_unwrapped_types() {
        let (dir, _store, ctx) = crate_ws(&[
            ("lib.rs", "pub mod st; pub mod caller;\n"),
            ("st.rs", "pub struct St; impl St { pub fn bump(&self) {} pub fn get(&self) -> Option<St> { None } }\n"),
            (
                "caller.rs",
                "use std::sync::{Arc, Mutex};\nuse std::cell::RefCell;\nuse std::collections::HashMap;\nuse crate::st::St;\n\
                 pub struct H { m: Arc<Mutex<St>>, c: RefCell<St>, map: HashMap<String, St>, v: Vec<St> }\n\
                 impl H { fn f(&self) -> std::io::Result<()> {\n\
                   self.m.lock().unwrap().bump();\n\
                   self.c.borrow_mut().bump();\n\
                   self.map.get(\"k\").unwrap().bump();\n\
                   self.v[0].bump();\n\
                   self.v.first().expect(\"x\").bump();\n\
                   Ok(())\n\
                 } }\n",
            ),
        ]);
        let c = "src/caller.rs";
        let src = std::fs::read_to_string(dir.path().join(c)).unwrap();
        let refs = super::super::edges::extract_references_for_extension("rs", &src);
        let scope = FileScope::build(Lang::Rust, &refs);
        let bumps: Vec<Outcome> = refs
            .iter()
            .filter(|r| r.target_name == "bump")
            .map(|r| ctx.resolve(c, Lang::Rust, &scope, r))
            .collect();
        assert_eq!(bumps.len(), 5);
        for (i, o) in bumps.iter().enumerate() {
            assert_eq!(files(o), ["src/st.rs"], "bump #{i}: {o:?}");
        }
    }

    #[test]
    fn js_specifiers_normalise_and_never_escape_the_root() {
        assert_eq!(normalize_path("src/a/../b/./c").as_deref(), Some("src/b/c"));
        assert_eq!(normalize_path("../x"), None);
    }
}
