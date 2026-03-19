; Indent queries from Helix editor (https://github.com/helix-editor/helix)
; Licensed under MPL-2.0

; Based on Helix C indents with C++-specific additions

[
  (compound_statement)
  (declaration_list)
  (field_declaration_list)
  (enumerator_list)
  (parameter_list)
  (init_declarator)
  (expression_statement)
] @indent

[
  "case"
  "}"
  "]"
  ")"
] @outdent

(if_statement
  consequence: (_) @indent
  (#not-kind-eq? @indent "compound_statement")
  (#set! "scope" "all"))
(while_statement
  body: (_) @indent
  (#not-kind-eq? @indent "compound_statement")
  (#set! "scope" "all"))
(do_statement
  body: (_) @indent
  (#not-kind-eq? @indent "compound_statement")
  (#set! "scope" "all"))
(for_statement
  ")"
  (_) @indent
  (#not-kind-eq? @indent "compound_statement")
  (#set! "scope" "all"))

(parameter_list
  . (parameter_declaration) @anchor
  (#set! "scope" "tail")) @align
(argument_list
  . (_) @anchor
  (#set! "scope" "tail")) @align
(binary_expression
  (#set! "scope" "tail")) @anchor @align
(conditional_expression
  "?" @anchor
  (#set! "scope" "tail")) @align

; C++-specific
(access_specifier) @outdent
