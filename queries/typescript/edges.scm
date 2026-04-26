; C4: typed-edge extraction queries for TypeScript / TSX.
;   @import     -> EdgeKind::Imports     (import / require)
;   @call       -> EdgeKind::Calls       (function / method invocation)
;   @implements -> EdgeKind::Implements  (class extends / implements)

; ── Imports ───────────────────────────────────────────────────
(import_statement
  source: (string) @import.source) @import

; CommonJS require()
(call_expression
  function: (identifier) @_req
  arguments: (arguments (string) @import.source)
  (#eq? @_req "require")) @import

; ── Calls ─────────────────────────────────────────────────────
(call_expression
  function: (identifier) @call.name) @call

(call_expression
  function: (member_expression
    property: (property_identifier) @call.method)) @call

; ── Implementations ───────────────────────────────────────────
(class_declaration
  (class_heritage
    (extends_clause
      value: (_) @implements.parent))) @implements

(class_declaration
  (class_heritage
    (implements_clause
      (type_identifier) @implements.interface))) @implements
