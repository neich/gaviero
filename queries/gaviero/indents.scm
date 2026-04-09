; Indent queries for the .gaviero DSL

; Block structures with braces
[
  (client_declaration)
  (agent_declaration)
  (workflow_declaration)
  (scope_block)
  (memory_block)
  (verify_block)
  (context_block)
  (loop_block)
  (until_verify)
] @indent

; List structures with brackets
[
  (string_list)
  (identifier_list)
  (step_list)
] @indent

; Closing delimiters
[
  "}"
  "]"
] @outdent
