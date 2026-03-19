; Indent queries from Helix editor (https://github.com/helix-editor/helix)
; Licensed under MPL-2.0

; Combined from Helix ecma + javascript indents.scm

[
  (array)
  (object)
  (arguments)
  (formal_parameters)

  (statement_block)
  (switch_statement)
  (object_pattern)
  (class_body)
  (named_imports)

  (binary_expression)
  (return_statement)
  (template_substitution)
  (export_clause)
] @indent

[
  (switch_case)
  (switch_default)
] @indent @extend

[
  "}"
  "]"
  ")"
] @outdent
