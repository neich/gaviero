#include "tree_sitter/parser.h"

#if defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic ignored "-Wmissing-field-initializers"
#endif

#ifdef _MSC_VER
#pragma optimize("", off)
#elif defined(__clang__)
#pragma clang optimize off
#elif defined(__GNUC__)
#pragma GCC optimize ("O0")
#endif

#define LANGUAGE_VERSION 14
#define STATE_COUNT 170
#define LARGE_STATE_COUNT 2
#define SYMBOL_COUNT 148
#define ALIAS_COUNT 0
#define TOKEN_COUNT 87
#define EXTERNAL_TOKEN_COUNT 0
#define FIELD_COUNT 0
#define MAX_ALIAS_SEQUENCE_LENGTH 5
#define PRODUCTION_ID_COUNT 1

enum ts_symbol_identifiers {
  anon_sym_include = 1,
  sym_comment = 2,
  anon_sym_client = 3,
  anon_sym_LBRACE = 4,
  anon_sym_RBRACE = 5,
  anon_sym_tier = 6,
  anon_sym_model = 7,
  anon_sym_effort = 8,
  anon_sym_privacy = 9,
  anon_sym_default = 10,
  anon_sym_extra = 11,
  anon_sym_vars = 12,
  anon_sym_cheap = 13,
  anon_sym_expensive = 14,
  anon_sym_coordinator = 15,
  anon_sym_reasoning = 16,
  anon_sym_execution = 17,
  anon_sym_mechanical = 18,
  anon_sym_prompt = 19,
  anon_sym_agent = 20,
  anon_sym_description = 21,
  anon_sym_depends_on = 22,
  anon_sym_max_retries = 23,
  anon_sym_tools = 24,
  anon_sym_template = 25,
  anon_sym_scope = 26,
  anon_sym_owned = 27,
  anon_sym_read_only = 28,
  anon_sym_impact_scope = 29,
  anon_sym_memory = 30,
  anon_sym_read_ns = 31,
  anon_sym_write_ns = 32,
  anon_sym_importance = 33,
  anon_sym_staleness_sources = 34,
  anon_sym_read_query = 35,
  anon_sym_read_limit = 36,
  anon_sym_write_content = 37,
  anon_sym_verify = 38,
  anon_sym_compile = 39,
  anon_sym_clippy = 40,
  anon_sym_test = 41,
  anon_sym_impact_tests = 42,
  anon_sym_context = 43,
  anon_sym_callers_of = 44,
  anon_sym_tests_for = 45,
  anon_sym_depth = 46,
  anon_sym_loop = 47,
  anon_sym_agents = 48,
  anon_sym_reviewers = 49,
  anon_sym_template_init = 50,
  anon_sym_template_refine = 51,
  anon_sym_consensus_mode = 52,
  anon_sym_max_iterations = 53,
  anon_sym_iter_start = 54,
  anon_sym_stability = 55,
  anon_sym_judge_timeout = 56,
  anon_sym_strict_judge = 57,
  anon_sym_branch_chain = 58,
  anon_sym_LBRACK = 59,
  anon_sym_RBRACK = 60,
  anon_sym_id = 61,
  anon_sym_strict = 62,
  anon_sym_partial_ok = 63,
  anon_sym_explore = 64,
  anon_sym_stacked = 65,
  anon_sym_none = 66,
  anon_sym_until = 67,
  anon_sym_command = 68,
  anon_sym_workflow = 69,
  anon_sym_steps = 70,
  anon_sym_max_parallel = 71,
  anon_sym_strategy = 72,
  anon_sym_test_first = 73,
  anon_sym_attempts = 74,
  anon_sym_escalate_after = 75,
  anon_sym_public = 76,
  anon_sym_local_only = 77,
  anon_sym_single_pass = 78,
  anon_sym_refine = 79,
  anon_sym_true = 80,
  anon_sym_false = 81,
  sym_string = 82,
  sym_raw_string = 83,
  sym_float = 84,
  sym_integer = 85,
  sym_identifier = 86,
  sym_source_file = 87,
  sym__definition = 88,
  sym_include_declaration = 89,
  sym_client_declaration = 90,
  sym_client_field = 91,
  sym__effort_value = 92,
  sym_extra_block = 93,
  sym_extra_pair = 94,
  sym_vars_block = 95,
  sym_vars_pair = 96,
  sym_tier_alias_declaration = 97,
  sym_tier_alias_name = 98,
  sym_prompt_declaration = 99,
  sym_agent_declaration = 100,
  sym_agent_field = 101,
  sym_scope_block = 102,
  sym_scope_field = 103,
  sym_memory_block = 104,
  sym_memory_field = 105,
  sym_verify_block = 106,
  sym_verify_field = 107,
  sym_context_block = 108,
  sym_context_field = 109,
  sym_loop_block = 110,
  sym_loop_field = 111,
  sym_reviewer_list = 112,
  sym_reviewer_entry = 113,
  sym_reviewer_field = 114,
  sym_consensus_mode_value = 115,
  sym_branch_chain_value = 116,
  sym_until_clause = 117,
  sym__until_condition = 118,
  sym_until_verify = 119,
  sym_until_agent = 120,
  sym_until_command = 121,
  sym_workflow_declaration = 122,
  sym_workflow_field = 123,
  sym_step_list = 124,
  sym_string_list = 125,
  sym_identifier_list = 126,
  sym_tier_value = 127,
  sym_privacy_value = 128,
  sym_strategy_value = 129,
  sym_boolean = 130,
  sym__string_value = 131,
  aux_sym_source_file_repeat1 = 132,
  aux_sym_client_declaration_repeat1 = 133,
  aux_sym_extra_block_repeat1 = 134,
  aux_sym_vars_block_repeat1 = 135,
  aux_sym_agent_declaration_repeat1 = 136,
  aux_sym_scope_block_repeat1 = 137,
  aux_sym_memory_block_repeat1 = 138,
  aux_sym_verify_block_repeat1 = 139,
  aux_sym_context_block_repeat1 = 140,
  aux_sym_loop_block_repeat1 = 141,
  aux_sym_reviewer_list_repeat1 = 142,
  aux_sym_reviewer_entry_repeat1 = 143,
  aux_sym_workflow_declaration_repeat1 = 144,
  aux_sym_step_list_repeat1 = 145,
  aux_sym_string_list_repeat1 = 146,
  aux_sym_identifier_list_repeat1 = 147,
};

static const char * const ts_symbol_names[] = {
  [ts_builtin_sym_end] = "end",
  [anon_sym_include] = "include",
  [sym_comment] = "comment",
  [anon_sym_client] = "client",
  [anon_sym_LBRACE] = "{",
  [anon_sym_RBRACE] = "}",
  [anon_sym_tier] = "tier",
  [anon_sym_model] = "model",
  [anon_sym_effort] = "effort",
  [anon_sym_privacy] = "privacy",
  [anon_sym_default] = "default",
  [anon_sym_extra] = "extra",
  [anon_sym_vars] = "vars",
  [anon_sym_cheap] = "cheap",
  [anon_sym_expensive] = "expensive",
  [anon_sym_coordinator] = "coordinator",
  [anon_sym_reasoning] = "reasoning",
  [anon_sym_execution] = "execution",
  [anon_sym_mechanical] = "mechanical",
  [anon_sym_prompt] = "prompt",
  [anon_sym_agent] = "agent",
  [anon_sym_description] = "description",
  [anon_sym_depends_on] = "depends_on",
  [anon_sym_max_retries] = "max_retries",
  [anon_sym_tools] = "tools",
  [anon_sym_template] = "template",
  [anon_sym_scope] = "scope",
  [anon_sym_owned] = "owned",
  [anon_sym_read_only] = "read_only",
  [anon_sym_impact_scope] = "impact_scope",
  [anon_sym_memory] = "memory",
  [anon_sym_read_ns] = "read_ns",
  [anon_sym_write_ns] = "write_ns",
  [anon_sym_importance] = "importance",
  [anon_sym_staleness_sources] = "staleness_sources",
  [anon_sym_read_query] = "read_query",
  [anon_sym_read_limit] = "read_limit",
  [anon_sym_write_content] = "write_content",
  [anon_sym_verify] = "verify",
  [anon_sym_compile] = "compile",
  [anon_sym_clippy] = "clippy",
  [anon_sym_test] = "test",
  [anon_sym_impact_tests] = "impact_tests",
  [anon_sym_context] = "context",
  [anon_sym_callers_of] = "callers_of",
  [anon_sym_tests_for] = "tests_for",
  [anon_sym_depth] = "depth",
  [anon_sym_loop] = "loop",
  [anon_sym_agents] = "agents",
  [anon_sym_reviewers] = "reviewers",
  [anon_sym_template_init] = "template_init",
  [anon_sym_template_refine] = "template_refine",
  [anon_sym_consensus_mode] = "consensus_mode",
  [anon_sym_max_iterations] = "max_iterations",
  [anon_sym_iter_start] = "iter_start",
  [anon_sym_stability] = "stability",
  [anon_sym_judge_timeout] = "judge_timeout",
  [anon_sym_strict_judge] = "strict_judge",
  [anon_sym_branch_chain] = "branch_chain",
  [anon_sym_LBRACK] = "[",
  [anon_sym_RBRACK] = "]",
  [anon_sym_id] = "id",
  [anon_sym_strict] = "strict",
  [anon_sym_partial_ok] = "partial_ok",
  [anon_sym_explore] = "explore",
  [anon_sym_stacked] = "stacked",
  [anon_sym_none] = "none",
  [anon_sym_until] = "until",
  [anon_sym_command] = "command",
  [anon_sym_workflow] = "workflow",
  [anon_sym_steps] = "steps",
  [anon_sym_max_parallel] = "max_parallel",
  [anon_sym_strategy] = "strategy",
  [anon_sym_test_first] = "test_first",
  [anon_sym_attempts] = "attempts",
  [anon_sym_escalate_after] = "escalate_after",
  [anon_sym_public] = "public",
  [anon_sym_local_only] = "local_only",
  [anon_sym_single_pass] = "single_pass",
  [anon_sym_refine] = "refine",
  [anon_sym_true] = "true",
  [anon_sym_false] = "false",
  [sym_string] = "string",
  [sym_raw_string] = "raw_string",
  [sym_float] = "float",
  [sym_integer] = "integer",
  [sym_identifier] = "identifier",
  [sym_source_file] = "source_file",
  [sym__definition] = "_definition",
  [sym_include_declaration] = "include_declaration",
  [sym_client_declaration] = "client_declaration",
  [sym_client_field] = "client_field",
  [sym__effort_value] = "_effort_value",
  [sym_extra_block] = "extra_block",
  [sym_extra_pair] = "extra_pair",
  [sym_vars_block] = "vars_block",
  [sym_vars_pair] = "vars_pair",
  [sym_tier_alias_declaration] = "tier_alias_declaration",
  [sym_tier_alias_name] = "tier_alias_name",
  [sym_prompt_declaration] = "prompt_declaration",
  [sym_agent_declaration] = "agent_declaration",
  [sym_agent_field] = "agent_field",
  [sym_scope_block] = "scope_block",
  [sym_scope_field] = "scope_field",
  [sym_memory_block] = "memory_block",
  [sym_memory_field] = "memory_field",
  [sym_verify_block] = "verify_block",
  [sym_verify_field] = "verify_field",
  [sym_context_block] = "context_block",
  [sym_context_field] = "context_field",
  [sym_loop_block] = "loop_block",
  [sym_loop_field] = "loop_field",
  [sym_reviewer_list] = "reviewer_list",
  [sym_reviewer_entry] = "reviewer_entry",
  [sym_reviewer_field] = "reviewer_field",
  [sym_consensus_mode_value] = "consensus_mode_value",
  [sym_branch_chain_value] = "branch_chain_value",
  [sym_until_clause] = "until_clause",
  [sym__until_condition] = "_until_condition",
  [sym_until_verify] = "until_verify",
  [sym_until_agent] = "until_agent",
  [sym_until_command] = "until_command",
  [sym_workflow_declaration] = "workflow_declaration",
  [sym_workflow_field] = "workflow_field",
  [sym_step_list] = "step_list",
  [sym_string_list] = "string_list",
  [sym_identifier_list] = "identifier_list",
  [sym_tier_value] = "tier_value",
  [sym_privacy_value] = "privacy_value",
  [sym_strategy_value] = "strategy_value",
  [sym_boolean] = "boolean",
  [sym__string_value] = "_string_value",
  [aux_sym_source_file_repeat1] = "source_file_repeat1",
  [aux_sym_client_declaration_repeat1] = "client_declaration_repeat1",
  [aux_sym_extra_block_repeat1] = "extra_block_repeat1",
  [aux_sym_vars_block_repeat1] = "vars_block_repeat1",
  [aux_sym_agent_declaration_repeat1] = "agent_declaration_repeat1",
  [aux_sym_scope_block_repeat1] = "scope_block_repeat1",
  [aux_sym_memory_block_repeat1] = "memory_block_repeat1",
  [aux_sym_verify_block_repeat1] = "verify_block_repeat1",
  [aux_sym_context_block_repeat1] = "context_block_repeat1",
  [aux_sym_loop_block_repeat1] = "loop_block_repeat1",
  [aux_sym_reviewer_list_repeat1] = "reviewer_list_repeat1",
  [aux_sym_reviewer_entry_repeat1] = "reviewer_entry_repeat1",
  [aux_sym_workflow_declaration_repeat1] = "workflow_declaration_repeat1",
  [aux_sym_step_list_repeat1] = "step_list_repeat1",
  [aux_sym_string_list_repeat1] = "string_list_repeat1",
  [aux_sym_identifier_list_repeat1] = "identifier_list_repeat1",
};

static const TSSymbol ts_symbol_map[] = {
  [ts_builtin_sym_end] = ts_builtin_sym_end,
  [anon_sym_include] = anon_sym_include,
  [sym_comment] = sym_comment,
  [anon_sym_client] = anon_sym_client,
  [anon_sym_LBRACE] = anon_sym_LBRACE,
  [anon_sym_RBRACE] = anon_sym_RBRACE,
  [anon_sym_tier] = anon_sym_tier,
  [anon_sym_model] = anon_sym_model,
  [anon_sym_effort] = anon_sym_effort,
  [anon_sym_privacy] = anon_sym_privacy,
  [anon_sym_default] = anon_sym_default,
  [anon_sym_extra] = anon_sym_extra,
  [anon_sym_vars] = anon_sym_vars,
  [anon_sym_cheap] = anon_sym_cheap,
  [anon_sym_expensive] = anon_sym_expensive,
  [anon_sym_coordinator] = anon_sym_coordinator,
  [anon_sym_reasoning] = anon_sym_reasoning,
  [anon_sym_execution] = anon_sym_execution,
  [anon_sym_mechanical] = anon_sym_mechanical,
  [anon_sym_prompt] = anon_sym_prompt,
  [anon_sym_agent] = anon_sym_agent,
  [anon_sym_description] = anon_sym_description,
  [anon_sym_depends_on] = anon_sym_depends_on,
  [anon_sym_max_retries] = anon_sym_max_retries,
  [anon_sym_tools] = anon_sym_tools,
  [anon_sym_template] = anon_sym_template,
  [anon_sym_scope] = anon_sym_scope,
  [anon_sym_owned] = anon_sym_owned,
  [anon_sym_read_only] = anon_sym_read_only,
  [anon_sym_impact_scope] = anon_sym_impact_scope,
  [anon_sym_memory] = anon_sym_memory,
  [anon_sym_read_ns] = anon_sym_read_ns,
  [anon_sym_write_ns] = anon_sym_write_ns,
  [anon_sym_importance] = anon_sym_importance,
  [anon_sym_staleness_sources] = anon_sym_staleness_sources,
  [anon_sym_read_query] = anon_sym_read_query,
  [anon_sym_read_limit] = anon_sym_read_limit,
  [anon_sym_write_content] = anon_sym_write_content,
  [anon_sym_verify] = anon_sym_verify,
  [anon_sym_compile] = anon_sym_compile,
  [anon_sym_clippy] = anon_sym_clippy,
  [anon_sym_test] = anon_sym_test,
  [anon_sym_impact_tests] = anon_sym_impact_tests,
  [anon_sym_context] = anon_sym_context,
  [anon_sym_callers_of] = anon_sym_callers_of,
  [anon_sym_tests_for] = anon_sym_tests_for,
  [anon_sym_depth] = anon_sym_depth,
  [anon_sym_loop] = anon_sym_loop,
  [anon_sym_agents] = anon_sym_agents,
  [anon_sym_reviewers] = anon_sym_reviewers,
  [anon_sym_template_init] = anon_sym_template_init,
  [anon_sym_template_refine] = anon_sym_template_refine,
  [anon_sym_consensus_mode] = anon_sym_consensus_mode,
  [anon_sym_max_iterations] = anon_sym_max_iterations,
  [anon_sym_iter_start] = anon_sym_iter_start,
  [anon_sym_stability] = anon_sym_stability,
  [anon_sym_judge_timeout] = anon_sym_judge_timeout,
  [anon_sym_strict_judge] = anon_sym_strict_judge,
  [anon_sym_branch_chain] = anon_sym_branch_chain,
  [anon_sym_LBRACK] = anon_sym_LBRACK,
  [anon_sym_RBRACK] = anon_sym_RBRACK,
  [anon_sym_id] = anon_sym_id,
  [anon_sym_strict] = anon_sym_strict,
  [anon_sym_partial_ok] = anon_sym_partial_ok,
  [anon_sym_explore] = anon_sym_explore,
  [anon_sym_stacked] = anon_sym_stacked,
  [anon_sym_none] = anon_sym_none,
  [anon_sym_until] = anon_sym_until,
  [anon_sym_command] = anon_sym_command,
  [anon_sym_workflow] = anon_sym_workflow,
  [anon_sym_steps] = anon_sym_steps,
  [anon_sym_max_parallel] = anon_sym_max_parallel,
  [anon_sym_strategy] = anon_sym_strategy,
  [anon_sym_test_first] = anon_sym_test_first,
  [anon_sym_attempts] = anon_sym_attempts,
  [anon_sym_escalate_after] = anon_sym_escalate_after,
  [anon_sym_public] = anon_sym_public,
  [anon_sym_local_only] = anon_sym_local_only,
  [anon_sym_single_pass] = anon_sym_single_pass,
  [anon_sym_refine] = anon_sym_refine,
  [anon_sym_true] = anon_sym_true,
  [anon_sym_false] = anon_sym_false,
  [sym_string] = sym_string,
  [sym_raw_string] = sym_raw_string,
  [sym_float] = sym_float,
  [sym_integer] = sym_integer,
  [sym_identifier] = sym_identifier,
  [sym_source_file] = sym_source_file,
  [sym__definition] = sym__definition,
  [sym_include_declaration] = sym_include_declaration,
  [sym_client_declaration] = sym_client_declaration,
  [sym_client_field] = sym_client_field,
  [sym__effort_value] = sym__effort_value,
  [sym_extra_block] = sym_extra_block,
  [sym_extra_pair] = sym_extra_pair,
  [sym_vars_block] = sym_vars_block,
  [sym_vars_pair] = sym_vars_pair,
  [sym_tier_alias_declaration] = sym_tier_alias_declaration,
  [sym_tier_alias_name] = sym_tier_alias_name,
  [sym_prompt_declaration] = sym_prompt_declaration,
  [sym_agent_declaration] = sym_agent_declaration,
  [sym_agent_field] = sym_agent_field,
  [sym_scope_block] = sym_scope_block,
  [sym_scope_field] = sym_scope_field,
  [sym_memory_block] = sym_memory_block,
  [sym_memory_field] = sym_memory_field,
  [sym_verify_block] = sym_verify_block,
  [sym_verify_field] = sym_verify_field,
  [sym_context_block] = sym_context_block,
  [sym_context_field] = sym_context_field,
  [sym_loop_block] = sym_loop_block,
  [sym_loop_field] = sym_loop_field,
  [sym_reviewer_list] = sym_reviewer_list,
  [sym_reviewer_entry] = sym_reviewer_entry,
  [sym_reviewer_field] = sym_reviewer_field,
  [sym_consensus_mode_value] = sym_consensus_mode_value,
  [sym_branch_chain_value] = sym_branch_chain_value,
  [sym_until_clause] = sym_until_clause,
  [sym__until_condition] = sym__until_condition,
  [sym_until_verify] = sym_until_verify,
  [sym_until_agent] = sym_until_agent,
  [sym_until_command] = sym_until_command,
  [sym_workflow_declaration] = sym_workflow_declaration,
  [sym_workflow_field] = sym_workflow_field,
  [sym_step_list] = sym_step_list,
  [sym_string_list] = sym_string_list,
  [sym_identifier_list] = sym_identifier_list,
  [sym_tier_value] = sym_tier_value,
  [sym_privacy_value] = sym_privacy_value,
  [sym_strategy_value] = sym_strategy_value,
  [sym_boolean] = sym_boolean,
  [sym__string_value] = sym__string_value,
  [aux_sym_source_file_repeat1] = aux_sym_source_file_repeat1,
  [aux_sym_client_declaration_repeat1] = aux_sym_client_declaration_repeat1,
  [aux_sym_extra_block_repeat1] = aux_sym_extra_block_repeat1,
  [aux_sym_vars_block_repeat1] = aux_sym_vars_block_repeat1,
  [aux_sym_agent_declaration_repeat1] = aux_sym_agent_declaration_repeat1,
  [aux_sym_scope_block_repeat1] = aux_sym_scope_block_repeat1,
  [aux_sym_memory_block_repeat1] = aux_sym_memory_block_repeat1,
  [aux_sym_verify_block_repeat1] = aux_sym_verify_block_repeat1,
  [aux_sym_context_block_repeat1] = aux_sym_context_block_repeat1,
  [aux_sym_loop_block_repeat1] = aux_sym_loop_block_repeat1,
  [aux_sym_reviewer_list_repeat1] = aux_sym_reviewer_list_repeat1,
  [aux_sym_reviewer_entry_repeat1] = aux_sym_reviewer_entry_repeat1,
  [aux_sym_workflow_declaration_repeat1] = aux_sym_workflow_declaration_repeat1,
  [aux_sym_step_list_repeat1] = aux_sym_step_list_repeat1,
  [aux_sym_string_list_repeat1] = aux_sym_string_list_repeat1,
  [aux_sym_identifier_list_repeat1] = aux_sym_identifier_list_repeat1,
};

static const TSSymbolMetadata ts_symbol_metadata[] = {
  [ts_builtin_sym_end] = {
    .visible = false,
    .named = true,
  },
  [anon_sym_include] = {
    .visible = true,
    .named = false,
  },
  [sym_comment] = {
    .visible = true,
    .named = true,
  },
  [anon_sym_client] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_LBRACE] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_RBRACE] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_tier] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_model] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_effort] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_privacy] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_default] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_extra] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_vars] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_cheap] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_expensive] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_coordinator] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_reasoning] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_execution] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_mechanical] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_prompt] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_agent] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_description] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_depends_on] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_max_retries] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_tools] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_template] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_scope] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_owned] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_read_only] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_impact_scope] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_memory] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_read_ns] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_write_ns] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_importance] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_staleness_sources] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_read_query] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_read_limit] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_write_content] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_verify] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_compile] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_clippy] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_test] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_impact_tests] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_context] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_callers_of] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_tests_for] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_depth] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_loop] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_agents] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_reviewers] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_template_init] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_template_refine] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_consensus_mode] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_max_iterations] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_iter_start] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_stability] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_judge_timeout] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_strict_judge] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_branch_chain] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_LBRACK] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_RBRACK] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_id] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_strict] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_partial_ok] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_explore] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_stacked] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_none] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_until] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_command] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_workflow] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_steps] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_max_parallel] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_strategy] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_test_first] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_attempts] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_escalate_after] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_public] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_local_only] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_single_pass] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_refine] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_true] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_false] = {
    .visible = true,
    .named = false,
  },
  [sym_string] = {
    .visible = true,
    .named = true,
  },
  [sym_raw_string] = {
    .visible = true,
    .named = true,
  },
  [sym_float] = {
    .visible = true,
    .named = true,
  },
  [sym_integer] = {
    .visible = true,
    .named = true,
  },
  [sym_identifier] = {
    .visible = true,
    .named = true,
  },
  [sym_source_file] = {
    .visible = true,
    .named = true,
  },
  [sym__definition] = {
    .visible = false,
    .named = true,
  },
  [sym_include_declaration] = {
    .visible = true,
    .named = true,
  },
  [sym_client_declaration] = {
    .visible = true,
    .named = true,
  },
  [sym_client_field] = {
    .visible = true,
    .named = true,
  },
  [sym__effort_value] = {
    .visible = false,
    .named = true,
  },
  [sym_extra_block] = {
    .visible = true,
    .named = true,
  },
  [sym_extra_pair] = {
    .visible = true,
    .named = true,
  },
  [sym_vars_block] = {
    .visible = true,
    .named = true,
  },
  [sym_vars_pair] = {
    .visible = true,
    .named = true,
  },
  [sym_tier_alias_declaration] = {
    .visible = true,
    .named = true,
  },
  [sym_tier_alias_name] = {
    .visible = true,
    .named = true,
  },
  [sym_prompt_declaration] = {
    .visible = true,
    .named = true,
  },
  [sym_agent_declaration] = {
    .visible = true,
    .named = true,
  },
  [sym_agent_field] = {
    .visible = true,
    .named = true,
  },
  [sym_scope_block] = {
    .visible = true,
    .named = true,
  },
  [sym_scope_field] = {
    .visible = true,
    .named = true,
  },
  [sym_memory_block] = {
    .visible = true,
    .named = true,
  },
  [sym_memory_field] = {
    .visible = true,
    .named = true,
  },
  [sym_verify_block] = {
    .visible = true,
    .named = true,
  },
  [sym_verify_field] = {
    .visible = true,
    .named = true,
  },
  [sym_context_block] = {
    .visible = true,
    .named = true,
  },
  [sym_context_field] = {
    .visible = true,
    .named = true,
  },
  [sym_loop_block] = {
    .visible = true,
    .named = true,
  },
  [sym_loop_field] = {
    .visible = true,
    .named = true,
  },
  [sym_reviewer_list] = {
    .visible = true,
    .named = true,
  },
  [sym_reviewer_entry] = {
    .visible = true,
    .named = true,
  },
  [sym_reviewer_field] = {
    .visible = true,
    .named = true,
  },
  [sym_consensus_mode_value] = {
    .visible = true,
    .named = true,
  },
  [sym_branch_chain_value] = {
    .visible = true,
    .named = true,
  },
  [sym_until_clause] = {
    .visible = true,
    .named = true,
  },
  [sym__until_condition] = {
    .visible = false,
    .named = true,
  },
  [sym_until_verify] = {
    .visible = true,
    .named = true,
  },
  [sym_until_agent] = {
    .visible = true,
    .named = true,
  },
  [sym_until_command] = {
    .visible = true,
    .named = true,
  },
  [sym_workflow_declaration] = {
    .visible = true,
    .named = true,
  },
  [sym_workflow_field] = {
    .visible = true,
    .named = true,
  },
  [sym_step_list] = {
    .visible = true,
    .named = true,
  },
  [sym_string_list] = {
    .visible = true,
    .named = true,
  },
  [sym_identifier_list] = {
    .visible = true,
    .named = true,
  },
  [sym_tier_value] = {
    .visible = true,
    .named = true,
  },
  [sym_privacy_value] = {
    .visible = true,
    .named = true,
  },
  [sym_strategy_value] = {
    .visible = true,
    .named = true,
  },
  [sym_boolean] = {
    .visible = true,
    .named = true,
  },
  [sym__string_value] = {
    .visible = false,
    .named = true,
  },
  [aux_sym_source_file_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_client_declaration_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_extra_block_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_vars_block_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_agent_declaration_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_scope_block_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_memory_block_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_verify_block_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_context_block_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_loop_block_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_reviewer_list_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_reviewer_entry_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_workflow_declaration_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_step_list_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_string_list_repeat1] = {
    .visible = false,
    .named = false,
  },
  [aux_sym_identifier_list_repeat1] = {
    .visible = false,
    .named = false,
  },
};

static const TSSymbol ts_alias_sequences[PRODUCTION_ID_COUNT][MAX_ALIAS_SEQUENCE_LENGTH] = {
  [0] = {0},
};

static const uint16_t ts_non_terminal_alias_map[] = {
  0,
};

static const TSStateId ts_primary_state_ids[STATE_COUNT] = {
  [0] = 0,
  [1] = 1,
  [2] = 2,
  [3] = 3,
  [4] = 4,
  [5] = 5,
  [6] = 6,
  [7] = 7,
  [8] = 8,
  [9] = 9,
  [10] = 10,
  [11] = 11,
  [12] = 12,
  [13] = 13,
  [14] = 14,
  [15] = 15,
  [16] = 16,
  [17] = 17,
  [18] = 18,
  [19] = 19,
  [20] = 20,
  [21] = 21,
  [22] = 22,
  [23] = 23,
  [24] = 24,
  [25] = 25,
  [26] = 26,
  [27] = 27,
  [28] = 28,
  [29] = 29,
  [30] = 30,
  [31] = 31,
  [32] = 32,
  [33] = 33,
  [34] = 34,
  [35] = 35,
  [36] = 36,
  [37] = 37,
  [38] = 38,
  [39] = 39,
  [40] = 40,
  [41] = 41,
  [42] = 42,
  [43] = 43,
  [44] = 44,
  [45] = 45,
  [46] = 46,
  [47] = 47,
  [48] = 48,
  [49] = 49,
  [50] = 50,
  [51] = 51,
  [52] = 52,
  [53] = 53,
  [54] = 54,
  [55] = 55,
  [56] = 56,
  [57] = 57,
  [58] = 58,
  [59] = 59,
  [60] = 60,
  [61] = 61,
  [62] = 62,
  [63] = 63,
  [64] = 64,
  [65] = 65,
  [66] = 66,
  [67] = 67,
  [68] = 68,
  [69] = 69,
  [70] = 70,
  [71] = 71,
  [72] = 72,
  [73] = 73,
  [74] = 74,
  [75] = 75,
  [76] = 76,
  [77] = 77,
  [78] = 78,
  [79] = 79,
  [80] = 80,
  [81] = 81,
  [82] = 82,
  [83] = 83,
  [84] = 84,
  [85] = 85,
  [86] = 86,
  [87] = 87,
  [88] = 88,
  [89] = 89,
  [90] = 90,
  [91] = 91,
  [92] = 92,
  [93] = 93,
  [94] = 94,
  [95] = 95,
  [96] = 96,
  [97] = 97,
  [98] = 98,
  [99] = 99,
  [100] = 100,
  [101] = 101,
  [102] = 102,
  [103] = 103,
  [104] = 104,
  [105] = 105,
  [106] = 106,
  [107] = 107,
  [108] = 108,
  [109] = 109,
  [110] = 110,
  [111] = 111,
  [112] = 112,
  [113] = 113,
  [114] = 114,
  [115] = 115,
  [116] = 116,
  [117] = 117,
  [118] = 118,
  [119] = 119,
  [120] = 120,
  [121] = 121,
  [122] = 122,
  [123] = 123,
  [124] = 124,
  [125] = 125,
  [126] = 126,
  [127] = 127,
  [128] = 128,
  [129] = 129,
  [130] = 130,
  [131] = 131,
  [132] = 132,
  [133] = 133,
  [134] = 134,
  [135] = 135,
  [136] = 136,
  [137] = 137,
  [138] = 138,
  [139] = 139,
  [140] = 140,
  [141] = 141,
  [142] = 142,
  [143] = 143,
  [144] = 144,
  [145] = 39,
  [146] = 146,
  [147] = 147,
  [148] = 148,
  [149] = 149,
  [150] = 150,
  [151] = 151,
  [152] = 152,
  [153] = 153,
  [154] = 154,
  [155] = 155,
  [156] = 156,
  [157] = 157,
  [158] = 158,
  [159] = 159,
  [160] = 160,
  [161] = 161,
  [162] = 162,
  [163] = 163,
  [164] = 164,
  [165] = 165,
  [166] = 166,
  [167] = 167,
  [168] = 168,
  [169] = 169,
};

static bool ts_lex(TSLexer *lexer, TSStateId state) {
  START_LEXER();
  eof = lexer->eof(lexer);
  switch (state) {
    case 0:
      if (eof) ADVANCE(519);
      ADVANCE_MAP(
        '"', 3,
        '#', 4,
        '/', 12,
        '[', 588,
        ']', 589,
        'a', 203,
        'b', 384,
        'c', 37,
        'd', 126,
        'e', 192,
        'f', 46,
        'i', 112,
        'j', 487,
        'l', 330,
        'm', 39,
        'n', 332,
        'o', 501,
        'p', 44,
        'r', 127,
        's', 90,
        't', 128,
        'u', 303,
        'v', 49,
        'w', 336,
        '{', 523,
        '}', 524,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(0);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(617);
      END_STATE();
    case 1:
      if (lookahead == '"') ADVANCE(3);
      if (lookahead == '#') ADVANCE(4);
      if (lookahead == '/') ADVANCE(12);
      if (lookahead == ']') ADVANCE(589);
      if (lookahead == '}') ADVANCE(524);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(1);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 2:
      if (lookahead == '"') ADVANCE(3);
      if (lookahead == '#') ADVANCE(4);
      if (lookahead == '/') ADVANCE(12);
      if (lookahead == 'c') ADVANCE(642);
      if (lookahead == 'e') ADVANCE(680);
      if (lookahead == 'm') ADVANCE(630);
      if (lookahead == 'r') ADVANCE(631);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(2);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 3:
      if (lookahead == '"') ADVANCE(614);
      if (lookahead == '\\') ADVANCE(517);
      if (lookahead != 0) ADVANCE(3);
      END_STATE();
    case 4:
      if (lookahead == '"') ADVANCE(5);
      END_STATE();
    case 5:
      if (lookahead == '"') ADVANCE(6);
      if (lookahead != 0) ADVANCE(5);
      END_STATE();
    case 6:
      if (lookahead == '#') ADVANCE(615);
      if (lookahead != 0) ADVANCE(5);
      END_STATE();
    case 7:
      if (lookahead == '.') ADVANCE(516);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(7);
      END_STATE();
    case 8:
      if (lookahead == '/') ADVANCE(12);
      if (lookahead == ']') ADVANCE(589);
      if (lookahead == 'l') ADVANCE(665);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(8);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 9:
      ADVANCE_MAP(
        '/', 12,
        'a', 208,
        'b', 384,
        'c', 273,
        'i', 297,
        'j', 487,
        'm', 74,
        'r', 152,
        's', 456,
        't', 187,
        'u', 303,
        '}', 524,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(9);
      END_STATE();
    case 10:
      ADVANCE_MAP(
        '/', 12,
        'a', 209,
        'b', 384,
        'c', 252,
        'd', 144,
        'e', 406,
        'i', 287,
        'j', 487,
        'm', 40,
        'o', 501,
        'p', 397,
        'r', 145,
        's', 91,
        't', 181,
        'u', 303,
        'v', 49,
        '}', 524,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(10);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(7);
      END_STATE();
    case 11:
      if (lookahead == '/') ADVANCE(12);
      if (lookahead == 'r') ADVANCE(634);
      if (lookahead == 's') ADVANCE(648);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(11);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 12:
      if (lookahead == '/') ADVANCE(521);
      END_STATE();
    case 13:
      if (lookahead == '_') ADVANCE(240);
      END_STATE();
    case 14:
      if (lookahead == '_') ADVANCE(268);
      END_STATE();
    case 15:
      if (lookahead == '_') ADVANCE(98);
      END_STATE();
    case 16:
      if (lookahead == '_') ADVANCE(248);
      END_STATE();
    case 17:
      if (lookahead == '_') ADVANCE(237);
      END_STATE();
    case 18:
      if (lookahead == '_') ADVANCE(364);
      END_STATE();
    case 19:
      if (lookahead == '_') ADVANCE(423);
      END_STATE();
    case 20:
      if (lookahead == '_') ADVANCE(198);
      if (lookahead == 's') ADVANCE(30);
      END_STATE();
    case 21:
      if (lookahead == '_') ADVANCE(288);
      END_STATE();
    case 22:
      if (lookahead == '_') ADVANCE(429);
      END_STATE();
    case 23:
      if (lookahead == '_') ADVANCE(337);
      END_STATE();
    case 24:
      if (lookahead == '_') ADVANCE(105);
      END_STATE();
    case 25:
      if (lookahead == '_') ADVANCE(335);
      END_STATE();
    case 26:
      if (lookahead == '_') ADVANCE(367);
      END_STATE();
    case 27:
      if (lookahead == '_') ADVANCE(52);
      END_STATE();
    case 28:
      if (lookahead == '_') ADVANCE(428);
      END_STATE();
    case 29:
      if (lookahead == '_') ADVANCE(422);
      END_STATE();
    case 30:
      if (lookahead == '_') ADVANCE(199);
      END_STATE();
    case 31:
      if (lookahead == '_') ADVANCE(353);
      END_STATE();
    case 32:
      if (lookahead == '_') ADVANCE(470);
      END_STATE();
    case 33:
      if (lookahead == '_') ADVANCE(345);
      END_STATE();
    case 34:
      if (lookahead == '_') ADVANCE(342);
      END_STATE();
    case 35:
      if (lookahead == '_') ADVANCE(476);
      END_STATE();
    case 36:
      if (lookahead == '_') ADVANCE(239);
      END_STATE();
    case 37:
      if (lookahead == 'a') ADVANCE(258);
      if (lookahead == 'h') ADVANCE(150);
      if (lookahead == 'l') ADVANCE(214);
      if (lookahead == 'o') ADVANCE(282);
      END_STATE();
    case 38:
      if (lookahead == 'a') ADVANCE(258);
      if (lookahead == 'l') ADVANCE(247);
      if (lookahead == 'o') ADVANCE(329);
      END_STATE();
    case 39:
      if (lookahead == 'a') ADVANCE(503);
      if (lookahead == 'e') ADVANCE(88);
      if (lookahead == 'o') ADVANCE(119);
      END_STATE();
    case 40:
      if (lookahead == 'a') ADVANCE(503);
      if (lookahead == 'e') ADVANCE(285);
      END_STATE();
    case 41:
      if (lookahead == 'a') ADVANCE(118);
      if (lookahead == 'f') ADVANCE(235);
      if (lookahead == 'v') ADVANCE(227);
      END_STATE();
    case 42:
      if (lookahead == 'a') ADVANCE(87);
      if (lookahead == 'e') ADVANCE(361);
      if (lookahead == 'r') ADVANCE(71);
      END_STATE();
    case 43:
      if (lookahead == 'a') ADVANCE(530);
      END_STATE();
    case 44:
      if (lookahead == 'a') ADVANCE(401);
      if (lookahead == 'r') ADVANCE(215);
      if (lookahead == 'u') ADVANCE(85);
      END_STATE();
    case 45:
      if (lookahead == 'a') ADVANCE(401);
      if (lookahead == 'r') ADVANCE(331);
      END_STATE();
    case 46:
      if (lookahead == 'a') ADVANCE(253);
      END_STATE();
    case 47:
      if (lookahead == 'a') ADVANCE(86);
      if (lookahead == 'e') ADVANCE(361);
      if (lookahead == 'r') ADVANCE(73);
      END_STATE();
    case 48:
      if (lookahead == 'a') ADVANCE(86);
      if (lookahead == 'r') ADVANCE(246);
      END_STATE();
    case 49:
      if (lookahead == 'a') ADVANCE(385);
      if (lookahead == 'e') ADVANCE(391);
      END_STATE();
    case 50:
      if (lookahead == 'a') ADVANCE(489);
      END_STATE();
    case 51:
      if (lookahead == 'a') ADVANCE(308);
      END_STATE();
    case 52:
      if (lookahead == 'a') ADVANCE(200);
      END_STATE();
    case 53:
      if (lookahead == 'a') ADVANCE(97);
      END_STATE();
    case 54:
      if (lookahead == 'a') ADVANCE(97);
      if (lookahead == 'o') ADVANCE(393);
      END_STATE();
    case 55:
      if (lookahead == 'a') ADVANCE(359);
      END_STATE();
    case 56:
      if (lookahead == 'a') ADVANCE(262);
      END_STATE();
    case 57:
      if (lookahead == 'a') ADVANCE(96);
      END_STATE();
    case 58:
      if (lookahead == 'a') ADVANCE(319);
      END_STATE();
    case 59:
      if (lookahead == 'a') ADVANCE(260);
      END_STATE();
    case 60:
      if (lookahead == 'a') ADVANCE(125);
      if (lookahead == 'v') ADVANCE(227);
      END_STATE();
    case 61:
      if (lookahead == 'a') ADVANCE(117);
      END_STATE();
    case 62:
      if (lookahead == 'a') ADVANCE(266);
      END_STATE();
    case 63:
      if (lookahead == 'a') ADVANCE(256);
      END_STATE();
    case 64:
      if (lookahead == 'a') ADVANCE(481);
      END_STATE();
    case 65:
      if (lookahead == 'a') ADVANCE(228);
      END_STATE();
    case 66:
      if (lookahead == 'a') ADVANCE(505);
      if (lookahead == 'e') ADVANCE(285);
      END_STATE();
    case 67:
      if (lookahead == 'a') ADVANCE(316);
      END_STATE();
    case 68:
      if (lookahead == 'a') ADVANCE(311);
      END_STATE();
    case 69:
      if (lookahead == 'a') ADVANCE(433);
      END_STATE();
    case 70:
      if (lookahead == 'a') ADVANCE(405);
      END_STATE();
    case 71:
      if (lookahead == 'a') ADVANCE(471);
      if (lookahead == 'i') ADVANCE(99);
      END_STATE();
    case 72:
      if (lookahead == 'a') ADVANCE(471);
      if (lookahead == 'i') ADVANCE(101);
      END_STATE();
    case 73:
      if (lookahead == 'a') ADVANCE(471);
      if (lookahead == 'i') ADVANCE(107);
      END_STATE();
    case 74:
      if (lookahead == 'a') ADVANCE(506);
      END_STATE();
    case 75:
      if (lookahead == 'a') ADVANCE(100);
      if (lookahead == 'o') ADVANCE(393);
      END_STATE();
    case 76:
      if (lookahead == 'a') ADVANCE(399);
      END_STATE();
    case 77:
      if (lookahead == 'a') ADVANCE(474);
      END_STATE();
    case 78:
      if (lookahead == 'a') ADVANCE(269);
      if (lookahead == 'e') ADVANCE(361);
      if (lookahead == 'r') ADVANCE(72);
      END_STATE();
    case 79:
      if (lookahead == 'a') ADVANCE(475);
      END_STATE();
    case 80:
      if (lookahead == 'a') ADVANCE(477);
      END_STATE();
    case 81:
      if (lookahead == 'a') ADVANCE(478);
      END_STATE();
    case 82:
      if (lookahead == 'a') ADVANCE(278);
      END_STATE();
    case 83:
      if (lookahead == 'a') ADVANCE(109);
      END_STATE();
    case 84:
      if (lookahead == 'a') ADVANCE(486);
      END_STATE();
    case 85:
      if (lookahead == 'b') ADVANCE(265);
      END_STATE();
    case 86:
      if (lookahead == 'b') ADVANCE(234);
      END_STATE();
    case 87:
      if (lookahead == 'b') ADVANCE(234);
      if (lookahead == 'c') ADVANCE(251);
      if (lookahead == 'l') ADVANCE(188);
      END_STATE();
    case 88:
      if (lookahead == 'c') ADVANCE(213);
      if (lookahead == 'm') ADVANCE(341);
      END_STATE();
    case 89:
      if (lookahead == 'c') ADVANCE(606);
      END_STATE();
    case 90:
      if (lookahead == 'c') ADVANCE(334);
      if (lookahead == 'i') ADVANCE(304);
      if (lookahead == 't') ADVANCE(42);
      END_STATE();
    case 91:
      if (lookahead == 'c') ADVANCE(334);
      if (lookahead == 't') ADVANCE(47);
      END_STATE();
    case 92:
      if (lookahead == 'c') ADVANCE(334);
      if (lookahead == 't') ADVANCE(78);
      END_STATE();
    case 93:
      if (lookahead == 'c') ADVANCE(211);
      END_STATE();
    case 94:
      if (lookahead == 'c') ADVANCE(494);
      END_STATE();
    case 95:
      if (lookahead == 'c') ADVANCE(274);
      END_STATE();
    case 96:
      if (lookahead == 'c') ADVANCE(510);
      END_STATE();
    case 97:
      if (lookahead == 'c') ADVANCE(461);
      END_STATE();
    case 98:
      if (lookahead == 'c') ADVANCE(355);
      if (lookahead == 'n') ADVANCE(413);
      END_STATE();
    case 99:
      if (lookahead == 'c') ADVANCE(443);
      END_STATE();
    case 100:
      if (lookahead == 'c') ADVANCE(479);
      END_STATE();
    case 101:
      if (lookahead == 'c') ADVANCE(454);
      END_STATE();
    case 102:
      if (lookahead == 'c') ADVANCE(139);
      END_STATE();
    case 103:
      if (lookahead == 'c') ADVANCE(179);
      END_STATE();
    case 104:
      if (lookahead == 'c') ADVANCE(56);
      END_STATE();
    case 105:
      if (lookahead == 'c') ADVANCE(212);
      END_STATE();
    case 106:
      if (lookahead == 'c') ADVANCE(396);
      END_STATE();
    case 107:
      if (lookahead == 'c') ADVANCE(466);
      END_STATE();
    case 108:
      if (lookahead == 'c') ADVANCE(59);
      if (lookahead == 'o') ADVANCE(358);
      END_STATE();
    case 109:
      if (lookahead == 'c') ADVANCE(468);
      END_STATE();
    case 110:
      if (lookahead == 'c') ADVANCE(63);
      END_STATE();
    case 111:
      if (lookahead == 'c') ADVANCE(350);
      END_STATE();
    case 112:
      if (lookahead == 'd') ADVANCE(590);
      if (lookahead == 'm') ADVANCE(357);
      if (lookahead == 'n') ADVANCE(95);
      if (lookahead == 't') ADVANCE(159);
      END_STATE();
    case 113:
      if (lookahead == 'd') ADVANCE(205);
      END_STATE();
    case 114:
      if (lookahead == 'd') ADVANCE(553);
      END_STATE();
    case 115:
      if (lookahead == 'd') ADVANCE(598);
      END_STATE();
    case 116:
      if (lookahead == 'd') ADVANCE(595);
      END_STATE();
    case 117:
      if (lookahead == 'd') ADVANCE(14);
      END_STATE();
    case 118:
      if (lookahead == 'd') ADVANCE(14);
      if (lookahead == 's') ADVANCE(352);
      END_STATE();
    case 119:
      if (lookahead == 'd') ADVANCE(165);
      END_STATE();
    case 120:
      if (lookahead == 'd') ADVANCE(437);
      END_STATE();
    case 121:
      if (lookahead == 'd') ADVANCE(221);
      END_STATE();
    case 122:
      if (lookahead == 'd') ADVANCE(136);
      END_STATE();
    case 123:
      if (lookahead == 'd') ADVANCE(142);
      END_STATE();
    case 124:
      if (lookahead == 'd') ADVANCE(206);
      END_STATE();
    case 125:
      if (lookahead == 'd') ADVANCE(34);
      END_STATE();
    case 126:
      if (lookahead == 'e') ADVANCE(195);
      END_STATE();
    case 127:
      if (lookahead == 'e') ADVANCE(41);
      END_STATE();
    case 128:
      if (lookahead == 'e') ADVANCE(283);
      if (lookahead == 'i') ADVANCE(164);
      if (lookahead == 'o') ADVANCE(339);
      if (lookahead == 'r') ADVANCE(488);
      END_STATE();
    case 129:
      if (lookahead == 'e') ADVANCE(596);
      END_STATE();
    case 130:
      if (lookahead == 'e') ADVANCE(612);
      END_STATE();
    case 131:
      if (lookahead == 'e') ADVANCE(613);
      END_STATE();
    case 132:
      if (lookahead == 'e') ADVANCE(552);
      END_STATE();
    case 133:
      if (lookahead == 'e') ADVANCE(610);
      END_STATE();
    case 134:
      if (lookahead == 'e') ADVANCE(565);
      END_STATE();
    case 135:
      if (lookahead == 'e') ADVANCE(594);
      END_STATE();
    case 136:
      if (lookahead == 'e') ADVANCE(520);
      END_STATE();
    case 137:
      if (lookahead == 'e') ADVANCE(551);
      END_STATE();
    case 138:
      if (lookahead == 'e') ADVANCE(534);
      END_STATE();
    case 139:
      if (lookahead == 'e') ADVANCE(559);
      END_STATE();
    case 140:
      if (lookahead == 'e') ADVANCE(555);
      END_STATE();
    case 141:
      if (lookahead == 'e') ADVANCE(586);
      END_STATE();
    case 142:
      if (lookahead == 'e') ADVANCE(581);
      END_STATE();
    case 143:
      if (lookahead == 'e') ADVANCE(580);
      END_STATE();
    case 144:
      if (lookahead == 'e') ADVANCE(371);
      END_STATE();
    case 145:
      if (lookahead == 'e') ADVANCE(60);
      END_STATE();
    case 146:
      if (lookahead == 'e') ADVANCE(550);
      END_STATE();
    case 147:
      if (lookahead == 'e') ADVANCE(502);
      END_STATE();
    case 148:
      if (lookahead == 'e') ADVANCE(504);
      END_STATE();
    case 149:
      if (lookahead == 'e') ADVANCE(356);
      END_STATE();
    case 150:
      if (lookahead == 'e') ADVANCE(55);
      END_STATE();
    case 151:
      if (lookahead == 'e') ADVANCE(204);
      END_STATE();
    case 152:
      if (lookahead == 'e') ADVANCE(497);
      END_STATE();
    case 153:
      if (lookahead == 'e') ADVANCE(94);
      if (lookahead == 'p') ADVANCE(158);
      if (lookahead == 't') ADVANCE(389);
      END_STATE();
    case 154:
      if (lookahead == 'e') ADVANCE(114);
      END_STATE();
    case 155:
      if (lookahead == 'e') ADVANCE(32);
      END_STATE();
    case 156:
      if (lookahead == 'e') ADVANCE(309);
      END_STATE();
    case 157:
      if (lookahead == 'e') ADVANCE(309);
      if (lookahead == 't') ADVANCE(210);
      END_STATE();
    case 158:
      if (lookahead == 'e') ADVANCE(310);
      if (lookahead == 'l') ADVANCE(343);
      END_STATE();
    case 159:
      if (lookahead == 'e') ADVANCE(386);
      END_STATE();
    case 160:
      if (lookahead == 'e') ADVANCE(313);
      END_STATE();
    case 161:
      if (lookahead == 'e') ADVANCE(201);
      END_STATE();
    case 162:
      if (lookahead == 'e') ADVANCE(116);
      END_STATE();
    case 163:
      if (lookahead == 'e') ADVANCE(15);
      END_STATE();
    case 164:
      if (lookahead == 'e') ADVANCE(380);
      END_STATE();
    case 165:
      if (lookahead == 'e') ADVANCE(254);
      END_STATE();
    case 166:
      if (lookahead == 'e') ADVANCE(392);
      END_STATE();
    case 167:
      if (lookahead == 'e') ADVANCE(26);
      END_STATE();
    case 168:
      if (lookahead == 'e') ADVANCE(338);
      END_STATE();
    case 169:
      if (lookahead == 'e') ADVANCE(398);
      END_STATE();
    case 170:
      if (lookahead == 'e') ADVANCE(432);
      END_STATE();
    case 171:
      if (lookahead == 'e') ADVANCE(27);
      END_STATE();
    case 172:
      if (lookahead == 'e') ADVANCE(394);
      END_STATE();
    case 173:
      if (lookahead == 'e') ADVANCE(61);
      END_STATE();
    case 174:
      if (lookahead == 'e') ADVANCE(480);
      END_STATE();
    case 175:
      if (lookahead == 'e') ADVANCE(415);
      END_STATE();
    case 176:
      if (lookahead == 'e') ADVANCE(383);
      END_STATE();
    case 177:
      if (lookahead == 'e') ADVANCE(257);
      END_STATE();
    case 178:
      if (lookahead == 'e') ADVANCE(17);
      END_STATE();
    case 179:
      if (lookahead == 'e') ADVANCE(419);
      END_STATE();
    case 180:
      if (lookahead == 'e') ADVANCE(305);
      END_STATE();
    case 181:
      if (lookahead == 'e') ADVANCE(284);
      if (lookahead == 'i') ADVANCE(164);
      if (lookahead == 'o') ADVANCE(339);
      END_STATE();
    case 182:
      if (lookahead == 'e') ADVANCE(307);
      END_STATE();
    case 183:
      if (lookahead == 'e') ADVANCE(307);
      if (lookahead == 'p') ADVANCE(360);
      END_STATE();
    case 184:
      if (lookahead == 'e') ADVANCE(296);
      if (lookahead == 'i') ADVANCE(164);
      if (lookahead == 'o') ADVANCE(339);
      END_STATE();
    case 185:
      if (lookahead == 'e') ADVANCE(436);
      END_STATE();
    case 186:
      if (lookahead == 'e') ADVANCE(395);
      END_STATE();
    case 187:
      if (lookahead == 'e') ADVANCE(298);
      END_STATE();
    case 188:
      if (lookahead == 'e') ADVANCE(325);
      END_STATE();
    case 189:
      if (lookahead == 'e') ADVANCE(323);
      END_STATE();
    case 190:
      if (lookahead == 'e') ADVANCE(327);
      END_STATE();
    case 191:
      if (lookahead == 'e') ADVANCE(294);
      END_STATE();
    case 192:
      if (lookahead == 'f') ADVANCE(196);
      if (lookahead == 's') ADVANCE(104);
      if (lookahead == 'x') ADVANCE(153);
      END_STATE();
    case 193:
      if (lookahead == 'f') ADVANCE(572);
      END_STATE();
    case 194:
      if (lookahead == 'f') ADVANCE(509);
      END_STATE();
    case 195:
      if (lookahead == 'f') ADVANCE(50);
      if (lookahead == 'p') ADVANCE(157);
      if (lookahead == 's') ADVANCE(106);
      END_STATE();
    case 196:
      if (lookahead == 'f') ADVANCE(349);
      END_STATE();
    case 197:
      if (lookahead == 'f') ADVANCE(263);
      END_STATE();
    case 198:
      if (lookahead == 'f') ADVANCE(225);
      END_STATE();
    case 199:
      if (lookahead == 'f') ADVANCE(347);
      END_STATE();
    case 200:
      if (lookahead == 'f') ADVANCE(482);
      END_STATE();
    case 201:
      if (lookahead == 'f') ADVANCE(242);
      END_STATE();
    case 202:
      if (lookahead == 'g') ADVANCE(538);
      END_STATE();
    case 203:
      if (lookahead == 'g') ADVANCE(180);
      if (lookahead == 't') ADVANCE(458);
      END_STATE();
    case 204:
      if (lookahead == 'g') ADVANCE(511);
      END_STATE();
    case 205:
      if (lookahead == 'g') ADVANCE(155);
      END_STATE();
    case 206:
      if (lookahead == 'g') ADVANCE(141);
      END_STATE();
    case 207:
      if (lookahead == 'g') ADVANCE(271);
      END_STATE();
    case 208:
      if (lookahead == 'g') ADVANCE(190);
      END_STATE();
    case 209:
      if (lookahead == 'g') ADVANCE(190);
      if (lookahead == 't') ADVANCE(458);
      END_STATE();
    case 210:
      if (lookahead == 'h') ADVANCE(574);
      END_STATE();
    case 211:
      if (lookahead == 'h') ADVANCE(24);
      END_STATE();
    case 212:
      if (lookahead == 'h') ADVANCE(65);
      END_STATE();
    case 213:
      if (lookahead == 'h') ADVANCE(58);
      END_STATE();
    case 214:
      if (lookahead == 'i') ADVANCE(183);
      END_STATE();
    case 215:
      if (lookahead == 'i') ADVANCE(498);
      if (lookahead == 'o') ADVANCE(286);
      END_STATE();
    case 216:
      if (lookahead == 'i') ADVANCE(194);
      END_STATE();
    case 217:
      if (lookahead == 'i') ADVANCE(499);
      END_STATE();
    case 218:
      if (lookahead == 'i') ADVANCE(291);
      END_STATE();
    case 219:
      if (lookahead == 'i') ADVANCE(292);
      END_STATE();
    case 220:
      if (lookahead == 'i') ADVANCE(89);
      END_STATE();
    case 221:
      if (lookahead == 'i') ADVANCE(315);
      END_STATE();
    case 222:
      if (lookahead == 'i') ADVANCE(376);
      END_STATE();
    case 223:
      if (lookahead == 'i') ADVANCE(255);
      END_STATE();
    case 224:
      if (lookahead == 'i') ADVANCE(306);
      END_STATE();
    case 225:
      if (lookahead == 'i') ADVANCE(403);
      END_STATE();
    case 226:
      if (lookahead == 'i') ADVANCE(363);
      END_STATE();
    case 227:
      if (lookahead == 'i') ADVANCE(147);
      END_STATE();
    case 228:
      if (lookahead == 'i') ADVANCE(302);
      END_STATE();
    case 229:
      if (lookahead == 'i') ADVANCE(459);
      END_STATE();
    case 230:
      if (lookahead == 'i') ADVANCE(447);
      END_STATE();
    case 231:
      if (lookahead == 'i') ADVANCE(450);
      END_STATE();
    case 232:
      if (lookahead == 'i') ADVANCE(175);
      END_STATE();
    case 233:
      if (lookahead == 'i') ADVANCE(467);
      END_STATE();
    case 234:
      if (lookahead == 'i') ADVANCE(270);
      END_STATE();
    case 235:
      if (lookahead == 'i') ADVANCE(321);
      END_STATE();
    case 236:
      if (lookahead == 'i') ADVANCE(272);
      END_STATE();
    case 237:
      if (lookahead == 'i') ADVANCE(324);
      if (lookahead == 'r') ADVANCE(161);
      END_STATE();
    case 238:
      if (lookahead == 'i') ADVANCE(62);
      END_STATE();
    case 239:
      if (lookahead == 'i') ADVANCE(473);
      END_STATE();
    case 240:
      if (lookahead == 'i') ADVANCE(473);
      if (lookahead == 'p') ADVANCE(70);
      if (lookahead == 'r') ADVANCE(174);
      END_STATE();
    case 241:
      if (lookahead == 'i') ADVANCE(344);
      END_STATE();
    case 242:
      if (lookahead == 'i') ADVANCE(326);
      END_STATE();
    case 243:
      if (lookahead == 'i') ADVANCE(346);
      END_STATE();
    case 244:
      if (lookahead == 'i') ADVANCE(351);
      END_STATE();
    case 245:
      if (lookahead == 'i') ADVANCE(110);
      END_STATE();
    case 246:
      if (lookahead == 'i') ADVANCE(107);
      END_STATE();
    case 247:
      if (lookahead == 'i') ADVANCE(182);
      END_STATE();
    case 248:
      if (lookahead == 'j') ADVANCE(496);
      END_STATE();
    case 249:
      if (lookahead == 'k') ADVANCE(593);
      END_STATE();
    case 250:
      if (lookahead == 'k') ADVANCE(197);
      END_STATE();
    case 251:
      if (lookahead == 'k') ADVANCE(162);
      END_STATE();
    case 252:
      if (lookahead == 'l') ADVANCE(214);
      if (lookahead == 'o') ADVANCE(289);
      END_STATE();
    case 253:
      if (lookahead == 'l') ADVANCE(427);
      END_STATE();
    case 254:
      if (lookahead == 'l') ADVANCE(526);
      END_STATE();
    case 255:
      if (lookahead == 'l') ADVANCE(597);
      END_STATE();
    case 256:
      if (lookahead == 'l') ADVANCE(542);
      END_STATE();
    case 257:
      if (lookahead == 'l') ADVANCE(601);
      END_STATE();
    case 258:
      if (lookahead == 'l') ADVANCE(277);
      END_STATE();
    case 259:
      if (lookahead == 'l') ADVANCE(410);
      END_STATE();
    case 260:
      if (lookahead == 'l') ADVANCE(31);
      END_STATE();
    case 261:
      if (lookahead == 'l') ADVANCE(512);
      END_STATE();
    case 262:
      if (lookahead == 'l') ADVANCE(77);
      END_STATE();
    case 263:
      if (lookahead == 'l') ADVANCE(333);
      END_STATE();
    case 264:
      if (lookahead == 'l') ADVANCE(514);
      END_STATE();
    case 265:
      if (lookahead == 'l') ADVANCE(220);
      END_STATE();
    case 266:
      if (lookahead == 'l') ADVANCE(25);
      END_STATE();
    case 267:
      if (lookahead == 'l') ADVANCE(445);
      END_STATE();
    case 268:
      if (lookahead == 'l') ADVANCE(218);
      if (lookahead == 'n') ADVANCE(411);
      if (lookahead == 'o') ADVANCE(317);
      if (lookahead == 'q') ADVANCE(495);
      END_STATE();
    case 269:
      if (lookahead == 'l') ADVANCE(188);
      END_STATE();
    case 270:
      if (lookahead == 'l') ADVANCE(229);
      END_STATE();
    case 271:
      if (lookahead == 'l') ADVANCE(167);
      END_STATE();
    case 272:
      if (lookahead == 'l') ADVANCE(134);
      END_STATE();
    case 273:
      if (lookahead == 'l') ADVANCE(226);
      if (lookahead == 'o') ADVANCE(290);
      END_STATE();
    case 274:
      if (lookahead == 'l') ADVANCE(492);
      END_STATE();
    case 275:
      if (lookahead == 'l') ADVANCE(343);
      END_STATE();
    case 276:
      if (lookahead == 'l') ADVANCE(177);
      END_STATE();
    case 277:
      if (lookahead == 'l') ADVANCE(166);
      END_STATE();
    case 278:
      if (lookahead == 'l') ADVANCE(276);
      END_STATE();
    case 279:
      if (lookahead == 'l') ADVANCE(79);
      END_STATE();
    case 280:
      if (lookahead == 'l') ADVANCE(80);
      END_STATE();
    case 281:
      if (lookahead == 'l') ADVANCE(81);
      END_STATE();
    case 282:
      if (lookahead == 'm') ADVANCE(293);
      if (lookahead == 'n') ADVANCE(425);
      if (lookahead == 'o') ADVANCE(388);
      END_STATE();
    case 283:
      if (lookahead == 'm') ADVANCE(374);
      if (lookahead == 's') ADVANCE(438);
      END_STATE();
    case 284:
      if (lookahead == 'm') ADVANCE(374);
      if (lookahead == 's') ADVANCE(452);
      END_STATE();
    case 285:
      if (lookahead == 'm') ADVANCE(341);
      END_STATE();
    case 286:
      if (lookahead == 'm') ADVANCE(365);
      END_STATE();
    case 287:
      if (lookahead == 'm') ADVANCE(372);
      if (lookahead == 't') ADVANCE(159);
      END_STATE();
    case 288:
      if (lookahead == 'm') ADVANCE(354);
      END_STATE();
    case 289:
      if (lookahead == 'm') ADVANCE(362);
      if (lookahead == 'n') ADVANCE(425);
      END_STATE();
    case 290:
      if (lookahead == 'm') ADVANCE(362);
      if (lookahead == 'n') ADVANCE(424);
      END_STATE();
    case 291:
      if (lookahead == 'm') ADVANCE(230);
      END_STATE();
    case 292:
      if (lookahead == 'm') ADVANCE(168);
      END_STATE();
    case 293:
      if (lookahead == 'm') ADVANCE(68);
      if (lookahead == 'p') ADVANCE(236);
      END_STATE();
    case 294:
      if (lookahead == 'm') ADVANCE(366);
      END_STATE();
    case 295:
      if (lookahead == 'm') ADVANCE(373);
      if (lookahead == 'n') ADVANCE(95);
      END_STATE();
    case 296:
      if (lookahead == 'm') ADVANCE(377);
      if (lookahead == 's') ADVANCE(453);
      END_STATE();
    case 297:
      if (lookahead == 'm') ADVANCE(375);
      if (lookahead == 't') ADVANCE(159);
      END_STATE();
    case 298:
      if (lookahead == 'm') ADVANCE(378);
      if (lookahead == 's') ADVANCE(455);
      END_STATE();
    case 299:
      if (lookahead == 'n') ADVANCE(540);
      END_STATE();
    case 300:
      if (lookahead == 'n') ADVANCE(547);
      END_STATE();
    case 301:
      if (lookahead == 'n') ADVANCE(546);
      END_STATE();
    case 302:
      if (lookahead == 'n') ADVANCE(587);
      END_STATE();
    case 303:
      if (lookahead == 'n') ADVANCE(457);
      END_STATE();
    case 304:
      if (lookahead == 'n') ADVANCE(207);
      END_STATE();
    case 305:
      if (lookahead == 'n') ADVANCE(439);
      END_STATE();
    case 306:
      if (lookahead == 'n') ADVANCE(202);
      END_STATE();
    case 307:
      if (lookahead == 'n') ADVANCE(440);
      END_STATE();
    case 308:
      if (lookahead == 'n') ADVANCE(93);
      END_STATE();
    case 309:
      if (lookahead == 'n') ADVANCE(120);
      END_STATE();
    case 310:
      if (lookahead == 'n') ADVANCE(430);
      END_STATE();
    case 311:
      if (lookahead == 'n') ADVANCE(115);
      END_STATE();
    case 312:
      if (lookahead == 'n') ADVANCE(129);
      END_STATE();
    case 313:
      if (lookahead == 'n') ADVANCE(421);
      END_STATE();
    case 314:
      if (lookahead == 'n') ADVANCE(154);
      END_STATE();
    case 315:
      if (lookahead == 'n') ADVANCE(64);
      END_STATE();
    case 316:
      if (lookahead == 'n') ADVANCE(102);
      END_STATE();
    case 317:
      if (lookahead == 'n') ADVANCE(261);
      END_STATE();
    case 318:
      if (lookahead == 'n') ADVANCE(264);
      END_STATE();
    case 319:
      if (lookahead == 'n') ADVANCE(245);
      END_STATE();
    case 320:
      if (lookahead == 'n') ADVANCE(224);
      END_STATE();
    case 321:
      if (lookahead == 'n') ADVANCE(133);
      END_STATE();
    case 322:
      if (lookahead == 'n') ADVANCE(418);
      END_STATE();
    case 323:
      if (lookahead == 'n') ADVANCE(451);
      END_STATE();
    case 324:
      if (lookahead == 'n') ADVANCE(231);
      END_STATE();
    case 325:
      if (lookahead == 'n') ADVANCE(170);
      END_STATE();
    case 326:
      if (lookahead == 'n') ADVANCE(143);
      END_STATE();
    case 327:
      if (lookahead == 'n') ADVANCE(472);
      END_STATE();
    case 328:
      if (lookahead == 'n') ADVANCE(484);
      END_STATE();
    case 329:
      if (lookahead == 'n') ADVANCE(464);
      END_STATE();
    case 330:
      if (lookahead == 'o') ADVANCE(108);
      END_STATE();
    case 331:
      if (lookahead == 'o') ADVANCE(286);
      END_STATE();
    case 332:
      if (lookahead == 'o') ADVANCE(312);
      END_STATE();
    case 333:
      if (lookahead == 'o') ADVANCE(500);
      END_STATE();
    case 334:
      if (lookahead == 'o') ADVANCE(368);
      END_STATE();
    case 335:
      if (lookahead == 'o') ADVANCE(249);
      END_STATE();
    case 336:
      if (lookahead == 'o') ADVANCE(379);
      if (lookahead == 'r') ADVANCE(233);
      END_STATE();
    case 337:
      if (lookahead == 'o') ADVANCE(193);
      END_STATE();
    case 338:
      if (lookahead == 'o') ADVANCE(491);
      END_STATE();
    case 339:
      if (lookahead == 'o') ADVANCE(259);
      END_STATE();
    case 340:
      if (lookahead == 'o') ADVANCE(490);
      END_STATE();
    case 341:
      if (lookahead == 'o') ADVANCE(387);
      END_STATE();
    case 342:
      if (lookahead == 'o') ADVANCE(317);
      END_STATE();
    case 343:
      if (lookahead == 'o') ADVANCE(400);
      END_STATE();
    case 344:
      if (lookahead == 'o') ADVANCE(299);
      END_STATE();
    case 345:
      if (lookahead == 'o') ADVANCE(300);
      END_STATE();
    case 346:
      if (lookahead == 'o') ADVANCE(301);
      END_STATE();
    case 347:
      if (lookahead == 'o') ADVANCE(381);
      END_STATE();
    case 348:
      if (lookahead == 'o') ADVANCE(382);
      END_STATE();
    case 349:
      if (lookahead == 'o') ADVANCE(390);
      END_STATE();
    case 350:
      if (lookahead == 'o') ADVANCE(370);
      END_STATE();
    case 351:
      if (lookahead == 'o') ADVANCE(322);
      END_STATE();
    case 352:
      if (lookahead == 'o') ADVANCE(320);
      END_STATE();
    case 353:
      if (lookahead == 'o') ADVANCE(318);
      END_STATE();
    case 354:
      if (lookahead == 'o') ADVANCE(123);
      END_STATE();
    case 355:
      if (lookahead == 'o') ADVANCE(328);
      END_STATE();
    case 356:
      if (lookahead == 'p') ADVANCE(157);
      if (lookahead == 's') ADVANCE(106);
      END_STATE();
    case 357:
      if (lookahead == 'p') ADVANCE(54);
      END_STATE();
    case 358:
      if (lookahead == 'p') ADVANCE(575);
      END_STATE();
    case 359:
      if (lookahead == 'p') ADVANCE(532);
      END_STATE();
    case 360:
      if (lookahead == 'p') ADVANCE(507);
      END_STATE();
    case 361:
      if (lookahead == 'p') ADVANCE(409);
      END_STATE();
    case 362:
      if (lookahead == 'p') ADVANCE(236);
      END_STATE();
    case 363:
      if (lookahead == 'p') ADVANCE(360);
      END_STATE();
    case 364:
      if (lookahead == 'p') ADVANCE(70);
      if (lookahead == 'r') ADVANCE(174);
      END_STATE();
    case 365:
      if (lookahead == 'p') ADVANCE(442);
      END_STATE();
    case 366:
      if (lookahead == 'p') ADVANCE(463);
      END_STATE();
    case 367:
      if (lookahead == 'p') ADVANCE(69);
      END_STATE();
    case 368:
      if (lookahead == 'p') ADVANCE(132);
      END_STATE();
    case 369:
      if (lookahead == 'p') ADVANCE(275);
      END_STATE();
    case 370:
      if (lookahead == 'p') ADVANCE(140);
      END_STATE();
    case 371:
      if (lookahead == 'p') ADVANCE(156);
      if (lookahead == 's') ADVANCE(106);
      END_STATE();
    case 372:
      if (lookahead == 'p') ADVANCE(53);
      END_STATE();
    case 373:
      if (lookahead == 'p') ADVANCE(75);
      END_STATE();
    case 374:
      if (lookahead == 'p') ADVANCE(279);
      END_STATE();
    case 375:
      if (lookahead == 'p') ADVANCE(83);
      END_STATE();
    case 376:
      if (lookahead == 'p') ADVANCE(485);
      END_STATE();
    case 377:
      if (lookahead == 'p') ADVANCE(280);
      END_STATE();
    case 378:
      if (lookahead == 'p') ADVANCE(281);
      END_STATE();
    case 379:
      if (lookahead == 'r') ADVANCE(250);
      END_STATE();
    case 380:
      if (lookahead == 'r') ADVANCE(525);
      END_STATE();
    case 381:
      if (lookahead == 'r') ADVANCE(573);
      END_STATE();
    case 382:
      if (lookahead == 'r') ADVANCE(536);
      END_STATE();
    case 383:
      if (lookahead == 'r') ADVANCE(605);
      END_STATE();
    case 384:
      if (lookahead == 'r') ADVANCE(51);
      END_STATE();
    case 385:
      if (lookahead == 'r') ADVANCE(408);
      END_STATE();
    case 386:
      if (lookahead == 'r') ADVANCE(22);
      END_STATE();
    case 387:
      if (lookahead == 'r') ADVANCE(508);
      END_STATE();
    case 388:
      if (lookahead == 'r') ADVANCE(121);
      END_STATE();
    case 389:
      if (lookahead == 'r') ADVANCE(43);
      END_STATE();
    case 390:
      if (lookahead == 'r') ADVANCE(441);
      END_STATE();
    case 391:
      if (lookahead == 'r') ADVANCE(216);
      END_STATE();
    case 392:
      if (lookahead == 'r') ADVANCE(426);
      END_STATE();
    case 393:
      if (lookahead == 'r') ADVANCE(483);
      END_STATE();
    case 394:
      if (lookahead == 'r') ADVANCE(515);
      END_STATE();
    case 395:
      if (lookahead == 'r') ADVANCE(84);
      END_STATE();
    case 396:
      if (lookahead == 'r') ADVANCE(222);
      END_STATE();
    case 397:
      if (lookahead == 'r') ADVANCE(331);
      END_STATE();
    case 398:
      if (lookahead == 'r') ADVANCE(414);
      END_STATE();
    case 399:
      if (lookahead == 'r') ADVANCE(446);
      END_STATE();
    case 400:
      if (lookahead == 'r') ADVANCE(135);
      END_STATE();
    case 401:
      if (lookahead == 'r') ADVANCE(460);
      END_STATE();
    case 402:
      if (lookahead == 'r') ADVANCE(232);
      END_STATE();
    case 403:
      if (lookahead == 'r') ADVANCE(434);
      END_STATE();
    case 404:
      if (lookahead == 'r') ADVANCE(103);
      END_STATE();
    case 405:
      if (lookahead == 'r') ADVANCE(82);
      END_STATE();
    case 406:
      if (lookahead == 's') ADVANCE(104);
      END_STATE();
    case 407:
      if (lookahead == 's') ADVANCE(104);
      if (lookahead == 'x') ADVANCE(369);
      END_STATE();
    case 408:
      if (lookahead == 's') ADVANCE(531);
      END_STATE();
    case 409:
      if (lookahead == 's') ADVANCE(600);
      END_STATE();
    case 410:
      if (lookahead == 's') ADVANCE(549);
      END_STATE();
    case 411:
      if (lookahead == 's') ADVANCE(557);
      END_STATE();
    case 412:
      if (lookahead == 's') ADVANCE(604);
      END_STATE();
    case 413:
      if (lookahead == 's') ADVANCE(558);
      END_STATE();
    case 414:
      if (lookahead == 's') ADVANCE(578);
      END_STATE();
    case 415:
      if (lookahead == 's') ADVANCE(548);
      END_STATE();
    case 416:
      if (lookahead == 's') ADVANCE(608);
      END_STATE();
    case 417:
      if (lookahead == 's') ADVANCE(570);
      END_STATE();
    case 418:
      if (lookahead == 's') ADVANCE(582);
      END_STATE();
    case 419:
      if (lookahead == 's') ADVANCE(560);
      END_STATE();
    case 420:
      if (lookahead == 's') ADVANCE(577);
      END_STATE();
    case 421:
      if (lookahead == 's') ADVANCE(493);
      END_STATE();
    case 422:
      if (lookahead == 's') ADVANCE(111);
      END_STATE();
    case 423:
      if (lookahead == 's') ADVANCE(111);
      if (lookahead == 't') ADVANCE(185);
      END_STATE();
    case 424:
      if (lookahead == 's') ADVANCE(160);
      END_STATE();
    case 425:
      if (lookahead == 's') ADVANCE(160);
      if (lookahead == 't') ADVANCE(148);
      END_STATE();
    case 426:
      if (lookahead == 's') ADVANCE(23);
      END_STATE();
    case 427:
      if (lookahead == 's') ADVANCE(131);
      END_STATE();
    case 428:
      if (lookahead == 's') ADVANCE(340);
      END_STATE();
    case 429:
      if (lookahead == 's') ADVANCE(462);
      END_STATE();
    case 430:
      if (lookahead == 's') ADVANCE(217);
      END_STATE();
    case 431:
      if (lookahead == 's') ADVANCE(21);
      END_STATE();
    case 432:
      if (lookahead == 's') ADVANCE(435);
      END_STATE();
    case 433:
      if (lookahead == 's') ADVANCE(416);
      END_STATE();
    case 434:
      if (lookahead == 's') ADVANCE(448);
      END_STATE();
    case 435:
      if (lookahead == 's') ADVANCE(28);
      END_STATE();
    case 436:
      if (lookahead == 's') ADVANCE(469);
      END_STATE();
    case 437:
      if (lookahead == 's') ADVANCE(33);
      END_STATE();
    case 438:
      if (lookahead == 't') ADVANCE(569);
      END_STATE();
    case 439:
      if (lookahead == 't') ADVANCE(545);
      END_STATE();
    case 440:
      if (lookahead == 't') ADVANCE(522);
      END_STATE();
    case 441:
      if (lookahead == 't') ADVANCE(527);
      END_STATE();
    case 442:
      if (lookahead == 't') ADVANCE(544);
      END_STATE();
    case 443:
      if (lookahead == 't') ADVANCE(592);
      END_STATE();
    case 444:
      if (lookahead == 't') ADVANCE(571);
      END_STATE();
    case 445:
      if (lookahead == 't') ADVANCE(529);
      END_STATE();
    case 446:
      if (lookahead == 't') ADVANCE(583);
      END_STATE();
    case 447:
      if (lookahead == 't') ADVANCE(562);
      END_STATE();
    case 448:
      if (lookahead == 't') ADVANCE(603);
      END_STATE();
    case 449:
      if (lookahead == 't') ADVANCE(585);
      END_STATE();
    case 450:
      if (lookahead == 't') ADVANCE(579);
      END_STATE();
    case 451:
      if (lookahead == 't') ADVANCE(563);
      END_STATE();
    case 452:
      if (lookahead == 't') ADVANCE(568);
      END_STATE();
    case 453:
      if (lookahead == 't') ADVANCE(20);
      END_STATE();
    case 454:
      if (lookahead == 't') ADVANCE(591);
      END_STATE();
    case 455:
      if (lookahead == 't') ADVANCE(567);
      END_STATE();
    case 456:
      if (lookahead == 't') ADVANCE(48);
      END_STATE();
    case 457:
      if (lookahead == 't') ADVANCE(223);
      END_STATE();
    case 458:
      if (lookahead == 't') ADVANCE(191);
      END_STATE();
    case 459:
      if (lookahead == 't') ADVANCE(513);
      END_STATE();
    case 460:
      if (lookahead == 't') ADVANCE(238);
      END_STATE();
    case 461:
      if (lookahead == 't') ADVANCE(19);
      END_STATE();
    case 462:
      if (lookahead == 't') ADVANCE(76);
      END_STATE();
    case 463:
      if (lookahead == 't') ADVANCE(412);
      END_STATE();
    case 464:
      if (lookahead == 't') ADVANCE(148);
      END_STATE();
    case 465:
      if (lookahead == 't') ADVANCE(241);
      END_STATE();
    case 466:
      if (lookahead == 't') ADVANCE(16);
      END_STATE();
    case 467:
      if (lookahead == 't') ADVANCE(163);
      END_STATE();
    case 468:
      if (lookahead == 't') ADVANCE(35);
      END_STATE();
    case 469:
      if (lookahead == 't') ADVANCE(417);
      END_STATE();
    case 470:
      if (lookahead == 't') ADVANCE(219);
      END_STATE();
    case 471:
      if (lookahead == 't') ADVANCE(151);
      END_STATE();
    case 472:
      if (lookahead == 't') ADVANCE(420);
      END_STATE();
    case 473:
      if (lookahead == 't') ADVANCE(186);
      END_STATE();
    case 474:
      if (lookahead == 't') ADVANCE(171);
      END_STATE();
    case 475:
      if (lookahead == 't') ADVANCE(137);
      END_STATE();
    case 476:
      if (lookahead == 't') ADVANCE(185);
      END_STATE();
    case 477:
      if (lookahead == 't') ADVANCE(146);
      END_STATE();
    case 478:
      if (lookahead == 't') ADVANCE(178);
      END_STATE();
    case 479:
      if (lookahead == 't') ADVANCE(29);
      END_STATE();
    case 480:
      if (lookahead == 't') ADVANCE(402);
      END_STATE();
    case 481:
      if (lookahead == 't') ADVANCE(348);
      END_STATE();
    case 482:
      if (lookahead == 't') ADVANCE(176);
      END_STATE();
    case 483:
      if (lookahead == 't') ADVANCE(67);
      END_STATE();
    case 484:
      if (lookahead == 't') ADVANCE(189);
      END_STATE();
    case 485:
      if (lookahead == 't') ADVANCE(243);
      END_STATE();
    case 486:
      if (lookahead == 't') ADVANCE(244);
      END_STATE();
    case 487:
      if (lookahead == 'u') ADVANCE(113);
      END_STATE();
    case 488:
      if (lookahead == 'u') ADVANCE(130);
      END_STATE();
    case 489:
      if (lookahead == 'u') ADVANCE(267);
      END_STATE();
    case 490:
      if (lookahead == 'u') ADVANCE(404);
      END_STATE();
    case 491:
      if (lookahead == 'u') ADVANCE(449);
      END_STATE();
    case 492:
      if (lookahead == 'u') ADVANCE(122);
      END_STATE();
    case 493:
      if (lookahead == 'u') ADVANCE(431);
      END_STATE();
    case 494:
      if (lookahead == 'u') ADVANCE(465);
      END_STATE();
    case 495:
      if (lookahead == 'u') ADVANCE(172);
      END_STATE();
    case 496:
      if (lookahead == 'u') ADVANCE(124);
      END_STATE();
    case 497:
      if (lookahead == 'v') ADVANCE(227);
      END_STATE();
    case 498:
      if (lookahead == 'v') ADVANCE(57);
      END_STATE();
    case 499:
      if (lookahead == 'v') ADVANCE(138);
      END_STATE();
    case 500:
      if (lookahead == 'w') ADVANCE(599);
      END_STATE();
    case 501:
      if (lookahead == 'w') ADVANCE(314);
      END_STATE();
    case 502:
      if (lookahead == 'w') ADVANCE(169);
      END_STATE();
    case 503:
      if (lookahead == 'x') ADVANCE(13);
      END_STATE();
    case 504:
      if (lookahead == 'x') ADVANCE(444);
      END_STATE();
    case 505:
      if (lookahead == 'x') ADVANCE(18);
      END_STATE();
    case 506:
      if (lookahead == 'x') ADVANCE(36);
      END_STATE();
    case 507:
      if (lookahead == 'y') ADVANCE(566);
      END_STATE();
    case 508:
      if (lookahead == 'y') ADVANCE(556);
      END_STATE();
    case 509:
      if (lookahead == 'y') ADVANCE(564);
      END_STATE();
    case 510:
      if (lookahead == 'y') ADVANCE(528);
      END_STATE();
    case 511:
      if (lookahead == 'y') ADVANCE(602);
      END_STATE();
    case 512:
      if (lookahead == 'y') ADVANCE(554);
      END_STATE();
    case 513:
      if (lookahead == 'y') ADVANCE(584);
      END_STATE();
    case 514:
      if (lookahead == 'y') ADVANCE(607);
      END_STATE();
    case 515:
      if (lookahead == 'y') ADVANCE(561);
      END_STATE();
    case 516:
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(616);
      END_STATE();
    case 517:
      if (lookahead != 0 &&
          lookahead != '\n') ADVANCE(3);
      END_STATE();
    case 518:
      if (eof) ADVANCE(519);
      ADVANCE_MAP(
        '/', 12,
        'a', 203,
        'c', 38,
        'd', 149,
        'e', 407,
        'i', 295,
        'm', 66,
        'o', 501,
        'p', 45,
        'r', 173,
        's', 92,
        't', 184,
        'v', 49,
        'w', 336,
        '}', 524,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(518);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(618);
      END_STATE();
    case 519:
      ACCEPT_TOKEN(ts_builtin_sym_end);
      END_STATE();
    case 520:
      ACCEPT_TOKEN(anon_sym_include);
      END_STATE();
    case 521:
      ACCEPT_TOKEN(sym_comment);
      if (lookahead != 0 &&
          lookahead != '\n') ADVANCE(521);
      END_STATE();
    case 522:
      ACCEPT_TOKEN(anon_sym_client);
      END_STATE();
    case 523:
      ACCEPT_TOKEN(anon_sym_LBRACE);
      END_STATE();
    case 524:
      ACCEPT_TOKEN(anon_sym_RBRACE);
      END_STATE();
    case 525:
      ACCEPT_TOKEN(anon_sym_tier);
      END_STATE();
    case 526:
      ACCEPT_TOKEN(anon_sym_model);
      END_STATE();
    case 527:
      ACCEPT_TOKEN(anon_sym_effort);
      END_STATE();
    case 528:
      ACCEPT_TOKEN(anon_sym_privacy);
      END_STATE();
    case 529:
      ACCEPT_TOKEN(anon_sym_default);
      END_STATE();
    case 530:
      ACCEPT_TOKEN(anon_sym_extra);
      END_STATE();
    case 531:
      ACCEPT_TOKEN(anon_sym_vars);
      END_STATE();
    case 532:
      ACCEPT_TOKEN(anon_sym_cheap);
      END_STATE();
    case 533:
      ACCEPT_TOKEN(anon_sym_cheap);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 534:
      ACCEPT_TOKEN(anon_sym_expensive);
      END_STATE();
    case 535:
      ACCEPT_TOKEN(anon_sym_expensive);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 536:
      ACCEPT_TOKEN(anon_sym_coordinator);
      END_STATE();
    case 537:
      ACCEPT_TOKEN(anon_sym_coordinator);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 538:
      ACCEPT_TOKEN(anon_sym_reasoning);
      END_STATE();
    case 539:
      ACCEPT_TOKEN(anon_sym_reasoning);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 540:
      ACCEPT_TOKEN(anon_sym_execution);
      END_STATE();
    case 541:
      ACCEPT_TOKEN(anon_sym_execution);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 542:
      ACCEPT_TOKEN(anon_sym_mechanical);
      END_STATE();
    case 543:
      ACCEPT_TOKEN(anon_sym_mechanical);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 544:
      ACCEPT_TOKEN(anon_sym_prompt);
      END_STATE();
    case 545:
      ACCEPT_TOKEN(anon_sym_agent);
      END_STATE();
    case 546:
      ACCEPT_TOKEN(anon_sym_description);
      END_STATE();
    case 547:
      ACCEPT_TOKEN(anon_sym_depends_on);
      END_STATE();
    case 548:
      ACCEPT_TOKEN(anon_sym_max_retries);
      END_STATE();
    case 549:
      ACCEPT_TOKEN(anon_sym_tools);
      END_STATE();
    case 550:
      ACCEPT_TOKEN(anon_sym_template);
      END_STATE();
    case 551:
      ACCEPT_TOKEN(anon_sym_template);
      if (lookahead == '_') ADVANCE(237);
      END_STATE();
    case 552:
      ACCEPT_TOKEN(anon_sym_scope);
      END_STATE();
    case 553:
      ACCEPT_TOKEN(anon_sym_owned);
      END_STATE();
    case 554:
      ACCEPT_TOKEN(anon_sym_read_only);
      END_STATE();
    case 555:
      ACCEPT_TOKEN(anon_sym_impact_scope);
      END_STATE();
    case 556:
      ACCEPT_TOKEN(anon_sym_memory);
      END_STATE();
    case 557:
      ACCEPT_TOKEN(anon_sym_read_ns);
      END_STATE();
    case 558:
      ACCEPT_TOKEN(anon_sym_write_ns);
      END_STATE();
    case 559:
      ACCEPT_TOKEN(anon_sym_importance);
      END_STATE();
    case 560:
      ACCEPT_TOKEN(anon_sym_staleness_sources);
      END_STATE();
    case 561:
      ACCEPT_TOKEN(anon_sym_read_query);
      END_STATE();
    case 562:
      ACCEPT_TOKEN(anon_sym_read_limit);
      END_STATE();
    case 563:
      ACCEPT_TOKEN(anon_sym_write_content);
      END_STATE();
    case 564:
      ACCEPT_TOKEN(anon_sym_verify);
      END_STATE();
    case 565:
      ACCEPT_TOKEN(anon_sym_compile);
      END_STATE();
    case 566:
      ACCEPT_TOKEN(anon_sym_clippy);
      END_STATE();
    case 567:
      ACCEPT_TOKEN(anon_sym_test);
      END_STATE();
    case 568:
      ACCEPT_TOKEN(anon_sym_test);
      if (lookahead == '_') ADVANCE(198);
      END_STATE();
    case 569:
      ACCEPT_TOKEN(anon_sym_test);
      if (lookahead == '_') ADVANCE(198);
      if (lookahead == 's') ADVANCE(30);
      END_STATE();
    case 570:
      ACCEPT_TOKEN(anon_sym_impact_tests);
      END_STATE();
    case 571:
      ACCEPT_TOKEN(anon_sym_context);
      END_STATE();
    case 572:
      ACCEPT_TOKEN(anon_sym_callers_of);
      END_STATE();
    case 573:
      ACCEPT_TOKEN(anon_sym_tests_for);
      END_STATE();
    case 574:
      ACCEPT_TOKEN(anon_sym_depth);
      END_STATE();
    case 575:
      ACCEPT_TOKEN(anon_sym_loop);
      END_STATE();
    case 576:
      ACCEPT_TOKEN(anon_sym_loop);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 577:
      ACCEPT_TOKEN(anon_sym_agents);
      END_STATE();
    case 578:
      ACCEPT_TOKEN(anon_sym_reviewers);
      END_STATE();
    case 579:
      ACCEPT_TOKEN(anon_sym_template_init);
      END_STATE();
    case 580:
      ACCEPT_TOKEN(anon_sym_template_refine);
      END_STATE();
    case 581:
      ACCEPT_TOKEN(anon_sym_consensus_mode);
      END_STATE();
    case 582:
      ACCEPT_TOKEN(anon_sym_max_iterations);
      END_STATE();
    case 583:
      ACCEPT_TOKEN(anon_sym_iter_start);
      END_STATE();
    case 584:
      ACCEPT_TOKEN(anon_sym_stability);
      END_STATE();
    case 585:
      ACCEPT_TOKEN(anon_sym_judge_timeout);
      END_STATE();
    case 586:
      ACCEPT_TOKEN(anon_sym_strict_judge);
      END_STATE();
    case 587:
      ACCEPT_TOKEN(anon_sym_branch_chain);
      END_STATE();
    case 588:
      ACCEPT_TOKEN(anon_sym_LBRACK);
      END_STATE();
    case 589:
      ACCEPT_TOKEN(anon_sym_RBRACK);
      END_STATE();
    case 590:
      ACCEPT_TOKEN(anon_sym_id);
      END_STATE();
    case 591:
      ACCEPT_TOKEN(anon_sym_strict);
      END_STATE();
    case 592:
      ACCEPT_TOKEN(anon_sym_strict);
      if (lookahead == '_') ADVANCE(248);
      END_STATE();
    case 593:
      ACCEPT_TOKEN(anon_sym_partial_ok);
      END_STATE();
    case 594:
      ACCEPT_TOKEN(anon_sym_explore);
      END_STATE();
    case 595:
      ACCEPT_TOKEN(anon_sym_stacked);
      END_STATE();
    case 596:
      ACCEPT_TOKEN(anon_sym_none);
      END_STATE();
    case 597:
      ACCEPT_TOKEN(anon_sym_until);
      END_STATE();
    case 598:
      ACCEPT_TOKEN(anon_sym_command);
      END_STATE();
    case 599:
      ACCEPT_TOKEN(anon_sym_workflow);
      END_STATE();
    case 600:
      ACCEPT_TOKEN(anon_sym_steps);
      END_STATE();
    case 601:
      ACCEPT_TOKEN(anon_sym_max_parallel);
      END_STATE();
    case 602:
      ACCEPT_TOKEN(anon_sym_strategy);
      END_STATE();
    case 603:
      ACCEPT_TOKEN(anon_sym_test_first);
      END_STATE();
    case 604:
      ACCEPT_TOKEN(anon_sym_attempts);
      END_STATE();
    case 605:
      ACCEPT_TOKEN(anon_sym_escalate_after);
      END_STATE();
    case 606:
      ACCEPT_TOKEN(anon_sym_public);
      END_STATE();
    case 607:
      ACCEPT_TOKEN(anon_sym_local_only);
      END_STATE();
    case 608:
      ACCEPT_TOKEN(anon_sym_single_pass);
      END_STATE();
    case 609:
      ACCEPT_TOKEN(anon_sym_single_pass);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 610:
      ACCEPT_TOKEN(anon_sym_refine);
      END_STATE();
    case 611:
      ACCEPT_TOKEN(anon_sym_refine);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 612:
      ACCEPT_TOKEN(anon_sym_true);
      END_STATE();
    case 613:
      ACCEPT_TOKEN(anon_sym_false);
      END_STATE();
    case 614:
      ACCEPT_TOKEN(sym_string);
      END_STATE();
    case 615:
      ACCEPT_TOKEN(sym_raw_string);
      END_STATE();
    case 616:
      ACCEPT_TOKEN(sym_float);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(616);
      END_STATE();
    case 617:
      ACCEPT_TOKEN(sym_integer);
      if (lookahead == '.') ADVANCE(516);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(617);
      END_STATE();
    case 618:
      ACCEPT_TOKEN(sym_integer);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(618);
      END_STATE();
    case 619:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '_') ADVANCE(669);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 620:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(673);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 621:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(667);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 622:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(651);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 623:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(658);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 624:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(677);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 625:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(675);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 626:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(643);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 627:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(678);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 628:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(622);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 629:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'd') ADVANCE(646);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 630:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(626);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 631:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(620);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 632:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(655);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 633:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(535);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 634:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(639);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 635:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(611);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 636:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(619);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 637:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(627);
      if (lookahead == 'p') ADVANCE(632);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 638:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(621);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 639:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'f') ADVANCE(649);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 640:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'g') ADVANCE(539);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 641:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'g') ADVANCE(652);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 642:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(638);
      if (lookahead == 'o') ADVANCE(661);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 643:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(623);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 644:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(679);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 645:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(628);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 646:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(657);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 647:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(653);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 648:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(656);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 649:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(659);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 650:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(666);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 651:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(543);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 652:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(636);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 653:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(640);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 654:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(541);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 655:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(674);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 656:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(641);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 657:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(624);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 658:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(645);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 659:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(635);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 660:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(647);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 661:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(670);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 662:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(671);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 663:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(668);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 664:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(660);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 665:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(663);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 666:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(654);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 667:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(533);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 668:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(576);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 669:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(625);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 670:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(629);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 671:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(537);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 672:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(609);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 673:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(664);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 674:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(644);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 675:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(672);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 676:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(650);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 677:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(662);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 678:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'u') ADVANCE(676);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 679:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'v') ADVANCE(633);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 680:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'x') ADVANCE(637);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    case 681:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(681);
      END_STATE();
    default:
      return false;
  }
}

static const TSLexMode ts_lex_modes[STATE_COUNT] = {
  [0] = {.lex_state = 0},
  [1] = {.lex_state = 0},
  [2] = {.lex_state = 10},
  [3] = {.lex_state = 518},
  [4] = {.lex_state = 518},
  [5] = {.lex_state = 10},
  [6] = {.lex_state = 10},
  [7] = {.lex_state = 518},
  [8] = {.lex_state = 518},
  [9] = {.lex_state = 518},
  [10] = {.lex_state = 518},
  [11] = {.lex_state = 518},
  [12] = {.lex_state = 518},
  [13] = {.lex_state = 0},
  [14] = {.lex_state = 518},
  [15] = {.lex_state = 0},
  [16] = {.lex_state = 9},
  [17] = {.lex_state = 9},
  [18] = {.lex_state = 9},
  [19] = {.lex_state = 518},
  [20] = {.lex_state = 518},
  [21] = {.lex_state = 518},
  [22] = {.lex_state = 9},
  [23] = {.lex_state = 9},
  [24] = {.lex_state = 518},
  [25] = {.lex_state = 518},
  [26] = {.lex_state = 518},
  [27] = {.lex_state = 518},
  [28] = {.lex_state = 518},
  [29] = {.lex_state = 9},
  [30] = {.lex_state = 9},
  [31] = {.lex_state = 9},
  [32] = {.lex_state = 9},
  [33] = {.lex_state = 9},
  [34] = {.lex_state = 9},
  [35] = {.lex_state = 9},
  [36] = {.lex_state = 9},
  [37] = {.lex_state = 518},
  [38] = {.lex_state = 9},
  [39] = {.lex_state = 518},
  [40] = {.lex_state = 2},
  [41] = {.lex_state = 0},
  [42] = {.lex_state = 0},
  [43] = {.lex_state = 518},
  [44] = {.lex_state = 518},
  [45] = {.lex_state = 0},
  [46] = {.lex_state = 518},
  [47] = {.lex_state = 0},
  [48] = {.lex_state = 518},
  [49] = {.lex_state = 518},
  [50] = {.lex_state = 0},
  [51] = {.lex_state = 0},
  [52] = {.lex_state = 518},
  [53] = {.lex_state = 518},
  [54] = {.lex_state = 0},
  [55] = {.lex_state = 0},
  [56] = {.lex_state = 0},
  [57] = {.lex_state = 0},
  [58] = {.lex_state = 2},
  [59] = {.lex_state = 0},
  [60] = {.lex_state = 2},
  [61] = {.lex_state = 0},
  [62] = {.lex_state = 0},
  [63] = {.lex_state = 0},
  [64] = {.lex_state = 0},
  [65] = {.lex_state = 0},
  [66] = {.lex_state = 9},
  [67] = {.lex_state = 0},
  [68] = {.lex_state = 0},
  [69] = {.lex_state = 1},
  [70] = {.lex_state = 0},
  [71] = {.lex_state = 0},
  [72] = {.lex_state = 9},
  [73] = {.lex_state = 0},
  [74] = {.lex_state = 0},
  [75] = {.lex_state = 0},
  [76] = {.lex_state = 9},
  [77] = {.lex_state = 9},
  [78] = {.lex_state = 1},
  [79] = {.lex_state = 9},
  [80] = {.lex_state = 0},
  [81] = {.lex_state = 1},
  [82] = {.lex_state = 518},
  [83] = {.lex_state = 0},
  [84] = {.lex_state = 0},
  [85] = {.lex_state = 518},
  [86] = {.lex_state = 0},
  [87] = {.lex_state = 518},
  [88] = {.lex_state = 0},
  [89] = {.lex_state = 8},
  [90] = {.lex_state = 0},
  [91] = {.lex_state = 0},
  [92] = {.lex_state = 8},
  [93] = {.lex_state = 8},
  [94] = {.lex_state = 9},
  [95] = {.lex_state = 0},
  [96] = {.lex_state = 0},
  [97] = {.lex_state = 0},
  [98] = {.lex_state = 518},
  [99] = {.lex_state = 11},
  [100] = {.lex_state = 1},
  [101] = {.lex_state = 0},
  [102] = {.lex_state = 1},
  [103] = {.lex_state = 0},
  [104] = {.lex_state = 518},
  [105] = {.lex_state = 1},
  [106] = {.lex_state = 1},
  [107] = {.lex_state = 0},
  [108] = {.lex_state = 1},
  [109] = {.lex_state = 0},
  [110] = {.lex_state = 0},
  [111] = {.lex_state = 1},
  [112] = {.lex_state = 1},
  [113] = {.lex_state = 0},
  [114] = {.lex_state = 0},
  [115] = {.lex_state = 0},
  [116] = {.lex_state = 0},
  [117] = {.lex_state = 0},
  [118] = {.lex_state = 8},
  [119] = {.lex_state = 1},
  [120] = {.lex_state = 0},
  [121] = {.lex_state = 0},
  [122] = {.lex_state = 0},
  [123] = {.lex_state = 0},
  [124] = {.lex_state = 0},
  [125] = {.lex_state = 8},
  [126] = {.lex_state = 0},
  [127] = {.lex_state = 0},
  [128] = {.lex_state = 0},
  [129] = {.lex_state = 0},
  [130] = {.lex_state = 0},
  [131] = {.lex_state = 0},
  [132] = {.lex_state = 1},
  [133] = {.lex_state = 0},
  [134] = {.lex_state = 0},
  [135] = {.lex_state = 0},
  [136] = {.lex_state = 0},
  [137] = {.lex_state = 0},
  [138] = {.lex_state = 0},
  [139] = {.lex_state = 0},
  [140] = {.lex_state = 0},
  [141] = {.lex_state = 0},
  [142] = {.lex_state = 0},
  [143] = {.lex_state = 1},
  [144] = {.lex_state = 1},
  [145] = {.lex_state = 1},
  [146] = {.lex_state = 0},
  [147] = {.lex_state = 1},
  [148] = {.lex_state = 1},
  [149] = {.lex_state = 0},
  [150] = {.lex_state = 0},
  [151] = {.lex_state = 10},
  [152] = {.lex_state = 518},
  [153] = {.lex_state = 0},
  [154] = {.lex_state = 1},
  [155] = {.lex_state = 518},
  [156] = {.lex_state = 518},
  [157] = {.lex_state = 0},
  [158] = {.lex_state = 1},
  [159] = {.lex_state = 1},
  [160] = {.lex_state = 0},
  [161] = {.lex_state = 518},
  [162] = {.lex_state = 0},
  [163] = {.lex_state = 0},
  [164] = {.lex_state = 1},
  [165] = {.lex_state = 0},
  [166] = {.lex_state = 1},
  [167] = {.lex_state = 518},
  [168] = {.lex_state = 0},
  [169] = {.lex_state = 0},
};

static const uint16_t ts_parse_table[LARGE_STATE_COUNT][SYMBOL_COUNT] = {
  [0] = {
    [ts_builtin_sym_end] = ACTIONS(1),
    [anon_sym_include] = ACTIONS(1),
    [sym_comment] = ACTIONS(3),
    [anon_sym_client] = ACTIONS(1),
    [anon_sym_LBRACE] = ACTIONS(1),
    [anon_sym_RBRACE] = ACTIONS(1),
    [anon_sym_tier] = ACTIONS(1),
    [anon_sym_model] = ACTIONS(1),
    [anon_sym_effort] = ACTIONS(1),
    [anon_sym_privacy] = ACTIONS(1),
    [anon_sym_default] = ACTIONS(1),
    [anon_sym_extra] = ACTIONS(1),
    [anon_sym_vars] = ACTIONS(1),
    [anon_sym_cheap] = ACTIONS(1),
    [anon_sym_expensive] = ACTIONS(1),
    [anon_sym_coordinator] = ACTIONS(1),
    [anon_sym_reasoning] = ACTIONS(1),
    [anon_sym_execution] = ACTIONS(1),
    [anon_sym_mechanical] = ACTIONS(1),
    [anon_sym_prompt] = ACTIONS(1),
    [anon_sym_agent] = ACTIONS(1),
    [anon_sym_description] = ACTIONS(1),
    [anon_sym_depends_on] = ACTIONS(1),
    [anon_sym_max_retries] = ACTIONS(1),
    [anon_sym_tools] = ACTIONS(1),
    [anon_sym_template] = ACTIONS(1),
    [anon_sym_scope] = ACTIONS(1),
    [anon_sym_owned] = ACTIONS(1),
    [anon_sym_read_only] = ACTIONS(1),
    [anon_sym_impact_scope] = ACTIONS(1),
    [anon_sym_memory] = ACTIONS(1),
    [anon_sym_read_ns] = ACTIONS(1),
    [anon_sym_write_ns] = ACTIONS(1),
    [anon_sym_importance] = ACTIONS(1),
    [anon_sym_staleness_sources] = ACTIONS(1),
    [anon_sym_read_query] = ACTIONS(1),
    [anon_sym_read_limit] = ACTIONS(1),
    [anon_sym_write_content] = ACTIONS(1),
    [anon_sym_verify] = ACTIONS(1),
    [anon_sym_compile] = ACTIONS(1),
    [anon_sym_clippy] = ACTIONS(1),
    [anon_sym_test] = ACTIONS(1),
    [anon_sym_impact_tests] = ACTIONS(1),
    [anon_sym_context] = ACTIONS(1),
    [anon_sym_callers_of] = ACTIONS(1),
    [anon_sym_tests_for] = ACTIONS(1),
    [anon_sym_depth] = ACTIONS(1),
    [anon_sym_loop] = ACTIONS(1),
    [anon_sym_reviewers] = ACTIONS(1),
    [anon_sym_template_init] = ACTIONS(1),
    [anon_sym_template_refine] = ACTIONS(1),
    [anon_sym_consensus_mode] = ACTIONS(1),
    [anon_sym_max_iterations] = ACTIONS(1),
    [anon_sym_iter_start] = ACTIONS(1),
    [anon_sym_stability] = ACTIONS(1),
    [anon_sym_judge_timeout] = ACTIONS(1),
    [anon_sym_strict_judge] = ACTIONS(1),
    [anon_sym_branch_chain] = ACTIONS(1),
    [anon_sym_LBRACK] = ACTIONS(1),
    [anon_sym_RBRACK] = ACTIONS(1),
    [anon_sym_id] = ACTIONS(1),
    [anon_sym_strict] = ACTIONS(1),
    [anon_sym_partial_ok] = ACTIONS(1),
    [anon_sym_explore] = ACTIONS(1),
    [anon_sym_stacked] = ACTIONS(1),
    [anon_sym_none] = ACTIONS(1),
    [anon_sym_until] = ACTIONS(1),
    [anon_sym_command] = ACTIONS(1),
    [anon_sym_workflow] = ACTIONS(1),
    [anon_sym_steps] = ACTIONS(1),
    [anon_sym_max_parallel] = ACTIONS(1),
    [anon_sym_strategy] = ACTIONS(1),
    [anon_sym_test_first] = ACTIONS(1),
    [anon_sym_attempts] = ACTIONS(1),
    [anon_sym_escalate_after] = ACTIONS(1),
    [anon_sym_public] = ACTIONS(1),
    [anon_sym_local_only] = ACTIONS(1),
    [anon_sym_single_pass] = ACTIONS(1),
    [anon_sym_refine] = ACTIONS(1),
    [anon_sym_true] = ACTIONS(1),
    [anon_sym_false] = ACTIONS(1),
    [sym_string] = ACTIONS(1),
    [sym_raw_string] = ACTIONS(1),
    [sym_float] = ACTIONS(1),
    [sym_integer] = ACTIONS(1),
  },
  [1] = {
    [sym_source_file] = STATE(162),
    [sym__definition] = STATE(15),
    [sym_include_declaration] = STATE(15),
    [sym_client_declaration] = STATE(15),
    [sym_vars_block] = STATE(15),
    [sym_tier_alias_declaration] = STATE(15),
    [sym_prompt_declaration] = STATE(15),
    [sym_agent_declaration] = STATE(15),
    [sym_workflow_declaration] = STATE(15),
    [aux_sym_source_file_repeat1] = STATE(15),
    [ts_builtin_sym_end] = ACTIONS(5),
    [anon_sym_include] = ACTIONS(7),
    [sym_comment] = ACTIONS(3),
    [anon_sym_client] = ACTIONS(9),
    [anon_sym_tier] = ACTIONS(11),
    [anon_sym_vars] = ACTIONS(13),
    [anon_sym_prompt] = ACTIONS(15),
    [anon_sym_agent] = ACTIONS(17),
    [anon_sym_workflow] = ACTIONS(19),
  },
};

static const uint16_t ts_small_parse_table[] = {
  [0] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(23), 2,
      anon_sym_template,
      anon_sym_test,
    ACTIONS(21), 37,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_scope,
      anon_sym_owned,
      anon_sym_read_only,
      anon_sym_impact_scope,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_impact_tests,
      anon_sym_context,
      anon_sym_agents,
      anon_sym_reviewers,
      anon_sym_template_init,
      anon_sym_template_refine,
      anon_sym_consensus_mode,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [47] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(25), 26,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_template,
      anon_sym_scope,
      anon_sym_owned,
      anon_sym_read_only,
      anon_sym_impact_scope,
      anon_sym_memory,
      anon_sym_read_ns,
      anon_sym_write_ns,
      anon_sym_importance,
      anon_sym_staleness_sources,
      anon_sym_read_query,
      anon_sym_read_limit,
      anon_sym_write_content,
      anon_sym_context,
      anon_sym_callers_of,
      anon_sym_tests_for,
      anon_sym_depth,
  [79] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(27), 26,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_template,
      anon_sym_scope,
      anon_sym_owned,
      anon_sym_read_only,
      anon_sym_impact_scope,
      anon_sym_memory,
      anon_sym_read_ns,
      anon_sym_write_ns,
      anon_sym_importance,
      anon_sym_staleness_sources,
      anon_sym_read_query,
      anon_sym_read_limit,
      anon_sym_write_content,
      anon_sym_context,
      anon_sym_callers_of,
      anon_sym_tests_for,
      anon_sym_depth,
  [111] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(31), 1,
      anon_sym_template,
    ACTIONS(29), 24,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
      anon_sym_agents,
      anon_sym_reviewers,
      anon_sym_template_init,
      anon_sym_template_refine,
      anon_sym_consensus_mode,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [144] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(35), 1,
      anon_sym_template,
    ACTIONS(33), 24,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
      anon_sym_agents,
      anon_sym_reviewers,
      anon_sym_template_init,
      anon_sym_template_refine,
      anon_sym_consensus_mode,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [177] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(37), 20,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_template,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_context,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [203] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(39), 20,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_template,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_context,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [229] = 16,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(13), 1,
      anon_sym_vars,
    ACTIONS(41), 1,
      anon_sym_client,
    ACTIONS(43), 1,
      anon_sym_RBRACE,
    ACTIONS(45), 1,
      anon_sym_tier,
    ACTIONS(47), 1,
      anon_sym_prompt,
    ACTIONS(49), 1,
      anon_sym_description,
    ACTIONS(51), 1,
      anon_sym_depends_on,
    ACTIONS(53), 1,
      anon_sym_max_retries,
    ACTIONS(55), 1,
      anon_sym_tools,
    ACTIONS(57), 1,
      anon_sym_template,
    ACTIONS(59), 1,
      anon_sym_scope,
    ACTIONS(61), 1,
      anon_sym_memory,
    ACTIONS(63), 1,
      anon_sym_context,
    STATE(11), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(37), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [282] = 16,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(13), 1,
      anon_sym_vars,
    ACTIONS(41), 1,
      anon_sym_client,
    ACTIONS(45), 1,
      anon_sym_tier,
    ACTIONS(47), 1,
      anon_sym_prompt,
    ACTIONS(49), 1,
      anon_sym_description,
    ACTIONS(51), 1,
      anon_sym_depends_on,
    ACTIONS(53), 1,
      anon_sym_max_retries,
    ACTIONS(55), 1,
      anon_sym_tools,
    ACTIONS(57), 1,
      anon_sym_template,
    ACTIONS(59), 1,
      anon_sym_scope,
    ACTIONS(61), 1,
      anon_sym_memory,
    ACTIONS(63), 1,
      anon_sym_context,
    ACTIONS(65), 1,
      anon_sym_RBRACE,
    STATE(9), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(37), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [335] = 16,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(67), 1,
      anon_sym_client,
    ACTIONS(70), 1,
      anon_sym_RBRACE,
    ACTIONS(72), 1,
      anon_sym_tier,
    ACTIONS(75), 1,
      anon_sym_vars,
    ACTIONS(78), 1,
      anon_sym_prompt,
    ACTIONS(81), 1,
      anon_sym_description,
    ACTIONS(84), 1,
      anon_sym_depends_on,
    ACTIONS(87), 1,
      anon_sym_max_retries,
    ACTIONS(90), 1,
      anon_sym_tools,
    ACTIONS(93), 1,
      anon_sym_template,
    ACTIONS(96), 1,
      anon_sym_scope,
    ACTIONS(99), 1,
      anon_sym_memory,
    ACTIONS(102), 1,
      anon_sym_context,
    STATE(11), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(37), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [388] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(105), 17,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_template,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
      anon_sym_workflow,
  [411] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(107), 1,
      ts_builtin_sym_end,
    ACTIONS(109), 1,
      anon_sym_include,
    ACTIONS(112), 1,
      anon_sym_client,
    ACTIONS(115), 1,
      anon_sym_tier,
    ACTIONS(118), 1,
      anon_sym_vars,
    ACTIONS(121), 1,
      anon_sym_prompt,
    ACTIONS(124), 1,
      anon_sym_agent,
    ACTIONS(127), 1,
      anon_sym_workflow,
    STATE(13), 9,
      sym__definition,
      sym_include_declaration,
      sym_client_declaration,
      sym_vars_block,
      sym_tier_alias_declaration,
      sym_prompt_declaration,
      sym_agent_declaration,
      sym_workflow_declaration,
      aux_sym_source_file_repeat1,
  [450] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(130), 17,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_template,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
      anon_sym_workflow,
  [473] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(7), 1,
      anon_sym_include,
    ACTIONS(9), 1,
      anon_sym_client,
    ACTIONS(11), 1,
      anon_sym_tier,
    ACTIONS(13), 1,
      anon_sym_vars,
    ACTIONS(15), 1,
      anon_sym_prompt,
    ACTIONS(17), 1,
      anon_sym_agent,
    ACTIONS(19), 1,
      anon_sym_workflow,
    ACTIONS(132), 1,
      ts_builtin_sym_end,
    STATE(13), 9,
      sym__definition,
      sym_include_declaration,
      sym_client_declaration,
      sym_vars_block,
      sym_tier_alias_declaration,
      sym_prompt_declaration,
      sym_agent_declaration,
      sym_workflow_declaration,
      aux_sym_source_file_repeat1,
  [512] = 12,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(134), 1,
      anon_sym_RBRACE,
    ACTIONS(136), 1,
      anon_sym_agents,
    ACTIONS(138), 1,
      anon_sym_reviewers,
    ACTIONS(142), 1,
      anon_sym_consensus_mode,
    ACTIONS(146), 1,
      anon_sym_strict_judge,
    ACTIONS(148), 1,
      anon_sym_branch_chain,
    ACTIONS(150), 1,
      anon_sym_until,
    STATE(29), 1,
      sym_until_clause,
    ACTIONS(140), 2,
      anon_sym_template_init,
      anon_sym_template_refine,
    STATE(18), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(144), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [554] = 12,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(136), 1,
      anon_sym_agents,
    ACTIONS(138), 1,
      anon_sym_reviewers,
    ACTIONS(142), 1,
      anon_sym_consensus_mode,
    ACTIONS(146), 1,
      anon_sym_strict_judge,
    ACTIONS(148), 1,
      anon_sym_branch_chain,
    ACTIONS(150), 1,
      anon_sym_until,
    ACTIONS(152), 1,
      anon_sym_RBRACE,
    STATE(29), 1,
      sym_until_clause,
    ACTIONS(140), 2,
      anon_sym_template_init,
      anon_sym_template_refine,
    STATE(16), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(144), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [596] = 12,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(154), 1,
      anon_sym_RBRACE,
    ACTIONS(156), 1,
      anon_sym_agents,
    ACTIONS(159), 1,
      anon_sym_reviewers,
    ACTIONS(165), 1,
      anon_sym_consensus_mode,
    ACTIONS(171), 1,
      anon_sym_strict_judge,
    ACTIONS(174), 1,
      anon_sym_branch_chain,
    ACTIONS(177), 1,
      anon_sym_until,
    STATE(29), 1,
      sym_until_clause,
    ACTIONS(162), 2,
      anon_sym_template_init,
      anon_sym_template_refine,
    STATE(18), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(168), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [638] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(61), 1,
      anon_sym_memory,
    ACTIONS(180), 1,
      anon_sym_RBRACE,
    ACTIONS(184), 1,
      anon_sym_verify,
    ACTIONS(186), 1,
      anon_sym_steps,
    ACTIONS(188), 1,
      anon_sym_strategy,
    ACTIONS(190), 1,
      anon_sym_test_first,
    STATE(20), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(46), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(182), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [674] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(61), 1,
      anon_sym_memory,
    ACTIONS(184), 1,
      anon_sym_verify,
    ACTIONS(186), 1,
      anon_sym_steps,
    ACTIONS(188), 1,
      anon_sym_strategy,
    ACTIONS(190), 1,
      anon_sym_test_first,
    ACTIONS(192), 1,
      anon_sym_RBRACE,
    STATE(21), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(46), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(182), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [710] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(194), 1,
      anon_sym_RBRACE,
    ACTIONS(199), 1,
      anon_sym_memory,
    ACTIONS(202), 1,
      anon_sym_verify,
    ACTIONS(205), 1,
      anon_sym_steps,
    ACTIONS(208), 1,
      anon_sym_strategy,
    ACTIONS(211), 1,
      anon_sym_test_first,
    STATE(21), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(46), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(196), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [746] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(214), 13,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_reviewers,
      anon_sym_template_init,
      anon_sym_template_refine,
      anon_sym_consensus_mode,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [765] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(216), 13,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_reviewers,
      anon_sym_template_init,
      anon_sym_template_refine,
      anon_sym_consensus_mode,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [784] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(218), 13,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_template,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [803] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(220), 13,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_template,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [822] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(222), 13,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_template,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [841] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(224), 13,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_template,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [860] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(226), 13,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_template,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [879] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(228), 13,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_reviewers,
      anon_sym_template_init,
      anon_sym_template_refine,
      anon_sym_consensus_mode,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [898] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(230), 13,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_reviewers,
      anon_sym_template_init,
      anon_sym_template_refine,
      anon_sym_consensus_mode,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [917] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(232), 13,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_reviewers,
      anon_sym_template_init,
      anon_sym_template_refine,
      anon_sym_consensus_mode,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [936] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(234), 13,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_reviewers,
      anon_sym_template_init,
      anon_sym_template_refine,
      anon_sym_consensus_mode,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [955] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(236), 13,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_reviewers,
      anon_sym_template_init,
      anon_sym_template_refine,
      anon_sym_consensus_mode,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [974] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(238), 13,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_reviewers,
      anon_sym_template_init,
      anon_sym_template_refine,
      anon_sym_consensus_mode,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [993] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(240), 13,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_reviewers,
      anon_sym_template_init,
      anon_sym_template_refine,
      anon_sym_consensus_mode,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [1012] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(242), 13,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_reviewers,
      anon_sym_template_init,
      anon_sym_template_refine,
      anon_sym_consensus_mode,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [1031] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(244), 13,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_template,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [1050] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(246), 13,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_reviewers,
      anon_sym_template_init,
      anon_sym_template_refine,
      anon_sym_consensus_mode,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [1069] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(248), 13,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_tools,
      anon_sym_template,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [1088] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(254), 1,
      sym_identifier,
    ACTIONS(252), 2,
      sym_string,
      sym_raw_string,
    STATE(70), 3,
      sym__effort_value,
      sym_tier_value,
      sym__string_value,
    ACTIONS(250), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [1112] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(256), 1,
      anon_sym_RBRACE,
    ACTIONS(262), 1,
      anon_sym_importance,
    ACTIONS(264), 1,
      anon_sym_read_limit,
    ACTIONS(258), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(51), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(260), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [1138] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(266), 1,
      anon_sym_RBRACE,
    ACTIONS(268), 1,
      anon_sym_tier,
    ACTIONS(271), 1,
      anon_sym_model,
    ACTIONS(274), 1,
      anon_sym_effort,
    ACTIONS(277), 1,
      anon_sym_privacy,
    ACTIONS(280), 1,
      anon_sym_default,
    ACTIONS(283), 1,
      anon_sym_extra,
    STATE(67), 1,
      sym_extra_block,
    STATE(42), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [1170] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(286), 10,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [1186] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(288), 10,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [1202] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(290), 1,
      anon_sym_RBRACE,
    ACTIONS(292), 1,
      anon_sym_tier,
    ACTIONS(294), 1,
      anon_sym_model,
    ACTIONS(296), 1,
      anon_sym_effort,
    ACTIONS(298), 1,
      anon_sym_privacy,
    ACTIONS(300), 1,
      anon_sym_default,
    ACTIONS(302), 1,
      anon_sym_extra,
    STATE(67), 1,
      sym_extra_block,
    STATE(42), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [1234] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(304), 10,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [1250] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(306), 1,
      anon_sym_RBRACE,
    ACTIONS(314), 1,
      anon_sym_importance,
    ACTIONS(317), 1,
      anon_sym_read_limit,
    ACTIONS(308), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(47), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(311), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [1276] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(320), 10,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [1292] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(322), 10,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [1308] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(292), 1,
      anon_sym_tier,
    ACTIONS(294), 1,
      anon_sym_model,
    ACTIONS(296), 1,
      anon_sym_effort,
    ACTIONS(298), 1,
      anon_sym_privacy,
    ACTIONS(300), 1,
      anon_sym_default,
    ACTIONS(302), 1,
      anon_sym_extra,
    ACTIONS(324), 1,
      anon_sym_RBRACE,
    STATE(67), 1,
      sym_extra_block,
    STATE(45), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [1340] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(262), 1,
      anon_sym_importance,
    ACTIONS(264), 1,
      anon_sym_read_limit,
    ACTIONS(326), 1,
      anon_sym_RBRACE,
    ACTIONS(258), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(47), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(260), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [1366] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(328), 10,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [1382] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(330), 10,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [1398] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(332), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1412] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(334), 8,
      anon_sym_RBRACE,
      anon_sym_read_ns,
      anon_sym_write_ns,
      anon_sym_importance,
      anon_sym_staleness_sources,
      anon_sym_read_query,
      anon_sym_read_limit,
      anon_sym_write_content,
  [1426] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(336), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1440] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(338), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1454] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(24), 1,
      sym_tier_alias_name,
    ACTIONS(340), 7,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
      sym_identifier,
  [1470] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(342), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1484] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(154), 1,
      sym_tier_alias_name,
    ACTIONS(344), 7,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
      sym_identifier,
  [1500] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(346), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1514] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(348), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1528] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(350), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1542] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(352), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1556] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(354), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1570] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(356), 1,
      anon_sym_RBRACE,
    STATE(72), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(358), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1587] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(360), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1600] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(362), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1613] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(364), 1,
      anon_sym_RBRACE,
    STATE(116), 1,
      sym__string_value,
    STATE(81), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(366), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1632] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(368), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1645] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(370), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1658] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(372), 1,
      anon_sym_RBRACE,
    STATE(72), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(374), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1675] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(70), 1,
      sym_tier_value,
    ACTIONS(377), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [1690] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(379), 1,
      anon_sym_LBRACE,
    ACTIONS(381), 1,
      anon_sym_agent,
    ACTIONS(383), 1,
      anon_sym_command,
    STATE(32), 4,
      sym__until_condition,
      sym_until_verify,
      sym_until_agent,
      sym_until_command,
  [1709] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(385), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1722] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(387), 1,
      anon_sym_RBRACE,
    STATE(66), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(358), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1739] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(389), 1,
      anon_sym_RBRACE,
    STATE(79), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(358), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1756] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(391), 1,
      anon_sym_RBRACE,
    STATE(116), 1,
      sym__string_value,
    STATE(69), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(366), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1775] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(393), 1,
      anon_sym_RBRACE,
    STATE(72), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(358), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1792] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(395), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1805] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(397), 1,
      anon_sym_RBRACE,
    STATE(116), 1,
      sym__string_value,
    STATE(81), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(399), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1824] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(402), 1,
      anon_sym_RBRACE,
    ACTIONS(407), 1,
      anon_sym_depth,
    ACTIONS(404), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(82), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1842] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(410), 1,
      anon_sym_RBRACE,
    ACTIONS(415), 1,
      anon_sym_impact_scope,
    ACTIONS(412), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(83), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1860] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(418), 1,
      anon_sym_RBRACE,
    ACTIONS(422), 1,
      anon_sym_impact_scope,
    ACTIONS(420), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(86), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1878] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(424), 1,
      anon_sym_RBRACE,
    ACTIONS(428), 1,
      anon_sym_depth,
    ACTIONS(426), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(82), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1896] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(422), 1,
      anon_sym_impact_scope,
    ACTIONS(430), 1,
      anon_sym_RBRACE,
    ACTIONS(420), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(83), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1914] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(428), 1,
      anon_sym_depth,
    ACTIONS(432), 1,
      anon_sym_RBRACE,
    ACTIONS(426), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(85), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1932] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(434), 1,
      anon_sym_RBRACK,
    ACTIONS(436), 2,
      sym_string,
      sym_raw_string,
    STATE(95), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [1947] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(438), 1,
      anon_sym_loop,
    ACTIONS(440), 1,
      anon_sym_RBRACK,
    ACTIONS(442), 1,
      sym_identifier,
    STATE(93), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [1964] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(444), 1,
      anon_sym_client,
    ACTIONS(446), 1,
      anon_sym_RBRACE,
    ACTIONS(448), 1,
      anon_sym_id,
    STATE(96), 2,
      sym_reviewer_field,
      aux_sym_reviewer_entry_repeat1,
  [1981] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(450), 1,
      anon_sym_RBRACK,
    ACTIONS(452), 2,
      sym_string,
      sym_raw_string,
    STATE(88), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [1996] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(454), 1,
      anon_sym_loop,
    ACTIONS(457), 1,
      anon_sym_RBRACK,
    ACTIONS(459), 1,
      sym_identifier,
    STATE(92), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [2013] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(438), 1,
      anon_sym_loop,
    ACTIONS(462), 1,
      anon_sym_RBRACK,
    ACTIONS(464), 1,
      sym_identifier,
    STATE(92), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [2030] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(466), 5,
      anon_sym_RBRACE,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [2041] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(468), 1,
      anon_sym_RBRACK,
    ACTIONS(470), 2,
      sym_string,
      sym_raw_string,
    STATE(95), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [2056] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(444), 1,
      anon_sym_client,
    ACTIONS(448), 1,
      anon_sym_id,
    ACTIONS(473), 1,
      anon_sym_RBRACE,
    STATE(97), 2,
      sym_reviewer_field,
      aux_sym_reviewer_entry_repeat1,
  [2073] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(475), 1,
      anon_sym_client,
    ACTIONS(478), 1,
      anon_sym_RBRACE,
    ACTIONS(480), 1,
      anon_sym_id,
    STATE(97), 2,
      sym_reviewer_field,
      aux_sym_reviewer_entry_repeat1,
  [2090] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(483), 4,
      anon_sym_RBRACE,
      anon_sym_callers_of,
      anon_sym_tests_for,
      anon_sym_depth,
  [2100] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(53), 1,
      sym_strategy_value,
    ACTIONS(485), 3,
      anon_sym_single_pass,
      anon_sym_refine,
      sym_identifier,
  [2112] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(24), 1,
      sym__string_value,
    ACTIONS(487), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [2124] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(489), 1,
      anon_sym_LBRACE,
    ACTIONS(491), 1,
      anon_sym_RBRACK,
    STATE(103), 2,
      sym_reviewer_entry,
      aux_sym_reviewer_list_repeat1,
  [2138] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(493), 1,
      anon_sym_RBRACE,
    ACTIONS(495), 1,
      sym_identifier,
    STATE(102), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [2152] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(489), 1,
      anon_sym_LBRACE,
    ACTIONS(498), 1,
      anon_sym_RBRACK,
    STATE(107), 2,
      sym_reviewer_entry,
      aux_sym_reviewer_list_repeat1,
  [2166] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(30), 1,
      sym_consensus_mode_value,
    ACTIONS(500), 3,
      anon_sym_strict,
      anon_sym_partial_ok,
      anon_sym_explore,
  [2178] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(502), 1,
      anon_sym_RBRACE,
    ACTIONS(504), 1,
      sym_identifier,
    STATE(102), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [2192] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(504), 1,
      sym_identifier,
    ACTIONS(506), 1,
      anon_sym_RBRACE,
    STATE(105), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [2206] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(508), 1,
      anon_sym_LBRACE,
    ACTIONS(511), 1,
      anon_sym_RBRACK,
    STATE(107), 2,
      sym_reviewer_entry,
      aux_sym_reviewer_list_repeat1,
  [2220] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(513), 4,
      anon_sym_RBRACE,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [2230] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(515), 4,
      anon_sym_RBRACE,
      anon_sym_owned,
      anon_sym_read_only,
      anon_sym_impact_scope,
  [2240] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(131), 1,
      sym__string_value,
    ACTIONS(517), 2,
      sym_string,
      sym_raw_string,
  [2251] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(519), 1,
      anon_sym_RBRACK,
    ACTIONS(521), 1,
      sym_identifier,
    STATE(119), 1,
      aux_sym_identifier_list_repeat1,
  [2264] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(523), 1,
      anon_sym_RBRACK,
    ACTIONS(525), 1,
      sym_identifier,
    STATE(112), 1,
      aux_sym_identifier_list_repeat1,
  [2277] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(132), 1,
      sym__string_value,
    ACTIONS(528), 2,
      sym_string,
      sym_raw_string,
  [2288] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(24), 1,
      sym__string_value,
    ACTIONS(487), 2,
      sym_string,
      sym_raw_string,
  [2299] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(24), 1,
      sym_boolean,
    ACTIONS(530), 2,
      anon_sym_true,
      anon_sym_false,
  [2310] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(108), 1,
      sym__string_value,
    ACTIONS(532), 2,
      sym_string,
      sym_raw_string,
  [2321] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(61), 1,
      sym__string_value,
    ACTIONS(534), 2,
      sym_string,
      sym_raw_string,
  [2332] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(538), 1,
      anon_sym_RBRACK,
    ACTIONS(536), 2,
      anon_sym_loop,
      sym_identifier,
  [2343] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(540), 1,
      anon_sym_RBRACK,
    ACTIONS(542), 1,
      sym_identifier,
    STATE(112), 1,
      aux_sym_identifier_list_repeat1,
  [2356] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(30), 1,
      sym_boolean,
    ACTIONS(530), 2,
      anon_sym_true,
      anon_sym_false,
  [2367] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(30), 1,
      sym_branch_chain_value,
    ACTIONS(544), 2,
      anon_sym_stacked,
      anon_sym_none,
  [2378] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(109), 1,
      sym_boolean,
    ACTIONS(530), 2,
      anon_sym_true,
      anon_sym_false,
  [2389] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(36), 1,
      sym__string_value,
    ACTIONS(546), 2,
      sym_string,
      sym_raw_string,
  [2400] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(55), 1,
      sym__string_value,
    ACTIONS(548), 2,
      sym_string,
      sym_raw_string,
  [2411] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(552), 1,
      anon_sym_RBRACK,
    ACTIONS(550), 2,
      anon_sym_loop,
      sym_identifier,
  [2422] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(53), 1,
      sym_boolean,
    ACTIONS(530), 2,
      anon_sym_true,
      anon_sym_false,
  [2433] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(70), 1,
      sym__string_value,
    ACTIONS(252), 2,
      sym_string,
      sym_raw_string,
  [2444] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(94), 1,
      sym_boolean,
    ACTIONS(530), 2,
      anon_sym_true,
      anon_sym_false,
  [2455] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(54), 1,
      sym__string_value,
    ACTIONS(554), 2,
      sym_string,
      sym_raw_string,
  [2466] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(70), 1,
      sym_privacy_value,
    ACTIONS(556), 2,
      anon_sym_public,
      anon_sym_local_only,
  [2477] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(558), 3,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_id,
  [2486] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(560), 2,
      anon_sym_RBRACE,
      sym_identifier,
  [2494] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(562), 2,
      anon_sym_LBRACE,
      anon_sym_RBRACK,
  [2502] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(564), 1,
      anon_sym_LBRACK,
    STATE(24), 1,
      sym_identifier_list,
  [2512] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(566), 1,
      anon_sym_LBRACK,
    STATE(109), 1,
      sym_string_list,
  [2522] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(566), 1,
      anon_sym_LBRACK,
    STATE(24), 1,
      sym_string_list,
  [2532] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(568), 1,
      anon_sym_LBRACK,
    STATE(53), 1,
      sym_step_list,
  [2542] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(566), 1,
      anon_sym_LBRACK,
    STATE(55), 1,
      sym_string_list,
  [2552] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(566), 1,
      anon_sym_LBRACK,
    STATE(98), 1,
      sym_string_list,
  [2562] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(570), 2,
      anon_sym_LBRACE,
      anon_sym_RBRACK,
  [2570] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(572), 1,
      anon_sym_LBRACK,
    STATE(30), 1,
      sym_reviewer_list,
  [2580] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(564), 1,
      anon_sym_LBRACK,
    STATE(30), 1,
      sym_identifier_list,
  [2590] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(574), 1,
      sym_identifier,
  [2597] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(576), 1,
      sym_identifier,
  [2604] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(248), 1,
      sym_identifier,
  [2611] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(578), 1,
      anon_sym_LBRACE,
  [2618] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(580), 1,
      sym_identifier,
  [2625] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(487), 1,
      sym_identifier,
  [2632] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(582), 1,
      anon_sym_LBRACE,
  [2639] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(584), 1,
      anon_sym_LBRACE,
  [2646] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(548), 1,
      sym_float,
  [2653] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(548), 1,
      sym_integer,
  [2660] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(586), 1,
      anon_sym_LBRACE,
  [2667] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(588), 1,
      sym_identifier,
  [2674] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(590), 1,
      sym_integer,
  [2681] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(592), 1,
      sym_integer,
  [2688] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(594), 1,
      anon_sym_LBRACE,
  [2695] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(596), 1,
      sym_identifier,
  [2702] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(517), 1,
      sym_identifier,
  [2709] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(598), 1,
      anon_sym_LBRACE,
  [2716] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(576), 1,
      sym_integer,
  [2723] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(600), 1,
      ts_builtin_sym_end,
  [2730] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(602), 1,
      anon_sym_LBRACE,
  [2737] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(604), 1,
      sym_identifier,
  [2744] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(606), 1,
      anon_sym_LBRACE,
  [2751] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(608), 1,
      sym_identifier,
  [2758] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(487), 1,
      sym_integer,
  [2765] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(610), 1,
      anon_sym_LBRACE,
  [2772] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(612), 1,
      anon_sym_LBRACE,
};

static const uint32_t ts_small_parse_table_map[] = {
  [SMALL_STATE(2)] = 0,
  [SMALL_STATE(3)] = 47,
  [SMALL_STATE(4)] = 79,
  [SMALL_STATE(5)] = 111,
  [SMALL_STATE(6)] = 144,
  [SMALL_STATE(7)] = 177,
  [SMALL_STATE(8)] = 203,
  [SMALL_STATE(9)] = 229,
  [SMALL_STATE(10)] = 282,
  [SMALL_STATE(11)] = 335,
  [SMALL_STATE(12)] = 388,
  [SMALL_STATE(13)] = 411,
  [SMALL_STATE(14)] = 450,
  [SMALL_STATE(15)] = 473,
  [SMALL_STATE(16)] = 512,
  [SMALL_STATE(17)] = 554,
  [SMALL_STATE(18)] = 596,
  [SMALL_STATE(19)] = 638,
  [SMALL_STATE(20)] = 674,
  [SMALL_STATE(21)] = 710,
  [SMALL_STATE(22)] = 746,
  [SMALL_STATE(23)] = 765,
  [SMALL_STATE(24)] = 784,
  [SMALL_STATE(25)] = 803,
  [SMALL_STATE(26)] = 822,
  [SMALL_STATE(27)] = 841,
  [SMALL_STATE(28)] = 860,
  [SMALL_STATE(29)] = 879,
  [SMALL_STATE(30)] = 898,
  [SMALL_STATE(31)] = 917,
  [SMALL_STATE(32)] = 936,
  [SMALL_STATE(33)] = 955,
  [SMALL_STATE(34)] = 974,
  [SMALL_STATE(35)] = 993,
  [SMALL_STATE(36)] = 1012,
  [SMALL_STATE(37)] = 1031,
  [SMALL_STATE(38)] = 1050,
  [SMALL_STATE(39)] = 1069,
  [SMALL_STATE(40)] = 1088,
  [SMALL_STATE(41)] = 1112,
  [SMALL_STATE(42)] = 1138,
  [SMALL_STATE(43)] = 1170,
  [SMALL_STATE(44)] = 1186,
  [SMALL_STATE(45)] = 1202,
  [SMALL_STATE(46)] = 1234,
  [SMALL_STATE(47)] = 1250,
  [SMALL_STATE(48)] = 1276,
  [SMALL_STATE(49)] = 1292,
  [SMALL_STATE(50)] = 1308,
  [SMALL_STATE(51)] = 1340,
  [SMALL_STATE(52)] = 1366,
  [SMALL_STATE(53)] = 1382,
  [SMALL_STATE(54)] = 1398,
  [SMALL_STATE(55)] = 1412,
  [SMALL_STATE(56)] = 1426,
  [SMALL_STATE(57)] = 1440,
  [SMALL_STATE(58)] = 1454,
  [SMALL_STATE(59)] = 1470,
  [SMALL_STATE(60)] = 1484,
  [SMALL_STATE(61)] = 1500,
  [SMALL_STATE(62)] = 1514,
  [SMALL_STATE(63)] = 1528,
  [SMALL_STATE(64)] = 1542,
  [SMALL_STATE(65)] = 1556,
  [SMALL_STATE(66)] = 1570,
  [SMALL_STATE(67)] = 1587,
  [SMALL_STATE(68)] = 1600,
  [SMALL_STATE(69)] = 1613,
  [SMALL_STATE(70)] = 1632,
  [SMALL_STATE(71)] = 1645,
  [SMALL_STATE(72)] = 1658,
  [SMALL_STATE(73)] = 1675,
  [SMALL_STATE(74)] = 1690,
  [SMALL_STATE(75)] = 1709,
  [SMALL_STATE(76)] = 1722,
  [SMALL_STATE(77)] = 1739,
  [SMALL_STATE(78)] = 1756,
  [SMALL_STATE(79)] = 1775,
  [SMALL_STATE(80)] = 1792,
  [SMALL_STATE(81)] = 1805,
  [SMALL_STATE(82)] = 1824,
  [SMALL_STATE(83)] = 1842,
  [SMALL_STATE(84)] = 1860,
  [SMALL_STATE(85)] = 1878,
  [SMALL_STATE(86)] = 1896,
  [SMALL_STATE(87)] = 1914,
  [SMALL_STATE(88)] = 1932,
  [SMALL_STATE(89)] = 1947,
  [SMALL_STATE(90)] = 1964,
  [SMALL_STATE(91)] = 1981,
  [SMALL_STATE(92)] = 1996,
  [SMALL_STATE(93)] = 2013,
  [SMALL_STATE(94)] = 2030,
  [SMALL_STATE(95)] = 2041,
  [SMALL_STATE(96)] = 2056,
  [SMALL_STATE(97)] = 2073,
  [SMALL_STATE(98)] = 2090,
  [SMALL_STATE(99)] = 2100,
  [SMALL_STATE(100)] = 2112,
  [SMALL_STATE(101)] = 2124,
  [SMALL_STATE(102)] = 2138,
  [SMALL_STATE(103)] = 2152,
  [SMALL_STATE(104)] = 2166,
  [SMALL_STATE(105)] = 2178,
  [SMALL_STATE(106)] = 2192,
  [SMALL_STATE(107)] = 2206,
  [SMALL_STATE(108)] = 2220,
  [SMALL_STATE(109)] = 2230,
  [SMALL_STATE(110)] = 2240,
  [SMALL_STATE(111)] = 2251,
  [SMALL_STATE(112)] = 2264,
  [SMALL_STATE(113)] = 2277,
  [SMALL_STATE(114)] = 2288,
  [SMALL_STATE(115)] = 2299,
  [SMALL_STATE(116)] = 2310,
  [SMALL_STATE(117)] = 2321,
  [SMALL_STATE(118)] = 2332,
  [SMALL_STATE(119)] = 2343,
  [SMALL_STATE(120)] = 2356,
  [SMALL_STATE(121)] = 2367,
  [SMALL_STATE(122)] = 2378,
  [SMALL_STATE(123)] = 2389,
  [SMALL_STATE(124)] = 2400,
  [SMALL_STATE(125)] = 2411,
  [SMALL_STATE(126)] = 2422,
  [SMALL_STATE(127)] = 2433,
  [SMALL_STATE(128)] = 2444,
  [SMALL_STATE(129)] = 2455,
  [SMALL_STATE(130)] = 2466,
  [SMALL_STATE(131)] = 2477,
  [SMALL_STATE(132)] = 2486,
  [SMALL_STATE(133)] = 2494,
  [SMALL_STATE(134)] = 2502,
  [SMALL_STATE(135)] = 2512,
  [SMALL_STATE(136)] = 2522,
  [SMALL_STATE(137)] = 2532,
  [SMALL_STATE(138)] = 2542,
  [SMALL_STATE(139)] = 2552,
  [SMALL_STATE(140)] = 2562,
  [SMALL_STATE(141)] = 2570,
  [SMALL_STATE(142)] = 2580,
  [SMALL_STATE(143)] = 2590,
  [SMALL_STATE(144)] = 2597,
  [SMALL_STATE(145)] = 2604,
  [SMALL_STATE(146)] = 2611,
  [SMALL_STATE(147)] = 2618,
  [SMALL_STATE(148)] = 2625,
  [SMALL_STATE(149)] = 2632,
  [SMALL_STATE(150)] = 2639,
  [SMALL_STATE(151)] = 2646,
  [SMALL_STATE(152)] = 2653,
  [SMALL_STATE(153)] = 2660,
  [SMALL_STATE(154)] = 2667,
  [SMALL_STATE(155)] = 2674,
  [SMALL_STATE(156)] = 2681,
  [SMALL_STATE(157)] = 2688,
  [SMALL_STATE(158)] = 2695,
  [SMALL_STATE(159)] = 2702,
  [SMALL_STATE(160)] = 2709,
  [SMALL_STATE(161)] = 2716,
  [SMALL_STATE(162)] = 2723,
  [SMALL_STATE(163)] = 2730,
  [SMALL_STATE(164)] = 2737,
  [SMALL_STATE(165)] = 2744,
  [SMALL_STATE(166)] = 2751,
  [SMALL_STATE(167)] = 2758,
  [SMALL_STATE(168)] = 2765,
  [SMALL_STATE(169)] = 2772,
};

static const TSParseActionEntry ts_parse_actions[] = {
  [0] = {.entry = {.count = 0, .reusable = false}},
  [1] = {.entry = {.count = 1, .reusable = false}}, RECOVER(),
  [3] = {.entry = {.count = 1, .reusable = true}}, SHIFT_EXTRA(),
  [5] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 0, 0, 0),
  [7] = {.entry = {.count = 1, .reusable = true}}, SHIFT(117),
  [9] = {.entry = {.count = 1, .reusable = true}}, SHIFT(143),
  [11] = {.entry = {.count = 1, .reusable = true}}, SHIFT(60),
  [13] = {.entry = {.count = 1, .reusable = true}}, SHIFT(157),
  [15] = {.entry = {.count = 1, .reusable = true}}, SHIFT(164),
  [17] = {.entry = {.count = 1, .reusable = true}}, SHIFT(166),
  [19] = {.entry = {.count = 1, .reusable = true}}, SHIFT(158),
  [21] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_boolean, 1, 0, 0),
  [23] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_boolean, 1, 0, 0),
  [25] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 2, 0, 0),
  [27] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 3, 0, 0),
  [29] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 2, 0, 0),
  [31] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_identifier_list, 2, 0, 0),
  [33] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 3, 0, 0),
  [35] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_identifier_list, 3, 0, 0),
  [37] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_block, 3, 0, 0),
  [39] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_block, 4, 0, 0),
  [41] = {.entry = {.count = 1, .reusable = true}}, SHIFT(148),
  [43] = {.entry = {.count = 1, .reusable = true}}, SHIFT(59),
  [45] = {.entry = {.count = 1, .reusable = true}}, SHIFT(58),
  [47] = {.entry = {.count = 1, .reusable = true}}, SHIFT(100),
  [49] = {.entry = {.count = 1, .reusable = true}}, SHIFT(114),
  [51] = {.entry = {.count = 1, .reusable = true}}, SHIFT(134),
  [53] = {.entry = {.count = 1, .reusable = true}}, SHIFT(167),
  [55] = {.entry = {.count = 1, .reusable = true}}, SHIFT(136),
  [57] = {.entry = {.count = 1, .reusable = true}}, SHIFT(115),
  [59] = {.entry = {.count = 1, .reusable = true}}, SHIFT(149),
  [61] = {.entry = {.count = 1, .reusable = true}}, SHIFT(150),
  [63] = {.entry = {.count = 1, .reusable = true}}, SHIFT(153),
  [65] = {.entry = {.count = 1, .reusable = true}}, SHIFT(65),
  [67] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(148),
  [70] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0),
  [72] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(58),
  [75] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(157),
  [78] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(100),
  [81] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(114),
  [84] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(134),
  [87] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(167),
  [90] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(136),
  [93] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(115),
  [96] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(149),
  [99] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(150),
  [102] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(153),
  [105] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_block, 3, 0, 0),
  [107] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0),
  [109] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(117),
  [112] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(143),
  [115] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(60),
  [118] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(157),
  [121] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(164),
  [124] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(166),
  [127] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(158),
  [130] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_block, 4, 0, 0),
  [132] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 1, 0, 0),
  [134] = {.entry = {.count = 1, .reusable = true}}, SHIFT(125),
  [136] = {.entry = {.count = 1, .reusable = true}}, SHIFT(142),
  [138] = {.entry = {.count = 1, .reusable = true}}, SHIFT(141),
  [140] = {.entry = {.count = 1, .reusable = true}}, SHIFT(144),
  [142] = {.entry = {.count = 1, .reusable = true}}, SHIFT(104),
  [144] = {.entry = {.count = 1, .reusable = true}}, SHIFT(161),
  [146] = {.entry = {.count = 1, .reusable = true}}, SHIFT(120),
  [148] = {.entry = {.count = 1, .reusable = true}}, SHIFT(121),
  [150] = {.entry = {.count = 1, .reusable = true}}, SHIFT(74),
  [152] = {.entry = {.count = 1, .reusable = true}}, SHIFT(118),
  [154] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0),
  [156] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(142),
  [159] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(141),
  [162] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(144),
  [165] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(104),
  [168] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(161),
  [171] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(120),
  [174] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(121),
  [177] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(74),
  [180] = {.entry = {.count = 1, .reusable = true}}, SHIFT(57),
  [182] = {.entry = {.count = 1, .reusable = true}}, SHIFT(155),
  [184] = {.entry = {.count = 1, .reusable = true}}, SHIFT(160),
  [186] = {.entry = {.count = 1, .reusable = true}}, SHIFT(137),
  [188] = {.entry = {.count = 1, .reusable = true}}, SHIFT(99),
  [190] = {.entry = {.count = 1, .reusable = true}}, SHIFT(126),
  [192] = {.entry = {.count = 1, .reusable = true}}, SHIFT(64),
  [194] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0),
  [196] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(155),
  [199] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(150),
  [202] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(160),
  [205] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(137),
  [208] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(99),
  [211] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(126),
  [214] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_consensus_mode_value, 1, 0, 0),
  [216] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_verify, 3, 0, 0),
  [218] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 2, 0, 0),
  [220] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 3, 0, 0),
  [222] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_block, 3, 0, 0),
  [224] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 4, 0, 0),
  [226] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_block, 4, 0, 0),
  [228] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_field, 1, 0, 0),
  [230] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_field, 2, 0, 0),
  [232] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_branch_chain_value, 1, 0, 0),
  [234] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_clause, 2, 0, 0),
  [236] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_list, 2, 0, 0),
  [238] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_verify, 2, 0, 0),
  [240] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_agent, 2, 0, 0),
  [242] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_command, 2, 0, 0),
  [244] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 1, 0, 0),
  [246] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_list, 3, 0, 0),
  [248] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_alias_name, 1, 0, 0),
  [250] = {.entry = {.count = 1, .reusable = false}}, SHIFT(68),
  [252] = {.entry = {.count = 1, .reusable = true}}, SHIFT(70),
  [254] = {.entry = {.count = 1, .reusable = false}}, SHIFT(70),
  [256] = {.entry = {.count = 1, .reusable = true}}, SHIFT(7),
  [258] = {.entry = {.count = 1, .reusable = true}}, SHIFT(138),
  [260] = {.entry = {.count = 1, .reusable = true}}, SHIFT(124),
  [262] = {.entry = {.count = 1, .reusable = true}}, SHIFT(151),
  [264] = {.entry = {.count = 1, .reusable = true}}, SHIFT(152),
  [266] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0),
  [268] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(73),
  [271] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(127),
  [274] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(40),
  [277] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(130),
  [280] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(67),
  [283] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(168),
  [286] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_block, 3, 0, 0),
  [288] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_step_list, 2, 0, 0),
  [290] = {.entry = {.count = 1, .reusable = true}}, SHIFT(63),
  [292] = {.entry = {.count = 1, .reusable = true}}, SHIFT(73),
  [294] = {.entry = {.count = 1, .reusable = true}}, SHIFT(127),
  [296] = {.entry = {.count = 1, .reusable = true}}, SHIFT(40),
  [298] = {.entry = {.count = 1, .reusable = true}}, SHIFT(130),
  [300] = {.entry = {.count = 1, .reusable = true}}, SHIFT(67),
  [302] = {.entry = {.count = 1, .reusable = true}}, SHIFT(168),
  [304] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 1, 0, 0),
  [306] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0),
  [308] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(138),
  [311] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(124),
  [314] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(151),
  [317] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(152),
  [320] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_block, 4, 0, 0),
  [322] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_step_list, 3, 0, 0),
  [324] = {.entry = {.count = 1, .reusable = true}}, SHIFT(62),
  [326] = {.entry = {.count = 1, .reusable = true}}, SHIFT(8),
  [328] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_strategy_value, 1, 0, 0),
  [330] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 2, 0, 0),
  [332] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_prompt_declaration, 3, 0, 0),
  [334] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_field, 2, 0, 0),
  [336] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_alias_declaration, 3, 0, 0),
  [338] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 4, 0, 0),
  [340] = {.entry = {.count = 1, .reusable = false}}, SHIFT(39),
  [342] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 5, 0, 0),
  [344] = {.entry = {.count = 1, .reusable = false}}, SHIFT(145),
  [346] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_include_declaration, 2, 0, 0),
  [348] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 4, 0, 0),
  [350] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 5, 0, 0),
  [352] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 5, 0, 0),
  [354] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 4, 0, 0),
  [356] = {.entry = {.count = 1, .reusable = true}}, SHIFT(23),
  [358] = {.entry = {.count = 1, .reusable = true}}, SHIFT(128),
  [360] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_field, 1, 0, 0),
  [362] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_value, 1, 0, 0),
  [364] = {.entry = {.count = 1, .reusable = true}}, SHIFT(80),
  [366] = {.entry = {.count = 1, .reusable = true}}, SHIFT(116),
  [368] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_field, 2, 0, 0),
  [370] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_block, 3, 0, 0),
  [372] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_verify_block_repeat1, 2, 0, 0),
  [374] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_verify_block_repeat1, 2, 0, 0), SHIFT_REPEAT(128),
  [377] = {.entry = {.count = 1, .reusable = true}}, SHIFT(68),
  [379] = {.entry = {.count = 1, .reusable = true}}, SHIFT(76),
  [381] = {.entry = {.count = 1, .reusable = true}}, SHIFT(147),
  [383] = {.entry = {.count = 1, .reusable = true}}, SHIFT(123),
  [385] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_privacy_value, 1, 0, 0),
  [387] = {.entry = {.count = 1, .reusable = true}}, SHIFT(34),
  [389] = {.entry = {.count = 1, .reusable = true}}, SHIFT(43),
  [391] = {.entry = {.count = 1, .reusable = true}}, SHIFT(71),
  [393] = {.entry = {.count = 1, .reusable = true}}, SHIFT(48),
  [395] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_block, 4, 0, 0),
  [397] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_extra_block_repeat1, 2, 0, 0),
  [399] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_extra_block_repeat1, 2, 0, 0), SHIFT_REPEAT(116),
  [402] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0),
  [404] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0), SHIFT_REPEAT(139),
  [407] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0), SHIFT_REPEAT(156),
  [410] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0),
  [412] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(135),
  [415] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(122),
  [418] = {.entry = {.count = 1, .reusable = true}}, SHIFT(25),
  [420] = {.entry = {.count = 1, .reusable = true}}, SHIFT(135),
  [422] = {.entry = {.count = 1, .reusable = true}}, SHIFT(122),
  [424] = {.entry = {.count = 1, .reusable = true}}, SHIFT(28),
  [426] = {.entry = {.count = 1, .reusable = true}}, SHIFT(139),
  [428] = {.entry = {.count = 1, .reusable = true}}, SHIFT(156),
  [430] = {.entry = {.count = 1, .reusable = true}}, SHIFT(27),
  [432] = {.entry = {.count = 1, .reusable = true}}, SHIFT(26),
  [434] = {.entry = {.count = 1, .reusable = true}}, SHIFT(4),
  [436] = {.entry = {.count = 1, .reusable = true}}, SHIFT(95),
  [438] = {.entry = {.count = 1, .reusable = false}}, SHIFT(163),
  [440] = {.entry = {.count = 1, .reusable = true}}, SHIFT(44),
  [442] = {.entry = {.count = 1, .reusable = false}}, SHIFT(93),
  [444] = {.entry = {.count = 1, .reusable = true}}, SHIFT(159),
  [446] = {.entry = {.count = 1, .reusable = true}}, SHIFT(140),
  [448] = {.entry = {.count = 1, .reusable = true}}, SHIFT(110),
  [450] = {.entry = {.count = 1, .reusable = true}}, SHIFT(3),
  [452] = {.entry = {.count = 1, .reusable = true}}, SHIFT(88),
  [454] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0), SHIFT_REPEAT(163),
  [457] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0),
  [459] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0), SHIFT_REPEAT(92),
  [462] = {.entry = {.count = 1, .reusable = true}}, SHIFT(49),
  [464] = {.entry = {.count = 1, .reusable = false}}, SHIFT(92),
  [466] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_field, 2, 0, 0),
  [468] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0),
  [470] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0), SHIFT_REPEAT(95),
  [473] = {.entry = {.count = 1, .reusable = true}}, SHIFT(133),
  [475] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_reviewer_entry_repeat1, 2, 0, 0), SHIFT_REPEAT(159),
  [478] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_reviewer_entry_repeat1, 2, 0, 0),
  [480] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_reviewer_entry_repeat1, 2, 0, 0), SHIFT_REPEAT(110),
  [483] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_field, 2, 0, 0),
  [485] = {.entry = {.count = 1, .reusable = false}}, SHIFT(52),
  [487] = {.entry = {.count = 1, .reusable = true}}, SHIFT(24),
  [489] = {.entry = {.count = 1, .reusable = true}}, SHIFT(90),
  [491] = {.entry = {.count = 1, .reusable = true}}, SHIFT(33),
  [493] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_vars_block_repeat1, 2, 0, 0),
  [495] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_vars_block_repeat1, 2, 0, 0), SHIFT_REPEAT(113),
  [498] = {.entry = {.count = 1, .reusable = true}}, SHIFT(38),
  [500] = {.entry = {.count = 1, .reusable = true}}, SHIFT(22),
  [502] = {.entry = {.count = 1, .reusable = true}}, SHIFT(14),
  [504] = {.entry = {.count = 1, .reusable = true}}, SHIFT(113),
  [506] = {.entry = {.count = 1, .reusable = true}}, SHIFT(12),
  [508] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_reviewer_list_repeat1, 2, 0, 0), SHIFT_REPEAT(90),
  [511] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_reviewer_list_repeat1, 2, 0, 0),
  [513] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_pair, 2, 0, 0),
  [515] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_field, 2, 0, 0),
  [517] = {.entry = {.count = 1, .reusable = true}}, SHIFT(131),
  [519] = {.entry = {.count = 1, .reusable = true}}, SHIFT(5),
  [521] = {.entry = {.count = 1, .reusable = true}}, SHIFT(119),
  [523] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0),
  [525] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0), SHIFT_REPEAT(112),
  [528] = {.entry = {.count = 1, .reusable = true}}, SHIFT(132),
  [530] = {.entry = {.count = 1, .reusable = true}}, SHIFT(2),
  [532] = {.entry = {.count = 1, .reusable = true}}, SHIFT(108),
  [534] = {.entry = {.count = 1, .reusable = true}}, SHIFT(61),
  [536] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_loop_block, 3, 0, 0),
  [538] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_block, 3, 0, 0),
  [540] = {.entry = {.count = 1, .reusable = true}}, SHIFT(6),
  [542] = {.entry = {.count = 1, .reusable = true}}, SHIFT(112),
  [544] = {.entry = {.count = 1, .reusable = true}}, SHIFT(31),
  [546] = {.entry = {.count = 1, .reusable = true}}, SHIFT(36),
  [548] = {.entry = {.count = 1, .reusable = true}}, SHIFT(55),
  [550] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_loop_block, 4, 0, 0),
  [552] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_block, 4, 0, 0),
  [554] = {.entry = {.count = 1, .reusable = true}}, SHIFT(54),
  [556] = {.entry = {.count = 1, .reusable = true}}, SHIFT(75),
  [558] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_field, 2, 0, 0),
  [560] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_pair, 2, 0, 0),
  [562] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_entry, 3, 0, 0),
  [564] = {.entry = {.count = 1, .reusable = true}}, SHIFT(111),
  [566] = {.entry = {.count = 1, .reusable = true}}, SHIFT(91),
  [568] = {.entry = {.count = 1, .reusable = true}}, SHIFT(89),
  [570] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_entry, 2, 0, 0),
  [572] = {.entry = {.count = 1, .reusable = true}}, SHIFT(101),
  [574] = {.entry = {.count = 1, .reusable = true}}, SHIFT(146),
  [576] = {.entry = {.count = 1, .reusable = true}}, SHIFT(30),
  [578] = {.entry = {.count = 1, .reusable = true}}, SHIFT(50),
  [580] = {.entry = {.count = 1, .reusable = true}}, SHIFT(35),
  [582] = {.entry = {.count = 1, .reusable = true}}, SHIFT(84),
  [584] = {.entry = {.count = 1, .reusable = true}}, SHIFT(41),
  [586] = {.entry = {.count = 1, .reusable = true}}, SHIFT(87),
  [588] = {.entry = {.count = 1, .reusable = true}}, SHIFT(56),
  [590] = {.entry = {.count = 1, .reusable = true}}, SHIFT(53),
  [592] = {.entry = {.count = 1, .reusable = true}}, SHIFT(98),
  [594] = {.entry = {.count = 1, .reusable = true}}, SHIFT(106),
  [596] = {.entry = {.count = 1, .reusable = true}}, SHIFT(169),
  [598] = {.entry = {.count = 1, .reusable = true}}, SHIFT(77),
  [600] = {.entry = {.count = 1, .reusable = true}},  ACCEPT_INPUT(),
  [602] = {.entry = {.count = 1, .reusable = true}}, SHIFT(17),
  [604] = {.entry = {.count = 1, .reusable = true}}, SHIFT(129),
  [606] = {.entry = {.count = 1, .reusable = true}}, SHIFT(10),
  [608] = {.entry = {.count = 1, .reusable = true}}, SHIFT(165),
  [610] = {.entry = {.count = 1, .reusable = true}}, SHIFT(78),
  [612] = {.entry = {.count = 1, .reusable = true}}, SHIFT(19),
};

#ifdef __cplusplus
extern "C" {
#endif
#ifdef TREE_SITTER_HIDE_SYMBOLS
#define TS_PUBLIC
#elif defined(_WIN32)
#define TS_PUBLIC __declspec(dllexport)
#else
#define TS_PUBLIC __attribute__((visibility("default")))
#endif

TS_PUBLIC const TSLanguage *tree_sitter_gaviero(void) {
  static const TSLanguage language = {
    .version = LANGUAGE_VERSION,
    .symbol_count = SYMBOL_COUNT,
    .alias_count = ALIAS_COUNT,
    .token_count = TOKEN_COUNT,
    .external_token_count = EXTERNAL_TOKEN_COUNT,
    .state_count = STATE_COUNT,
    .large_state_count = LARGE_STATE_COUNT,
    .production_id_count = PRODUCTION_ID_COUNT,
    .field_count = FIELD_COUNT,
    .max_alias_sequence_length = MAX_ALIAS_SEQUENCE_LENGTH,
    .parse_table = &ts_parse_table[0][0],
    .small_parse_table = ts_small_parse_table,
    .small_parse_table_map = ts_small_parse_table_map,
    .parse_actions = ts_parse_actions,
    .symbol_names = ts_symbol_names,
    .symbol_metadata = ts_symbol_metadata,
    .public_symbol_map = ts_symbol_map,
    .alias_map = ts_non_terminal_alias_map,
    .alias_sequences = &ts_alias_sequences[0][0],
    .lex_modes = ts_lex_modes,
    .lex_fn = ts_lex,
    .primary_state_ids = ts_primary_state_ids,
  };
  return &language;
}
#ifdef __cplusplus
}
#endif
