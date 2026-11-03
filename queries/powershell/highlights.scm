; PowerShell highlights.scm — for tree-sitter-powershell 0.26.6 (airbus-cert).
; Adapted from the grammar's bundled queries/highlights.scm, renamed to the
; capture groups gaviero themes define (`@delimiter`, `@array`, `@assignvalue`
; have no theme entry and are dropped).
;
; Spans are applied widest-first and the last one wins, so the generic
; `(variable)` / `(string_literal)` captures come before the narrower ones
; that refine them.

; ── Literals ────────────────────────────────────────────────────

(string_literal) @string

(integer_literal) @number
(real_literal) @number

; ── Variables ───────────────────────────────────────────────────

(variable) @variable
(braced_variable) @variable

((variable) @constant.builtin
  (#match? @constant.builtin "^\\$(?i:true|false|null)$"))

; ── Types and attributes ────────────────────────────────────────

(type_spec) @type
(attribute_name) @type
(simple_name) @type

; ── Commands and functions ──────────────────────────────────────

(command
  command_name: (command_name) @function.call)

(command_parameter) @constant.builtin

(function_statement
  (function_name) @function)

(class_method_definition
  (simple_name) @function)

(invokation_expression
  (member_name) @function.call)

(member_access
  (member_name) @property)

(hash_entry
  (key_expression) @string.special.key)

; ── Keywords ────────────────────────────────────────────────────

[
  "param"
  "dynamicparam"
  "begin"
  "process"
  "end"
  "if"
  "elseif"
  "else"
  "switch"
  "foreach"
  "for"
  "while"
  "do"
  "until"
  "filter"
  "workflow"
  "break"
  "continue"
  "throw"
  "return"
  "exit"
  "trap"
  "try"
  "catch"
  "finally"
  "data"
  "inlinescript"
  "parallel"
  "sequence"
  "in"
] @keyword

"function" @keyword.function

(class_attribute) @keyword

; ── Operators ───────────────────────────────────────────────────

[
  "-as"
  "-ccontains"
  "-ceq"
  "-cge"
  "-cgt"
  "-cle"
  "-clike"
  "-clt"
  "-cmatch"
  "-cne"
  "-cnotcontains"
  "-cnotlike"
  "-cnotmatch"
  "-contains"
  "-creplace"
  "-csplit"
  "-eq"
  "-ge"
  "-gt"
  "-icontains"
  "-ieq"
  "-ige"
  "-igt"
  "-ile"
  "-ilike"
  "-ilt"
  "-imatch"
  "-in"
  "-ine"
  "-inotcontains"
  "-inotlike"
  "-inotmatch"
  "-ireplace"
  "-is"
  "-isnot"
  "-isplit"
  "-join"
  "-le"
  "-like"
  "-lt"
  "-match"
  "-ne"
  "-notcontains"
  "-notin"
  "-notlike"
  "-notmatch"
  "-replace"
  "-shl"
  "-shr"
  "-split"
  "-and"
  "-or"
  "-xor"
  "-band"
  "-bor"
  "-bxor"
  "-bnot"
  "-not"
  "-f"
  "+"
  "-"
  "*"
  "/"
  "\\"
  "%"
  ".."
  "!"
  "="
  "+="
  "-="
  "*="
  "/="
  "%="
  "++"
  "--"
  "|"
  "&&"
  "||"
  "::"
] @operator

(comparison_operator) @operator
(assignement_operator) @operator
(command_invokation_operator) @operator

; ── Punctuation ─────────────────────────────────────────────────

[
  ";"
  ","
  "("
  ")"
  "{"
  "}"
  "["
  "]"
  "@("
  "@{"
  "$("
] @punctuation

; ── Comments (last, so nothing overrides them) ──────────────────

(comment) @comment
