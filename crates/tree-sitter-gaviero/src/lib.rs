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
        let tree = parser.parse(r#"client c { tier coordinator model "opus" }"#, None).unwrap();
        assert!(!tree.root_node().has_error(), "parse tree: {}", tree.root_node().to_sexp());
    }

    #[test]
    fn parse_agent_with_quoted_prompt() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(r#"agent x { prompt "do the thing" }"#, None).unwrap();
        assert!(!tree.root_node().has_error(), "parse tree: {}", tree.root_node().to_sexp());
    }

    #[test]
    fn parse_raw_string() {
        let mut parser = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = LANGUAGE.into();
        parser.set_language(&lang).unwrap();
        let src = "agent x { prompt #\"\nhello\nworld\n\"# }";
        let tree = parser.parse(src, None).unwrap();
        assert!(!tree.root_node().has_error(), "parse tree: {}", tree.root_node().to_sexp());
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
        assert!(!tree.root_node().has_error(), "parse tree: {}", tree.root_node().to_sexp());
    }
}
