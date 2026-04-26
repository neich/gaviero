; C4: typed-edge extraction queries for Rust.
; Captures are interpreted by the graph builder as edge kinds:
;   @import     -> EdgeKind::Imports     (use / mod / extern crate)
;   @call       -> EdgeKind::Calls       (function / method invocation)
;   @implements -> EdgeKind::Implements  (impl Trait for Type)

; ── Imports ───────────────────────────────────────────────────
(use_declaration
  argument: (_) @import.path) @import

(extern_crate_declaration
  name: (identifier) @import.name) @import

; ── Calls (free + method) ─────────────────────────────────────
(call_expression
  function: (identifier) @call.name) @call

(call_expression
  function: (scoped_identifier) @call.path) @call

(call_expression
  function: (field_expression
    field: (field_identifier) @call.method)) @call

; ── Implementations ───────────────────────────────────────────
(impl_item
  trait: (_) @implements.trait) @implements
