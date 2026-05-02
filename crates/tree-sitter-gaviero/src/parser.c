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
#define STATE_COUNT 150
#define LARGE_STATE_COUNT 2
#define SYMBOL_COUNT 129
#define ALIAS_COUNT 0
#define TOKEN_COUNT 75
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
  anon_sym_until = 53,
  anon_sym_command = 54,
  anon_sym_workflow = 55,
  anon_sym_steps = 56,
  anon_sym_max_parallel = 57,
  anon_sym_strategy = 58,
  anon_sym_test_first = 59,
  anon_sym_attempts = 60,
  anon_sym_escalate_after = 61,
  anon_sym_LBRACK = 62,
  anon_sym_RBRACK = 63,
  anon_sym_public = 64,
  anon_sym_local_only = 65,
  anon_sym_single_pass = 66,
  anon_sym_refine = 67,
  anon_sym_true = 68,
  anon_sym_false = 69,
  sym_string = 70,
  sym_raw_string = 71,
  sym_float = 72,
  sym_integer = 73,
  sym_identifier = 74,
  sym_source_file = 75,
  sym__definition = 76,
  sym_include_declaration = 77,
  sym_client_declaration = 78,
  sym_client_field = 79,
  sym__effort_value = 80,
  sym_extra_block = 81,
  sym_extra_pair = 82,
  sym_vars_block = 83,
  sym_vars_pair = 84,
  sym_tier_alias_declaration = 85,
  sym_tier_alias_name = 86,
  sym_prompt_declaration = 87,
  sym_agent_declaration = 88,
  sym_agent_field = 89,
  sym_scope_block = 90,
  sym_scope_field = 91,
  sym_memory_block = 92,
  sym_memory_field = 93,
  sym_verify_block = 94,
  sym_verify_field = 95,
  sym_context_block = 96,
  sym_context_field = 97,
  sym_loop_block = 98,
  sym_loop_field = 99,
  sym_until_clause = 100,
  sym__until_condition = 101,
  sym_until_verify = 102,
  sym_until_agent = 103,
  sym_until_command = 104,
  sym_workflow_declaration = 105,
  sym_workflow_field = 106,
  sym_step_list = 107,
  sym_string_list = 108,
  sym_identifier_list = 109,
  sym_tier_value = 110,
  sym_privacy_value = 111,
  sym_strategy_value = 112,
  sym_boolean = 113,
  sym__string_value = 114,
  aux_sym_source_file_repeat1 = 115,
  aux_sym_client_declaration_repeat1 = 116,
  aux_sym_extra_block_repeat1 = 117,
  aux_sym_vars_block_repeat1 = 118,
  aux_sym_agent_declaration_repeat1 = 119,
  aux_sym_scope_block_repeat1 = 120,
  aux_sym_memory_block_repeat1 = 121,
  aux_sym_verify_block_repeat1 = 122,
  aux_sym_context_block_repeat1 = 123,
  aux_sym_loop_block_repeat1 = 124,
  aux_sym_workflow_declaration_repeat1 = 125,
  aux_sym_step_list_repeat1 = 126,
  aux_sym_string_list_repeat1 = 127,
  aux_sym_identifier_list_repeat1 = 128,
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
  [140] = 140,
  [141] = 141,
  [142] = 142,
  [143] = 143,
  [144] = 144,
  [145] = 145,
  [146] = 22,
  [147] = 147,
  [148] = 148,
  [149] = 149,
};

static bool ts_lex(TSLexer *lexer, TSStateId state) {
  START_LEXER();
  eof = lexer->eof(lexer);
  switch (state) {
    case 0:
      if (eof) ADVANCE(418);
      ADVANCE_MAP(
        '"', 3,
        '#', 5,
        '/', 13,
        '[', 489,
        ']', 490,
        'a', 161,
        'c', 32,
        'd', 99,
        'e', 151,
        'f', 40,
        'i', 220,
        'j', 393,
        'l', 257,
        'm', 34,
        'o', 405,
        'p', 296,
        'r', 100,
        's', 71,
        't', 101,
        'u', 235,
        'v', 36,
        'w', 261,
        '{', 422,
        '}', 423,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(0);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(502);
      END_STATE();
    case 1:
      if (lookahead == '"') ADVANCE(3);
      if (lookahead == '#') ADVANCE(5);
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == ']') ADVANCE(490);
      if (lookahead == '}') ADVANCE(423);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(1);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 2:
      if (lookahead == '"') ADVANCE(3);
      if (lookahead == '#') ADVANCE(5);
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'c') ADVANCE(527);
      if (lookahead == 'e') ADVANCE(565);
      if (lookahead == 'm') ADVANCE(515);
      if (lookahead == 'r') ADVANCE(516);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(2);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 3:
      if (lookahead == '"') ADVANCE(499);
      if (lookahead != 0) ADVANCE(3);
      END_STATE();
    case 4:
      if (lookahead == '"') ADVANCE(6);
      if (lookahead != 0) ADVANCE(4);
      END_STATE();
    case 5:
      if (lookahead == '"') ADVANCE(4);
      END_STATE();
    case 6:
      if (lookahead == '#') ADVANCE(500);
      if (lookahead != 0) ADVANCE(4);
      END_STATE();
    case 7:
      if (lookahead == '.') ADVANCE(417);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(7);
      END_STATE();
    case 8:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == ']') ADVANCE(490);
      if (lookahead == 'l') ADVANCE(550);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(8);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 9:
      ADVANCE_MAP(
        '/', 13,
        'a', 166,
        'c', 33,
        'd', 112,
        'e', 321,
        'i', 231,
        'j', 393,
        'm', 35,
        'o', 405,
        'p', 315,
        'r', 131,
        's', 72,
        't', 146,
        'u', 235,
        'v', 36,
        'w', 302,
        '}', 423,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(9);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(503);
      END_STATE();
    case 10:
      ADVANCE_MAP(
        '/', 13,
        'a', 166,
        'c', 215,
        'e', 321,
        'i', 229,
        'j', 393,
        'm', 35,
        'o', 405,
        'r', 132,
        's', 366,
        't', 147,
        'u', 235,
        'v', 115,
        '}', 423,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(10);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(7);
      END_STATE();
    case 11:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'c') ADVANCE(215);
      if (lookahead == 'i') ADVANCE(230);
      if (lookahead == 't') ADVANCE(149);
      if (lookahead == '}') ADVANCE(423);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(11);
      END_STATE();
    case 12:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'r') ADVANCE(519);
      if (lookahead == 's') ADVANCE(533);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(12);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 13:
      if (lookahead == '/') ADVANCE(420);
      END_STATE();
    case 14:
      if (lookahead == '_') ADVANCE(190);
      END_STATE();
    case 15:
      if (lookahead == '_') ADVANCE(212);
      END_STATE();
    case 16:
      if (lookahead == '_') ADVANCE(76);
      END_STATE();
    case 17:
      if (lookahead == '_') ADVANCE(336);
      END_STATE();
    case 18:
      if (lookahead == '_') ADVANCE(195);
      END_STATE();
    case 19:
      if (lookahead == '_') ADVANCE(157);
      if (lookahead == 's') ADVANCE(31);
      END_STATE();
    case 20:
      if (lookahead == '_') ADVANCE(263);
      END_STATE();
    case 21:
      if (lookahead == '_') ADVANCE(289);
      END_STATE();
    case 22:
      if (lookahead == '_') ADVANCE(48);
      END_STATE();
    case 23:
      if (lookahead == '_') ADVANCE(337);
      END_STATE();
    case 24:
      if (lookahead == '_') ADVANCE(335);
      END_STATE();
    case 25:
      if (lookahead == '_') ADVANCE(341);
      END_STATE();
    case 26:
      if (lookahead == '_') ADVANCE(277);
      END_STATE();
    case 27:
      if (lookahead == '_') ADVANCE(377);
      END_STATE();
    case 28:
      if (lookahead == '_') ADVANCE(269);
      END_STATE();
    case 29:
      if (lookahead == '_') ADVANCE(265);
      END_STATE();
    case 30:
      if (lookahead == '_') ADVANCE(382);
      END_STATE();
    case 31:
      if (lookahead == '_') ADVANCE(158);
      END_STATE();
    case 32:
      if (lookahead == 'a') ADVANCE(202);
      if (lookahead == 'h') ADVANCE(113);
      if (lookahead == 'l') ADVANCE(169);
      if (lookahead == 'o') ADVANCE(221);
      END_STATE();
    case 33:
      if (lookahead == 'a') ADVANCE(202);
      if (lookahead == 'l') ADVANCE(194);
      if (lookahead == 'o') ADVANCE(254);
      END_STATE();
    case 34:
      if (lookahead == 'a') ADVANCE(406);
      if (lookahead == 'e') ADVANCE(69);
      if (lookahead == 'o') ADVANCE(93);
      END_STATE();
    case 35:
      if (lookahead == 'a') ADVANCE(406);
      if (lookahead == 'e') ADVANCE(227);
      END_STATE();
    case 36:
      if (lookahead == 'a') ADVANCE(306);
      if (lookahead == 'e') ADVANCE(308);
      END_STATE();
    case 37:
      if (lookahead == 'a') ADVANCE(92);
      if (lookahead == 'f') ADVANCE(185);
      END_STATE();
    case 38:
      if (lookahead == 'a') ADVANCE(68);
      if (lookahead == 'e') ADVANCE(284);
      if (lookahead == 'r') ADVANCE(60);
      END_STATE();
    case 39:
      if (lookahead == 'a') ADVANCE(429);
      END_STATE();
    case 40:
      if (lookahead == 'a') ADVANCE(201);
      END_STATE();
    case 41:
      if (lookahead == 'a') ADVANCE(67);
      if (lookahead == 'e') ADVANCE(284);
      if (lookahead == 'r') ADVANCE(60);
      END_STATE();
    case 42:
      if (lookahead == 'a') ADVANCE(395);
      END_STATE();
    case 43:
      if (lookahead == 'a') ADVANCE(77);
      END_STATE();
    case 44:
      if (lookahead == 'a') ADVANCE(77);
      if (lookahead == 'o') ADVANCE(311);
      END_STATE();
    case 45:
      if (lookahead == 'a') ADVANCE(282);
      END_STATE();
    case 46:
      if (lookahead == 'a') ADVANCE(247);
      END_STATE();
    case 47:
      if (lookahead == 'a') ADVANCE(207);
      END_STATE();
    case 48:
      if (lookahead == 'a') ADVANCE(159);
      END_STATE();
    case 49:
      if (lookahead == 'a') ADVANCE(75);
      END_STATE();
    case 50:
      if (lookahead == 'a') ADVANCE(204);
      END_STATE();
    case 51:
      if (lookahead == 'a') ADVANCE(91);
      END_STATE();
    case 52:
      if (lookahead == 'a') ADVANCE(98);
      END_STATE();
    case 53:
      if (lookahead == 'a') ADVANCE(245);
      END_STATE();
    case 54:
      if (lookahead == 'a') ADVANCE(320);
      END_STATE();
    case 55:
      if (lookahead == 'a') ADVANCE(343);
      END_STATE();
    case 56:
      if (lookahead == 'a') ADVANCE(199);
      END_STATE();
    case 57:
      if (lookahead == 'a') ADVANCE(316);
      END_STATE();
    case 58:
      if (lookahead == 'a') ADVANCE(388);
      END_STATE();
    case 59:
      if (lookahead == 'a') ADVANCE(241);
      END_STATE();
    case 60:
      if (lookahead == 'a') ADVANCE(379);
      if (lookahead == 'i') ADVANCE(82);
      END_STATE();
    case 61:
      if (lookahead == 'a') ADVANCE(381);
      END_STATE();
    case 62:
      if (lookahead == 'a') ADVANCE(219);
      END_STATE();
    case 63:
      if (lookahead == 'a') ADVANCE(84);
      if (lookahead == 'o') ADVANCE(311);
      END_STATE();
    case 64:
      if (lookahead == 'a') ADVANCE(392);
      END_STATE();
    case 65:
      if (lookahead == 'a') ADVANCE(85);
      END_STATE();
    case 66:
      if (lookahead == 'b') ADVANCE(208);
      END_STATE();
    case 67:
      if (lookahead == 'b') ADVANCE(186);
      END_STATE();
    case 68:
      if (lookahead == 'b') ADVANCE(186);
      if (lookahead == 'l') ADVANCE(143);
      END_STATE();
    case 69:
      if (lookahead == 'c') ADVANCE(168);
      if (lookahead == 'm') ADVANCE(267);
      END_STATE();
    case 70:
      if (lookahead == 'c') ADVANCE(491);
      END_STATE();
    case 71:
      if (lookahead == 'c') ADVANCE(260);
      if (lookahead == 'i') ADVANCE(236);
      if (lookahead == 't') ADVANCE(38);
      END_STATE();
    case 72:
      if (lookahead == 'c') ADVANCE(260);
      if (lookahead == 't') ADVANCE(38);
      END_STATE();
    case 73:
      if (lookahead == 'c') ADVANCE(398);
      END_STATE();
    case 74:
      if (lookahead == 'c') ADVANCE(217);
      END_STATE();
    case 75:
      if (lookahead == 'c') ADVANCE(411);
      END_STATE();
    case 76:
      if (lookahead == 'c') ADVANCE(278);
      if (lookahead == 'n') ADVANCE(327);
      END_STATE();
    case 77:
      if (lookahead == 'c') ADVANCE(370);
      END_STATE();
    case 78:
      if (lookahead == 'c') ADVANCE(109);
      END_STATE();
    case 79:
      if (lookahead == 'c') ADVANCE(138);
      END_STATE();
    case 80:
      if (lookahead == 'c') ADVANCE(47);
      END_STATE();
    case 81:
      if (lookahead == 'c') ADVANCE(312);
      END_STATE();
    case 82:
      if (lookahead == 'c') ADVANCE(373);
      END_STATE();
    case 83:
      if (lookahead == 'c') ADVANCE(50);
      if (lookahead == 'o') ADVANCE(281);
      END_STATE();
    case 84:
      if (lookahead == 'c') ADVANCE(384);
      END_STATE();
    case 85:
      if (lookahead == 'c') ADVANCE(375);
      END_STATE();
    case 86:
      if (lookahead == 'c') ADVANCE(56);
      END_STATE();
    case 87:
      if (lookahead == 'c') ADVANCE(274);
      END_STATE();
    case 88:
      if (lookahead == 'd') ADVANCE(163);
      END_STATE();
    case 89:
      if (lookahead == 'd') ADVANCE(450);
      END_STATE();
    case 90:
      if (lookahead == 'd') ADVANCE(481);
      END_STATE();
    case 91:
      if (lookahead == 'd') ADVANCE(15);
      END_STATE();
    case 92:
      if (lookahead == 'd') ADVANCE(15);
      if (lookahead == 's') ADVANCE(276);
      END_STATE();
    case 93:
      if (lookahead == 'd') ADVANCE(125);
      END_STATE();
    case 94:
      if (lookahead == 'd') ADVANCE(175);
      END_STATE();
    case 95:
      if (lookahead == 'd') ADVANCE(350);
      END_STATE();
    case 96:
      if (lookahead == 'd') ADVANCE(107);
      END_STATE();
    case 97:
      if (lookahead == 'd') ADVANCE(164);
      END_STATE();
    case 98:
      if (lookahead == 'd') ADVANCE(29);
      END_STATE();
    case 99:
      if (lookahead == 'e') ADVANCE(153);
      END_STATE();
    case 100:
      if (lookahead == 'e') ADVANCE(37);
      END_STATE();
    case 101:
      if (lookahead == 'e') ADVANCE(334);
      if (lookahead == 'i') ADVANCE(123);
      if (lookahead == 'o') ADVANCE(273);
      if (lookahead == 'r') ADVANCE(394);
      END_STATE();
    case 102:
      if (lookahead == 'e') ADVANCE(497);
      END_STATE();
    case 103:
      if (lookahead == 'e') ADVANCE(498);
      END_STATE();
    case 104:
      if (lookahead == 'e') ADVANCE(449);
      END_STATE();
    case 105:
      if (lookahead == 'e') ADVANCE(495);
      END_STATE();
    case 106:
      if (lookahead == 'e') ADVANCE(462);
      END_STATE();
    case 107:
      if (lookahead == 'e') ADVANCE(419);
      END_STATE();
    case 108:
      if (lookahead == 'e') ADVANCE(433);
      END_STATE();
    case 109:
      if (lookahead == 'e') ADVANCE(456);
      END_STATE();
    case 110:
      if (lookahead == 'e') ADVANCE(452);
      END_STATE();
    case 111:
      if (lookahead == 'e') ADVANCE(479);
      END_STATE();
    case 112:
      if (lookahead == 'e') ADVANCE(279);
      END_STATE();
    case 113:
      if (lookahead == 'e') ADVANCE(45);
      END_STATE();
    case 114:
      if (lookahead == 'e') ADVANCE(407);
      END_STATE();
    case 115:
      if (lookahead == 'e') ADVANCE(308);
      END_STATE();
    case 116:
      if (lookahead == 'e') ADVANCE(162);
      END_STATE();
    case 117:
      if (lookahead == 'e') ADVANCE(240);
      if (lookahead == 't') ADVANCE(167);
      END_STATE();
    case 118:
      if (lookahead == 'e') ADVANCE(73);
      if (lookahead == 'p') ADVANCE(121);
      if (lookahead == 't') ADVANCE(307);
      END_STATE();
    case 119:
      if (lookahead == 'e') ADVANCE(89);
      END_STATE();
    case 120:
      if (lookahead == 'e') ADVANCE(27);
      END_STATE();
    case 121:
      if (lookahead == 'e') ADVANCE(242);
      END_STATE();
    case 122:
      if (lookahead == 'e') ADVANCE(303);
      END_STATE();
    case 123:
      if (lookahead == 'e') ADVANCE(298);
      END_STATE();
    case 124:
      if (lookahead == 'e') ADVANCE(16);
      END_STATE();
    case 125:
      if (lookahead == 'e') ADVANCE(197);
      END_STATE();
    case 126:
      if (lookahead == 'e') ADVANCE(262);
      END_STATE();
    case 127:
      if (lookahead == 'e') ADVANCE(21);
      END_STATE();
    case 128:
      if (lookahead == 'e') ADVANCE(348);
      END_STATE();
    case 129:
      if (lookahead == 'e') ADVANCE(314);
      END_STATE();
    case 130:
      if (lookahead == 'e') ADVANCE(22);
      END_STATE();
    case 131:
      if (lookahead == 'e') ADVANCE(51);
      END_STATE();
    case 132:
      if (lookahead == 'e') ADVANCE(52);
      END_STATE();
    case 133:
      if (lookahead == 'e') ADVANCE(385);
      END_STATE();
    case 134:
      if (lookahead == 'e') ADVANCE(328);
      END_STATE();
    case 135:
      if (lookahead == 'e') ADVANCE(200);
      END_STATE();
    case 136:
      if (lookahead == 'e') ADVANCE(313);
      END_STATE();
    case 137:
      if (lookahead == 'e') ADVANCE(301);
      END_STATE();
    case 138:
      if (lookahead == 'e') ADVANCE(332);
      END_STATE();
    case 139:
      if (lookahead == 'e') ADVANCE(237);
      END_STATE();
    case 140:
      if (lookahead == 'e') ADVANCE(310);
      END_STATE();
    case 141:
      if (lookahead == 'e') ADVANCE(239);
      END_STATE();
    case 142:
      if (lookahead == 'e') ADVANCE(239);
      if (lookahead == 'p') ADVANCE(283);
      END_STATE();
    case 143:
      if (lookahead == 'e') ADVANCE(253);
      END_STATE();
    case 144:
      if (lookahead == 'e') ADVANCE(349);
      END_STATE();
    case 145:
      if (lookahead == 'e') ADVANCE(252);
      END_STATE();
    case 146:
      if (lookahead == 'e') ADVANCE(345);
      if (lookahead == 'i') ADVANCE(123);
      if (lookahead == 'o') ADVANCE(273);
      END_STATE();
    case 147:
      if (lookahead == 'e') ADVANCE(346);
      END_STATE();
    case 148:
      if (lookahead == 'e') ADVANCE(255);
      END_STATE();
    case 149:
      if (lookahead == 'e') ADVANCE(347);
      END_STATE();
    case 150:
      if (lookahead == 'e') ADVANCE(228);
      END_STATE();
    case 151:
      if (lookahead == 'f') ADVANCE(155);
      if (lookahead == 's') ADVANCE(80);
      if (lookahead == 'x') ADVANCE(118);
      END_STATE();
    case 152:
      if (lookahead == 'f') ADVANCE(469);
      END_STATE();
    case 153:
      if (lookahead == 'f') ADVANCE(42);
      if (lookahead == 'p') ADVANCE(117);
      if (lookahead == 's') ADVANCE(81);
      END_STATE();
    case 154:
      if (lookahead == 'f') ADVANCE(410);
      END_STATE();
    case 155:
      if (lookahead == 'f') ADVANCE(266);
      END_STATE();
    case 156:
      if (lookahead == 'f') ADVANCE(205);
      END_STATE();
    case 157:
      if (lookahead == 'f') ADVANCE(181);
      END_STATE();
    case 158:
      if (lookahead == 'f') ADVANCE(271);
      END_STATE();
    case 159:
      if (lookahead == 'f') ADVANCE(389);
      END_STATE();
    case 160:
      if (lookahead == 'g') ADVANCE(437);
      END_STATE();
    case 161:
      if (lookahead == 'g') ADVANCE(139);
      if (lookahead == 't') ADVANCE(368);
      END_STATE();
    case 162:
      if (lookahead == 'g') ADVANCE(412);
      END_STATE();
    case 163:
      if (lookahead == 'g') ADVANCE(120);
      END_STATE();
    case 164:
      if (lookahead == 'g') ADVANCE(111);
      END_STATE();
    case 165:
      if (lookahead == 'g') ADVANCE(214);
      END_STATE();
    case 166:
      if (lookahead == 'g') ADVANCE(148);
      if (lookahead == 't') ADVANCE(368);
      END_STATE();
    case 167:
      if (lookahead == 'h') ADVANCE(471);
      END_STATE();
    case 168:
      if (lookahead == 'h') ADVANCE(46);
      END_STATE();
    case 169:
      if (lookahead == 'i') ADVANCE(142);
      END_STATE();
    case 170:
      if (lookahead == 'i') ADVANCE(402);
      if (lookahead == 'o') ADVANCE(223);
      END_STATE();
    case 171:
      if (lookahead == 'i') ADVANCE(154);
      END_STATE();
    case 172:
      if (lookahead == 'i') ADVANCE(403);
      END_STATE();
    case 173:
      if (lookahead == 'i') ADVANCE(225);
      END_STATE();
    case 174:
      if (lookahead == 'i') ADVANCE(226);
      END_STATE();
    case 175:
      if (lookahead == 'i') ADVANCE(244);
      END_STATE();
    case 176:
      if (lookahead == 'i') ADVANCE(70);
      END_STATE();
    case 177:
      if (lookahead == 'i') ADVANCE(294);
      END_STATE();
    case 178:
      if (lookahead == 'i') ADVANCE(198);
      END_STATE();
    case 179:
      if (lookahead == 'i') ADVANCE(238);
      END_STATE();
    case 180:
      if (lookahead == 'i') ADVANCE(286);
      END_STATE();
    case 181:
      if (lookahead == 'i') ADVANCE(319);
      END_STATE();
    case 182:
      if (lookahead == 'i') ADVANCE(369);
      END_STATE();
    case 183:
      if (lookahead == 'i') ADVANCE(359);
      END_STATE();
    case 184:
      if (lookahead == 'i') ADVANCE(134);
      END_STATE();
    case 185:
      if (lookahead == 'i') ADVANCE(251);
      END_STATE();
    case 186:
      if (lookahead == 'i') ADVANCE(213);
      END_STATE();
    case 187:
      if (lookahead == 'i') ADVANCE(376);
      END_STATE();
    case 188:
      if (lookahead == 'i') ADVANCE(216);
      END_STATE();
    case 189:
      if (lookahead == 'i') ADVANCE(268);
      END_STATE();
    case 190:
      if (lookahead == 'i') ADVANCE(387);
      if (lookahead == 'p') ADVANCE(54);
      if (lookahead == 'r') ADVANCE(133);
      END_STATE();
    case 191:
      if (lookahead == 'i') ADVANCE(270);
      END_STATE();
    case 192:
      if (lookahead == 'i') ADVANCE(275);
      END_STATE();
    case 193:
      if (lookahead == 'i') ADVANCE(86);
      END_STATE();
    case 194:
      if (lookahead == 'i') ADVANCE(141);
      END_STATE();
    case 195:
      if (lookahead == 'j') ADVANCE(401);
      END_STATE();
    case 196:
      if (lookahead == 'k') ADVANCE(156);
      END_STATE();
    case 197:
      if (lookahead == 'l') ADVANCE(425);
      END_STATE();
    case 198:
      if (lookahead == 'l') ADVANCE(480);
      END_STATE();
    case 199:
      if (lookahead == 'l') ADVANCE(441);
      END_STATE();
    case 200:
      if (lookahead == 'l') ADVANCE(484);
      END_STATE();
    case 201:
      if (lookahead == 'l') ADVANCE(340);
      END_STATE();
    case 202:
      if (lookahead == 'l') ADVANCE(210);
      END_STATE();
    case 203:
      if (lookahead == 'l') ADVANCE(324);
      END_STATE();
    case 204:
      if (lookahead == 'l') ADVANCE(26);
      END_STATE();
    case 205:
      if (lookahead == 'l') ADVANCE(259);
      END_STATE();
    case 206:
      if (lookahead == 'l') ADVANCE(413);
      END_STATE();
    case 207:
      if (lookahead == 'l') ADVANCE(61);
      END_STATE();
    case 208:
      if (lookahead == 'l') ADVANCE(176);
      END_STATE();
    case 209:
      if (lookahead == 'l') ADVANCE(415);
      END_STATE();
    case 210:
      if (lookahead == 'l') ADVANCE(140);
      END_STATE();
    case 211:
      if (lookahead == 'l') ADVANCE(357);
      END_STATE();
    case 212:
      if (lookahead == 'l') ADVANCE(173);
      if (lookahead == 'n') ADVANCE(325);
      if (lookahead == 'o') ADVANCE(246);
      if (lookahead == 'q') ADVANCE(400);
      END_STATE();
    case 213:
      if (lookahead == 'l') ADVANCE(182);
      END_STATE();
    case 214:
      if (lookahead == 'l') ADVANCE(127);
      END_STATE();
    case 215:
      if (lookahead == 'l') ADVANCE(180);
      if (lookahead == 'o') ADVANCE(224);
      END_STATE();
    case 216:
      if (lookahead == 'l') ADVANCE(106);
      END_STATE();
    case 217:
      if (lookahead == 'l') ADVANCE(399);
      END_STATE();
    case 218:
      if (lookahead == 'l') ADVANCE(135);
      END_STATE();
    case 219:
      if (lookahead == 'l') ADVANCE(218);
      END_STATE();
    case 220:
      if (lookahead == 'm') ADVANCE(280);
      if (lookahead == 'n') ADVANCE(74);
      if (lookahead == 't') ADVANCE(122);
      END_STATE();
    case 221:
      if (lookahead == 'm') ADVANCE(222);
      if (lookahead == 'n') ADVANCE(374);
      if (lookahead == 'o') ADVANCE(305);
      END_STATE();
    case 222:
      if (lookahead == 'm') ADVANCE(59);
      if (lookahead == 'p') ADVANCE(188);
      END_STATE();
    case 223:
      if (lookahead == 'm') ADVANCE(287);
      END_STATE();
    case 224:
      if (lookahead == 'm') ADVANCE(285);
      END_STATE();
    case 225:
      if (lookahead == 'm') ADVANCE(183);
      END_STATE();
    case 226:
      if (lookahead == 'm') ADVANCE(126);
      END_STATE();
    case 227:
      if (lookahead == 'm') ADVANCE(267);
      END_STATE();
    case 228:
      if (lookahead == 'm') ADVANCE(288);
      END_STATE();
    case 229:
      if (lookahead == 'm') ADVANCE(291);
      if (lookahead == 't') ADVANCE(122);
      END_STATE();
    case 230:
      if (lookahead == 'm') ADVANCE(293);
      END_STATE();
    case 231:
      if (lookahead == 'm') ADVANCE(295);
      if (lookahead == 't') ADVANCE(122);
      END_STATE();
    case 232:
      if (lookahead == 'n') ADVANCE(439);
      END_STATE();
    case 233:
      if (lookahead == 'n') ADVANCE(446);
      END_STATE();
    case 234:
      if (lookahead == 'n') ADVANCE(445);
      END_STATE();
    case 235:
      if (lookahead == 'n') ADVANCE(367);
      END_STATE();
    case 236:
      if (lookahead == 'n') ADVANCE(165);
      END_STATE();
    case 237:
      if (lookahead == 'n') ADVANCE(352);
      END_STATE();
    case 238:
      if (lookahead == 'n') ADVANCE(160);
      END_STATE();
    case 239:
      if (lookahead == 'n') ADVANCE(353);
      END_STATE();
    case 240:
      if (lookahead == 'n') ADVANCE(95);
      END_STATE();
    case 241:
      if (lookahead == 'n') ADVANCE(90);
      END_STATE();
    case 242:
      if (lookahead == 'n') ADVANCE(338);
      END_STATE();
    case 243:
      if (lookahead == 'n') ADVANCE(119);
      END_STATE();
    case 244:
      if (lookahead == 'n') ADVANCE(58);
      END_STATE();
    case 245:
      if (lookahead == 'n') ADVANCE(78);
      END_STATE();
    case 246:
      if (lookahead == 'n') ADVANCE(206);
      END_STATE();
    case 247:
      if (lookahead == 'n') ADVANCE(193);
      END_STATE();
    case 248:
      if (lookahead == 'n') ADVANCE(209);
      END_STATE();
    case 249:
      if (lookahead == 'n') ADVANCE(179);
      END_STATE();
    case 250:
      if (lookahead == 'n') ADVANCE(331);
      END_STATE();
    case 251:
      if (lookahead == 'n') ADVANCE(105);
      END_STATE();
    case 252:
      if (lookahead == 'n') ADVANCE(362);
      END_STATE();
    case 253:
      if (lookahead == 'n') ADVANCE(128);
      END_STATE();
    case 254:
      if (lookahead == 'n') ADVANCE(374);
      END_STATE();
    case 255:
      if (lookahead == 'n') ADVANCE(380);
      END_STATE();
    case 256:
      if (lookahead == 'n') ADVANCE(390);
      END_STATE();
    case 257:
      if (lookahead == 'o') ADVANCE(83);
      END_STATE();
    case 258:
      if (lookahead == 'o') ADVANCE(223);
      END_STATE();
    case 259:
      if (lookahead == 'o') ADVANCE(404);
      END_STATE();
    case 260:
      if (lookahead == 'o') ADVANCE(290);
      END_STATE();
    case 261:
      if (lookahead == 'o') ADVANCE(297);
      if (lookahead == 'r') ADVANCE(187);
      END_STATE();
    case 262:
      if (lookahead == 'o') ADVANCE(397);
      END_STATE();
    case 263:
      if (lookahead == 'o') ADVANCE(152);
      END_STATE();
    case 264:
      if (lookahead == 'o') ADVANCE(396);
      END_STATE();
    case 265:
      if (lookahead == 'o') ADVANCE(246);
      END_STATE();
    case 266:
      if (lookahead == 'o') ADVANCE(309);
      END_STATE();
    case 267:
      if (lookahead == 'o') ADVANCE(304);
      END_STATE();
    case 268:
      if (lookahead == 'o') ADVANCE(232);
      END_STATE();
    case 269:
      if (lookahead == 'o') ADVANCE(233);
      END_STATE();
    case 270:
      if (lookahead == 'o') ADVANCE(234);
      END_STATE();
    case 271:
      if (lookahead == 'o') ADVANCE(299);
      END_STATE();
    case 272:
      if (lookahead == 'o') ADVANCE(300);
      END_STATE();
    case 273:
      if (lookahead == 'o') ADVANCE(203);
      END_STATE();
    case 274:
      if (lookahead == 'o') ADVANCE(292);
      END_STATE();
    case 275:
      if (lookahead == 'o') ADVANCE(250);
      END_STATE();
    case 276:
      if (lookahead == 'o') ADVANCE(249);
      END_STATE();
    case 277:
      if (lookahead == 'o') ADVANCE(248);
      END_STATE();
    case 278:
      if (lookahead == 'o') ADVANCE(256);
      END_STATE();
    case 279:
      if (lookahead == 'p') ADVANCE(117);
      if (lookahead == 's') ADVANCE(81);
      END_STATE();
    case 280:
      if (lookahead == 'p') ADVANCE(44);
      END_STATE();
    case 281:
      if (lookahead == 'p') ADVANCE(472);
      END_STATE();
    case 282:
      if (lookahead == 'p') ADVANCE(431);
      END_STATE();
    case 283:
      if (lookahead == 'p') ADVANCE(408);
      END_STATE();
    case 284:
      if (lookahead == 'p') ADVANCE(323);
      END_STATE();
    case 285:
      if (lookahead == 'p') ADVANCE(188);
      END_STATE();
    case 286:
      if (lookahead == 'p') ADVANCE(283);
      END_STATE();
    case 287:
      if (lookahead == 'p') ADVANCE(355);
      END_STATE();
    case 288:
      if (lookahead == 'p') ADVANCE(372);
      END_STATE();
    case 289:
      if (lookahead == 'p') ADVANCE(55);
      END_STATE();
    case 290:
      if (lookahead == 'p') ADVANCE(104);
      END_STATE();
    case 291:
      if (lookahead == 'p') ADVANCE(43);
      END_STATE();
    case 292:
      if (lookahead == 'p') ADVANCE(110);
      END_STATE();
    case 293:
      if (lookahead == 'p') ADVANCE(65);
      END_STATE();
    case 294:
      if (lookahead == 'p') ADVANCE(391);
      END_STATE();
    case 295:
      if (lookahead == 'p') ADVANCE(63);
      END_STATE();
    case 296:
      if (lookahead == 'r') ADVANCE(170);
      if (lookahead == 'u') ADVANCE(66);
      END_STATE();
    case 297:
      if (lookahead == 'r') ADVANCE(196);
      END_STATE();
    case 298:
      if (lookahead == 'r') ADVANCE(424);
      END_STATE();
    case 299:
      if (lookahead == 'r') ADVANCE(470);
      END_STATE();
    case 300:
      if (lookahead == 'r') ADVANCE(435);
      END_STATE();
    case 301:
      if (lookahead == 'r') ADVANCE(488);
      END_STATE();
    case 302:
      if (lookahead == 'r') ADVANCE(187);
      END_STATE();
    case 303:
      if (lookahead == 'r') ADVANCE(25);
      END_STATE();
    case 304:
      if (lookahead == 'r') ADVANCE(409);
      END_STATE();
    case 305:
      if (lookahead == 'r') ADVANCE(94);
      END_STATE();
    case 306:
      if (lookahead == 'r') ADVANCE(322);
      END_STATE();
    case 307:
      if (lookahead == 'r') ADVANCE(39);
      END_STATE();
    case 308:
      if (lookahead == 'r') ADVANCE(171);
      END_STATE();
    case 309:
      if (lookahead == 'r') ADVANCE(354);
      END_STATE();
    case 310:
      if (lookahead == 'r') ADVANCE(339);
      END_STATE();
    case 311:
      if (lookahead == 'r') ADVANCE(386);
      END_STATE();
    case 312:
      if (lookahead == 'r') ADVANCE(177);
      END_STATE();
    case 313:
      if (lookahead == 'r') ADVANCE(416);
      END_STATE();
    case 314:
      if (lookahead == 'r') ADVANCE(64);
      END_STATE();
    case 315:
      if (lookahead == 'r') ADVANCE(258);
      END_STATE();
    case 316:
      if (lookahead == 'r') ADVANCE(358);
      END_STATE();
    case 317:
      if (lookahead == 'r') ADVANCE(184);
      END_STATE();
    case 318:
      if (lookahead == 'r') ADVANCE(79);
      END_STATE();
    case 319:
      if (lookahead == 'r') ADVANCE(344);
      END_STATE();
    case 320:
      if (lookahead == 'r') ADVANCE(62);
      END_STATE();
    case 321:
      if (lookahead == 's') ADVANCE(80);
      END_STATE();
    case 322:
      if (lookahead == 's') ADVANCE(430);
      END_STATE();
    case 323:
      if (lookahead == 's') ADVANCE(483);
      END_STATE();
    case 324:
      if (lookahead == 's') ADVANCE(448);
      END_STATE();
    case 325:
      if (lookahead == 's') ADVANCE(454);
      END_STATE();
    case 326:
      if (lookahead == 's') ADVANCE(487);
      END_STATE();
    case 327:
      if (lookahead == 's') ADVANCE(455);
      END_STATE();
    case 328:
      if (lookahead == 's') ADVANCE(447);
      END_STATE();
    case 329:
      if (lookahead == 's') ADVANCE(493);
      END_STATE();
    case 330:
      if (lookahead == 's') ADVANCE(467);
      END_STATE();
    case 331:
      if (lookahead == 's') ADVANCE(475);
      END_STATE();
    case 332:
      if (lookahead == 's') ADVANCE(457);
      END_STATE();
    case 333:
      if (lookahead == 's') ADVANCE(474);
      END_STATE();
    case 334:
      if (lookahead == 's') ADVANCE(351);
      END_STATE();
    case 335:
      if (lookahead == 's') ADVANCE(87);
      END_STATE();
    case 336:
      if (lookahead == 's') ADVANCE(87);
      if (lookahead == 't') ADVANCE(144);
      END_STATE();
    case 337:
      if (lookahead == 's') ADVANCE(264);
      END_STATE();
    case 338:
      if (lookahead == 's') ADVANCE(172);
      END_STATE();
    case 339:
      if (lookahead == 's') ADVANCE(20);
      END_STATE();
    case 340:
      if (lookahead == 's') ADVANCE(103);
      END_STATE();
    case 341:
      if (lookahead == 's') ADVANCE(383);
      END_STATE();
    case 342:
      if (lookahead == 's') ADVANCE(23);
      END_STATE();
    case 343:
      if (lookahead == 's') ADVANCE(329);
      END_STATE();
    case 344:
      if (lookahead == 's') ADVANCE(360);
      END_STATE();
    case 345:
      if (lookahead == 's') ADVANCE(363);
      END_STATE();
    case 346:
      if (lookahead == 's') ADVANCE(364);
      END_STATE();
    case 347:
      if (lookahead == 's') ADVANCE(365);
      END_STATE();
    case 348:
      if (lookahead == 's') ADVANCE(342);
      END_STATE();
    case 349:
      if (lookahead == 's') ADVANCE(378);
      END_STATE();
    case 350:
      if (lookahead == 's') ADVANCE(28);
      END_STATE();
    case 351:
      if (lookahead == 't') ADVANCE(466);
      END_STATE();
    case 352:
      if (lookahead == 't') ADVANCE(444);
      END_STATE();
    case 353:
      if (lookahead == 't') ADVANCE(421);
      END_STATE();
    case 354:
      if (lookahead == 't') ADVANCE(426);
      END_STATE();
    case 355:
      if (lookahead == 't') ADVANCE(443);
      END_STATE();
    case 356:
      if (lookahead == 't') ADVANCE(468);
      END_STATE();
    case 357:
      if (lookahead == 't') ADVANCE(428);
      END_STATE();
    case 358:
      if (lookahead == 't') ADVANCE(476);
      END_STATE();
    case 359:
      if (lookahead == 't') ADVANCE(459);
      END_STATE();
    case 360:
      if (lookahead == 't') ADVANCE(486);
      END_STATE();
    case 361:
      if (lookahead == 't') ADVANCE(478);
      END_STATE();
    case 362:
      if (lookahead == 't') ADVANCE(460);
      END_STATE();
    case 363:
      if (lookahead == 't') ADVANCE(19);
      END_STATE();
    case 364:
      if (lookahead == 't') ADVANCE(465);
      END_STATE();
    case 365:
      if (lookahead == 't') ADVANCE(464);
      END_STATE();
    case 366:
      if (lookahead == 't') ADVANCE(41);
      END_STATE();
    case 367:
      if (lookahead == 't') ADVANCE(178);
      END_STATE();
    case 368:
      if (lookahead == 't') ADVANCE(150);
      END_STATE();
    case 369:
      if (lookahead == 't') ADVANCE(414);
      END_STATE();
    case 370:
      if (lookahead == 't') ADVANCE(17);
      END_STATE();
    case 371:
      if (lookahead == 't') ADVANCE(189);
      END_STATE();
    case 372:
      if (lookahead == 't') ADVANCE(326);
      END_STATE();
    case 373:
      if (lookahead == 't') ADVANCE(18);
      END_STATE();
    case 374:
      if (lookahead == 't') ADVANCE(114);
      END_STATE();
    case 375:
      if (lookahead == 't') ADVANCE(30);
      END_STATE();
    case 376:
      if (lookahead == 't') ADVANCE(124);
      END_STATE();
    case 377:
      if (lookahead == 't') ADVANCE(174);
      END_STATE();
    case 378:
      if (lookahead == 't') ADVANCE(330);
      END_STATE();
    case 379:
      if (lookahead == 't') ADVANCE(116);
      END_STATE();
    case 380:
      if (lookahead == 't') ADVANCE(333);
      END_STATE();
    case 381:
      if (lookahead == 't') ADVANCE(130);
      END_STATE();
    case 382:
      if (lookahead == 't') ADVANCE(144);
      END_STATE();
    case 383:
      if (lookahead == 't') ADVANCE(57);
      END_STATE();
    case 384:
      if (lookahead == 't') ADVANCE(24);
      END_STATE();
    case 385:
      if (lookahead == 't') ADVANCE(317);
      END_STATE();
    case 386:
      if (lookahead == 't') ADVANCE(53);
      END_STATE();
    case 387:
      if (lookahead == 't') ADVANCE(129);
      END_STATE();
    case 388:
      if (lookahead == 't') ADVANCE(272);
      END_STATE();
    case 389:
      if (lookahead == 't') ADVANCE(137);
      END_STATE();
    case 390:
      if (lookahead == 't') ADVANCE(145);
      END_STATE();
    case 391:
      if (lookahead == 't') ADVANCE(191);
      END_STATE();
    case 392:
      if (lookahead == 't') ADVANCE(192);
      END_STATE();
    case 393:
      if (lookahead == 'u') ADVANCE(88);
      END_STATE();
    case 394:
      if (lookahead == 'u') ADVANCE(102);
      END_STATE();
    case 395:
      if (lookahead == 'u') ADVANCE(211);
      END_STATE();
    case 396:
      if (lookahead == 'u') ADVANCE(318);
      END_STATE();
    case 397:
      if (lookahead == 'u') ADVANCE(361);
      END_STATE();
    case 398:
      if (lookahead == 'u') ADVANCE(371);
      END_STATE();
    case 399:
      if (lookahead == 'u') ADVANCE(96);
      END_STATE();
    case 400:
      if (lookahead == 'u') ADVANCE(136);
      END_STATE();
    case 401:
      if (lookahead == 'u') ADVANCE(97);
      END_STATE();
    case 402:
      if (lookahead == 'v') ADVANCE(49);
      END_STATE();
    case 403:
      if (lookahead == 'v') ADVANCE(108);
      END_STATE();
    case 404:
      if (lookahead == 'w') ADVANCE(482);
      END_STATE();
    case 405:
      if (lookahead == 'w') ADVANCE(243);
      END_STATE();
    case 406:
      if (lookahead == 'x') ADVANCE(14);
      END_STATE();
    case 407:
      if (lookahead == 'x') ADVANCE(356);
      END_STATE();
    case 408:
      if (lookahead == 'y') ADVANCE(463);
      END_STATE();
    case 409:
      if (lookahead == 'y') ADVANCE(453);
      END_STATE();
    case 410:
      if (lookahead == 'y') ADVANCE(461);
      END_STATE();
    case 411:
      if (lookahead == 'y') ADVANCE(427);
      END_STATE();
    case 412:
      if (lookahead == 'y') ADVANCE(485);
      END_STATE();
    case 413:
      if (lookahead == 'y') ADVANCE(451);
      END_STATE();
    case 414:
      if (lookahead == 'y') ADVANCE(477);
      END_STATE();
    case 415:
      if (lookahead == 'y') ADVANCE(492);
      END_STATE();
    case 416:
      if (lookahead == 'y') ADVANCE(458);
      END_STATE();
    case 417:
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(501);
      END_STATE();
    case 418:
      ACCEPT_TOKEN(ts_builtin_sym_end);
      END_STATE();
    case 419:
      ACCEPT_TOKEN(anon_sym_include);
      END_STATE();
    case 420:
      ACCEPT_TOKEN(sym_comment);
      if (lookahead != 0 &&
          lookahead != '\n') ADVANCE(420);
      END_STATE();
    case 421:
      ACCEPT_TOKEN(anon_sym_client);
      END_STATE();
    case 422:
      ACCEPT_TOKEN(anon_sym_LBRACE);
      END_STATE();
    case 423:
      ACCEPT_TOKEN(anon_sym_RBRACE);
      END_STATE();
    case 424:
      ACCEPT_TOKEN(anon_sym_tier);
      END_STATE();
    case 425:
      ACCEPT_TOKEN(anon_sym_model);
      END_STATE();
    case 426:
      ACCEPT_TOKEN(anon_sym_effort);
      END_STATE();
    case 427:
      ACCEPT_TOKEN(anon_sym_privacy);
      END_STATE();
    case 428:
      ACCEPT_TOKEN(anon_sym_default);
      END_STATE();
    case 429:
      ACCEPT_TOKEN(anon_sym_extra);
      END_STATE();
    case 430:
      ACCEPT_TOKEN(anon_sym_vars);
      END_STATE();
    case 431:
      ACCEPT_TOKEN(anon_sym_cheap);
      END_STATE();
    case 432:
      ACCEPT_TOKEN(anon_sym_cheap);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 433:
      ACCEPT_TOKEN(anon_sym_expensive);
      END_STATE();
    case 434:
      ACCEPT_TOKEN(anon_sym_expensive);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 435:
      ACCEPT_TOKEN(anon_sym_coordinator);
      END_STATE();
    case 436:
      ACCEPT_TOKEN(anon_sym_coordinator);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 437:
      ACCEPT_TOKEN(anon_sym_reasoning);
      END_STATE();
    case 438:
      ACCEPT_TOKEN(anon_sym_reasoning);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 439:
      ACCEPT_TOKEN(anon_sym_execution);
      END_STATE();
    case 440:
      ACCEPT_TOKEN(anon_sym_execution);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 441:
      ACCEPT_TOKEN(anon_sym_mechanical);
      END_STATE();
    case 442:
      ACCEPT_TOKEN(anon_sym_mechanical);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 443:
      ACCEPT_TOKEN(anon_sym_prompt);
      END_STATE();
    case 444:
      ACCEPT_TOKEN(anon_sym_agent);
      END_STATE();
    case 445:
      ACCEPT_TOKEN(anon_sym_description);
      END_STATE();
    case 446:
      ACCEPT_TOKEN(anon_sym_depends_on);
      END_STATE();
    case 447:
      ACCEPT_TOKEN(anon_sym_max_retries);
      END_STATE();
    case 448:
      ACCEPT_TOKEN(anon_sym_tools);
      END_STATE();
    case 449:
      ACCEPT_TOKEN(anon_sym_scope);
      END_STATE();
    case 450:
      ACCEPT_TOKEN(anon_sym_owned);
      END_STATE();
    case 451:
      ACCEPT_TOKEN(anon_sym_read_only);
      END_STATE();
    case 452:
      ACCEPT_TOKEN(anon_sym_impact_scope);
      END_STATE();
    case 453:
      ACCEPT_TOKEN(anon_sym_memory);
      END_STATE();
    case 454:
      ACCEPT_TOKEN(anon_sym_read_ns);
      END_STATE();
    case 455:
      ACCEPT_TOKEN(anon_sym_write_ns);
      END_STATE();
    case 456:
      ACCEPT_TOKEN(anon_sym_importance);
      END_STATE();
    case 457:
      ACCEPT_TOKEN(anon_sym_staleness_sources);
      END_STATE();
    case 458:
      ACCEPT_TOKEN(anon_sym_read_query);
      END_STATE();
    case 459:
      ACCEPT_TOKEN(anon_sym_read_limit);
      END_STATE();
    case 460:
      ACCEPT_TOKEN(anon_sym_write_content);
      END_STATE();
    case 461:
      ACCEPT_TOKEN(anon_sym_verify);
      END_STATE();
    case 462:
      ACCEPT_TOKEN(anon_sym_compile);
      END_STATE();
    case 463:
      ACCEPT_TOKEN(anon_sym_clippy);
      END_STATE();
    case 464:
      ACCEPT_TOKEN(anon_sym_test);
      END_STATE();
    case 465:
      ACCEPT_TOKEN(anon_sym_test);
      if (lookahead == '_') ADVANCE(157);
      END_STATE();
    case 466:
      ACCEPT_TOKEN(anon_sym_test);
      if (lookahead == '_') ADVANCE(157);
      if (lookahead == 's') ADVANCE(31);
      END_STATE();
    case 467:
      ACCEPT_TOKEN(anon_sym_impact_tests);
      END_STATE();
    case 468:
      ACCEPT_TOKEN(anon_sym_context);
      END_STATE();
    case 469:
      ACCEPT_TOKEN(anon_sym_callers_of);
      END_STATE();
    case 470:
      ACCEPT_TOKEN(anon_sym_tests_for);
      END_STATE();
    case 471:
      ACCEPT_TOKEN(anon_sym_depth);
      END_STATE();
    case 472:
      ACCEPT_TOKEN(anon_sym_loop);
      END_STATE();
    case 473:
      ACCEPT_TOKEN(anon_sym_loop);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 474:
      ACCEPT_TOKEN(anon_sym_agents);
      END_STATE();
    case 475:
      ACCEPT_TOKEN(anon_sym_max_iterations);
      END_STATE();
    case 476:
      ACCEPT_TOKEN(anon_sym_iter_start);
      END_STATE();
    case 477:
      ACCEPT_TOKEN(anon_sym_stability);
      END_STATE();
    case 478:
      ACCEPT_TOKEN(anon_sym_judge_timeout);
      END_STATE();
    case 479:
      ACCEPT_TOKEN(anon_sym_strict_judge);
      END_STATE();
    case 480:
      ACCEPT_TOKEN(anon_sym_until);
      END_STATE();
    case 481:
      ACCEPT_TOKEN(anon_sym_command);
      END_STATE();
    case 482:
      ACCEPT_TOKEN(anon_sym_workflow);
      END_STATE();
    case 483:
      ACCEPT_TOKEN(anon_sym_steps);
      END_STATE();
    case 484:
      ACCEPT_TOKEN(anon_sym_max_parallel);
      END_STATE();
    case 485:
      ACCEPT_TOKEN(anon_sym_strategy);
      END_STATE();
    case 486:
      ACCEPT_TOKEN(anon_sym_test_first);
      END_STATE();
    case 487:
      ACCEPT_TOKEN(anon_sym_attempts);
      END_STATE();
    case 488:
      ACCEPT_TOKEN(anon_sym_escalate_after);
      END_STATE();
    case 489:
      ACCEPT_TOKEN(anon_sym_LBRACK);
      END_STATE();
    case 490:
      ACCEPT_TOKEN(anon_sym_RBRACK);
      END_STATE();
    case 491:
      ACCEPT_TOKEN(anon_sym_public);
      END_STATE();
    case 492:
      ACCEPT_TOKEN(anon_sym_local_only);
      END_STATE();
    case 493:
      ACCEPT_TOKEN(anon_sym_single_pass);
      END_STATE();
    case 494:
      ACCEPT_TOKEN(anon_sym_single_pass);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 495:
      ACCEPT_TOKEN(anon_sym_refine);
      END_STATE();
    case 496:
      ACCEPT_TOKEN(anon_sym_refine);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 497:
      ACCEPT_TOKEN(anon_sym_true);
      END_STATE();
    case 498:
      ACCEPT_TOKEN(anon_sym_false);
      END_STATE();
    case 499:
      ACCEPT_TOKEN(sym_string);
      END_STATE();
    case 500:
      ACCEPT_TOKEN(sym_raw_string);
      END_STATE();
    case 501:
      ACCEPT_TOKEN(sym_float);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(501);
      END_STATE();
    case 502:
      ACCEPT_TOKEN(sym_integer);
      if (lookahead == '.') ADVANCE(417);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(502);
      END_STATE();
    case 503:
      ACCEPT_TOKEN(sym_integer);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(503);
      END_STATE();
    case 504:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '_') ADVANCE(554);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 505:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(558);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 506:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(552);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 507:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(536);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 508:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(543);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 509:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(562);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 510:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(560);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 511:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(528);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 512:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(563);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 513:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(507);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 514:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'd') ADVANCE(531);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 515:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(511);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 516:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(505);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 517:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(540);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 518:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(434);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 519:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(524);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 520:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(496);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 521:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(504);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 522:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(512);
      if (lookahead == 'p') ADVANCE(517);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 523:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(506);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 524:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'f') ADVANCE(534);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 525:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'g') ADVANCE(438);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 526:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'g') ADVANCE(537);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 527:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(523);
      if (lookahead == 'o') ADVANCE(546);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 528:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(508);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 529:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(564);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 530:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(513);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 531:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(542);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 532:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(538);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 533:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(541);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 534:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(544);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 535:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(551);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 536:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(442);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 537:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(521);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 538:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(525);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 539:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(440);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 540:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(559);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 541:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(526);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 542:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(509);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 543:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(530);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 544:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(520);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 545:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(532);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 546:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(555);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 547:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(556);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 548:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(553);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 549:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(545);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 550:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(548);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 551:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(539);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 552:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(432);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 553:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(473);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 554:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(510);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 555:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(514);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 556:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(436);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 557:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(494);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 558:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(549);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 559:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(529);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 560:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(557);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 561:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(535);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 562:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(547);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 563:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'u') ADVANCE(561);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 564:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'v') ADVANCE(518);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 565:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'x') ADVANCE(522);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    case 566:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(566);
      END_STATE();
    default:
      return false;
  }
}

static const TSLexMode ts_lex_modes[STATE_COUNT] = {
  [0] = {.lex_state = 0},
  [1] = {.lex_state = 0},
  [2] = {.lex_state = 9},
  [3] = {.lex_state = 9},
  [4] = {.lex_state = 10},
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
  [19] = {.lex_state = 0},
  [20] = {.lex_state = 0},
  [21] = {.lex_state = 2},
  [22] = {.lex_state = 0},
  [23] = {.lex_state = 0},
  [24] = {.lex_state = 0},
  [25] = {.lex_state = 0},
  [26] = {.lex_state = 0},
  [27] = {.lex_state = 9},
  [28] = {.lex_state = 9},
  [29] = {.lex_state = 9},
  [30] = {.lex_state = 9},
  [31] = {.lex_state = 0},
  [32] = {.lex_state = 0},
  [33] = {.lex_state = 0},
  [34] = {.lex_state = 9},
  [35] = {.lex_state = 0},
  [36] = {.lex_state = 9},
  [37] = {.lex_state = 9},
  [38] = {.lex_state = 0},
  [39] = {.lex_state = 0},
  [40] = {.lex_state = 9},
  [41] = {.lex_state = 9},
  [42] = {.lex_state = 9},
  [43] = {.lex_state = 2},
  [44] = {.lex_state = 0},
  [45] = {.lex_state = 0},
  [46] = {.lex_state = 0},
  [47] = {.lex_state = 0},
  [48] = {.lex_state = 0},
  [49] = {.lex_state = 0},
  [50] = {.lex_state = 0},
  [51] = {.lex_state = 2},
  [52] = {.lex_state = 9},
  [53] = {.lex_state = 0},
  [54] = {.lex_state = 9},
  [55] = {.lex_state = 9},
  [56] = {.lex_state = 0},
  [57] = {.lex_state = 9},
  [58] = {.lex_state = 9},
  [59] = {.lex_state = 9},
  [60] = {.lex_state = 9},
  [61] = {.lex_state = 0},
  [62] = {.lex_state = 11},
  [63] = {.lex_state = 0},
  [64] = {.lex_state = 0},
  [65] = {.lex_state = 11},
  [66] = {.lex_state = 0},
  [67] = {.lex_state = 0},
  [68] = {.lex_state = 1},
  [69] = {.lex_state = 11},
  [70] = {.lex_state = 0},
  [71] = {.lex_state = 1},
  [72] = {.lex_state = 1},
  [73] = {.lex_state = 0},
  [74] = {.lex_state = 11},
  [75] = {.lex_state = 11},
  [76] = {.lex_state = 0},
  [77] = {.lex_state = 0},
  [78] = {.lex_state = 9},
  [79] = {.lex_state = 0},
  [80] = {.lex_state = 0},
  [81] = {.lex_state = 9},
  [82] = {.lex_state = 0},
  [83] = {.lex_state = 9},
  [84] = {.lex_state = 8},
  [85] = {.lex_state = 0},
  [86] = {.lex_state = 0},
  [87] = {.lex_state = 8},
  [88] = {.lex_state = 0},
  [89] = {.lex_state = 8},
  [90] = {.lex_state = 11},
  [91] = {.lex_state = 1},
  [92] = {.lex_state = 1},
  [93] = {.lex_state = 1},
  [94] = {.lex_state = 0},
  [95] = {.lex_state = 9},
  [96] = {.lex_state = 12},
  [97] = {.lex_state = 1},
  [98] = {.lex_state = 1},
  [99] = {.lex_state = 0},
  [100] = {.lex_state = 0},
  [101] = {.lex_state = 0},
  [102] = {.lex_state = 0},
  [103] = {.lex_state = 0},
  [104] = {.lex_state = 0},
  [105] = {.lex_state = 1},
  [106] = {.lex_state = 1},
  [107] = {.lex_state = 1},
  [108] = {.lex_state = 0},
  [109] = {.lex_state = 0},
  [110] = {.lex_state = 0},
  [111] = {.lex_state = 8},
  [112] = {.lex_state = 0},
  [113] = {.lex_state = 0},
  [114] = {.lex_state = 8},
  [115] = {.lex_state = 0},
  [116] = {.lex_state = 0},
  [117] = {.lex_state = 1},
  [118] = {.lex_state = 0},
  [119] = {.lex_state = 0},
  [120] = {.lex_state = 0},
  [121] = {.lex_state = 0},
  [122] = {.lex_state = 0},
  [123] = {.lex_state = 0},
  [124] = {.lex_state = 0},
  [125] = {.lex_state = 9},
  [126] = {.lex_state = 0},
  [127] = {.lex_state = 0},
  [128] = {.lex_state = 0},
  [129] = {.lex_state = 0},
  [130] = {.lex_state = 0},
  [131] = {.lex_state = 0},
  [132] = {.lex_state = 0},
  [133] = {.lex_state = 1},
  [134] = {.lex_state = 0},
  [135] = {.lex_state = 0},
  [136] = {.lex_state = 9},
  [137] = {.lex_state = 9},
  [138] = {.lex_state = 0},
  [139] = {.lex_state = 10},
  [140] = {.lex_state = 1},
  [141] = {.lex_state = 1},
  [142] = {.lex_state = 9},
  [143] = {.lex_state = 1},
  [144] = {.lex_state = 0},
  [145] = {.lex_state = 1},
  [146] = {.lex_state = 1},
  [147] = {.lex_state = 1},
  [148] = {.lex_state = 1},
  [149] = {.lex_state = 9},
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
    [sym_source_file] = STATE(129),
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
  [31] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(23), 25,
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
  [62] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(27), 1,
      anon_sym_test,
    ACTIONS(25), 23,
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
      anon_sym_until,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [94] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(29), 19,
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
  [119] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(31), 19,
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
      anon_sym_until,
  [144] = 2,
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
  [169] = 2,
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
      anon_sym_context,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [194] = 15,
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
    STATE(19), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [244] = 15,
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
    STATE(19), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [294] = 15,
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
    STATE(19), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [344] = 10,
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
  [383] = 10,
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
  [422] = 2,
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
  [444] = 2,
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
  [466] = 10,
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
    STATE(36), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(127), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [502] = 10,
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
    STATE(36), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(127), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [538] = 10,
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
    STATE(36), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(141), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [574] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(159), 12,
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
  [592] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(161), 12,
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
  [610] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(167), 1,
      sym_identifier,
    ACTIONS(165), 2,
      sym_string,
      sym_raw_string,
    STATE(63), 3,
      sym__effort_value,
      sym_tier_value,
      sym__string_value,
    ACTIONS(163), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [634] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(169), 12,
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
  [652] = 2,
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
  [670] = 2,
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
  [688] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(175), 12,
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
  [706] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(177), 12,
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
  [724] = 8,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(179), 1,
      anon_sym_RBRACE,
    ACTIONS(181), 1,
      anon_sym_agents,
    ACTIONS(185), 1,
      anon_sym_strict_judge,
    ACTIONS(187), 1,
      anon_sym_until,
    STATE(52), 1,
      sym_until_clause,
    STATE(28), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(183), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [753] = 8,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(181), 1,
      anon_sym_agents,
    ACTIONS(185), 1,
      anon_sym_strict_judge,
    ACTIONS(187), 1,
      anon_sym_until,
    ACTIONS(189), 1,
      anon_sym_RBRACE,
    STATE(52), 1,
      sym_until_clause,
    STATE(29), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(183), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [782] = 8,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(191), 1,
      anon_sym_RBRACE,
    ACTIONS(193), 1,
      anon_sym_agents,
    ACTIONS(199), 1,
      anon_sym_strict_judge,
    ACTIONS(202), 1,
      anon_sym_until,
    STATE(52), 1,
      sym_until_clause,
    STATE(29), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(196), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [811] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(205), 10,
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
  [827] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(207), 1,
      anon_sym_RBRACE,
    ACTIONS(209), 1,
      anon_sym_tier,
    ACTIONS(211), 1,
      anon_sym_model,
    ACTIONS(213), 1,
      anon_sym_effort,
    ACTIONS(215), 1,
      anon_sym_privacy,
    ACTIONS(217), 1,
      anon_sym_default,
    ACTIONS(219), 1,
      anon_sym_extra,
    STATE(66), 1,
      sym_extra_block,
    STATE(35), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [859] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(221), 1,
      anon_sym_RBRACE,
    ACTIONS(227), 1,
      anon_sym_importance,
    ACTIONS(229), 1,
      anon_sym_read_limit,
    ACTIONS(223), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(39), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(225), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [885] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(209), 1,
      anon_sym_tier,
    ACTIONS(211), 1,
      anon_sym_model,
    ACTIONS(213), 1,
      anon_sym_effort,
    ACTIONS(215), 1,
      anon_sym_privacy,
    ACTIONS(217), 1,
      anon_sym_default,
    ACTIONS(219), 1,
      anon_sym_extra,
    ACTIONS(231), 1,
      anon_sym_RBRACE,
    STATE(66), 1,
      sym_extra_block,
    STATE(31), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [917] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(233), 10,
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
  [933] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(235), 1,
      anon_sym_RBRACE,
    ACTIONS(237), 1,
      anon_sym_tier,
    ACTIONS(240), 1,
      anon_sym_model,
    ACTIONS(243), 1,
      anon_sym_effort,
    ACTIONS(246), 1,
      anon_sym_privacy,
    ACTIONS(249), 1,
      anon_sym_default,
    ACTIONS(252), 1,
      anon_sym_extra,
    STATE(66), 1,
      sym_extra_block,
    STATE(35), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [965] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(255), 10,
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
  [981] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(257), 10,
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
  [997] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(227), 1,
      anon_sym_importance,
    ACTIONS(229), 1,
      anon_sym_read_limit,
    ACTIONS(259), 1,
      anon_sym_RBRACE,
    ACTIONS(223), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(32), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(225), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [1023] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(261), 1,
      anon_sym_RBRACE,
    ACTIONS(269), 1,
      anon_sym_importance,
    ACTIONS(272), 1,
      anon_sym_read_limit,
    ACTIONS(263), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(39), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(266), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [1049] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(275), 10,
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
  [1065] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(277), 10,
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
  [1081] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(279), 10,
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
  [1097] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(20), 1,
      sym_tier_alias_name,
    ACTIONS(281), 7,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
      sym_identifier,
  [1113] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(283), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1127] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(285), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1141] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(287), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1155] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(289), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1169] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(291), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1183] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(293), 8,
      anon_sym_RBRACE,
      anon_sym_read_ns,
      anon_sym_write_ns,
      anon_sym_importance,
      anon_sym_staleness_sources,
      anon_sym_read_query,
      anon_sym_read_limit,
      anon_sym_write_content,
  [1197] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(295), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1211] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(141), 1,
      sym_tier_alias_name,
    ACTIONS(297), 7,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
      sym_identifier,
  [1227] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(299), 8,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [1241] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(301), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1255] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(303), 8,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [1269] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(305), 8,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [1283] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(307), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1297] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(309), 8,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [1311] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(311), 8,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [1325] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(313), 8,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [1339] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(315), 8,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [1353] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(317), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1367] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(319), 1,
      anon_sym_RBRACE,
    STATE(62), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(321), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1384] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(324), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1397] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(63), 1,
      sym_tier_value,
    ACTIONS(326), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [1412] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(328), 1,
      anon_sym_RBRACE,
    STATE(69), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(330), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1429] = 2,
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
  [1442] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(334), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1455] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(336), 1,
      anon_sym_RBRACE,
    STATE(104), 1,
      sym__string_value,
    STATE(71), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(338), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1474] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(340), 1,
      anon_sym_RBRACE,
    STATE(62), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(330), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1491] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(342), 7,
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
    ACTIONS(344), 1,
      anon_sym_RBRACE,
    STATE(104), 1,
      sym__string_value,
    STATE(71), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(346), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1523] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(349), 1,
      anon_sym_RBRACE,
    STATE(104), 1,
      sym__string_value,
    STATE(68), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(338), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1542] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(351), 1,
      anon_sym_LBRACE,
    ACTIONS(353), 1,
      anon_sym_agent,
    ACTIONS(355), 1,
      anon_sym_command,
    STATE(55), 4,
      sym__until_condition,
      sym_until_verify,
      sym_until_agent,
      sym_until_command,
  [1561] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(357), 1,
      anon_sym_RBRACE,
    STATE(75), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(330), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1578] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(359), 1,
      anon_sym_RBRACE,
    STATE(62), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(330), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1595] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(361), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1608] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(363), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1621] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(365), 1,
      anon_sym_RBRACE,
    ACTIONS(369), 1,
      anon_sym_depth,
    ACTIONS(367), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(83), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1639] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(371), 1,
      anon_sym_RBRACE,
    ACTIONS(376), 1,
      anon_sym_impact_scope,
    ACTIONS(373), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(79), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1657] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(379), 1,
      anon_sym_RBRACE,
    ACTIONS(383), 1,
      anon_sym_impact_scope,
    ACTIONS(381), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(82), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1675] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(385), 1,
      anon_sym_RBRACE,
    ACTIONS(390), 1,
      anon_sym_depth,
    ACTIONS(387), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(81), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1693] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(383), 1,
      anon_sym_impact_scope,
    ACTIONS(393), 1,
      anon_sym_RBRACE,
    ACTIONS(381), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(79), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1711] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(369), 1,
      anon_sym_depth,
    ACTIONS(395), 1,
      anon_sym_RBRACE,
    ACTIONS(367), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(81), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1729] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(397), 1,
      anon_sym_loop,
    ACTIONS(399), 1,
      anon_sym_RBRACK,
    ACTIONS(401), 1,
      sym_identifier,
    STATE(87), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [1746] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(403), 1,
      anon_sym_RBRACK,
    ACTIONS(405), 2,
      sym_string,
      sym_raw_string,
    STATE(85), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [1761] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(408), 1,
      anon_sym_RBRACK,
    ACTIONS(410), 2,
      sym_string,
      sym_raw_string,
    STATE(88), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [1776] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(397), 1,
      anon_sym_loop,
    ACTIONS(412), 1,
      anon_sym_RBRACK,
    ACTIONS(414), 1,
      sym_identifier,
    STATE(89), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [1793] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(416), 1,
      anon_sym_RBRACK,
    ACTIONS(418), 2,
      sym_string,
      sym_raw_string,
    STATE(85), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [1808] = 5,
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
  [1825] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(428), 5,
      anon_sym_RBRACE,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1836] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(430), 4,
      anon_sym_RBRACE,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1846] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(20), 1,
      sym__string_value,
    ACTIONS(432), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1858] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(434), 1,
      anon_sym_RBRACE,
    ACTIONS(436), 1,
      sym_identifier,
    STATE(98), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [1872] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(438), 4,
      anon_sym_RBRACE,
      anon_sym_owned,
      anon_sym_read_only,
      anon_sym_impact_scope,
  [1882] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(440), 4,
      anon_sym_RBRACE,
      anon_sym_callers_of,
      anon_sym_tests_for,
      anon_sym_depth,
  [1892] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(41), 1,
      sym_strategy_value,
    ACTIONS(442), 3,
      anon_sym_single_pass,
      anon_sym_refine,
      sym_identifier,
  [1904] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(436), 1,
      sym_identifier,
    ACTIONS(444), 1,
      anon_sym_RBRACE,
    STATE(93), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [1918] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(446), 1,
      anon_sym_RBRACE,
    ACTIONS(448), 1,
      sym_identifier,
    STATE(98), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [1932] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(54), 1,
      sym_boolean,
    ACTIONS(451), 2,
      anon_sym_true,
      anon_sym_false,
  [1943] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(63), 1,
      sym_privacy_value,
    ACTIONS(453), 2,
      anon_sym_public,
      anon_sym_local_only,
  [1954] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(45), 1,
      sym__string_value,
    ACTIONS(455), 2,
      sym_string,
      sym_raw_string,
  [1965] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(90), 1,
      sym_boolean,
    ACTIONS(451), 2,
      anon_sym_true,
      anon_sym_false,
  [1976] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(48), 1,
      sym__string_value,
    ACTIONS(457), 2,
      sym_string,
      sym_raw_string,
  [1987] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(91), 1,
      sym__string_value,
    ACTIONS(459), 2,
      sym_string,
      sym_raw_string,
  [1998] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(461), 1,
      anon_sym_RBRACK,
    ACTIONS(463), 1,
      sym_identifier,
    STATE(106), 1,
      aux_sym_identifier_list_repeat1,
  [2011] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(465), 1,
      anon_sym_RBRACK,
    ACTIONS(467), 1,
      sym_identifier,
    STATE(107), 1,
      aux_sym_identifier_list_repeat1,
  [2024] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(469), 1,
      anon_sym_RBRACK,
    ACTIONS(471), 1,
      sym_identifier,
    STATE(107), 1,
      aux_sym_identifier_list_repeat1,
  [2037] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(20), 1,
      sym__string_value,
    ACTIONS(432), 2,
      sym_string,
      sym_raw_string,
  [2048] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(94), 1,
      sym_boolean,
    ACTIONS(451), 2,
      anon_sym_true,
      anon_sym_false,
  [2059] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(63), 1,
      sym__string_value,
    ACTIONS(165), 2,
      sym_string,
      sym_raw_string,
  [2070] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(476), 1,
      anon_sym_RBRACK,
    ACTIONS(474), 2,
      anon_sym_loop,
      sym_identifier,
  [2081] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(49), 1,
      sym__string_value,
    ACTIONS(478), 2,
      sym_string,
      sym_raw_string,
  [2092] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(59), 1,
      sym__string_value,
    ACTIONS(480), 2,
      sym_string,
      sym_raw_string,
  [2103] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(484), 1,
      anon_sym_RBRACK,
    ACTIONS(482), 2,
      anon_sym_loop,
      sym_identifier,
  [2114] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(41), 1,
      sym_boolean,
    ACTIONS(451), 2,
      anon_sym_true,
      anon_sym_false,
  [2125] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(117), 1,
      sym__string_value,
    ACTIONS(486), 2,
      sym_string,
      sym_raw_string,
  [2136] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(488), 2,
      anon_sym_RBRACE,
      sym_identifier,
  [2144] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(490), 1,
      anon_sym_LBRACK,
    STATE(20), 1,
      sym_string_list,
  [2154] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(490), 1,
      anon_sym_LBRACK,
    STATE(94), 1,
      sym_string_list,
  [2164] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(492), 1,
      anon_sym_LBRACK,
    STATE(54), 1,
      sym_identifier_list,
  [2174] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(490), 1,
      anon_sym_LBRACK,
    STATE(49), 1,
      sym_string_list,
  [2184] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(492), 1,
      anon_sym_LBRACK,
    STATE(20), 1,
      sym_identifier_list,
  [2194] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(494), 1,
      anon_sym_LBRACK,
    STATE(41), 1,
      sym_step_list,
  [2204] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(490), 1,
      anon_sym_LBRACK,
    STATE(95), 1,
      sym_string_list,
  [2214] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(432), 1,
      sym_integer,
  [2221] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(496), 1,
      anon_sym_LBRACE,
  [2228] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(498), 1,
      anon_sym_LBRACE,
  [2235] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(500), 1,
      anon_sym_LBRACE,
  [2242] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(502), 1,
      ts_builtin_sym_end,
  [2249] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(504), 1,
      anon_sym_LBRACE,
  [2256] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(506), 1,
      anon_sym_LBRACE,
  [2263] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(508), 1,
      anon_sym_LBRACE,
  [2270] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(510), 1,
      sym_identifier,
  [2277] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(512), 1,
      anon_sym_LBRACE,
  [2284] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(514), 1,
      anon_sym_LBRACE,
  [2291] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(516), 1,
      sym_integer,
  [2298] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(518), 1,
      sym_integer,
  [2305] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(520), 1,
      anon_sym_LBRACE,
  [2312] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(478), 1,
      sym_float,
  [2319] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(522), 1,
      sym_identifier,
  [2326] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(524), 1,
      sym_identifier,
  [2333] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(478), 1,
      sym_integer,
  [2340] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(526), 1,
      sym_identifier,
  [2347] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(528), 1,
      anon_sym_LBRACE,
  [2354] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(530), 1,
      sym_identifier,
  [2361] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(169), 1,
      sym_identifier,
  [2368] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(532), 1,
      sym_identifier,
  [2375] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(432), 1,
      sym_identifier,
  [2382] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(534), 1,
      sym_integer,
};

static const uint32_t ts_small_parse_table_map[] = {
  [SMALL_STATE(2)] = 0,
  [SMALL_STATE(3)] = 31,
  [SMALL_STATE(4)] = 62,
  [SMALL_STATE(5)] = 94,
  [SMALL_STATE(6)] = 119,
  [SMALL_STATE(7)] = 144,
  [SMALL_STATE(8)] = 169,
  [SMALL_STATE(9)] = 194,
  [SMALL_STATE(10)] = 244,
  [SMALL_STATE(11)] = 294,
  [SMALL_STATE(12)] = 344,
  [SMALL_STATE(13)] = 383,
  [SMALL_STATE(14)] = 422,
  [SMALL_STATE(15)] = 444,
  [SMALL_STATE(16)] = 466,
  [SMALL_STATE(17)] = 502,
  [SMALL_STATE(18)] = 538,
  [SMALL_STATE(19)] = 574,
  [SMALL_STATE(20)] = 592,
  [SMALL_STATE(21)] = 610,
  [SMALL_STATE(22)] = 634,
  [SMALL_STATE(23)] = 652,
  [SMALL_STATE(24)] = 670,
  [SMALL_STATE(25)] = 688,
  [SMALL_STATE(26)] = 706,
  [SMALL_STATE(27)] = 724,
  [SMALL_STATE(28)] = 753,
  [SMALL_STATE(29)] = 782,
  [SMALL_STATE(30)] = 811,
  [SMALL_STATE(31)] = 827,
  [SMALL_STATE(32)] = 859,
  [SMALL_STATE(33)] = 885,
  [SMALL_STATE(34)] = 917,
  [SMALL_STATE(35)] = 933,
  [SMALL_STATE(36)] = 965,
  [SMALL_STATE(37)] = 981,
  [SMALL_STATE(38)] = 997,
  [SMALL_STATE(39)] = 1023,
  [SMALL_STATE(40)] = 1049,
  [SMALL_STATE(41)] = 1065,
  [SMALL_STATE(42)] = 1081,
  [SMALL_STATE(43)] = 1097,
  [SMALL_STATE(44)] = 1113,
  [SMALL_STATE(45)] = 1127,
  [SMALL_STATE(46)] = 1141,
  [SMALL_STATE(47)] = 1155,
  [SMALL_STATE(48)] = 1169,
  [SMALL_STATE(49)] = 1183,
  [SMALL_STATE(50)] = 1197,
  [SMALL_STATE(51)] = 1211,
  [SMALL_STATE(52)] = 1227,
  [SMALL_STATE(53)] = 1241,
  [SMALL_STATE(54)] = 1255,
  [SMALL_STATE(55)] = 1269,
  [SMALL_STATE(56)] = 1283,
  [SMALL_STATE(57)] = 1297,
  [SMALL_STATE(58)] = 1311,
  [SMALL_STATE(59)] = 1325,
  [SMALL_STATE(60)] = 1339,
  [SMALL_STATE(61)] = 1353,
  [SMALL_STATE(62)] = 1367,
  [SMALL_STATE(63)] = 1384,
  [SMALL_STATE(64)] = 1397,
  [SMALL_STATE(65)] = 1412,
  [SMALL_STATE(66)] = 1429,
  [SMALL_STATE(67)] = 1442,
  [SMALL_STATE(68)] = 1455,
  [SMALL_STATE(69)] = 1474,
  [SMALL_STATE(70)] = 1491,
  [SMALL_STATE(71)] = 1504,
  [SMALL_STATE(72)] = 1523,
  [SMALL_STATE(73)] = 1542,
  [SMALL_STATE(74)] = 1561,
  [SMALL_STATE(75)] = 1578,
  [SMALL_STATE(76)] = 1595,
  [SMALL_STATE(77)] = 1608,
  [SMALL_STATE(78)] = 1621,
  [SMALL_STATE(79)] = 1639,
  [SMALL_STATE(80)] = 1657,
  [SMALL_STATE(81)] = 1675,
  [SMALL_STATE(82)] = 1693,
  [SMALL_STATE(83)] = 1711,
  [SMALL_STATE(84)] = 1729,
  [SMALL_STATE(85)] = 1746,
  [SMALL_STATE(86)] = 1761,
  [SMALL_STATE(87)] = 1776,
  [SMALL_STATE(88)] = 1793,
  [SMALL_STATE(89)] = 1808,
  [SMALL_STATE(90)] = 1825,
  [SMALL_STATE(91)] = 1836,
  [SMALL_STATE(92)] = 1846,
  [SMALL_STATE(93)] = 1858,
  [SMALL_STATE(94)] = 1872,
  [SMALL_STATE(95)] = 1882,
  [SMALL_STATE(96)] = 1892,
  [SMALL_STATE(97)] = 1904,
  [SMALL_STATE(98)] = 1918,
  [SMALL_STATE(99)] = 1932,
  [SMALL_STATE(100)] = 1943,
  [SMALL_STATE(101)] = 1954,
  [SMALL_STATE(102)] = 1965,
  [SMALL_STATE(103)] = 1976,
  [SMALL_STATE(104)] = 1987,
  [SMALL_STATE(105)] = 1998,
  [SMALL_STATE(106)] = 2011,
  [SMALL_STATE(107)] = 2024,
  [SMALL_STATE(108)] = 2037,
  [SMALL_STATE(109)] = 2048,
  [SMALL_STATE(110)] = 2059,
  [SMALL_STATE(111)] = 2070,
  [SMALL_STATE(112)] = 2081,
  [SMALL_STATE(113)] = 2092,
  [SMALL_STATE(114)] = 2103,
  [SMALL_STATE(115)] = 2114,
  [SMALL_STATE(116)] = 2125,
  [SMALL_STATE(117)] = 2136,
  [SMALL_STATE(118)] = 2144,
  [SMALL_STATE(119)] = 2154,
  [SMALL_STATE(120)] = 2164,
  [SMALL_STATE(121)] = 2174,
  [SMALL_STATE(122)] = 2184,
  [SMALL_STATE(123)] = 2194,
  [SMALL_STATE(124)] = 2204,
  [SMALL_STATE(125)] = 2214,
  [SMALL_STATE(126)] = 2221,
  [SMALL_STATE(127)] = 2228,
  [SMALL_STATE(128)] = 2235,
  [SMALL_STATE(129)] = 2242,
  [SMALL_STATE(130)] = 2249,
  [SMALL_STATE(131)] = 2256,
  [SMALL_STATE(132)] = 2263,
  [SMALL_STATE(133)] = 2270,
  [SMALL_STATE(134)] = 2277,
  [SMALL_STATE(135)] = 2284,
  [SMALL_STATE(136)] = 2291,
  [SMALL_STATE(137)] = 2298,
  [SMALL_STATE(138)] = 2305,
  [SMALL_STATE(139)] = 2312,
  [SMALL_STATE(140)] = 2319,
  [SMALL_STATE(141)] = 2326,
  [SMALL_STATE(142)] = 2333,
  [SMALL_STATE(143)] = 2340,
  [SMALL_STATE(144)] = 2347,
  [SMALL_STATE(145)] = 2354,
  [SMALL_STATE(146)] = 2361,
  [SMALL_STATE(147)] = 2368,
  [SMALL_STATE(148)] = 2375,
  [SMALL_STATE(149)] = 2382,
};

static const TSParseActionEntry ts_parse_actions[] = {
  [0] = {.entry = {.count = 0, .reusable = false}},
  [1] = {.entry = {.count = 1, .reusable = false}}, RECOVER(),
  [3] = {.entry = {.count = 1, .reusable = true}}, SHIFT_EXTRA(),
  [5] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 0, 0, 0),
  [7] = {.entry = {.count = 1, .reusable = true}}, SHIFT(103),
  [9] = {.entry = {.count = 1, .reusable = true}}, SHIFT(143),
  [11] = {.entry = {.count = 1, .reusable = true}}, SHIFT(51),
  [13] = {.entry = {.count = 1, .reusable = true}}, SHIFT(132),
  [15] = {.entry = {.count = 1, .reusable = true}}, SHIFT(133),
  [17] = {.entry = {.count = 1, .reusable = true}}, SHIFT(145),
  [19] = {.entry = {.count = 1, .reusable = true}}, SHIFT(147),
  [21] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 2, 0, 0),
  [23] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 3, 0, 0),
  [25] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_boolean, 1, 0, 0),
  [27] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_boolean, 1, 0, 0),
  [29] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_block, 3, 0, 0),
  [31] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 2, 0, 0),
  [33] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_block, 4, 0, 0),
  [35] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 3, 0, 0),
  [37] = {.entry = {.count = 1, .reusable = true}}, SHIFT(148),
  [39] = {.entry = {.count = 1, .reusable = true}}, SHIFT(50),
  [41] = {.entry = {.count = 1, .reusable = true}}, SHIFT(43),
  [43] = {.entry = {.count = 1, .reusable = true}}, SHIFT(92),
  [45] = {.entry = {.count = 1, .reusable = true}}, SHIFT(108),
  [47] = {.entry = {.count = 1, .reusable = true}}, SHIFT(122),
  [49] = {.entry = {.count = 1, .reusable = true}}, SHIFT(125),
  [51] = {.entry = {.count = 1, .reusable = true}}, SHIFT(118),
  [53] = {.entry = {.count = 1, .reusable = true}}, SHIFT(126),
  [55] = {.entry = {.count = 1, .reusable = true}}, SHIFT(127),
  [57] = {.entry = {.count = 1, .reusable = true}}, SHIFT(130),
  [59] = {.entry = {.count = 1, .reusable = true}}, SHIFT(56),
  [61] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(148),
  [64] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0),
  [66] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(43),
  [69] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(132),
  [72] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(92),
  [75] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(108),
  [78] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(122),
  [81] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(125),
  [84] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(118),
  [87] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(126),
  [90] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(127),
  [93] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(130),
  [96] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0),
  [98] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(103),
  [101] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(143),
  [104] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(51),
  [107] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(132),
  [110] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(133),
  [113] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(145),
  [116] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(147),
  [119] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 1, 0, 0),
  [121] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_block, 3, 0, 0),
  [123] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_block, 4, 0, 0),
  [125] = {.entry = {.count = 1, .reusable = true}}, SHIFT(47),
  [127] = {.entry = {.count = 1, .reusable = true}}, SHIFT(137),
  [129] = {.entry = {.count = 1, .reusable = true}}, SHIFT(144),
  [131] = {.entry = {.count = 1, .reusable = true}}, SHIFT(123),
  [133] = {.entry = {.count = 1, .reusable = true}}, SHIFT(96),
  [135] = {.entry = {.count = 1, .reusable = true}}, SHIFT(115),
  [137] = {.entry = {.count = 1, .reusable = true}}, SHIFT(61),
  [139] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0),
  [141] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(137),
  [144] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(127),
  [147] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(144),
  [150] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(123),
  [153] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(96),
  [156] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(115),
  [159] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 1, 0, 0),
  [161] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 2, 0, 0),
  [163] = {.entry = {.count = 1, .reusable = false}}, SHIFT(76),
  [165] = {.entry = {.count = 1, .reusable = true}}, SHIFT(63),
  [167] = {.entry = {.count = 1, .reusable = false}}, SHIFT(63),
  [169] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_alias_name, 1, 0, 0),
  [171] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_block, 3, 0, 0),
  [173] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 4, 0, 0),
  [175] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 3, 0, 0),
  [177] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_block, 4, 0, 0),
  [179] = {.entry = {.count = 1, .reusable = true}}, SHIFT(111),
  [181] = {.entry = {.count = 1, .reusable = true}}, SHIFT(120),
  [183] = {.entry = {.count = 1, .reusable = true}}, SHIFT(136),
  [185] = {.entry = {.count = 1, .reusable = true}}, SHIFT(99),
  [187] = {.entry = {.count = 1, .reusable = true}}, SHIFT(73),
  [189] = {.entry = {.count = 1, .reusable = true}}, SHIFT(114),
  [191] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0),
  [193] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(120),
  [196] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(136),
  [199] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(99),
  [202] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(73),
  [205] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_step_list, 3, 0, 0),
  [207] = {.entry = {.count = 1, .reusable = true}}, SHIFT(46),
  [209] = {.entry = {.count = 1, .reusable = true}}, SHIFT(64),
  [211] = {.entry = {.count = 1, .reusable = true}}, SHIFT(110),
  [213] = {.entry = {.count = 1, .reusable = true}}, SHIFT(21),
  [215] = {.entry = {.count = 1, .reusable = true}}, SHIFT(100),
  [217] = {.entry = {.count = 1, .reusable = true}}, SHIFT(66),
  [219] = {.entry = {.count = 1, .reusable = true}}, SHIFT(128),
  [221] = {.entry = {.count = 1, .reusable = true}}, SHIFT(7),
  [223] = {.entry = {.count = 1, .reusable = true}}, SHIFT(121),
  [225] = {.entry = {.count = 1, .reusable = true}}, SHIFT(112),
  [227] = {.entry = {.count = 1, .reusable = true}}, SHIFT(139),
  [229] = {.entry = {.count = 1, .reusable = true}}, SHIFT(142),
  [231] = {.entry = {.count = 1, .reusable = true}}, SHIFT(44),
  [233] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_block, 3, 0, 0),
  [235] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0),
  [237] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(64),
  [240] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(110),
  [243] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(21),
  [246] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(100),
  [249] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(66),
  [252] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(128),
  [255] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 1, 0, 0),
  [257] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_step_list, 2, 0, 0),
  [259] = {.entry = {.count = 1, .reusable = true}}, SHIFT(5),
  [261] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0),
  [263] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(121),
  [266] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(112),
  [269] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(139),
  [272] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(142),
  [275] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_block, 4, 0, 0),
  [277] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 2, 0, 0),
  [279] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_strategy_value, 1, 0, 0),
  [281] = {.entry = {.count = 1, .reusable = false}}, SHIFT(22),
  [283] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 4, 0, 0),
  [285] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_prompt_declaration, 3, 0, 0),
  [287] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 5, 0, 0),
  [289] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 4, 0, 0),
  [291] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_include_declaration, 2, 0, 0),
  [293] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_field, 2, 0, 0),
  [295] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 5, 0, 0),
  [297] = {.entry = {.count = 1, .reusable = false}}, SHIFT(146),
  [299] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_field, 1, 0, 0),
  [301] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_alias_declaration, 3, 0, 0),
  [303] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_field, 2, 0, 0),
  [305] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_clause, 2, 0, 0),
  [307] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 4, 0, 0),
  [309] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_verify, 2, 0, 0),
  [311] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_agent, 2, 0, 0),
  [313] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_command, 2, 0, 0),
  [315] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_verify, 3, 0, 0),
  [317] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 5, 0, 0),
  [319] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_verify_block_repeat1, 2, 0, 0),
  [321] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_verify_block_repeat1, 2, 0, 0), SHIFT_REPEAT(102),
  [324] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_field, 2, 0, 0),
  [326] = {.entry = {.count = 1, .reusable = true}}, SHIFT(76),
  [328] = {.entry = {.count = 1, .reusable = true}}, SHIFT(34),
  [330] = {.entry = {.count = 1, .reusable = true}}, SHIFT(102),
  [332] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_field, 1, 0, 0),
  [334] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_block, 3, 0, 0),
  [336] = {.entry = {.count = 1, .reusable = true}}, SHIFT(70),
  [338] = {.entry = {.count = 1, .reusable = true}}, SHIFT(104),
  [340] = {.entry = {.count = 1, .reusable = true}}, SHIFT(40),
  [342] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_block, 4, 0, 0),
  [344] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_extra_block_repeat1, 2, 0, 0),
  [346] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_extra_block_repeat1, 2, 0, 0), SHIFT_REPEAT(104),
  [349] = {.entry = {.count = 1, .reusable = true}}, SHIFT(67),
  [351] = {.entry = {.count = 1, .reusable = true}}, SHIFT(74),
  [353] = {.entry = {.count = 1, .reusable = true}}, SHIFT(140),
  [355] = {.entry = {.count = 1, .reusable = true}}, SHIFT(113),
  [357] = {.entry = {.count = 1, .reusable = true}}, SHIFT(57),
  [359] = {.entry = {.count = 1, .reusable = true}}, SHIFT(60),
  [361] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_value, 1, 0, 0),
  [363] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_privacy_value, 1, 0, 0),
  [365] = {.entry = {.count = 1, .reusable = true}}, SHIFT(23),
  [367] = {.entry = {.count = 1, .reusable = true}}, SHIFT(124),
  [369] = {.entry = {.count = 1, .reusable = true}}, SHIFT(149),
  [371] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0),
  [373] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(119),
  [376] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(109),
  [379] = {.entry = {.count = 1, .reusable = true}}, SHIFT(25),
  [381] = {.entry = {.count = 1, .reusable = true}}, SHIFT(119),
  [383] = {.entry = {.count = 1, .reusable = true}}, SHIFT(109),
  [385] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0),
  [387] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0), SHIFT_REPEAT(124),
  [390] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0), SHIFT_REPEAT(149),
  [393] = {.entry = {.count = 1, .reusable = true}}, SHIFT(24),
  [395] = {.entry = {.count = 1, .reusable = true}}, SHIFT(26),
  [397] = {.entry = {.count = 1, .reusable = false}}, SHIFT(135),
  [399] = {.entry = {.count = 1, .reusable = true}}, SHIFT(37),
  [401] = {.entry = {.count = 1, .reusable = false}}, SHIFT(87),
  [403] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0),
  [405] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0), SHIFT_REPEAT(85),
  [408] = {.entry = {.count = 1, .reusable = true}}, SHIFT(2),
  [410] = {.entry = {.count = 1, .reusable = true}}, SHIFT(88),
  [412] = {.entry = {.count = 1, .reusable = true}}, SHIFT(30),
  [414] = {.entry = {.count = 1, .reusable = false}}, SHIFT(89),
  [416] = {.entry = {.count = 1, .reusable = true}}, SHIFT(3),
  [418] = {.entry = {.count = 1, .reusable = true}}, SHIFT(85),
  [420] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0), SHIFT_REPEAT(135),
  [423] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0),
  [425] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0), SHIFT_REPEAT(89),
  [428] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_field, 2, 0, 0),
  [430] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_pair, 2, 0, 0),
  [432] = {.entry = {.count = 1, .reusable = true}}, SHIFT(20),
  [434] = {.entry = {.count = 1, .reusable = true}}, SHIFT(15),
  [436] = {.entry = {.count = 1, .reusable = true}}, SHIFT(116),
  [438] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_field, 2, 0, 0),
  [440] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_field, 2, 0, 0),
  [442] = {.entry = {.count = 1, .reusable = false}}, SHIFT(42),
  [444] = {.entry = {.count = 1, .reusable = true}}, SHIFT(14),
  [446] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_vars_block_repeat1, 2, 0, 0),
  [448] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_vars_block_repeat1, 2, 0, 0), SHIFT_REPEAT(116),
  [451] = {.entry = {.count = 1, .reusable = true}}, SHIFT(4),
  [453] = {.entry = {.count = 1, .reusable = true}}, SHIFT(77),
  [455] = {.entry = {.count = 1, .reusable = true}}, SHIFT(45),
  [457] = {.entry = {.count = 1, .reusable = true}}, SHIFT(48),
  [459] = {.entry = {.count = 1, .reusable = true}}, SHIFT(91),
  [461] = {.entry = {.count = 1, .reusable = true}}, SHIFT(6),
  [463] = {.entry = {.count = 1, .reusable = true}}, SHIFT(106),
  [465] = {.entry = {.count = 1, .reusable = true}}, SHIFT(8),
  [467] = {.entry = {.count = 1, .reusable = true}}, SHIFT(107),
  [469] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0),
  [471] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0), SHIFT_REPEAT(107),
  [474] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_loop_block, 3, 0, 0),
  [476] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_block, 3, 0, 0),
  [478] = {.entry = {.count = 1, .reusable = true}}, SHIFT(49),
  [480] = {.entry = {.count = 1, .reusable = true}}, SHIFT(59),
  [482] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_loop_block, 4, 0, 0),
  [484] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_block, 4, 0, 0),
  [486] = {.entry = {.count = 1, .reusable = true}}, SHIFT(117),
  [488] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_pair, 2, 0, 0),
  [490] = {.entry = {.count = 1, .reusable = true}}, SHIFT(86),
  [492] = {.entry = {.count = 1, .reusable = true}}, SHIFT(105),
  [494] = {.entry = {.count = 1, .reusable = true}}, SHIFT(84),
  [496] = {.entry = {.count = 1, .reusable = true}}, SHIFT(80),
  [498] = {.entry = {.count = 1, .reusable = true}}, SHIFT(38),
  [500] = {.entry = {.count = 1, .reusable = true}}, SHIFT(72),
  [502] = {.entry = {.count = 1, .reusable = true}},  ACCEPT_INPUT(),
  [504] = {.entry = {.count = 1, .reusable = true}}, SHIFT(78),
  [506] = {.entry = {.count = 1, .reusable = true}}, SHIFT(16),
  [508] = {.entry = {.count = 1, .reusable = true}}, SHIFT(97),
  [510] = {.entry = {.count = 1, .reusable = true}}, SHIFT(101),
  [512] = {.entry = {.count = 1, .reusable = true}}, SHIFT(33),
  [514] = {.entry = {.count = 1, .reusable = true}}, SHIFT(27),
  [516] = {.entry = {.count = 1, .reusable = true}}, SHIFT(54),
  [518] = {.entry = {.count = 1, .reusable = true}}, SHIFT(41),
  [520] = {.entry = {.count = 1, .reusable = true}}, SHIFT(10),
  [522] = {.entry = {.count = 1, .reusable = true}}, SHIFT(58),
  [524] = {.entry = {.count = 1, .reusable = true}}, SHIFT(53),
  [526] = {.entry = {.count = 1, .reusable = true}}, SHIFT(134),
  [528] = {.entry = {.count = 1, .reusable = true}}, SHIFT(65),
  [530] = {.entry = {.count = 1, .reusable = true}}, SHIFT(138),
  [532] = {.entry = {.count = 1, .reusable = true}}, SHIFT(131),
  [534] = {.entry = {.count = 1, .reusable = true}}, SHIFT(95),
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
