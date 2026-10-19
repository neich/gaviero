//! Syntactic types for receiver typing (receiver-typing plan R2–R5).
//!
//! [`TypeRef`] is a type as *written* (`Arc<Mutex<crate::a::Foo>>`,
//! `&dyn Observer`), built from tree-sitter type nodes and persisted as
//! compact text. [`ReceiverExpr`] is how a method call's receiver was
//! written (`self.state.lock().unwrap()`), with locals already replaced by
//! their annotation or initialiser. Neither knows what a name *means*:
//! [`super::resolve`] resolves them against the workspace's type facts.

use std::fmt;

/// Nesting deeper than this degrades to [`TypeRef::Unknown`].
const MAX_TYPE_DEPTH: usize = 32;

/// A type as written.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeRef {
    /// `a::b::Foo<Args…>`; primitives are one-segment paths.
    Path { segs: Vec<String>, args: Vec<TypeRef> },
    /// `&T` / `&mut T` / `*const T` (receivers auto-deref through them).
    Ref(Box<TypeRef>),
    /// `dyn A + B`: the trait paths.
    Dyn(Vec<Vec<String>>),
    /// `impl A + B`, or a generic parameter replaced by its bounds.
    Impl(Vec<Vec<String>>),
    Tuple(Vec<TypeRef>),
    /// `[T; N]` and `[T]`.
    Array(Box<TypeRef>),
    /// `Self`.
    SelfType,
    Unknown,
}

impl TypeRef {
    pub fn path(segs: &[&str]) -> Self {
        Self::Path {
            segs: segs.iter().map(|s| s.to_string()).collect(),
            args: Vec::new(),
        }
    }

    /// Build from a tree-sitter type node. Lifetimes and associated-type
    /// bindings (`Item = u8`) are dropped; unsupported forms are `Unknown`.
    pub fn from_node(node: tree_sitter::Node, source: &[u8]) -> Self {
        Self::from_node_at(node, source, 0)
    }

    fn from_node_at(node: tree_sitter::Node, source: &[u8], depth: usize) -> Self {
        if depth > MAX_TYPE_DEPTH {
            return Self::Unknown;
        }
        let text = |n: tree_sitter::Node| n.utf8_text(source).unwrap_or("").to_string();
        match node.kind() {
            "type_identifier" | "primitive_type" => {
                let t = text(node);
                if t == "Self" {
                    Self::SelfType
                } else {
                    Self::Path { segs: vec![t], args: Vec::new() }
                }
            }
            "scoped_type_identifier" | "scoped_identifier" => Self::Path {
                segs: super::edges::path_segments(&text(node)),
                args: Vec::new(),
            },
            "generic_type" | "generic_type_with_turbofish" => {
                let base = node
                    .child_by_field_name("type")
                    .map(|t| Self::from_node_at(t, source, depth + 1))
                    .unwrap_or(Self::Unknown);
                let mut args = Vec::new();
                if let Some(list) = node.child_by_field_name("type_arguments") {
                    let mut c = list.walk();
                    for a in list.named_children(&mut c) {
                        if !matches!(a.kind(), "lifetime" | "type_binding" | "trait_bounds") {
                            args.push(Self::from_node_at(a, source, depth + 1));
                        }
                    }
                }
                match base {
                    Self::Path { segs, .. } => Self::Path { segs, args },
                    Self::SelfType => Self::SelfType,
                    _ => Self::Unknown,
                }
            }
            "reference_type" | "pointer_type" => node
                .child_by_field_name("type")
                .map(|t| Self::Ref(Box::new(Self::from_node_at(t, source, depth + 1))))
                .unwrap_or(Self::Unknown),
            "dynamic_type" | "abstract_type" => {
                let traits = trait_paths(node, source);
                if traits.is_empty() {
                    Self::Unknown
                } else if node.kind() == "dynamic_type" {
                    Self::Dyn(traits)
                } else {
                    Self::Impl(traits)
                }
            }
            "bounded_type" => {
                // `dyn A + Send` parses as a bounded type around the dyn.
                let mut c = node.walk();
                let kids: Vec<_> = node.named_children(&mut c).collect();
                let Some((first, rest)) = kids.split_first() else {
                    return Self::Unknown;
                };
                let extra: Vec<Vec<String>> = rest.iter().flat_map(|k| trait_paths(*k, source)).collect();
                match Self::from_node_at(*first, source, depth + 1) {
                    Self::Dyn(mut ts) => {
                        ts.extend(extra);
                        Self::Dyn(ts)
                    }
                    Self::Impl(mut ts) => {
                        ts.extend(extra);
                        Self::Impl(ts)
                    }
                    other => other,
                }
            }
            "tuple_type" => {
                let mut c = node.walk();
                Self::Tuple(
                    node.named_children(&mut c)
                        .map(|t| Self::from_node_at(t, source, depth + 1))
                        .collect(),
                )
            }
            "array_type" => node
                .child_by_field_name("element")
                .map(|t| Self::Array(Box::new(Self::from_node_at(t, source, depth + 1))))
                .unwrap_or(Self::Unknown),
            "unit_type" => Self::Tuple(Vec::new()),
            _ => Self::Unknown,
        }
    }

    /// Parse compact text (the [`fmt::Display`] form). Malformed input
    /// yields `Unknown` (or an `Unknown` component), never a panic.
    pub fn parse(text: &str) -> Self {
        let mut p = Parser { s: text.as_bytes(), i: 0, failed: false };
        let t = p.ty(0);
        p.ws();
        if !p.failed && p.i == p.s.len() { t } else { Self::Unknown }
    }

    /// Last path segment, if this is a path.
    pub fn last_seg(&self) -> Option<&str> {
        match self {
            Self::Path { segs, .. } => segs.last().map(String::as_str),
            _ => None,
        }
    }

    /// Replace generic parameters (`G` → its bounds, or `Unknown` without
    /// any) and, when given, `Self`.
    pub fn substitute(&self, generics: &dyn Fn(&str) -> Option<Vec<Vec<String>>>, self_ty: Option<&TypeRef>) -> Self {
        self.substitute_at(generics, self_ty, 0)
    }

    fn substitute_at(
        &self,
        generics: &dyn Fn(&str) -> Option<Vec<Vec<String>>>,
        self_ty: Option<&TypeRef>,
        depth: usize,
    ) -> Self {
        if depth > MAX_TYPE_DEPTH {
            return Self::Unknown;
        }
        match self {
            Self::Path { segs, args } => {
                if segs.len() == 1 && args.is_empty()
                    && let Some(bounds) = generics(&segs[0])
                {
                    return if bounds.is_empty() { Self::Unknown } else { Self::Impl(bounds) };
                }
                Self::Path {
                    segs: segs.clone(),
                    args: args.iter().map(|a| a.substitute_at(generics, self_ty, depth + 1)).collect(),
                }
            }
            Self::Ref(t) => Self::Ref(Box::new(t.substitute_at(generics, self_ty, depth + 1))),
            Self::Array(t) => Self::Array(Box::new(t.substitute_at(generics, self_ty, depth + 1))),
            Self::Tuple(ts) => Self::Tuple(ts.iter().map(|t| t.substitute_at(generics, self_ty, depth + 1)).collect()),
            Self::SelfType => self_ty.cloned().unwrap_or(Self::SelfType),
            other => other.clone(),
        }
    }
}

/// The trait paths named by a `dyn` / `impl` type or a bounds list.
pub fn trait_paths(node: tree_sitter::Node, source: &[u8]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    let mut seen = 0;
    while let Some(n) = stack.pop() {
        seen += 1;
        if seen > 64 {
            break;
        }
        match n.kind() {
            "type_identifier" | "scoped_type_identifier" => {
                let segs = super::edges::path_segments(n.utf8_text(source).unwrap_or(""));
                if !segs.is_empty() {
                    out.push(segs);
                }
            }
            "generic_type" => {
                if let Some(t) = n.child_by_field_name("type") {
                    stack.push(t);
                }
            }
            "lifetime" | "type_arguments" | "higher_ranked_trait_bound" | "removed_trait_bound" => {}
            _ => {
                let mut c = n.walk();
                let kids: Vec<_> = n.named_children(&mut c).collect();
                stack.extend(kids.into_iter().rev());
            }
        }
    }
    out
}

impl fmt::Display for TypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |f: &mut fmt::Formatter<'_>, ts: &[TypeRef]| -> fmt::Result {
            for (i, t) in ts.iter().enumerate() {
                if i > 0 {
                    f.write_str(",")?;
                }
                write!(f, "{t}")?;
            }
            Ok(())
        };
        let traits = |f: &mut fmt::Formatter<'_>, ps: &[Vec<String>]| -> fmt::Result {
            for (i, p) in ps.iter().enumerate() {
                if i > 0 {
                    f.write_str("+")?;
                }
                f.write_str(&p.join("::"))?;
            }
            Ok(())
        };
        match self {
            Self::Path { segs, args } => {
                f.write_str(&segs.join("::"))?;
                if !args.is_empty() {
                    f.write_str("<")?;
                    list(f, args)?;
                    f.write_str(">")?;
                }
                Ok(())
            }
            Self::Ref(t) => write!(f, "&{t}"),
            Self::Dyn(ps) => {
                f.write_str("dyn ")?;
                traits(f, ps)
            }
            Self::Impl(ps) => {
                f.write_str("impl ")?;
                traits(f, ps)
            }
            Self::Tuple(ts) => {
                f.write_str("(")?;
                list(f, ts)?;
                f.write_str(")")
            }
            Self::Array(t) => write!(f, "[{t}]"),
            Self::SelfType => f.write_str("Self"),
            Self::Unknown => f.write_str("_"),
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    failed: bool,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        self.ws();
        if self.s.get(self.i..).is_some_and(|rest| rest.starts_with(lit.as_bytes())) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn ident(&mut self) -> Option<String> {
        self.ws();
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_') {
            self.i += 1;
        }
        (self.i > start).then(|| String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }

    fn path(&mut self) -> Option<Vec<String>> {
        let mut segs = vec![self.ident()?];
        while self.eat("::") {
            segs.push(self.ident()?);
        }
        Some(segs)
    }

    fn traits(&mut self) -> Option<Vec<Vec<String>>> {
        let mut out = vec![self.path()?];
        while self.eat("+") {
            out.push(self.path()?);
        }
        Some(out)
    }

    fn list(&mut self, close: &str, depth: usize) -> Option<Vec<TypeRef>> {
        let mut out = Vec::new();
        if self.eat(close) {
            return Some(out);
        }
        loop {
            out.push(self.ty(depth + 1));
            if self.failed {
                return None;
            }
            if self.eat(close) {
                return Some(out);
            }
            if !self.eat(",") {
                return None;
            }
        }
    }

    fn fail(&mut self) -> TypeRef {
        self.failed = true;
        self.i = self.s.len();
        TypeRef::Unknown
    }

    fn ty(&mut self, depth: usize) -> TypeRef {
        if depth > MAX_TYPE_DEPTH || self.failed {
            return self.fail();
        }
        if self.eat("&") {
            return TypeRef::Ref(Box::new(self.ty(depth + 1)));
        }
        if self.eat("dyn ") {
            return match self.traits() {
                Some(t) => TypeRef::Dyn(t),
                None => self.fail(),
            };
        }
        if self.eat("impl ") {
            return match self.traits() {
                Some(t) => TypeRef::Impl(t),
                None => self.fail(),
            };
        }
        if self.eat("(") {
            return match self.list(")", depth) {
                Some(t) => TypeRef::Tuple(t),
                None => self.fail(),
            };
        }
        if self.eat("[") {
            let t = self.ty(depth + 1);
            return if self.eat("]") { TypeRef::Array(Box::new(t)) } else { self.fail() };
        }
        let lone_underscore = self.s.get(self.i) == Some(&b'_')
            && !self
                .s
                .get(self.i + 1)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_');
        if lone_underscore {
            self.i += 1;
            return TypeRef::Unknown;
        }
        let Some(segs) = self.path() else {
            return self.fail();
        };
        if segs == ["Self"] {
            return TypeRef::SelfType;
        }
        let args = if self.eat("<") {
            match self.list(">", depth) {
                Some(a) => a,
                None => return self.fail(),
            }
        } else {
            Vec::new()
        };
        TypeRef::Path { segs, args }
    }
}

// ── Receiver expressions ─────────────────────────────────────────────

/// Where a receiver chain starts.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Root {
    /// A value of a written type: an annotated local or parameter, `self`
    /// (the impl's type), a cast.
    Typed(TypeRef),
    /// The result of calling `a::b::f(…)` / `Type::new(…)` / `f(…)`.
    CallPath(Vec<String>),
    /// A bare path that is not a known local (a `const` / `static`).
    Path(Vec<String>),
    /// `Type { … }`.
    StructLit(Vec<String>),
    /// A literal of a std type: `str`, `i32`, `f64`, `bool`, `char`, `Vec`, `String`, `array`.
    Literal(String),
    Unknown,
}

/// One step applied to the value so far.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Step {
    Field(String),
    /// The result of calling this method.
    Method(String),
    Try,
    Await,
    Deref,
    Ref,
    Index,
}

/// How a receiver chain's root was reached (report shape only; not part
/// of the text form).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Origin {
    /// Written in place (`Foo::new().m()`, `"x".m()`).
    #[default]
    Direct,
    /// `self`.
    SelfValue,
    /// A local or parameter with a written type.
    LocalTyped,
    /// A local replaced by its initialiser.
    LocalInit,
}

/// How a receiver was written, locals substituted.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReceiverExpr {
    pub root: Root,
    pub steps: Vec<Step>,
    pub origin: Origin,
}

/// Chains longer than this are not followed.
pub const MAX_RECEIVER_STEPS: usize = 16;

impl ReceiverExpr {
    pub fn unknown() -> Self {
        Self { root: Root::Unknown, steps: Vec::new(), origin: Origin::Direct }
    }

    pub fn is_unknown(&self) -> bool {
        self.root == Root::Unknown
    }

    /// `self` + steps appended (for substitution). Too long → unknown.
    pub fn then(mut self, steps: impl IntoIterator<Item = Step>) -> Self {
        self.steps.extend(steps);
        if self.steps.len() > MAX_RECEIVER_STEPS { Self::unknown() } else { self }
    }

    /// Shape bucket for the report: `self`, `self.chain`, `local typed`,
    /// `local init`, `call result`, `struct literal`, `literal`, `cast`,
    /// `path`, `unknown`.
    pub fn shape(&self) -> &'static str {
        match (&self.root, self.origin) {
            (Root::Unknown, _) => "unknown",
            (_, Origin::SelfValue) if self.steps.is_empty() => "self",
            (_, Origin::SelfValue) => "self.chain",
            (_, Origin::LocalTyped) => "local typed",
            (_, Origin::LocalInit) => "local init",
            (Root::Typed(_), _) => "cast",
            (Root::CallPath(_), _) => "call result",
            (Root::StructLit(_), _) => "struct literal",
            (Root::Literal(_), _) => "literal",
            (Root::Path(_), _) => "path",
        }
    }

    /// Parse the [`fmt::Display`] form. Malformed input → unknown.
    pub fn parse(text: &str) -> Self {
        let mut parts = text.split(" | ");
        let Some(root) = parts.next() else { return Self::unknown() };
        let root = if let Some(t) = root.strip_prefix("typed ") {
            Root::Typed(TypeRef::parse(t))
        } else if let Some(p) = root.strip_prefix("call ") {
            Root::CallPath(p.split("::").map(str::to_string).collect())
        } else if let Some(p) = root.strip_prefix("path ") {
            Root::Path(p.split("::").map(str::to_string).collect())
        } else if let Some(p) = root.strip_prefix("struct ") {
            Root::StructLit(p.split("::").map(str::to_string).collect())
        } else if let Some(l) = root.strip_prefix("lit ") {
            Root::Literal(l.to_string())
        } else {
            return Self::unknown();
        };
        let mut steps = Vec::new();
        for p in parts {
            steps.push(match p {
                "?" => Step::Try,
                ".await" => Step::Await,
                "*" => Step::Deref,
                "&" => Step::Ref,
                "[]" => Step::Index,
                s if s.starts_with('.') && s.ends_with("()") => Step::Method(s[1..s.len() - 2].to_string()),
                s if s.starts_with('.') => Step::Field(s[1..].to_string()),
                _ => return Self::unknown(),
            });
        }
        Self { root, steps, origin: Origin::Direct }
    }
}

impl Default for ReceiverExpr {
    fn default() -> Self {
        Self::unknown()
    }
}

impl fmt::Display for ReceiverExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.root {
            Root::Typed(t) => write!(f, "typed {t}")?,
            Root::CallPath(p) => write!(f, "call {}", p.join("::"))?,
            Root::Path(p) => write!(f, "path {}", p.join("::"))?,
            Root::StructLit(p) => write!(f, "struct {}", p.join("::"))?,
            Root::Literal(l) => write!(f, "lit {l}")?,
            Root::Unknown => f.write_str("unknown")?,
        }
        for s in &self.steps {
            match s {
                Step::Field(n) => write!(f, " | .{n}")?,
                Step::Method(n) => write!(f, " | .{n}()")?,
                Step::Try => f.write_str(" | ?")?,
                Step::Await => f.write_str(" | .await")?,
                Step::Deref => f.write_str(" | *")?,
                Step::Ref => f.write_str(" | &")?,
                Step::Index => f.write_str(" | []")?,
            }
        }
        Ok(())
    }
}

// ── Std model (hand-written; every entry has a test) ─────────────────

/// Smart pointers and guards a method call auto-derefs through: a method
/// on `Arc<T>` that `Arc` does not define is `T`'s.
pub const TRANSPARENT_WRAPPERS: &[&str] = &[
    "Box", "Arc", "Rc", "Pin", "Cow", "MutexGuard", "RwLockReadGuard", "RwLockWriteGuard",
    "Ref", "RefMut", "OwnedMutexGuard", "MappedMutexGuard",
];

/// Prelude / always-std type names: a receiver of this type is external
/// unless the workspace defines (and the file sees) a type of that name.
pub const STD_TYPE_NAMES: &[&str] = &[
    "Option", "Result", "Vec", "String", "str", "Box", "Arc", "Rc", "Mutex", "RwLock", "RefCell", "Cell",
    "HashMap", "HashSet", "BTreeMap", "BTreeSet", "VecDeque", "PathBuf", "Path", "Instant", "Duration",
    "SystemTime", "OsString", "OsStr", "Cow", "Pin", "Weak", "Sender", "Receiver", "UnboundedSender",
    "UnboundedReceiver", "JoinHandle", "File", "Command", "Child", "AtomicBool", "AtomicUsize", "AtomicU64",
    "AtomicI64", "AtomicU32", "bool", "char", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32",
    "i64", "i128", "isize", "f32", "f64",
];

/// What a std method returns, for the few methods a receiver chain passes
/// through on its way to a workspace type. `receiver` is the receiver's
/// last path segment; `args` its type arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdReturn {
    /// The receiver's type argument `i` (`Option<X>::unwrap` → `X`).
    Arg(usize),
    /// The receiver itself (`clone`, `as_ref`, …).
    Same,
    /// `Option<receiver arg i>` (`HashMap::get`, `Vec::first`, `Result::ok`).
    OptionOfArg(usize),
    /// A lock guard over arg 0 that may or may not be wrapped in a
    /// `Result` (std vs tokio `lock()`); unwrapping or using it reaches
    /// arg 0.
    Guard,
}

/// The std transform table (receiver-typing plan 5.6).
pub fn std_return(receiver: &str, method: &str) -> Option<StdReturn> {
    use StdReturn::*;
    Some(match (receiver, method) {
        ("Option" | "Result", "unwrap" | "expect" | "unwrap_or_default" | "unwrap_or" | "unwrap_or_else" | "unwrap_unchecked") => Arg(0),
        ("Result", "ok") => OptionOfArg(0),
        ("Option", "as_ref" | "as_mut" | "as_deref" | "as_deref_mut" | "take" | "cloned" | "copied") => Same,
        ("Mutex" | "RwLock", "lock" | "read" | "write" | "try_lock" | "blocking_lock" | "blocking_read" | "blocking_write") => Guard,
        ("RefCell", "borrow" | "borrow_mut") => Arg(0),
        ("HashMap" | "BTreeMap", "get" | "get_mut") => OptionOfArg(1),
        ("Vec" | "VecDeque" | "Array", "get" | "get_mut" | "first" | "last" | "first_mut" | "last_mut" | "front" | "back" | "pop" | "pop_front" | "pop_back") => OptionOfArg(0),
        (_, "clone" | "to_owned" | "as_ref" | "as_mut" | "by_ref" | "borrow" | "borrow_mut") => Same,
        _ => return None,
    })
}

/// The element type index for `x[i]` on a std container (`Vec<T>` → 0,
/// `HashMap<K, V>` → 1).
pub fn std_index_arg(receiver: &str) -> Option<usize> {
    match receiver {
        "Vec" | "VecDeque" | "Array" => Some(0),
        "HashMap" | "BTreeMap" => Some(1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust_type(src: &str) -> TypeRef {
        // Wrap in a type alias and take its `type` field.
        let code = format!("type X = {src};");
        let lang = crate::tree_sitter::language_for_extension("rs").unwrap();
        let mut p = tree_sitter::Parser::new();
        p.set_language(&lang).unwrap();
        let tree = p.parse(&code, None).unwrap();
        let item = tree.root_node().named_child(0).unwrap();
        TypeRef::from_node(item.child_by_field_name("type").unwrap(), code.as_bytes())
    }

    #[test]
    fn types_round_trip_through_text() {
        for (src, text) in [
            ("Arc<Mutex<crate::a::Foo>>", "Arc<Mutex<crate::a::Foo>>"),
            ("&mut Vec<u8>", "&Vec<u8>"),
            ("&'a dyn Observer", "&dyn Observer"),
            ("Box<dyn Observer + Send>", "Box<dyn Observer+Send>"),
            ("impl Iterator<Item = u8>", "impl Iterator"),
            ("Result<Self, anyhow::Error>", "Result<Self,anyhow::Error>"),
            ("(A, B)", "(A,B)"),
            ("()", "()"),
            ("[u8; 4]", "[u8]"),
            ("&[String]", "&[String]"),
            ("fn(u8) -> u8", "_"),
            ("Self", "Self"),
            ("HashMap<String, Vec<Foo>>", "HashMap<String,Vec<Foo>>"),
        ] {
            let t = rust_type(src);
            assert_eq!(t.to_string(), text, "{src}");
            assert_eq!(TypeRef::parse(text), t, "{text} parses back");
        }
    }

    #[test]
    fn malformed_type_text_is_unknown_and_never_panics() {
        for bad in ["", "<", "Foo<", "Foo<Bar", "&", "dyn ", "Foo::", "[u8", "a b", "Foo<,>", "((((", "Foo>"] {
            let _ = TypeRef::parse(bad);
        }
        for bad in ["Foo<", "Foo<Bar", "a b", "Foo>", "Foo<,>"] {
            assert_eq!(TypeRef::parse(bad), TypeRef::Unknown, "{bad:?}");
        }
        let deep = "Box<".repeat(10_000) + "u8" + &">".repeat(10_000);
        assert_eq!(TypeRef::parse(&deep), TypeRef::Unknown, "deep nesting degrades");
    }

    #[test]
    fn generics_become_their_bounds_and_self_is_replaced() {
        let generics = |n: &str| match n {
            "G" => Some(vec![vec!["Observer".to_string()]]),
            "T" => Some(vec![]),
            _ => None,
        };
        let me = TypeRef::path(&["Widget"]);
        assert_eq!(
            TypeRef::parse("Vec<G>").substitute(&generics, Some(&me)).to_string(),
            "Vec<impl Observer>"
        );
        assert_eq!(TypeRef::parse("&T").substitute(&generics, None), TypeRef::Ref(Box::new(TypeRef::Unknown)));
        assert_eq!(TypeRef::parse("Option<Self>").substitute(&generics, Some(&me)).to_string(), "Option<Widget>");
    }

    #[test]
    fn receiver_exprs_round_trip() {
        let e = ReceiverExpr {
            root: Root::Typed(TypeRef::parse("Arc<Mutex<Foo>>")),
            steps: vec![
                Step::Field("state".into()),
                Step::Method("lock".into()),
                Step::Try,
                Step::Await,
                Step::Deref,
                Step::Ref,
                Step::Index,
            ],
            origin: Origin::Direct,
        };
        let text = e.to_string();
        assert_eq!(text, "typed Arc<Mutex<Foo>> | .state | .lock() | ? | .await | * | & | []");
        assert_eq!(ReceiverExpr::parse(&text), e);
        for root in [
            Root::CallPath(vec!["Foo".into(), "new".into()]),
            Root::Path(vec!["CONFIG".into()]),
            Root::StructLit(vec!["a".into(), "Foo".into()]),
            Root::Literal("str".into()),
        ] {
            let e = ReceiverExpr { root, steps: vec![Step::Method("m".into())], origin: Origin::Direct };
            assert_eq!(ReceiverExpr::parse(&e.to_string()), e);
        }
        assert!(ReceiverExpr::parse("garbage | x").is_unknown());
        assert!(ReceiverExpr::unknown().then((0..20).map(|_| Step::Try)).is_unknown(), "too long");
    }

    #[test]
    fn std_transform_table_entries() {
        use StdReturn::*;
        assert_eq!(std_return("Option", "unwrap"), Some(Arg(0)));
        assert_eq!(std_return("Result", "expect"), Some(Arg(0)));
        assert_eq!(std_return("Result", "ok"), Some(OptionOfArg(0)));
        assert_eq!(std_return("Mutex", "lock"), Some(Guard));
        assert_eq!(std_return("RwLock", "write"), Some(Guard));
        assert_eq!(std_return("RefCell", "borrow_mut"), Some(Arg(0)));
        assert_eq!(std_return("HashMap", "get"), Some(OptionOfArg(1)));
        assert_eq!(std_return("Vec", "first"), Some(OptionOfArg(0)));
        assert_eq!(std_return("Instant", "clone"), Some(Same));
        assert_eq!(std_return("Vec", "iter"), None, "iterators are not followed");
        assert_eq!(std_index_arg("Vec"), Some(0));
        assert_eq!(std_index_arg("HashMap"), Some(1));
        assert!(TRANSPARENT_WRAPPERS.contains(&"Arc") && !TRANSPARENT_WRAPPERS.contains(&"Mutex"));
    }
}
