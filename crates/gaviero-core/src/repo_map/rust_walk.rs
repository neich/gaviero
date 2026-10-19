//! The Rust reference walk: one iterative pre-order pass that emits the
//! file's references (edge-resolution plan E1) and, for receiver typing
//! (receiver-typing plan R2/R3), its type facts and the scope environment
//! each method call's receiver is written in.
//!
//! Containers (`impl`, `trait`, functions), scopes (functions, closures,
//! blocks) and inline modules are byte-range stacks popped when the walk
//! reaches a node starting at or after their end, the same pattern the
//! symbol walk uses for owners. Nothing recurses over the tree: a deeply
//! nested file must not overflow the stack.

use std::collections::{HashMap, HashSet};

use super::edges::{CallStyle, RawReference, RefKind, expand_use_tree, lift_inline_super, path_segments, rust_call};
use super::types::{MAX_RECEIVER_STEPS, Origin, ReceiverExpr, Root, Step, TypeRef, trait_paths};

/// Workspace-wide type facts of one Rust file (receiver-typing plan 5.4).
/// Type texts are [`TypeRef`] display forms, written relative to this file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RustFileFacts {
    /// `(type name, field, type)`; tuple-struct fields are `0`, `1`, …
    pub fields: Vec<(String, String, String)>,
    /// `(owner, fn name, return type, is_async)`; owner as in
    /// `impl:Ty`, `impl:Ty:Trait`, `trait:Name` or `""`; Ty keeps its path.
    pub returns: Vec<(String, String, String, bool)>,
    /// `(written type path, trait path or "")` per `impl` header.
    pub impls: Vec<(String, String)>,
    /// `(alias name, comma-separated type parameters, aliased type)`.
    pub aliases: Vec<(String, String, String)>,
    /// `(local name or "*", written path, glob)` per `use` / `extern crate`.
    pub imports: Vec<(String, String, bool)>,
    /// `(owner, fn name)` for every function inside an `impl` / `trait`
    /// block. Nodes cannot carry this: two impls of one trait in one file
    /// share a `qualified_name` per method, and only one owner survives.
    pub methods: Vec<(String, String)>,
}

/// A Rust file's references and type facts.
#[derive(Debug, Clone, Default)]
pub struct RustExtracted {
    pub refs: Vec<RawReference>,
    pub facts: RustFileFacts,
}

/// Generic parameters in scope: name → trait bounds.
type Generics = Vec<(String, Vec<Vec<String>>)>;

struct Container {
    end: usize,
    /// `impl:…` / `trait:…` for impl and trait blocks.
    owner: Option<String>,
    /// The type `self` has inside (impl type, or `impl Trait` in a trait).
    self_ty: Option<TypeRef>,
    generics: Generics,
}

#[derive(Debug, Clone)]
enum Binding {
    Typed(TypeRef),
    /// Initialised from this expression, after `depth` substitutions.
    Init(ReceiverExpr, usize),
    Unknown,
}

struct Scope {
    id: usize,
    end: usize,
    bindings: HashMap<String, Binding>,
}

/// A `let` binding becomes visible after its statement ends.
struct Pending {
    at: usize,
    scope_id: usize,
    name: String,
    binding: Binding,
}

/// Local substitutions deeper than this make the receiver unknown.
const MAX_SUBSTITUTIONS: usize = 4;

struct Walk<'s> {
    source: &'s [u8],
    out: Vec<RawReference>,
    facts: RustFileFacts,
    inline_mods: Vec<usize>,
    closures: HashSet<String>,
    containers: Vec<Container>,
    scopes: Vec<Scope>,
    pending: Vec<Pending>,
    next_scope: usize,
}

/// Walk a parsed Rust file.
pub fn walk(root: tree_sitter::Node, source: &[u8]) -> RustExtracted {
    let mut w = Walk {
        source,
        out: Vec::new(),
        facts: RustFileFacts::default(),
        inline_mods: Vec::new(),
        closures: HashSet::new(),
        containers: Vec::new(),
        scopes: Vec::new(),
        pending: Vec::new(),
        next_scope: 0,
    };
    w.run(root);
    let imports = import_rows(&w.out);
    w.facts.imports = imports;
    RustExtracted { refs: w.out, facts: w.facts }
}

/// `use` / `extern crate` bindings as `(local, path, glob)` rows.
fn import_rows(refs: &[RawReference]) -> Vec<(String, String, bool)> {
    refs.iter()
        .filter(|r| r.kind == RefKind::Import && !r.path.is_empty())
        .filter_map(|r| {
            if r.glob {
                return Some(("*".to_string(), r.path.join("::"), true));
            }
            let local = r.alias.clone().unwrap_or_else(|| r.target_name.clone());
            (local != "_" && local != "self").then(|| (local, r.path.join("::"), false))
        })
        .collect()
}

impl<'s> Walk<'s> {
    fn text(&self, n: tree_sitter::Node) -> &'s str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn run(&mut self, root: tree_sitter::Node) {
        let mut cursor = root.walk();
        loop {
            let node = cursor.node();
            self.enter(node);
            if cursor.goto_first_child() {
                continue;
            }
            loop {
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    return;
                }
            }
        }
    }

    // ── Scope bookkeeping ────────────────────────────────────────────

    fn pop_finished(&mut self, start: usize) {
        while self.inline_mods.last().is_some_and(|end| start >= *end) {
            self.inline_mods.pop();
        }
        while self.containers.last().is_some_and(|c| start >= c.end) {
            self.containers.pop();
        }
        while self.scopes.last().is_some_and(|s| start >= s.end) {
            self.scopes.pop();
        }
        if self.pending.is_empty() {
            return;
        }
        let (ready, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending).into_iter().partition(|p| p.at <= start);
        self.pending = waiting;
        for p in ready {
            if let Some(scope) = self.scopes.iter_mut().rev().find(|s| s.id == p.scope_id) {
                scope.bindings.insert(p.name, p.binding);
            }
        }
    }

    fn push_scope(&mut self, end: usize) -> usize {
        let id = self.next_scope;
        self.next_scope += 1;
        self.scopes.push(Scope { id, end, bindings: HashMap::new() });
        id
    }

    fn current_scope(&self) -> Option<usize> {
        self.scopes.last().map(|s| s.id)
    }

    fn lookup(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|s| s.bindings.get(name))
    }

    fn self_ty(&self) -> Option<&TypeRef> {
        self.containers.iter().rev().find_map(|c| c.self_ty.as_ref())
    }

    fn owner(&self) -> Option<&str> {
        self.containers.iter().rev().find_map(|c| c.owner.as_deref())
    }

    fn generic_bounds(&self, name: &str) -> Option<Vec<Vec<String>>> {
        self.containers
            .iter()
            .rev()
            .find_map(|c| c.generics.iter().find(|(n, _)| n == name).map(|(_, b)| b.clone()))
    }

    /// A written type with generics replaced by their bounds and, when
    /// `with_self`, `Self` replaced by the impl type.
    fn written_type(&self, node: tree_sitter::Node, with_self: bool) -> TypeRef {
        let t = TypeRef::from_node(node, self.source);
        let generics = |n: &str| self.generic_bounds(n);
        let me = if with_self { self.self_ty().cloned() } else { None };
        t.substitute(&generics, me.as_ref())
    }

    /// `type_parameters` and `where` bounds of an item.
    fn generics_of(&self, node: tree_sitter::Node) -> Generics {
        let mut out: Generics = Vec::new();
        if let Some(params) = node.child_by_field_name("type_parameters") {
            let mut c = params.walk();
            for p in params.named_children(&mut c) {
                if p.kind() == "type_parameter"
                    && let Some(name) = p.child_by_field_name("name")
                {
                    let bounds = p
                        .child_by_field_name("bounds")
                        .map(|b| trait_paths(b, self.source))
                        .unwrap_or_default();
                    out.push((self.text(name).to_string(), bounds));
                }
            }
        }
        let mut c = node.walk();
        for child in node.named_children(&mut c) {
            if child.kind() != "where_clause" {
                continue;
            }
            let mut c2 = child.walk();
            for pred in child.named_children(&mut c2) {
                if pred.kind() != "where_predicate" {
                    continue;
                }
                let (Some(left), Some(bounds)) = (pred.child_by_field_name("left"), pred.child_by_field_name("bounds")) else {
                    continue;
                };
                let name = self.text(left).to_string();
                let b = trait_paths(bounds, self.source);
                match out.iter_mut().find(|(n, _)| *n == name) {
                    Some((_, existing)) => existing.extend(b),
                    None => out.push((name, b)),
                }
            }
        }
        out
    }

    /// Identifiers a pattern binds (`Some(x)`, `(a, b)`, `Foo { a, b: c }`).
    fn pattern_names(&self, pat: tree_sitter::Node) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![pat];
        let mut seen = 0;
        while let Some(n) = stack.pop() {
            seen += 1;
            if seen > 256 {
                break;
            }
            match n.kind() {
                "identifier" | "shorthand_field_identifier" => out.push(self.text(n).to_string()),
                _ => {
                    let skip = n.child_by_field_name("type").map(|t| t.id());
                    let mut c = n.walk();
                    for k in n.named_children(&mut c) {
                        if Some(k.id()) != skip && k.kind() != "field_identifier" {
                            stack.push(k);
                        }
                    }
                }
            }
        }
        out
    }

    /// The single name a simple pattern binds (`x`, `mut x`).
    fn simple_name(&self, pat: tree_sitter::Node) -> Option<String> {
        match pat.kind() {
            "identifier" => Some(self.text(pat).to_string()),
            "mut_pattern" => {
                let mut c = pat.walk();
                let id = pat.named_children(&mut c).find(|k| k.kind() == "identifier");
                id.map(|i| self.text(i).to_string())
            }
            _ => None,
        }
    }

    fn bind_later(&mut self, at: usize, name: String, binding: Binding) {
        if let Some(scope_id) = self.current_scope() {
            self.pending.push(Pending { at, scope_id, name, binding });
        }
    }

    fn bind_now(&mut self, name: String, binding: Binding) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.bindings.insert(name, binding);
        }
    }

    // ── Receivers ────────────────────────────────────────────────────

    /// How `node` (a method call's receiver) was written, locals
    /// substituted. Returns the expression and the substitution depth.
    fn receiver(&self, node: tree_sitter::Node) -> (ReceiverExpr, usize) {
        let mut rev: Vec<Step> = Vec::new();
        let mut cur = node;
        let mut origin = Origin::Direct;
        let root = loop {
            if rev.len() > MAX_RECEIVER_STEPS {
                return (ReceiverExpr::unknown(), 0);
            }
            match cur.kind() {
                "field_expression" => {
                    let Some(field) = cur.child_by_field_name("field") else { break Root::Unknown };
                    rev.push(Step::Field(self.text(field).to_string()));
                    match cur.child_by_field_name("value") {
                        Some(v) => cur = v,
                        None => break Root::Unknown,
                    }
                }
                "call_expression" => {
                    let Some(mut func) = cur.child_by_field_name("function") else { break Root::Unknown };
                    if func.kind() == "generic_function"
                        && let Some(f) = func.child_by_field_name("function")
                    {
                        func = f;
                    }
                    match func.kind() {
                        "field_expression" => {
                            let Some(field) = func.child_by_field_name("field") else { break Root::Unknown };
                            rev.push(Step::Method(self.text(field).to_string()));
                            match func.child_by_field_name("value") {
                                Some(v) => cur = v,
                                None => break Root::Unknown,
                            }
                        }
                        "identifier" | "scoped_identifier" => break Root::CallPath(path_segments(self.text(func))),
                        _ => break Root::Unknown,
                    }
                }
                "try_expression" | "await_expression" | "parenthesized_expression" => {
                    match cur.kind() {
                        "try_expression" => rev.push(Step::Try),
                        "await_expression" => rev.push(Step::Await),
                        _ => {}
                    }
                    match cur.named_child(0) {
                        Some(v) => cur = v,
                        None => break Root::Unknown,
                    }
                }
                "reference_expression" => {
                    rev.push(Step::Ref);
                    match cur.child_by_field_name("value") {
                        Some(v) => cur = v,
                        None => break Root::Unknown,
                    }
                }
                "unary_expression" if self.text(cur).starts_with('*') => {
                    rev.push(Step::Deref);
                    match cur.named_child(0) {
                        Some(v) => cur = v,
                        None => break Root::Unknown,
                    }
                }
                "index_expression" => {
                    rev.push(Step::Index);
                    match cur.named_child(0) {
                        Some(v) => cur = v,
                        None => break Root::Unknown,
                    }
                }
                "type_cast_expression" => {
                    break match cur.child_by_field_name("type") {
                        Some(t) => Root::Typed(self.written_type(t, true)),
                        None => Root::Unknown,
                    };
                }
                "self" => {
                    origin = Origin::SelfValue;
                    break match self.self_ty() {
                        Some(t) => Root::Typed(t.clone()),
                        None => Root::Unknown,
                    };
                }
                "identifier" => {
                    let name = self.text(cur);
                    match self.lookup(name) {
                        Some(Binding::Typed(t)) => {
                            origin = Origin::LocalTyped;
                            break Root::Typed(t.clone());
                        }
                        Some(Binding::Init(e, d)) => {
                            if *d >= MAX_SUBSTITUTIONS {
                                return (ReceiverExpr::unknown(), 0);
                            }
                            rev.reverse();
                            let mut out = e.clone().then(rev);
                            if !out.is_unknown() {
                                out.origin = Origin::LocalInit;
                            }
                            return (out, d + 1);
                        }
                        Some(Binding::Unknown) => break Root::Unknown,
                        None => break Root::Path(vec![name.to_string()]),
                    }
                }
                "scoped_identifier" => break Root::Path(path_segments(self.text(cur))),
                "struct_expression" => {
                    break match cur.child_by_field_name("name") {
                        Some(n) => Root::StructLit(path_segments(self.text(n))),
                        None => Root::Unknown,
                    };
                }
                "string_literal" | "raw_string_literal" => break Root::Literal("str".into()),
                "integer_literal" => break Root::Literal("i32".into()),
                "float_literal" => break Root::Literal("f64".into()),
                "boolean_literal" => break Root::Literal("bool".into()),
                "char_literal" => break Root::Literal("char".into()),
                "array_expression" => break Root::Literal("Array".into()),
                "macro_invocation" => {
                    let name = cur.child_by_field_name("macro").map(|m| self.text(m)).unwrap_or("");
                    break match name {
                        "vec" => Root::Literal("Vec".into()),
                        "format" => Root::Literal("String".into()),
                        _ => Root::Unknown,
                    };
                }
                _ => break Root::Unknown,
            }
        };
        if root == Root::Unknown {
            return (ReceiverExpr::unknown(), 0);
        }
        rev.reverse();
        (ReceiverExpr { root, steps: rev, origin }, 0)
    }

    // ── The walk ─────────────────────────────────────────────────────

    fn enter(&mut self, node: tree_sitter::Node) {
        let start = node.start_byte();
        self.pop_finished(start);
        let line = node.start_position().row;
        if node.kind() == "mod_item" && node.child_by_field_name("body").is_some() {
            self.inline_mods.push(node.end_byte());
        }
        let first_new = self.out.len();
        match node.kind() {
            "use_declaration" => {
                let public = {
                    let mut c = node.walk();
                    node.children(&mut c).any(|ch| ch.kind() == "visibility_modifier")
                };
                if let Some(arg) = node.child_by_field_name("argument") {
                    expand_use_tree(arg, self.source, &[], public, line, 0, &mut self.out);
                }
            }
            "extern_crate_declaration" => {
                if let Some(name) = node.child_by_field_name("name").map(|n| self.text(n)) {
                    let alias = node.child_by_field_name("alias").map(|n| self.text(n).to_string());
                    self.out.push(RawReference {
                        path: vec![name.to_string()],
                        alias,
                        ..RawReference::new(name, RefKind::Import, line)
                    });
                }
            }
            "impl_item" => self.enter_impl(node, line),
            "trait_item" => {
                let name = node.child_by_field_name("name").map(|n| self.text(n).to_string());
                let generics = self.generics_of(node);
                self.containers.push(Container {
                    end: node.end_byte(),
                    owner: super::builder::container_owner(node, self.source),
                    self_ty: name.map(|n| TypeRef::Impl(vec![vec![n]])),
                    generics,
                });
            }
            "struct_item" => self.enter_struct(node),
            "type_item" => {
                if let (Some(name), Some(ty)) = (node.child_by_field_name("name"), node.child_by_field_name("type")) {
                    let params: Vec<String> = self.generics_of(node).into_iter().map(|(n, _)| n).collect();
                    let t = TypeRef::from_node(ty, self.source);
                    self.facts.aliases.push((self.text(name).to_string(), params.join(","), t.to_string()));
                }
            }
            "function_item" | "function_signature_item" => self.enter_fn(node),
            "closure_expression" => {
                self.push_scope(node.end_byte());
                if let Some(params) = node.child_by_field_name("parameters") {
                    let mut c = params.walk();
                    let kids: Vec<_> = params.named_children(&mut c).collect();
                    for p in kids {
                        match p.kind() {
                            "parameter" => {
                                let Some(pat) = p.child_by_field_name("pattern") else { continue };
                                let ty = p.child_by_field_name("type").map(|t| self.written_type(t, true));
                                match (self.simple_name(pat), ty) {
                                    (Some(n), Some(t)) => self.bind_now(n, Binding::Typed(t)),
                                    _ => {
                                        for n in self.pattern_names(pat) {
                                            self.bind_now(n, Binding::Unknown);
                                        }
                                    }
                                }
                            }
                            _ => {
                                for n in self.pattern_names(p) {
                                    self.bind_now(n, Binding::Unknown);
                                }
                            }
                        }
                    }
                }
            }
            "block" => {
                self.push_scope(node.end_byte());
            }
            "let_declaration" => self.enter_let(node),
            "if_expression" | "while_expression" => {
                // Condition bindings extend through the successful body, but
                // must be gone before an else branch or the next statement.
                let body = node.child_by_field_name("consequence")
                    .or_else(|| node.child_by_field_name("body"));
                if let Some(body) = body {
                    self.push_scope(body.end_byte());
                }
            }
            "let_condition" | "match_arm" | "for_expression" => {
                if matches!(node.kind(), "match_arm" | "for_expression") {
                    self.push_scope(node.end_byte());
                }
                let (pat, at) = match node.kind() {
                    "match_arm" => {
                        // A match_pattern also contains the guard. Bind before
                        // the guard, and do not treat its identifiers as names.
                        let pat = node.child_by_field_name("pattern").and_then(|p| p.named_child(0));
                        (pat, pat.map(|p| p.end_byte()))
                    }
                    _ => (
                        node.child_by_field_name("pattern"),
                        node.child_by_field_name("value").map(|v| v.end_byte()),
                    ),
                };
                if let (Some(pat), Some(at)) = (pat, at) {
                    for n in self.pattern_names(pat) {
                        self.bind_later(at, n, Binding::Unknown);
                    }
                }
            }
            "call_expression" => {
                if let Some(func) = node.child_by_field_name("function")
                    && let Some(mut r) = rust_call(func, self.source, line)
                    && !(r.style == CallStyle::Bare && self.closures.contains(&r.target_name))
                {
                    if r.style == CallStyle::Method {
                        let callee = if func.kind() == "generic_function" {
                            func.child_by_field_name("function").unwrap_or(func)
                        } else {
                            func
                        };
                        if let Some(value) = callee.child_by_field_name("value") {
                            r.receiver_expr = Some(self.receiver(value).0);
                        }
                    }
                    self.out.push(r);
                }
            }
            _ => {}
        }
        if !self.inline_mods.is_empty() {
            let depth = self.inline_mods.len();
            for r in &mut self.out[first_new..] {
                lift_inline_super(&mut r.path, depth);
            }
        }
    }

    fn enter_impl(&mut self, node: tree_sitter::Node, line: usize) {
        if let Some(path) = node
            .child_by_field_name("trait")
            .map(|n| path_segments(self.text(n)))
            && let Some(name) = path.last().cloned()
        {
            self.out.push(RawReference {
                path,
                ..RawReference::new(name, RefKind::Implements, line)
            });
        }
        let generics = self.generics_of(node);
        // The impl's own generics apply to its type.
        self.containers.push(Container {
            end: node.end_byte(),
            owner: None,
            self_ty: None,
            generics,
        });
        let self_ty = node.child_by_field_name("type").map(|t| self.written_type(t, false));
        if let Some(ty) = node.child_by_field_name("type") {
            let base = path_segments(self.text(ty)).join("::");
            let tr = node
                .child_by_field_name("trait")
                .map(|t| path_segments(self.text(t)).join("::"))
                .unwrap_or_default();
            if !base.is_empty() {
                self.facts.impls.push((base.clone(), tr.clone()));
                self.containers.last_mut().unwrap().owner = Some(if tr.is_empty() {
                    format!("impl:{base}")
                } else {
                    format!("impl:{base}:{tr}")
                });
            }
        }
        if let Some(c) = self.containers.last_mut() {
            c.self_ty = self_ty;
        }
    }

    fn enter_struct(&mut self, node: tree_sitter::Node) {
        let Some(name) = node.child_by_field_name("name").map(|n| self.text(n).to_string()) else { return };
        let generics = self.generics_of(node);
        let Some(body) = node.child_by_field_name("body") else { return };
        let lookup = |n: &str| generics.iter().find(|(g, _)| g == n).map(|(_, b)| b.clone());
        let mut c = body.walk();
        let mut index = 0;
        for f in body.named_children(&mut c) {
            match (body.kind(), f.kind()) {
                ("field_declaration_list", "field_declaration") => {
                    if let (Some(fname), Some(ty)) = (f.child_by_field_name("name"), f.child_by_field_name("type")) {
                        let t = TypeRef::from_node(ty, self.source).substitute(&lookup, None);
                        self.facts.fields.push((name.clone(), self.text(fname).to_string(), t.to_string()));
                    }
                }
                ("ordered_field_declaration_list", k) if k != "visibility_modifier" && k != "attribute_item" => {
                    let t = TypeRef::from_node(f, self.source).substitute(&lookup, None);
                    self.facts.fields.push((name.clone(), index.to_string(), t.to_string()));
                    index += 1;
                }
                _ => {}
            }
        }
    }

    fn enter_fn(&mut self, node: tree_sitter::Node) {
        let generics = self.generics_of(node);
        let owner = self.owner().unwrap_or("").to_string();
        if !owner.is_empty()
            && let Some(name) = node.child_by_field_name("name")
        {
            self.facts.methods.push((owner.clone(), self.text(name).to_string()));
        }
        self.containers.push(Container {
            end: node.end_byte(),
            owner: None,
            self_ty: None,
            generics,
        });
        if let (Some(name), Some(ret)) = (node.child_by_field_name("name"), node.child_by_field_name("return_type")) {
            let is_async = {
                let mut c = node.walk();
                node.children(&mut c)
                    .any(|k| k.kind() == "function_modifiers" && self.text(k).contains("async"))
            };
            let t = self.written_type(ret, false);
            self.facts.returns.push((owner, self.text(name).to_string(), t.to_string(), is_async));
        }
        if node.kind() != "function_item" {
            return;
        }
        self.push_scope(node.end_byte());
        let Some(params) = node.child_by_field_name("parameters") else { return };
        let mut c = params.walk();
        let kids: Vec<_> = params.named_children(&mut c).collect();
        for p in kids {
            if p.kind() != "parameter" {
                continue;
            }
            let Some(pat) = p.child_by_field_name("pattern") else { continue };
            let ty = p.child_by_field_name("type").map(|t| self.written_type(t, true));
            match (self.simple_name(pat), ty) {
                (Some(n), Some(t)) => self.bind_now(n, Binding::Typed(t)),
                _ => {
                    for n in self.pattern_names(pat) {
                        self.bind_now(n, Binding::Unknown);
                    }
                }
            }
        }
    }

    fn enter_let(&mut self, node: tree_sitter::Node) {
        let Some(pat) = node.child_by_field_name("pattern") else { return };
        let value = node.child_by_field_name("value");
        if let (Some(v), Some(name)) = (value, self.simple_name(pat))
            && v.kind() == "closure_expression"
        {
            self.closures.insert(name);
        }
        let at = node.end_byte();
        let Some(name) = self.simple_name(pat) else {
            for n in self.pattern_names(pat) {
                self.bind_later(at, n, Binding::Unknown);
            }
            return;
        };
        let binding = if let Some(t) = node.child_by_field_name("type") {
            Binding::Typed(self.written_type(t, true))
        } else if let Some(v) = value {
            let (e, depth) = self.receiver(v);
            if e.is_unknown() { Binding::Unknown } else { Binding::Init(e, depth) }
        } else {
            Binding::Unknown
        };
        self.bind_later(at, name, binding);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk_src(src: &str) -> RustExtracted {
        super::super::edges::extract_rust_file(src)
    }

    fn receivers(src: &str) -> Vec<(String, String)> {
        walk_src(src)
            .refs
            .into_iter()
            .filter(|r| r.style == CallStyle::Method)
            .map(|r| (r.target_name.clone(), r.receiver_expr.map(|e| e.to_string()).unwrap_or_default()))
            .collect()
    }

    fn get(r: &[(String, String)], n: &str) -> String {
        r.iter().find(|(m, _)| m == n).map(|(_, e)| e.clone()).unwrap_or_else(|| panic!("no call {n}: {r:#?}"))
    }

    #[test]
    fn struct_fields_including_generic_tuple_and_restricted_ones() {
        let f = walk_src(
            "pub struct App { pub(crate) git_panel: GitPanel, state: Arc<Mutex<State>>, items: Vec<Item> }\n\
             struct Wrap<T: Observer, U>(T, pub U, u8);\n\
             struct Unit;\n",
        )
        .facts;
        let want: Vec<(String, String, String)> = [
            ("App", "git_panel", "GitPanel"),
            ("App", "state", "Arc<Mutex<State>>"),
            ("App", "items", "Vec<Item>"),
            ("Wrap", "0", "impl Observer"),
            ("Wrap", "1", "_"),
            ("Wrap", "2", "u8"),
        ]
        .iter()
        .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
        .collect();
        assert_eq!(f.fields, want);
    }

    #[test]
    fn returns_impls_aliases_and_imports() {
        let f = walk_src(
            "use crate::{git::{self, GitPanel as Panel}, ui::*};\nextern crate beta_core as bc;\n\
             pub type Res<T> = std::result::Result<T, Error>;\n\
             pub async fn load() -> Res<Config> { todo!() }\n\
             fn plain() {}\n\
             pub trait Tr { fn make(&self) -> Self; fn name(&self) -> &str; }\n\
             impl<T> Tr for Vec<T> { fn make(&self) -> Self { todo!() } fn name(&self) -> &str { \"\" } }\n\
             impl App { pub fn panel(&self) -> &Panel { &self.p } fn first<G: Tr>(&self, g: G) -> G { g } }\n",
        )
        .facts;
        assert_eq!(f.aliases, [("Res".to_string(), "T".to_string(), "std::result::Result<T,Error>".to_string())]);
        let returns: Vec<(&str, &str, &str, bool)> = f
            .returns
            .iter()
            .map(|(o, n, r, a)| (o.as_str(), n.as_str(), r.as_str(), *a))
            .collect();
        assert_eq!(
            returns,
            [
                ("", "load", "Res<Config>", true),
                ("trait:Tr", "make", "Self", false),
                ("trait:Tr", "name", "&str", false),
                ("impl:Vec:Tr", "make", "Self", false),
                ("impl:Vec:Tr", "name", "&str", false),
                ("impl:App", "panel", "&Panel", false),
                ("impl:App", "first", "impl Tr", false),
            ]
        );
        let impls: Vec<(&str, &str)> = f.impls.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        assert_eq!(impls, [("Vec", "Tr"), ("App", "")]);
        let imports: Vec<(&str, &str, bool)> = f.imports.iter().map(|(a, b, g)| (a.as_str(), b.as_str(), *g)).collect();
        assert_eq!(
            imports,
            [
                ("git", "crate::git", false),
                ("Panel", "crate::git::GitPanel", false),
                ("*", "crate::ui", true),
                ("bc", "beta_core", false),
            ]
        );
    }

    #[test]
    fn receivers_follow_self_params_lets_and_chains() {
        let r = receivers(
            "impl App {\n\
               fn f(&mut self, p: &Panel, mut q: Queue, g: impl Tr) {\n\
                 self.go();\n\
                 self.git_panel.refresh();\n\
                 p.scroll();\n\
                 q.push_job();\n\
                 g.act();\n\
                 let t = Instant::now();\n\
                 t.since();\n\
                 let s = self.state.lock().unwrap();\n\
                 s.bump();\n\
                 let w: Widget = make();\n\
                 w.spin();\n\
                 Foo::open(x)?.run();\n\
                 \"lit\".shout();\n\
                 Bar { a: 1 }.touch();\n\
                 (*self.inner).deref_me();\n\
                 (x as Widget).cast_me();\n\
                 unknown_thing.whatever();\n\
               }\n\
             }\n",
        );
        for (name, expr) in [
            ("go", "typed App"),
            ("refresh", "typed App | .git_panel"),
            ("scroll", "typed &Panel"),
            ("push_job", "typed Queue"),
            ("act", "typed impl Tr"),
            ("since", "call Instant::now"),
            ("bump", "typed App | .state | .lock() | .unwrap()"),
            ("spin", "typed Widget"),
            ("run", "call Foo::open | ?"),
            ("shout", "lit str"),
            ("touch", "struct Bar"),
            ("deref_me", "typed App | .inner | *"),
            ("cast_me", "typed Widget"),
            ("whatever", "path unknown_thing"),
        ] {
            assert_eq!(get(&r, name), expr, "{name}");
        }
    }

    #[test]
    fn shadowing_closures_generics_and_trait_self() {
        let r = receivers(
            "fn outer<G>(g: G) where G: Observer {\n\
               g.notify_g();\n\
               let x: A = a();\n\
               { let x: B = b(); x.in_block(); }\n\
               x.after_block();\n\
               let c = |x| x.closure_param();\n\
               let d = |y: Widget| y.typed_param();\n\
               let x = x.next_x();\n\
               x.chained();\n\
               if let Some(x) = maybe() { x.if_let(); }\n\
               for x in xs { x.in_for(); }\n\
             }\n\
             trait Tr { fn d(&self) { self.default_body(); } }\n",
        );
        assert_eq!(get(&r, "notify_g"), "typed impl Observer", "where-clause bound");
        assert_eq!(get(&r, "in_block"), "typed B", "inner let shadows");
        assert_eq!(get(&r, "after_block"), "typed A", "block scope ended");
        assert_eq!(get(&r, "closure_param"), "unknown", "an unannotated closure param is unknown");
        assert_eq!(get(&r, "typed_param"), "typed Widget");
        assert_eq!(get(&r, "next_x"), "typed A", "a let's own initialiser sees the outer binding");
        assert_eq!(get(&r, "chained"), "typed A | .next_x()");
        assert_eq!(get(&r, "if_let"), "unknown", "pattern bindings are unknown");
        assert_eq!(get(&r, "in_for"), "unknown");
        assert_eq!(get(&r, "default_body"), "typed impl Tr", "`self` in a trait body");
    }

    #[test]
    fn pattern_bindings_stay_in_their_scopes() {
        let r = receivers(
            "fn f(x: A) {
                for x in x.iter_source() { x.in_for(); }
                x.after_for();
                if let Some(x) = x.if_source() { x.in_if(); }
                else { x.in_else(); }
                x.after_if();
                while let Some(x) = x.while_source() { x.in_while(); }
                x.after_while();
                match x.match_source() {
                    Some(x) if x.guard() => x.in_arm(),
                    _ => x.other_arm(),
                }
                x.after_match();
                if let Some(x) = x.chain_source() && x.chain_guard() {
                    x.in_chain();
                }
                x.after_chain();
            }",
        );
        for name in [
            "iter_source", "after_for", "if_source", "in_else", "after_if",
            "while_source", "after_while", "match_source", "other_arm",
            "after_match", "chain_source", "after_chain",
        ] {
            assert_eq!(get(&r, name), "typed A", "{name}");
        }
        for name in ["in_for", "in_if", "in_while", "guard", "in_arm", "chain_guard", "in_chain"] {
            assert_eq!(get(&r, name), "unknown", "{name}");
        }
    }

    #[test]
    fn substitution_is_depth_limited() {
        let r = receivers(
            "fn f() { let a = A::new(); let b = a.x(); let c = b.x(); let d = c.x(); let e = d.x(); let g = e.x(); g.end(); d.mid(); }\n",
        );
        assert_eq!(get(&r, "mid"), "call A::new | .x() | .x() | .x()");
        assert_eq!(get(&r, "end"), "unknown", "past four substitutions the receiver is unknown");
    }

    #[test]
    fn method_calls_and_only_they_carry_a_receiver() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/edge_resolution/rust");
        let mut stack = vec![dir];
        let mut n = 0;
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    let src = std::fs::read_to_string(&p).unwrap();
                    for r in walk_src(&src).refs {
                        n += 1;
                        assert_eq!(r.receiver_expr.is_some(), r.style == CallStyle::Method, "{r:?}");
                    }
                }
            }
        }
        assert!(n > 20, "fixture walked ({n} refs)");
    }

    #[test]
    fn a_deeply_nested_file_extracts_on_a_small_stack() {
        let depth = 60_000;
        let src = format!(
            "struct S {{ a: u8 }}\nfn deep() -> i32 {{ {}x.m(){} }}\n",
            "(".repeat(depth),
            ")".repeat(depth)
        );
        let out = std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || walk_src(&src).facts.fields.len())
            .unwrap()
            .join()
            .expect("walk must not overflow");
        assert_eq!(out, 1);
    }
}
