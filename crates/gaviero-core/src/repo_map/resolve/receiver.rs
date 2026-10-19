//! Receiver typing for Rust method calls (receiver-typing plan R4/R5).
//!
//! A method call carries its [`ReceiverExpr`]: how the receiver was
//! written, locals already substituted. Here it is evaluated against the
//! workspace's type facts (struct fields, function returns, impl headers,
//! aliases, per-file imports) to a [`Ty`], and the method is looked up on
//! that type:
//!
//! - a workspace type → its inherent methods, then trait impls, then
//!   default methods of traits it implements; none → no edge (a derived or
//!   std trait method);
//! - trait bounds (`&dyn T`, `impl T`, `G: T`) → the declaring trait;
//! - an external type → no edge, unless a workspace trait in scope is
//!   implemented for it and declares the method (an extension trait);
//! - unknown → the name-based steps of [`ResolveContext::rust_method_call`].

use std::collections::BTreeSet;

use super::super::types::{
    ReceiverExpr, Root, STD_TYPE_NAMES, Step, StdReturn, TRANSPARENT_WRAPPERS, TypeRef, std_index_arg, std_return,
};
use super::{Outcome, ResolveContext, RustImports, RustModuleMap, RustRes, files_of};

/// What a receiver evaluated to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Ty {
    /// A workspace struct / enum `name` defined in `file`.
    Workspace { name: String, file: String },
    /// A std / dependency type; `args` are its written type arguments,
    /// to be resolved in `ctx` with `self_ty` standing for `Self`
    /// (`Result<Self, E>` returned by an inherent method of `self_ty`).
    External { name: String, args: Vec<TypeRef>, ctx: String, self_ty: Option<Box<Ty>> },
    /// Only trait bounds are known: `(trait, file)` per workspace trait.
    Traits(Vec<(String, String)>),
    Unknown,
}

/// Type resolution deeper than this (aliases, wrappers, nesting) gives up.
const MAX_TYPE_HOPS: usize = 8;

/// Split the single colon between an impl type and trait, preserving `::` paths.
pub(super) fn split_impl_owner(owner: &str) -> (&str, Option<&str>) {
    let bytes = owner.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b':' && (i == 0 || bytes[i - 1] != b':') && bytes.get(i + 1) != Some(&b':') {
            return (&owner[..i], Some(&owner[i + 1..]));
        }
    }
    (owner, None)
}

/// `Type::f(…)` with no recorded return type is taken to build a `Type`
/// only for these names (a logged heuristic, plan 5.6).
fn constructor_like(name: &str) -> bool {
    name == "new" || name == "default" || name.starts_with("from") || name.starts_with("with_")
}

/// Positional replacement of an alias's type parameters by its arguments.
fn subst_params(t: &TypeRef, params: &[String], args: &[TypeRef], depth: usize) -> TypeRef {
    if depth > 32 {
        return TypeRef::Unknown;
    }
    match t {
        TypeRef::Path { segs, args: a } => {
            if segs.len() == 1
                && a.is_empty()
                && let Some(i) = params.iter().position(|p| *p == segs[0])
            {
                return args.get(i).cloned().unwrap_or(TypeRef::Unknown);
            }
            TypeRef::Path {
                segs: segs.clone(),
                args: a.iter().map(|x| subst_params(x, params, args, depth + 1)).collect(),
            }
        }
        TypeRef::Ref(x) => TypeRef::Ref(Box::new(subst_params(x, params, args, depth + 1))),
        TypeRef::Array(x) => TypeRef::Array(Box::new(subst_params(x, params, args, depth + 1))),
        TypeRef::Tuple(xs) => TypeRef::Tuple(xs.iter().map(|x| subst_params(x, params, args, depth + 1)).collect()),
        other => other.clone(),
    }
}

impl ResolveContext {
    /// Debug view of a receiver's evaluation, step by step (for
    /// `examples/receiver_debug.rs`).
    pub fn explain_receiver(&self, file: &str, e: &ReceiverExpr) -> Vec<String> {
        let mut out = vec![format!("expr: {e}")];
        let mut ty = match &e.root {
            Root::Typed(t) => self.resolve_type(file, t, None, 0),
            Root::CallPath(segs) => self.call_return(file, segs, None, 0),
            Root::StructLit(segs) => self.resolve_type(file, &TypeRef::Path { segs: segs.clone(), args: Vec::new() }, None, 0),
            Root::Literal(kind) => Ty::External { name: kind.clone(), args: Vec::new(), ctx: file.to_string(), self_ty: None },
            Root::Path(_) | Root::Unknown => Ty::Unknown,
        };
        out.push(format!("root → {ty:?}"));
        for step in &e.steps {
            ty = self.apply_step(ty, step);
            out.push(format!("{step:?} → {ty:?}"));
        }
        out
    }

    /// The import table of any Rust file (from the persisted `rust_imports`).
    pub(super) fn imports_of(&self, file: &str) -> &RustImports {
        static EMPTY: std::sync::LazyLock<RustImports> = std::sync::LazyLock::new(RustImports::default);
        self.file_imports.get(file).unwrap_or(&EMPTY)
    }

    fn crate_of_file(&self, file: &str) -> Option<&str> {
        self.rust.module_of(file).map(RustModuleMap::crate_of)
    }

    /// Whether a written path can only name something outside the
    /// workspace (`std::…`, `tokio::…`, a name imported from there).
    fn is_external_path(&self, ctx: &str, segs: &[String]) -> bool {
        let Some(first) = segs.first() else { return false };
        if matches!(first.as_str(), "crate" | "self" | "super" | "Self") {
            return false;
        }
        if self.rust.crate_root(first).is_some() {
            return false;
        }
        if let Some(module) = self.rust.module_of(ctx)
            && self.rust.is_module(&format!("{module}::{first}"))
        {
            return false;
        }
        true
    }

    /// An alias named by `segs` as seen from `ctx`: `(alias file, params,
    /// aliased type text)`.
    fn alias_for(&self, ctx: &str, segs: &[String]) -> Option<(String, Vec<String>, String)> {
        let name = segs.last()?;
        let lookup = |file: &str| {
            self.aliases
                .get(&(file.to_string(), name.clone()))
                .map(|(ps, t)| (file.to_string(), ps.clone(), t.clone()))
        };
        if segs.len() == 1 {
            if let Some(hit) = lookup(ctx) {
                return Some(hit);
            }
            let bound = self.imports_of(ctx).bindings.get(name)?;
            return self.alias_at_path(ctx, bound);
        }
        self.alias_at_path(ctx, segs)
    }

    /// The alias at the end of a full path: its module prefix resolves to a
    /// module file that defines the alias.
    fn alias_at_path(&self, ctx: &str, segs: &[String]) -> Option<(String, Vec<String>, String)> {
        let (name, prefix) = segs.split_last()?;
        let file = if prefix.is_empty() {
            ctx.to_string()
        } else {
            match self.resolve_rust_path(ctx, prefix, Some(self.imports_of(ctx)), 0)? {
                RustRes::Module(m) => self.rust.file_of(&m)?.to_string(),
                _ => return None,
            }
        };
        self.aliases
            .get(&(file.clone(), name.clone()))
            .map(|(ps, t)| (file, ps.clone(), t.clone()))
    }

    /// The kind of the type-like definition `name` in `file`. Symbols of
    /// test files are all stored as `Test`; there a capitalised name is
    /// taken to be a struct (traits in test files are not modelled).
    fn type_kind(&self, file: &str, name: &str) -> Option<&str> {
        let defs = self.defs_by_file.get(file)?.get(name)?;
        defs.iter()
            .filter(|d| d.owner.is_none())
            .map(|d| d.kind.as_str())
            .find_map(|k| match k {
                "Struct" | "Enum" | "Trait" | "Class" | "Interface" => Some(k),
                "Test" if name.starts_with(|c: char| c.is_ascii_uppercase()) => Some("Struct"),
                _ => None,
            })
    }

    /// A trait path written in `ctx`, if it names a workspace trait.
    fn resolve_trait(&self, ctx: &str, path: &[String]) -> Option<(String, String)> {
        let name = path.last()?.clone();
        let imports = self.imports_of(ctx);
        let (file, name) = if path.len() == 1 {
            self.rust_bare_type_identity(ctx, imports, &name)?
        } else {
            match self.resolve_rust_path(ctx, path, Some(imports), 0)? {
                RustRes::Item { file, name } => (file, name),
                _ => return None,
            }
        };
        (self.type_kind(&file, &name) == Some("Trait")).then_some((name, file))
    }

    /// Resolve a written type in the context of file `ctx`.
    pub(super) fn resolve_type(&self, ctx: &str, t: &TypeRef, self_ty: Option<&Ty>, hops: usize) -> Ty {
        if hops > MAX_TYPE_HOPS {
            return Ty::Unknown;
        }
        match t {
            TypeRef::Ref(inner) => self.resolve_type(ctx, inner, self_ty, hops + 1),
            TypeRef::SelfType => self_ty.cloned().unwrap_or(Ty::Unknown),
            TypeRef::Dyn(ps) | TypeRef::Impl(ps) => {
                let traits: Vec<_> = ps.iter().filter_map(|p| self.resolve_trait(ctx, p)).collect();
                if !traits.is_empty() {
                    Ty::Traits(traits)
                } else if let Some(first) = ps.first().and_then(|p| p.last()) {
                    // Only external traits (`Iterator`, `Fn`, `Any`).
                    Ty::External { name: first.clone(), args: Vec::new(), ctx: ctx.to_string(), self_ty: None }
                } else {
                    Ty::Unknown
                }
            }
            TypeRef::Array(inner) => Ty::External {
                name: "Array".into(),
                args: vec![(**inner).clone()],
                ctx: ctx.to_string(),
                self_ty: self_ty.cloned().map(Box::new),
            },
            TypeRef::Tuple(_) => Ty::External { name: "Tuple".into(), args: Vec::new(), ctx: ctx.to_string(), self_ty: None },
            TypeRef::Unknown => Ty::Unknown,
            TypeRef::Path { segs, args } => self.resolve_path_type(ctx, segs, args, self_ty, hops),
        }
    }

    fn resolve_path_type(&self, ctx: &str, segs: &[String], args: &[TypeRef], self_ty: Option<&Ty>, hops: usize) -> Ty {
        let Some(last) = segs.last() else { return Ty::Unknown };
        if segs.len() == 1 && last == "Self" {
            return self_ty.cloned().unwrap_or(Ty::Unknown);
        }
        if let Some((afile, params, text)) = self.alias_for(ctx, segs) {
            let aliased = subst_params(&TypeRef::parse(&text), &params, args, 0);
            return self.resolve_type(&afile, &aliased, self_ty, hops + 1);
        }
        let imports = self.imports_of(ctx);
        let bound = (segs.len() == 1).then(|| imports.bindings.get(last)).flatten();
        let found = if segs.len() == 1 {
            let std_name = bound.is_none() && !self.defines(ctx, last) && STD_TYPE_NAMES.contains(&last.as_str());
            if std_name { None } else { self.rust_bare_type_identity(ctx, imports, last) }
        } else {
            match self.resolve_rust_path(ctx, segs, Some(imports), 0) {
                Some(RustRes::Item { file, name }) => Some((file, name)),
                _ => None,
            }
        };
        if let Some((file, name)) = found {
            return match self.type_kind(&file, &name) {
                Some("Trait") => Ty::Traits(vec![(name, file)]),
                Some(_) => Ty::Workspace { name, file },
                None => Ty::Unknown,
            };
        }
        let external = if segs.len() == 1 {
            match bound {
                Some(path) => self.is_external_path(ctx, path),
                None => STD_TYPE_NAMES.contains(&last.as_str()),
            }
        } else {
            self.is_external_path(ctx, segs)
        };
        if !external {
            return Ty::Unknown;
        }
        if TRANSPARENT_WRAPPERS.contains(&last.as_str()) {
            return match args.first() {
                Some(inner) => self.resolve_type(ctx, inner, self_ty, hops + 1),
                None => Ty::Unknown,
            };
        }
        Ty::External {
            name: last.clone(),
            args: args.to_vec(),
            ctx: ctx.to_string(),
            self_ty: self_ty.cloned().map(Box::new),
        }
    }

    /// Prefer the type's own file, then its crate, then anything.
    fn prefer_home(&self, tfile: &str, files: BTreeSet<String>) -> BTreeSet<String> {
        if files.contains(tfile) {
            return [tfile.to_string()].into();
        }
        let home = self.crate_of_file(tfile);
        let same: BTreeSet<String> = files.iter().filter(|f| self.crate_of_file(f) == home).cloned().collect();
        if same.is_empty() { files } else { same }
    }

    /// Files with an `impl T` method `m`: inherent ones, or trait-impl ones.
    fn impl_method_files(&self, ty: &str, tfile: &str, m: &str, inherent: bool) -> BTreeSet<String> {
        self.impl_methods
            .iter()
            .filter(|((_, method), _)| method == m)
            .flat_map(|((written, _), rows)| rows.iter().map(move |row| (written, row)))
            .filter(|(written, (f, tr))| tr.is_none() == inherent && self.impl_matches(f, written, ty, tfile))
            .map(|(_, (f, _))| f.clone())
            .collect()
    }

    fn impl_matches(&self, ctx: &str, written: &str, name: &str, file: &str) -> bool {
        matches!(self.resolve_type(ctx, &TypeRef::parse(written), None, 0),
            Ty::Workspace { name: n, file: f } if n == name && f == file)
    }

    /// Traits `T` (defined in `tfile`) implements, as `(trait name, file)`.
    fn traits_implemented(&self, ty: &str, tfile: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (written, rows) in &self.type_impls {
            for (file, tr) in rows {
                if tr.is_empty() || !self.impl_matches(file, written, ty, tfile) {
                    continue;
                }
                let segs: Vec<String> = tr.split("::").map(str::to_string).collect();
                if let Some(hit) = self.resolve_trait(file, &segs) {
                    out.push(hit);
                }
            }
        }
        out
    }

    /// The return type of method `m` on a value of type `on`.
    fn method_return(&self, on: &Ty, m: &str, hops: usize) -> Ty {
        let rows = match self.fn_returns.get(m) {
            Some(r) => r,
            None => return Ty::Unknown,
        };
        match on {
            Ty::Workspace { name, file } => {
                let matches_owner = |f: &str, owner: &str, inherent: bool| {
                    owner.strip_prefix("impl:").is_some_and(|rest| {
                        let (written, tr) = split_impl_owner(rest);
                        tr.is_none() == inherent && self.impl_matches(f, written, name, file)
                    })
                };
                // Select the method tier before looking at return facts. A
                // method without a recorded return must not borrow one from
                // a lower-priority trait method.
                let inherent_files = self.impl_method_files(name, file, m, true);
                let trait_files = self.impl_method_files(name, file, m, false);
                let cands: Vec<&(String, String, String)> = if !inherent_files.is_empty() {
                    let pick = self.prefer_home(file, inherent_files);
                    rows.iter().filter(|(f, o, _)| matches_owner(f, o, true) && pick.contains(f)).collect()
                } else if !trait_files.is_empty() {
                    let pick = self.prefer_home(file, trait_files);
                    rows.iter().filter(|(f, o, _)| matches_owner(f, o, false) && pick.contains(f)).collect()
                } else {
                    let traits = self.traits_implemented(name, file);
                    rows.iter().filter(|(f, o, _)| traits.iter().any(|(t, tf)| f == tf && *o == format!("trait:{t}"))).collect()
                };
                let files: BTreeSet<String> = cands.iter().map(|(f, _, _)| f.clone()).collect();
                let pick = self.prefer_home(file, files);
                cands
                    .into_iter()
                    .find(|(f, _, _)| pick.contains(f))
                    .map(|(f, _, ret)| self.resolve_type(f, &TypeRef::parse(ret), Some(on), hops + 1))
                    .unwrap_or(Ty::Unknown)
            }
            Ty::Traits(trs) => rows
                .iter()
                .find(|(f, o, _)| trs.iter().any(|(t, tf)| *o == format!("trait:{t}") && f == tf))
                .map(|(f, _, ret)| self.resolve_type(f, &TypeRef::parse(ret), Some(on), hops + 1))
                .unwrap_or(Ty::Unknown),
            _ => Ty::Unknown,
        }
    }

    /// The return type of the function a call path names, called from `file`.
    fn call_return(&self, file: &str, segs: &[String], self_ty: Option<&Ty>, hops: usize) -> Ty {
        let imports = self.imports_of(file);
        let free_return = |f: &str, n: &str| {
            self.fn_returns
                .get(n)
                .and_then(|rows| rows.iter().find(|(rf, o, _)| rf == f && o.is_empty()))
                .map(|(rf, _, ret)| self.resolve_type(rf, &TypeRef::parse(ret), None, hops + 1))
        };
        let member = |ty: &str, tfile: &str, n: &str| -> Ty {
            let me = Ty::Workspace { name: ty.to_string(), file: tfile.to_string() };
            match self.method_return(&me, n, hops) {
                Ty::Unknown if constructor_like(n) || self.type_kind(tfile, ty) == Some("Enum") => me,
                other => other,
            }
        };
        if segs.len() == 1 {
            let name = &segs[0];
            if let Some((f, canonical)) = self.rust_bare_type_identity(file, imports, name)
                .filter(|(f, n)| self.type_kind(f, n).is_some()) {
                // A tuple struct constructor `Foo(…)`.
                return Ty::Workspace { name: canonical, file: f };
            }
            return match self.rust_bare_call(file, imports, name) {
                Outcome::Files { targets, .. } if targets.len() == 1 => {
                    let (f, n) = targets.into_iter().next().unwrap();
                    free_return(&f, n.as_deref().unwrap_or(name)).unwrap_or(Ty::Unknown)
                }
                Outcome::Local => free_return(file, name).unwrap_or(Ty::Unknown),
                _ => Ty::Unknown,
            };
        }
        if segs[0] == "Self"
            && segs.len() == 2
            && let Some(Ty::Workspace { name, file: tf }) = self_ty
        {
            return member(name, tf, &segs[1]);
        }
        match self.resolve_rust_path(file, segs, Some(imports), 0) {
            Some(RustRes::Item { file: f, name }) => match self.type_kind(&f, &name) {
                Some(_) => Ty::Workspace { name, file: f },
                None => free_return(&f, &name).unwrap_or(Ty::Unknown),
            },
            Some(RustRes::Member { ty, file: tf, name }) => member(&ty, &tf, &name),
            Some(RustRes::Module(_)) => Ty::Unknown,
            None if segs.len() == 2 => {
                let (ty, n) = (&segs[0], &segs[1]);
                match self.resolve_type(file, &TypeRef::Path { segs: vec![ty.clone()], args: Vec::new() }, self_ty, hops + 1) {
                    Ty::Workspace { name, file: tf } => member(&name, &tf, n),
                    ext @ Ty::External { .. } if constructor_like(n) => ext,
                    _ => Ty::Unknown,
                }
            }
            None => Ty::Unknown,
        }
    }

    /// Evaluate a receiver written in `file`.
    pub(super) fn eval_receiver(&self, file: &str, e: &ReceiverExpr) -> Ty {
        let mut ty = match &e.root {
            Root::Typed(t) => self.resolve_type(file, t, None, 0),
            Root::CallPath(segs) => self.call_return(file, segs, None, 0),
            Root::StructLit(segs) => self.resolve_type(file, &TypeRef::Path { segs: segs.clone(), args: Vec::new() }, None, 0),
            Root::Literal(kind) => Ty::External { name: kind.clone(), args: Vec::new(), ctx: file.to_string(), self_ty: None },
            Root::Path(_) | Root::Unknown => Ty::Unknown,
        };
        for step in &e.steps {
            ty = self.apply_step(ty, step);
            if ty == Ty::Unknown {
                break;
            }
        }
        ty
    }

    /// Deref a lock guard to what it guards.
    fn deref_guard(&self, ty: Ty) -> Ty {
        match ty {
            Ty::External { name, args, ctx, self_ty } if name == "Guard" => match args.first() {
                Some(inner) => self.resolve_type(&ctx, inner, self_ty.as_deref(), 1),
                None => Ty::Unknown,
            },
            other => other,
        }
    }

    fn apply_step(&self, ty: Ty, step: &Step) -> Ty {
        // A guard (std `lock()` → `Result<MutexGuard<T>>`, tokio `lock()` →
        // `MutexGuard<T>`): unwrapping it or using it reaches `T`.
        if let Ty::External { name, .. } = &ty
            && name == "Guard"
        {
            return match step {
                Step::Try | Step::Await | Step::Deref | Step::Ref => {
                    if matches!(step, Step::Await) { ty } else { self.deref_guard(ty) }
                }
                Step::Method(m) if matches!(m.as_str(), "unwrap" | "expect" | "unwrap_or_else") => self.deref_guard(ty),
                _ => {
                    let inner = self.deref_guard(ty);
                    self.apply_step(inner, step)
                }
            };
        }
        match step {
            Step::Await | Step::Deref | Step::Ref => ty,
            Step::Field(f) => match &ty {
                Ty::Workspace { name, file } => self
                    .type_fields
                    .get(&(file.clone(), name.clone()))
                    .and_then(|fields| fields.get(f))
                    .map(|text| self.resolve_type(file, &TypeRef::parse(text), Some(&ty), 1))
                    .unwrap_or(Ty::Unknown),
                _ => Ty::Unknown,
            },
            Step::Method(m) => match &ty {
                Ty::Workspace { .. } | Ty::Traits(_) => self.method_return(&ty, m, 0),
                Ty::External { name, args, ctx, self_ty } => match std_return(name, m) {
                    Some(StdReturn::Arg(i)) => args
                        .get(i)
                        .map(|a| self.resolve_type(ctx, a, self_ty.as_deref(), 1))
                        .unwrap_or(Ty::Unknown),
                    Some(StdReturn::Same) => ty.clone(),
                    Some(StdReturn::OptionOfArg(i)) => match args.get(i) {
                        Some(a) => Ty::External {
                            name: "Option".into(),
                            args: vec![a.clone()],
                            ctx: ctx.clone(),
                            self_ty: self_ty.clone(),
                        },
                        None => Ty::Unknown,
                    },
                    Some(StdReturn::Guard) => match args.first() {
                        Some(a) => Ty::External {
                            name: "Guard".into(),
                            args: vec![a.clone()],
                            ctx: ctx.clone(),
                            self_ty: self_ty.clone(),
                        },
                        None => Ty::Unknown,
                    },
                    None => Ty::Unknown,
                },
                Ty::Unknown => Ty::Unknown,
            },
            Step::Try => match &ty {
                Ty::External { name, args, ctx, self_ty } if name == "Result" || name == "Option" => args
                    .first()
                    .map(|a| self.resolve_type(ctx, a, self_ty.as_deref(), 1))
                    .unwrap_or(Ty::Unknown),
                _ => Ty::Unknown,
            },
            Step::Index => match &ty {
                Ty::External { name, args, ctx, self_ty } => std_index_arg(name)
                    .and_then(|i| args.get(i))
                    .map(|a| self.resolve_type(ctx, a, self_ty.as_deref(), 1))
                    .unwrap_or(Ty::Unknown),
                _ => Ty::Unknown,
            },
        }
    }

    /// Whether trait `tr` (defined in `tfile`) is usable from `file`.
    fn trait_in_scope(&self, file: &str, tr: &str, tfile: &str) -> bool {
        tfile == file
            || self.imports_of(file).bindings.contains_key(tr)
            || self.crate_of_file(tfile).is_some() && self.crate_of_file(tfile) == self.crate_of_file(file)
    }

    /// Resolve `.m()` on a typed receiver. `None` when the receiver's type
    /// is unknown (the caller falls back to the name-based steps).
    pub(super) fn typed_method(&self, file: &str, ty: &Ty, m: &str) -> Option<Outcome> {
        let ty = self.deref_guard(ty.clone());
        let name_target = |files: BTreeSet<String>, step: &'static str| {
            files_of(step, files.into_iter().map(|f| (f, Some(m.to_string()))))
        };
        match &ty {
            Ty::Workspace { name, file: tfile } => {
                let inherent = self.impl_method_files(name, tfile, m, true);
                if !inherent.is_empty() {
                    return name_target(self.prefer_home(tfile, inherent), "rust_receiver_inherent");
                }
                let trait_impl = self.impl_method_files(name, tfile, m, false);
                if !trait_impl.is_empty() {
                    return name_target(self.prefer_home(tfile, trait_impl), "rust_receiver_trait_impl");
                }
                let traits = self.traits_implemented(name, tfile);
                let defaults: BTreeSet<String> = self
                    .trait_methods
                    .get(m)
                    .into_iter()
                    .flatten()
                    .filter(|(t, tf)| traits.iter().any(|(it, itf)| it == t && itf == tf))
                    .map(|(_, tf)| tf.clone())
                    .collect();
                if !defaults.is_empty() {
                    return name_target(defaults, "rust_receiver_trait_default");
                }
                Some(Outcome::NoEdge { reason: "typed_no_method" })
            }
            Ty::Traits(trs) => {
                let decl: BTreeSet<String> = self
                    .trait_methods
                    .get(m)
                    .into_iter()
                    .flatten()
                    .filter(|(t, tf)| trs.iter().any(|(bt, btf)| bt == t && btf == tf))
                    .map(|(_, tf)| tf.clone())
                    .collect();
                if decl.is_empty() { None } else { name_target(decl, "rust_receiver_bound") }
            }
            Ty::External { name, .. } => {
                let mut ext = BTreeSet::new();
                // `impl Ext for Vec<u8> { fn ext(..) }` records `impl:Vec:Ext`
                // even when the trait's own declaration of `ext` shares the
                // file (and therefore the node) with it.
                let external_matches = |ctx: &str, written: &str| {
                    matches!(self.resolve_type(ctx, &TypeRef::parse(written), None, 0),
                        Ty::External { name: n, .. } if n == *name)
                };
                for (_, (ifile, tr)) in self.impl_methods.iter()
                    .filter(|((_, method), _)| method == m)
                    .flat_map(|((written, _), rows)| rows.iter().map(move |row| (written, row)))
                    .filter(|(written, (f, _))| external_matches(f, written))
                {
                    if let Some(tr) = tr
                        && self.trait_in_scope(file, tr, ifile)
                    {
                        ext.insert(ifile.clone());
                    }
                }
                for (ifile, tr) in self.type_impls.iter()
                    .flat_map(|(written, rows)| rows.iter().filter(|(f, _)| external_matches(f, written)))
                {
                    if tr.is_empty() {
                        continue;
                    }
                    let segs: Vec<String> = tr.split("::").map(str::to_string).collect();
                    let Some((tname, tfile)) = self.resolve_trait(ifile, &segs) else { continue };
                    let declares = self
                        .trait_methods
                        .get(m)
                        .is_some_and(|v| v.iter().any(|(t, tf)| *t == tname && *tf == tfile));
                    if declares && self.trait_in_scope(file, &tname, &tfile) {
                        ext.insert(tfile);
                    }
                }
                if ext.is_empty() {
                    Some(Outcome::NoEdge { reason: "external_receiver" })
                } else {
                    name_target(ext, "rust_receiver_extension")
                }
            }
            Ty::Unknown => None,
        }
    }
}
