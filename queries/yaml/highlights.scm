; Comments
;---------

(comment) @comment

; Keys
;-----

(block_mapping_pair
  key: (_) @property)

(flow_pair
  key: (_) @property)

; Strings
;--------

[
  (double_quote_scalar)
  (single_quote_scalar)
] @string

(block_scalar) @string
(string_scalar) @string

; Numbers
;--------

(integer_scalar) @number
(float_scalar) @number

; Booleans and null
;------------------

(boolean_scalar) @constant.builtin
(null_scalar) @constant.builtin

; Anchors and aliases
;--------------------

(anchor) @operator
(alias) @variable

; Tags
;-----

(tag) @type

; Punctuation
;------------

[
  ","
  "-"
  ":"
  ">"
  "?"
  "|"
] @punctuation.delimiter

[
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

; Directives
;-----------

[
  (yaml_directive)
  (tag_directive)
] @keyword
