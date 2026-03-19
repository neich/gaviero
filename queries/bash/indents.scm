; Indent queries from Helix editor (https://github.com/helix-editor/helix)
; Licensed under MPL-2.0

[
  (function_definition)
  (if_statement)
  (for_statement)
  (while_statement)
  (case_statement)
  (pipeline)
] @indent

[
  "}"
] @outdent
