; ── Declaration keywords ──────────────────────────────────────
"client" @keyword
"agent" @keyword
"workflow" @keyword

; ── Declaration names ─────────────────────────────────────────
(client_declaration (identifier) @type)
(agent_declaration (identifier) @type)
(workflow_declaration (identifier) @type)

; ── Field keywords ────────────────────────────────────────────
"tier" @keyword
"model" @keyword
"privacy" @keyword
"scope" @keyword
"owned" @keyword
"read_only" @keyword
"depends_on" @keyword
"prompt" @keyword
"description" @keyword
"max_retries" @keyword
"steps" @keyword
"max_parallel" @keyword

; ── Memory block keywords ─────────────────────────────────────
"memory" @keyword
"read_ns" @keyword
"write_ns" @keyword
"importance" @keyword
"staleness_sources" @keyword
"read_query" @keyword
"read_limit" @keyword
"write_content" @keyword

; ── Verify block keywords ─────────────────────────────────────
"verify" @keyword
"compile" @keyword
"clippy" @keyword
"test" @keyword
"impact_tests" @keyword

; ── Context block keywords ────────────────────────────────────
"context" @keyword
"callers_of" @keyword
"tests_for" @keyword
"depth" @keyword

; ── Loop block keywords ───────────────────────────────────────
"loop" @keyword
"until" @keyword
"agents" @keyword
"max_iterations" @keyword
"command" @keyword

; ── Scope keywords ────────────────────────────────────────────
"impact_scope" @keyword

; ── Workflow strategy keywords ────────────────────────────────
"strategy" @keyword
"test_first" @keyword
"attempts" @keyword
"escalate_after" @keyword

; ── Tier values ───────────────────────────────────────────────
(tier_value) @constant.builtin

; ── Privacy values ────────────────────────────────────────────
(privacy_value) @constant.builtin

; ── Strategy values ───────────────────────────────────────────
(strategy_value) @constant.builtin

; ── Boolean values ────────────────────────────────────────────
(boolean) @constant.builtin

; ── Identifiers in lists and references ───────────────────────
(identifier_list (identifier) @variable)
(step_list (identifier) @variable)
(agent_field (identifier) @variable)
(until_agent (identifier) @variable)

; ── Strings ───────────────────────────────────────────────────
(string) @string
(raw_string) @string

; ── Numbers ───────────────────────────────────────────────────
(integer) @number
(float) @number

; ── Comments ──────────────────────────────────────────────────
(comment) @comment

; ── Punctuation ───────────────────────────────────────────────
"{" @punctuation.bracket
"}" @punctuation.bracket
"[" @punctuation.bracket
"]" @punctuation.bracket
