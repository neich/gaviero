; Variables
;----------

(identifier) @variable

; Function and class definitions
;-------------------------------

(function_definition
  name: (identifier) @function)

(class_definition
  name: (identifier) @type)

(decorator) @function

; Function calls
;---------------

(call
  function: (identifier) @function)

(call
  function: (attribute
    attribute: (identifier) @function.method))

; Parameters
;-----------

(parameters
  (identifier) @variable.parameter)

(default_parameter
  name: (identifier) @variable.parameter)

(typed_parameter
  (identifier) @variable.parameter)

(typed_default_parameter
  name: (identifier) @variable.parameter)

(list_splat_pattern
  (identifier) @variable.parameter)

(dictionary_splat_pattern
  (identifier) @variable.parameter)

; Builtins
;---------

((identifier) @variable.builtin
 (#match? @variable.builtin "^(self|cls)$"))

((identifier) @function.builtin
 (#match? @function.builtin "^(print|len|range|int|str|float|list|dict|set|tuple|bool|type|isinstance|issubclass|hasattr|getattr|setattr|delattr|super|property|staticmethod|classmethod|enumerate|zip|map|filter|sorted|reversed|min|max|sum|abs|round|open|input|id|hash|repr|format|iter|next|all|any|dir|vars|globals|locals|callable|chr|ord|hex|oct|bin|pow|divmod|complex|memoryview|bytearray|bytes|frozenset|object|slice|breakpoint|compile|eval|exec|__import__)$"))

((identifier) @constant.builtin
 (#match? @constant.builtin "^(NotImplemented|Ellipsis|__debug__|__name__|__doc__|__file__|__package__|__spec__|__loader__|__path__|__all__|__builtins__|__cached__)$"))

; Literals
;---------

[
  (true)
  (false)
] @constant.builtin

(none) @constant.builtin

(comment) @comment

[
  (string)
  (concatenated_string)
] @string

(interpolation
  "{" @punctuation.special
  "}" @punctuation.special)

[
  (integer)
  (float)
] @number

; Constants (UPPER_CASE identifiers)
;-----------------------------------

((identifier) @constant
 (#match? @constant "^[A-Z_][A-Z\\d_]+$"))

; Attributes
;-----------

(attribute
  attribute: (identifier) @property)

; Operators
;----------

[
  "-"
  "-="
  ":="
  "!="
  "*"
  "**"
  "**="
  "*="
  "/"
  "//"
  "//="
  "/="
  "&"
  "&="
  "%"
  "%="
  "^"
  "^="
  "+"
  "+="
  "<"
  "<<"
  "<<="
  "<="
  "<>"
  "="
  "=="
  ">"
  ">="
  ">>"
  ">>="
  "|"
  "|="
  "~"
  "->"
] @operator

; Keywords
;---------

[
  "and"
  "as"
  "assert"
  "async"
  "await"
  "break"
  "class"
  "continue"
  "def"
  "del"
  "elif"
  "else"
  "except"
  "exec"
  "finally"
  "for"
  "from"
  "global"
  "if"
  "import"
  "in"
  "is"
  "lambda"
  "nonlocal"
  "not"
  "or"
  "pass"
  "raise"
  "return"
  "try"
  "type"
  "while"
  "with"
  "yield"
  "match"
  "case"
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
  ":"
  ";"
  "."
  ","
] @punctuation.delimiter

; Type annotations
;-----------------

(type
  (identifier) @type)

(type
  (generic_type
    (identifier) @type))
