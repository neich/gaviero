; ── Declaration keywords ──────────────────────────────────────
"client" @keyword
"agent" @keyword
"workflow" @keyword
"prompt" @keyword
"vars" @keyword
"include" @keyword.import

; ── Declaration names ─────────────────────────────────────────
(client_declaration (identifier) @type)
(agent_declaration (identifier) @type)
(workflow_declaration (identifier) @type)
(prompt_declaration (identifier) @type)
(tier_alias_declaration (tier_alias_name) @type)
(tier_alias_declaration (identifier) @variable)

; ── Field keywords ────────────────────────────────────────────
"tier" @keyword
"model" @keyword
"effort" @keyword
"extra" @keyword
"default" @keyword.modifier
"privacy" @keyword
"scope" @keyword
"owned" @keyword
"read_only" @keyword
"depends_on" @keyword
"description" @keyword
"max_retries" @keyword
"steps" @keyword
"max_parallel" @keyword
"param" @keyword
"template" @keyword
"id" @keyword

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
"reviewers" @keyword
"template_init" @keyword
"template_refine" @keyword
"consensus_mode" @keyword
"max_iterations" @keyword
"iter_start" @keyword
"stability" @keyword
"judge_timeout" @keyword
"strict_judge" @keyword
"branch_chain" @keyword
"command" @keyword

; ── Scope keywords ────────────────────────────────────────────
"impact_scope" @keyword

; ── Workflow strategy keywords ────────────────────────────────
"strategy" @keyword
"test_first" @keyword
"attempts" @keyword
"escalate_after" @keyword
"execution_mode" @keyword

; ── Tier values ───────────────────────────────────────────────
(tier_value) @constant.builtin

; ── Privacy values ────────────────────────────────────────────
(privacy_value) @constant.builtin

; ── Strategy values ───────────────────────────────────────────
(strategy_value) @constant.builtin

; ── Execution mode values ─────────────────────────────────────
(execution_mode_value) @constant.builtin

; ── Boolean values ────────────────────────────────────────────
(boolean) @constant.builtin

; ── Identifiers in lists and references ───────────────────────
(identifier_list (identifier) @variable)
(step_list (identifier) @variable)
(agent_field (identifier) @variable)
(agent_field (tier_alias_name) @variable)
(until_agent (identifier) @variable)

; ── Vars / extra pair keys ────────────────────────────────────
(vars_pair (identifier) @property)
(extra_pair (identifier) @property)

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
