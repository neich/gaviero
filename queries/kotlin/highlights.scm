; Kotlin highlights.scm — for tree-sitter-kotlin-ng 1.1.0

; Comments
;---------

(line_comment) @comment
(block_comment) @comment

; Functions
;----------

(function_declaration
  (identifier) @function)

(call_expression
  (identifier) @function.call)

(call_expression
  (navigation_expression
    (identifier) @function.call) .)

; Annotations
;------------

(annotation
  (user_type
    (identifier) @attribute))

"@" @operator

; Types
;------

(user_type
  (identifier) @type)

(class_declaration
  (identifier) @type)

(object_declaration
  (identifier) @type)

(type_alias
  (identifier) @type)

; Kotlin builtins
(this_expression) @variable.builtin
(super_expression) @variable.builtin

; Constants (UPPER_CASE identifiers)
((identifier) @constant
 (#match? @constant "^_*[A-Z][A-Z\\d_]+$"))

; Variables / Identifiers (fallback)
(identifier) @variable

; Literals
;---------

(number_literal) @constant.numeric
(float_literal) @constant.numeric

[
  (character_literal)
  (string_literal)
  (multiline_string_literal)
] @string

(string_content) @string
(interpolation) @string.escape
(escape_sequence) @string.escape

; Keywords
;---------

[
  "abstract"
  "actual"
  "annotation"
  "as"
  "by"
  "catch"
  "class"
  "companion"
  "const"
  "constructor"
  "crossinline"
  "data"
  "delegate"
  "do"
  "dynamic"
  "else"
  "enum"
  "expect"
  "external"
  "final"
  "finally"
  "for"
  "fun"
  "get"
  "if"
  "import"
  "in"
  "infix"
  "init"
  "inline"
  "inner"
  "interface"
  "internal"
  "is"
  "lateinit"
  "noinline"
  "object"
  "open"
  "operator"
  "out"
  "override"
  "package"
  "private"
  "protected"
  "public"
  "return"
  "sealed"
  "set"
  "suspend"
  "tailrec"
  "this"
  "super"
  "throw"
  "try"
  "typealias"
  "val"
  "value"
  "var"
  "vararg"
  "when"
  "where"
  "while"
] @keyword

; Punctuation
;------------

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

[
  ";"
  ","
  "."
  "::"
  ":"
] @punctuation.delimiter

; Operators
;----------

[
  "+"
  "-"
  "*"
  "/"
  "%"
  "="
  "+="
  "-="
  "*="
  "/="
  "%="
  "=="
  "!="
  "<"
  ">"
  "<="
  ">="
  "&&"
  "||"
  "!"
  ".."
  "->"
  "?:"
  "?."
  "?"
] @operator

; Properties
;-----------

(navigation_expression
  (identifier) @property .)
