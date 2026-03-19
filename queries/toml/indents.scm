; Indent queries from Helix editor (https://github.com/helix-editor/helix)
; Licensed under MPL-2.0

; Helix does not provide indents.scm for TOML; minimal bracket indentation patterns.

[
  (table)
  (array)
  (inline_table)
] @indent

[
  "]"
  "}"
] @outdent
