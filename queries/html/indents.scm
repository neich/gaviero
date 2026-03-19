; Indent queries from Helix editor (https://github.com/helix-editor/helix)
; Licensed under MPL-2.0

; Helix does not provide indents.scm for HTML; minimal bracket indentation patterns.

(element) @indent

[
  (end_tag)
] @outdent
