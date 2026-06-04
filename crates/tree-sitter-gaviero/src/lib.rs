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
            agent a { client c description "task" }
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

        let examples_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../gaviero-dsl/examples");
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
        assert!(count > 0, "no .gaviero examples found in {}", examples_dir.display());
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
