use chumsky::input::ValueInput;
use chumsky::prelude::*;
use miette::NamedSource;

use crate::ast::*;
use crate::error::DslError;
use crate::lexer::Token;

// ── Internal field enums (defined inside parser functions) ─────────────────

#[derive(Debug)]
enum ClientField {
    Tier(TierLit, Span),
    Model(String, Span),
    Privacy(PrivacyLit, Span),
}

#[derive(Debug)]
enum AgentField {
    Description(String, Span),
    Client(String, Span),
    Scope(ScopeBlock),
    DependsOn(Vec<(String, Span)>, Span),
    Prompt(String, Span),
    MaxRetries(u8, Span),
    Memory(MemoryBlock),
}

#[derive(Debug)]
enum ScopeField {
    Owned(Vec<String>),
    ReadOnly(Vec<String>),
}

#[derive(Debug)]
enum WorkflowField {
    Steps(Vec<(String, Span)>, Span),
    MaxParallel(usize, Span),
    Memory(MemoryBlock),
}

#[derive(Debug)]
enum MemoryField {
    ReadNs(Vec<String>),
    WriteNs(String),
    Importance(f32),
    StalenessSources(Vec<String>),
}

// ── Parser builder ─────────────────────────────────────────────────────────

fn script_parser<'src, I>() -> impl Parser<'src, I, Script, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>,
{
    // ── Primitives ───────────────────────────────────────────────

    let ident = select! { Token::Ident(s) => s };

    let string = select! {
        Token::Str(s)    => s,
        Token::RawStr(s) => s,
    };

    let tier_lit = select! {
        Token::TierCoordinator => TierLit::Coordinator,
        Token::TierReasoning   => TierLit::Reasoning,
        Token::TierExecution   => TierLit::Execution,
        Token::TierMechanical  => TierLit::Mechanical,
    };

    let privacy_lit = select! {
        Token::PrivPublic    => PrivacyLit::Public,
        Token::PrivLocalOnly => PrivacyLit::LocalOnly,
    };

    let integer = select! { Token::Int(n) => n };
    let float_lit = select! { Token::Float(v) => v };

    // ── String list: [ "a" "b" ... ] ─────────────────────────────

    let str_list = string
        .repeated()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LBracket), just(Token::RBracket));

    // ── Ident list with spans: [ foo bar ... ] ───────────────────

    let ident_list = ident
        .map_with(|s, e| (s, e.span()))
        .repeated()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LBracket), just(Token::RBracket));

    // ── client declaration ────────────────────────────────────────

    let client_field = choice((
        just(Token::KwTier)
            .ignore_then(tier_lit.map_with(|v, e| (v, e.span())))
            .map(|(v, s)| ClientField::Tier(v, s)),
        just(Token::KwModel)
            .ignore_then(string.map_with(|s, e| (s, e.span())))
            .map(|(v, s)| ClientField::Model(v, s)),
        just(Token::KwPrivacy)
            .ignore_then(privacy_lit.map_with(|v, e| (v, e.span())))
            .map(|(v, s)| ClientField::Privacy(v, s)),
    ));

    let client_decl = just(Token::KwClient)
        .ignore_then(ident.map_with(|n, e| (n, e.span())))
        .then(
            client_field
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|((name, name_span), fields), e| {
            let mut tier = None;
            let mut model = None;
            let mut privacy = None;
            for f in fields {
                match f {
                    ClientField::Tier(v, s) => {
                        tier.get_or_insert((v, s));
                    }
                    ClientField::Model(v, s) => {
                        model.get_or_insert((v, s));
                    }
                    ClientField::Privacy(v, s) => {
                        privacy.get_or_insert((v, s));
                    }
                }
            }
            ClientDecl { name, name_span, tier, model, privacy, span: e.span() }
        });

    // ── scope block ───────────────────────────────────────────────

    let scope_field = choice((
        just(Token::KwOwned)
            .ignore_then(str_list.clone())
            .map(ScopeField::Owned),
        just(Token::KwReadOnly)
            .ignore_then(str_list.clone())
            .map(ScopeField::ReadOnly),
    ));

    let scope_block = just(Token::KwScope)
        .ignore_then(
            scope_field
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|fields, e| {
            let mut owned = Vec::new();
            let mut read_only = Vec::new();
            for f in fields {
                match f {
                    ScopeField::Owned(v) => owned.extend(v),
                    ScopeField::ReadOnly(v) => read_only.extend(v),
                }
            }
            ScopeBlock { owned, read_only, span: e.span() }
        });

    // ── memory block ──────────────────────────────────────────────

    let memory_field = choice((
        just(Token::KwReadNs)
            .ignore_then(str_list.clone())
            .map(MemoryField::ReadNs),
        just(Token::KwWriteNs)
            .ignore_then(string.clone())
            .map(MemoryField::WriteNs),
        just(Token::KwImportance)
            .ignore_then(float_lit)
            .map(MemoryField::Importance),
        just(Token::KwStalenessSources)
            .ignore_then(str_list.clone())
            .map(MemoryField::StalenessSources),
    ));

    let memory_block = just(Token::KwMemory)
        .ignore_then(
            memory_field
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|fields, e| {
            let mut read_ns = Vec::new();
            let mut write_ns = None;
            let mut importance = None;
            let mut staleness_sources = Vec::new();
            for f in fields {
                match f {
                    MemoryField::ReadNs(v) => read_ns.extend(v),
                    MemoryField::WriteNs(s) => { write_ns.get_or_insert(s); }
                    MemoryField::Importance(v) => { importance.get_or_insert(v); }
                    MemoryField::StalenessSources(v) => staleness_sources.extend(v),
                }
            }
            MemoryBlock { read_ns, write_ns, importance, staleness_sources, span: e.span() }
        });

    // ── agent declaration ─────────────────────────────────────────

    let agent_field = choice((
        just(Token::KwDescription)
            .ignore_then(string.map_with(|s, e| (s, e.span())))
            .map(|(v, s)| AgentField::Description(v, s)),
        just(Token::KwClient)
            .ignore_then(ident.map_with(|s, e| (s, e.span())))
            .map(|(v, s)| AgentField::Client(v, s)),
        scope_block.map(AgentField::Scope),
        just(Token::KwDependsOn)
            .ignore_then(ident_list.clone().map_with(|v, e| (v, e.span())))
            .map(|(v, s)| AgentField::DependsOn(v, s)),
        just(Token::KwPrompt)
            .ignore_then(string.map_with(|s, e| (s, e.span())))
            .map(|(v, s)| AgentField::Prompt(v, s)),
        just(Token::KwMaxRetries)
            .ignore_then(integer.map_with(|n, e| (n, e.span())))
            .map(|(n, s)| AgentField::MaxRetries(n.min(255) as u8, s)),
        memory_block.clone().map(AgentField::Memory),
    ));

    let agent_decl = just(Token::KwAgent)
        .ignore_then(ident.map_with(|n, e| (n, e.span())))
        .then(
            agent_field
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|((name, name_span), fields), e| {
            let mut description = None;
            let mut client = None;
            let mut scope = None;
            let mut depends_on = None;
            let mut prompt = None;
            let mut max_retries = None;
            let mut memory = None;
            for f in fields {
                match f {
                    AgentField::Description(v, s) => {
                        description.get_or_insert((v, s));
                    }
                    AgentField::Client(v, s) => {
                        client.get_or_insert((v, s));
                    }
                    AgentField::Scope(b) => {
                        scope.get_or_insert(b);
                    }
                    AgentField::DependsOn(v, s) => {
                        depends_on.get_or_insert((v, s));
                    }
                    AgentField::Prompt(v, s) => {
                        prompt.get_or_insert((v, s));
                    }
                    AgentField::MaxRetries(n, s) => {
                        max_retries.get_or_insert((n, s));
                    }
                    AgentField::Memory(b) => {
                        memory.get_or_insert(b);
                    }
                }
            }
            AgentDecl { name, name_span, description, client, scope, depends_on, prompt, max_retries, memory, span: e.span() }
        });

    // ── workflow declaration ──────────────────────────────────────

    let workflow_field = choice((
        just(Token::KwSteps)
            .ignore_then(ident_list.map_with(|v, e| (v, e.span())))
            .map(|(v, s)| WorkflowField::Steps(v, s)),
        just(Token::KwMaxParallel)
            .ignore_then(integer.map_with(|n, e| (n, e.span())))
            .map(|(n, s)| WorkflowField::MaxParallel(n as usize, s)),
        memory_block.map(WorkflowField::Memory),
    ));

    let workflow_decl = just(Token::KwWorkflow)
        .ignore_then(ident.map_with(|n, e| (n, e.span())))
        .then(
            workflow_field
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|((name, name_span), fields), e| {
            let mut steps = None;
            let mut max_parallel = None;
            let mut memory = None;
            for f in fields {
                match f {
                    WorkflowField::Steps(v, s) => {
                        steps.get_or_insert((v, s));
                    }
                    WorkflowField::MaxParallel(n, s) => {
                        max_parallel.get_or_insert((n, s));
                    }
                    WorkflowField::Memory(b) => {
                        memory.get_or_insert(b);
                    }
                }
            }
            WorkflowDecl { name, name_span, steps, max_parallel, memory, span: e.span() }
        });

    // ── top-level ─────────────────────────────────────────────────

    let item = choice((
        client_decl.map(Item::Client),
        agent_decl.map(Item::Agent),
        workflow_decl.map(Item::Workflow),
    ));

    item.repeated()
        .collect::<Vec<_>>()
        .then_ignore(end())
        .map(|items| Script { items })
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Parse a token stream into a [`Script`].
///
/// Returns `(Some(script), [])` on success.
/// Returns `(None, errors)` on failure.
/// May return `(Some(partial), errors)` if chumsky partially recovers.
pub fn parse(
    tokens: &[(Token, Span)],
    source: &str,
    filename: &str,
) -> (Option<Script>, Vec<DslError>) {
    let eoi = SimpleSpan::from(source.len()..source.len());
    let stream = tokens.split_token_span(eoi);

    let (ast, errors) = script_parser().parse(stream).into_output_errors();

    let dsl_errors = errors
        .into_iter()
        .map(|e| {
            let span = e.span();
            let offset = span.start;
            let len = span.end.saturating_sub(span.start).max(1);
            DslError::Parse {
                src: NamedSource::new(filename, source.to_string()),
                span: (offset, len).into(),
                reason: format!("{}", e),
            }
        })
        .collect();

    (ast, dsl_errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer;

    fn parse_str(src: &str) -> (Option<Script>, Vec<DslError>) {
        let (tokens, _) = lexer::lex(src);
        parse(&tokens, src, "test.gaviero")
    }

    #[test]
    fn empty_script() {
        let (ast, errs) = parse_str("");
        assert!(errs.is_empty());
        assert!(ast.is_some());
        assert_eq!(ast.unwrap().items.len(), 0);
    }

    #[test]
    fn client_decl_full() {
        let src = r#"client c { tier coordinator model "claude-opus-4-6" privacy public }"#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let ast = ast.unwrap();
        assert_eq!(ast.items.len(), 1);
        if let Item::Client(c) = &ast.items[0] {
            assert_eq!(c.name, "c");
            assert!(matches!(c.tier, Some((TierLit::Coordinator, _))));
            assert!(matches!(&c.model, Some((m, _)) if m == "claude-opus-4-6"));
            assert!(matches!(c.privacy, Some((PrivacyLit::Public, _))));
        } else {
            panic!("expected Client item");
        }
    }

    #[test]
    fn agent_decl_minimal() {
        let src = r#"agent foo { description "do stuff" }"#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        let ast = ast.unwrap();
        if let Item::Agent(a) = &ast.items[0] {
            assert_eq!(a.name, "foo");
            assert!(matches!(&a.description, Some((d, _)) if d == "do stuff"));
        } else {
            panic!("expected Agent");
        }
    }

    #[test]
    fn agent_with_scope_and_deps() {
        let src = r#"
            agent worker {
                scope { owned ["src/"] read_only ["docs/"] }
                depends_on [other]
                max_retries 3
            }
        "#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        let ast = ast.unwrap();
        if let Item::Agent(a) = &ast.items[0] {
            let scope = a.scope.as_ref().unwrap();
            assert_eq!(scope.owned, vec!["src/"]);
            assert_eq!(scope.read_only, vec!["docs/"]);
            let (deps, _) = a.depends_on.as_ref().unwrap();
            assert_eq!(deps[0].0, "other");
            assert!(matches!(a.max_retries, Some((3, _))));
        }
    }

    #[test]
    fn agent_with_raw_prompt() {
        let src = "#\"\nFix the bug.\n\"#";
        let full = format!("agent x {{ prompt {} }}", src);
        let (ast, errs) = parse_str(&full);
        assert!(errs.is_empty(), "{:?}", errs);
        let ast = ast.unwrap();
        if let Item::Agent(a) = &ast.items[0] {
            assert!(a.prompt.is_some());
        }
    }

    #[test]
    fn workflow_decl() {
        let src = r#"workflow wf { steps [a b] max_parallel 2 }"#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        let ast = ast.unwrap();
        if let Item::Workflow(w) = &ast.items[0] {
            assert_eq!(w.name, "wf");
            let (steps, _) = w.steps.as_ref().unwrap();
            assert_eq!(steps.len(), 2);
            assert!(matches!(w.max_parallel, Some((2, _))));
        }
    }

    #[test]
    fn full_example_three_items() {
        let src = r#"
            client c { tier execution model "sonnet" }
            agent a { client c description "task" }
            workflow w { steps [a] }
        "#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        assert_eq!(ast.unwrap().items.len(), 3);
    }

    #[test]
    fn parse_error_missing_brace() {
        let src = "agent bad { description \"x\" ";
        let (_, errs) = parse_str(src);
        assert!(!errs.is_empty(), "expected parse error");
    }

    #[test]
    fn agent_with_memory_block_minimal() {
        let src = r#"agent x { memory { write_ns "findings" } }"#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Agent(a) = &ast.unwrap().items[0] {
            let mem = a.memory.as_ref().expect("memory block");
            assert_eq!(mem.write_ns.as_deref(), Some("findings"));
            assert!(mem.read_ns.is_empty());
            assert!(mem.importance.is_none());
        }
    }

    #[test]
    fn agent_with_memory_block_full() {
        let src = r#"
            agent scan {
                memory {
                    read_ns ["prior-audits" "shared"]
                    write_ns "scan-findings"
                    importance 0.9
                    staleness_sources ["src/"]
                }
            }
        "#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Agent(a) = &ast.unwrap().items[0] {
            let mem = a.memory.as_ref().expect("memory block");
            assert_eq!(mem.read_ns, vec!["prior-audits", "shared"]);
            assert_eq!(mem.write_ns.as_deref(), Some("scan-findings"));
            assert!(matches!(mem.importance, Some(v) if (v - 0.9).abs() < 1e-5));
            assert_eq!(mem.staleness_sources, vec!["src/"]);
        }
    }

    #[test]
    fn workflow_with_memory_block() {
        let src = r#"workflow wf { steps [a] memory { read_ns ["shared"] write_ns "wf-out" } }"#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Workflow(w) = &ast.unwrap().items[0] {
            let mem = w.memory.as_ref().expect("memory block");
            assert_eq!(mem.read_ns, vec!["shared"]);
            assert_eq!(mem.write_ns.as_deref(), Some("wf-out"));
        }
    }
}
