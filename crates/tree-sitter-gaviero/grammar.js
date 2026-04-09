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
        seq("max_retries", $.integer),
        $.memory_block,
        $.context_block
      ),

    // ── Scope block ──────────────────────────────────────────────
    scope_block: ($) =>
      seq("scope", "{", repeat($.scope_field), "}"),

    scope_field: ($) =>
      choice(
        seq("owned", $.string_list),
        seq("read_only", $.string_list),
        seq("impact_scope", $.boolean)
      ),

    // ── Memory block ─────────────────────────────────────────────
    memory_block: ($) =>
      seq("memory", "{", repeat($.memory_field), "}"),

    memory_field: ($) =>
      choice(
        seq("read_ns", $.string_list),
        seq("write_ns", $._string_value),
        seq("importance", $.float),
        seq("staleness_sources", $.string_list),
        seq("read_query", $._string_value),
        seq("read_limit", $.integer),
        seq("write_content", $._string_value)
      ),

    // ── Verify block ─────────────────────────────────────────────
    verify_block: ($) =>
      seq("verify", "{", repeat($.verify_field), "}"),

    verify_field: ($) =>
      choice(
        seq("compile", $.boolean),
        seq("clippy", $.boolean),
        seq("test", $.boolean),
        seq("impact_tests", $.boolean)
      ),

    // ── Context block ────────────────────────────────────────────
    context_block: ($) =>
      seq("context", "{", repeat($.context_field), "}"),

    context_field: ($) =>
      choice(
        seq("callers_of", $.string_list),
        seq("tests_for", $.string_list),
        seq("depth", $.integer)
      ),

    // ── Loop block ───────────────────────────────────────────────
    loop_block: ($) =>
      seq("loop", "{", repeat($.loop_field), "}"),

    loop_field: ($) =>
      choice(
        seq("agents", $.identifier_list),
        $.until_clause,
        seq("max_iterations", $.integer)
      ),

    // ── Until clause (3 variants) ────────────────────────────────
    until_clause: ($) => seq("until", $._until_condition),

    _until_condition: ($) =>
      choice($.until_verify, $.until_agent, $.until_command),

    until_verify: ($) => seq("{", repeat($.verify_field), "}"),

    until_agent: ($) => seq("agent", $.identifier),

    until_command: ($) => seq("command", $._string_value),

    // ── Workflow ─────────────────────────────────────────────────
    workflow_declaration: ($) =>
      seq("workflow", $.identifier, "{", repeat($.workflow_field), "}"),

    workflow_field: ($) =>
      choice(
        seq("steps", $.step_list),
        seq("max_parallel", $.integer),
        $.memory_block,
        seq("strategy", $.strategy_value),
        seq("test_first", $.boolean),
        seq("max_retries", $.integer),
        seq("attempts", $.integer),
        seq("escalate_after", $.integer),
        $.verify_block
      ),

    // ── Step list (identifiers + loop blocks) ────────────────────
    step_list: ($) => seq("[", repeat(choice($.loop_block, $.identifier)), "]"),

    // ── Lists ────────────────────────────────────────────────────
    string_list: ($) => seq("[", repeat($._string_value), "]"),

    identifier_list: ($) => seq("[", repeat($.identifier), "]"),

    // ── Values ───────────────────────────────────────────────────
    tier_value: (_) =>
      choice(
        "cheap",
        "expensive",
        "coordinator",
        "reasoning",
        "execution",
        "mechanical"
      ),

    privacy_value: (_) => choice("public", "local_only"),

    strategy_value: ($) => choice("single_pass", "refine", $.identifier),

    boolean: (_) => choice("true", "false"),

    // ── Literals ─────────────────────────────────────────────────
    _string_value: ($) => choice($.string, $.raw_string),

    string: (_) => token(seq('"', /[^"]*/, '"')),

    // Raw string: #"..."#
    raw_string: (_) =>
      token(
        seq(
          "#\"",
          repeat(choice(/[^"\n]/, /\n/, seq("\"", /[^#\n]/), seq("\"", /\n/))),
          "\"#"
        )
      ),

    float: (_) => token(/[0-9]+\.[0-9]+/),

    integer: (_) => token(/[0-9]+/),

    identifier: (_) => token(/[a-zA-Z_][a-zA-Z0-9_-]*/),
  },
});
