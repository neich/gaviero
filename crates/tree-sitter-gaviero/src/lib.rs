use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_gaviero() -> *const ();
}

/// Returns the tree-sitter [`LanguageFn`] for the Gaviero DSL.
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_gaviero) };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_load_grammar() {
        let _lang: tree_sitter::Language = LANGUAGE.into();
    }

    #[test]
    fn parse_typed_vars() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = "vars {
  A \"x\"
  B file \"docs/b.md\"
  C file
  file \"y\"
  D dir \"src\"
  E dir
}
";
        let tree = parser.parse(src, None).unwrap();
        let sexp = tree.root_node().to_sexp();
        assert!(!tree.root_node().has_error(), "parse tree: {sexp}");
        assert_eq!(sexp.matches("(vars_pair").count(), 6, "{sexp}");
        assert_eq!(sexp.matches("(var_type").count(), 4, "{sexp}");
    }

    #[test]
    fn parse_client() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let tree = parser
            .parse(r#"client c { tier coordinator model "opus" }"#, None)
            .unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn parse_agent_with_quoted_prompt() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let tree = parser
            .parse(r#"agent x { prompt "do the thing" }"#, None)
            .unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn parse_raw_string() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = "agent x { prompt #\"\nhello\nworld\n\"# }";
        let tree = parser.parse(src, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn parse_full_example() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = r#"
            client c { tier execution model "sonnet" }
            agent a { tier c description "task" }
            workflow w { steps [a] max_parallel 2 }
        "#;
        let tree = parser.parse(src, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn parse_client_with_effort_and_extra() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = r#"
            client c {
                model "opus"
                effort high
                extra {
                    thinking_budget "8000"
                    max_tokens "32768"
                }
                default
            }
        "#;
        let tree = parser.parse(src, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn parse_top_level_tier_alias() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = r#"
            client deep { model "opus" }
            tier expensive deep
            tier fast deep
        "#;
        let tree = parser.parse(src, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn parse_top_level_prompt_and_ref() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = r#"
            prompt body "write to {{AGENT}}.md"
            agent a { prompt body }
        "#;
        let tree = parser.parse(src, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn parse_top_level_include() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = r#"
            include "lib/clients.gaviero"
            include "../shared/prompts.gaviero"
            agent a { prompt "x" }
            workflow w { steps [a] }
        "#;
        let tree = parser.parse(src, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn parse_vars_block_top_level_and_agent() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = r#"
            vars { TARGET "src/" LANG "rust" }
            agent a { vars { OUT "dist/" } description "d" }
        "#;
        let tree = parser.parse(src, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn parse_agent_tier_ref() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = r#"
            client deep { model "opus" }
            tier expensive deep
            agent a { tier expensive description "task" }
        "#;
        let tree = parser.parse(src, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn parse_loop_with_new_fields() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = r#"
            agent impl { description "i" }
            agent judge { description "j" }
            workflow w {
                steps [
                    loop {
                        agents [impl]
                        max_iterations 5
                        iter_start 2
                        stability 3
                        judge_timeout 60
                        strict_judge false
                        until agent judge
                    }
                ]
            }
        "#;
        let tree = parser.parse(src, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    /// H0 / PR-4: `until <cond> and <cond> …`.
    #[test]
    fn parse_loop_with_composed_until() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = r#"
            agent impl { description "i" }
            agent judge { description "j" }
            workflow w {
                steps [
                    loop {
                        agents [impl]
                        max_iterations 5
                        until { compile true } and command "cargo test" and agent judge
                    }
                ]
            }
        "#;
        let tree = parser.parse(src, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn parse_loop_with_branch_chain() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = r#"
            agent impl { description "i" }
            agent judge { description "j" }
            workflow w {
                steps [
                    loop {
                        agents [impl]
                        branch_chain stacked
                        until agent judge
                    }
                ]
            }
        "#;
        let tree = parser.parse(src, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn parse_workflow_execution_mode() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = r#"
            agent a { description "a" }
            workflow doc { execution_mode document steps [a] }
            workflow rep { execution_mode repo steps [a] }
        "#;
        let tree = parser.parse(src, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn parse_string_with_escaped_quote() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = r#"agent x { description "say \"hi\"" }"#;
        let tree = parser.parse(src, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }

    /// Cross-grammar compatibility: every `.gaviero` example shipped with
    /// `gaviero-dsl` must parse cleanly here. Catches drift between the
    /// authoritative grammar in `gaviero-dsl/src/parser.rs` and this one.
    #[test]
    fn parse_dsl_examples() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();

        let examples_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../gaviero-dsl/examples");
        let entries = std::fs::read_dir(&examples_dir)
            .unwrap_or_else(|e| panic!("read examples dir {}: {}", examples_dir.display(), e));

        let mut count = 0;
        for entry in entries {
            let path = entry.unwrap().path();
            if path.extension().and_then(|s| s.to_str()) != Some("gaviero") {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap();
            let tree = parser.parse(&src, None).unwrap();
            assert!(
                !tree.root_node().has_error(),
                "tree-sitter rejected `{}`:\n{}",
                path.display(),
                tree.root_node().to_sexp()
            );
            count += 1;
        }
        assert!(
            count > 0,
            "no .gaviero examples found in {}",
            examples_dir.display()
        );
    }

    fn parse_ok(src: &str) -> String {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(src, None).unwrap();
        let sexp = tree.root_node().to_sexp();
        assert!(!tree.root_node().has_error(), "parse tree: {sexp}");
        sexp
    }

    #[test]
    fn parse_for_parallel_with_peer_template() {
        let sexp = parse_ok(
            "workflow w { steps [ for t in topics parallel { a b peer_template #\"- {{peer}}\"# } c ] }",
        );
        assert_eq!(sexp.matches("(for_block").count(), 1, "{sexp}");
        assert_eq!(sexp.matches("(peer_template").count(), 1, "{sexp}");
    }

    #[test]
    fn parse_for_sequential() {
        let sexp = parse_ok("workflow w { steps [ for c in chapters sequential { editor } ] }");
        assert!(sexp.contains("(for_mode"), "{sexp}");
    }

    #[test]
    fn parse_for_inside_loop() {
        let sexp = parse_ok(
            "workflow w { steps [ loop { for t in topics parallel { r } until agent j } ] }",
        );
        assert!(sexp.contains("(loop_block"), "{sexp}");
        assert_eq!(sexp.matches("(for_block").count(), 1, "{sexp}");
    }

    #[test]
    fn parse_for_over_a_json_file() {
        let sexp = parse_ok(
            "workflow w { execution_mode document steps [ for q in json \"out/i.json\" parallel max 3 { worker } ] }",
        );
        assert_eq!(sexp.matches("(for_block").count(), 1, "{sexp}");
        assert_eq!(sexp.matches("(for_json_source").count(), 1, "{sexp}");
    }

    #[test]
    fn parse_workflow_budget_block() {
        let sexp = parse_ok("workflow w { steps [ a ] budget { wall \"8h\" usd 40 } }");
        assert_eq!(sexp.matches("(budget_block").count(), 1, "{sexp}");
        assert_eq!(sexp.matches("(budget_field").count(), 2, "{sexp}");
        parse_ok("workflow w { budget { usd 12.5 } steps [ a ] }");
    }

    #[test]
    fn parse_agent_produces_json() {
        let sexp = parse_ok(
            "agent a { produces [\"out/r.md\"] produces json [\"out/f.json\"] prompt \"x\" }",
        );
        assert!(!sexp.contains("ERROR"), "{sexp}");
    }

    #[test]
    fn parse_list_entry_with_free_field() {
        let sexp = parse_ok(
            "workflow w { param topics [ { id \"a\" focus \"storage\" } { id \"b\" tier high-codex } ] steps [ x ] }",
        );
        assert_eq!(sexp.matches("(reviewer_entry").count(), 2, "{sexp}");
        assert!(sexp.contains("(tier_alias_name"), "{sexp}");
    }

    #[test]
    fn parse_workflow_tiers_block() {
        let sexp = parse_ok(
            "agent a { tier low } workflow w { tiers { a medium-codex } steps [ a ] }",
        );
        assert!(sexp.contains("(tiers_block (agent_tier"), "{sexp}");
    }

    /// Every case of `test/corpus/*.txt` parses to its expected tree — the
    /// check `tree-sitter test` makes, run through `cargo test` so it needs
    /// no CLI.
    #[test]
    fn corpus_cases_parse_to_their_expected_trees() {
        let squash = |s: &str| {
            s.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .replace(" )", ")")
        };
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test/corpus");
        let mut cases = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            let mut parts = text.split("=====\n").skip(1);
            while let (Some(name), Some(body)) = (parts.next(), parts.next()) {
                let (source, expected) = body.split_once("\n-----\n").expect("case has -----");
                let mut parser = tree_sitter::Parser::new();
                let lang: tree_sitter::Language = LANGUAGE.into();
                parser.set_language(&lang).unwrap();
                let tree = parser.parse(source, None).unwrap();
                assert_eq!(
                    squash(&tree.root_node().to_sexp()),
                    squash(expected),
                    "corpus case `{}`",
                    name.trim()
                );
                cases += 1;
            }
        }
        assert!(cases > 10, "found {cases} corpus cases");
    }

    /// A model is chosen by tier only: no `client` on an agent, no
    /// model-valued param. (`model "…"` in a list item reads as a free
    /// field here; gaviero-dsl rejects it with a diagnostic.)
    #[test]
    fn model_forms_outside_profiles_are_syntax_errors() {
        for src in [
            "agent a { client c }",
            "workflow w { param j { model \"claude:opus\" } steps [ a ] }",
        ] {
            let mut parser = tree_sitter::Parser::new();
            let lang: tree_sitter::Language = LANGUAGE.into();
            parser.set_language(&lang).unwrap();
            let tree = parser.parse(src, None).unwrap();
            assert!(tree.root_node().has_error(), "{src} should not parse");
        }
    }

    #[test]
    fn invalid_gaviero_syntax_produces_error_nodes() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();

        let tree = parser
            .parse(r#"agent broken { prompt "missing closing brace""#, None)
            .unwrap();

        assert!(
            tree.root_node().has_error(),
            "parse tree: {}",
            tree.root_node().to_sexp()
        );
    }
}
