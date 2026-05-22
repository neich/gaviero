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
#define STATE_COUNT 173
#define LARGE_STATE_COUNT 2
#define SYMBOL_COUNT 150
#define ALIAS_COUNT 0
#define TOKEN_COUNT 88
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
  anon_sym_param = 76,
  anon_sym_public = 77,
  anon_sym_local_only = 78,
  anon_sym_single_pass = 79,
  anon_sym_refine = 80,
  anon_sym_true = 81,
  anon_sym_false = 82,
  sym_string = 83,
  sym_raw_string = 84,
  sym_float = 85,
  sym_integer = 86,
  sym_identifier = 87,
  sym_source_file = 88,
  sym__definition = 89,
  sym_include_declaration = 90,
  sym_client_declaration = 91,
  sym_client_field = 92,
  sym__effort_value = 93,
  sym_extra_block = 94,
  sym_extra_pair = 95,
  sym_vars_block = 96,
  sym_vars_pair = 97,
  sym_tier_alias_declaration = 98,
  sym_tier_alias_name = 99,
  sym_prompt_declaration = 100,
  sym_agent_declaration = 101,
  sym_agent_field = 102,
  sym_scope_block = 103,
  sym_scope_field = 104,
  sym_memory_block = 105,
  sym_memory_field = 106,
  sym_verify_block = 107,
  sym_verify_field = 108,
  sym_context_block = 109,
  sym_context_field = 110,
  sym_loop_block = 111,
  sym_loop_field = 112,
  sym_reviewer_list = 113,
  sym_reviewer_entry = 114,
  sym_reviewer_field = 115,
  sym_consensus_mode_value = 116,
  sym_branch_chain_value = 117,
  sym_until_clause = 118,
  sym__until_condition = 119,
  sym_until_verify = 120,
  sym_until_agent = 121,
  sym_until_command = 122,
  sym_workflow_declaration = 123,
  sym_workflow_field = 124,
  sym_param_declaration = 125,
  sym_step_list = 126,
  sym_string_list = 127,
  sym_identifier_list = 128,
  sym_tier_value = 129,
  sym_privacy_value = 130,
  sym_strategy_value = 131,
  sym_boolean = 132,
  sym__string_value = 133,
  aux_sym_source_file_repeat1 = 134,
  aux_sym_client_declaration_repeat1 = 135,
  aux_sym_extra_block_repeat1 = 136,
  aux_sym_vars_block_repeat1 = 137,
  aux_sym_agent_declaration_repeat1 = 138,
  aux_sym_scope_block_repeat1 = 139,
  aux_sym_memory_block_repeat1 = 140,
  aux_sym_verify_block_repeat1 = 141,
  aux_sym_context_block_repeat1 = 142,
  aux_sym_loop_block_repeat1 = 143,
  aux_sym_reviewer_list_repeat1 = 144,
  aux_sym_reviewer_entry_repeat1 = 145,
  aux_sym_workflow_declaration_repeat1 = 146,
  aux_sym_step_list_repeat1 = 147,
  aux_sym_string_list_repeat1 = 148,
  aux_sym_identifier_list_repeat1 = 149,
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
  [anon_sym_param] = "param",
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
  [sym_param_declaration] = "param_declaration",
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
  [anon_sym_param] = anon_sym_param,
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
  [sym_param_declaration] = sym_param_declaration,
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
  [anon_sym_param] = {
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
  [sym_param_declaration] = {
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
  [145] = 145,
  [146] = 146,
  [147] = 147,
  [148] = 40,
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
  [170] = 170,
  [171] = 171,
  [172] = 172,
};

static bool ts_lex(TSLexer *lexer, TSStateId state) {
  START_LEXER();
  eof = lexer->eof(lexer);
  switch (state) {
    case 0:
      if (eof) ADVANCE(525);
      ADVANCE_MAP(
        '"', 3,
        '#', 4,
        '/', 13,
        '[', 594,
        ']', 595,
        'a', 208,
        'b', 390,
        'c', 38,
        'd', 129,
        'e', 197,
        'f', 47,
        'i', 115,
        'j', 494,
        'l', 334,
        'm', 40,
        'n', 336,
        'o', 508,
        'p', 45,
        'r', 130,
        's', 93,
        't', 131,
        'u', 306,
        'v', 51,
        'w', 340,
        '{', 529,
        '}', 530,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(0);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(624);
      END_STATE();
    case 1:
      if (lookahead == '"') ADVANCE(3);
      if (lookahead == '#') ADVANCE(4);
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == '[') ADVANCE(594);
      if (lookahead == ']') ADVANCE(595);
      if (lookahead == '}') ADVANCE(530);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(1);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 2:
      if (lookahead == '"') ADVANCE(3);
      if (lookahead == '#') ADVANCE(4);
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'c') ADVANCE(649);
      if (lookahead == 'e') ADVANCE(687);
      if (lookahead == 'm') ADVANCE(637);
      if (lookahead == 'r') ADVANCE(638);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(2);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 3:
      if (lookahead == '"') ADVANCE(621);
      if (lookahead == '\\') ADVANCE(523);
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
      if (lookahead == '#') ADVANCE(622);
      if (lookahead != 0) ADVANCE(5);
      END_STATE();
    case 7:
      if (lookahead == '.') ADVANCE(522);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(7);
      END_STATE();
    case 8:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == ']') ADVANCE(595);
      if (lookahead == 'l') ADVANCE(672);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(8);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 9:
      ADVANCE_MAP(
        '/', 13,
        'a', 213,
        'b', 390,
        'c', 254,
        'd', 147,
        'e', 411,
        'i', 290,
        'j', 494,
        'm', 41,
        'o', 508,
        'p', 79,
        'r', 148,
        's', 94,
        't', 185,
        'u', 306,
        'v', 51,
        '}', 530,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(9);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(625);
      END_STATE();
    case 10:
      ADVANCE_MAP(
        '/', 13,
        'a', 213,
        'b', 390,
        'c', 352,
        'e', 411,
        'i', 463,
        'j', 494,
        'm', 41,
        'p', 78,
        'r', 155,
        's', 462,
        't', 191,
        'u', 306,
        'v', 156,
        '}', 530,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(10);
      END_STATE();
    case 11:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'c') ADVANCE(275);
      if (lookahead == 'i') ADVANCE(300);
      if (lookahead == 't') ADVANCE(193);
      if (lookahead == '}') ADVANCE(530);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(11);
      END_STATE();
    case 12:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'r') ADVANCE(641);
      if (lookahead == 's') ADVANCE(655);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(12);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 13:
      if (lookahead == '/') ADVANCE(527);
      END_STATE();
    case 14:
      if (lookahead == '_') ADVANCE(243);
      END_STATE();
    case 15:
      if (lookahead == '_') ADVANCE(270);
      END_STATE();
    case 16:
      if (lookahead == '_') ADVANCE(100);
      END_STATE();
    case 17:
      if (lookahead == '_') ADVANCE(250);
      END_STATE();
    case 18:
      if (lookahead == '_') ADVANCE(241);
      END_STATE();
    case 19:
      if (lookahead == '_') ADVANCE(369);
      END_STATE();
    case 20:
      if (lookahead == '_') ADVANCE(428);
      END_STATE();
    case 21:
      if (lookahead == '_') ADVANCE(203);
      END_STATE();
    case 22:
      if (lookahead == '_') ADVANCE(203);
      if (lookahead == 's') ADVANCE(37);
      END_STATE();
    case 23:
      if (lookahead == '_') ADVANCE(289);
      END_STATE();
    case 24:
      if (lookahead == '_') ADVANCE(434);
      END_STATE();
    case 25:
      if (lookahead == '_') ADVANCE(341);
      END_STATE();
    case 26:
      if (lookahead == '_') ADVANCE(339);
      END_STATE();
    case 27:
      if (lookahead == '_') ADVANCE(108);
      END_STATE();
    case 28:
      if (lookahead == '_') ADVANCE(372);
      END_STATE();
    case 29:
      if (lookahead == '_') ADVANCE(54);
      END_STATE();
    case 30:
      if (lookahead == '_') ADVANCE(432);
      END_STATE();
    case 31:
      if (lookahead == '_') ADVANCE(427);
      END_STATE();
    case 32:
      if (lookahead == '_') ADVANCE(358);
      END_STATE();
    case 33:
      if (lookahead == '_') ADVANCE(477);
      END_STATE();
    case 34:
      if (lookahead == '_') ADVANCE(350);
      END_STATE();
    case 35:
      if (lookahead == '_') ADVANCE(346);
      END_STATE();
    case 36:
      if (lookahead == '_') ADVANCE(483);
      END_STATE();
    case 37:
      if (lookahead == '_') ADVANCE(204);
      END_STATE();
    case 38:
      if (lookahead == 'a') ADVANCE(260);
      if (lookahead == 'h') ADVANCE(153);
      if (lookahead == 'l') ADVANCE(218);
      if (lookahead == 'o') ADVANCE(284);
      END_STATE();
    case 39:
      if (lookahead == 'a') ADVANCE(260);
      if (lookahead == 'l') ADVANCE(249);
      if (lookahead == 'o') ADVANCE(333);
      END_STATE();
    case 40:
      if (lookahead == 'a') ADVANCE(510);
      if (lookahead == 'e') ADVANCE(91);
      if (lookahead == 'o') ADVANCE(122);
      END_STATE();
    case 41:
      if (lookahead == 'a') ADVANCE(510);
      if (lookahead == 'e') ADVANCE(296);
      END_STATE();
    case 42:
      if (lookahead == 'a') ADVANCE(121);
      if (lookahead == 'f') ADVANCE(239);
      if (lookahead == 'v') ADVANCE(231);
      END_STATE();
    case 43:
      if (lookahead == 'a') ADVANCE(90);
      if (lookahead == 'e') ADVANCE(366);
      if (lookahead == 'r') ADVANCE(74);
      END_STATE();
    case 44:
      if (lookahead == 'a') ADVANCE(536);
      END_STATE();
    case 45:
      if (lookahead == 'a') ADVANCE(384);
      if (lookahead == 'r') ADVANCE(219);
      if (lookahead == 'u') ADVANCE(88);
      END_STATE();
    case 46:
      if (lookahead == 'a') ADVANCE(384);
      if (lookahead == 'r') ADVANCE(335);
      END_STATE();
    case 47:
      if (lookahead == 'a') ADVANCE(255);
      END_STATE();
    case 48:
      if (lookahead == 'a') ADVANCE(89);
      if (lookahead == 'e') ADVANCE(366);
      if (lookahead == 'r') ADVANCE(76);
      END_STATE();
    case 49:
      if (lookahead == 'a') ADVANCE(285);
      END_STATE();
    case 50:
      if (lookahead == 'a') ADVANCE(285);
      if (lookahead == 't') ADVANCE(242);
      END_STATE();
    case 51:
      if (lookahead == 'a') ADVANCE(391);
      if (lookahead == 'e') ADVANCE(397);
      END_STATE();
    case 52:
      if (lookahead == 'a') ADVANCE(496);
      END_STATE();
    case 53:
      if (lookahead == 'a') ADVANCE(311);
      END_STATE();
    case 54:
      if (lookahead == 'a') ADVANCE(205);
      END_STATE();
    case 55:
      if (lookahead == 'a') ADVANCE(101);
      END_STATE();
    case 56:
      if (lookahead == 'a') ADVANCE(101);
      if (lookahead == 'o') ADVANCE(399);
      END_STATE();
    case 57:
      if (lookahead == 'a') ADVANCE(364);
      END_STATE();
    case 58:
      if (lookahead == 'a') ADVANCE(265);
      END_STATE();
    case 59:
      if (lookahead == 'a') ADVANCE(99);
      END_STATE();
    case 60:
      if (lookahead == 'a') ADVANCE(322);
      END_STATE();
    case 61:
      if (lookahead == 'a') ADVANCE(262);
      END_STATE();
    case 62:
      if (lookahead == 'a') ADVANCE(128);
      if (lookahead == 'v') ADVANCE(231);
      END_STATE();
    case 63:
      if (lookahead == 'a') ADVANCE(120);
      END_STATE();
    case 64:
      if (lookahead == 'a') ADVANCE(268);
      END_STATE();
    case 65:
      if (lookahead == 'a') ADVANCE(405);
      END_STATE();
    case 66:
      if (lookahead == 'a') ADVANCE(258);
      END_STATE();
    case 67:
      if (lookahead == 'a') ADVANCE(488);
      END_STATE();
    case 68:
      if (lookahead == 'a') ADVANCE(232);
      END_STATE();
    case 69:
      if (lookahead == 'a') ADVANCE(512);
      if (lookahead == 'e') ADVANCE(296);
      END_STATE();
    case 70:
      if (lookahead == 'a') ADVANCE(319);
      END_STATE();
    case 71:
      if (lookahead == 'a') ADVANCE(314);
      END_STATE();
    case 72:
      if (lookahead == 'a') ADVANCE(438);
      END_STATE();
    case 73:
      if (lookahead == 'a') ADVANCE(410);
      END_STATE();
    case 74:
      if (lookahead == 'a') ADVANCE(478);
      if (lookahead == 'i') ADVANCE(102);
      END_STATE();
    case 75:
      if (lookahead == 'a') ADVANCE(478);
      if (lookahead == 'i') ADVANCE(104);
      END_STATE();
    case 76:
      if (lookahead == 'a') ADVANCE(478);
      if (lookahead == 'i') ADVANCE(110);
      END_STATE();
    case 77:
      if (lookahead == 'a') ADVANCE(103);
      if (lookahead == 'o') ADVANCE(399);
      END_STATE();
    case 78:
      if (lookahead == 'a') ADVANCE(404);
      END_STATE();
    case 79:
      if (lookahead == 'a') ADVANCE(404);
      if (lookahead == 'r') ADVANCE(335);
      END_STATE();
    case 80:
      if (lookahead == 'a') ADVANCE(481);
      END_STATE();
    case 81:
      if (lookahead == 'a') ADVANCE(271);
      if (lookahead == 'e') ADVANCE(366);
      if (lookahead == 'r') ADVANCE(75);
      END_STATE();
    case 82:
      if (lookahead == 'a') ADVANCE(482);
      END_STATE();
    case 83:
      if (lookahead == 'a') ADVANCE(484);
      END_STATE();
    case 84:
      if (lookahead == 'a') ADVANCE(485);
      END_STATE();
    case 85:
      if (lookahead == 'a') ADVANCE(280);
      END_STATE();
    case 86:
      if (lookahead == 'a') ADVANCE(112);
      END_STATE();
    case 87:
      if (lookahead == 'a') ADVANCE(493);
      END_STATE();
    case 88:
      if (lookahead == 'b') ADVANCE(267);
      END_STATE();
    case 89:
      if (lookahead == 'b') ADVANCE(238);
      END_STATE();
    case 90:
      if (lookahead == 'b') ADVANCE(238);
      if (lookahead == 'c') ADVANCE(253);
      if (lookahead == 'l') ADVANCE(192);
      END_STATE();
    case 91:
      if (lookahead == 'c') ADVANCE(217);
      if (lookahead == 'm') ADVANCE(347);
      END_STATE();
    case 92:
      if (lookahead == 'c') ADVANCE(613);
      END_STATE();
    case 93:
      if (lookahead == 'c') ADVANCE(338);
      if (lookahead == 'i') ADVANCE(307);
      if (lookahead == 't') ADVANCE(43);
      END_STATE();
    case 94:
      if (lookahead == 'c') ADVANCE(338);
      if (lookahead == 't') ADVANCE(48);
      END_STATE();
    case 95:
      if (lookahead == 'c') ADVANCE(338);
      if (lookahead == 't') ADVANCE(81);
      END_STATE();
    case 96:
      if (lookahead == 'c') ADVANCE(215);
      END_STATE();
    case 97:
      if (lookahead == 'c') ADVANCE(499);
      END_STATE();
    case 98:
      if (lookahead == 'c') ADVANCE(276);
      END_STATE();
    case 99:
      if (lookahead == 'c') ADVANCE(516);
      END_STATE();
    case 100:
      if (lookahead == 'c') ADVANCE(360);
      if (lookahead == 'n') ADVANCE(418);
      END_STATE();
    case 101:
      if (lookahead == 'c') ADVANCE(467);
      END_STATE();
    case 102:
      if (lookahead == 'c') ADVANCE(449);
      END_STATE();
    case 103:
      if (lookahead == 'c') ADVANCE(486);
      END_STATE();
    case 104:
      if (lookahead == 'c') ADVANCE(460);
      END_STATE();
    case 105:
      if (lookahead == 'c') ADVANCE(142);
      END_STATE();
    case 106:
      if (lookahead == 'c') ADVANCE(183);
      END_STATE();
    case 107:
      if (lookahead == 'c') ADVANCE(58);
      END_STATE();
    case 108:
      if (lookahead == 'c') ADVANCE(216);
      END_STATE();
    case 109:
      if (lookahead == 'c') ADVANCE(402);
      END_STATE();
    case 110:
      if (lookahead == 'c') ADVANCE(472);
      END_STATE();
    case 111:
      if (lookahead == 'c') ADVANCE(61);
      if (lookahead == 'o') ADVANCE(363);
      END_STATE();
    case 112:
      if (lookahead == 'c') ADVANCE(476);
      END_STATE();
    case 113:
      if (lookahead == 'c') ADVANCE(66);
      END_STATE();
    case 114:
      if (lookahead == 'c') ADVANCE(355);
      END_STATE();
    case 115:
      if (lookahead == 'd') ADVANCE(596);
      if (lookahead == 'm') ADVANCE(362);
      if (lookahead == 'n') ADVANCE(98);
      if (lookahead == 't') ADVANCE(163);
      END_STATE();
    case 116:
      if (lookahead == 'd') ADVANCE(210);
      END_STATE();
    case 117:
      if (lookahead == 'd') ADVANCE(559);
      END_STATE();
    case 118:
      if (lookahead == 'd') ADVANCE(604);
      END_STATE();
    case 119:
      if (lookahead == 'd') ADVANCE(601);
      END_STATE();
    case 120:
      if (lookahead == 'd') ADVANCE(15);
      END_STATE();
    case 121:
      if (lookahead == 'd') ADVANCE(15);
      if (lookahead == 's') ADVANCE(357);
      END_STATE();
    case 122:
      if (lookahead == 'd') ADVANCE(169);
      END_STATE();
    case 123:
      if (lookahead == 'd') ADVANCE(443);
      END_STATE();
    case 124:
      if (lookahead == 'd') ADVANCE(225);
      END_STATE();
    case 125:
      if (lookahead == 'd') ADVANCE(139);
      END_STATE();
    case 126:
      if (lookahead == 'd') ADVANCE(145);
      END_STATE();
    case 127:
      if (lookahead == 'd') ADVANCE(211);
      END_STATE();
    case 128:
      if (lookahead == 'd') ADVANCE(35);
      END_STATE();
    case 129:
      if (lookahead == 'e') ADVANCE(200);
      END_STATE();
    case 130:
      if (lookahead == 'e') ADVANCE(42);
      END_STATE();
    case 131:
      if (lookahead == 'e') ADVANCE(286);
      if (lookahead == 'i') ADVANCE(168);
      if (lookahead == 'o') ADVANCE(343);
      if (lookahead == 'r') ADVANCE(495);
      END_STATE();
    case 132:
      if (lookahead == 'e') ADVANCE(602);
      END_STATE();
    case 133:
      if (lookahead == 'e') ADVANCE(619);
      END_STATE();
    case 134:
      if (lookahead == 'e') ADVANCE(620);
      END_STATE();
    case 135:
      if (lookahead == 'e') ADVANCE(558);
      END_STATE();
    case 136:
      if (lookahead == 'e') ADVANCE(617);
      END_STATE();
    case 137:
      if (lookahead == 'e') ADVANCE(571);
      END_STATE();
    case 138:
      if (lookahead == 'e') ADVANCE(600);
      END_STATE();
    case 139:
      if (lookahead == 'e') ADVANCE(526);
      END_STATE();
    case 140:
      if (lookahead == 'e') ADVANCE(557);
      END_STATE();
    case 141:
      if (lookahead == 'e') ADVANCE(540);
      END_STATE();
    case 142:
      if (lookahead == 'e') ADVANCE(565);
      END_STATE();
    case 143:
      if (lookahead == 'e') ADVANCE(561);
      END_STATE();
    case 144:
      if (lookahead == 'e') ADVANCE(592);
      END_STATE();
    case 145:
      if (lookahead == 'e') ADVANCE(587);
      END_STATE();
    case 146:
      if (lookahead == 'e') ADVANCE(586);
      END_STATE();
    case 147:
      if (lookahead == 'e') ADVANCE(376);
      END_STATE();
    case 148:
      if (lookahead == 'e') ADVANCE(62);
      END_STATE();
    case 149:
      if (lookahead == 'e') ADVANCE(556);
      END_STATE();
    case 150:
      if (lookahead == 'e') ADVANCE(509);
      END_STATE();
    case 151:
      if (lookahead == 'e') ADVANCE(511);
      END_STATE();
    case 152:
      if (lookahead == 'e') ADVANCE(361);
      END_STATE();
    case 153:
      if (lookahead == 'e') ADVANCE(57);
      END_STATE();
    case 154:
      if (lookahead == 'e') ADVANCE(209);
      END_STATE();
    case 155:
      if (lookahead == 'e') ADVANCE(504);
      END_STATE();
    case 156:
      if (lookahead == 'e') ADVANCE(397);
      END_STATE();
    case 157:
      if (lookahead == 'e') ADVANCE(97);
      if (lookahead == 'p') ADVANCE(162);
      if (lookahead == 't') ADVANCE(395);
      END_STATE();
    case 158:
      if (lookahead == 'e') ADVANCE(117);
      END_STATE();
    case 159:
      if (lookahead == 'e') ADVANCE(33);
      END_STATE();
    case 160:
      if (lookahead == 'e') ADVANCE(312);
      END_STATE();
    case 161:
      if (lookahead == 'e') ADVANCE(312);
      if (lookahead == 't') ADVANCE(214);
      END_STATE();
    case 162:
      if (lookahead == 'e') ADVANCE(313);
      if (lookahead == 'l') ADVANCE(349);
      END_STATE();
    case 163:
      if (lookahead == 'e') ADVANCE(392);
      END_STATE();
    case 164:
      if (lookahead == 'e') ADVANCE(316);
      END_STATE();
    case 165:
      if (lookahead == 'e') ADVANCE(206);
      END_STATE();
    case 166:
      if (lookahead == 'e') ADVANCE(119);
      END_STATE();
    case 167:
      if (lookahead == 'e') ADVANCE(16);
      END_STATE();
    case 168:
      if (lookahead == 'e') ADVANCE(386);
      END_STATE();
    case 169:
      if (lookahead == 'e') ADVANCE(256);
      END_STATE();
    case 170:
      if (lookahead == 'e') ADVANCE(342);
      END_STATE();
    case 171:
      if (lookahead == 'e') ADVANCE(28);
      END_STATE();
    case 172:
      if (lookahead == 'e') ADVANCE(398);
      END_STATE();
    case 173:
      if (lookahead == 'e') ADVANCE(437);
      END_STATE();
    case 174:
      if (lookahead == 'e') ADVANCE(29);
      END_STATE();
    case 175:
      if (lookahead == 'e') ADVANCE(403);
      END_STATE();
    case 176:
      if (lookahead == 'e') ADVANCE(487);
      END_STATE();
    case 177:
      if (lookahead == 'e') ADVANCE(63);
      END_STATE();
    case 178:
      if (lookahead == 'e') ADVANCE(420);
      END_STATE();
    case 179:
      if (lookahead == 'e') ADVANCE(400);
      END_STATE();
    case 180:
      if (lookahead == 'e') ADVANCE(259);
      END_STATE();
    case 181:
      if (lookahead == 'e') ADVANCE(18);
      END_STATE();
    case 182:
      if (lookahead == 'e') ADVANCE(389);
      END_STATE();
    case 183:
      if (lookahead == 'e') ADVANCE(424);
      END_STATE();
    case 184:
      if (lookahead == 'e') ADVANCE(308);
      END_STATE();
    case 185:
      if (lookahead == 'e') ADVANCE(287);
      if (lookahead == 'i') ADVANCE(168);
      if (lookahead == 'o') ADVANCE(343);
      END_STATE();
    case 186:
      if (lookahead == 'e') ADVANCE(310);
      END_STATE();
    case 187:
      if (lookahead == 'e') ADVANCE(310);
      if (lookahead == 'p') ADVANCE(365);
      END_STATE();
    case 188:
      if (lookahead == 'e') ADVANCE(299);
      if (lookahead == 'i') ADVANCE(168);
      if (lookahead == 'o') ADVANCE(343);
      END_STATE();
    case 189:
      if (lookahead == 'e') ADVANCE(442);
      END_STATE();
    case 190:
      if (lookahead == 'e') ADVANCE(401);
      END_STATE();
    case 191:
      if (lookahead == 'e') ADVANCE(301);
      END_STATE();
    case 192:
      if (lookahead == 'e') ADVANCE(328);
      END_STATE();
    case 193:
      if (lookahead == 'e') ADVANCE(440);
      END_STATE();
    case 194:
      if (lookahead == 'e') ADVANCE(326);
      END_STATE();
    case 195:
      if (lookahead == 'e') ADVANCE(331);
      END_STATE();
    case 196:
      if (lookahead == 'e') ADVANCE(297);
      END_STATE();
    case 197:
      if (lookahead == 'f') ADVANCE(201);
      if (lookahead == 's') ADVANCE(107);
      if (lookahead == 'x') ADVANCE(157);
      END_STATE();
    case 198:
      if (lookahead == 'f') ADVANCE(578);
      END_STATE();
    case 199:
      if (lookahead == 'f') ADVANCE(515);
      END_STATE();
    case 200:
      if (lookahead == 'f') ADVANCE(52);
      if (lookahead == 'p') ADVANCE(161);
      if (lookahead == 's') ADVANCE(109);
      END_STATE();
    case 201:
      if (lookahead == 'f') ADVANCE(345);
      END_STATE();
    case 202:
      if (lookahead == 'f') ADVANCE(263);
      END_STATE();
    case 203:
      if (lookahead == 'f') ADVANCE(230);
      END_STATE();
    case 204:
      if (lookahead == 'f') ADVANCE(353);
      END_STATE();
    case 205:
      if (lookahead == 'f') ADVANCE(489);
      END_STATE();
    case 206:
      if (lookahead == 'f') ADVANCE(245);
      END_STATE();
    case 207:
      if (lookahead == 'g') ADVANCE(544);
      END_STATE();
    case 208:
      if (lookahead == 'g') ADVANCE(184);
      if (lookahead == 't') ADVANCE(465);
      END_STATE();
    case 209:
      if (lookahead == 'g') ADVANCE(517);
      END_STATE();
    case 210:
      if (lookahead == 'g') ADVANCE(159);
      END_STATE();
    case 211:
      if (lookahead == 'g') ADVANCE(144);
      END_STATE();
    case 212:
      if (lookahead == 'g') ADVANCE(273);
      END_STATE();
    case 213:
      if (lookahead == 'g') ADVANCE(195);
      if (lookahead == 't') ADVANCE(465);
      END_STATE();
    case 214:
      if (lookahead == 'h') ADVANCE(580);
      END_STATE();
    case 215:
      if (lookahead == 'h') ADVANCE(27);
      END_STATE();
    case 216:
      if (lookahead == 'h') ADVANCE(68);
      END_STATE();
    case 217:
      if (lookahead == 'h') ADVANCE(60);
      END_STATE();
    case 218:
      if (lookahead == 'i') ADVANCE(187);
      END_STATE();
    case 219:
      if (lookahead == 'i') ADVANCE(505);
      if (lookahead == 'o') ADVANCE(288);
      END_STATE();
    case 220:
      if (lookahead == 'i') ADVANCE(199);
      END_STATE();
    case 221:
      if (lookahead == 'i') ADVANCE(506);
      END_STATE();
    case 222:
      if (lookahead == 'i') ADVANCE(293);
      END_STATE();
    case 223:
      if (lookahead == 'i') ADVANCE(294);
      END_STATE();
    case 224:
      if (lookahead == 'i') ADVANCE(92);
      END_STATE();
    case 225:
      if (lookahead == 'i') ADVANCE(318);
      END_STATE();
    case 226:
      if (lookahead == 'i') ADVANCE(380);
      END_STATE();
    case 227:
      if (lookahead == 'i') ADVANCE(257);
      END_STATE();
    case 228:
      if (lookahead == 'i') ADVANCE(309);
      END_STATE();
    case 229:
      if (lookahead == 'i') ADVANCE(368);
      END_STATE();
    case 230:
      if (lookahead == 'i') ADVANCE(408);
      END_STATE();
    case 231:
      if (lookahead == 'i') ADVANCE(150);
      END_STATE();
    case 232:
      if (lookahead == 'i') ADVANCE(305);
      END_STATE();
    case 233:
      if (lookahead == 'i') ADVANCE(466);
      END_STATE();
    case 234:
      if (lookahead == 'i') ADVANCE(453);
      END_STATE();
    case 235:
      if (lookahead == 'i') ADVANCE(456);
      END_STATE();
    case 236:
      if (lookahead == 'i') ADVANCE(178);
      END_STATE();
    case 237:
      if (lookahead == 'i') ADVANCE(474);
      END_STATE();
    case 238:
      if (lookahead == 'i') ADVANCE(272);
      END_STATE();
    case 239:
      if (lookahead == 'i') ADVANCE(324);
      END_STATE();
    case 240:
      if (lookahead == 'i') ADVANCE(274);
      END_STATE();
    case 241:
      if (lookahead == 'i') ADVANCE(327);
      if (lookahead == 'r') ADVANCE(165);
      END_STATE();
    case 242:
      if (lookahead == 'i') ADVANCE(64);
      END_STATE();
    case 243:
      if (lookahead == 'i') ADVANCE(480);
      if (lookahead == 'p') ADVANCE(73);
      if (lookahead == 'r') ADVANCE(176);
      END_STATE();
    case 244:
      if (lookahead == 'i') ADVANCE(348);
      END_STATE();
    case 245:
      if (lookahead == 'i') ADVANCE(329);
      END_STATE();
    case 246:
      if (lookahead == 'i') ADVANCE(351);
      END_STATE();
    case 247:
      if (lookahead == 'i') ADVANCE(356);
      END_STATE();
    case 248:
      if (lookahead == 'i') ADVANCE(113);
      END_STATE();
    case 249:
      if (lookahead == 'i') ADVANCE(186);
      END_STATE();
    case 250:
      if (lookahead == 'j') ADVANCE(503);
      END_STATE();
    case 251:
      if (lookahead == 'k') ADVANCE(599);
      END_STATE();
    case 252:
      if (lookahead == 'k') ADVANCE(202);
      END_STATE();
    case 253:
      if (lookahead == 'k') ADVANCE(166);
      END_STATE();
    case 254:
      if (lookahead == 'l') ADVANCE(218);
      if (lookahead == 'o') ADVANCE(292);
      END_STATE();
    case 255:
      if (lookahead == 'l') ADVANCE(433);
      END_STATE();
    case 256:
      if (lookahead == 'l') ADVANCE(532);
      END_STATE();
    case 257:
      if (lookahead == 'l') ADVANCE(603);
      END_STATE();
    case 258:
      if (lookahead == 'l') ADVANCE(548);
      END_STATE();
    case 259:
      if (lookahead == 'l') ADVANCE(607);
      END_STATE();
    case 260:
      if (lookahead == 'l') ADVANCE(278);
      END_STATE();
    case 261:
      if (lookahead == 'l') ADVANCE(415);
      END_STATE();
    case 262:
      if (lookahead == 'l') ADVANCE(32);
      END_STATE();
    case 263:
      if (lookahead == 'l') ADVANCE(337);
      END_STATE();
    case 264:
      if (lookahead == 'l') ADVANCE(518);
      END_STATE();
    case 265:
      if (lookahead == 'l') ADVANCE(80);
      END_STATE();
    case 266:
      if (lookahead == 'l') ADVANCE(520);
      END_STATE();
    case 267:
      if (lookahead == 'l') ADVANCE(224);
      END_STATE();
    case 268:
      if (lookahead == 'l') ADVANCE(26);
      END_STATE();
    case 269:
      if (lookahead == 'l') ADVANCE(451);
      END_STATE();
    case 270:
      if (lookahead == 'l') ADVANCE(222);
      if (lookahead == 'n') ADVANCE(416);
      if (lookahead == 'o') ADVANCE(320);
      if (lookahead == 'q') ADVANCE(502);
      END_STATE();
    case 271:
      if (lookahead == 'l') ADVANCE(192);
      END_STATE();
    case 272:
      if (lookahead == 'l') ADVANCE(233);
      END_STATE();
    case 273:
      if (lookahead == 'l') ADVANCE(171);
      END_STATE();
    case 274:
      if (lookahead == 'l') ADVANCE(137);
      END_STATE();
    case 275:
      if (lookahead == 'l') ADVANCE(229);
      if (lookahead == 'o') ADVANCE(291);
      END_STATE();
    case 276:
      if (lookahead == 'l') ADVANCE(500);
      END_STATE();
    case 277:
      if (lookahead == 'l') ADVANCE(180);
      END_STATE();
    case 278:
      if (lookahead == 'l') ADVANCE(172);
      END_STATE();
    case 279:
      if (lookahead == 'l') ADVANCE(349);
      END_STATE();
    case 280:
      if (lookahead == 'l') ADVANCE(277);
      END_STATE();
    case 281:
      if (lookahead == 'l') ADVANCE(82);
      END_STATE();
    case 282:
      if (lookahead == 'l') ADVANCE(83);
      END_STATE();
    case 283:
      if (lookahead == 'l') ADVANCE(84);
      END_STATE();
    case 284:
      if (lookahead == 'm') ADVANCE(295);
      if (lookahead == 'n') ADVANCE(430);
      if (lookahead == 'o') ADVANCE(394);
      END_STATE();
    case 285:
      if (lookahead == 'm') ADVANCE(612);
      END_STATE();
    case 286:
      if (lookahead == 'm') ADVANCE(379);
      if (lookahead == 's') ADVANCE(444);
      END_STATE();
    case 287:
      if (lookahead == 'm') ADVANCE(379);
      if (lookahead == 's') ADVANCE(458);
      END_STATE();
    case 288:
      if (lookahead == 'm') ADVANCE(370);
      END_STATE();
    case 289:
      if (lookahead == 'm') ADVANCE(359);
      END_STATE();
    case 290:
      if (lookahead == 'm') ADVANCE(377);
      if (lookahead == 't') ADVANCE(163);
      END_STATE();
    case 291:
      if (lookahead == 'm') ADVANCE(367);
      END_STATE();
    case 292:
      if (lookahead == 'm') ADVANCE(367);
      if (lookahead == 'n') ADVANCE(430);
      END_STATE();
    case 293:
      if (lookahead == 'm') ADVANCE(234);
      END_STATE();
    case 294:
      if (lookahead == 'm') ADVANCE(170);
      END_STATE();
    case 295:
      if (lookahead == 'm') ADVANCE(71);
      if (lookahead == 'p') ADVANCE(240);
      END_STATE();
    case 296:
      if (lookahead == 'm') ADVANCE(347);
      END_STATE();
    case 297:
      if (lookahead == 'm') ADVANCE(371);
      END_STATE();
    case 298:
      if (lookahead == 'm') ADVANCE(378);
      if (lookahead == 'n') ADVANCE(98);
      END_STATE();
    case 299:
      if (lookahead == 'm') ADVANCE(382);
      if (lookahead == 's') ADVANCE(459);
      END_STATE();
    case 300:
      if (lookahead == 'm') ADVANCE(381);
      END_STATE();
    case 301:
      if (lookahead == 'm') ADVANCE(383);
      if (lookahead == 's') ADVANCE(473);
      END_STATE();
    case 302:
      if (lookahead == 'n') ADVANCE(546);
      END_STATE();
    case 303:
      if (lookahead == 'n') ADVANCE(553);
      END_STATE();
    case 304:
      if (lookahead == 'n') ADVANCE(552);
      END_STATE();
    case 305:
      if (lookahead == 'n') ADVANCE(593);
      END_STATE();
    case 306:
      if (lookahead == 'n') ADVANCE(464);
      END_STATE();
    case 307:
      if (lookahead == 'n') ADVANCE(212);
      END_STATE();
    case 308:
      if (lookahead == 'n') ADVANCE(445);
      END_STATE();
    case 309:
      if (lookahead == 'n') ADVANCE(207);
      END_STATE();
    case 310:
      if (lookahead == 'n') ADVANCE(446);
      END_STATE();
    case 311:
      if (lookahead == 'n') ADVANCE(96);
      END_STATE();
    case 312:
      if (lookahead == 'n') ADVANCE(123);
      END_STATE();
    case 313:
      if (lookahead == 'n') ADVANCE(435);
      END_STATE();
    case 314:
      if (lookahead == 'n') ADVANCE(118);
      END_STATE();
    case 315:
      if (lookahead == 'n') ADVANCE(132);
      END_STATE();
    case 316:
      if (lookahead == 'n') ADVANCE(426);
      END_STATE();
    case 317:
      if (lookahead == 'n') ADVANCE(158);
      END_STATE();
    case 318:
      if (lookahead == 'n') ADVANCE(67);
      END_STATE();
    case 319:
      if (lookahead == 'n') ADVANCE(105);
      END_STATE();
    case 320:
      if (lookahead == 'n') ADVANCE(264);
      END_STATE();
    case 321:
      if (lookahead == 'n') ADVANCE(266);
      END_STATE();
    case 322:
      if (lookahead == 'n') ADVANCE(248);
      END_STATE();
    case 323:
      if (lookahead == 'n') ADVANCE(228);
      END_STATE();
    case 324:
      if (lookahead == 'n') ADVANCE(136);
      END_STATE();
    case 325:
      if (lookahead == 'n') ADVANCE(423);
      END_STATE();
    case 326:
      if (lookahead == 'n') ADVANCE(457);
      END_STATE();
    case 327:
      if (lookahead == 'n') ADVANCE(235);
      END_STATE();
    case 328:
      if (lookahead == 'n') ADVANCE(173);
      END_STATE();
    case 329:
      if (lookahead == 'n') ADVANCE(146);
      END_STATE();
    case 330:
      if (lookahead == 'n') ADVANCE(429);
      END_STATE();
    case 331:
      if (lookahead == 'n') ADVANCE(479);
      END_STATE();
    case 332:
      if (lookahead == 'n') ADVANCE(491);
      END_STATE();
    case 333:
      if (lookahead == 'n') ADVANCE(470);
      END_STATE();
    case 334:
      if (lookahead == 'o') ADVANCE(111);
      END_STATE();
    case 335:
      if (lookahead == 'o') ADVANCE(288);
      END_STATE();
    case 336:
      if (lookahead == 'o') ADVANCE(315);
      END_STATE();
    case 337:
      if (lookahead == 'o') ADVANCE(507);
      END_STATE();
    case 338:
      if (lookahead == 'o') ADVANCE(373);
      END_STATE();
    case 339:
      if (lookahead == 'o') ADVANCE(251);
      END_STATE();
    case 340:
      if (lookahead == 'o') ADVANCE(385);
      if (lookahead == 'r') ADVANCE(237);
      END_STATE();
    case 341:
      if (lookahead == 'o') ADVANCE(198);
      END_STATE();
    case 342:
      if (lookahead == 'o') ADVANCE(498);
      END_STATE();
    case 343:
      if (lookahead == 'o') ADVANCE(261);
      END_STATE();
    case 344:
      if (lookahead == 'o') ADVANCE(497);
      END_STATE();
    case 345:
      if (lookahead == 'o') ADVANCE(396);
      END_STATE();
    case 346:
      if (lookahead == 'o') ADVANCE(320);
      END_STATE();
    case 347:
      if (lookahead == 'o') ADVANCE(393);
      END_STATE();
    case 348:
      if (lookahead == 'o') ADVANCE(302);
      END_STATE();
    case 349:
      if (lookahead == 'o') ADVANCE(406);
      END_STATE();
    case 350:
      if (lookahead == 'o') ADVANCE(303);
      END_STATE();
    case 351:
      if (lookahead == 'o') ADVANCE(304);
      END_STATE();
    case 352:
      if (lookahead == 'o') ADVANCE(330);
      END_STATE();
    case 353:
      if (lookahead == 'o') ADVANCE(387);
      END_STATE();
    case 354:
      if (lookahead == 'o') ADVANCE(388);
      END_STATE();
    case 355:
      if (lookahead == 'o') ADVANCE(375);
      END_STATE();
    case 356:
      if (lookahead == 'o') ADVANCE(325);
      END_STATE();
    case 357:
      if (lookahead == 'o') ADVANCE(323);
      END_STATE();
    case 358:
      if (lookahead == 'o') ADVANCE(321);
      END_STATE();
    case 359:
      if (lookahead == 'o') ADVANCE(126);
      END_STATE();
    case 360:
      if (lookahead == 'o') ADVANCE(332);
      END_STATE();
    case 361:
      if (lookahead == 'p') ADVANCE(161);
      if (lookahead == 's') ADVANCE(109);
      END_STATE();
    case 362:
      if (lookahead == 'p') ADVANCE(56);
      END_STATE();
    case 363:
      if (lookahead == 'p') ADVANCE(581);
      END_STATE();
    case 364:
      if (lookahead == 'p') ADVANCE(538);
      END_STATE();
    case 365:
      if (lookahead == 'p') ADVANCE(513);
      END_STATE();
    case 366:
      if (lookahead == 'p') ADVANCE(414);
      END_STATE();
    case 367:
      if (lookahead == 'p') ADVANCE(240);
      END_STATE();
    case 368:
      if (lookahead == 'p') ADVANCE(365);
      END_STATE();
    case 369:
      if (lookahead == 'p') ADVANCE(73);
      if (lookahead == 'r') ADVANCE(176);
      END_STATE();
    case 370:
      if (lookahead == 'p') ADVANCE(448);
      END_STATE();
    case 371:
      if (lookahead == 'p') ADVANCE(469);
      END_STATE();
    case 372:
      if (lookahead == 'p') ADVANCE(72);
      END_STATE();
    case 373:
      if (lookahead == 'p') ADVANCE(135);
      END_STATE();
    case 374:
      if (lookahead == 'p') ADVANCE(279);
      END_STATE();
    case 375:
      if (lookahead == 'p') ADVANCE(143);
      END_STATE();
    case 376:
      if (lookahead == 'p') ADVANCE(160);
      if (lookahead == 's') ADVANCE(109);
      END_STATE();
    case 377:
      if (lookahead == 'p') ADVANCE(55);
      END_STATE();
    case 378:
      if (lookahead == 'p') ADVANCE(77);
      END_STATE();
    case 379:
      if (lookahead == 'p') ADVANCE(281);
      END_STATE();
    case 380:
      if (lookahead == 'p') ADVANCE(492);
      END_STATE();
    case 381:
      if (lookahead == 'p') ADVANCE(86);
      END_STATE();
    case 382:
      if (lookahead == 'p') ADVANCE(282);
      END_STATE();
    case 383:
      if (lookahead == 'p') ADVANCE(283);
      END_STATE();
    case 384:
      if (lookahead == 'r') ADVANCE(50);
      END_STATE();
    case 385:
      if (lookahead == 'r') ADVANCE(252);
      END_STATE();
    case 386:
      if (lookahead == 'r') ADVANCE(531);
      END_STATE();
    case 387:
      if (lookahead == 'r') ADVANCE(579);
      END_STATE();
    case 388:
      if (lookahead == 'r') ADVANCE(542);
      END_STATE();
    case 389:
      if (lookahead == 'r') ADVANCE(611);
      END_STATE();
    case 390:
      if (lookahead == 'r') ADVANCE(53);
      END_STATE();
    case 391:
      if (lookahead == 'r') ADVANCE(413);
      END_STATE();
    case 392:
      if (lookahead == 'r') ADVANCE(24);
      END_STATE();
    case 393:
      if (lookahead == 'r') ADVANCE(514);
      END_STATE();
    case 394:
      if (lookahead == 'r') ADVANCE(124);
      END_STATE();
    case 395:
      if (lookahead == 'r') ADVANCE(44);
      END_STATE();
    case 396:
      if (lookahead == 'r') ADVANCE(447);
      END_STATE();
    case 397:
      if (lookahead == 'r') ADVANCE(220);
      END_STATE();
    case 398:
      if (lookahead == 'r') ADVANCE(431);
      END_STATE();
    case 399:
      if (lookahead == 'r') ADVANCE(490);
      END_STATE();
    case 400:
      if (lookahead == 'r') ADVANCE(521);
      END_STATE();
    case 401:
      if (lookahead == 'r') ADVANCE(87);
      END_STATE();
    case 402:
      if (lookahead == 'r') ADVANCE(226);
      END_STATE();
    case 403:
      if (lookahead == 'r') ADVANCE(419);
      END_STATE();
    case 404:
      if (lookahead == 'r') ADVANCE(49);
      END_STATE();
    case 405:
      if (lookahead == 'r') ADVANCE(452);
      END_STATE();
    case 406:
      if (lookahead == 'r') ADVANCE(138);
      END_STATE();
    case 407:
      if (lookahead == 'r') ADVANCE(236);
      END_STATE();
    case 408:
      if (lookahead == 'r') ADVANCE(439);
      END_STATE();
    case 409:
      if (lookahead == 'r') ADVANCE(106);
      END_STATE();
    case 410:
      if (lookahead == 'r') ADVANCE(85);
      END_STATE();
    case 411:
      if (lookahead == 's') ADVANCE(107);
      END_STATE();
    case 412:
      if (lookahead == 's') ADVANCE(107);
      if (lookahead == 'x') ADVANCE(374);
      END_STATE();
    case 413:
      if (lookahead == 's') ADVANCE(537);
      END_STATE();
    case 414:
      if (lookahead == 's') ADVANCE(606);
      END_STATE();
    case 415:
      if (lookahead == 's') ADVANCE(555);
      END_STATE();
    case 416:
      if (lookahead == 's') ADVANCE(563);
      END_STATE();
    case 417:
      if (lookahead == 's') ADVANCE(610);
      END_STATE();
    case 418:
      if (lookahead == 's') ADVANCE(564);
      END_STATE();
    case 419:
      if (lookahead == 's') ADVANCE(584);
      END_STATE();
    case 420:
      if (lookahead == 's') ADVANCE(554);
      END_STATE();
    case 421:
      if (lookahead == 's') ADVANCE(615);
      END_STATE();
    case 422:
      if (lookahead == 's') ADVANCE(576);
      END_STATE();
    case 423:
      if (lookahead == 's') ADVANCE(588);
      END_STATE();
    case 424:
      if (lookahead == 's') ADVANCE(566);
      END_STATE();
    case 425:
      if (lookahead == 's') ADVANCE(583);
      END_STATE();
    case 426:
      if (lookahead == 's') ADVANCE(501);
      END_STATE();
    case 427:
      if (lookahead == 's') ADVANCE(114);
      END_STATE();
    case 428:
      if (lookahead == 's') ADVANCE(114);
      if (lookahead == 't') ADVANCE(189);
      END_STATE();
    case 429:
      if (lookahead == 's') ADVANCE(164);
      END_STATE();
    case 430:
      if (lookahead == 's') ADVANCE(164);
      if (lookahead == 't') ADVANCE(151);
      END_STATE();
    case 431:
      if (lookahead == 's') ADVANCE(25);
      END_STATE();
    case 432:
      if (lookahead == 's') ADVANCE(344);
      END_STATE();
    case 433:
      if (lookahead == 's') ADVANCE(134);
      END_STATE();
    case 434:
      if (lookahead == 's') ADVANCE(468);
      END_STATE();
    case 435:
      if (lookahead == 's') ADVANCE(221);
      END_STATE();
    case 436:
      if (lookahead == 's') ADVANCE(23);
      END_STATE();
    case 437:
      if (lookahead == 's') ADVANCE(441);
      END_STATE();
    case 438:
      if (lookahead == 's') ADVANCE(421);
      END_STATE();
    case 439:
      if (lookahead == 's') ADVANCE(454);
      END_STATE();
    case 440:
      if (lookahead == 's') ADVANCE(461);
      END_STATE();
    case 441:
      if (lookahead == 's') ADVANCE(30);
      END_STATE();
    case 442:
      if (lookahead == 's') ADVANCE(475);
      END_STATE();
    case 443:
      if (lookahead == 's') ADVANCE(34);
      END_STATE();
    case 444:
      if (lookahead == 't') ADVANCE(575);
      END_STATE();
    case 445:
      if (lookahead == 't') ADVANCE(551);
      END_STATE();
    case 446:
      if (lookahead == 't') ADVANCE(528);
      END_STATE();
    case 447:
      if (lookahead == 't') ADVANCE(533);
      END_STATE();
    case 448:
      if (lookahead == 't') ADVANCE(550);
      END_STATE();
    case 449:
      if (lookahead == 't') ADVANCE(598);
      END_STATE();
    case 450:
      if (lookahead == 't') ADVANCE(577);
      END_STATE();
    case 451:
      if (lookahead == 't') ADVANCE(535);
      END_STATE();
    case 452:
      if (lookahead == 't') ADVANCE(589);
      END_STATE();
    case 453:
      if (lookahead == 't') ADVANCE(568);
      END_STATE();
    case 454:
      if (lookahead == 't') ADVANCE(609);
      END_STATE();
    case 455:
      if (lookahead == 't') ADVANCE(591);
      END_STATE();
    case 456:
      if (lookahead == 't') ADVANCE(585);
      END_STATE();
    case 457:
      if (lookahead == 't') ADVANCE(569);
      END_STATE();
    case 458:
      if (lookahead == 't') ADVANCE(574);
      END_STATE();
    case 459:
      if (lookahead == 't') ADVANCE(22);
      END_STATE();
    case 460:
      if (lookahead == 't') ADVANCE(597);
      END_STATE();
    case 461:
      if (lookahead == 't') ADVANCE(573);
      END_STATE();
    case 462:
      if (lookahead == 't') ADVANCE(48);
      END_STATE();
    case 463:
      if (lookahead == 't') ADVANCE(163);
      END_STATE();
    case 464:
      if (lookahead == 't') ADVANCE(227);
      END_STATE();
    case 465:
      if (lookahead == 't') ADVANCE(196);
      END_STATE();
    case 466:
      if (lookahead == 't') ADVANCE(519);
      END_STATE();
    case 467:
      if (lookahead == 't') ADVANCE(20);
      END_STATE();
    case 468:
      if (lookahead == 't') ADVANCE(65);
      END_STATE();
    case 469:
      if (lookahead == 't') ADVANCE(417);
      END_STATE();
    case 470:
      if (lookahead == 't') ADVANCE(151);
      END_STATE();
    case 471:
      if (lookahead == 't') ADVANCE(244);
      END_STATE();
    case 472:
      if (lookahead == 't') ADVANCE(17);
      END_STATE();
    case 473:
      if (lookahead == 't') ADVANCE(21);
      END_STATE();
    case 474:
      if (lookahead == 't') ADVANCE(167);
      END_STATE();
    case 475:
      if (lookahead == 't') ADVANCE(422);
      END_STATE();
    case 476:
      if (lookahead == 't') ADVANCE(36);
      END_STATE();
    case 477:
      if (lookahead == 't') ADVANCE(223);
      END_STATE();
    case 478:
      if (lookahead == 't') ADVANCE(154);
      END_STATE();
    case 479:
      if (lookahead == 't') ADVANCE(425);
      END_STATE();
    case 480:
      if (lookahead == 't') ADVANCE(190);
      END_STATE();
    case 481:
      if (lookahead == 't') ADVANCE(174);
      END_STATE();
    case 482:
      if (lookahead == 't') ADVANCE(140);
      END_STATE();
    case 483:
      if (lookahead == 't') ADVANCE(189);
      END_STATE();
    case 484:
      if (lookahead == 't') ADVANCE(149);
      END_STATE();
    case 485:
      if (lookahead == 't') ADVANCE(181);
      END_STATE();
    case 486:
      if (lookahead == 't') ADVANCE(31);
      END_STATE();
    case 487:
      if (lookahead == 't') ADVANCE(407);
      END_STATE();
    case 488:
      if (lookahead == 't') ADVANCE(354);
      END_STATE();
    case 489:
      if (lookahead == 't') ADVANCE(182);
      END_STATE();
    case 490:
      if (lookahead == 't') ADVANCE(70);
      END_STATE();
    case 491:
      if (lookahead == 't') ADVANCE(194);
      END_STATE();
    case 492:
      if (lookahead == 't') ADVANCE(246);
      END_STATE();
    case 493:
      if (lookahead == 't') ADVANCE(247);
      END_STATE();
    case 494:
      if (lookahead == 'u') ADVANCE(116);
      END_STATE();
    case 495:
      if (lookahead == 'u') ADVANCE(133);
      END_STATE();
    case 496:
      if (lookahead == 'u') ADVANCE(269);
      END_STATE();
    case 497:
      if (lookahead == 'u') ADVANCE(409);
      END_STATE();
    case 498:
      if (lookahead == 'u') ADVANCE(455);
      END_STATE();
    case 499:
      if (lookahead == 'u') ADVANCE(471);
      END_STATE();
    case 500:
      if (lookahead == 'u') ADVANCE(125);
      END_STATE();
    case 501:
      if (lookahead == 'u') ADVANCE(436);
      END_STATE();
    case 502:
      if (lookahead == 'u') ADVANCE(179);
      END_STATE();
    case 503:
      if (lookahead == 'u') ADVANCE(127);
      END_STATE();
    case 504:
      if (lookahead == 'v') ADVANCE(231);
      END_STATE();
    case 505:
      if (lookahead == 'v') ADVANCE(59);
      END_STATE();
    case 506:
      if (lookahead == 'v') ADVANCE(141);
      END_STATE();
    case 507:
      if (lookahead == 'w') ADVANCE(605);
      END_STATE();
    case 508:
      if (lookahead == 'w') ADVANCE(317);
      END_STATE();
    case 509:
      if (lookahead == 'w') ADVANCE(175);
      END_STATE();
    case 510:
      if (lookahead == 'x') ADVANCE(14);
      END_STATE();
    case 511:
      if (lookahead == 'x') ADVANCE(450);
      END_STATE();
    case 512:
      if (lookahead == 'x') ADVANCE(19);
      END_STATE();
    case 513:
      if (lookahead == 'y') ADVANCE(572);
      END_STATE();
    case 514:
      if (lookahead == 'y') ADVANCE(562);
      END_STATE();
    case 515:
      if (lookahead == 'y') ADVANCE(570);
      END_STATE();
    case 516:
      if (lookahead == 'y') ADVANCE(534);
      END_STATE();
    case 517:
      if (lookahead == 'y') ADVANCE(608);
      END_STATE();
    case 518:
      if (lookahead == 'y') ADVANCE(560);
      END_STATE();
    case 519:
      if (lookahead == 'y') ADVANCE(590);
      END_STATE();
    case 520:
      if (lookahead == 'y') ADVANCE(614);
      END_STATE();
    case 521:
      if (lookahead == 'y') ADVANCE(567);
      END_STATE();
    case 522:
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(623);
      END_STATE();
    case 523:
      if (lookahead != 0 &&
          lookahead != '\n') ADVANCE(3);
      END_STATE();
    case 524:
      if (eof) ADVANCE(525);
      ADVANCE_MAP(
        '/', 13,
        '[', 594,
        'a', 208,
        'c', 39,
        'd', 152,
        'e', 412,
        'i', 298,
        'm', 69,
        'o', 508,
        'p', 46,
        'r', 177,
        's', 95,
        't', 188,
        'v', 51,
        'w', 340,
        '}', 530,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(524);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(7);
      END_STATE();
    case 525:
      ACCEPT_TOKEN(ts_builtin_sym_end);
      END_STATE();
    case 526:
      ACCEPT_TOKEN(anon_sym_include);
      END_STATE();
    case 527:
      ACCEPT_TOKEN(sym_comment);
      if (lookahead != 0 &&
          lookahead != '\n') ADVANCE(527);
      END_STATE();
    case 528:
      ACCEPT_TOKEN(anon_sym_client);
      END_STATE();
    case 529:
      ACCEPT_TOKEN(anon_sym_LBRACE);
      END_STATE();
    case 530:
      ACCEPT_TOKEN(anon_sym_RBRACE);
      END_STATE();
    case 531:
      ACCEPT_TOKEN(anon_sym_tier);
      END_STATE();
    case 532:
      ACCEPT_TOKEN(anon_sym_model);
      END_STATE();
    case 533:
      ACCEPT_TOKEN(anon_sym_effort);
      END_STATE();
    case 534:
      ACCEPT_TOKEN(anon_sym_privacy);
      END_STATE();
    case 535:
      ACCEPT_TOKEN(anon_sym_default);
      END_STATE();
    case 536:
      ACCEPT_TOKEN(anon_sym_extra);
      END_STATE();
    case 537:
      ACCEPT_TOKEN(anon_sym_vars);
      END_STATE();
    case 538:
      ACCEPT_TOKEN(anon_sym_cheap);
      END_STATE();
    case 539:
      ACCEPT_TOKEN(anon_sym_cheap);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 540:
      ACCEPT_TOKEN(anon_sym_expensive);
      END_STATE();
    case 541:
      ACCEPT_TOKEN(anon_sym_expensive);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 542:
      ACCEPT_TOKEN(anon_sym_coordinator);
      END_STATE();
    case 543:
      ACCEPT_TOKEN(anon_sym_coordinator);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 544:
      ACCEPT_TOKEN(anon_sym_reasoning);
      END_STATE();
    case 545:
      ACCEPT_TOKEN(anon_sym_reasoning);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 546:
      ACCEPT_TOKEN(anon_sym_execution);
      END_STATE();
    case 547:
      ACCEPT_TOKEN(anon_sym_execution);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 548:
      ACCEPT_TOKEN(anon_sym_mechanical);
      END_STATE();
    case 549:
      ACCEPT_TOKEN(anon_sym_mechanical);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 550:
      ACCEPT_TOKEN(anon_sym_prompt);
      END_STATE();
    case 551:
      ACCEPT_TOKEN(anon_sym_agent);
      END_STATE();
    case 552:
      ACCEPT_TOKEN(anon_sym_description);
      END_STATE();
    case 553:
      ACCEPT_TOKEN(anon_sym_depends_on);
      END_STATE();
    case 554:
      ACCEPT_TOKEN(anon_sym_max_retries);
      END_STATE();
    case 555:
      ACCEPT_TOKEN(anon_sym_tools);
      END_STATE();
    case 556:
      ACCEPT_TOKEN(anon_sym_template);
      END_STATE();
    case 557:
      ACCEPT_TOKEN(anon_sym_template);
      if (lookahead == '_') ADVANCE(241);
      END_STATE();
    case 558:
      ACCEPT_TOKEN(anon_sym_scope);
      END_STATE();
    case 559:
      ACCEPT_TOKEN(anon_sym_owned);
      END_STATE();
    case 560:
      ACCEPT_TOKEN(anon_sym_read_only);
      END_STATE();
    case 561:
      ACCEPT_TOKEN(anon_sym_impact_scope);
      END_STATE();
    case 562:
      ACCEPT_TOKEN(anon_sym_memory);
      END_STATE();
    case 563:
      ACCEPT_TOKEN(anon_sym_read_ns);
      END_STATE();
    case 564:
      ACCEPT_TOKEN(anon_sym_write_ns);
      END_STATE();
    case 565:
      ACCEPT_TOKEN(anon_sym_importance);
      END_STATE();
    case 566:
      ACCEPT_TOKEN(anon_sym_staleness_sources);
      END_STATE();
    case 567:
      ACCEPT_TOKEN(anon_sym_read_query);
      END_STATE();
    case 568:
      ACCEPT_TOKEN(anon_sym_read_limit);
      END_STATE();
    case 569:
      ACCEPT_TOKEN(anon_sym_write_content);
      END_STATE();
    case 570:
      ACCEPT_TOKEN(anon_sym_verify);
      END_STATE();
    case 571:
      ACCEPT_TOKEN(anon_sym_compile);
      END_STATE();
    case 572:
      ACCEPT_TOKEN(anon_sym_clippy);
      END_STATE();
    case 573:
      ACCEPT_TOKEN(anon_sym_test);
      END_STATE();
    case 574:
      ACCEPT_TOKEN(anon_sym_test);
      if (lookahead == '_') ADVANCE(203);
      END_STATE();
    case 575:
      ACCEPT_TOKEN(anon_sym_test);
      if (lookahead == '_') ADVANCE(203);
      if (lookahead == 's') ADVANCE(37);
      END_STATE();
    case 576:
      ACCEPT_TOKEN(anon_sym_impact_tests);
      END_STATE();
    case 577:
      ACCEPT_TOKEN(anon_sym_context);
      END_STATE();
    case 578:
      ACCEPT_TOKEN(anon_sym_callers_of);
      END_STATE();
    case 579:
      ACCEPT_TOKEN(anon_sym_tests_for);
      END_STATE();
    case 580:
      ACCEPT_TOKEN(anon_sym_depth);
      END_STATE();
    case 581:
      ACCEPT_TOKEN(anon_sym_loop);
      END_STATE();
    case 582:
      ACCEPT_TOKEN(anon_sym_loop);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 583:
      ACCEPT_TOKEN(anon_sym_agents);
      END_STATE();
    case 584:
      ACCEPT_TOKEN(anon_sym_reviewers);
      END_STATE();
    case 585:
      ACCEPT_TOKEN(anon_sym_template_init);
      END_STATE();
    case 586:
      ACCEPT_TOKEN(anon_sym_template_refine);
      END_STATE();
    case 587:
      ACCEPT_TOKEN(anon_sym_consensus_mode);
      END_STATE();
    case 588:
      ACCEPT_TOKEN(anon_sym_max_iterations);
      END_STATE();
    case 589:
      ACCEPT_TOKEN(anon_sym_iter_start);
      END_STATE();
    case 590:
      ACCEPT_TOKEN(anon_sym_stability);
      END_STATE();
    case 591:
      ACCEPT_TOKEN(anon_sym_judge_timeout);
      END_STATE();
    case 592:
      ACCEPT_TOKEN(anon_sym_strict_judge);
      END_STATE();
    case 593:
      ACCEPT_TOKEN(anon_sym_branch_chain);
      END_STATE();
    case 594:
      ACCEPT_TOKEN(anon_sym_LBRACK);
      END_STATE();
    case 595:
      ACCEPT_TOKEN(anon_sym_RBRACK);
      END_STATE();
    case 596:
      ACCEPT_TOKEN(anon_sym_id);
      END_STATE();
    case 597:
      ACCEPT_TOKEN(anon_sym_strict);
      END_STATE();
    case 598:
      ACCEPT_TOKEN(anon_sym_strict);
      if (lookahead == '_') ADVANCE(250);
      END_STATE();
    case 599:
      ACCEPT_TOKEN(anon_sym_partial_ok);
      END_STATE();
    case 600:
      ACCEPT_TOKEN(anon_sym_explore);
      END_STATE();
    case 601:
      ACCEPT_TOKEN(anon_sym_stacked);
      END_STATE();
    case 602:
      ACCEPT_TOKEN(anon_sym_none);
      END_STATE();
    case 603:
      ACCEPT_TOKEN(anon_sym_until);
      END_STATE();
    case 604:
      ACCEPT_TOKEN(anon_sym_command);
      END_STATE();
    case 605:
      ACCEPT_TOKEN(anon_sym_workflow);
      END_STATE();
    case 606:
      ACCEPT_TOKEN(anon_sym_steps);
      END_STATE();
    case 607:
      ACCEPT_TOKEN(anon_sym_max_parallel);
      END_STATE();
    case 608:
      ACCEPT_TOKEN(anon_sym_strategy);
      END_STATE();
    case 609:
      ACCEPT_TOKEN(anon_sym_test_first);
      END_STATE();
    case 610:
      ACCEPT_TOKEN(anon_sym_attempts);
      END_STATE();
    case 611:
      ACCEPT_TOKEN(anon_sym_escalate_after);
      END_STATE();
    case 612:
      ACCEPT_TOKEN(anon_sym_param);
      END_STATE();
    case 613:
      ACCEPT_TOKEN(anon_sym_public);
      END_STATE();
    case 614:
      ACCEPT_TOKEN(anon_sym_local_only);
      END_STATE();
    case 615:
      ACCEPT_TOKEN(anon_sym_single_pass);
      END_STATE();
    case 616:
      ACCEPT_TOKEN(anon_sym_single_pass);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 617:
      ACCEPT_TOKEN(anon_sym_refine);
      END_STATE();
    case 618:
      ACCEPT_TOKEN(anon_sym_refine);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 619:
      ACCEPT_TOKEN(anon_sym_true);
      END_STATE();
    case 620:
      ACCEPT_TOKEN(anon_sym_false);
      END_STATE();
    case 621:
      ACCEPT_TOKEN(sym_string);
      END_STATE();
    case 622:
      ACCEPT_TOKEN(sym_raw_string);
      END_STATE();
    case 623:
      ACCEPT_TOKEN(sym_float);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(623);
      END_STATE();
    case 624:
      ACCEPT_TOKEN(sym_integer);
      if (lookahead == '.') ADVANCE(522);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(624);
      END_STATE();
    case 625:
      ACCEPT_TOKEN(sym_integer);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(625);
      END_STATE();
    case 626:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '_') ADVANCE(676);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 627:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(680);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 628:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(674);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 629:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(658);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 630:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(665);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 631:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(684);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 632:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(682);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 633:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(650);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 634:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(685);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 635:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(629);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 636:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'd') ADVANCE(653);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 637:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(633);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 638:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(627);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 639:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(662);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 640:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(541);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 641:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(646);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 642:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(618);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 643:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(626);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 644:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(634);
      if (lookahead == 'p') ADVANCE(639);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 645:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(628);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 646:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'f') ADVANCE(656);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 647:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'g') ADVANCE(545);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 648:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'g') ADVANCE(659);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 649:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(645);
      if (lookahead == 'o') ADVANCE(668);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 650:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(630);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 651:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(686);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 652:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(635);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 653:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(664);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 654:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(660);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 655:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(663);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 656:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(666);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 657:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(673);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 658:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(549);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 659:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(643);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 660:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(647);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 661:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(547);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 662:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(681);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 663:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(648);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 664:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(631);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 665:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(652);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 666:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(642);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 667:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(654);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 668:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(677);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 669:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(678);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 670:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(675);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 671:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(667);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 672:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(670);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 673:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(661);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 674:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(539);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 675:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(582);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 676:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(632);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 677:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(636);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 678:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(543);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 679:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(616);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 680:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(671);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 681:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(651);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 682:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(679);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 683:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(657);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 684:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(669);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 685:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'u') ADVANCE(683);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 686:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'v') ADVANCE(640);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 687:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'x') ADVANCE(644);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    case 688:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(688);
      END_STATE();
    default:
      return false;
  }
}

static const TSLexMode ts_lex_modes[STATE_COUNT] = {
  [0] = {.lex_state = 0},
  [1] = {.lex_state = 0},
  [2] = {.lex_state = 9},
  [3] = {.lex_state = 524},
  [4] = {.lex_state = 524},
  [5] = {.lex_state = 9},
  [6] = {.lex_state = 9},
  [7] = {.lex_state = 10},
  [8] = {.lex_state = 10},
  [9] = {.lex_state = 524},
  [10] = {.lex_state = 524},
  [11] = {.lex_state = 524},
  [12] = {.lex_state = 524},
  [13] = {.lex_state = 524},
  [14] = {.lex_state = 524},
  [15] = {.lex_state = 0},
  [16] = {.lex_state = 524},
  [17] = {.lex_state = 0},
  [18] = {.lex_state = 524},
  [19] = {.lex_state = 524},
  [20] = {.lex_state = 524},
  [21] = {.lex_state = 10},
  [22] = {.lex_state = 10},
  [23] = {.lex_state = 10},
  [24] = {.lex_state = 10},
  [25] = {.lex_state = 524},
  [26] = {.lex_state = 524},
  [27] = {.lex_state = 524},
  [28] = {.lex_state = 524},
  [29] = {.lex_state = 524},
  [30] = {.lex_state = 524},
  [31] = {.lex_state = 10},
  [32] = {.lex_state = 10},
  [33] = {.lex_state = 524},
  [34] = {.lex_state = 10},
  [35] = {.lex_state = 10},
  [36] = {.lex_state = 10},
  [37] = {.lex_state = 10},
  [38] = {.lex_state = 10},
  [39] = {.lex_state = 10},
  [40] = {.lex_state = 524},
  [41] = {.lex_state = 2},
  [42] = {.lex_state = 2},
  [43] = {.lex_state = 524},
  [44] = {.lex_state = 524},
  [45] = {.lex_state = 524},
  [46] = {.lex_state = 524},
  [47] = {.lex_state = 524},
  [48] = {.lex_state = 524},
  [49] = {.lex_state = 524},
  [50] = {.lex_state = 524},
  [51] = {.lex_state = 0},
  [52] = {.lex_state = 0},
  [53] = {.lex_state = 0},
  [54] = {.lex_state = 0},
  [55] = {.lex_state = 0},
  [56] = {.lex_state = 0},
  [57] = {.lex_state = 2},
  [58] = {.lex_state = 0},
  [59] = {.lex_state = 0},
  [60] = {.lex_state = 2},
  [61] = {.lex_state = 0},
  [62] = {.lex_state = 0},
  [63] = {.lex_state = 0},
  [64] = {.lex_state = 0},
  [65] = {.lex_state = 0},
  [66] = {.lex_state = 0},
  [67] = {.lex_state = 0},
  [68] = {.lex_state = 0},
  [69] = {.lex_state = 0},
  [70] = {.lex_state = 1},
  [71] = {.lex_state = 11},
  [72] = {.lex_state = 0},
  [73] = {.lex_state = 0},
  [74] = {.lex_state = 0},
  [75] = {.lex_state = 0},
  [76] = {.lex_state = 11},
  [77] = {.lex_state = 1},
  [78] = {.lex_state = 0},
  [79] = {.lex_state = 11},
  [80] = {.lex_state = 0},
  [81] = {.lex_state = 11},
  [82] = {.lex_state = 0},
  [83] = {.lex_state = 1},
  [84] = {.lex_state = 11},
  [85] = {.lex_state = 0},
  [86] = {.lex_state = 524},
  [87] = {.lex_state = 0},
  [88] = {.lex_state = 524},
  [89] = {.lex_state = 0},
  [90] = {.lex_state = 0},
  [91] = {.lex_state = 524},
  [92] = {.lex_state = 0},
  [93] = {.lex_state = 0},
  [94] = {.lex_state = 0},
  [95] = {.lex_state = 0},
  [96] = {.lex_state = 8},
  [97] = {.lex_state = 8},
  [98] = {.lex_state = 11},
  [99] = {.lex_state = 0},
  [100] = {.lex_state = 8},
  [101] = {.lex_state = 524},
  [102] = {.lex_state = 0},
  [103] = {.lex_state = 1},
  [104] = {.lex_state = 524},
  [105] = {.lex_state = 0},
  [106] = {.lex_state = 0},
  [107] = {.lex_state = 1},
  [108] = {.lex_state = 0},
  [109] = {.lex_state = 1},
  [110] = {.lex_state = 1},
  [111] = {.lex_state = 0},
  [112] = {.lex_state = 12},
  [113] = {.lex_state = 1},
  [114] = {.lex_state = 0},
  [115] = {.lex_state = 0},
  [116] = {.lex_state = 0},
  [117] = {.lex_state = 0},
  [118] = {.lex_state = 0},
  [119] = {.lex_state = 0},
  [120] = {.lex_state = 1},
  [121] = {.lex_state = 1},
  [122] = {.lex_state = 8},
  [123] = {.lex_state = 1},
  [124] = {.lex_state = 0},
  [125] = {.lex_state = 0},
  [126] = {.lex_state = 8},
  [127] = {.lex_state = 0},
  [128] = {.lex_state = 0},
  [129] = {.lex_state = 0},
  [130] = {.lex_state = 1},
  [131] = {.lex_state = 0},
  [132] = {.lex_state = 0},
  [133] = {.lex_state = 0},
  [134] = {.lex_state = 0},
  [135] = {.lex_state = 0},
  [136] = {.lex_state = 0},
  [137] = {.lex_state = 0},
  [138] = {.lex_state = 0},
  [139] = {.lex_state = 0},
  [140] = {.lex_state = 1},
  [141] = {.lex_state = 0},
  [142] = {.lex_state = 0},
  [143] = {.lex_state = 0},
  [144] = {.lex_state = 0},
  [145] = {.lex_state = 0},
  [146] = {.lex_state = 1},
  [147] = {.lex_state = 0},
  [148] = {.lex_state = 1},
  [149] = {.lex_state = 0},
  [150] = {.lex_state = 1},
  [151] = {.lex_state = 1},
  [152] = {.lex_state = 1},
  [153] = {.lex_state = 9},
  [154] = {.lex_state = 0},
  [155] = {.lex_state = 0},
  [156] = {.lex_state = 9},
  [157] = {.lex_state = 9},
  [158] = {.lex_state = 0},
  [159] = {.lex_state = 1},
  [160] = {.lex_state = 1},
  [161] = {.lex_state = 0},
  [162] = {.lex_state = 0},
  [163] = {.lex_state = 524},
  [164] = {.lex_state = 9},
  [165] = {.lex_state = 1},
  [166] = {.lex_state = 0},
  [167] = {.lex_state = 0},
  [168] = {.lex_state = 1},
  [169] = {.lex_state = 9},
  [170] = {.lex_state = 0},
  [171] = {.lex_state = 1},
  [172] = {.lex_state = 0},
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
    [anon_sym_param] = ACTIONS(1),
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
    [sym_source_file] = STATE(170),
    [sym__definition] = STATE(17),
    [sym_include_declaration] = STATE(17),
    [sym_client_declaration] = STATE(17),
    [sym_vars_block] = STATE(17),
    [sym_tier_alias_declaration] = STATE(17),
    [sym_prompt_declaration] = STATE(17),
    [sym_agent_declaration] = STATE(17),
    [sym_workflow_declaration] = STATE(17),
    [aux_sym_source_file_repeat1] = STATE(17),
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
    ACTIONS(21), 38,
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
      anon_sym_param,
  [48] = 2,
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
  [80] = 2,
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
  [112] = 3,
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
  [145] = 3,
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
  [178] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(37), 23,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
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
      anon_sym_param,
  [207] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(39), 23,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
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
      anon_sym_param,
  [236] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(41), 21,
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
      anon_sym_param,
  [263] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(43), 21,
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
      anon_sym_param,
  [290] = 16,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(13), 1,
      anon_sym_vars,
    ACTIONS(45), 1,
      anon_sym_client,
    ACTIONS(47), 1,
      anon_sym_RBRACE,
    ACTIONS(49), 1,
      anon_sym_tier,
    ACTIONS(51), 1,
      anon_sym_prompt,
    ACTIONS(53), 1,
      anon_sym_description,
    ACTIONS(55), 1,
      anon_sym_depends_on,
    ACTIONS(57), 1,
      anon_sym_max_retries,
    ACTIONS(59), 1,
      anon_sym_tools,
    ACTIONS(61), 1,
      anon_sym_template,
    ACTIONS(63), 1,
      anon_sym_scope,
    ACTIONS(65), 1,
      anon_sym_memory,
    ACTIONS(67), 1,
      anon_sym_context,
    STATE(13), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(25), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [343] = 16,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(13), 1,
      anon_sym_vars,
    ACTIONS(45), 1,
      anon_sym_client,
    ACTIONS(49), 1,
      anon_sym_tier,
    ACTIONS(51), 1,
      anon_sym_prompt,
    ACTIONS(53), 1,
      anon_sym_description,
    ACTIONS(55), 1,
      anon_sym_depends_on,
    ACTIONS(57), 1,
      anon_sym_max_retries,
    ACTIONS(59), 1,
      anon_sym_tools,
    ACTIONS(61), 1,
      anon_sym_template,
    ACTIONS(63), 1,
      anon_sym_scope,
    ACTIONS(65), 1,
      anon_sym_memory,
    ACTIONS(67), 1,
      anon_sym_context,
    ACTIONS(69), 1,
      anon_sym_RBRACE,
    STATE(11), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(25), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [396] = 16,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(71), 1,
      anon_sym_client,
    ACTIONS(74), 1,
      anon_sym_RBRACE,
    ACTIONS(76), 1,
      anon_sym_tier,
    ACTIONS(79), 1,
      anon_sym_vars,
    ACTIONS(82), 1,
      anon_sym_prompt,
    ACTIONS(85), 1,
      anon_sym_description,
    ACTIONS(88), 1,
      anon_sym_depends_on,
    ACTIONS(91), 1,
      anon_sym_max_retries,
    ACTIONS(94), 1,
      anon_sym_tools,
    ACTIONS(97), 1,
      anon_sym_template,
    ACTIONS(100), 1,
      anon_sym_scope,
    ACTIONS(103), 1,
      anon_sym_memory,
    ACTIONS(106), 1,
      anon_sym_context,
    STATE(13), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(25), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [449] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(109), 17,
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
  [472] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(111), 1,
      ts_builtin_sym_end,
    ACTIONS(113), 1,
      anon_sym_include,
    ACTIONS(116), 1,
      anon_sym_client,
    ACTIONS(119), 1,
      anon_sym_tier,
    ACTIONS(122), 1,
      anon_sym_vars,
    ACTIONS(125), 1,
      anon_sym_prompt,
    ACTIONS(128), 1,
      anon_sym_agent,
    ACTIONS(131), 1,
      anon_sym_workflow,
    STATE(15), 9,
      sym__definition,
      sym_include_declaration,
      sym_client_declaration,
      sym_vars_block,
      sym_tier_alias_declaration,
      sym_prompt_declaration,
      sym_agent_declaration,
      sym_workflow_declaration,
      aux_sym_source_file_repeat1,
  [511] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(134), 17,
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
  [534] = 10,
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
    ACTIONS(136), 1,
      ts_builtin_sym_end,
    STATE(15), 9,
      sym__definition,
      sym_include_declaration,
      sym_client_declaration,
      sym_vars_block,
      sym_tier_alias_declaration,
      sym_prompt_declaration,
      sym_agent_declaration,
      sym_workflow_declaration,
      aux_sym_source_file_repeat1,
  [573] = 11,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(65), 1,
      anon_sym_memory,
    ACTIONS(138), 1,
      anon_sym_RBRACE,
    ACTIONS(142), 1,
      anon_sym_verify,
    ACTIONS(144), 1,
      anon_sym_steps,
    ACTIONS(146), 1,
      anon_sym_strategy,
    ACTIONS(148), 1,
      anon_sym_test_first,
    ACTIONS(150), 1,
      anon_sym_param,
    STATE(19), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(45), 3,
      sym_memory_block,
      sym_verify_block,
      sym_param_declaration,
    ACTIONS(140), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [613] = 11,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(65), 1,
      anon_sym_memory,
    ACTIONS(142), 1,
      anon_sym_verify,
    ACTIONS(144), 1,
      anon_sym_steps,
    ACTIONS(146), 1,
      anon_sym_strategy,
    ACTIONS(148), 1,
      anon_sym_test_first,
    ACTIONS(150), 1,
      anon_sym_param,
    ACTIONS(152), 1,
      anon_sym_RBRACE,
    STATE(20), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(45), 3,
      sym_memory_block,
      sym_verify_block,
      sym_param_declaration,
    ACTIONS(140), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [653] = 11,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(154), 1,
      anon_sym_RBRACE,
    ACTIONS(159), 1,
      anon_sym_memory,
    ACTIONS(162), 1,
      anon_sym_verify,
    ACTIONS(165), 1,
      anon_sym_steps,
    ACTIONS(168), 1,
      anon_sym_strategy,
    ACTIONS(171), 1,
      anon_sym_test_first,
    ACTIONS(174), 1,
      anon_sym_param,
    STATE(20), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(45), 3,
      sym_memory_block,
      sym_verify_block,
      sym_param_declaration,
    ACTIONS(156), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [693] = 12,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(177), 1,
      anon_sym_RBRACE,
    ACTIONS(179), 1,
      anon_sym_agents,
    ACTIONS(181), 1,
      anon_sym_reviewers,
    ACTIONS(185), 1,
      anon_sym_consensus_mode,
    ACTIONS(189), 1,
      anon_sym_strict_judge,
    ACTIONS(191), 1,
      anon_sym_branch_chain,
    ACTIONS(193), 1,
      anon_sym_until,
    STATE(36), 1,
      sym_until_clause,
    ACTIONS(183), 2,
      anon_sym_template_init,
      anon_sym_template_refine,
    STATE(22), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(187), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [735] = 12,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(179), 1,
      anon_sym_agents,
    ACTIONS(181), 1,
      anon_sym_reviewers,
    ACTIONS(185), 1,
      anon_sym_consensus_mode,
    ACTIONS(189), 1,
      anon_sym_strict_judge,
    ACTIONS(191), 1,
      anon_sym_branch_chain,
    ACTIONS(193), 1,
      anon_sym_until,
    ACTIONS(195), 1,
      anon_sym_RBRACE,
    STATE(36), 1,
      sym_until_clause,
    ACTIONS(183), 2,
      anon_sym_template_init,
      anon_sym_template_refine,
    STATE(23), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(187), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [777] = 12,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(197), 1,
      anon_sym_RBRACE,
    ACTIONS(199), 1,
      anon_sym_agents,
    ACTIONS(202), 1,
      anon_sym_reviewers,
    ACTIONS(208), 1,
      anon_sym_consensus_mode,
    ACTIONS(214), 1,
      anon_sym_strict_judge,
    ACTIONS(217), 1,
      anon_sym_branch_chain,
    ACTIONS(220), 1,
      anon_sym_until,
    STATE(36), 1,
      sym_until_clause,
    ACTIONS(205), 2,
      anon_sym_template_init,
      anon_sym_template_refine,
    STATE(23), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(211), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [819] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(223), 13,
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
  [838] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(225), 13,
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
  [857] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(229), 1,
      anon_sym_LBRACK,
    STATE(48), 1,
      sym_reviewer_list,
    ACTIONS(227), 11,
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
      anon_sym_param,
  [880] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(231), 13,
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
  [899] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(233), 13,
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
  [918] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(235), 13,
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
  [937] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(237), 13,
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
  [956] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(239), 13,
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
  [975] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(241), 13,
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
  [994] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(243), 13,
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
  [1013] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(245), 13,
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
  [1032] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(247), 13,
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
  [1051] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(249), 13,
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
  [1070] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(251), 13,
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
  [1089] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(253), 13,
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
  [1108] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(255), 13,
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
  [1127] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(257), 13,
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
  [1146] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(263), 1,
      sym_identifier,
    ACTIONS(261), 2,
      sym_string,
      sym_raw_string,
    STATE(111), 3,
      sym__effort_value,
      sym_tier_value,
      sym__string_value,
    ACTIONS(259), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [1170] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(267), 1,
      sym_identifier,
    ACTIONS(265), 2,
      sym_string,
      sym_raw_string,
    STATE(73), 3,
      sym__effort_value,
      sym_tier_value,
      sym__string_value,
    ACTIONS(259), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [1194] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(269), 11,
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
      anon_sym_param,
  [1211] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(271), 11,
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
      anon_sym_param,
  [1228] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(273), 11,
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
      anon_sym_param,
  [1245] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(275), 11,
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
      anon_sym_param,
  [1262] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(277), 11,
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
      anon_sym_param,
  [1279] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(279), 11,
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
      anon_sym_param,
  [1296] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(281), 11,
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
      anon_sym_param,
  [1313] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(283), 11,
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
      anon_sym_param,
  [1330] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(285), 1,
      anon_sym_RBRACE,
    ACTIONS(287), 1,
      anon_sym_tier,
    ACTIONS(290), 1,
      anon_sym_model,
    ACTIONS(293), 1,
      anon_sym_effort,
    ACTIONS(296), 1,
      anon_sym_privacy,
    ACTIONS(299), 1,
      anon_sym_default,
    ACTIONS(302), 1,
      anon_sym_extra,
    STATE(78), 1,
      sym_extra_block,
    STATE(51), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [1362] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(305), 1,
      anon_sym_RBRACE,
    ACTIONS(311), 1,
      anon_sym_importance,
    ACTIONS(313), 1,
      anon_sym_read_limit,
    ACTIONS(307), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(56), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(309), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [1388] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(315), 1,
      anon_sym_RBRACE,
    ACTIONS(317), 1,
      anon_sym_tier,
    ACTIONS(319), 1,
      anon_sym_model,
    ACTIONS(321), 1,
      anon_sym_effort,
    ACTIONS(323), 1,
      anon_sym_privacy,
    ACTIONS(325), 1,
      anon_sym_default,
    ACTIONS(327), 1,
      anon_sym_extra,
    STATE(78), 1,
      sym_extra_block,
    STATE(55), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [1420] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(311), 1,
      anon_sym_importance,
    ACTIONS(313), 1,
      anon_sym_read_limit,
    ACTIONS(329), 1,
      anon_sym_RBRACE,
    ACTIONS(307), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(52), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(309), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [1446] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(317), 1,
      anon_sym_tier,
    ACTIONS(319), 1,
      anon_sym_model,
    ACTIONS(321), 1,
      anon_sym_effort,
    ACTIONS(323), 1,
      anon_sym_privacy,
    ACTIONS(325), 1,
      anon_sym_default,
    ACTIONS(327), 1,
      anon_sym_extra,
    ACTIONS(331), 1,
      anon_sym_RBRACE,
    STATE(78), 1,
      sym_extra_block,
    STATE(51), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [1478] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(333), 1,
      anon_sym_RBRACE,
    ACTIONS(341), 1,
      anon_sym_importance,
    ACTIONS(344), 1,
      anon_sym_read_limit,
    ACTIONS(335), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(56), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(338), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [1504] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(150), 1,
      sym_tier_alias_name,
    ACTIONS(347), 7,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
      sym_identifier,
  [1520] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(349), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1534] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(351), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1548] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(28), 1,
      sym_tier_alias_name,
    ACTIONS(353), 7,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
      sym_identifier,
  [1564] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(355), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1578] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(357), 8,
      anon_sym_RBRACE,
      anon_sym_read_ns,
      anon_sym_write_ns,
      anon_sym_importance,
      anon_sym_staleness_sources,
      anon_sym_read_query,
      anon_sym_read_limit,
      anon_sym_write_content,
  [1592] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(359), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1606] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(361), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1620] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(363), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1634] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(365), 8,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
      anon_sym_id,
  [1648] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(367), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1662] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(369), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1676] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(371), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1690] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(373), 1,
      anon_sym_RBRACE,
    STATE(116), 1,
      sym__string_value,
    STATE(70), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(375), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1709] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(378), 1,
      anon_sym_RBRACE,
    STATE(79), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(380), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1726] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(382), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1739] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(384), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1752] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(73), 1,
      sym_tier_value,
    ACTIONS(386), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [1767] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(388), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1780] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(390), 1,
      anon_sym_RBRACE,
    STATE(71), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(380), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1797] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(392), 1,
      anon_sym_RBRACE,
    STATE(116), 1,
      sym__string_value,
    STATE(83), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(394), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1816] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(396), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1829] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(398), 1,
      anon_sym_RBRACE,
    STATE(79), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(400), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1846] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(403), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1859] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(405), 1,
      anon_sym_RBRACE,
    STATE(84), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(380), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1876] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(407), 1,
      anon_sym_LBRACE,
    ACTIONS(409), 1,
      anon_sym_agent,
    ACTIONS(411), 1,
      anon_sym_command,
    STATE(35), 4,
      sym__until_condition,
      sym_until_verify,
      sym_until_agent,
      sym_until_command,
  [1895] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(413), 1,
      anon_sym_RBRACE,
    STATE(116), 1,
      sym__string_value,
    STATE(70), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(394), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1914] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(415), 1,
      anon_sym_RBRACE,
    STATE(79), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(380), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1931] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(417), 1,
      anon_sym_RBRACE,
    ACTIONS(421), 1,
      anon_sym_impact_scope,
    ACTIONS(419), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(93), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1949] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(423), 1,
      anon_sym_RBRACE,
    ACTIONS(427), 1,
      anon_sym_depth,
    ACTIONS(425), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(91), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1967] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(429), 1,
      anon_sym_RBRACE,
    ACTIONS(434), 1,
      anon_sym_impact_scope,
    ACTIONS(431), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(87), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1985] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(427), 1,
      anon_sym_depth,
    ACTIONS(437), 1,
      anon_sym_RBRACE,
    ACTIONS(425), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(86), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [2003] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(439), 1,
      anon_sym_RBRACE,
    ACTIONS(443), 1,
      anon_sym_effort,
    ACTIONS(441), 2,
      anon_sym_model,
      anon_sym_id,
    STATE(92), 2,
      sym_reviewer_field,
      aux_sym_reviewer_entry_repeat1,
  [2021] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(443), 1,
      anon_sym_effort,
    ACTIONS(445), 1,
      anon_sym_RBRACE,
    ACTIONS(441), 2,
      anon_sym_model,
      anon_sym_id,
    STATE(89), 2,
      sym_reviewer_field,
      aux_sym_reviewer_entry_repeat1,
  [2039] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(447), 1,
      anon_sym_RBRACE,
    ACTIONS(452), 1,
      anon_sym_depth,
    ACTIONS(449), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(91), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [2057] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(455), 1,
      anon_sym_RBRACE,
    ACTIONS(460), 1,
      anon_sym_effort,
    ACTIONS(457), 2,
      anon_sym_model,
      anon_sym_id,
    STATE(92), 2,
      sym_reviewer_field,
      aux_sym_reviewer_entry_repeat1,
  [2075] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(421), 1,
      anon_sym_impact_scope,
    ACTIONS(463), 1,
      anon_sym_RBRACE,
    ACTIONS(419), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(87), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [2093] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(465), 1,
      anon_sym_RBRACK,
    ACTIONS(467), 2,
      sym_string,
      sym_raw_string,
    STATE(99), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [2108] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(469), 1,
      anon_sym_RBRACK,
    ACTIONS(471), 2,
      sym_string,
      sym_raw_string,
    STATE(95), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [2123] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(474), 1,
      anon_sym_loop,
    ACTIONS(476), 1,
      anon_sym_RBRACK,
    ACTIONS(478), 1,
      sym_identifier,
    STATE(100), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [2140] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(474), 1,
      anon_sym_loop,
    ACTIONS(480), 1,
      anon_sym_RBRACK,
    ACTIONS(482), 1,
      sym_identifier,
    STATE(96), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [2157] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(484), 5,
      anon_sym_RBRACE,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [2168] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(486), 1,
      anon_sym_RBRACK,
    ACTIONS(488), 2,
      sym_string,
      sym_raw_string,
    STATE(95), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [2183] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(490), 1,
      anon_sym_loop,
    ACTIONS(493), 1,
      anon_sym_RBRACK,
    ACTIONS(495), 1,
      sym_identifier,
    STATE(100), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [2200] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(34), 1,
      sym_consensus_mode_value,
    ACTIONS(498), 3,
      anon_sym_strict,
      anon_sym_partial_ok,
      anon_sym_explore,
  [2212] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(500), 1,
      anon_sym_LBRACE,
    ACTIONS(502), 1,
      anon_sym_RBRACK,
    STATE(108), 2,
      sym_reviewer_entry,
      aux_sym_reviewer_list_repeat1,
  [2226] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(504), 1,
      anon_sym_RBRACE,
    ACTIONS(506), 1,
      sym_identifier,
    STATE(110), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [2240] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(508), 4,
      anon_sym_RBRACE,
      anon_sym_callers_of,
      anon_sym_tests_for,
      anon_sym_depth,
  [2250] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(500), 1,
      anon_sym_LBRACE,
    ACTIONS(510), 1,
      anon_sym_RBRACK,
    STATE(102), 2,
      sym_reviewer_entry,
      aux_sym_reviewer_list_repeat1,
  [2264] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(512), 4,
      anon_sym_RBRACE,
      anon_sym_owned,
      anon_sym_read_only,
      anon_sym_impact_scope,
  [2274] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(28), 1,
      sym__string_value,
    ACTIONS(514), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [2286] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(516), 1,
      anon_sym_LBRACE,
    ACTIONS(519), 1,
      anon_sym_RBRACK,
    STATE(108), 2,
      sym_reviewer_entry,
      aux_sym_reviewer_list_repeat1,
  [2300] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(506), 1,
      sym_identifier,
    ACTIONS(521), 1,
      anon_sym_RBRACE,
    STATE(103), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [2314] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(523), 1,
      anon_sym_RBRACE,
    ACTIONS(525), 1,
      sym_identifier,
    STATE(110), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [2328] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(528), 4,
      anon_sym_RBRACE,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_id,
  [2338] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(46), 1,
      sym_strategy_value,
    ACTIONS(530), 3,
      anon_sym_single_pass,
      anon_sym_refine,
      sym_identifier,
  [2350] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(532), 4,
      anon_sym_RBRACE,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [2360] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(34), 1,
      sym_boolean,
    ACTIONS(534), 2,
      anon_sym_true,
      anon_sym_false,
  [2371] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(28), 1,
      sym__string_value,
    ACTIONS(514), 2,
      sym_string,
      sym_raw_string,
  [2382] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(113), 1,
      sym__string_value,
    ACTIONS(536), 2,
      sym_string,
      sym_raw_string,
  [2393] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(64), 1,
      sym__string_value,
    ACTIONS(538), 2,
      sym_string,
      sym_raw_string,
  [2404] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(140), 1,
      sym__string_value,
    ACTIONS(540), 2,
      sym_string,
      sym_raw_string,
  [2415] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(98), 1,
      sym_boolean,
    ACTIONS(534), 2,
      anon_sym_true,
      anon_sym_false,
  [2426] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(542), 1,
      anon_sym_RBRACK,
    ACTIONS(544), 1,
      sym_identifier,
    STATE(120), 1,
      aux_sym_identifier_list_repeat1,
  [2439] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(547), 1,
      anon_sym_RBRACK,
    ACTIONS(549), 1,
      sym_identifier,
    STATE(120), 1,
      aux_sym_identifier_list_repeat1,
  [2452] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(553), 1,
      anon_sym_RBRACK,
    ACTIONS(551), 2,
      anon_sym_loop,
      sym_identifier,
  [2463] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(229), 1,
      anon_sym_LBRACK,
    ACTIONS(555), 1,
      sym_identifier,
    STATE(34), 1,
      sym_reviewer_list,
  [2476] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(24), 1,
      sym__string_value,
    ACTIONS(557), 2,
      sym_string,
      sym_raw_string,
  [2487] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(73), 1,
      sym__string_value,
    ACTIONS(265), 2,
      sym_string,
      sym_raw_string,
  [2498] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(561), 1,
      anon_sym_RBRACK,
    ACTIONS(559), 2,
      anon_sym_loop,
      sym_identifier,
  [2509] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(62), 1,
      sym__string_value,
    ACTIONS(563), 2,
      sym_string,
      sym_raw_string,
  [2520] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(34), 1,
      sym_branch_chain_value,
    ACTIONS(565), 2,
      anon_sym_stacked,
      anon_sym_none,
  [2531] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(28), 1,
      sym_boolean,
    ACTIONS(534), 2,
      anon_sym_true,
      anon_sym_false,
  [2542] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(567), 1,
      anon_sym_RBRACK,
    ACTIONS(569), 1,
      sym_identifier,
    STATE(121), 1,
      aux_sym_identifier_list_repeat1,
  [2555] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(73), 1,
      sym_privacy_value,
    ACTIONS(571), 2,
      anon_sym_public,
      anon_sym_local_only,
  [2566] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(63), 1,
      sym__string_value,
    ACTIONS(573), 2,
      sym_string,
      sym_raw_string,
  [2577] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(106), 1,
      sym_boolean,
    ACTIONS(534), 2,
      anon_sym_true,
      anon_sym_false,
  [2588] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(46), 1,
      sym_boolean,
    ACTIONS(534), 2,
      anon_sym_true,
      anon_sym_false,
  [2599] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(111), 1,
      sym__string_value,
    ACTIONS(261), 2,
      sym_string,
      sym_raw_string,
  [2610] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(575), 1,
      anon_sym_LBRACK,
    STATE(106), 1,
      sym_string_list,
  [2620] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(575), 1,
      anon_sym_LBRACK,
    STATE(104), 1,
      sym_string_list,
  [2630] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(577), 1,
      anon_sym_LBRACK,
    STATE(34), 1,
      sym_identifier_list,
  [2640] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(579), 1,
      anon_sym_LBRACK,
    STATE(46), 1,
      sym_step_list,
  [2650] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(581), 2,
      anon_sym_RBRACE,
      sym_identifier,
  [2658] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(583), 2,
      anon_sym_LBRACE,
      anon_sym_RBRACK,
  [2666] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(577), 1,
      anon_sym_LBRACK,
    STATE(28), 1,
      sym_identifier_list,
  [2676] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(575), 1,
      anon_sym_LBRACK,
    STATE(28), 1,
      sym_string_list,
  [2686] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(575), 1,
      anon_sym_LBRACK,
    STATE(62), 1,
      sym_string_list,
  [2696] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(585), 2,
      anon_sym_LBRACE,
      anon_sym_RBRACK,
  [2704] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(587), 1,
      sym_identifier,
  [2711] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(589), 1,
      anon_sym_LBRACE,
  [2718] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(257), 1,
      sym_identifier,
  [2725] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(591), 1,
      anon_sym_LBRACE,
  [2732] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(593), 1,
      sym_identifier,
  [2739] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(514), 1,
      sym_identifier,
  [2746] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(595), 1,
      sym_identifier,
  [2753] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(597), 1,
      sym_integer,
  [2760] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(599), 1,
      anon_sym_LBRACE,
  [2767] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(601), 1,
      anon_sym_LBRACE,
  [2774] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(555), 1,
      sym_integer,
  [2781] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(514), 1,
      sym_integer,
  [2788] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(603), 1,
      anon_sym_LBRACE,
  [2795] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(605), 1,
      sym_identifier,
  [2802] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(607), 1,
      sym_identifier,
  [2809] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(609), 1,
      anon_sym_LBRACE,
  [2816] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(611), 1,
      anon_sym_LBRACE,
  [2823] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(563), 1,
      sym_float,
  [2830] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(563), 1,
      sym_integer,
  [2837] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(613), 1,
      sym_identifier,
  [2844] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(615), 1,
      anon_sym_LBRACE,
  [2851] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(617), 1,
      anon_sym_LBRACE,
  [2858] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(555), 1,
      sym_identifier,
  [2865] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(619), 1,
      sym_integer,
  [2872] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(621), 1,
      ts_builtin_sym_end,
  [2879] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(623), 1,
      sym_identifier,
  [2886] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(625), 1,
      anon_sym_LBRACE,
};

static const uint32_t ts_small_parse_table_map[] = {
  [SMALL_STATE(2)] = 0,
  [SMALL_STATE(3)] = 48,
  [SMALL_STATE(4)] = 80,
  [SMALL_STATE(5)] = 112,
  [SMALL_STATE(6)] = 145,
  [SMALL_STATE(7)] = 178,
  [SMALL_STATE(8)] = 207,
  [SMALL_STATE(9)] = 236,
  [SMALL_STATE(10)] = 263,
  [SMALL_STATE(11)] = 290,
  [SMALL_STATE(12)] = 343,
  [SMALL_STATE(13)] = 396,
  [SMALL_STATE(14)] = 449,
  [SMALL_STATE(15)] = 472,
  [SMALL_STATE(16)] = 511,
  [SMALL_STATE(17)] = 534,
  [SMALL_STATE(18)] = 573,
  [SMALL_STATE(19)] = 613,
  [SMALL_STATE(20)] = 653,
  [SMALL_STATE(21)] = 693,
  [SMALL_STATE(22)] = 735,
  [SMALL_STATE(23)] = 777,
  [SMALL_STATE(24)] = 819,
  [SMALL_STATE(25)] = 838,
  [SMALL_STATE(26)] = 857,
  [SMALL_STATE(27)] = 880,
  [SMALL_STATE(28)] = 899,
  [SMALL_STATE(29)] = 918,
  [SMALL_STATE(30)] = 937,
  [SMALL_STATE(31)] = 956,
  [SMALL_STATE(32)] = 975,
  [SMALL_STATE(33)] = 994,
  [SMALL_STATE(34)] = 1013,
  [SMALL_STATE(35)] = 1032,
  [SMALL_STATE(36)] = 1051,
  [SMALL_STATE(37)] = 1070,
  [SMALL_STATE(38)] = 1089,
  [SMALL_STATE(39)] = 1108,
  [SMALL_STATE(40)] = 1127,
  [SMALL_STATE(41)] = 1146,
  [SMALL_STATE(42)] = 1170,
  [SMALL_STATE(43)] = 1194,
  [SMALL_STATE(44)] = 1211,
  [SMALL_STATE(45)] = 1228,
  [SMALL_STATE(46)] = 1245,
  [SMALL_STATE(47)] = 1262,
  [SMALL_STATE(48)] = 1279,
  [SMALL_STATE(49)] = 1296,
  [SMALL_STATE(50)] = 1313,
  [SMALL_STATE(51)] = 1330,
  [SMALL_STATE(52)] = 1362,
  [SMALL_STATE(53)] = 1388,
  [SMALL_STATE(54)] = 1420,
  [SMALL_STATE(55)] = 1446,
  [SMALL_STATE(56)] = 1478,
  [SMALL_STATE(57)] = 1504,
  [SMALL_STATE(58)] = 1520,
  [SMALL_STATE(59)] = 1534,
  [SMALL_STATE(60)] = 1548,
  [SMALL_STATE(61)] = 1564,
  [SMALL_STATE(62)] = 1578,
  [SMALL_STATE(63)] = 1592,
  [SMALL_STATE(64)] = 1606,
  [SMALL_STATE(65)] = 1620,
  [SMALL_STATE(66)] = 1634,
  [SMALL_STATE(67)] = 1648,
  [SMALL_STATE(68)] = 1662,
  [SMALL_STATE(69)] = 1676,
  [SMALL_STATE(70)] = 1690,
  [SMALL_STATE(71)] = 1709,
  [SMALL_STATE(72)] = 1726,
  [SMALL_STATE(73)] = 1739,
  [SMALL_STATE(74)] = 1752,
  [SMALL_STATE(75)] = 1767,
  [SMALL_STATE(76)] = 1780,
  [SMALL_STATE(77)] = 1797,
  [SMALL_STATE(78)] = 1816,
  [SMALL_STATE(79)] = 1829,
  [SMALL_STATE(80)] = 1846,
  [SMALL_STATE(81)] = 1859,
  [SMALL_STATE(82)] = 1876,
  [SMALL_STATE(83)] = 1895,
  [SMALL_STATE(84)] = 1914,
  [SMALL_STATE(85)] = 1931,
  [SMALL_STATE(86)] = 1949,
  [SMALL_STATE(87)] = 1967,
  [SMALL_STATE(88)] = 1985,
  [SMALL_STATE(89)] = 2003,
  [SMALL_STATE(90)] = 2021,
  [SMALL_STATE(91)] = 2039,
  [SMALL_STATE(92)] = 2057,
  [SMALL_STATE(93)] = 2075,
  [SMALL_STATE(94)] = 2093,
  [SMALL_STATE(95)] = 2108,
  [SMALL_STATE(96)] = 2123,
  [SMALL_STATE(97)] = 2140,
  [SMALL_STATE(98)] = 2157,
  [SMALL_STATE(99)] = 2168,
  [SMALL_STATE(100)] = 2183,
  [SMALL_STATE(101)] = 2200,
  [SMALL_STATE(102)] = 2212,
  [SMALL_STATE(103)] = 2226,
  [SMALL_STATE(104)] = 2240,
  [SMALL_STATE(105)] = 2250,
  [SMALL_STATE(106)] = 2264,
  [SMALL_STATE(107)] = 2274,
  [SMALL_STATE(108)] = 2286,
  [SMALL_STATE(109)] = 2300,
  [SMALL_STATE(110)] = 2314,
  [SMALL_STATE(111)] = 2328,
  [SMALL_STATE(112)] = 2338,
  [SMALL_STATE(113)] = 2350,
  [SMALL_STATE(114)] = 2360,
  [SMALL_STATE(115)] = 2371,
  [SMALL_STATE(116)] = 2382,
  [SMALL_STATE(117)] = 2393,
  [SMALL_STATE(118)] = 2404,
  [SMALL_STATE(119)] = 2415,
  [SMALL_STATE(120)] = 2426,
  [SMALL_STATE(121)] = 2439,
  [SMALL_STATE(122)] = 2452,
  [SMALL_STATE(123)] = 2463,
  [SMALL_STATE(124)] = 2476,
  [SMALL_STATE(125)] = 2487,
  [SMALL_STATE(126)] = 2498,
  [SMALL_STATE(127)] = 2509,
  [SMALL_STATE(128)] = 2520,
  [SMALL_STATE(129)] = 2531,
  [SMALL_STATE(130)] = 2542,
  [SMALL_STATE(131)] = 2555,
  [SMALL_STATE(132)] = 2566,
  [SMALL_STATE(133)] = 2577,
  [SMALL_STATE(134)] = 2588,
  [SMALL_STATE(135)] = 2599,
  [SMALL_STATE(136)] = 2610,
  [SMALL_STATE(137)] = 2620,
  [SMALL_STATE(138)] = 2630,
  [SMALL_STATE(139)] = 2640,
  [SMALL_STATE(140)] = 2650,
  [SMALL_STATE(141)] = 2658,
  [SMALL_STATE(142)] = 2666,
  [SMALL_STATE(143)] = 2676,
  [SMALL_STATE(144)] = 2686,
  [SMALL_STATE(145)] = 2696,
  [SMALL_STATE(146)] = 2704,
  [SMALL_STATE(147)] = 2711,
  [SMALL_STATE(148)] = 2718,
  [SMALL_STATE(149)] = 2725,
  [SMALL_STATE(150)] = 2732,
  [SMALL_STATE(151)] = 2739,
  [SMALL_STATE(152)] = 2746,
  [SMALL_STATE(153)] = 2753,
  [SMALL_STATE(154)] = 2760,
  [SMALL_STATE(155)] = 2767,
  [SMALL_STATE(156)] = 2774,
  [SMALL_STATE(157)] = 2781,
  [SMALL_STATE(158)] = 2788,
  [SMALL_STATE(159)] = 2795,
  [SMALL_STATE(160)] = 2802,
  [SMALL_STATE(161)] = 2809,
  [SMALL_STATE(162)] = 2816,
  [SMALL_STATE(163)] = 2823,
  [SMALL_STATE(164)] = 2830,
  [SMALL_STATE(165)] = 2837,
  [SMALL_STATE(166)] = 2844,
  [SMALL_STATE(167)] = 2851,
  [SMALL_STATE(168)] = 2858,
  [SMALL_STATE(169)] = 2865,
  [SMALL_STATE(170)] = 2872,
  [SMALL_STATE(171)] = 2879,
  [SMALL_STATE(172)] = 2886,
};

static const TSParseActionEntry ts_parse_actions[] = {
  [0] = {.entry = {.count = 0, .reusable = false}},
  [1] = {.entry = {.count = 1, .reusable = false}}, RECOVER(),
  [3] = {.entry = {.count = 1, .reusable = true}}, SHIFT_EXTRA(),
  [5] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 0, 0, 0),
  [7] = {.entry = {.count = 1, .reusable = true}}, SHIFT(117),
  [9] = {.entry = {.count = 1, .reusable = true}}, SHIFT(171),
  [11] = {.entry = {.count = 1, .reusable = true}}, SHIFT(57),
  [13] = {.entry = {.count = 1, .reusable = true}}, SHIFT(172),
  [15] = {.entry = {.count = 1, .reusable = true}}, SHIFT(152),
  [17] = {.entry = {.count = 1, .reusable = true}}, SHIFT(146),
  [19] = {.entry = {.count = 1, .reusable = true}}, SHIFT(160),
  [21] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_boolean, 1, 0, 0),
  [23] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_boolean, 1, 0, 0),
  [25] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 2, 0, 0),
  [27] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 3, 0, 0),
  [29] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 3, 0, 0),
  [31] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_identifier_list, 3, 0, 0),
  [33] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 2, 0, 0),
  [35] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_identifier_list, 2, 0, 0),
  [37] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_list, 2, 0, 0),
  [39] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_list, 3, 0, 0),
  [41] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_block, 4, 0, 0),
  [43] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_block, 3, 0, 0),
  [45] = {.entry = {.count = 1, .reusable = true}}, SHIFT(151),
  [47] = {.entry = {.count = 1, .reusable = true}}, SHIFT(68),
  [49] = {.entry = {.count = 1, .reusable = true}}, SHIFT(60),
  [51] = {.entry = {.count = 1, .reusable = true}}, SHIFT(107),
  [53] = {.entry = {.count = 1, .reusable = true}}, SHIFT(115),
  [55] = {.entry = {.count = 1, .reusable = true}}, SHIFT(142),
  [57] = {.entry = {.count = 1, .reusable = true}}, SHIFT(157),
  [59] = {.entry = {.count = 1, .reusable = true}}, SHIFT(143),
  [61] = {.entry = {.count = 1, .reusable = true}}, SHIFT(129),
  [63] = {.entry = {.count = 1, .reusable = true}}, SHIFT(162),
  [65] = {.entry = {.count = 1, .reusable = true}}, SHIFT(166),
  [67] = {.entry = {.count = 1, .reusable = true}}, SHIFT(167),
  [69] = {.entry = {.count = 1, .reusable = true}}, SHIFT(67),
  [71] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(151),
  [74] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0),
  [76] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(60),
  [79] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(172),
  [82] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(107),
  [85] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(115),
  [88] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(142),
  [91] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(157),
  [94] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(143),
  [97] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(129),
  [100] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(162),
  [103] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(166),
  [106] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(167),
  [109] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_block, 3, 0, 0),
  [111] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0),
  [113] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(117),
  [116] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(171),
  [119] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(57),
  [122] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(172),
  [125] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(152),
  [128] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(146),
  [131] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(160),
  [134] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_block, 4, 0, 0),
  [136] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 1, 0, 0),
  [138] = {.entry = {.count = 1, .reusable = true}}, SHIFT(58),
  [140] = {.entry = {.count = 1, .reusable = true}}, SHIFT(153),
  [142] = {.entry = {.count = 1, .reusable = true}}, SHIFT(149),
  [144] = {.entry = {.count = 1, .reusable = true}}, SHIFT(139),
  [146] = {.entry = {.count = 1, .reusable = true}}, SHIFT(112),
  [148] = {.entry = {.count = 1, .reusable = true}}, SHIFT(134),
  [150] = {.entry = {.count = 1, .reusable = true}}, SHIFT(165),
  [152] = {.entry = {.count = 1, .reusable = true}}, SHIFT(61),
  [154] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0),
  [156] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(153),
  [159] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(166),
  [162] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(149),
  [165] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(139),
  [168] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(112),
  [171] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(134),
  [174] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(165),
  [177] = {.entry = {.count = 1, .reusable = true}}, SHIFT(122),
  [179] = {.entry = {.count = 1, .reusable = true}}, SHIFT(138),
  [181] = {.entry = {.count = 1, .reusable = true}}, SHIFT(123),
  [183] = {.entry = {.count = 1, .reusable = true}}, SHIFT(168),
  [185] = {.entry = {.count = 1, .reusable = true}}, SHIFT(101),
  [187] = {.entry = {.count = 1, .reusable = true}}, SHIFT(156),
  [189] = {.entry = {.count = 1, .reusable = true}}, SHIFT(114),
  [191] = {.entry = {.count = 1, .reusable = true}}, SHIFT(128),
  [193] = {.entry = {.count = 1, .reusable = true}}, SHIFT(82),
  [195] = {.entry = {.count = 1, .reusable = true}}, SHIFT(126),
  [197] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0),
  [199] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(138),
  [202] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(123),
  [205] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(168),
  [208] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(101),
  [211] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(156),
  [214] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(114),
  [217] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(128),
  [220] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(82),
  [223] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_command, 2, 0, 0),
  [225] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 1, 0, 0),
  [227] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_param_declaration, 2, 0, 0),
  [229] = {.entry = {.count = 1, .reusable = true}}, SHIFT(105),
  [231] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 3, 0, 0),
  [233] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 2, 0, 0),
  [235] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_block, 3, 0, 0),
  [237] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 4, 0, 0),
  [239] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_verify, 2, 0, 0),
  [241] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_branch_chain_value, 1, 0, 0),
  [243] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_block, 4, 0, 0),
  [245] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_field, 2, 0, 0),
  [247] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_clause, 2, 0, 0),
  [249] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_field, 1, 0, 0),
  [251] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_consensus_mode_value, 1, 0, 0),
  [253] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_agent, 2, 0, 0),
  [255] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_verify, 3, 0, 0),
  [257] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_alias_name, 1, 0, 0),
  [259] = {.entry = {.count = 1, .reusable = false}}, SHIFT(66),
  [261] = {.entry = {.count = 1, .reusable = true}}, SHIFT(111),
  [263] = {.entry = {.count = 1, .reusable = false}}, SHIFT(111),
  [265] = {.entry = {.count = 1, .reusable = true}}, SHIFT(73),
  [267] = {.entry = {.count = 1, .reusable = false}}, SHIFT(73),
  [269] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_strategy_value, 1, 0, 0),
  [271] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_block, 3, 0, 0),
  [273] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 1, 0, 0),
  [275] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 2, 0, 0),
  [277] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_step_list, 2, 0, 0),
  [279] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_param_declaration, 3, 0, 0),
  [281] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_block, 4, 0, 0),
  [283] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_step_list, 3, 0, 0),
  [285] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0),
  [287] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(74),
  [290] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(125),
  [293] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(42),
  [296] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(131),
  [299] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(78),
  [302] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(158),
  [305] = {.entry = {.count = 1, .reusable = true}}, SHIFT(9),
  [307] = {.entry = {.count = 1, .reusable = true}}, SHIFT(144),
  [309] = {.entry = {.count = 1, .reusable = true}}, SHIFT(127),
  [311] = {.entry = {.count = 1, .reusable = true}}, SHIFT(163),
  [313] = {.entry = {.count = 1, .reusable = true}}, SHIFT(164),
  [315] = {.entry = {.count = 1, .reusable = true}}, SHIFT(65),
  [317] = {.entry = {.count = 1, .reusable = true}}, SHIFT(74),
  [319] = {.entry = {.count = 1, .reusable = true}}, SHIFT(125),
  [321] = {.entry = {.count = 1, .reusable = true}}, SHIFT(42),
  [323] = {.entry = {.count = 1, .reusable = true}}, SHIFT(131),
  [325] = {.entry = {.count = 1, .reusable = true}}, SHIFT(78),
  [327] = {.entry = {.count = 1, .reusable = true}}, SHIFT(158),
  [329] = {.entry = {.count = 1, .reusable = true}}, SHIFT(10),
  [331] = {.entry = {.count = 1, .reusable = true}}, SHIFT(69),
  [333] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0),
  [335] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(144),
  [338] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(127),
  [341] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(163),
  [344] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(164),
  [347] = {.entry = {.count = 1, .reusable = false}}, SHIFT(148),
  [349] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 4, 0, 0),
  [351] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_alias_declaration, 3, 0, 0),
  [353] = {.entry = {.count = 1, .reusable = false}}, SHIFT(40),
  [355] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 5, 0, 0),
  [357] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_field, 2, 0, 0),
  [359] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_prompt_declaration, 3, 0, 0),
  [361] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_include_declaration, 2, 0, 0),
  [363] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 4, 0, 0),
  [365] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_value, 1, 0, 0),
  [367] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 4, 0, 0),
  [369] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 5, 0, 0),
  [371] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 5, 0, 0),
  [373] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_extra_block_repeat1, 2, 0, 0),
  [375] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_extra_block_repeat1, 2, 0, 0), SHIFT_REPEAT(116),
  [378] = {.entry = {.count = 1, .reusable = true}}, SHIFT(49),
  [380] = {.entry = {.count = 1, .reusable = true}}, SHIFT(119),
  [382] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_privacy_value, 1, 0, 0),
  [384] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_field, 2, 0, 0),
  [386] = {.entry = {.count = 1, .reusable = true}}, SHIFT(66),
  [388] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_block, 4, 0, 0),
  [390] = {.entry = {.count = 1, .reusable = true}}, SHIFT(44),
  [392] = {.entry = {.count = 1, .reusable = true}}, SHIFT(80),
  [394] = {.entry = {.count = 1, .reusable = true}}, SHIFT(116),
  [396] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_field, 1, 0, 0),
  [398] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_verify_block_repeat1, 2, 0, 0),
  [400] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_verify_block_repeat1, 2, 0, 0), SHIFT_REPEAT(119),
  [403] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_block, 3, 0, 0),
  [405] = {.entry = {.count = 1, .reusable = true}}, SHIFT(31),
  [407] = {.entry = {.count = 1, .reusable = true}}, SHIFT(81),
  [409] = {.entry = {.count = 1, .reusable = true}}, SHIFT(159),
  [411] = {.entry = {.count = 1, .reusable = true}}, SHIFT(124),
  [413] = {.entry = {.count = 1, .reusable = true}}, SHIFT(75),
  [415] = {.entry = {.count = 1, .reusable = true}}, SHIFT(39),
  [417] = {.entry = {.count = 1, .reusable = true}}, SHIFT(27),
  [419] = {.entry = {.count = 1, .reusable = true}}, SHIFT(136),
  [421] = {.entry = {.count = 1, .reusable = true}}, SHIFT(133),
  [423] = {.entry = {.count = 1, .reusable = true}}, SHIFT(33),
  [425] = {.entry = {.count = 1, .reusable = true}}, SHIFT(137),
  [427] = {.entry = {.count = 1, .reusable = true}}, SHIFT(169),
  [429] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0),
  [431] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(136),
  [434] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(133),
  [437] = {.entry = {.count = 1, .reusable = true}}, SHIFT(29),
  [439] = {.entry = {.count = 1, .reusable = true}}, SHIFT(145),
  [441] = {.entry = {.count = 1, .reusable = true}}, SHIFT(135),
  [443] = {.entry = {.count = 1, .reusable = true}}, SHIFT(41),
  [445] = {.entry = {.count = 1, .reusable = true}}, SHIFT(141),
  [447] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0),
  [449] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0), SHIFT_REPEAT(137),
  [452] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0), SHIFT_REPEAT(169),
  [455] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_reviewer_entry_repeat1, 2, 0, 0),
  [457] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_reviewer_entry_repeat1, 2, 0, 0), SHIFT_REPEAT(135),
  [460] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_reviewer_entry_repeat1, 2, 0, 0), SHIFT_REPEAT(41),
  [463] = {.entry = {.count = 1, .reusable = true}}, SHIFT(30),
  [465] = {.entry = {.count = 1, .reusable = true}}, SHIFT(3),
  [467] = {.entry = {.count = 1, .reusable = true}}, SHIFT(99),
  [469] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0),
  [471] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0), SHIFT_REPEAT(95),
  [474] = {.entry = {.count = 1, .reusable = false}}, SHIFT(147),
  [476] = {.entry = {.count = 1, .reusable = true}}, SHIFT(50),
  [478] = {.entry = {.count = 1, .reusable = false}}, SHIFT(100),
  [480] = {.entry = {.count = 1, .reusable = true}}, SHIFT(47),
  [482] = {.entry = {.count = 1, .reusable = false}}, SHIFT(96),
  [484] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_field, 2, 0, 0),
  [486] = {.entry = {.count = 1, .reusable = true}}, SHIFT(4),
  [488] = {.entry = {.count = 1, .reusable = true}}, SHIFT(95),
  [490] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0), SHIFT_REPEAT(147),
  [493] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0),
  [495] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0), SHIFT_REPEAT(100),
  [498] = {.entry = {.count = 1, .reusable = true}}, SHIFT(37),
  [500] = {.entry = {.count = 1, .reusable = true}}, SHIFT(90),
  [502] = {.entry = {.count = 1, .reusable = true}}, SHIFT(8),
  [504] = {.entry = {.count = 1, .reusable = true}}, SHIFT(16),
  [506] = {.entry = {.count = 1, .reusable = true}}, SHIFT(118),
  [508] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_field, 2, 0, 0),
  [510] = {.entry = {.count = 1, .reusable = true}}, SHIFT(7),
  [512] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_field, 2, 0, 0),
  [514] = {.entry = {.count = 1, .reusable = true}}, SHIFT(28),
  [516] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_reviewer_list_repeat1, 2, 0, 0), SHIFT_REPEAT(90),
  [519] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_reviewer_list_repeat1, 2, 0, 0),
  [521] = {.entry = {.count = 1, .reusable = true}}, SHIFT(14),
  [523] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_vars_block_repeat1, 2, 0, 0),
  [525] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_vars_block_repeat1, 2, 0, 0), SHIFT_REPEAT(118),
  [528] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_field, 2, 0, 0),
  [530] = {.entry = {.count = 1, .reusable = false}}, SHIFT(43),
  [532] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_pair, 2, 0, 0),
  [534] = {.entry = {.count = 1, .reusable = true}}, SHIFT(2),
  [536] = {.entry = {.count = 1, .reusable = true}}, SHIFT(113),
  [538] = {.entry = {.count = 1, .reusable = true}}, SHIFT(64),
  [540] = {.entry = {.count = 1, .reusable = true}}, SHIFT(140),
  [542] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0),
  [544] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0), SHIFT_REPEAT(120),
  [547] = {.entry = {.count = 1, .reusable = true}}, SHIFT(5),
  [549] = {.entry = {.count = 1, .reusable = true}}, SHIFT(120),
  [551] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_loop_block, 3, 0, 0),
  [553] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_block, 3, 0, 0),
  [555] = {.entry = {.count = 1, .reusable = true}}, SHIFT(34),
  [557] = {.entry = {.count = 1, .reusable = true}}, SHIFT(24),
  [559] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_loop_block, 4, 0, 0),
  [561] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_block, 4, 0, 0),
  [563] = {.entry = {.count = 1, .reusable = true}}, SHIFT(62),
  [565] = {.entry = {.count = 1, .reusable = true}}, SHIFT(32),
  [567] = {.entry = {.count = 1, .reusable = true}}, SHIFT(6),
  [569] = {.entry = {.count = 1, .reusable = true}}, SHIFT(121),
  [571] = {.entry = {.count = 1, .reusable = true}}, SHIFT(72),
  [573] = {.entry = {.count = 1, .reusable = true}}, SHIFT(63),
  [575] = {.entry = {.count = 1, .reusable = true}}, SHIFT(94),
  [577] = {.entry = {.count = 1, .reusable = true}}, SHIFT(130),
  [579] = {.entry = {.count = 1, .reusable = true}}, SHIFT(97),
  [581] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_pair, 2, 0, 0),
  [583] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_entry, 2, 0, 0),
  [585] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_entry, 3, 0, 0),
  [587] = {.entry = {.count = 1, .reusable = true}}, SHIFT(154),
  [589] = {.entry = {.count = 1, .reusable = true}}, SHIFT(21),
  [591] = {.entry = {.count = 1, .reusable = true}}, SHIFT(76),
  [593] = {.entry = {.count = 1, .reusable = true}}, SHIFT(59),
  [595] = {.entry = {.count = 1, .reusable = true}}, SHIFT(132),
  [597] = {.entry = {.count = 1, .reusable = true}}, SHIFT(46),
  [599] = {.entry = {.count = 1, .reusable = true}}, SHIFT(12),
  [601] = {.entry = {.count = 1, .reusable = true}}, SHIFT(18),
  [603] = {.entry = {.count = 1, .reusable = true}}, SHIFT(77),
  [605] = {.entry = {.count = 1, .reusable = true}}, SHIFT(38),
  [607] = {.entry = {.count = 1, .reusable = true}}, SHIFT(155),
  [609] = {.entry = {.count = 1, .reusable = true}}, SHIFT(53),
  [611] = {.entry = {.count = 1, .reusable = true}}, SHIFT(85),
  [613] = {.entry = {.count = 1, .reusable = true}}, SHIFT(26),
  [615] = {.entry = {.count = 1, .reusable = true}}, SHIFT(54),
  [617] = {.entry = {.count = 1, .reusable = true}}, SHIFT(88),
  [619] = {.entry = {.count = 1, .reusable = true}}, SHIFT(104),
  [621] = {.entry = {.count = 1, .reusable = true}},  ACCEPT_INPUT(),
  [623] = {.entry = {.count = 1, .reusable = true}}, SHIFT(161),
  [625] = {.entry = {.count = 1, .reusable = true}}, SHIFT(109),
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
