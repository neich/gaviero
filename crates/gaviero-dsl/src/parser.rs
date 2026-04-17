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
    Default,
}

#[derive(Debug)]
enum AgentField {
    Description(String, Span),
    Client(String, Span),
    Scope(ScopeBlock),
    DependsOn(Vec<(String, Span)>, Span),
    Prompt(PromptSource, Span),
    MaxRetries(u8, Span),
    Memory(MemoryBlock),
    Context(ContextBlock),
    Vars(Vec<(String, String)>),
}

#[derive(Debug)]
enum ContextField {
    CallersOf(Vec<String>),
    TestsFor(Vec<String>),
    Depth(u32),
}

#[derive(Debug)]
enum ScopeField {
    Owned(Vec<String>),
    ReadOnly(Vec<String>),
    ImpactScope(bool),
}

#[derive(Debug)]
enum WorkflowField {
    Steps(Vec<StepItem>, Span),
    MaxParallel(usize, Span),
    Memory(MemoryBlock),
    Strategy(StrategyLit, Span),
    TestFirst(bool, Span),
    MaxRetries(u32, Span),
    Attempts(u32, Span),
    EscalateAfter(u32, Span),
    Verify(VerifyBlock),
}

#[derive(Debug)]
enum VerifyField {
    Compile(bool),
    Clippy(bool),
    Test(bool),
    ImpactTests(bool),
}

#[derive(Debug)]
enum LoopField {
    Agents(Vec<(String, Span)>),
    Until(UntilCondition),
    MaxIterations(u32),
    IterStart(u32),
    Stability(u32),
    JudgeTimeout(u32),
    StrictJudge(bool),
}

#[derive(Debug)]
enum MemoryField {
    ReadNs(Vec<String>),
    WriteNs(String),
    Importance(f32),
    StalenessSources(Vec<String>),
    ReadQuery(String),
    ReadLimit(usize),
    WriteContent(String),
}

// ── Parser builder ─────────────────────────────────────────────────────────

fn script_parser<'src, I>()
-> impl Parser<'src, I, Script, extra::Err<Rich<'src, Token, Span>>> + Clone
where
    I: ValueInput<'src, Token = Token, Span = Span>,
{
    // ── Primitives ───────────────────────────────────────────────

    // Some keywords are also common agent/workflow names (e.g. "verify", "compile",
    // "test"). We allow them as identifiers in name positions (after `agent`,
    // `workflow`, `client`, inside `steps []`, `depends_on []`) so that existing
    // example files continue to work.
    let ident = select! {
        Token::Ident(s) => s,
        // contextual keywords allowed as names
        Token::KwVerify     => "verify".to_owned(),
        Token::KwCompile    => "compile".to_owned(),
        Token::KwClippy     => "clippy".to_owned(),
        Token::KwTest       => "test".to_owned(),
        Token::KwStrategy   => "strategy".to_owned(),
        Token::KwTestFirst  => "test_first".to_owned(),
        Token::KwAttempts   => "attempts".to_owned(),
        Token::KwEscalateAfter => "escalate_after".to_owned(),
        Token::StratSinglePass => "single_pass".to_owned(),
        Token::StratRefine  => "refine".to_owned(),
        // loop-related contextual keywords
        Token::KwLoop       => "loop".to_owned(),
        Token::KwUntil      => "until".to_owned(),
        Token::KwAgents     => "agents".to_owned(),
        Token::KwMaxIterations => "max_iterations".to_owned(),
        Token::KwIterStart  => "iter_start".to_owned(),
        Token::KwStability  => "stability".to_owned(),
        Token::KwJudgeTimeout => "judge_timeout".to_owned(),
        Token::KwStrictJudge => "strict_judge".to_owned(),
        Token::KwCommand    => "command".to_owned(),
        // graph/impact contextual keywords
        Token::KwImpactScope => "impact_scope".to_owned(),
        Token::KwImpactTests => "impact_tests".to_owned(),
        Token::KwContext    => "context".to_owned(),
        Token::KwCallersOf  => "callers_of".to_owned(),
        Token::KwTestsFor   => "tests_for".to_owned(),
        Token::KwDepth      => "depth".to_owned(),
        // vars contextual keyword
        Token::KwVars       => "vars".to_owned(),
    };

    let string = select! {
        Token::Str(s)    => s,
        Token::RawStr(s) => s,
    };

    let tier_lit = select! {
        Token::TierCoordinator => TierLit::Coordinator,
        Token::TierReasoning   => TierLit::Reasoning,
        Token::TierExecution   => TierLit::Execution,
        Token::TierMechanical  => TierLit::Mechanical,
        Token::TierCheap       => TierLit::Cheap,
        Token::TierExpensive   => TierLit::Expensive,
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

    // ── vars block: { IDENT "value" ... } ────────────────────────
    //
    // Used at top level (Item::Vars) and inside agent declarations.
    // Keys are identifiers (or contextual keywords); values are strings.

    let vars_pair = ident.then(string.clone()).map(|(k, v)| (k, v));

    let vars_block = just(Token::KwVars).ignore_then(
        vars_pair
            .repeated()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LBrace), just(Token::RBrace)),
    );

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
        just(Token::KwDefault).map(|_| ClientField::Default),
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
            let mut is_default = false;
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
                    ClientField::Default => {
                        is_default = true;
                    }
                }
            }
            ClientDecl {
                name,
                name_span,
                tier,
                model,
                privacy,
                is_default,
                span: e.span(),
            }
        });

    // ── scope block ───────────────────────────────────────────────

    let scope_field = choice((
        just(Token::KwOwned)
            .ignore_then(str_list.clone())
            .map(ScopeField::Owned),
        just(Token::KwReadOnly)
            .ignore_then(str_list.clone())
            .map(ScopeField::ReadOnly),
        just(Token::KwImpactScope)
            .ignore_then(select! {
                Token::Ident(s) if s == "true"  => true,
                Token::Ident(s) if s == "false" => false,
            })
            .map(ScopeField::ImpactScope),
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
            let mut impact_scope = None;
            for f in fields {
                match f {
                    ScopeField::Owned(v) => owned.extend(v),
                    ScopeField::ReadOnly(v) => read_only.extend(v),
                    ScopeField::ImpactScope(v) => {
                        impact_scope.get_or_insert((v, e.span()));
                    }
                }
            }
            ScopeBlock {
                owned,
                read_only,
                impact_scope,
                span: e.span(),
            }
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
        just(Token::KwReadQuery)
            .ignore_then(string.clone())
            .map(MemoryField::ReadQuery),
        just(Token::KwReadLimit)
            .ignore_then(integer)
            .map(|n| MemoryField::ReadLimit(n as usize)),
        just(Token::KwWriteContent)
            .ignore_then(string.clone())
            .map(MemoryField::WriteContent),
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
            let mut read_query = None;
            let mut read_limit = None;
            let mut write_content = None;
            for f in fields {
                match f {
                    MemoryField::ReadNs(v) => read_ns.extend(v),
                    MemoryField::WriteNs(s) => {
                        write_ns.get_or_insert(s);
                    }
                    MemoryField::Importance(v) => {
                        importance.get_or_insert(v);
                    }
                    MemoryField::StalenessSources(v) => staleness_sources.extend(v),
                    MemoryField::ReadQuery(s) => {
                        read_query.get_or_insert((s, e.span()));
                    }
                    MemoryField::ReadLimit(n) => {
                        read_limit.get_or_insert((n, e.span()));
                    }
                    MemoryField::WriteContent(s) => {
                        write_content.get_or_insert((s, e.span()));
                    }
                }
            }
            MemoryBlock {
                read_ns,
                write_ns,
                importance,
                staleness_sources,
                read_query,
                read_limit,
                write_content,
                span: e.span(),
            }
        });

    // ── bool literal (true/false as Ident) ───────────────────────

    let bool_lit = select! {
        Token::Ident(s) if s == "true"  => true,
        Token::Ident(s) if s == "false" => false,
    };

    // ── strategy literal ──────────────────────────────────────────

    let strategy_lit = choice((
        just(Token::StratSinglePass).map(|_| StrategyLit::SinglePass),
        just(Token::StratRefine).map(|_| StrategyLit::Refine),
        // best_of_N: lex as Ident("best_of_N"), parse the trailing number.
        // The guard ensures we only match tokens of the form "best_of_<digits>".
        select! {
            Token::Ident(s) if s.starts_with("best_of_") && s["best_of_".len()..].parse::<u32>().is_ok() => {
                s["best_of_".len()..].parse::<u32>().unwrap()
            }
        }.map(StrategyLit::BestOfN),
    ));

    // ── verify block ──────────────────────────────────────────────

    let verify_field = {
        let b1 = bool_lit.clone();
        let b2 = bool_lit.clone();
        let b3 = bool_lit.clone();
        let b4 = bool_lit.clone();
        choice((
            just(Token::KwCompile)
                .ignore_then(b1)
                .map(VerifyField::Compile),
            just(Token::KwClippy)
                .ignore_then(b2)
                .map(VerifyField::Clippy),
            just(Token::KwTest).ignore_then(b3).map(VerifyField::Test),
            just(Token::KwImpactTests)
                .ignore_then(b4)
                .map(VerifyField::ImpactTests),
        ))
    };

    let verify_block = just(Token::KwVerify)
        .ignore_then(
            verify_field
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|fields, e| {
            let mut compile = false;
            let mut clippy = false;
            let mut test = false;
            let mut impact_tests = false;
            for f in fields {
                match f {
                    VerifyField::Compile(v) => compile = v,
                    VerifyField::Clippy(v) => clippy = v,
                    VerifyField::Test(v) => test = v,
                    VerifyField::ImpactTests(v) => impact_tests = v,
                }
            }
            VerifyBlock {
                compile,
                clippy,
                test,
                impact_tests,
                span: e.span(),
            }
        });

    // ── context block ─────────────────────────────────────────────

    let context_field = choice((
        just(Token::KwCallersOf)
            .ignore_then(str_list.clone())
            .map(ContextField::CallersOf),
        just(Token::KwTestsFor)
            .ignore_then(str_list.clone())
            .map(ContextField::TestsFor),
        just(Token::KwDepth)
            .ignore_then(integer)
            .map(|n| ContextField::Depth(n as u32)),
    ));

    let context_block = just(Token::KwContext)
        .ignore_then(
            context_field
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|fields, e| {
            let mut callers_of = Vec::new();
            let mut tests_for = Vec::new();
            let mut depth = None;
            for f in fields {
                match f {
                    ContextField::CallersOf(v) => callers_of.extend(v),
                    ContextField::TestsFor(v) => tests_for.extend(v),
                    ContextField::Depth(n) => {
                        depth.get_or_insert((n, e.span()));
                    }
                }
            }
            ContextBlock {
                callers_of,
                tests_for,
                depth,
                span: e.span(),
            }
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
            .ignore_then(choice((
                string.map_with(|s, e| (PromptSource::Inline(s), e.span())),
                ident.map_with(|s, e| (PromptSource::Ref(s, e.span()), e.span())),
            )))
            .map(|(src, s)| AgentField::Prompt(src, s)),
        just(Token::KwMaxRetries)
            .ignore_then(integer.map_with(|n, e| (n, e.span())))
            .map(|(n, s)| AgentField::MaxRetries(n.min(255) as u8, s)),
        memory_block.clone().map(AgentField::Memory),
        context_block.map(AgentField::Context),
        vars_block.clone().map(AgentField::Vars),
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
            let mut context = None;
            let mut vars: Vec<(String, String)> = Vec::new();
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
                    AgentField::Context(b) => {
                        context.get_or_insert(b);
                    }
                    AgentField::Vars(pairs) => {
                        vars.extend(pairs);
                    }
                }
            }
            AgentDecl {
                name,
                name_span,
                description,
                client,
                scope,
                depends_on,
                prompt,
                max_retries,
                memory,
                context,
                vars,
                span: e.span(),
            }
        });

    // ── loop block (inside workflow steps) ─────────────────────────

    // until condition: verify block | agent <name> | command "..."
    let until_condition = {
        let b1 = bool_lit.clone();
        let b2 = bool_lit.clone();
        let b3 = bool_lit.clone();
        let b4 = bool_lit.clone();
        let until_verify_field = choice((
            just(Token::KwCompile)
                .ignore_then(b1)
                .map(VerifyField::Compile),
            just(Token::KwClippy)
                .ignore_then(b2)
                .map(VerifyField::Clippy),
            just(Token::KwTest).ignore_then(b3).map(VerifyField::Test),
            just(Token::KwImpactTests)
                .ignore_then(b4)
                .map(VerifyField::ImpactTests),
        ));
        let until_verify = until_verify_field
            .repeated()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LBrace), just(Token::RBrace))
            .map_with(|fields, e| {
                let mut compile = false;
                let mut clippy = false;
                let mut test = false;
                let mut impact_tests = false;
                for f in fields {
                    match f {
                        VerifyField::Compile(v) => compile = v,
                        VerifyField::Clippy(v) => clippy = v,
                        VerifyField::Test(v) => test = v,
                        VerifyField::ImpactTests(v) => impact_tests = v,
                    }
                }
                UntilCondition::Verify(VerifyBlock {
                    compile,
                    clippy,
                    test,
                    impact_tests,
                    span: e.span(),
                })
            });

        let until_agent = just(Token::KwAgent)
            .ignore_then(ident.map_with(|s, e| (s, e.span())))
            .map(|(name, span)| UntilCondition::Agent(name, span));

        let until_command = just(Token::KwCommand)
            .ignore_then(string.map_with(|s, e| (s, e.span())))
            .map(|(cmd, span)| UntilCondition::Command(cmd, span));

        choice((until_verify, until_agent, until_command))
    };

    let loop_field = choice((
        just(Token::KwAgents)
            .ignore_then(ident_list.clone())
            .map(LoopField::Agents),
        just(Token::KwUntil)
            .ignore_then(until_condition)
            .map(LoopField::Until),
        just(Token::KwMaxIterations)
            .ignore_then(integer)
            .map(|n| LoopField::MaxIterations(n as u32)),
        just(Token::KwIterStart)
            .ignore_then(integer)
            .map(|n| LoopField::IterStart(n as u32)),
        just(Token::KwStability)
            .ignore_then(integer)
            .map(|n| LoopField::Stability(n as u32)),
        just(Token::KwJudgeTimeout)
            .ignore_then(integer)
            .map(|n| LoopField::JudgeTimeout(n as u32)),
        just(Token::KwStrictJudge)
            .ignore_then(bool_lit)
            .map(LoopField::StrictJudge),
    ));

    let loop_block = just(Token::KwLoop)
        .ignore_then(
            loop_field
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|fields, e| {
            let mut agents = Vec::new();
            let mut until = None;
            let mut max_iterations = None;
            let mut iter_start = None;
            let mut stability = None;
            let mut judge_timeout_secs = None;
            let mut strict_judge = None;
            for f in fields {
                match f {
                    LoopField::Agents(v) => agents = v,
                    LoopField::Until(c) => {
                        until.get_or_insert(c);
                    }
                    LoopField::MaxIterations(n) => {
                        max_iterations.get_or_insert(n);
                    }
                    LoopField::IterStart(n) => {
                        iter_start.get_or_insert(n);
                    }
                    LoopField::Stability(n) => {
                        stability.get_or_insert(n);
                    }
                    LoopField::JudgeTimeout(n) => {
                        judge_timeout_secs.get_or_insert(n);
                    }
                    LoopField::StrictJudge(b) => {
                        strict_judge.get_or_insert(b);
                    }
                }
            }
            LoopBlock {
                agents,
                until: until.unwrap_or(UntilCondition::Verify(VerifyBlock {
                    compile: false,
                    clippy: false,
                    test: false,
                    impact_tests: false,
                    span: e.span(),
                })),
                max_iterations: max_iterations.unwrap_or(10),
                iter_start: iter_start.unwrap_or(1),
                stability: stability.unwrap_or(1),
                judge_timeout_secs: judge_timeout_secs.unwrap_or(120),
                strict_judge: strict_judge.unwrap_or(true),
                span: e.span(),
            }
        });

    // ── step item (agent ref or loop block) ──────────────────────

    let step_item = choice((
        loop_block.map(StepItem::Loop),
        ident.map_with(|s, e| StepItem::Agent(s, e.span())),
    ));

    let step_list = step_item
        .repeated()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LBracket), just(Token::RBracket));

    // ── workflow declaration ──────────────────────────────────────

    let workflow_field = choice((
        just(Token::KwSteps)
            .ignore_then(step_list.map_with(|v, e| (v, e.span())))
            .map(|(v, s)| WorkflowField::Steps(v, s)),
        just(Token::KwMaxParallel)
            .ignore_then(integer.map_with(|n, e| (n, e.span())))
            .map(|(n, s)| WorkflowField::MaxParallel(n as usize, s)),
        memory_block.map(WorkflowField::Memory),
        just(Token::KwStrategy)
            .ignore_then(strategy_lit.map_with(|v, e| (v, e.span())))
            .map(|(v, s)| WorkflowField::Strategy(v, s)),
        just(Token::KwTestFirst)
            .ignore_then(bool_lit.map_with(|v, e| (v, e.span())))
            .map(|(v, s)| WorkflowField::TestFirst(v, s)),
        just(Token::KwMaxRetries)
            .ignore_then(integer.map_with(|n, e| (n, e.span())))
            .map(|(n, s)| WorkflowField::MaxRetries(n as u32, s)),
        just(Token::KwAttempts)
            .ignore_then(integer.map_with(|n, e| (n, e.span())))
            .map(|(n, s)| WorkflowField::Attempts(n as u32, s)),
        just(Token::KwEscalateAfter)
            .ignore_then(integer.map_with(|n, e| (n, e.span())))
            .map(|(n, s)| WorkflowField::EscalateAfter(n as u32, s)),
        verify_block.map(WorkflowField::Verify),
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
            let mut strategy = None;
            let mut test_first = None;
            let mut max_retries = None;
            let mut attempts = None;
            let mut escalate_after = None;
            let mut verify = None;
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
                    WorkflowField::Strategy(v, s) => {
                        strategy.get_or_insert((v, s));
                    }
                    WorkflowField::TestFirst(v, s) => {
                        test_first.get_or_insert((v, s));
                    }
                    WorkflowField::MaxRetries(n, s) => {
                        max_retries.get_or_insert((n, s));
                    }
                    WorkflowField::Attempts(n, s) => {
                        attempts.get_or_insert((n, s));
                    }
                    WorkflowField::EscalateAfter(n, s) => {
                        escalate_after.get_or_insert((n, s));
                    }
                    WorkflowField::Verify(b) => {
                        verify.get_or_insert(b);
                    }
                }
            }
            WorkflowDecl {
                name,
                name_span,
                steps,
                max_parallel,
                memory,
                strategy,
                test_first,
                max_retries,
                attempts,
                escalate_after,
                verify,
                span: e.span(),
            }
        });

    // ── top-level prompt declaration ──────────────────────────────
    //
    // prompt <name> <string>
    //
    // Agents reference it by identifier: `prompt <name>`

    let prompt_decl = just(Token::KwPrompt)
        .ignore_then(ident.map_with(|n, e| (n, e.span())))
        .then(string.map_with(|s, e| (s, e.span())))
        .map_with(
            |((name, name_span), (content, content_span)), e| PromptDecl {
                name,
                name_span,
                content,
                content_span,
                span: e.span(),
            },
        );

    // ── top-level ─────────────────────────────────────────────────

    let item = choice((
        client_decl.map(Item::Client),
        agent_decl.map(Item::Agent),
        workflow_decl.map(Item::Workflow),
        prompt_decl.map(Item::Prompt),
        vars_block.map(Item::Vars),
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

    #[test]
    fn agent_memory_read_query_and_limit() {
        let src = r#"
            agent x {
                memory {
                    read_query "security vulnerabilities in auth"
                    read_limit 10
                }
            }
        "#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Agent(a) = &ast.unwrap().items[0] {
            let mem = a.memory.as_ref().expect("memory block");
            assert_eq!(
                mem.read_query.as_ref().map(|(s, _)| s.as_str()),
                Some("security vulnerabilities in auth")
            );
            assert_eq!(mem.read_limit.as_ref().map(|(n, _)| *n), Some(10));
        }
    }

    #[test]
    fn agent_memory_write_content_template() {
        let src = r##"
            agent scan {
                memory {
                    write_ns "findings"
                    write_content #"Findings: {{SUMMARY}} Files: {{FILES}}"#
                }
            }
        "##;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Agent(a) = &ast.unwrap().items[0] {
            let mem = a.memory.as_ref().expect("memory block");
            assert_eq!(mem.write_ns.as_deref(), Some("findings"));
            let (content, _) = mem.write_content.as_ref().expect("write_content");
            assert!(content.contains("{{SUMMARY}}"));
            assert!(content.contains("{{FILES}}"));
        }
    }

    #[test]
    fn agent_memory_all_new_fields_combined() {
        let src = r##"
            agent full {
                memory {
                    read_ns ["shared"]
                    write_ns "output"
                    importance 0.8
                    read_query "prior analysis results"
                    read_limit 15
                    write_content #"Agent: {{AGENT}} Summary: {{SUMMARY}}"#
                }
            }
        "##;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Agent(a) = &ast.unwrap().items[0] {
            let mem = a.memory.as_ref().expect("memory block");
            assert_eq!(mem.read_ns, vec!["shared"]);
            assert_eq!(mem.write_ns.as_deref(), Some("output"));
            assert!(matches!(mem.importance, Some(v) if (v - 0.8).abs() < 1e-5));
            assert_eq!(
                mem.read_query.as_ref().map(|(s, _)| s.as_str()),
                Some("prior analysis results")
            );
            assert_eq!(mem.read_limit.as_ref().map(|(n, _)| *n), Some(15));
            assert!(mem.write_content.is_some());
        }
    }

    // ── Loop tests ───────────────────────────────────────────────

    #[test]
    fn workflow_with_loop_verify_until() {
        let src = r#"
            agent a { description "impl" }
            agent b { description "test" }
            workflow w {
                steps [
                    a
                    loop {
                        agents [a b]
                        max_iterations 5
                        until { compile true test true }
                    }
                ]
            }
        "#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Workflow(w) = &ast.unwrap().items[2] {
            let (steps, _) = w.steps.as_ref().unwrap();
            assert_eq!(steps.len(), 2);
            assert!(matches!(&steps[0], StepItem::Agent(name, _) if name == "a"));
            if let StepItem::Loop(lb) = &steps[1] {
                assert_eq!(lb.agents.len(), 2);
                assert_eq!(lb.agents[0].0, "a");
                assert_eq!(lb.agents[1].0, "b");
                assert_eq!(lb.max_iterations, 5);
                assert!(matches!(&lb.until, UntilCondition::Verify(vb) if vb.compile && vb.test));
            } else {
                panic!("expected Loop step");
            }
        }
    }

    #[test]
    fn workflow_with_loop_agent_until() {
        let src = r#"
            agent impl_agent { description "implement" }
            agent judge { description "evaluate" }
            workflow w {
                steps [
                    loop {
                        agents [impl_agent]
                        max_iterations 3
                        until agent judge
                    }
                ]
            }
        "#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Workflow(w) = &ast.unwrap().items[2] {
            let (steps, _) = w.steps.as_ref().unwrap();
            if let StepItem::Loop(lb) = &steps[0] {
                assert!(matches!(&lb.until, UntilCondition::Agent(name, _) if name == "judge"));
                assert_eq!(lb.max_iterations, 3);
            } else {
                panic!("expected Loop step");
            }
        }
    }

    #[test]
    fn workflow_with_loop_command_until() {
        let src = r#"
            agent fixer { description "fix" }
            workflow w {
                steps [
                    loop {
                        agents [fixer]
                        max_iterations 10
                        until command "cargo test --quiet"
                    }
                ]
            }
        "#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Workflow(w) = &ast.unwrap().items[1] {
            let (steps, _) = w.steps.as_ref().unwrap();
            if let StepItem::Loop(lb) = &steps[0] {
                assert!(
                    matches!(&lb.until, UntilCondition::Command(cmd, _) if cmd == "cargo test --quiet")
                );
                assert_eq!(lb.max_iterations, 10);
            } else {
                panic!("expected Loop step");
            }
        }
    }

    #[test]
    fn workflow_mixed_steps_and_loops() {
        let src = r#"
            agent a { description "first" }
            agent b { description "second" }
            agent c { description "third" }
            workflow w {
                steps [
                    a
                    loop {
                        agents [b]
                        max_iterations 3
                        until { test true }
                    }
                    c
                ]
            }
        "#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Workflow(w) = &ast.unwrap().items[3] {
            let (steps, _) = w.steps.as_ref().unwrap();
            assert_eq!(steps.len(), 3);
            assert!(matches!(&steps[0], StepItem::Agent(n, _) if n == "a"));
            assert!(matches!(&steps[1], StepItem::Loop(_)));
            assert!(matches!(&steps[2], StepItem::Agent(n, _) if n == "c"));
        }
    }

    // ── Graph / impact tests ─────────────────────────────────────

    #[test]
    fn scope_with_impact_scope() {
        let src = r#"
            agent x {
                scope {
                    owned ["src/"]
                    read_only ["docs/"]
                    impact_scope true
                }
            }
        "#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Agent(a) = &ast.unwrap().items[0] {
            let scope = a.scope.as_ref().unwrap();
            assert_eq!(scope.impact_scope.as_ref().map(|(v, _)| *v), Some(true));
        }
    }

    #[test]
    fn verify_with_impact_tests() {
        let src = r#"
            agent a { description "t" }
            workflow w {
                steps [a]
                verify { compile true impact_tests true }
            }
        "#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Workflow(w) = &ast.unwrap().items[1] {
            let v = w.verify.as_ref().unwrap();
            assert!(v.compile);
            assert!(v.impact_tests);
            assert!(!v.test);
        }
    }

    #[test]
    fn loop_until_impact_tests() {
        let src = r#"
            agent a { description "impl" }
            workflow w {
                steps [
                    loop {
                        agents [a]
                        max_iterations 3
                        until { compile true impact_tests true }
                    }
                ]
            }
        "#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Workflow(w) = &ast.unwrap().items[1] {
            let (steps, _) = w.steps.as_ref().unwrap();
            if let StepItem::Loop(lb) = &steps[0] {
                if let UntilCondition::Verify(vb) = &lb.until {
                    assert!(vb.compile);
                    assert!(vb.impact_tests);
                } else {
                    panic!("expected Verify until condition");
                }
            } else {
                panic!("expected Loop step");
            }
        }
    }

    #[test]
    fn agent_with_context_block() {
        let src = r#"
            agent x {
                description "impl"
                context {
                    callers_of ["src/auth/session.rs"]
                    tests_for  ["src/auth/"]
                    depth      3
                }
            }
        "#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Agent(a) = &ast.unwrap().items[0] {
            let ctx = a.context.as_ref().expect("context block");
            assert_eq!(ctx.callers_of, vec!["src/auth/session.rs"]);
            assert_eq!(ctx.tests_for, vec!["src/auth/"]);
            assert_eq!(ctx.depth.as_ref().map(|(n, _)| *n), Some(3));
        }
    }

    #[test]
    fn agent_context_block_minimal() {
        let src = r#"
            agent x {
                context {
                    callers_of ["src/lib.rs"]
                }
            }
        "#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Agent(a) = &ast.unwrap().items[0] {
            let ctx = a.context.as_ref().expect("context block");
            assert_eq!(ctx.callers_of, vec!["src/lib.rs"]);
            assert!(ctx.tests_for.is_empty());
            assert!(ctx.depth.is_none());
        }
    }

    // ── Named prompt declaration tests ────────────────────────────────────────

    #[test]
    fn prompt_decl_parses() {
        let src = r#"prompt my-prompt "do the thing""#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        let ast = ast.unwrap();
        assert_eq!(ast.items.len(), 1);
        if let Item::Prompt(p) = &ast.items[0] {
            assert_eq!(p.name, "my-prompt");
            assert_eq!(p.content, "do the thing");
        } else {
            panic!("expected Prompt item");
        }
    }

    #[test]
    fn prompt_decl_raw_string_parses() {
        let src = "prompt body #\"write to {{AGENT}}.md\"#";
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Prompt(p) = &ast.unwrap().items[0] {
            assert_eq!(p.name, "body");
            assert!(p.content.contains("{{AGENT}}"));
        } else {
            panic!("expected Prompt item");
        }
    }

    #[test]
    fn agent_prompt_ref_parses() {
        let src = r#"
            prompt my-p "text"
            agent x { prompt my-p }
        "#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        let ast = ast.unwrap();
        // items[0] = Prompt, items[1] = Agent
        if let Item::Agent(a) = &ast.items[1] {
            match a.prompt.as_ref().expect("prompt field") {
                (PromptSource::Ref(name, _), _) => assert_eq!(name, "my-p"),
                (PromptSource::Inline(_), _) => panic!("expected Ref, got Inline"),
            }
        } else {
            panic!("expected Agent item at index 1");
        }
    }

    #[test]
    fn agent_prompt_inline_is_inline_variant() {
        let src = r#"agent x { prompt "inline text" }"#;
        let (ast, errs) = parse_str(src);
        assert!(errs.is_empty(), "{:?}", errs);
        if let Item::Agent(a) = &ast.unwrap().items[0] {
            match a.prompt.as_ref().expect("prompt field") {
                (PromptSource::Inline(s), _) => assert_eq!(s, "inline text"),
                (PromptSource::Ref(_, _), _) => panic!("expected Inline, got Ref"),
            }
        } else {
            panic!("expected Agent item");
        }
    }
}
