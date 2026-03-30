/// <reference types="tree-sitter-cli/dsl" />

module.exports = grammar({
  name: "gaviero",

  extras: ($) => [/\s/, $.comment],

  rules: {
    source_file: ($) => repeat($._definition),

    _definition: ($) =>
      choice($.client_declaration, $.agent_declaration, $.workflow_declaration),

    // ── Comments ─────────────────────────────────────────────────
    comment: (_) => token(seq("//", /.*/)),

    // ── Client ───────────────────────────────────────────────────
    client_declaration: ($) =>
      seq("client", $.identifier, "{", repeat($.client_field), "}"),

    client_field: ($) =>
      choice(
        seq("tier", $.tier_value),
        seq("model", $._string_value),
        seq("privacy", $.privacy_value)
      ),

    // ── Agent ────────────────────────────────────────────────────
    agent_declaration: ($) =>
      seq("agent", $.identifier, "{", repeat($.agent_field), "}"),

    agent_field: ($) =>
      choice(
        seq("description", $._string_value),
        seq("client", $.identifier),
        $.scope_block,
        seq("depends_on", $.identifier_list),
        seq("prompt", $._string_value),
        seq("max_retries", $.integer)
      ),

    scope_block: ($) =>
      seq("scope", "{", repeat($.scope_field), "}"),

    scope_field: ($) =>
      choice(
        seq("owned", $.string_list),
        seq("read_only", $.string_list)
      ),

    // ── Workflow ─────────────────────────────────────────────────
    workflow_declaration: ($) =>
      seq("workflow", $.identifier, "{", repeat($.workflow_field), "}"),

    workflow_field: ($) =>
      choice(
        seq("steps", $.identifier_list),
        seq("max_parallel", $.integer)
      ),

    // ── Lists ────────────────────────────────────────────────────
    string_list: ($) => seq("[", repeat($._string_value), "]"),

    identifier_list: ($) => seq("[", repeat($.identifier), "]"),

    // ── Values ───────────────────────────────────────────────────
    tier_value: (_) =>
      choice("coordinator", "reasoning", "execution", "mechanical"),

    privacy_value: (_) => choice("public", "local_only"),

    // ── Literals ─────────────────────────────────────────────────
    _string_value: ($) => choice($.string, $.raw_string),

    string: (_) => token(seq('"', /[^"]*/, '"')),

    // Raw string: #"..."#
    // Match #" then any chars (including newlines) then "#
    // [^"] matches non-quote chars, /"[^#]/ matches quote-not-before-hash
    raw_string: (_) =>
      token(
        seq(
          "#\"",
          repeat(choice(/[^"\n]/, /\n/, seq("\"", /[^#\n]/), seq("\"", /\n/))),
          "\"#"
        )
      ),

    integer: (_) => token(/[0-9]+/),

    identifier: (_) => token(/[a-zA-Z_][a-zA-Z0-9_-]*/),
  },
});
