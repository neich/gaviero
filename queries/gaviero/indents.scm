; Indent queries for the .gaviero DSL

; Block structures with braces
[
  (client_declaration)
  (agent_declaration)
  (workflow_declaration)
  (scope_block)
] @indent

; List structures with brackets
[
  (string_list)
  (identifier_list)
] @indent

; Closing delimiters
[
  "}"
  "]"
] @outdent
