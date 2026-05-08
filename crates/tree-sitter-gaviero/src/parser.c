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
#define STATE_COUNT 152
#define LARGE_STATE_COUNT 2
#define SYMBOL_COUNT 133
#define ALIAS_COUNT 0
#define TOKEN_COUNT 78
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
  anon_sym_scope = 25,
  anon_sym_owned = 26,
  anon_sym_read_only = 27,
  anon_sym_impact_scope = 28,
  anon_sym_memory = 29,
  anon_sym_read_ns = 30,
  anon_sym_write_ns = 31,
  anon_sym_importance = 32,
  anon_sym_staleness_sources = 33,
  anon_sym_read_query = 34,
  anon_sym_read_limit = 35,
  anon_sym_write_content = 36,
  anon_sym_verify = 37,
  anon_sym_compile = 38,
  anon_sym_clippy = 39,
  anon_sym_test = 40,
  anon_sym_impact_tests = 41,
  anon_sym_context = 42,
  anon_sym_callers_of = 43,
  anon_sym_tests_for = 44,
  anon_sym_depth = 45,
  anon_sym_loop = 46,
  anon_sym_agents = 47,
  anon_sym_max_iterations = 48,
  anon_sym_iter_start = 49,
  anon_sym_stability = 50,
  anon_sym_judge_timeout = 51,
  anon_sym_strict_judge = 52,
  anon_sym_branch_chain = 53,
  anon_sym_stacked = 54,
  anon_sym_none = 55,
  anon_sym_until = 56,
  anon_sym_command = 57,
  anon_sym_workflow = 58,
  anon_sym_steps = 59,
  anon_sym_max_parallel = 60,
  anon_sym_strategy = 61,
  anon_sym_test_first = 62,
  anon_sym_attempts = 63,
  anon_sym_escalate_after = 64,
  anon_sym_LBRACK = 65,
  anon_sym_RBRACK = 66,
  anon_sym_public = 67,
  anon_sym_local_only = 68,
  anon_sym_single_pass = 69,
  anon_sym_refine = 70,
  anon_sym_true = 71,
  anon_sym_false = 72,
  sym_string = 73,
  sym_raw_string = 74,
  sym_float = 75,
  sym_integer = 76,
  sym_identifier = 77,
  sym_source_file = 78,
  sym__definition = 79,
  sym_include_declaration = 80,
  sym_client_declaration = 81,
  sym_client_field = 82,
  sym__effort_value = 83,
  sym_extra_block = 84,
  sym_extra_pair = 85,
  sym_vars_block = 86,
  sym_vars_pair = 87,
  sym_tier_alias_declaration = 88,
  sym_tier_alias_name = 89,
  sym_prompt_declaration = 90,
  sym_agent_declaration = 91,
  sym_agent_field = 92,
  sym_scope_block = 93,
  sym_scope_field = 94,
  sym_memory_block = 95,
  sym_memory_field = 96,
  sym_verify_block = 97,
  sym_verify_field = 98,
  sym_context_block = 99,
  sym_context_field = 100,
  sym_loop_block = 101,
  sym_loop_field = 102,
  sym_branch_chain_value = 103,
  sym_until_clause = 104,
  sym__until_condition = 105,
  sym_until_verify = 106,
  sym_until_agent = 107,
  sym_until_command = 108,
  sym_workflow_declaration = 109,
  sym_workflow_field = 110,
  sym_step_list = 111,
  sym_string_list = 112,
  sym_identifier_list = 113,
  sym_tier_value = 114,
  sym_privacy_value = 115,
  sym_strategy_value = 116,
  sym_boolean = 117,
  sym__string_value = 118,
  aux_sym_source_file_repeat1 = 119,
  aux_sym_client_declaration_repeat1 = 120,
  aux_sym_extra_block_repeat1 = 121,
  aux_sym_vars_block_repeat1 = 122,
  aux_sym_agent_declaration_repeat1 = 123,
  aux_sym_scope_block_repeat1 = 124,
  aux_sym_memory_block_repeat1 = 125,
  aux_sym_verify_block_repeat1 = 126,
  aux_sym_context_block_repeat1 = 127,
  aux_sym_loop_block_repeat1 = 128,
  aux_sym_workflow_declaration_repeat1 = 129,
  aux_sym_step_list_repeat1 = 130,
  aux_sym_string_list_repeat1 = 131,
  aux_sym_identifier_list_repeat1 = 132,
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
  [anon_sym_max_iterations] = "max_iterations",
  [anon_sym_iter_start] = "iter_start",
  [anon_sym_stability] = "stability",
  [anon_sym_judge_timeout] = "judge_timeout",
  [anon_sym_strict_judge] = "strict_judge",
  [anon_sym_branch_chain] = "branch_chain",
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
  [anon_sym_LBRACK] = "[",
  [anon_sym_RBRACK] = "]",
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
  [anon_sym_max_iterations] = anon_sym_max_iterations,
  [anon_sym_iter_start] = anon_sym_iter_start,
  [anon_sym_stability] = anon_sym_stability,
  [anon_sym_judge_timeout] = anon_sym_judge_timeout,
  [anon_sym_strict_judge] = anon_sym_strict_judge,
  [anon_sym_branch_chain] = anon_sym_branch_chain,
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
  [anon_sym_LBRACK] = anon_sym_LBRACK,
  [anon_sym_RBRACK] = anon_sym_RBRACK,
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
  [anon_sym_LBRACK] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_RBRACK] = {
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
  [140] = 29,
  [141] = 141,
  [142] = 142,
  [143] = 143,
  [144] = 144,
  [145] = 145,
  [146] = 146,
  [147] = 147,
  [148] = 148,
  [149] = 149,
  [150] = 150,
  [151] = 151,
};

static bool ts_lex(TSLexer *lexer, TSStateId state) {
  START_LEXER();
  eof = lexer->eof(lexer);
  switch (state) {
    case 0:
      if (eof) ADVANCE(438);
      ADVANCE_MAP(
        '"', 3,
        '#', 4,
        '/', 13,
        '[', 512,
        ']', 513,
        'a', 171,
        'b', 320,
        'c', 33,
        'd', 107,
        'e', 161,
        'f', 42,
        'i', 234,
        'j', 412,
        'l', 274,
        'm', 35,
        'n', 277,
        'o', 424,
        'p', 314,
        'r', 108,
        's', 76,
        't', 109,
        'u', 250,
        'v', 41,
        'w', 279,
        '{', 442,
        '}', 443,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(0);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(525);
      END_STATE();
    case 1:
      if (lookahead == '"') ADVANCE(3);
      if (lookahead == '#') ADVANCE(4);
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == ']') ADVANCE(513);
      if (lookahead == '}') ADVANCE(443);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(1);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 2:
      if (lookahead == '"') ADVANCE(3);
      if (lookahead == '#') ADVANCE(4);
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'c') ADVANCE(550);
      if (lookahead == 'e') ADVANCE(588);
      if (lookahead == 'm') ADVANCE(538);
      if (lookahead == 'r') ADVANCE(539);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(2);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 3:
      if (lookahead == '"') ADVANCE(522);
      if (lookahead == '\\') ADVANCE(437);
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
      if (lookahead == '#') ADVANCE(523);
      if (lookahead != 0) ADVANCE(5);
      END_STATE();
    case 7:
      if (lookahead == '.') ADVANCE(436);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(7);
      END_STATE();
    case 8:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == ']') ADVANCE(513);
      if (lookahead == 'l') ADVANCE(573);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(8);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 9:
      ADVANCE_MAP(
        '/', 13,
        'a', 176,
        'b', 320,
        'c', 34,
        'd', 121,
        'e', 340,
        'i', 245,
        'j', 412,
        'm', 36,
        'o', 424,
        'p', 334,
        'r', 143,
        's', 77,
        't', 156,
        'u', 250,
        'v', 41,
        'w', 321,
        '}', 443,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(9);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(526);
      END_STATE();
    case 10:
      ADVANCE_MAP(
        '/', 13,
        'a', 176,
        'b', 320,
        'c', 229,
        'e', 340,
        'i', 237,
        'j', 412,
        'm', 36,
        'o', 424,
        'r', 146,
        's', 385,
        't', 157,
        'u', 250,
        'v', 123,
        '}', 443,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(10);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(7);
      END_STATE();
    case 11:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'c') ADVANCE(229);
      if (lookahead == 'i') ADVANCE(244);
      if (lookahead == 't') ADVANCE(159);
      if (lookahead == '}') ADVANCE(443);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(11);
      END_STATE();
    case 12:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'r') ADVANCE(542);
      if (lookahead == 's') ADVANCE(556);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(12);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 13:
      if (lookahead == '/') ADVANCE(440);
      END_STATE();
    case 14:
      if (lookahead == '_') ADVANCE(203);
      END_STATE();
    case 15:
      if (lookahead == '_') ADVANCE(226);
      END_STATE();
    case 16:
      if (lookahead == '_') ADVANCE(82);
      END_STATE();
    case 17:
      if (lookahead == '_') ADVANCE(356);
      END_STATE();
    case 18:
      if (lookahead == '_') ADVANCE(208);
      END_STATE();
    case 19:
      if (lookahead == '_') ADVANCE(167);
      if (lookahead == 's') ADVANCE(32);
      END_STATE();
    case 20:
      if (lookahead == '_') ADVANCE(281);
      END_STATE();
    case 21:
      if (lookahead == '_') ADVANCE(87);
      END_STATE();
    case 22:
      if (lookahead == '_') ADVANCE(307);
      END_STATE();
    case 23:
      if (lookahead == '_') ADVANCE(48);
      END_STATE();
    case 24:
      if (lookahead == '_') ADVANCE(354);
      END_STATE();
    case 25:
      if (lookahead == '_') ADVANCE(355);
      END_STATE();
    case 26:
      if (lookahead == '_') ADVANCE(359);
      END_STATE();
    case 27:
      if (lookahead == '_') ADVANCE(295);
      END_STATE();
    case 28:
      if (lookahead == '_') ADVANCE(396);
      END_STATE();
    case 29:
      if (lookahead == '_') ADVANCE(287);
      END_STATE();
    case 30:
      if (lookahead == '_') ADVANCE(284);
      END_STATE();
    case 31:
      if (lookahead == '_') ADVANCE(403);
      END_STATE();
    case 32:
      if (lookahead == '_') ADVANCE(168);
      END_STATE();
    case 33:
      if (lookahead == 'a') ADVANCE(216);
      if (lookahead == 'h') ADVANCE(124);
      if (lookahead == 'l') ADVANCE(181);
      if (lookahead == 'o') ADVANCE(235);
      END_STATE();
    case 34:
      if (lookahead == 'a') ADVANCE(216);
      if (lookahead == 'l') ADVANCE(207);
      if (lookahead == 'o') ADVANCE(271);
      END_STATE();
    case 35:
      if (lookahead == 'a') ADVANCE(425);
      if (lookahead == 'e') ADVANCE(74);
      if (lookahead == 'o') ADVANCE(101);
      END_STATE();
    case 36:
      if (lookahead == 'a') ADVANCE(425);
      if (lookahead == 'e') ADVANCE(242);
      END_STATE();
    case 37:
      if (lookahead == 'a') ADVANCE(100);
      if (lookahead == 'f') ADVANCE(200);
      END_STATE();
    case 38:
      if (lookahead == 'a') ADVANCE(72);
      if (lookahead == 'e') ADVANCE(302);
      if (lookahead == 'r') ADVANCE(64);
      END_STATE();
    case 39:
      if (lookahead == 'a') ADVANCE(449);
      END_STATE();
    case 40:
      if (lookahead == 'a') ADVANCE(73);
      if (lookahead == 'e') ADVANCE(302);
      if (lookahead == 'r') ADVANCE(64);
      END_STATE();
    case 41:
      if (lookahead == 'a') ADVANCE(325);
      if (lookahead == 'e') ADVANCE(327);
      END_STATE();
    case 42:
      if (lookahead == 'a') ADVANCE(215);
      END_STATE();
    case 43:
      if (lookahead == 'a') ADVANCE(71);
      if (lookahead == 'e') ADVANCE(302);
      if (lookahead == 'r') ADVANCE(64);
      END_STATE();
    case 44:
      if (lookahead == 'a') ADVANCE(413);
      END_STATE();
    case 45:
      if (lookahead == 'a') ADVANCE(255);
      END_STATE();
    case 46:
      if (lookahead == 'a') ADVANCE(300);
      END_STATE();
    case 47:
      if (lookahead == 'a') ADVANCE(221);
      END_STATE();
    case 48:
      if (lookahead == 'a') ADVANCE(169);
      END_STATE();
    case 49:
      if (lookahead == 'a') ADVANCE(83);
      END_STATE();
    case 50:
      if (lookahead == 'a') ADVANCE(83);
      if (lookahead == 'o') ADVANCE(330);
      END_STATE();
    case 51:
      if (lookahead == 'a') ADVANCE(264);
      END_STATE();
    case 52:
      if (lookahead == 'a') ADVANCE(218);
      END_STATE();
    case 53:
      if (lookahead == 'a') ADVANCE(81);
      END_STATE();
    case 54:
      if (lookahead == 'a') ADVANCE(99);
      END_STATE();
    case 55:
      if (lookahead == 'a') ADVANCE(106);
      END_STATE();
    case 56:
      if (lookahead == 'a') ADVANCE(335);
      END_STATE();
    case 57:
      if (lookahead == 'a') ADVANCE(362);
      END_STATE();
    case 58:
      if (lookahead == 'a') ADVANCE(213);
      END_STATE();
    case 59:
      if (lookahead == 'a') ADVANCE(407);
      END_STATE();
    case 60:
      if (lookahead == 'a') ADVANCE(194);
      END_STATE();
    case 61:
      if (lookahead == 'a') ADVANCE(262);
      END_STATE();
    case 62:
      if (lookahead == 'a') ADVANCE(257);
      END_STATE();
    case 63:
      if (lookahead == 'a') ADVANCE(339);
      END_STATE();
    case 64:
      if (lookahead == 'a') ADVANCE(400);
      if (lookahead == 'i') ADVANCE(89);
      END_STATE();
    case 65:
      if (lookahead == 'a') ADVANCE(402);
      END_STATE();
    case 66:
      if (lookahead == 'a') ADVANCE(233);
      END_STATE();
    case 67:
      if (lookahead == 'a') ADVANCE(91);
      if (lookahead == 'o') ADVANCE(330);
      END_STATE();
    case 68:
      if (lookahead == 'a') ADVANCE(411);
      END_STATE();
    case 69:
      if (lookahead == 'a') ADVANCE(92);
      END_STATE();
    case 70:
      if (lookahead == 'b') ADVANCE(222);
      END_STATE();
    case 71:
      if (lookahead == 'b') ADVANCE(198);
      END_STATE();
    case 72:
      if (lookahead == 'b') ADVANCE(198);
      if (lookahead == 'c') ADVANCE(210);
      if (lookahead == 'l') ADVANCE(154);
      END_STATE();
    case 73:
      if (lookahead == 'b') ADVANCE(198);
      if (lookahead == 'l') ADVANCE(154);
      END_STATE();
    case 74:
      if (lookahead == 'c') ADVANCE(180);
      if (lookahead == 'm') ADVANCE(285);
      END_STATE();
    case 75:
      if (lookahead == 'c') ADVANCE(514);
      END_STATE();
    case 76:
      if (lookahead == 'c') ADVANCE(278);
      if (lookahead == 'i') ADVANCE(251);
      if (lookahead == 't') ADVANCE(38);
      END_STATE();
    case 77:
      if (lookahead == 'c') ADVANCE(278);
      if (lookahead == 't') ADVANCE(40);
      END_STATE();
    case 78:
      if (lookahead == 'c') ADVANCE(178);
      END_STATE();
    case 79:
      if (lookahead == 'c') ADVANCE(417);
      END_STATE();
    case 80:
      if (lookahead == 'c') ADVANCE(231);
      END_STATE();
    case 81:
      if (lookahead == 'c') ADVANCE(430);
      END_STATE();
    case 82:
      if (lookahead == 'c') ADVANCE(296);
      if (lookahead == 'n') ADVANCE(346);
      END_STATE();
    case 83:
      if (lookahead == 'c') ADVANCE(389);
      END_STATE();
    case 84:
      if (lookahead == 'c') ADVANCE(118);
      END_STATE();
    case 85:
      if (lookahead == 'c') ADVANCE(147);
      END_STATE();
    case 86:
      if (lookahead == 'c') ADVANCE(47);
      END_STATE();
    case 87:
      if (lookahead == 'c') ADVANCE(179);
      END_STATE();
    case 88:
      if (lookahead == 'c') ADVANCE(331);
      END_STATE();
    case 89:
      if (lookahead == 'c') ADVANCE(393);
      END_STATE();
    case 90:
      if (lookahead == 'c') ADVANCE(52);
      if (lookahead == 'o') ADVANCE(299);
      END_STATE();
    case 91:
      if (lookahead == 'c') ADVANCE(404);
      END_STATE();
    case 92:
      if (lookahead == 'c') ADVANCE(395);
      END_STATE();
    case 93:
      if (lookahead == 'c') ADVANCE(58);
      END_STATE();
    case 94:
      if (lookahead == 'c') ADVANCE(292);
      END_STATE();
    case 95:
      if (lookahead == 'd') ADVANCE(173);
      END_STATE();
    case 96:
      if (lookahead == 'd') ADVANCE(470);
      END_STATE();
    case 97:
      if (lookahead == 'd') ADVANCE(504);
      END_STATE();
    case 98:
      if (lookahead == 'd') ADVANCE(501);
      END_STATE();
    case 99:
      if (lookahead == 'd') ADVANCE(15);
      END_STATE();
    case 100:
      if (lookahead == 'd') ADVANCE(15);
      if (lookahead == 's') ADVANCE(294);
      END_STATE();
    case 101:
      if (lookahead == 'd') ADVANCE(135);
      END_STATE();
    case 102:
      if (lookahead == 'd') ADVANCE(187);
      END_STATE();
    case 103:
      if (lookahead == 'd') ADVANCE(369);
      END_STATE();
    case 104:
      if (lookahead == 'd') ADVANCE(116);
      END_STATE();
    case 105:
      if (lookahead == 'd') ADVANCE(174);
      END_STATE();
    case 106:
      if (lookahead == 'd') ADVANCE(30);
      END_STATE();
    case 107:
      if (lookahead == 'e') ADVANCE(164);
      END_STATE();
    case 108:
      if (lookahead == 'e') ADVANCE(37);
      END_STATE();
    case 109:
      if (lookahead == 'e') ADVANCE(353);
      if (lookahead == 'i') ADVANCE(132);
      if (lookahead == 'o') ADVANCE(291);
      if (lookahead == 'r') ADVANCE(414);
      END_STATE();
    case 110:
      if (lookahead == 'e') ADVANCE(502);
      END_STATE();
    case 111:
      if (lookahead == 'e') ADVANCE(520);
      END_STATE();
    case 112:
      if (lookahead == 'e') ADVANCE(521);
      END_STATE();
    case 113:
      if (lookahead == 'e') ADVANCE(469);
      END_STATE();
    case 114:
      if (lookahead == 'e') ADVANCE(518);
      END_STATE();
    case 115:
      if (lookahead == 'e') ADVANCE(482);
      END_STATE();
    case 116:
      if (lookahead == 'e') ADVANCE(439);
      END_STATE();
    case 117:
      if (lookahead == 'e') ADVANCE(453);
      END_STATE();
    case 118:
      if (lookahead == 'e') ADVANCE(476);
      END_STATE();
    case 119:
      if (lookahead == 'e') ADVANCE(472);
      END_STATE();
    case 120:
      if (lookahead == 'e') ADVANCE(499);
      END_STATE();
    case 121:
      if (lookahead == 'e') ADVANCE(297);
      END_STATE();
    case 122:
      if (lookahead == 'e') ADVANCE(426);
      END_STATE();
    case 123:
      if (lookahead == 'e') ADVANCE(327);
      END_STATE();
    case 124:
      if (lookahead == 'e') ADVANCE(46);
      END_STATE();
    case 125:
      if (lookahead == 'e') ADVANCE(172);
      END_STATE();
    case 126:
      if (lookahead == 'e') ADVANCE(79);
      if (lookahead == 'p') ADVANCE(131);
      if (lookahead == 't') ADVANCE(326);
      END_STATE();
    case 127:
      if (lookahead == 'e') ADVANCE(96);
      END_STATE();
    case 128:
      if (lookahead == 'e') ADVANCE(28);
      END_STATE();
    case 129:
      if (lookahead == 'e') ADVANCE(256);
      if (lookahead == 't') ADVANCE(177);
      END_STATE();
    case 130:
      if (lookahead == 'e') ADVANCE(322);
      END_STATE();
    case 131:
      if (lookahead == 'e') ADVANCE(258);
      END_STATE();
    case 132:
      if (lookahead == 'e') ADVANCE(316);
      END_STATE();
    case 133:
      if (lookahead == 'e') ADVANCE(98);
      END_STATE();
    case 134:
      if (lookahead == 'e') ADVANCE(16);
      END_STATE();
    case 135:
      if (lookahead == 'e') ADVANCE(211);
      END_STATE();
    case 136:
      if (lookahead == 'e') ADVANCE(280);
      END_STATE();
    case 137:
      if (lookahead == 'e') ADVANCE(22);
      END_STATE();
    case 138:
      if (lookahead == 'e') ADVANCE(367);
      END_STATE();
    case 139:
      if (lookahead == 'e') ADVANCE(332);
      END_STATE();
    case 140:
      if (lookahead == 'e') ADVANCE(23);
      END_STATE();
    case 141:
      if (lookahead == 'e') ADVANCE(405);
      END_STATE();
    case 142:
      if (lookahead == 'e') ADVANCE(347);
      END_STATE();
    case 143:
      if (lookahead == 'e') ADVANCE(54);
      END_STATE();
    case 144:
      if (lookahead == 'e') ADVANCE(214);
      END_STATE();
    case 145:
      if (lookahead == 'e') ADVANCE(319);
      END_STATE();
    case 146:
      if (lookahead == 'e') ADVANCE(55);
      END_STATE();
    case 147:
      if (lookahead == 'e') ADVANCE(351);
      END_STATE();
    case 148:
      if (lookahead == 'e') ADVANCE(252);
      END_STATE();
    case 149:
      if (lookahead == 'e') ADVANCE(329);
      END_STATE();
    case 150:
      if (lookahead == 'e') ADVANCE(254);
      END_STATE();
    case 151:
      if (lookahead == 'e') ADVANCE(254);
      if (lookahead == 'p') ADVANCE(301);
      END_STATE();
    case 152:
      if (lookahead == 'e') ADVANCE(333);
      END_STATE();
    case 153:
      if (lookahead == 'e') ADVANCE(368);
      END_STATE();
    case 154:
      if (lookahead == 'e') ADVANCE(270);
      END_STATE();
    case 155:
      if (lookahead == 'e') ADVANCE(269);
      END_STATE();
    case 156:
      if (lookahead == 'e') ADVANCE(364);
      if (lookahead == 'i') ADVANCE(132);
      if (lookahead == 'o') ADVANCE(291);
      END_STATE();
    case 157:
      if (lookahead == 'e') ADVANCE(365);
      END_STATE();
    case 158:
      if (lookahead == 'e') ADVANCE(272);
      END_STATE();
    case 159:
      if (lookahead == 'e') ADVANCE(366);
      END_STATE();
    case 160:
      if (lookahead == 'e') ADVANCE(243);
      END_STATE();
    case 161:
      if (lookahead == 'f') ADVANCE(165);
      if (lookahead == 's') ADVANCE(86);
      if (lookahead == 'x') ADVANCE(126);
      END_STATE();
    case 162:
      if (lookahead == 'f') ADVANCE(489);
      END_STATE();
    case 163:
      if (lookahead == 'f') ADVANCE(429);
      END_STATE();
    case 164:
      if (lookahead == 'f') ADVANCE(44);
      if (lookahead == 'p') ADVANCE(129);
      if (lookahead == 's') ADVANCE(88);
      END_STATE();
    case 165:
      if (lookahead == 'f') ADVANCE(283);
      END_STATE();
    case 166:
      if (lookahead == 'f') ADVANCE(219);
      END_STATE();
    case 167:
      if (lookahead == 'f') ADVANCE(192);
      END_STATE();
    case 168:
      if (lookahead == 'f') ADVANCE(289);
      END_STATE();
    case 169:
      if (lookahead == 'f') ADVANCE(406);
      END_STATE();
    case 170:
      if (lookahead == 'g') ADVANCE(457);
      END_STATE();
    case 171:
      if (lookahead == 'g') ADVANCE(148);
      if (lookahead == 't') ADVANCE(387);
      END_STATE();
    case 172:
      if (lookahead == 'g') ADVANCE(431);
      END_STATE();
    case 173:
      if (lookahead == 'g') ADVANCE(128);
      END_STATE();
    case 174:
      if (lookahead == 'g') ADVANCE(120);
      END_STATE();
    case 175:
      if (lookahead == 'g') ADVANCE(228);
      END_STATE();
    case 176:
      if (lookahead == 'g') ADVANCE(158);
      if (lookahead == 't') ADVANCE(387);
      END_STATE();
    case 177:
      if (lookahead == 'h') ADVANCE(491);
      END_STATE();
    case 178:
      if (lookahead == 'h') ADVANCE(21);
      END_STATE();
    case 179:
      if (lookahead == 'h') ADVANCE(60);
      END_STATE();
    case 180:
      if (lookahead == 'h') ADVANCE(51);
      END_STATE();
    case 181:
      if (lookahead == 'i') ADVANCE(151);
      END_STATE();
    case 182:
      if (lookahead == 'i') ADVANCE(421);
      if (lookahead == 'o') ADVANCE(236);
      END_STATE();
    case 183:
      if (lookahead == 'i') ADVANCE(163);
      END_STATE();
    case 184:
      if (lookahead == 'i') ADVANCE(422);
      END_STATE();
    case 185:
      if (lookahead == 'i') ADVANCE(239);
      END_STATE();
    case 186:
      if (lookahead == 'i') ADVANCE(240);
      END_STATE();
    case 187:
      if (lookahead == 'i') ADVANCE(260);
      END_STATE();
    case 188:
      if (lookahead == 'i') ADVANCE(75);
      END_STATE();
    case 189:
      if (lookahead == 'i') ADVANCE(311);
      END_STATE();
    case 190:
      if (lookahead == 'i') ADVANCE(212);
      END_STATE();
    case 191:
      if (lookahead == 'i') ADVANCE(253);
      END_STATE();
    case 192:
      if (lookahead == 'i') ADVANCE(338);
      END_STATE();
    case 193:
      if (lookahead == 'i') ADVANCE(304);
      END_STATE();
    case 194:
      if (lookahead == 'i') ADVANCE(249);
      END_STATE();
    case 195:
      if (lookahead == 'i') ADVANCE(388);
      END_STATE();
    case 196:
      if (lookahead == 'i') ADVANCE(378);
      END_STATE();
    case 197:
      if (lookahead == 'i') ADVANCE(142);
      END_STATE();
    case 198:
      if (lookahead == 'i') ADVANCE(227);
      END_STATE();
    case 199:
      if (lookahead == 'i') ADVANCE(398);
      END_STATE();
    case 200:
      if (lookahead == 'i') ADVANCE(268);
      END_STATE();
    case 201:
      if (lookahead == 'i') ADVANCE(230);
      END_STATE();
    case 202:
      if (lookahead == 'i') ADVANCE(286);
      END_STATE();
    case 203:
      if (lookahead == 'i') ADVANCE(401);
      if (lookahead == 'p') ADVANCE(63);
      if (lookahead == 'r') ADVANCE(141);
      END_STATE();
    case 204:
      if (lookahead == 'i') ADVANCE(288);
      END_STATE();
    case 205:
      if (lookahead == 'i') ADVANCE(293);
      END_STATE();
    case 206:
      if (lookahead == 'i') ADVANCE(93);
      END_STATE();
    case 207:
      if (lookahead == 'i') ADVANCE(150);
      END_STATE();
    case 208:
      if (lookahead == 'j') ADVANCE(420);
      END_STATE();
    case 209:
      if (lookahead == 'k') ADVANCE(166);
      END_STATE();
    case 210:
      if (lookahead == 'k') ADVANCE(133);
      END_STATE();
    case 211:
      if (lookahead == 'l') ADVANCE(445);
      END_STATE();
    case 212:
      if (lookahead == 'l') ADVANCE(503);
      END_STATE();
    case 213:
      if (lookahead == 'l') ADVANCE(461);
      END_STATE();
    case 214:
      if (lookahead == 'l') ADVANCE(507);
      END_STATE();
    case 215:
      if (lookahead == 'l') ADVANCE(360);
      END_STATE();
    case 216:
      if (lookahead == 'l') ADVANCE(224);
      END_STATE();
    case 217:
      if (lookahead == 'l') ADVANCE(343);
      END_STATE();
    case 218:
      if (lookahead == 'l') ADVANCE(27);
      END_STATE();
    case 219:
      if (lookahead == 'l') ADVANCE(276);
      END_STATE();
    case 220:
      if (lookahead == 'l') ADVANCE(432);
      END_STATE();
    case 221:
      if (lookahead == 'l') ADVANCE(65);
      END_STATE();
    case 222:
      if (lookahead == 'l') ADVANCE(188);
      END_STATE();
    case 223:
      if (lookahead == 'l') ADVANCE(434);
      END_STATE();
    case 224:
      if (lookahead == 'l') ADVANCE(149);
      END_STATE();
    case 225:
      if (lookahead == 'l') ADVANCE(376);
      END_STATE();
    case 226:
      if (lookahead == 'l') ADVANCE(185);
      if (lookahead == 'n') ADVANCE(344);
      if (lookahead == 'o') ADVANCE(263);
      if (lookahead == 'q') ADVANCE(419);
      END_STATE();
    case 227:
      if (lookahead == 'l') ADVANCE(195);
      END_STATE();
    case 228:
      if (lookahead == 'l') ADVANCE(137);
      END_STATE();
    case 229:
      if (lookahead == 'l') ADVANCE(193);
      if (lookahead == 'o') ADVANCE(238);
      END_STATE();
    case 230:
      if (lookahead == 'l') ADVANCE(115);
      END_STATE();
    case 231:
      if (lookahead == 'l') ADVANCE(418);
      END_STATE();
    case 232:
      if (lookahead == 'l') ADVANCE(144);
      END_STATE();
    case 233:
      if (lookahead == 'l') ADVANCE(232);
      END_STATE();
    case 234:
      if (lookahead == 'm') ADVANCE(298);
      if (lookahead == 'n') ADVANCE(80);
      if (lookahead == 't') ADVANCE(130);
      END_STATE();
    case 235:
      if (lookahead == 'm') ADVANCE(241);
      if (lookahead == 'n') ADVANCE(394);
      if (lookahead == 'o') ADVANCE(324);
      END_STATE();
    case 236:
      if (lookahead == 'm') ADVANCE(305);
      END_STATE();
    case 237:
      if (lookahead == 'm') ADVANCE(310);
      if (lookahead == 't') ADVANCE(130);
      END_STATE();
    case 238:
      if (lookahead == 'm') ADVANCE(303);
      END_STATE();
    case 239:
      if (lookahead == 'm') ADVANCE(196);
      END_STATE();
    case 240:
      if (lookahead == 'm') ADVANCE(136);
      END_STATE();
    case 241:
      if (lookahead == 'm') ADVANCE(62);
      if (lookahead == 'p') ADVANCE(201);
      END_STATE();
    case 242:
      if (lookahead == 'm') ADVANCE(285);
      END_STATE();
    case 243:
      if (lookahead == 'm') ADVANCE(306);
      END_STATE();
    case 244:
      if (lookahead == 'm') ADVANCE(312);
      END_STATE();
    case 245:
      if (lookahead == 'm') ADVANCE(313);
      if (lookahead == 't') ADVANCE(130);
      END_STATE();
    case 246:
      if (lookahead == 'n') ADVANCE(459);
      END_STATE();
    case 247:
      if (lookahead == 'n') ADVANCE(466);
      END_STATE();
    case 248:
      if (lookahead == 'n') ADVANCE(465);
      END_STATE();
    case 249:
      if (lookahead == 'n') ADVANCE(500);
      END_STATE();
    case 250:
      if (lookahead == 'n') ADVANCE(386);
      END_STATE();
    case 251:
      if (lookahead == 'n') ADVANCE(175);
      END_STATE();
    case 252:
      if (lookahead == 'n') ADVANCE(371);
      END_STATE();
    case 253:
      if (lookahead == 'n') ADVANCE(170);
      END_STATE();
    case 254:
      if (lookahead == 'n') ADVANCE(372);
      END_STATE();
    case 255:
      if (lookahead == 'n') ADVANCE(78);
      END_STATE();
    case 256:
      if (lookahead == 'n') ADVANCE(103);
      END_STATE();
    case 257:
      if (lookahead == 'n') ADVANCE(97);
      END_STATE();
    case 258:
      if (lookahead == 'n') ADVANCE(357);
      END_STATE();
    case 259:
      if (lookahead == 'n') ADVANCE(110);
      END_STATE();
    case 260:
      if (lookahead == 'n') ADVANCE(59);
      END_STATE();
    case 261:
      if (lookahead == 'n') ADVANCE(127);
      END_STATE();
    case 262:
      if (lookahead == 'n') ADVANCE(84);
      END_STATE();
    case 263:
      if (lookahead == 'n') ADVANCE(220);
      END_STATE();
    case 264:
      if (lookahead == 'n') ADVANCE(206);
      END_STATE();
    case 265:
      if (lookahead == 'n') ADVANCE(223);
      END_STATE();
    case 266:
      if (lookahead == 'n') ADVANCE(191);
      END_STATE();
    case 267:
      if (lookahead == 'n') ADVANCE(350);
      END_STATE();
    case 268:
      if (lookahead == 'n') ADVANCE(114);
      END_STATE();
    case 269:
      if (lookahead == 'n') ADVANCE(381);
      END_STATE();
    case 270:
      if (lookahead == 'n') ADVANCE(138);
      END_STATE();
    case 271:
      if (lookahead == 'n') ADVANCE(394);
      END_STATE();
    case 272:
      if (lookahead == 'n') ADVANCE(399);
      END_STATE();
    case 273:
      if (lookahead == 'n') ADVANCE(409);
      END_STATE();
    case 274:
      if (lookahead == 'o') ADVANCE(90);
      END_STATE();
    case 275:
      if (lookahead == 'o') ADVANCE(236);
      END_STATE();
    case 276:
      if (lookahead == 'o') ADVANCE(423);
      END_STATE();
    case 277:
      if (lookahead == 'o') ADVANCE(259);
      END_STATE();
    case 278:
      if (lookahead == 'o') ADVANCE(308);
      END_STATE();
    case 279:
      if (lookahead == 'o') ADVANCE(315);
      if (lookahead == 'r') ADVANCE(199);
      END_STATE();
    case 280:
      if (lookahead == 'o') ADVANCE(416);
      END_STATE();
    case 281:
      if (lookahead == 'o') ADVANCE(162);
      END_STATE();
    case 282:
      if (lookahead == 'o') ADVANCE(415);
      END_STATE();
    case 283:
      if (lookahead == 'o') ADVANCE(328);
      END_STATE();
    case 284:
      if (lookahead == 'o') ADVANCE(263);
      END_STATE();
    case 285:
      if (lookahead == 'o') ADVANCE(323);
      END_STATE();
    case 286:
      if (lookahead == 'o') ADVANCE(246);
      END_STATE();
    case 287:
      if (lookahead == 'o') ADVANCE(247);
      END_STATE();
    case 288:
      if (lookahead == 'o') ADVANCE(248);
      END_STATE();
    case 289:
      if (lookahead == 'o') ADVANCE(317);
      END_STATE();
    case 290:
      if (lookahead == 'o') ADVANCE(318);
      END_STATE();
    case 291:
      if (lookahead == 'o') ADVANCE(217);
      END_STATE();
    case 292:
      if (lookahead == 'o') ADVANCE(309);
      END_STATE();
    case 293:
      if (lookahead == 'o') ADVANCE(267);
      END_STATE();
    case 294:
      if (lookahead == 'o') ADVANCE(266);
      END_STATE();
    case 295:
      if (lookahead == 'o') ADVANCE(265);
      END_STATE();
    case 296:
      if (lookahead == 'o') ADVANCE(273);
      END_STATE();
    case 297:
      if (lookahead == 'p') ADVANCE(129);
      if (lookahead == 's') ADVANCE(88);
      END_STATE();
    case 298:
      if (lookahead == 'p') ADVANCE(50);
      END_STATE();
    case 299:
      if (lookahead == 'p') ADVANCE(492);
      END_STATE();
    case 300:
      if (lookahead == 'p') ADVANCE(451);
      END_STATE();
    case 301:
      if (lookahead == 'p') ADVANCE(427);
      END_STATE();
    case 302:
      if (lookahead == 'p') ADVANCE(342);
      END_STATE();
    case 303:
      if (lookahead == 'p') ADVANCE(201);
      END_STATE();
    case 304:
      if (lookahead == 'p') ADVANCE(301);
      END_STATE();
    case 305:
      if (lookahead == 'p') ADVANCE(374);
      END_STATE();
    case 306:
      if (lookahead == 'p') ADVANCE(392);
      END_STATE();
    case 307:
      if (lookahead == 'p') ADVANCE(57);
      END_STATE();
    case 308:
      if (lookahead == 'p') ADVANCE(113);
      END_STATE();
    case 309:
      if (lookahead == 'p') ADVANCE(119);
      END_STATE();
    case 310:
      if (lookahead == 'p') ADVANCE(49);
      END_STATE();
    case 311:
      if (lookahead == 'p') ADVANCE(410);
      END_STATE();
    case 312:
      if (lookahead == 'p') ADVANCE(69);
      END_STATE();
    case 313:
      if (lookahead == 'p') ADVANCE(67);
      END_STATE();
    case 314:
      if (lookahead == 'r') ADVANCE(182);
      if (lookahead == 'u') ADVANCE(70);
      END_STATE();
    case 315:
      if (lookahead == 'r') ADVANCE(209);
      END_STATE();
    case 316:
      if (lookahead == 'r') ADVANCE(444);
      END_STATE();
    case 317:
      if (lookahead == 'r') ADVANCE(490);
      END_STATE();
    case 318:
      if (lookahead == 'r') ADVANCE(455);
      END_STATE();
    case 319:
      if (lookahead == 'r') ADVANCE(511);
      END_STATE();
    case 320:
      if (lookahead == 'r') ADVANCE(45);
      END_STATE();
    case 321:
      if (lookahead == 'r') ADVANCE(199);
      END_STATE();
    case 322:
      if (lookahead == 'r') ADVANCE(26);
      END_STATE();
    case 323:
      if (lookahead == 'r') ADVANCE(428);
      END_STATE();
    case 324:
      if (lookahead == 'r') ADVANCE(102);
      END_STATE();
    case 325:
      if (lookahead == 'r') ADVANCE(341);
      END_STATE();
    case 326:
      if (lookahead == 'r') ADVANCE(39);
      END_STATE();
    case 327:
      if (lookahead == 'r') ADVANCE(183);
      END_STATE();
    case 328:
      if (lookahead == 'r') ADVANCE(373);
      END_STATE();
    case 329:
      if (lookahead == 'r') ADVANCE(358);
      END_STATE();
    case 330:
      if (lookahead == 'r') ADVANCE(408);
      END_STATE();
    case 331:
      if (lookahead == 'r') ADVANCE(189);
      END_STATE();
    case 332:
      if (lookahead == 'r') ADVANCE(435);
      END_STATE();
    case 333:
      if (lookahead == 'r') ADVANCE(68);
      END_STATE();
    case 334:
      if (lookahead == 'r') ADVANCE(275);
      END_STATE();
    case 335:
      if (lookahead == 'r') ADVANCE(377);
      END_STATE();
    case 336:
      if (lookahead == 'r') ADVANCE(197);
      END_STATE();
    case 337:
      if (lookahead == 'r') ADVANCE(85);
      END_STATE();
    case 338:
      if (lookahead == 'r') ADVANCE(363);
      END_STATE();
    case 339:
      if (lookahead == 'r') ADVANCE(66);
      END_STATE();
    case 340:
      if (lookahead == 's') ADVANCE(86);
      END_STATE();
    case 341:
      if (lookahead == 's') ADVANCE(450);
      END_STATE();
    case 342:
      if (lookahead == 's') ADVANCE(506);
      END_STATE();
    case 343:
      if (lookahead == 's') ADVANCE(468);
      END_STATE();
    case 344:
      if (lookahead == 's') ADVANCE(474);
      END_STATE();
    case 345:
      if (lookahead == 's') ADVANCE(510);
      END_STATE();
    case 346:
      if (lookahead == 's') ADVANCE(475);
      END_STATE();
    case 347:
      if (lookahead == 's') ADVANCE(467);
      END_STATE();
    case 348:
      if (lookahead == 's') ADVANCE(516);
      END_STATE();
    case 349:
      if (lookahead == 's') ADVANCE(487);
      END_STATE();
    case 350:
      if (lookahead == 's') ADVANCE(495);
      END_STATE();
    case 351:
      if (lookahead == 's') ADVANCE(477);
      END_STATE();
    case 352:
      if (lookahead == 's') ADVANCE(494);
      END_STATE();
    case 353:
      if (lookahead == 's') ADVANCE(370);
      END_STATE();
    case 354:
      if (lookahead == 's') ADVANCE(282);
      END_STATE();
    case 355:
      if (lookahead == 's') ADVANCE(94);
      END_STATE();
    case 356:
      if (lookahead == 's') ADVANCE(94);
      if (lookahead == 't') ADVANCE(153);
      END_STATE();
    case 357:
      if (lookahead == 's') ADVANCE(184);
      END_STATE();
    case 358:
      if (lookahead == 's') ADVANCE(20);
      END_STATE();
    case 359:
      if (lookahead == 's') ADVANCE(390);
      END_STATE();
    case 360:
      if (lookahead == 's') ADVANCE(112);
      END_STATE();
    case 361:
      if (lookahead == 's') ADVANCE(24);
      END_STATE();
    case 362:
      if (lookahead == 's') ADVANCE(348);
      END_STATE();
    case 363:
      if (lookahead == 's') ADVANCE(379);
      END_STATE();
    case 364:
      if (lookahead == 's') ADVANCE(382);
      END_STATE();
    case 365:
      if (lookahead == 's') ADVANCE(383);
      END_STATE();
    case 366:
      if (lookahead == 's') ADVANCE(384);
      END_STATE();
    case 367:
      if (lookahead == 's') ADVANCE(361);
      END_STATE();
    case 368:
      if (lookahead == 's') ADVANCE(397);
      END_STATE();
    case 369:
      if (lookahead == 's') ADVANCE(29);
      END_STATE();
    case 370:
      if (lookahead == 't') ADVANCE(486);
      END_STATE();
    case 371:
      if (lookahead == 't') ADVANCE(464);
      END_STATE();
    case 372:
      if (lookahead == 't') ADVANCE(441);
      END_STATE();
    case 373:
      if (lookahead == 't') ADVANCE(446);
      END_STATE();
    case 374:
      if (lookahead == 't') ADVANCE(463);
      END_STATE();
    case 375:
      if (lookahead == 't') ADVANCE(488);
      END_STATE();
    case 376:
      if (lookahead == 't') ADVANCE(448);
      END_STATE();
    case 377:
      if (lookahead == 't') ADVANCE(496);
      END_STATE();
    case 378:
      if (lookahead == 't') ADVANCE(479);
      END_STATE();
    case 379:
      if (lookahead == 't') ADVANCE(509);
      END_STATE();
    case 380:
      if (lookahead == 't') ADVANCE(498);
      END_STATE();
    case 381:
      if (lookahead == 't') ADVANCE(480);
      END_STATE();
    case 382:
      if (lookahead == 't') ADVANCE(19);
      END_STATE();
    case 383:
      if (lookahead == 't') ADVANCE(485);
      END_STATE();
    case 384:
      if (lookahead == 't') ADVANCE(484);
      END_STATE();
    case 385:
      if (lookahead == 't') ADVANCE(43);
      END_STATE();
    case 386:
      if (lookahead == 't') ADVANCE(190);
      END_STATE();
    case 387:
      if (lookahead == 't') ADVANCE(160);
      END_STATE();
    case 388:
      if (lookahead == 't') ADVANCE(433);
      END_STATE();
    case 389:
      if (lookahead == 't') ADVANCE(17);
      END_STATE();
    case 390:
      if (lookahead == 't') ADVANCE(56);
      END_STATE();
    case 391:
      if (lookahead == 't') ADVANCE(202);
      END_STATE();
    case 392:
      if (lookahead == 't') ADVANCE(345);
      END_STATE();
    case 393:
      if (lookahead == 't') ADVANCE(18);
      END_STATE();
    case 394:
      if (lookahead == 't') ADVANCE(122);
      END_STATE();
    case 395:
      if (lookahead == 't') ADVANCE(31);
      END_STATE();
    case 396:
      if (lookahead == 't') ADVANCE(186);
      END_STATE();
    case 397:
      if (lookahead == 't') ADVANCE(349);
      END_STATE();
    case 398:
      if (lookahead == 't') ADVANCE(134);
      END_STATE();
    case 399:
      if (lookahead == 't') ADVANCE(352);
      END_STATE();
    case 400:
      if (lookahead == 't') ADVANCE(125);
      END_STATE();
    case 401:
      if (lookahead == 't') ADVANCE(152);
      END_STATE();
    case 402:
      if (lookahead == 't') ADVANCE(140);
      END_STATE();
    case 403:
      if (lookahead == 't') ADVANCE(153);
      END_STATE();
    case 404:
      if (lookahead == 't') ADVANCE(25);
      END_STATE();
    case 405:
      if (lookahead == 't') ADVANCE(336);
      END_STATE();
    case 406:
      if (lookahead == 't') ADVANCE(145);
      END_STATE();
    case 407:
      if (lookahead == 't') ADVANCE(290);
      END_STATE();
    case 408:
      if (lookahead == 't') ADVANCE(61);
      END_STATE();
    case 409:
      if (lookahead == 't') ADVANCE(155);
      END_STATE();
    case 410:
      if (lookahead == 't') ADVANCE(204);
      END_STATE();
    case 411:
      if (lookahead == 't') ADVANCE(205);
      END_STATE();
    case 412:
      if (lookahead == 'u') ADVANCE(95);
      END_STATE();
    case 413:
      if (lookahead == 'u') ADVANCE(225);
      END_STATE();
    case 414:
      if (lookahead == 'u') ADVANCE(111);
      END_STATE();
    case 415:
      if (lookahead == 'u') ADVANCE(337);
      END_STATE();
    case 416:
      if (lookahead == 'u') ADVANCE(380);
      END_STATE();
    case 417:
      if (lookahead == 'u') ADVANCE(391);
      END_STATE();
    case 418:
      if (lookahead == 'u') ADVANCE(104);
      END_STATE();
    case 419:
      if (lookahead == 'u') ADVANCE(139);
      END_STATE();
    case 420:
      if (lookahead == 'u') ADVANCE(105);
      END_STATE();
    case 421:
      if (lookahead == 'v') ADVANCE(53);
      END_STATE();
    case 422:
      if (lookahead == 'v') ADVANCE(117);
      END_STATE();
    case 423:
      if (lookahead == 'w') ADVANCE(505);
      END_STATE();
    case 424:
      if (lookahead == 'w') ADVANCE(261);
      END_STATE();
    case 425:
      if (lookahead == 'x') ADVANCE(14);
      END_STATE();
    case 426:
      if (lookahead == 'x') ADVANCE(375);
      END_STATE();
    case 427:
      if (lookahead == 'y') ADVANCE(483);
      END_STATE();
    case 428:
      if (lookahead == 'y') ADVANCE(473);
      END_STATE();
    case 429:
      if (lookahead == 'y') ADVANCE(481);
      END_STATE();
    case 430:
      if (lookahead == 'y') ADVANCE(447);
      END_STATE();
    case 431:
      if (lookahead == 'y') ADVANCE(508);
      END_STATE();
    case 432:
      if (lookahead == 'y') ADVANCE(471);
      END_STATE();
    case 433:
      if (lookahead == 'y') ADVANCE(497);
      END_STATE();
    case 434:
      if (lookahead == 'y') ADVANCE(515);
      END_STATE();
    case 435:
      if (lookahead == 'y') ADVANCE(478);
      END_STATE();
    case 436:
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(524);
      END_STATE();
    case 437:
      if (lookahead != 0 &&
          lookahead != '\n') ADVANCE(3);
      END_STATE();
    case 438:
      ACCEPT_TOKEN(ts_builtin_sym_end);
      END_STATE();
    case 439:
      ACCEPT_TOKEN(anon_sym_include);
      END_STATE();
    case 440:
      ACCEPT_TOKEN(sym_comment);
      if (lookahead != 0 &&
          lookahead != '\n') ADVANCE(440);
      END_STATE();
    case 441:
      ACCEPT_TOKEN(anon_sym_client);
      END_STATE();
    case 442:
      ACCEPT_TOKEN(anon_sym_LBRACE);
      END_STATE();
    case 443:
      ACCEPT_TOKEN(anon_sym_RBRACE);
      END_STATE();
    case 444:
      ACCEPT_TOKEN(anon_sym_tier);
      END_STATE();
    case 445:
      ACCEPT_TOKEN(anon_sym_model);
      END_STATE();
    case 446:
      ACCEPT_TOKEN(anon_sym_effort);
      END_STATE();
    case 447:
      ACCEPT_TOKEN(anon_sym_privacy);
      END_STATE();
    case 448:
      ACCEPT_TOKEN(anon_sym_default);
      END_STATE();
    case 449:
      ACCEPT_TOKEN(anon_sym_extra);
      END_STATE();
    case 450:
      ACCEPT_TOKEN(anon_sym_vars);
      END_STATE();
    case 451:
      ACCEPT_TOKEN(anon_sym_cheap);
      END_STATE();
    case 452:
      ACCEPT_TOKEN(anon_sym_cheap);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 453:
      ACCEPT_TOKEN(anon_sym_expensive);
      END_STATE();
    case 454:
      ACCEPT_TOKEN(anon_sym_expensive);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 455:
      ACCEPT_TOKEN(anon_sym_coordinator);
      END_STATE();
    case 456:
      ACCEPT_TOKEN(anon_sym_coordinator);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 457:
      ACCEPT_TOKEN(anon_sym_reasoning);
      END_STATE();
    case 458:
      ACCEPT_TOKEN(anon_sym_reasoning);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 459:
      ACCEPT_TOKEN(anon_sym_execution);
      END_STATE();
    case 460:
      ACCEPT_TOKEN(anon_sym_execution);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 461:
      ACCEPT_TOKEN(anon_sym_mechanical);
      END_STATE();
    case 462:
      ACCEPT_TOKEN(anon_sym_mechanical);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 463:
      ACCEPT_TOKEN(anon_sym_prompt);
      END_STATE();
    case 464:
      ACCEPT_TOKEN(anon_sym_agent);
      END_STATE();
    case 465:
      ACCEPT_TOKEN(anon_sym_description);
      END_STATE();
    case 466:
      ACCEPT_TOKEN(anon_sym_depends_on);
      END_STATE();
    case 467:
      ACCEPT_TOKEN(anon_sym_max_retries);
      END_STATE();
    case 468:
      ACCEPT_TOKEN(anon_sym_tools);
      END_STATE();
    case 469:
      ACCEPT_TOKEN(anon_sym_scope);
      END_STATE();
    case 470:
      ACCEPT_TOKEN(anon_sym_owned);
      END_STATE();
    case 471:
      ACCEPT_TOKEN(anon_sym_read_only);
      END_STATE();
    case 472:
      ACCEPT_TOKEN(anon_sym_impact_scope);
      END_STATE();
    case 473:
      ACCEPT_TOKEN(anon_sym_memory);
      END_STATE();
    case 474:
      ACCEPT_TOKEN(anon_sym_read_ns);
      END_STATE();
    case 475:
      ACCEPT_TOKEN(anon_sym_write_ns);
      END_STATE();
    case 476:
      ACCEPT_TOKEN(anon_sym_importance);
      END_STATE();
    case 477:
      ACCEPT_TOKEN(anon_sym_staleness_sources);
      END_STATE();
    case 478:
      ACCEPT_TOKEN(anon_sym_read_query);
      END_STATE();
    case 479:
      ACCEPT_TOKEN(anon_sym_read_limit);
      END_STATE();
    case 480:
      ACCEPT_TOKEN(anon_sym_write_content);
      END_STATE();
    case 481:
      ACCEPT_TOKEN(anon_sym_verify);
      END_STATE();
    case 482:
      ACCEPT_TOKEN(anon_sym_compile);
      END_STATE();
    case 483:
      ACCEPT_TOKEN(anon_sym_clippy);
      END_STATE();
    case 484:
      ACCEPT_TOKEN(anon_sym_test);
      END_STATE();
    case 485:
      ACCEPT_TOKEN(anon_sym_test);
      if (lookahead == '_') ADVANCE(167);
      END_STATE();
    case 486:
      ACCEPT_TOKEN(anon_sym_test);
      if (lookahead == '_') ADVANCE(167);
      if (lookahead == 's') ADVANCE(32);
      END_STATE();
    case 487:
      ACCEPT_TOKEN(anon_sym_impact_tests);
      END_STATE();
    case 488:
      ACCEPT_TOKEN(anon_sym_context);
      END_STATE();
    case 489:
      ACCEPT_TOKEN(anon_sym_callers_of);
      END_STATE();
    case 490:
      ACCEPT_TOKEN(anon_sym_tests_for);
      END_STATE();
    case 491:
      ACCEPT_TOKEN(anon_sym_depth);
      END_STATE();
    case 492:
      ACCEPT_TOKEN(anon_sym_loop);
      END_STATE();
    case 493:
      ACCEPT_TOKEN(anon_sym_loop);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 494:
      ACCEPT_TOKEN(anon_sym_agents);
      END_STATE();
    case 495:
      ACCEPT_TOKEN(anon_sym_max_iterations);
      END_STATE();
    case 496:
      ACCEPT_TOKEN(anon_sym_iter_start);
      END_STATE();
    case 497:
      ACCEPT_TOKEN(anon_sym_stability);
      END_STATE();
    case 498:
      ACCEPT_TOKEN(anon_sym_judge_timeout);
      END_STATE();
    case 499:
      ACCEPT_TOKEN(anon_sym_strict_judge);
      END_STATE();
    case 500:
      ACCEPT_TOKEN(anon_sym_branch_chain);
      END_STATE();
    case 501:
      ACCEPT_TOKEN(anon_sym_stacked);
      END_STATE();
    case 502:
      ACCEPT_TOKEN(anon_sym_none);
      END_STATE();
    case 503:
      ACCEPT_TOKEN(anon_sym_until);
      END_STATE();
    case 504:
      ACCEPT_TOKEN(anon_sym_command);
      END_STATE();
    case 505:
      ACCEPT_TOKEN(anon_sym_workflow);
      END_STATE();
    case 506:
      ACCEPT_TOKEN(anon_sym_steps);
      END_STATE();
    case 507:
      ACCEPT_TOKEN(anon_sym_max_parallel);
      END_STATE();
    case 508:
      ACCEPT_TOKEN(anon_sym_strategy);
      END_STATE();
    case 509:
      ACCEPT_TOKEN(anon_sym_test_first);
      END_STATE();
    case 510:
      ACCEPT_TOKEN(anon_sym_attempts);
      END_STATE();
    case 511:
      ACCEPT_TOKEN(anon_sym_escalate_after);
      END_STATE();
    case 512:
      ACCEPT_TOKEN(anon_sym_LBRACK);
      END_STATE();
    case 513:
      ACCEPT_TOKEN(anon_sym_RBRACK);
      END_STATE();
    case 514:
      ACCEPT_TOKEN(anon_sym_public);
      END_STATE();
    case 515:
      ACCEPT_TOKEN(anon_sym_local_only);
      END_STATE();
    case 516:
      ACCEPT_TOKEN(anon_sym_single_pass);
      END_STATE();
    case 517:
      ACCEPT_TOKEN(anon_sym_single_pass);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 518:
      ACCEPT_TOKEN(anon_sym_refine);
      END_STATE();
    case 519:
      ACCEPT_TOKEN(anon_sym_refine);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 520:
      ACCEPT_TOKEN(anon_sym_true);
      END_STATE();
    case 521:
      ACCEPT_TOKEN(anon_sym_false);
      END_STATE();
    case 522:
      ACCEPT_TOKEN(sym_string);
      END_STATE();
    case 523:
      ACCEPT_TOKEN(sym_raw_string);
      END_STATE();
    case 524:
      ACCEPT_TOKEN(sym_float);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(524);
      END_STATE();
    case 525:
      ACCEPT_TOKEN(sym_integer);
      if (lookahead == '.') ADVANCE(436);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(525);
      END_STATE();
    case 526:
      ACCEPT_TOKEN(sym_integer);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(526);
      END_STATE();
    case 527:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '_') ADVANCE(577);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 528:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(581);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 529:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(575);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 530:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(559);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 531:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(566);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 532:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(585);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 533:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(583);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 534:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(551);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 535:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(586);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 536:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(530);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 537:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'd') ADVANCE(554);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 538:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(534);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 539:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(528);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 540:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(563);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 541:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(454);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 542:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(547);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 543:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(519);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 544:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(527);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 545:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(535);
      if (lookahead == 'p') ADVANCE(540);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 546:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(529);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 547:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'f') ADVANCE(557);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 548:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'g') ADVANCE(458);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 549:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'g') ADVANCE(560);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 550:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(546);
      if (lookahead == 'o') ADVANCE(569);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 551:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(531);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 552:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(587);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 553:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(536);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 554:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(565);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 555:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(561);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 556:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(564);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 557:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(567);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 558:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(574);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 559:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(462);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 560:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(544);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 561:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(548);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 562:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(460);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 563:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(582);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 564:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(549);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 565:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(532);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 566:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(553);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 567:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(543);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 568:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(555);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 569:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(578);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 570:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(579);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 571:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(576);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 572:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(568);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 573:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(571);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 574:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(562);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 575:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(452);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 576:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(493);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 577:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(533);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 578:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(537);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 579:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(456);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 580:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(517);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 581:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(572);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 582:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(552);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 583:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(580);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 584:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(558);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 585:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(570);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 586:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'u') ADVANCE(584);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 587:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'v') ADVANCE(541);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 588:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'x') ADVANCE(545);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    case 589:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(589);
      END_STATE();
    default:
      return false;
  }
}

static const TSLexMode ts_lex_modes[STATE_COUNT] = {
  [0] = {.lex_state = 0},
  [1] = {.lex_state = 0},
  [2] = {.lex_state = 9},
  [3] = {.lex_state = 10},
  [4] = {.lex_state = 9},
  [5] = {.lex_state = 9},
  [6] = {.lex_state = 9},
  [7] = {.lex_state = 9},
  [8] = {.lex_state = 9},
  [9] = {.lex_state = 0},
  [10] = {.lex_state = 0},
  [11] = {.lex_state = 0},
  [12] = {.lex_state = 0},
  [13] = {.lex_state = 0},
  [14] = {.lex_state = 0},
  [15] = {.lex_state = 0},
  [16] = {.lex_state = 9},
  [17] = {.lex_state = 9},
  [18] = {.lex_state = 9},
  [19] = {.lex_state = 9},
  [20] = {.lex_state = 0},
  [21] = {.lex_state = 0},
  [22] = {.lex_state = 2},
  [23] = {.lex_state = 0},
  [24] = {.lex_state = 0},
  [25] = {.lex_state = 0},
  [26] = {.lex_state = 9},
  [27] = {.lex_state = 0},
  [28] = {.lex_state = 9},
  [29] = {.lex_state = 0},
  [30] = {.lex_state = 9},
  [31] = {.lex_state = 9},
  [32] = {.lex_state = 0},
  [33] = {.lex_state = 0},
  [34] = {.lex_state = 9},
  [35] = {.lex_state = 9},
  [36] = {.lex_state = 0},
  [37] = {.lex_state = 0},
  [38] = {.lex_state = 0},
  [39] = {.lex_state = 9},
  [40] = {.lex_state = 0},
  [41] = {.lex_state = 9},
  [42] = {.lex_state = 9},
  [43] = {.lex_state = 9},
  [44] = {.lex_state = 9},
  [45] = {.lex_state = 9},
  [46] = {.lex_state = 9},
  [47] = {.lex_state = 9},
  [48] = {.lex_state = 9},
  [49] = {.lex_state = 9},
  [50] = {.lex_state = 9},
  [51] = {.lex_state = 0},
  [52] = {.lex_state = 0},
  [53] = {.lex_state = 0},
  [54] = {.lex_state = 0},
  [55] = {.lex_state = 0},
  [56] = {.lex_state = 0},
  [57] = {.lex_state = 2},
  [58] = {.lex_state = 0},
  [59] = {.lex_state = 0},
  [60] = {.lex_state = 0},
  [61] = {.lex_state = 2},
  [62] = {.lex_state = 0},
  [63] = {.lex_state = 11},
  [64] = {.lex_state = 0},
  [65] = {.lex_state = 0},
  [66] = {.lex_state = 1},
  [67] = {.lex_state = 0},
  [68] = {.lex_state = 0},
  [69] = {.lex_state = 0},
  [70] = {.lex_state = 1},
  [71] = {.lex_state = 0},
  [72] = {.lex_state = 11},
  [73] = {.lex_state = 11},
  [74] = {.lex_state = 0},
  [75] = {.lex_state = 11},
  [76] = {.lex_state = 11},
  [77] = {.lex_state = 0},
  [78] = {.lex_state = 1},
  [79] = {.lex_state = 9},
  [80] = {.lex_state = 0},
  [81] = {.lex_state = 0},
  [82] = {.lex_state = 9},
  [83] = {.lex_state = 9},
  [84] = {.lex_state = 0},
  [85] = {.lex_state = 0},
  [86] = {.lex_state = 8},
  [87] = {.lex_state = 11},
  [88] = {.lex_state = 8},
  [89] = {.lex_state = 8},
  [90] = {.lex_state = 0},
  [91] = {.lex_state = 0},
  [92] = {.lex_state = 1},
  [93] = {.lex_state = 1},
  [94] = {.lex_state = 9},
  [95] = {.lex_state = 12},
  [96] = {.lex_state = 1},
  [97] = {.lex_state = 0},
  [98] = {.lex_state = 1},
  [99] = {.lex_state = 1},
  [100] = {.lex_state = 0},
  [101] = {.lex_state = 0},
  [102] = {.lex_state = 0},
  [103] = {.lex_state = 0},
  [104] = {.lex_state = 1},
  [105] = {.lex_state = 1},
  [106] = {.lex_state = 0},
  [107] = {.lex_state = 0},
  [108] = {.lex_state = 0},
  [109] = {.lex_state = 0},
  [110] = {.lex_state = 1},
  [111] = {.lex_state = 8},
  [112] = {.lex_state = 0},
  [113] = {.lex_state = 0},
  [114] = {.lex_state = 0},
  [115] = {.lex_state = 0},
  [116] = {.lex_state = 0},
  [117] = {.lex_state = 8},
  [118] = {.lex_state = 0},
  [119] = {.lex_state = 0},
  [120] = {.lex_state = 0},
  [121] = {.lex_state = 0},
  [122] = {.lex_state = 1},
  [123] = {.lex_state = 0},
  [124] = {.lex_state = 0},
  [125] = {.lex_state = 0},
  [126] = {.lex_state = 0},
  [127] = {.lex_state = 0},
  [128] = {.lex_state = 0},
  [129] = {.lex_state = 9},
  [130] = {.lex_state = 0},
  [131] = {.lex_state = 0},
  [132] = {.lex_state = 0},
  [133] = {.lex_state = 1},
  [134] = {.lex_state = 0},
  [135] = {.lex_state = 1},
  [136] = {.lex_state = 9},
  [137] = {.lex_state = 9},
  [138] = {.lex_state = 1},
  [139] = {.lex_state = 0},
  [140] = {.lex_state = 1},
  [141] = {.lex_state = 1},
  [142] = {.lex_state = 1},
  [143] = {.lex_state = 0},
  [144] = {.lex_state = 0},
  [145] = {.lex_state = 0},
  [146] = {.lex_state = 1},
  [147] = {.lex_state = 10},
  [148] = {.lex_state = 9},
  [149] = {.lex_state = 1},
  [150] = {.lex_state = 0},
  [151] = {.lex_state = 9},
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
    [anon_sym_max_iterations] = ACTIONS(1),
    [anon_sym_iter_start] = ACTIONS(1),
    [anon_sym_stability] = ACTIONS(1),
    [anon_sym_judge_timeout] = ACTIONS(1),
    [anon_sym_strict_judge] = ACTIONS(1),
    [anon_sym_branch_chain] = ACTIONS(1),
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
    [anon_sym_LBRACK] = ACTIONS(1),
    [anon_sym_RBRACK] = ACTIONS(1),
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
    [sym_source_file] = STATE(143),
    [sym__definition] = STATE(13),
    [sym_include_declaration] = STATE(13),
    [sym_client_declaration] = STATE(13),
    [sym_vars_block] = STATE(13),
    [sym_tier_alias_declaration] = STATE(13),
    [sym_prompt_declaration] = STATE(13),
    [sym_agent_declaration] = STATE(13),
    [sym_workflow_declaration] = STATE(13),
    [aux_sym_source_file_repeat1] = STATE(13),
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
  [0] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(21), 25,
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
  [31] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(25), 1,
      anon_sym_test,
    ACTIONS(23), 24,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_owned,
      anon_sym_read_only,
      anon_sym_impact_scope,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_impact_tests,
      anon_sym_agents,
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
  [64] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(27), 25,
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
  [95] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(29), 20,
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
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [121] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(31), 20,
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
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [147] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(33), 19,
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
      anon_sym_verify,
      anon_sym_context,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [172] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(35), 19,
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
      anon_sym_verify,
      anon_sym_context,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [197] = 15,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(13), 1,
      anon_sym_vars,
    ACTIONS(37), 1,
      anon_sym_client,
    ACTIONS(39), 1,
      anon_sym_RBRACE,
    ACTIONS(41), 1,
      anon_sym_tier,
    ACTIONS(43), 1,
      anon_sym_prompt,
    ACTIONS(45), 1,
      anon_sym_description,
    ACTIONS(47), 1,
      anon_sym_depends_on,
    ACTIONS(49), 1,
      anon_sym_max_retries,
    ACTIONS(51), 1,
      anon_sym_tools,
    ACTIONS(53), 1,
      anon_sym_scope,
    ACTIONS(55), 1,
      anon_sym_memory,
    ACTIONS(57), 1,
      anon_sym_context,
    STATE(11), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(21), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [247] = 15,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(13), 1,
      anon_sym_vars,
    ACTIONS(37), 1,
      anon_sym_client,
    ACTIONS(41), 1,
      anon_sym_tier,
    ACTIONS(43), 1,
      anon_sym_prompt,
    ACTIONS(45), 1,
      anon_sym_description,
    ACTIONS(47), 1,
      anon_sym_depends_on,
    ACTIONS(49), 1,
      anon_sym_max_retries,
    ACTIONS(51), 1,
      anon_sym_tools,
    ACTIONS(53), 1,
      anon_sym_scope,
    ACTIONS(55), 1,
      anon_sym_memory,
    ACTIONS(57), 1,
      anon_sym_context,
    ACTIONS(59), 1,
      anon_sym_RBRACE,
    STATE(9), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(21), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [297] = 15,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(61), 1,
      anon_sym_client,
    ACTIONS(64), 1,
      anon_sym_RBRACE,
    ACTIONS(66), 1,
      anon_sym_tier,
    ACTIONS(69), 1,
      anon_sym_vars,
    ACTIONS(72), 1,
      anon_sym_prompt,
    ACTIONS(75), 1,
      anon_sym_description,
    ACTIONS(78), 1,
      anon_sym_depends_on,
    ACTIONS(81), 1,
      anon_sym_max_retries,
    ACTIONS(84), 1,
      anon_sym_tools,
    ACTIONS(87), 1,
      anon_sym_scope,
    ACTIONS(90), 1,
      anon_sym_memory,
    ACTIONS(93), 1,
      anon_sym_context,
    STATE(11), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(21), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [347] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(96), 1,
      ts_builtin_sym_end,
    ACTIONS(98), 1,
      anon_sym_include,
    ACTIONS(101), 1,
      anon_sym_client,
    ACTIONS(104), 1,
      anon_sym_tier,
    ACTIONS(107), 1,
      anon_sym_vars,
    ACTIONS(110), 1,
      anon_sym_prompt,
    ACTIONS(113), 1,
      anon_sym_agent,
    ACTIONS(116), 1,
      anon_sym_workflow,
    STATE(12), 9,
      sym__definition,
      sym_include_declaration,
      sym_client_declaration,
      sym_vars_block,
      sym_tier_alias_declaration,
      sym_prompt_declaration,
      sym_agent_declaration,
      sym_workflow_declaration,
      aux_sym_source_file_repeat1,
  [386] = 10,
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
    ACTIONS(119), 1,
      ts_builtin_sym_end,
    STATE(12), 9,
      sym__definition,
      sym_include_declaration,
      sym_client_declaration,
      sym_vars_block,
      sym_tier_alias_declaration,
      sym_prompt_declaration,
      sym_agent_declaration,
      sym_workflow_declaration,
      aux_sym_source_file_repeat1,
  [425] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(121), 16,
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
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
      anon_sym_workflow,
  [447] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(123), 16,
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
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
      anon_sym_workflow,
  [469] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(55), 1,
      anon_sym_memory,
    ACTIONS(125), 1,
      anon_sym_RBRACE,
    ACTIONS(129), 1,
      anon_sym_verify,
    ACTIONS(131), 1,
      anon_sym_steps,
    ACTIONS(133), 1,
      anon_sym_strategy,
    ACTIONS(135), 1,
      anon_sym_test_first,
    STATE(17), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(31), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(127), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [505] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(55), 1,
      anon_sym_memory,
    ACTIONS(129), 1,
      anon_sym_verify,
    ACTIONS(131), 1,
      anon_sym_steps,
    ACTIONS(133), 1,
      anon_sym_strategy,
    ACTIONS(135), 1,
      anon_sym_test_first,
    ACTIONS(137), 1,
      anon_sym_RBRACE,
    STATE(18), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(31), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(127), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [541] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(139), 1,
      anon_sym_RBRACE,
    ACTIONS(144), 1,
      anon_sym_memory,
    ACTIONS(147), 1,
      anon_sym_verify,
    ACTIONS(150), 1,
      anon_sym_steps,
    ACTIONS(153), 1,
      anon_sym_strategy,
    ACTIONS(156), 1,
      anon_sym_test_first,
    STATE(18), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(31), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(141), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [577] = 9,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(159), 1,
      anon_sym_RBRACE,
    ACTIONS(161), 1,
      anon_sym_agents,
    ACTIONS(165), 1,
      anon_sym_strict_judge,
    ACTIONS(167), 1,
      anon_sym_branch_chain,
    ACTIONS(169), 1,
      anon_sym_until,
    STATE(43), 1,
      sym_until_clause,
    STATE(26), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(163), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [609] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(171), 12,
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
  [627] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(173), 12,
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
  [645] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(179), 1,
      sym_identifier,
    ACTIONS(177), 2,
      sym_string,
      sym_raw_string,
    STATE(67), 3,
      sym__effort_value,
      sym_tier_value,
      sym__string_value,
    ACTIONS(175), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [669] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(181), 12,
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
  [687] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(183), 12,
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
  [705] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(185), 12,
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
  [723] = 9,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(187), 1,
      anon_sym_RBRACE,
    ACTIONS(189), 1,
      anon_sym_agents,
    ACTIONS(195), 1,
      anon_sym_strict_judge,
    ACTIONS(198), 1,
      anon_sym_branch_chain,
    ACTIONS(201), 1,
      anon_sym_until,
    STATE(43), 1,
      sym_until_clause,
    STATE(26), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(192), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [755] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(204), 12,
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
  [773] = 9,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(161), 1,
      anon_sym_agents,
    ACTIONS(165), 1,
      anon_sym_strict_judge,
    ACTIONS(167), 1,
      anon_sym_branch_chain,
    ACTIONS(169), 1,
      anon_sym_until,
    ACTIONS(206), 1,
      anon_sym_RBRACE,
    STATE(43), 1,
      sym_until_clause,
    STATE(19), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(163), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [805] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(208), 12,
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
  [823] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(210), 10,
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
  [839] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(212), 10,
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
  [855] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(214), 1,
      anon_sym_RBRACE,
    ACTIONS(220), 1,
      anon_sym_importance,
    ACTIONS(222), 1,
      anon_sym_read_limit,
    ACTIONS(216), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(40), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(218), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [881] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(224), 1,
      anon_sym_RBRACE,
    ACTIONS(226), 1,
      anon_sym_tier,
    ACTIONS(229), 1,
      anon_sym_model,
    ACTIONS(232), 1,
      anon_sym_effort,
    ACTIONS(235), 1,
      anon_sym_privacy,
    ACTIONS(238), 1,
      anon_sym_default,
    ACTIONS(241), 1,
      anon_sym_extra,
    STATE(69), 1,
      sym_extra_block,
    STATE(33), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [913] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(244), 10,
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
  [929] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(246), 10,
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
  [945] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(248), 1,
      anon_sym_RBRACE,
    ACTIONS(250), 1,
      anon_sym_tier,
    ACTIONS(252), 1,
      anon_sym_model,
    ACTIONS(254), 1,
      anon_sym_effort,
    ACTIONS(256), 1,
      anon_sym_privacy,
    ACTIONS(258), 1,
      anon_sym_default,
    ACTIONS(260), 1,
      anon_sym_extra,
    STATE(69), 1,
      sym_extra_block,
    STATE(33), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [977] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(220), 1,
      anon_sym_importance,
    ACTIONS(222), 1,
      anon_sym_read_limit,
    ACTIONS(262), 1,
      anon_sym_RBRACE,
    ACTIONS(216), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(32), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(218), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [1003] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(250), 1,
      anon_sym_tier,
    ACTIONS(252), 1,
      anon_sym_model,
    ACTIONS(254), 1,
      anon_sym_effort,
    ACTIONS(256), 1,
      anon_sym_privacy,
    ACTIONS(258), 1,
      anon_sym_default,
    ACTIONS(260), 1,
      anon_sym_extra,
    ACTIONS(264), 1,
      anon_sym_RBRACE,
    STATE(69), 1,
      sym_extra_block,
    STATE(36), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [1035] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(266), 10,
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
  [1051] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(268), 1,
      anon_sym_RBRACE,
    ACTIONS(276), 1,
      anon_sym_importance,
    ACTIONS(279), 1,
      anon_sym_read_limit,
    ACTIONS(270), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(40), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(273), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [1077] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(282), 10,
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
  [1093] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(284), 10,
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
  [1109] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(286), 9,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [1124] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(288), 9,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [1139] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(290), 9,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [1154] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(292), 9,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [1169] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(294), 9,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [1184] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(296), 9,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [1199] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(298), 9,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [1214] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(300), 9,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_branch_chain,
      anon_sym_until,
  [1229] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(302), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1243] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(304), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1257] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(306), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1271] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(308), 8,
      anon_sym_RBRACE,
      anon_sym_read_ns,
      anon_sym_write_ns,
      anon_sym_importance,
      anon_sym_staleness_sources,
      anon_sym_read_query,
      anon_sym_read_limit,
      anon_sym_write_content,
  [1285] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(310), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1299] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(312), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1313] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(20), 1,
      sym_tier_alias_name,
    ACTIONS(314), 7,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
      sym_identifier,
  [1329] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(316), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1343] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(318), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1357] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(320), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1371] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(149), 1,
      sym_tier_alias_name,
    ACTIONS(322), 7,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
      sym_identifier,
  [1387] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(324), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1401] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(326), 1,
      anon_sym_RBRACE,
    STATE(72), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(328), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1418] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(67), 1,
      sym_tier_value,
    ACTIONS(330), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [1433] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(332), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1446] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(334), 1,
      anon_sym_RBRACE,
    STATE(108), 1,
      sym__string_value,
    STATE(66), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(336), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1465] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(339), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1478] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(341), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1491] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(343), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1504] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(345), 1,
      anon_sym_RBRACE,
    STATE(108), 1,
      sym__string_value,
    STATE(78), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(347), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1523] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(349), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1536] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(351), 1,
      anon_sym_RBRACE,
    STATE(72), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(353), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1553] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(356), 1,
      anon_sym_RBRACE,
    STATE(63), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(328), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1570] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(358), 1,
      anon_sym_LBRACE,
    ACTIONS(360), 1,
      anon_sym_agent,
    ACTIONS(362), 1,
      anon_sym_command,
    STATE(45), 4,
      sym__until_condition,
      sym_until_verify,
      sym_until_agent,
      sym_until_command,
  [1589] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(364), 1,
      anon_sym_RBRACE,
    STATE(76), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(328), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1606] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(366), 1,
      anon_sym_RBRACE,
    STATE(72), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(328), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1623] = 2,
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
  [1636] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(370), 1,
      anon_sym_RBRACE,
    STATE(108), 1,
      sym__string_value,
    STATE(66), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(347), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1655] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(372), 1,
      anon_sym_RBRACE,
    ACTIONS(377), 1,
      anon_sym_depth,
    ACTIONS(374), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(79), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1673] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(380), 1,
      anon_sym_RBRACE,
    ACTIONS(384), 1,
      anon_sym_impact_scope,
    ACTIONS(382), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(84), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1691] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(384), 1,
      anon_sym_impact_scope,
    ACTIONS(386), 1,
      anon_sym_RBRACE,
    ACTIONS(382), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(80), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1709] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(388), 1,
      anon_sym_RBRACE,
    ACTIONS(392), 1,
      anon_sym_depth,
    ACTIONS(390), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(79), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1727] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(392), 1,
      anon_sym_depth,
    ACTIONS(394), 1,
      anon_sym_RBRACE,
    ACTIONS(390), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(82), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1745] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(396), 1,
      anon_sym_RBRACE,
    ACTIONS(401), 1,
      anon_sym_impact_scope,
    ACTIONS(398), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(84), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1763] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(404), 1,
      anon_sym_RBRACK,
    ACTIONS(406), 2,
      sym_string,
      sym_raw_string,
    STATE(90), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [1778] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(408), 1,
      anon_sym_loop,
    ACTIONS(410), 1,
      anon_sym_RBRACK,
    ACTIONS(412), 1,
      sym_identifier,
    STATE(88), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [1795] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(414), 5,
      anon_sym_RBRACE,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1806] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(408), 1,
      anon_sym_loop,
    ACTIONS(416), 1,
      anon_sym_RBRACK,
    ACTIONS(418), 1,
      sym_identifier,
    STATE(89), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [1823] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(420), 1,
      anon_sym_loop,
    ACTIONS(423), 1,
      anon_sym_RBRACK,
    ACTIONS(425), 1,
      sym_identifier,
    STATE(89), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [1840] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(428), 1,
      anon_sym_RBRACK,
    ACTIONS(430), 2,
      sym_string,
      sym_raw_string,
    STATE(90), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [1855] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(433), 1,
      anon_sym_RBRACK,
    ACTIONS(435), 2,
      sym_string,
      sym_raw_string,
    STATE(85), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [1870] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(437), 1,
      anon_sym_RBRACE,
    ACTIONS(439), 1,
      sym_identifier,
    STATE(96), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [1884] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(441), 4,
      anon_sym_RBRACE,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1894] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(443), 4,
      anon_sym_RBRACE,
      anon_sym_callers_of,
      anon_sym_tests_for,
      anon_sym_depth,
  [1904] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(41), 1,
      sym_strategy_value,
    ACTIONS(445), 3,
      anon_sym_single_pass,
      anon_sym_refine,
      sym_identifier,
  [1916] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(439), 1,
      sym_identifier,
    ACTIONS(447), 1,
      anon_sym_RBRACE,
    STATE(98), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [1930] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(449), 4,
      anon_sym_RBRACE,
      anon_sym_owned,
      anon_sym_read_only,
      anon_sym_impact_scope,
  [1940] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(451), 1,
      anon_sym_RBRACE,
    ACTIONS(453), 1,
      sym_identifier,
    STATE(98), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [1954] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(20), 1,
      sym__string_value,
    ACTIONS(456), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1966] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(46), 1,
      sym_boolean,
    ACTIONS(458), 2,
      anon_sym_true,
      anon_sym_false,
  [1977] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(55), 1,
      sym__string_value,
    ACTIONS(460), 2,
      sym_string,
      sym_raw_string,
  [1988] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(87), 1,
      sym_boolean,
    ACTIONS(458), 2,
      anon_sym_true,
      anon_sym_false,
  [1999] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(67), 1,
      sym_privacy_value,
    ACTIONS(462), 2,
      anon_sym_public,
      anon_sym_local_only,
  [2010] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(464), 1,
      anon_sym_RBRACK,
    ACTIONS(466), 1,
      sym_identifier,
    STATE(110), 1,
      aux_sym_identifier_list_repeat1,
  [2023] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(468), 1,
      anon_sym_RBRACK,
    ACTIONS(470), 1,
      sym_identifier,
    STATE(105), 1,
      aux_sym_identifier_list_repeat1,
  [2036] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(122), 1,
      sym__string_value,
    ACTIONS(473), 2,
      sym_string,
      sym_raw_string,
  [2047] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(20), 1,
      sym__string_value,
    ACTIONS(456), 2,
      sym_string,
      sym_raw_string,
  [2058] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(93), 1,
      sym__string_value,
    ACTIONS(475), 2,
      sym_string,
      sym_raw_string,
  [2069] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(56), 1,
      sym__string_value,
    ACTIONS(477), 2,
      sym_string,
      sym_raw_string,
  [2080] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(479), 1,
      anon_sym_RBRACK,
    ACTIONS(481), 1,
      sym_identifier,
    STATE(105), 1,
      aux_sym_identifier_list_repeat1,
  [2093] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(485), 1,
      anon_sym_RBRACK,
    ACTIONS(483), 2,
      anon_sym_loop,
      sym_identifier,
  [2104] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(46), 1,
      sym_branch_chain_value,
    ACTIONS(487), 2,
      anon_sym_stacked,
      anon_sym_none,
  [2115] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(97), 1,
      sym_boolean,
    ACTIONS(458), 2,
      anon_sym_true,
      anon_sym_false,
  [2126] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(41), 1,
      sym_boolean,
    ACTIONS(458), 2,
      anon_sym_true,
      anon_sym_false,
  [2137] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(49), 1,
      sym__string_value,
    ACTIONS(489), 2,
      sym_string,
      sym_raw_string,
  [2148] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(67), 1,
      sym__string_value,
    ACTIONS(177), 2,
      sym_string,
      sym_raw_string,
  [2159] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(493), 1,
      anon_sym_RBRACK,
    ACTIONS(491), 2,
      anon_sym_loop,
      sym_identifier,
  [2170] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(54), 1,
      sym__string_value,
    ACTIONS(495), 2,
      sym_string,
      sym_raw_string,
  [2181] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(497), 1,
      anon_sym_LBRACK,
    STATE(54), 1,
      sym_string_list,
  [2191] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(499), 1,
      anon_sym_LBRACK,
    STATE(46), 1,
      sym_identifier_list,
  [2201] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(497), 1,
      anon_sym_LBRACK,
    STATE(20), 1,
      sym_string_list,
  [2211] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(501), 2,
      anon_sym_RBRACE,
      sym_identifier,
  [2219] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(503), 1,
      anon_sym_LBRACK,
    STATE(41), 1,
      sym_step_list,
  [2229] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(497), 1,
      anon_sym_LBRACK,
    STATE(97), 1,
      sym_string_list,
  [2239] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(499), 1,
      anon_sym_LBRACK,
    STATE(20), 1,
      sym_identifier_list,
  [2249] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(497), 1,
      anon_sym_LBRACK,
    STATE(94), 1,
      sym_string_list,
  [2259] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(505), 1,
      anon_sym_LBRACE,
  [2266] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(507), 1,
      anon_sym_LBRACE,
  [2273] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(456), 1,
      sym_integer,
  [2280] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(509), 1,
      anon_sym_LBRACE,
  [2287] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(511), 1,
      anon_sym_LBRACE,
  [2294] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(513), 1,
      anon_sym_LBRACE,
  [2301] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(515), 1,
      sym_identifier,
  [2308] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(517), 1,
      anon_sym_LBRACE,
  [2315] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(519), 1,
      sym_identifier,
  [2322] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(521), 1,
      sym_integer,
  [2329] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(523), 1,
      sym_integer,
  [2336] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(525), 1,
      sym_identifier,
  [2343] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(527), 1,
      anon_sym_LBRACE,
  [2350] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(208), 1,
      sym_identifier,
  [2357] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(529), 1,
      sym_identifier,
  [2364] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(531), 1,
      sym_identifier,
  [2371] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(533), 1,
      ts_builtin_sym_end,
  [2378] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(535), 1,
      anon_sym_LBRACE,
  [2385] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(537), 1,
      anon_sym_LBRACE,
  [2392] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(456), 1,
      sym_identifier,
  [2399] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(495), 1,
      sym_float,
  [2406] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(495), 1,
      sym_integer,
  [2413] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(539), 1,
      sym_identifier,
  [2420] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(541), 1,
      anon_sym_LBRACE,
  [2427] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(543), 1,
      sym_integer,
};

static const uint32_t ts_small_parse_table_map[] = {
  [SMALL_STATE(2)] = 0,
  [SMALL_STATE(3)] = 31,
  [SMALL_STATE(4)] = 64,
  [SMALL_STATE(5)] = 95,
  [SMALL_STATE(6)] = 121,
  [SMALL_STATE(7)] = 147,
  [SMALL_STATE(8)] = 172,
  [SMALL_STATE(9)] = 197,
  [SMALL_STATE(10)] = 247,
  [SMALL_STATE(11)] = 297,
  [SMALL_STATE(12)] = 347,
  [SMALL_STATE(13)] = 386,
  [SMALL_STATE(14)] = 425,
  [SMALL_STATE(15)] = 447,
  [SMALL_STATE(16)] = 469,
  [SMALL_STATE(17)] = 505,
  [SMALL_STATE(18)] = 541,
  [SMALL_STATE(19)] = 577,
  [SMALL_STATE(20)] = 609,
  [SMALL_STATE(21)] = 627,
  [SMALL_STATE(22)] = 645,
  [SMALL_STATE(23)] = 669,
  [SMALL_STATE(24)] = 687,
  [SMALL_STATE(25)] = 705,
  [SMALL_STATE(26)] = 723,
  [SMALL_STATE(27)] = 755,
  [SMALL_STATE(28)] = 773,
  [SMALL_STATE(29)] = 805,
  [SMALL_STATE(30)] = 823,
  [SMALL_STATE(31)] = 839,
  [SMALL_STATE(32)] = 855,
  [SMALL_STATE(33)] = 881,
  [SMALL_STATE(34)] = 913,
  [SMALL_STATE(35)] = 929,
  [SMALL_STATE(36)] = 945,
  [SMALL_STATE(37)] = 977,
  [SMALL_STATE(38)] = 1003,
  [SMALL_STATE(39)] = 1035,
  [SMALL_STATE(40)] = 1051,
  [SMALL_STATE(41)] = 1077,
  [SMALL_STATE(42)] = 1093,
  [SMALL_STATE(43)] = 1109,
  [SMALL_STATE(44)] = 1124,
  [SMALL_STATE(45)] = 1139,
  [SMALL_STATE(46)] = 1154,
  [SMALL_STATE(47)] = 1169,
  [SMALL_STATE(48)] = 1184,
  [SMALL_STATE(49)] = 1199,
  [SMALL_STATE(50)] = 1214,
  [SMALL_STATE(51)] = 1229,
  [SMALL_STATE(52)] = 1243,
  [SMALL_STATE(53)] = 1257,
  [SMALL_STATE(54)] = 1271,
  [SMALL_STATE(55)] = 1285,
  [SMALL_STATE(56)] = 1299,
  [SMALL_STATE(57)] = 1313,
  [SMALL_STATE(58)] = 1329,
  [SMALL_STATE(59)] = 1343,
  [SMALL_STATE(60)] = 1357,
  [SMALL_STATE(61)] = 1371,
  [SMALL_STATE(62)] = 1387,
  [SMALL_STATE(63)] = 1401,
  [SMALL_STATE(64)] = 1418,
  [SMALL_STATE(65)] = 1433,
  [SMALL_STATE(66)] = 1446,
  [SMALL_STATE(67)] = 1465,
  [SMALL_STATE(68)] = 1478,
  [SMALL_STATE(69)] = 1491,
  [SMALL_STATE(70)] = 1504,
  [SMALL_STATE(71)] = 1523,
  [SMALL_STATE(72)] = 1536,
  [SMALL_STATE(73)] = 1553,
  [SMALL_STATE(74)] = 1570,
  [SMALL_STATE(75)] = 1589,
  [SMALL_STATE(76)] = 1606,
  [SMALL_STATE(77)] = 1623,
  [SMALL_STATE(78)] = 1636,
  [SMALL_STATE(79)] = 1655,
  [SMALL_STATE(80)] = 1673,
  [SMALL_STATE(81)] = 1691,
  [SMALL_STATE(82)] = 1709,
  [SMALL_STATE(83)] = 1727,
  [SMALL_STATE(84)] = 1745,
  [SMALL_STATE(85)] = 1763,
  [SMALL_STATE(86)] = 1778,
  [SMALL_STATE(87)] = 1795,
  [SMALL_STATE(88)] = 1806,
  [SMALL_STATE(89)] = 1823,
  [SMALL_STATE(90)] = 1840,
  [SMALL_STATE(91)] = 1855,
  [SMALL_STATE(92)] = 1870,
  [SMALL_STATE(93)] = 1884,
  [SMALL_STATE(94)] = 1894,
  [SMALL_STATE(95)] = 1904,
  [SMALL_STATE(96)] = 1916,
  [SMALL_STATE(97)] = 1930,
  [SMALL_STATE(98)] = 1940,
  [SMALL_STATE(99)] = 1954,
  [SMALL_STATE(100)] = 1966,
  [SMALL_STATE(101)] = 1977,
  [SMALL_STATE(102)] = 1988,
  [SMALL_STATE(103)] = 1999,
  [SMALL_STATE(104)] = 2010,
  [SMALL_STATE(105)] = 2023,
  [SMALL_STATE(106)] = 2036,
  [SMALL_STATE(107)] = 2047,
  [SMALL_STATE(108)] = 2058,
  [SMALL_STATE(109)] = 2069,
  [SMALL_STATE(110)] = 2080,
  [SMALL_STATE(111)] = 2093,
  [SMALL_STATE(112)] = 2104,
  [SMALL_STATE(113)] = 2115,
  [SMALL_STATE(114)] = 2126,
  [SMALL_STATE(115)] = 2137,
  [SMALL_STATE(116)] = 2148,
  [SMALL_STATE(117)] = 2159,
  [SMALL_STATE(118)] = 2170,
  [SMALL_STATE(119)] = 2181,
  [SMALL_STATE(120)] = 2191,
  [SMALL_STATE(121)] = 2201,
  [SMALL_STATE(122)] = 2211,
  [SMALL_STATE(123)] = 2219,
  [SMALL_STATE(124)] = 2229,
  [SMALL_STATE(125)] = 2239,
  [SMALL_STATE(126)] = 2249,
  [SMALL_STATE(127)] = 2259,
  [SMALL_STATE(128)] = 2266,
  [SMALL_STATE(129)] = 2273,
  [SMALL_STATE(130)] = 2280,
  [SMALL_STATE(131)] = 2287,
  [SMALL_STATE(132)] = 2294,
  [SMALL_STATE(133)] = 2301,
  [SMALL_STATE(134)] = 2308,
  [SMALL_STATE(135)] = 2315,
  [SMALL_STATE(136)] = 2322,
  [SMALL_STATE(137)] = 2329,
  [SMALL_STATE(138)] = 2336,
  [SMALL_STATE(139)] = 2343,
  [SMALL_STATE(140)] = 2350,
  [SMALL_STATE(141)] = 2357,
  [SMALL_STATE(142)] = 2364,
  [SMALL_STATE(143)] = 2371,
  [SMALL_STATE(144)] = 2378,
  [SMALL_STATE(145)] = 2385,
  [SMALL_STATE(146)] = 2392,
  [SMALL_STATE(147)] = 2399,
  [SMALL_STATE(148)] = 2406,
  [SMALL_STATE(149)] = 2413,
  [SMALL_STATE(150)] = 2420,
  [SMALL_STATE(151)] = 2427,
};

static const TSParseActionEntry ts_parse_actions[] = {
  [0] = {.entry = {.count = 0, .reusable = false}},
  [1] = {.entry = {.count = 1, .reusable = false}}, RECOVER(),
  [3] = {.entry = {.count = 1, .reusable = true}}, SHIFT_EXTRA(),
  [5] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 0, 0, 0),
  [7] = {.entry = {.count = 1, .reusable = true}}, SHIFT(109),
  [9] = {.entry = {.count = 1, .reusable = true}}, SHIFT(135),
  [11] = {.entry = {.count = 1, .reusable = true}}, SHIFT(61),
  [13] = {.entry = {.count = 1, .reusable = true}}, SHIFT(150),
  [15] = {.entry = {.count = 1, .reusable = true}}, SHIFT(133),
  [17] = {.entry = {.count = 1, .reusable = true}}, SHIFT(138),
  [19] = {.entry = {.count = 1, .reusable = true}}, SHIFT(141),
  [21] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 3, 0, 0),
  [23] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_boolean, 1, 0, 0),
  [25] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_boolean, 1, 0, 0),
  [27] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 2, 0, 0),
  [29] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 2, 0, 0),
  [31] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 3, 0, 0),
  [33] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_block, 4, 0, 0),
  [35] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_block, 3, 0, 0),
  [37] = {.entry = {.count = 1, .reusable = true}}, SHIFT(146),
  [39] = {.entry = {.count = 1, .reusable = true}}, SHIFT(58),
  [41] = {.entry = {.count = 1, .reusable = true}}, SHIFT(57),
  [43] = {.entry = {.count = 1, .reusable = true}}, SHIFT(99),
  [45] = {.entry = {.count = 1, .reusable = true}}, SHIFT(107),
  [47] = {.entry = {.count = 1, .reusable = true}}, SHIFT(125),
  [49] = {.entry = {.count = 1, .reusable = true}}, SHIFT(129),
  [51] = {.entry = {.count = 1, .reusable = true}}, SHIFT(121),
  [53] = {.entry = {.count = 1, .reusable = true}}, SHIFT(127),
  [55] = {.entry = {.count = 1, .reusable = true}}, SHIFT(130),
  [57] = {.entry = {.count = 1, .reusable = true}}, SHIFT(128),
  [59] = {.entry = {.count = 1, .reusable = true}}, SHIFT(60),
  [61] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(146),
  [64] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0),
  [66] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(57),
  [69] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(150),
  [72] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(99),
  [75] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(107),
  [78] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(125),
  [81] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(129),
  [84] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(121),
  [87] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(127),
  [90] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(130),
  [93] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(128),
  [96] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0),
  [98] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(109),
  [101] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(135),
  [104] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(61),
  [107] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(150),
  [110] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(133),
  [113] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(138),
  [116] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(141),
  [119] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 1, 0, 0),
  [121] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_block, 3, 0, 0),
  [123] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_block, 4, 0, 0),
  [125] = {.entry = {.count = 1, .reusable = true}}, SHIFT(52),
  [127] = {.entry = {.count = 1, .reusable = true}}, SHIFT(137),
  [129] = {.entry = {.count = 1, .reusable = true}}, SHIFT(139),
  [131] = {.entry = {.count = 1, .reusable = true}}, SHIFT(123),
  [133] = {.entry = {.count = 1, .reusable = true}}, SHIFT(95),
  [135] = {.entry = {.count = 1, .reusable = true}}, SHIFT(114),
  [137] = {.entry = {.count = 1, .reusable = true}}, SHIFT(59),
  [139] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0),
  [141] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(137),
  [144] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(130),
  [147] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(139),
  [150] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(123),
  [153] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(95),
  [156] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(114),
  [159] = {.entry = {.count = 1, .reusable = true}}, SHIFT(117),
  [161] = {.entry = {.count = 1, .reusable = true}}, SHIFT(120),
  [163] = {.entry = {.count = 1, .reusable = true}}, SHIFT(136),
  [165] = {.entry = {.count = 1, .reusable = true}}, SHIFT(100),
  [167] = {.entry = {.count = 1, .reusable = true}}, SHIFT(112),
  [169] = {.entry = {.count = 1, .reusable = true}}, SHIFT(74),
  [171] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 2, 0, 0),
  [173] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 1, 0, 0),
  [175] = {.entry = {.count = 1, .reusable = false}}, SHIFT(68),
  [177] = {.entry = {.count = 1, .reusable = true}}, SHIFT(67),
  [179] = {.entry = {.count = 1, .reusable = false}}, SHIFT(67),
  [181] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_block, 3, 0, 0),
  [183] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 3, 0, 0),
  [185] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 4, 0, 0),
  [187] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0),
  [189] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(120),
  [192] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(136),
  [195] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(100),
  [198] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(112),
  [201] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(74),
  [204] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_block, 4, 0, 0),
  [206] = {.entry = {.count = 1, .reusable = true}}, SHIFT(111),
  [208] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_alias_name, 1, 0, 0),
  [210] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_strategy_value, 1, 0, 0),
  [212] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 1, 0, 0),
  [214] = {.entry = {.count = 1, .reusable = true}}, SHIFT(7),
  [216] = {.entry = {.count = 1, .reusable = true}}, SHIFT(119),
  [218] = {.entry = {.count = 1, .reusable = true}}, SHIFT(118),
  [220] = {.entry = {.count = 1, .reusable = true}}, SHIFT(147),
  [222] = {.entry = {.count = 1, .reusable = true}}, SHIFT(148),
  [224] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0),
  [226] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(64),
  [229] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(116),
  [232] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(22),
  [235] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(103),
  [238] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(69),
  [241] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(144),
  [244] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_block, 3, 0, 0),
  [246] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_step_list, 2, 0, 0),
  [248] = {.entry = {.count = 1, .reusable = true}}, SHIFT(53),
  [250] = {.entry = {.count = 1, .reusable = true}}, SHIFT(64),
  [252] = {.entry = {.count = 1, .reusable = true}}, SHIFT(116),
  [254] = {.entry = {.count = 1, .reusable = true}}, SHIFT(22),
  [256] = {.entry = {.count = 1, .reusable = true}}, SHIFT(103),
  [258] = {.entry = {.count = 1, .reusable = true}}, SHIFT(69),
  [260] = {.entry = {.count = 1, .reusable = true}}, SHIFT(144),
  [262] = {.entry = {.count = 1, .reusable = true}}, SHIFT(8),
  [264] = {.entry = {.count = 1, .reusable = true}}, SHIFT(62),
  [266] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_block, 4, 0, 0),
  [268] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0),
  [270] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(119),
  [273] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(118),
  [276] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(147),
  [279] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(148),
  [282] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 2, 0, 0),
  [284] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_step_list, 3, 0, 0),
  [286] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_field, 1, 0, 0),
  [288] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_branch_chain_value, 1, 0, 0),
  [290] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_clause, 2, 0, 0),
  [292] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_field, 2, 0, 0),
  [294] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_verify, 2, 0, 0),
  [296] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_agent, 2, 0, 0),
  [298] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_command, 2, 0, 0),
  [300] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_verify, 3, 0, 0),
  [302] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_alias_declaration, 3, 0, 0),
  [304] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 4, 0, 0),
  [306] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 5, 0, 0),
  [308] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_field, 2, 0, 0),
  [310] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_prompt_declaration, 3, 0, 0),
  [312] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_include_declaration, 2, 0, 0),
  [314] = {.entry = {.count = 1, .reusable = false}}, SHIFT(29),
  [316] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 5, 0, 0),
  [318] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 5, 0, 0),
  [320] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 4, 0, 0),
  [322] = {.entry = {.count = 1, .reusable = false}}, SHIFT(140),
  [324] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 4, 0, 0),
  [326] = {.entry = {.count = 1, .reusable = true}}, SHIFT(39),
  [328] = {.entry = {.count = 1, .reusable = true}}, SHIFT(102),
  [330] = {.entry = {.count = 1, .reusable = true}}, SHIFT(68),
  [332] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_block, 4, 0, 0),
  [334] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_extra_block_repeat1, 2, 0, 0),
  [336] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_extra_block_repeat1, 2, 0, 0), SHIFT_REPEAT(108),
  [339] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_field, 2, 0, 0),
  [341] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_value, 1, 0, 0),
  [343] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_field, 1, 0, 0),
  [345] = {.entry = {.count = 1, .reusable = true}}, SHIFT(77),
  [347] = {.entry = {.count = 1, .reusable = true}}, SHIFT(108),
  [349] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_privacy_value, 1, 0, 0),
  [351] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_verify_block_repeat1, 2, 0, 0),
  [353] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_verify_block_repeat1, 2, 0, 0), SHIFT_REPEAT(102),
  [356] = {.entry = {.count = 1, .reusable = true}}, SHIFT(34),
  [358] = {.entry = {.count = 1, .reusable = true}}, SHIFT(75),
  [360] = {.entry = {.count = 1, .reusable = true}}, SHIFT(142),
  [362] = {.entry = {.count = 1, .reusable = true}}, SHIFT(115),
  [364] = {.entry = {.count = 1, .reusable = true}}, SHIFT(47),
  [366] = {.entry = {.count = 1, .reusable = true}}, SHIFT(50),
  [368] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_block, 3, 0, 0),
  [370] = {.entry = {.count = 1, .reusable = true}}, SHIFT(65),
  [372] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0),
  [374] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0), SHIFT_REPEAT(126),
  [377] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0), SHIFT_REPEAT(151),
  [380] = {.entry = {.count = 1, .reusable = true}}, SHIFT(25),
  [382] = {.entry = {.count = 1, .reusable = true}}, SHIFT(124),
  [384] = {.entry = {.count = 1, .reusable = true}}, SHIFT(113),
  [386] = {.entry = {.count = 1, .reusable = true}}, SHIFT(24),
  [388] = {.entry = {.count = 1, .reusable = true}}, SHIFT(27),
  [390] = {.entry = {.count = 1, .reusable = true}}, SHIFT(126),
  [392] = {.entry = {.count = 1, .reusable = true}}, SHIFT(151),
  [394] = {.entry = {.count = 1, .reusable = true}}, SHIFT(23),
  [396] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0),
  [398] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(124),
  [401] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(113),
  [404] = {.entry = {.count = 1, .reusable = true}}, SHIFT(2),
  [406] = {.entry = {.count = 1, .reusable = true}}, SHIFT(90),
  [408] = {.entry = {.count = 1, .reusable = false}}, SHIFT(131),
  [410] = {.entry = {.count = 1, .reusable = true}}, SHIFT(35),
  [412] = {.entry = {.count = 1, .reusable = false}}, SHIFT(88),
  [414] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_field, 2, 0, 0),
  [416] = {.entry = {.count = 1, .reusable = true}}, SHIFT(42),
  [418] = {.entry = {.count = 1, .reusable = false}}, SHIFT(89),
  [420] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0), SHIFT_REPEAT(131),
  [423] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0),
  [425] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0), SHIFT_REPEAT(89),
  [428] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0),
  [430] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0), SHIFT_REPEAT(90),
  [433] = {.entry = {.count = 1, .reusable = true}}, SHIFT(4),
  [435] = {.entry = {.count = 1, .reusable = true}}, SHIFT(85),
  [437] = {.entry = {.count = 1, .reusable = true}}, SHIFT(14),
  [439] = {.entry = {.count = 1, .reusable = true}}, SHIFT(106),
  [441] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_pair, 2, 0, 0),
  [443] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_field, 2, 0, 0),
  [445] = {.entry = {.count = 1, .reusable = false}}, SHIFT(30),
  [447] = {.entry = {.count = 1, .reusable = true}}, SHIFT(15),
  [449] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_field, 2, 0, 0),
  [451] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_vars_block_repeat1, 2, 0, 0),
  [453] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_vars_block_repeat1, 2, 0, 0), SHIFT_REPEAT(106),
  [456] = {.entry = {.count = 1, .reusable = true}}, SHIFT(20),
  [458] = {.entry = {.count = 1, .reusable = true}}, SHIFT(3),
  [460] = {.entry = {.count = 1, .reusable = true}}, SHIFT(55),
  [462] = {.entry = {.count = 1, .reusable = true}}, SHIFT(71),
  [464] = {.entry = {.count = 1, .reusable = true}}, SHIFT(5),
  [466] = {.entry = {.count = 1, .reusable = true}}, SHIFT(110),
  [468] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0),
  [470] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0), SHIFT_REPEAT(105),
  [473] = {.entry = {.count = 1, .reusable = true}}, SHIFT(122),
  [475] = {.entry = {.count = 1, .reusable = true}}, SHIFT(93),
  [477] = {.entry = {.count = 1, .reusable = true}}, SHIFT(56),
  [479] = {.entry = {.count = 1, .reusable = true}}, SHIFT(6),
  [481] = {.entry = {.count = 1, .reusable = true}}, SHIFT(105),
  [483] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_loop_block, 3, 0, 0),
  [485] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_block, 3, 0, 0),
  [487] = {.entry = {.count = 1, .reusable = true}}, SHIFT(44),
  [489] = {.entry = {.count = 1, .reusable = true}}, SHIFT(49),
  [491] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_loop_block, 4, 0, 0),
  [493] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_block, 4, 0, 0),
  [495] = {.entry = {.count = 1, .reusable = true}}, SHIFT(54),
  [497] = {.entry = {.count = 1, .reusable = true}}, SHIFT(91),
  [499] = {.entry = {.count = 1, .reusable = true}}, SHIFT(104),
  [501] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_pair, 2, 0, 0),
  [503] = {.entry = {.count = 1, .reusable = true}}, SHIFT(86),
  [505] = {.entry = {.count = 1, .reusable = true}}, SHIFT(81),
  [507] = {.entry = {.count = 1, .reusable = true}}, SHIFT(83),
  [509] = {.entry = {.count = 1, .reusable = true}}, SHIFT(37),
  [511] = {.entry = {.count = 1, .reusable = true}}, SHIFT(28),
  [513] = {.entry = {.count = 1, .reusable = true}}, SHIFT(38),
  [515] = {.entry = {.count = 1, .reusable = true}}, SHIFT(101),
  [517] = {.entry = {.count = 1, .reusable = true}}, SHIFT(10),
  [519] = {.entry = {.count = 1, .reusable = true}}, SHIFT(132),
  [521] = {.entry = {.count = 1, .reusable = true}}, SHIFT(46),
  [523] = {.entry = {.count = 1, .reusable = true}}, SHIFT(41),
  [525] = {.entry = {.count = 1, .reusable = true}}, SHIFT(134),
  [527] = {.entry = {.count = 1, .reusable = true}}, SHIFT(73),
  [529] = {.entry = {.count = 1, .reusable = true}}, SHIFT(145),
  [531] = {.entry = {.count = 1, .reusable = true}}, SHIFT(48),
  [533] = {.entry = {.count = 1, .reusable = true}},  ACCEPT_INPUT(),
  [535] = {.entry = {.count = 1, .reusable = true}}, SHIFT(70),
  [537] = {.entry = {.count = 1, .reusable = true}}, SHIFT(16),
  [539] = {.entry = {.count = 1, .reusable = true}}, SHIFT(51),
  [541] = {.entry = {.count = 1, .reusable = true}}, SHIFT(92),
  [543] = {.entry = {.count = 1, .reusable = true}}, SHIFT(94),
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
