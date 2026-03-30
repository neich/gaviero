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

; ── Tier values ───────────────────────────────────────────────
(tier_value) @constant.builtin

; ── Privacy values ────────────────────────────────────────────
(privacy_value) @constant.builtin

; ── Identifiers in lists and references ───────────────────────
(identifier_list (identifier) @variable)
(agent_field (identifier) @variable)

; ── Strings ───────────────────────────────────────────────────
(string) @string
(raw_string) @string

; ── Numbers ───────────────────────────────────────────────────
(integer) @number

; ── Comments ──────────────────────────────────────────────────
(comment) @comment

; ── Punctuation ───────────────────────────────────────────────
"{" @punctuation.bracket
"}" @punctuation.bracket
"[" @punctuation.bracket
"]" @punctuation.bracket
