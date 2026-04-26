; C4: typed-edge extraction queries for Python.
;   @import     -> EdgeKind::Imports     (import / from-import)
;   @call       -> EdgeKind::Calls       (function / method invocation)
;   @implements -> EdgeKind::Implements  (class base classes)

; ── Imports ───────────────────────────────────────────────────
(import_statement
  name: (dotted_name) @import.name) @import

(import_from_statement
  module_name: (dotted_name) @import.module) @import

; ── Calls ─────────────────────────────────────────────────────
(call
  function: (identifier) @call.name) @call

(call
  function: (attribute
    attribute: (identifier) @call.method)) @call

; ── Implementations (class base list) ─────────────────────────
(class_definition
  superclasses: (argument_list
    (identifier) @implements.parent)) @implements
