; LaTeX indents.scm for gaviero

; Environments indent their body
(generic_environment) @indent
(end) @outdent

; Curly groups
(curly_group) @indent
"}" @outdent

; Bracket groups
(brack_group_key_value) @indent
"]" @outdent
