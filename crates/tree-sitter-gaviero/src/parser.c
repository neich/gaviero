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
#define STATE_COUNT 147
#define LARGE_STATE_COUNT 2
#define SYMBOL_COUNT 126
#define ALIAS_COUNT 0
#define TOKEN_COUNT 73
#define EXTERNAL_TOKEN_COUNT 0
#define FIELD_COUNT 0
#define MAX_ALIAS_SEQUENCE_LENGTH 5
#define PRODUCTION_ID_COUNT 1

enum ts_symbol_identifiers {
  sym_comment = 1,
  anon_sym_client = 2,
  anon_sym_LBRACE = 3,
  anon_sym_RBRACE = 4,
  anon_sym_tier = 5,
  anon_sym_model = 6,
  anon_sym_effort = 7,
  anon_sym_privacy = 8,
  anon_sym_default = 9,
  anon_sym_extra = 10,
  anon_sym_vars = 11,
  anon_sym_cheap = 12,
  anon_sym_expensive = 13,
  anon_sym_coordinator = 14,
  anon_sym_reasoning = 15,
  anon_sym_execution = 16,
  anon_sym_mechanical = 17,
  anon_sym_prompt = 18,
  anon_sym_agent = 19,
  anon_sym_description = 20,
  anon_sym_depends_on = 21,
  anon_sym_max_retries = 22,
  anon_sym_scope = 23,
  anon_sym_owned = 24,
  anon_sym_read_only = 25,
  anon_sym_impact_scope = 26,
  anon_sym_memory = 27,
  anon_sym_read_ns = 28,
  anon_sym_write_ns = 29,
  anon_sym_importance = 30,
  anon_sym_staleness_sources = 31,
  anon_sym_read_query = 32,
  anon_sym_read_limit = 33,
  anon_sym_write_content = 34,
  anon_sym_verify = 35,
  anon_sym_compile = 36,
  anon_sym_clippy = 37,
  anon_sym_test = 38,
  anon_sym_impact_tests = 39,
  anon_sym_context = 40,
  anon_sym_callers_of = 41,
  anon_sym_tests_for = 42,
  anon_sym_depth = 43,
  anon_sym_loop = 44,
  anon_sym_agents = 45,
  anon_sym_max_iterations = 46,
  anon_sym_iter_start = 47,
  anon_sym_stability = 48,
  anon_sym_judge_timeout = 49,
  anon_sym_strict_judge = 50,
  anon_sym_until = 51,
  anon_sym_command = 52,
  anon_sym_workflow = 53,
  anon_sym_steps = 54,
  anon_sym_max_parallel = 55,
  anon_sym_strategy = 56,
  anon_sym_test_first = 57,
  anon_sym_attempts = 58,
  anon_sym_escalate_after = 59,
  anon_sym_LBRACK = 60,
  anon_sym_RBRACK = 61,
  anon_sym_public = 62,
  anon_sym_local_only = 63,
  anon_sym_single_pass = 64,
  anon_sym_refine = 65,
  anon_sym_true = 66,
  anon_sym_false = 67,
  sym_string = 68,
  sym_raw_string = 69,
  sym_float = 70,
  sym_integer = 71,
  sym_identifier = 72,
  sym_source_file = 73,
  sym__definition = 74,
  sym_client_declaration = 75,
  sym_client_field = 76,
  sym__effort_value = 77,
  sym_extra_block = 78,
  sym_extra_pair = 79,
  sym_vars_block = 80,
  sym_vars_pair = 81,
  sym_tier_alias_declaration = 82,
  sym_tier_alias_name = 83,
  sym_prompt_declaration = 84,
  sym_agent_declaration = 85,
  sym_agent_field = 86,
  sym_scope_block = 87,
  sym_scope_field = 88,
  sym_memory_block = 89,
  sym_memory_field = 90,
  sym_verify_block = 91,
  sym_verify_field = 92,
  sym_context_block = 93,
  sym_context_field = 94,
  sym_loop_block = 95,
  sym_loop_field = 96,
  sym_until_clause = 97,
  sym__until_condition = 98,
  sym_until_verify = 99,
  sym_until_agent = 100,
  sym_until_command = 101,
  sym_workflow_declaration = 102,
  sym_workflow_field = 103,
  sym_step_list = 104,
  sym_string_list = 105,
  sym_identifier_list = 106,
  sym_tier_value = 107,
  sym_privacy_value = 108,
  sym_strategy_value = 109,
  sym_boolean = 110,
  sym__string_value = 111,
  aux_sym_source_file_repeat1 = 112,
  aux_sym_client_declaration_repeat1 = 113,
  aux_sym_extra_block_repeat1 = 114,
  aux_sym_vars_block_repeat1 = 115,
  aux_sym_agent_declaration_repeat1 = 116,
  aux_sym_scope_block_repeat1 = 117,
  aux_sym_memory_block_repeat1 = 118,
  aux_sym_verify_block_repeat1 = 119,
  aux_sym_context_block_repeat1 = 120,
  aux_sym_loop_block_repeat1 = 121,
  aux_sym_workflow_declaration_repeat1 = 122,
  aux_sym_step_list_repeat1 = 123,
  aux_sym_string_list_repeat1 = 124,
  aux_sym_identifier_list_repeat1 = 125,
};

static const char * const ts_symbol_names[] = {
  [ts_builtin_sym_end] = "end",
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
  [126] = 29,
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
};

static bool ts_lex(TSLexer *lexer, TSStateId state) {
  START_LEXER();
  eof = lexer->eof(lexer);
  switch (state) {
    case 0:
      if (eof) ADVANCE(422);
      ADVANCE_MAP(
        '"', 3,
        '#', 5,
        '/', 13,
        '[', 491,
        ']', 492,
        'a', 164,
        'c', 33,
        'd', 102,
        'e', 154,
        'f', 41,
        'i', 223,
        'j', 397,
        'l', 261,
        'm', 35,
        'o', 408,
        'p', 301,
        'r', 103,
        's', 75,
        't', 104,
        'u', 239,
        'v', 37,
        'w', 265,
        '{', 425,
        '}', 426,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(0);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(504);
      END_STATE();
    case 1:
      if (lookahead == '"') ADVANCE(3);
      if (lookahead == '#') ADVANCE(5);
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == ']') ADVANCE(492);
      if (lookahead == '}') ADVANCE(426);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(1);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 2:
      if (lookahead == '"') ADVANCE(3);
      if (lookahead == '#') ADVANCE(5);
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'c') ADVANCE(529);
      if (lookahead == 'e') ADVANCE(567);
      if (lookahead == 'm') ADVANCE(517);
      if (lookahead == 'r') ADVANCE(518);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(2);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 3:
      if (lookahead == '"') ADVANCE(501);
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
      if (lookahead == '#') ADVANCE(502);
      if (lookahead != 0) ADVANCE(4);
      END_STATE();
    case 7:
      if (lookahead == '.') ADVANCE(421);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(7);
      END_STATE();
    case 8:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == ']') ADVANCE(492);
      if (lookahead == 'l') ADVANCE(552);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(8);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 9:
      ADVANCE_MAP(
        '/', 13,
        'a', 355,
        'c', 34,
        'd', 117,
        'e', 326,
        'i', 235,
        'm', 60,
        'o', 408,
        'p', 319,
        'r', 138,
        's', 77,
        't', 151,
        'v', 37,
        'w', 307,
        '}', 426,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(9);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(7);
      END_STATE();
    case 10:
      ADVANCE_MAP(
        '/', 13,
        'a', 169,
        'c', 200,
        'd', 114,
        'e', 326,
        'i', 233,
        'j', 397,
        'm', 36,
        'o', 408,
        'p', 319,
        'r', 134,
        's', 76,
        't', 149,
        'u', 239,
        'v', 37,
        '}', 426,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(10);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(505);
      END_STATE();
    case 11:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'c') ADVANCE(220);
      if (lookahead == 'i') ADVANCE(234);
      if (lookahead == 't') ADVANCE(152);
      if (lookahead == '}') ADVANCE(426);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(11);
      END_STATE();
    case 12:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'r') ADVANCE(521);
      if (lookahead == 's') ADVANCE(535);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(12);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 13:
      if (lookahead == '/') ADVANCE(423);
      END_STATE();
    case 14:
      if (lookahead == '_') ADVANCE(193);
      END_STATE();
    case 15:
      if (lookahead == '_') ADVANCE(215);
      END_STATE();
    case 16:
      if (lookahead == '_') ADVANCE(80);
      END_STATE();
    case 17:
      if (lookahead == '_') ADVANCE(340);
      END_STATE();
    case 18:
      if (lookahead == '_') ADVANCE(198);
      END_STATE();
    case 19:
      if (lookahead == '_') ADVANCE(290);
      END_STATE();
    case 20:
      if (lookahead == '_') ADVANCE(160);
      if (lookahead == 's') ADVANCE(32);
      END_STATE();
    case 21:
      if (lookahead == '_') ADVANCE(267);
      END_STATE();
    case 22:
      if (lookahead == '_') ADVANCE(293);
      END_STATE();
    case 23:
      if (lookahead == '_') ADVANCE(51);
      END_STATE();
    case 24:
      if (lookahead == '_') ADVANCE(341);
      END_STATE();
    case 25:
      if (lookahead == '_') ADVANCE(339);
      END_STATE();
    case 26:
      if (lookahead == '_') ADVANCE(345);
      END_STATE();
    case 27:
      if (lookahead == '_') ADVANCE(280);
      END_STATE();
    case 28:
      if (lookahead == '_') ADVANCE(382);
      END_STATE();
    case 29:
      if (lookahead == '_') ADVANCE(273);
      END_STATE();
    case 30:
      if (lookahead == '_') ADVANCE(269);
      END_STATE();
    case 31:
      if (lookahead == '_') ADVANCE(386);
      END_STATE();
    case 32:
      if (lookahead == '_') ADVANCE(161);
      END_STATE();
    case 33:
      if (lookahead == 'a') ADVANCE(206);
      if (lookahead == 'h') ADVANCE(115);
      if (lookahead == 'l') ADVANCE(172);
      if (lookahead == 'o') ADVANCE(224);
      END_STATE();
    case 34:
      if (lookahead == 'a') ADVANCE(206);
      if (lookahead == 'l') ADVANCE(197);
      if (lookahead == 'o') ADVANCE(259);
      END_STATE();
    case 35:
      if (lookahead == 'a') ADVANCE(409);
      if (lookahead == 'e') ADVANCE(73);
      if (lookahead == 'o') ADVANCE(98);
      END_STATE();
    case 36:
      if (lookahead == 'a') ADVANCE(409);
      if (lookahead == 'e') ADVANCE(231);
      END_STATE();
    case 37:
      if (lookahead == 'a') ADVANCE(311);
      if (lookahead == 'e') ADVANCE(313);
      END_STATE();
    case 38:
      if (lookahead == 'a') ADVANCE(96);
      if (lookahead == 'f') ADVANCE(188);
      END_STATE();
    case 39:
      if (lookahead == 'a') ADVANCE(72);
      if (lookahead == 'e') ADVANCE(287);
      if (lookahead == 'r') ADVANCE(63);
      END_STATE();
    case 40:
      if (lookahead == 'a') ADVANCE(432);
      END_STATE();
    case 41:
      if (lookahead == 'a') ADVANCE(205);
      END_STATE();
    case 42:
      if (lookahead == 'a') ADVANCE(71);
      if (lookahead == 'e') ADVANCE(287);
      if (lookahead == 'r') ADVANCE(63);
      END_STATE();
    case 43:
      if (lookahead == 'a') ADVANCE(398);
      END_STATE();
    case 44:
      if (lookahead == 'a') ADVANCE(81);
      END_STATE();
    case 45:
      if (lookahead == 'a') ADVANCE(81);
      if (lookahead == 'o') ADVANCE(316);
      END_STATE();
    case 46:
      if (lookahead == 'a') ADVANCE(210);
      END_STATE();
    case 47:
      if (lookahead == 'a') ADVANCE(79);
      END_STATE();
    case 48:
      if (lookahead == 'a') ADVANCE(285);
      END_STATE();
    case 49:
      if (lookahead == 'a') ADVANCE(252);
      END_STATE();
    case 50:
      if (lookahead == 'a') ADVANCE(208);
      END_STATE();
    case 51:
      if (lookahead == 'a') ADVANCE(162);
      END_STATE();
    case 52:
      if (lookahead == 'a') ADVANCE(101);
      END_STATE();
    case 53:
      if (lookahead == 'a') ADVANCE(95);
      END_STATE();
    case 54:
      if (lookahead == 'a') ADVANCE(247);
      END_STATE();
    case 55:
      if (lookahead == 'a') ADVANCE(325);
      END_STATE();
    case 56:
      if (lookahead == 'a') ADVANCE(347);
      END_STATE();
    case 57:
      if (lookahead == 'a') ADVANCE(203);
      END_STATE();
    case 58:
      if (lookahead == 'a') ADVANCE(321);
      END_STATE();
    case 59:
      if (lookahead == 'a') ADVANCE(392);
      END_STATE();
    case 60:
      if (lookahead == 'a') ADVANCE(411);
      if (lookahead == 'e') ADVANCE(231);
      END_STATE();
    case 61:
      if (lookahead == 'a') ADVANCE(246);
      END_STATE();
    case 62:
      if (lookahead == 'a') ADVANCE(384);
      END_STATE();
    case 63:
      if (lookahead == 'a') ADVANCE(384);
      if (lookahead == 'i') ADVANCE(86);
      END_STATE();
    case 64:
      if (lookahead == 'a') ADVANCE(216);
      if (lookahead == 'e') ADVANCE(287);
      if (lookahead == 'r') ADVANCE(62);
      END_STATE();
    case 65:
      if (lookahead == 'a') ADVANCE(385);
      END_STATE();
    case 66:
      if (lookahead == 'a') ADVANCE(222);
      END_STATE();
    case 67:
      if (lookahead == 'a') ADVANCE(88);
      if (lookahead == 'o') ADVANCE(316);
      END_STATE();
    case 68:
      if (lookahead == 'a') ADVANCE(396);
      END_STATE();
    case 69:
      if (lookahead == 'a') ADVANCE(89);
      END_STATE();
    case 70:
      if (lookahead == 'b') ADVANCE(211);
      END_STATE();
    case 71:
      if (lookahead == 'b') ADVANCE(189);
      END_STATE();
    case 72:
      if (lookahead == 'b') ADVANCE(189);
      if (lookahead == 'l') ADVANCE(146);
      END_STATE();
    case 73:
      if (lookahead == 'c') ADVANCE(171);
      if (lookahead == 'm') ADVANCE(271);
      END_STATE();
    case 74:
      if (lookahead == 'c') ADVANCE(493);
      END_STATE();
    case 75:
      if (lookahead == 'c') ADVANCE(264);
      if (lookahead == 'i') ADVANCE(240);
      if (lookahead == 't') ADVANCE(39);
      END_STATE();
    case 76:
      if (lookahead == 'c') ADVANCE(264);
      if (lookahead == 't') ADVANCE(42);
      END_STATE();
    case 77:
      if (lookahead == 'c') ADVANCE(264);
      if (lookahead == 't') ADVANCE(64);
      END_STATE();
    case 78:
      if (lookahead == 'c') ADVANCE(402);
      END_STATE();
    case 79:
      if (lookahead == 'c') ADVANCE(415);
      END_STATE();
    case 80:
      if (lookahead == 'c') ADVANCE(281);
      if (lookahead == 'n') ADVANCE(331);
      END_STATE();
    case 81:
      if (lookahead == 'c') ADVANCE(374);
      END_STATE();
    case 82:
      if (lookahead == 'c') ADVANCE(111);
      END_STATE();
    case 83:
      if (lookahead == 'c') ADVANCE(140);
      END_STATE();
    case 84:
      if (lookahead == 'c') ADVANCE(46);
      END_STATE();
    case 85:
      if (lookahead == 'c') ADVANCE(320);
      END_STATE();
    case 86:
      if (lookahead == 'c') ADVANCE(376);
      END_STATE();
    case 87:
      if (lookahead == 'c') ADVANCE(50);
      if (lookahead == 'o') ADVANCE(284);
      END_STATE();
    case 88:
      if (lookahead == 'c') ADVANCE(388);
      END_STATE();
    case 89:
      if (lookahead == 'c') ADVANCE(379);
      END_STATE();
    case 90:
      if (lookahead == 'c') ADVANCE(57);
      END_STATE();
    case 91:
      if (lookahead == 'c') ADVANCE(277);
      END_STATE();
    case 92:
      if (lookahead == 'd') ADVANCE(166);
      END_STATE();
    case 93:
      if (lookahead == 'd') ADVANCE(452);
      END_STATE();
    case 94:
      if (lookahead == 'd') ADVANCE(483);
      END_STATE();
    case 95:
      if (lookahead == 'd') ADVANCE(15);
      END_STATE();
    case 96:
      if (lookahead == 'd') ADVANCE(15);
      if (lookahead == 's') ADVANCE(279);
      END_STATE();
    case 97:
      if (lookahead == 'd') ADVANCE(354);
      END_STATE();
    case 98:
      if (lookahead == 'd') ADVANCE(126);
      END_STATE();
    case 99:
      if (lookahead == 'd') ADVANCE(179);
      END_STATE();
    case 100:
      if (lookahead == 'd') ADVANCE(167);
      END_STATE();
    case 101:
      if (lookahead == 'd') ADVANCE(30);
      END_STATE();
    case 102:
      if (lookahead == 'e') ADVANCE(156);
      END_STATE();
    case 103:
      if (lookahead == 'e') ADVANCE(38);
      END_STATE();
    case 104:
      if (lookahead == 'e') ADVANCE(338);
      if (lookahead == 'i') ADVANCE(127);
      if (lookahead == 'r') ADVANCE(399);
      END_STATE();
    case 105:
      if (lookahead == 'e') ADVANCE(499);
      END_STATE();
    case 106:
      if (lookahead == 'e') ADVANCE(500);
      END_STATE();
    case 107:
      if (lookahead == 'e') ADVANCE(451);
      END_STATE();
    case 108:
      if (lookahead == 'e') ADVANCE(497);
      END_STATE();
    case 109:
      if (lookahead == 'e') ADVANCE(464);
      END_STATE();
    case 110:
      if (lookahead == 'e') ADVANCE(436);
      END_STATE();
    case 111:
      if (lookahead == 'e') ADVANCE(458);
      END_STATE();
    case 112:
      if (lookahead == 'e') ADVANCE(454);
      END_STATE();
    case 113:
      if (lookahead == 'e') ADVANCE(481);
      END_STATE();
    case 114:
      if (lookahead == 'e') ADVANCE(297);
      END_STATE();
    case 115:
      if (lookahead == 'e') ADVANCE(48);
      END_STATE();
    case 116:
      if (lookahead == 'e') ADVANCE(410);
      END_STATE();
    case 117:
      if (lookahead == 'e') ADVANCE(282);
      END_STATE();
    case 118:
      if (lookahead == 'e') ADVANCE(78);
      if (lookahead == 'p') ADVANCE(124);
      if (lookahead == 't') ADVANCE(312);
      END_STATE();
    case 119:
      if (lookahead == 'e') ADVANCE(165);
      END_STATE();
    case 120:
      if (lookahead == 'e') ADVANCE(244);
      END_STATE();
    case 121:
      if (lookahead == 'e') ADVANCE(244);
      if (lookahead == 't') ADVANCE(170);
      END_STATE();
    case 122:
      if (lookahead == 'e') ADVANCE(93);
      END_STATE();
    case 123:
      if (lookahead == 'e') ADVANCE(28);
      END_STATE();
    case 124:
      if (lookahead == 'e') ADVANCE(245);
      END_STATE();
    case 125:
      if (lookahead == 'e') ADVANCE(308);
      END_STATE();
    case 126:
      if (lookahead == 'e') ADVANCE(201);
      END_STATE();
    case 127:
      if (lookahead == 'e') ADVANCE(303);
      END_STATE();
    case 128:
      if (lookahead == 'e') ADVANCE(16);
      END_STATE();
    case 129:
      if (lookahead == 'e') ADVANCE(266);
      END_STATE();
    case 130:
      if (lookahead == 'e') ADVANCE(22);
      END_STATE();
    case 131:
      if (lookahead == 'e') ADVANCE(353);
      END_STATE();
    case 132:
      if (lookahead == 'e') ADVANCE(318);
      END_STATE();
    case 133:
      if (lookahead == 'e') ADVANCE(23);
      END_STATE();
    case 134:
      if (lookahead == 'e') ADVANCE(52);
      END_STATE();
    case 135:
      if (lookahead == 'e') ADVANCE(332);
      END_STATE();
    case 136:
      if (lookahead == 'e') ADVANCE(389);
      END_STATE();
    case 137:
      if (lookahead == 'e') ADVANCE(204);
      END_STATE();
    case 138:
      if (lookahead == 'e') ADVANCE(53);
      END_STATE();
    case 139:
      if (lookahead == 'e') ADVANCE(317);
      END_STATE();
    case 140:
      if (lookahead == 'e') ADVANCE(336);
      END_STATE();
    case 141:
      if (lookahead == 'e') ADVANCE(306);
      END_STATE();
    case 142:
      if (lookahead == 'e') ADVANCE(241);
      END_STATE();
    case 143:
      if (lookahead == 'e') ADVANCE(315);
      END_STATE();
    case 144:
      if (lookahead == 'e') ADVANCE(243);
      END_STATE();
    case 145:
      if (lookahead == 'e') ADVANCE(243);
      if (lookahead == 'p') ADVANCE(286);
      END_STATE();
    case 146:
      if (lookahead == 'e') ADVANCE(257);
      END_STATE();
    case 147:
      if (lookahead == 'e') ADVANCE(349);
      END_STATE();
    case 148:
      if (lookahead == 'e') ADVANCE(256);
      END_STATE();
    case 149:
      if (lookahead == 'e') ADVANCE(350);
      if (lookahead == 'i') ADVANCE(127);
      END_STATE();
    case 150:
      if (lookahead == 'e') ADVANCE(258);
      END_STATE();
    case 151:
      if (lookahead == 'e') ADVANCE(351);
      if (lookahead == 'i') ADVANCE(127);
      END_STATE();
    case 152:
      if (lookahead == 'e') ADVANCE(352);
      END_STATE();
    case 153:
      if (lookahead == 'e') ADVANCE(232);
      END_STATE();
    case 154:
      if (lookahead == 'f') ADVANCE(157);
      if (lookahead == 's') ADVANCE(84);
      if (lookahead == 'x') ADVANCE(118);
      END_STATE();
    case 155:
      if (lookahead == 'f') ADVANCE(471);
      END_STATE();
    case 156:
      if (lookahead == 'f') ADVANCE(43);
      if (lookahead == 'p') ADVANCE(121);
      if (lookahead == 's') ADVANCE(85);
      END_STATE();
    case 157:
      if (lookahead == 'f') ADVANCE(270);
      END_STATE();
    case 158:
      if (lookahead == 'f') ADVANCE(414);
      END_STATE();
    case 159:
      if (lookahead == 'f') ADVANCE(207);
      END_STATE();
    case 160:
      if (lookahead == 'f') ADVANCE(184);
      END_STATE();
    case 161:
      if (lookahead == 'f') ADVANCE(275);
      END_STATE();
    case 162:
      if (lookahead == 'f') ADVANCE(393);
      END_STATE();
    case 163:
      if (lookahead == 'g') ADVANCE(440);
      END_STATE();
    case 164:
      if (lookahead == 'g') ADVANCE(142);
      if (lookahead == 't') ADVANCE(372);
      END_STATE();
    case 165:
      if (lookahead == 'g') ADVANCE(416);
      END_STATE();
    case 166:
      if (lookahead == 'g') ADVANCE(123);
      END_STATE();
    case 167:
      if (lookahead == 'g') ADVANCE(113);
      END_STATE();
    case 168:
      if (lookahead == 'g') ADVANCE(218);
      END_STATE();
    case 169:
      if (lookahead == 'g') ADVANCE(150);
      if (lookahead == 't') ADVANCE(372);
      END_STATE();
    case 170:
      if (lookahead == 'h') ADVANCE(473);
      END_STATE();
    case 171:
      if (lookahead == 'h') ADVANCE(49);
      END_STATE();
    case 172:
      if (lookahead == 'i') ADVANCE(145);
      END_STATE();
    case 173:
      if (lookahead == 'i') ADVANCE(405);
      if (lookahead == 'o') ADVANCE(226);
      END_STATE();
    case 174:
      if (lookahead == 'i') ADVANCE(158);
      END_STATE();
    case 175:
      if (lookahead == 'i') ADVANCE(406);
      END_STATE();
    case 176:
      if (lookahead == 'i') ADVANCE(229);
      END_STATE();
    case 177:
      if (lookahead == 'i') ADVANCE(230);
      END_STATE();
    case 178:
      if (lookahead == 'i') ADVANCE(74);
      END_STATE();
    case 179:
      if (lookahead == 'i') ADVANCE(249);
      END_STATE();
    case 180:
      if (lookahead == 'i') ADVANCE(202);
      END_STATE();
    case 181:
      if (lookahead == 'i') ADVANCE(299);
      END_STATE();
    case 182:
      if (lookahead == 'i') ADVANCE(242);
      END_STATE();
    case 183:
      if (lookahead == 'i') ADVANCE(289);
      END_STATE();
    case 184:
      if (lookahead == 'i') ADVANCE(324);
      END_STATE();
    case 185:
      if (lookahead == 'i') ADVANCE(373);
      END_STATE();
    case 186:
      if (lookahead == 'i') ADVANCE(364);
      END_STATE();
    case 187:
      if (lookahead == 'i') ADVANCE(135);
      END_STATE();
    case 188:
      if (lookahead == 'i') ADVANCE(255);
      END_STATE();
    case 189:
      if (lookahead == 'i') ADVANCE(217);
      END_STATE();
    case 190:
      if (lookahead == 'i') ADVANCE(381);
      END_STATE();
    case 191:
      if (lookahead == 'i') ADVANCE(219);
      END_STATE();
    case 192:
      if (lookahead == 'i') ADVANCE(272);
      END_STATE();
    case 193:
      if (lookahead == 'i') ADVANCE(391);
      if (lookahead == 'p') ADVANCE(55);
      if (lookahead == 'r') ADVANCE(136);
      END_STATE();
    case 194:
      if (lookahead == 'i') ADVANCE(274);
      END_STATE();
    case 195:
      if (lookahead == 'i') ADVANCE(278);
      END_STATE();
    case 196:
      if (lookahead == 'i') ADVANCE(90);
      END_STATE();
    case 197:
      if (lookahead == 'i') ADVANCE(144);
      END_STATE();
    case 198:
      if (lookahead == 'j') ADVANCE(404);
      END_STATE();
    case 199:
      if (lookahead == 'k') ADVANCE(159);
      END_STATE();
    case 200:
      if (lookahead == 'l') ADVANCE(172);
      if (lookahead == 'o') ADVANCE(228);
      END_STATE();
    case 201:
      if (lookahead == 'l') ADVANCE(428);
      END_STATE();
    case 202:
      if (lookahead == 'l') ADVANCE(482);
      END_STATE();
    case 203:
      if (lookahead == 'l') ADVANCE(444);
      END_STATE();
    case 204:
      if (lookahead == 'l') ADVANCE(486);
      END_STATE();
    case 205:
      if (lookahead == 'l') ADVANCE(344);
      END_STATE();
    case 206:
      if (lookahead == 'l') ADVANCE(213);
      END_STATE();
    case 207:
      if (lookahead == 'l') ADVANCE(263);
      END_STATE();
    case 208:
      if (lookahead == 'l') ADVANCE(27);
      END_STATE();
    case 209:
      if (lookahead == 'l') ADVANCE(417);
      END_STATE();
    case 210:
      if (lookahead == 'l') ADVANCE(65);
      END_STATE();
    case 211:
      if (lookahead == 'l') ADVANCE(178);
      END_STATE();
    case 212:
      if (lookahead == 'l') ADVANCE(419);
      END_STATE();
    case 213:
      if (lookahead == 'l') ADVANCE(143);
      END_STATE();
    case 214:
      if (lookahead == 'l') ADVANCE(362);
      END_STATE();
    case 215:
      if (lookahead == 'l') ADVANCE(176);
      if (lookahead == 'n') ADVANCE(329);
      if (lookahead == 'o') ADVANCE(250);
      if (lookahead == 'q') ADVANCE(403);
      END_STATE();
    case 216:
      if (lookahead == 'l') ADVANCE(146);
      END_STATE();
    case 217:
      if (lookahead == 'l') ADVANCE(185);
      END_STATE();
    case 218:
      if (lookahead == 'l') ADVANCE(130);
      END_STATE();
    case 219:
      if (lookahead == 'l') ADVANCE(109);
      END_STATE();
    case 220:
      if (lookahead == 'l') ADVANCE(183);
      if (lookahead == 'o') ADVANCE(227);
      END_STATE();
    case 221:
      if (lookahead == 'l') ADVANCE(137);
      END_STATE();
    case 222:
      if (lookahead == 'l') ADVANCE(221);
      END_STATE();
    case 223:
      if (lookahead == 'm') ADVANCE(283);
      if (lookahead == 't') ADVANCE(125);
      END_STATE();
    case 224:
      if (lookahead == 'm') ADVANCE(225);
      if (lookahead == 'n') ADVANCE(378);
      if (lookahead == 'o') ADVANCE(310);
      END_STATE();
    case 225:
      if (lookahead == 'm') ADVANCE(61);
      if (lookahead == 'p') ADVANCE(191);
      END_STATE();
    case 226:
      if (lookahead == 'm') ADVANCE(291);
      END_STATE();
    case 227:
      if (lookahead == 'm') ADVANCE(288);
      END_STATE();
    case 228:
      if (lookahead == 'm') ADVANCE(288);
      if (lookahead == 'n') ADVANCE(378);
      END_STATE();
    case 229:
      if (lookahead == 'm') ADVANCE(186);
      END_STATE();
    case 230:
      if (lookahead == 'm') ADVANCE(129);
      END_STATE();
    case 231:
      if (lookahead == 'm') ADVANCE(271);
      END_STATE();
    case 232:
      if (lookahead == 'm') ADVANCE(292);
      END_STATE();
    case 233:
      if (lookahead == 'm') ADVANCE(294);
      if (lookahead == 't') ADVANCE(125);
      END_STATE();
    case 234:
      if (lookahead == 'm') ADVANCE(298);
      END_STATE();
    case 235:
      if (lookahead == 'm') ADVANCE(300);
      END_STATE();
    case 236:
      if (lookahead == 'n') ADVANCE(442);
      END_STATE();
    case 237:
      if (lookahead == 'n') ADVANCE(449);
      END_STATE();
    case 238:
      if (lookahead == 'n') ADVANCE(448);
      END_STATE();
    case 239:
      if (lookahead == 'n') ADVANCE(371);
      END_STATE();
    case 240:
      if (lookahead == 'n') ADVANCE(168);
      END_STATE();
    case 241:
      if (lookahead == 'n') ADVANCE(357);
      END_STATE();
    case 242:
      if (lookahead == 'n') ADVANCE(163);
      END_STATE();
    case 243:
      if (lookahead == 'n') ADVANCE(358);
      END_STATE();
    case 244:
      if (lookahead == 'n') ADVANCE(97);
      END_STATE();
    case 245:
      if (lookahead == 'n') ADVANCE(343);
      END_STATE();
    case 246:
      if (lookahead == 'n') ADVANCE(94);
      END_STATE();
    case 247:
      if (lookahead == 'n') ADVANCE(82);
      END_STATE();
    case 248:
      if (lookahead == 'n') ADVANCE(122);
      END_STATE();
    case 249:
      if (lookahead == 'n') ADVANCE(59);
      END_STATE();
    case 250:
      if (lookahead == 'n') ADVANCE(209);
      END_STATE();
    case 251:
      if (lookahead == 'n') ADVANCE(212);
      END_STATE();
    case 252:
      if (lookahead == 'n') ADVANCE(196);
      END_STATE();
    case 253:
      if (lookahead == 'n') ADVANCE(182);
      END_STATE();
    case 254:
      if (lookahead == 'n') ADVANCE(335);
      END_STATE();
    case 255:
      if (lookahead == 'n') ADVANCE(108);
      END_STATE();
    case 256:
      if (lookahead == 'n') ADVANCE(367);
      END_STATE();
    case 257:
      if (lookahead == 'n') ADVANCE(131);
      END_STATE();
    case 258:
      if (lookahead == 'n') ADVANCE(383);
      END_STATE();
    case 259:
      if (lookahead == 'n') ADVANCE(378);
      END_STATE();
    case 260:
      if (lookahead == 'n') ADVANCE(394);
      END_STATE();
    case 261:
      if (lookahead == 'o') ADVANCE(87);
      END_STATE();
    case 262:
      if (lookahead == 'o') ADVANCE(226);
      END_STATE();
    case 263:
      if (lookahead == 'o') ADVANCE(407);
      END_STATE();
    case 264:
      if (lookahead == 'o') ADVANCE(295);
      END_STATE();
    case 265:
      if (lookahead == 'o') ADVANCE(302);
      if (lookahead == 'r') ADVANCE(190);
      END_STATE();
    case 266:
      if (lookahead == 'o') ADVANCE(401);
      END_STATE();
    case 267:
      if (lookahead == 'o') ADVANCE(155);
      END_STATE();
    case 268:
      if (lookahead == 'o') ADVANCE(400);
      END_STATE();
    case 269:
      if (lookahead == 'o') ADVANCE(250);
      END_STATE();
    case 270:
      if (lookahead == 'o') ADVANCE(314);
      END_STATE();
    case 271:
      if (lookahead == 'o') ADVANCE(309);
      END_STATE();
    case 272:
      if (lookahead == 'o') ADVANCE(236);
      END_STATE();
    case 273:
      if (lookahead == 'o') ADVANCE(237);
      END_STATE();
    case 274:
      if (lookahead == 'o') ADVANCE(238);
      END_STATE();
    case 275:
      if (lookahead == 'o') ADVANCE(304);
      END_STATE();
    case 276:
      if (lookahead == 'o') ADVANCE(305);
      END_STATE();
    case 277:
      if (lookahead == 'o') ADVANCE(296);
      END_STATE();
    case 278:
      if (lookahead == 'o') ADVANCE(254);
      END_STATE();
    case 279:
      if (lookahead == 'o') ADVANCE(253);
      END_STATE();
    case 280:
      if (lookahead == 'o') ADVANCE(251);
      END_STATE();
    case 281:
      if (lookahead == 'o') ADVANCE(260);
      END_STATE();
    case 282:
      if (lookahead == 'p') ADVANCE(121);
      if (lookahead == 's') ADVANCE(85);
      END_STATE();
    case 283:
      if (lookahead == 'p') ADVANCE(45);
      END_STATE();
    case 284:
      if (lookahead == 'p') ADVANCE(474);
      END_STATE();
    case 285:
      if (lookahead == 'p') ADVANCE(434);
      END_STATE();
    case 286:
      if (lookahead == 'p') ADVANCE(412);
      END_STATE();
    case 287:
      if (lookahead == 'p') ADVANCE(328);
      END_STATE();
    case 288:
      if (lookahead == 'p') ADVANCE(191);
      END_STATE();
    case 289:
      if (lookahead == 'p') ADVANCE(286);
      END_STATE();
    case 290:
      if (lookahead == 'p') ADVANCE(55);
      if (lookahead == 'r') ADVANCE(136);
      END_STATE();
    case 291:
      if (lookahead == 'p') ADVANCE(360);
      END_STATE();
    case 292:
      if (lookahead == 'p') ADVANCE(375);
      END_STATE();
    case 293:
      if (lookahead == 'p') ADVANCE(56);
      END_STATE();
    case 294:
      if (lookahead == 'p') ADVANCE(44);
      END_STATE();
    case 295:
      if (lookahead == 'p') ADVANCE(107);
      END_STATE();
    case 296:
      if (lookahead == 'p') ADVANCE(112);
      END_STATE();
    case 297:
      if (lookahead == 'p') ADVANCE(120);
      if (lookahead == 's') ADVANCE(85);
      END_STATE();
    case 298:
      if (lookahead == 'p') ADVANCE(69);
      END_STATE();
    case 299:
      if (lookahead == 'p') ADVANCE(395);
      END_STATE();
    case 300:
      if (lookahead == 'p') ADVANCE(67);
      END_STATE();
    case 301:
      if (lookahead == 'r') ADVANCE(173);
      if (lookahead == 'u') ADVANCE(70);
      END_STATE();
    case 302:
      if (lookahead == 'r') ADVANCE(199);
      END_STATE();
    case 303:
      if (lookahead == 'r') ADVANCE(427);
      END_STATE();
    case 304:
      if (lookahead == 'r') ADVANCE(472);
      END_STATE();
    case 305:
      if (lookahead == 'r') ADVANCE(438);
      END_STATE();
    case 306:
      if (lookahead == 'r') ADVANCE(490);
      END_STATE();
    case 307:
      if (lookahead == 'r') ADVANCE(190);
      END_STATE();
    case 308:
      if (lookahead == 'r') ADVANCE(26);
      END_STATE();
    case 309:
      if (lookahead == 'r') ADVANCE(413);
      END_STATE();
    case 310:
      if (lookahead == 'r') ADVANCE(99);
      END_STATE();
    case 311:
      if (lookahead == 'r') ADVANCE(327);
      END_STATE();
    case 312:
      if (lookahead == 'r') ADVANCE(40);
      END_STATE();
    case 313:
      if (lookahead == 'r') ADVANCE(174);
      END_STATE();
    case 314:
      if (lookahead == 'r') ADVANCE(359);
      END_STATE();
    case 315:
      if (lookahead == 'r') ADVANCE(342);
      END_STATE();
    case 316:
      if (lookahead == 'r') ADVANCE(390);
      END_STATE();
    case 317:
      if (lookahead == 'r') ADVANCE(420);
      END_STATE();
    case 318:
      if (lookahead == 'r') ADVANCE(68);
      END_STATE();
    case 319:
      if (lookahead == 'r') ADVANCE(262);
      END_STATE();
    case 320:
      if (lookahead == 'r') ADVANCE(181);
      END_STATE();
    case 321:
      if (lookahead == 'r') ADVANCE(363);
      END_STATE();
    case 322:
      if (lookahead == 'r') ADVANCE(187);
      END_STATE();
    case 323:
      if (lookahead == 'r') ADVANCE(83);
      END_STATE();
    case 324:
      if (lookahead == 'r') ADVANCE(348);
      END_STATE();
    case 325:
      if (lookahead == 'r') ADVANCE(66);
      END_STATE();
    case 326:
      if (lookahead == 's') ADVANCE(84);
      END_STATE();
    case 327:
      if (lookahead == 's') ADVANCE(433);
      END_STATE();
    case 328:
      if (lookahead == 's') ADVANCE(485);
      END_STATE();
    case 329:
      if (lookahead == 's') ADVANCE(456);
      END_STATE();
    case 330:
      if (lookahead == 's') ADVANCE(489);
      END_STATE();
    case 331:
      if (lookahead == 's') ADVANCE(457);
      END_STATE();
    case 332:
      if (lookahead == 's') ADVANCE(450);
      END_STATE();
    case 333:
      if (lookahead == 's') ADVANCE(495);
      END_STATE();
    case 334:
      if (lookahead == 's') ADVANCE(469);
      END_STATE();
    case 335:
      if (lookahead == 's') ADVANCE(477);
      END_STATE();
    case 336:
      if (lookahead == 's') ADVANCE(459);
      END_STATE();
    case 337:
      if (lookahead == 's') ADVANCE(476);
      END_STATE();
    case 338:
      if (lookahead == 's') ADVANCE(356);
      END_STATE();
    case 339:
      if (lookahead == 's') ADVANCE(91);
      END_STATE();
    case 340:
      if (lookahead == 's') ADVANCE(91);
      if (lookahead == 't') ADVANCE(147);
      END_STATE();
    case 341:
      if (lookahead == 's') ADVANCE(268);
      END_STATE();
    case 342:
      if (lookahead == 's') ADVANCE(21);
      END_STATE();
    case 343:
      if (lookahead == 's') ADVANCE(175);
      END_STATE();
    case 344:
      if (lookahead == 's') ADVANCE(106);
      END_STATE();
    case 345:
      if (lookahead == 's') ADVANCE(387);
      END_STATE();
    case 346:
      if (lookahead == 's') ADVANCE(24);
      END_STATE();
    case 347:
      if (lookahead == 's') ADVANCE(333);
      END_STATE();
    case 348:
      if (lookahead == 's') ADVANCE(365);
      END_STATE();
    case 349:
      if (lookahead == 's') ADVANCE(380);
      END_STATE();
    case 350:
      if (lookahead == 's') ADVANCE(368);
      END_STATE();
    case 351:
      if (lookahead == 's') ADVANCE(369);
      END_STATE();
    case 352:
      if (lookahead == 's') ADVANCE(370);
      END_STATE();
    case 353:
      if (lookahead == 's') ADVANCE(346);
      END_STATE();
    case 354:
      if (lookahead == 's') ADVANCE(29);
      END_STATE();
    case 355:
      if (lookahead == 't') ADVANCE(372);
      END_STATE();
    case 356:
      if (lookahead == 't') ADVANCE(468);
      END_STATE();
    case 357:
      if (lookahead == 't') ADVANCE(447);
      END_STATE();
    case 358:
      if (lookahead == 't') ADVANCE(424);
      END_STATE();
    case 359:
      if (lookahead == 't') ADVANCE(429);
      END_STATE();
    case 360:
      if (lookahead == 't') ADVANCE(446);
      END_STATE();
    case 361:
      if (lookahead == 't') ADVANCE(470);
      END_STATE();
    case 362:
      if (lookahead == 't') ADVANCE(431);
      END_STATE();
    case 363:
      if (lookahead == 't') ADVANCE(478);
      END_STATE();
    case 364:
      if (lookahead == 't') ADVANCE(461);
      END_STATE();
    case 365:
      if (lookahead == 't') ADVANCE(488);
      END_STATE();
    case 366:
      if (lookahead == 't') ADVANCE(480);
      END_STATE();
    case 367:
      if (lookahead == 't') ADVANCE(462);
      END_STATE();
    case 368:
      if (lookahead == 't') ADVANCE(467);
      END_STATE();
    case 369:
      if (lookahead == 't') ADVANCE(20);
      END_STATE();
    case 370:
      if (lookahead == 't') ADVANCE(466);
      END_STATE();
    case 371:
      if (lookahead == 't') ADVANCE(180);
      END_STATE();
    case 372:
      if (lookahead == 't') ADVANCE(153);
      END_STATE();
    case 373:
      if (lookahead == 't') ADVANCE(418);
      END_STATE();
    case 374:
      if (lookahead == 't') ADVANCE(17);
      END_STATE();
    case 375:
      if (lookahead == 't') ADVANCE(330);
      END_STATE();
    case 376:
      if (lookahead == 't') ADVANCE(18);
      END_STATE();
    case 377:
      if (lookahead == 't') ADVANCE(192);
      END_STATE();
    case 378:
      if (lookahead == 't') ADVANCE(116);
      END_STATE();
    case 379:
      if (lookahead == 't') ADVANCE(31);
      END_STATE();
    case 380:
      if (lookahead == 't') ADVANCE(334);
      END_STATE();
    case 381:
      if (lookahead == 't') ADVANCE(128);
      END_STATE();
    case 382:
      if (lookahead == 't') ADVANCE(177);
      END_STATE();
    case 383:
      if (lookahead == 't') ADVANCE(337);
      END_STATE();
    case 384:
      if (lookahead == 't') ADVANCE(119);
      END_STATE();
    case 385:
      if (lookahead == 't') ADVANCE(133);
      END_STATE();
    case 386:
      if (lookahead == 't') ADVANCE(147);
      END_STATE();
    case 387:
      if (lookahead == 't') ADVANCE(58);
      END_STATE();
    case 388:
      if (lookahead == 't') ADVANCE(25);
      END_STATE();
    case 389:
      if (lookahead == 't') ADVANCE(322);
      END_STATE();
    case 390:
      if (lookahead == 't') ADVANCE(54);
      END_STATE();
    case 391:
      if (lookahead == 't') ADVANCE(132);
      END_STATE();
    case 392:
      if (lookahead == 't') ADVANCE(276);
      END_STATE();
    case 393:
      if (lookahead == 't') ADVANCE(141);
      END_STATE();
    case 394:
      if (lookahead == 't') ADVANCE(148);
      END_STATE();
    case 395:
      if (lookahead == 't') ADVANCE(194);
      END_STATE();
    case 396:
      if (lookahead == 't') ADVANCE(195);
      END_STATE();
    case 397:
      if (lookahead == 'u') ADVANCE(92);
      END_STATE();
    case 398:
      if (lookahead == 'u') ADVANCE(214);
      END_STATE();
    case 399:
      if (lookahead == 'u') ADVANCE(105);
      END_STATE();
    case 400:
      if (lookahead == 'u') ADVANCE(323);
      END_STATE();
    case 401:
      if (lookahead == 'u') ADVANCE(366);
      END_STATE();
    case 402:
      if (lookahead == 'u') ADVANCE(377);
      END_STATE();
    case 403:
      if (lookahead == 'u') ADVANCE(139);
      END_STATE();
    case 404:
      if (lookahead == 'u') ADVANCE(100);
      END_STATE();
    case 405:
      if (lookahead == 'v') ADVANCE(47);
      END_STATE();
    case 406:
      if (lookahead == 'v') ADVANCE(110);
      END_STATE();
    case 407:
      if (lookahead == 'w') ADVANCE(484);
      END_STATE();
    case 408:
      if (lookahead == 'w') ADVANCE(248);
      END_STATE();
    case 409:
      if (lookahead == 'x') ADVANCE(14);
      END_STATE();
    case 410:
      if (lookahead == 'x') ADVANCE(361);
      END_STATE();
    case 411:
      if (lookahead == 'x') ADVANCE(19);
      END_STATE();
    case 412:
      if (lookahead == 'y') ADVANCE(465);
      END_STATE();
    case 413:
      if (lookahead == 'y') ADVANCE(455);
      END_STATE();
    case 414:
      if (lookahead == 'y') ADVANCE(463);
      END_STATE();
    case 415:
      if (lookahead == 'y') ADVANCE(430);
      END_STATE();
    case 416:
      if (lookahead == 'y') ADVANCE(487);
      END_STATE();
    case 417:
      if (lookahead == 'y') ADVANCE(453);
      END_STATE();
    case 418:
      if (lookahead == 'y') ADVANCE(479);
      END_STATE();
    case 419:
      if (lookahead == 'y') ADVANCE(494);
      END_STATE();
    case 420:
      if (lookahead == 'y') ADVANCE(460);
      END_STATE();
    case 421:
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(503);
      END_STATE();
    case 422:
      ACCEPT_TOKEN(ts_builtin_sym_end);
      END_STATE();
    case 423:
      ACCEPT_TOKEN(sym_comment);
      if (lookahead != 0 &&
          lookahead != '\n') ADVANCE(423);
      END_STATE();
    case 424:
      ACCEPT_TOKEN(anon_sym_client);
      END_STATE();
    case 425:
      ACCEPT_TOKEN(anon_sym_LBRACE);
      END_STATE();
    case 426:
      ACCEPT_TOKEN(anon_sym_RBRACE);
      END_STATE();
    case 427:
      ACCEPT_TOKEN(anon_sym_tier);
      END_STATE();
    case 428:
      ACCEPT_TOKEN(anon_sym_model);
      END_STATE();
    case 429:
      ACCEPT_TOKEN(anon_sym_effort);
      END_STATE();
    case 430:
      ACCEPT_TOKEN(anon_sym_privacy);
      END_STATE();
    case 431:
      ACCEPT_TOKEN(anon_sym_default);
      END_STATE();
    case 432:
      ACCEPT_TOKEN(anon_sym_extra);
      END_STATE();
    case 433:
      ACCEPT_TOKEN(anon_sym_vars);
      END_STATE();
    case 434:
      ACCEPT_TOKEN(anon_sym_cheap);
      END_STATE();
    case 435:
      ACCEPT_TOKEN(anon_sym_cheap);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 436:
      ACCEPT_TOKEN(anon_sym_expensive);
      END_STATE();
    case 437:
      ACCEPT_TOKEN(anon_sym_expensive);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 438:
      ACCEPT_TOKEN(anon_sym_coordinator);
      END_STATE();
    case 439:
      ACCEPT_TOKEN(anon_sym_coordinator);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 440:
      ACCEPT_TOKEN(anon_sym_reasoning);
      END_STATE();
    case 441:
      ACCEPT_TOKEN(anon_sym_reasoning);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 442:
      ACCEPT_TOKEN(anon_sym_execution);
      END_STATE();
    case 443:
      ACCEPT_TOKEN(anon_sym_execution);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 444:
      ACCEPT_TOKEN(anon_sym_mechanical);
      END_STATE();
    case 445:
      ACCEPT_TOKEN(anon_sym_mechanical);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 446:
      ACCEPT_TOKEN(anon_sym_prompt);
      END_STATE();
    case 447:
      ACCEPT_TOKEN(anon_sym_agent);
      END_STATE();
    case 448:
      ACCEPT_TOKEN(anon_sym_description);
      END_STATE();
    case 449:
      ACCEPT_TOKEN(anon_sym_depends_on);
      END_STATE();
    case 450:
      ACCEPT_TOKEN(anon_sym_max_retries);
      END_STATE();
    case 451:
      ACCEPT_TOKEN(anon_sym_scope);
      END_STATE();
    case 452:
      ACCEPT_TOKEN(anon_sym_owned);
      END_STATE();
    case 453:
      ACCEPT_TOKEN(anon_sym_read_only);
      END_STATE();
    case 454:
      ACCEPT_TOKEN(anon_sym_impact_scope);
      END_STATE();
    case 455:
      ACCEPT_TOKEN(anon_sym_memory);
      END_STATE();
    case 456:
      ACCEPT_TOKEN(anon_sym_read_ns);
      END_STATE();
    case 457:
      ACCEPT_TOKEN(anon_sym_write_ns);
      END_STATE();
    case 458:
      ACCEPT_TOKEN(anon_sym_importance);
      END_STATE();
    case 459:
      ACCEPT_TOKEN(anon_sym_staleness_sources);
      END_STATE();
    case 460:
      ACCEPT_TOKEN(anon_sym_read_query);
      END_STATE();
    case 461:
      ACCEPT_TOKEN(anon_sym_read_limit);
      END_STATE();
    case 462:
      ACCEPT_TOKEN(anon_sym_write_content);
      END_STATE();
    case 463:
      ACCEPT_TOKEN(anon_sym_verify);
      END_STATE();
    case 464:
      ACCEPT_TOKEN(anon_sym_compile);
      END_STATE();
    case 465:
      ACCEPT_TOKEN(anon_sym_clippy);
      END_STATE();
    case 466:
      ACCEPT_TOKEN(anon_sym_test);
      END_STATE();
    case 467:
      ACCEPT_TOKEN(anon_sym_test);
      if (lookahead == '_') ADVANCE(160);
      END_STATE();
    case 468:
      ACCEPT_TOKEN(anon_sym_test);
      if (lookahead == '_') ADVANCE(160);
      if (lookahead == 's') ADVANCE(32);
      END_STATE();
    case 469:
      ACCEPT_TOKEN(anon_sym_impact_tests);
      END_STATE();
    case 470:
      ACCEPT_TOKEN(anon_sym_context);
      END_STATE();
    case 471:
      ACCEPT_TOKEN(anon_sym_callers_of);
      END_STATE();
    case 472:
      ACCEPT_TOKEN(anon_sym_tests_for);
      END_STATE();
    case 473:
      ACCEPT_TOKEN(anon_sym_depth);
      END_STATE();
    case 474:
      ACCEPT_TOKEN(anon_sym_loop);
      END_STATE();
    case 475:
      ACCEPT_TOKEN(anon_sym_loop);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 476:
      ACCEPT_TOKEN(anon_sym_agents);
      END_STATE();
    case 477:
      ACCEPT_TOKEN(anon_sym_max_iterations);
      END_STATE();
    case 478:
      ACCEPT_TOKEN(anon_sym_iter_start);
      END_STATE();
    case 479:
      ACCEPT_TOKEN(anon_sym_stability);
      END_STATE();
    case 480:
      ACCEPT_TOKEN(anon_sym_judge_timeout);
      END_STATE();
    case 481:
      ACCEPT_TOKEN(anon_sym_strict_judge);
      END_STATE();
    case 482:
      ACCEPT_TOKEN(anon_sym_until);
      END_STATE();
    case 483:
      ACCEPT_TOKEN(anon_sym_command);
      END_STATE();
    case 484:
      ACCEPT_TOKEN(anon_sym_workflow);
      END_STATE();
    case 485:
      ACCEPT_TOKEN(anon_sym_steps);
      END_STATE();
    case 486:
      ACCEPT_TOKEN(anon_sym_max_parallel);
      END_STATE();
    case 487:
      ACCEPT_TOKEN(anon_sym_strategy);
      END_STATE();
    case 488:
      ACCEPT_TOKEN(anon_sym_test_first);
      END_STATE();
    case 489:
      ACCEPT_TOKEN(anon_sym_attempts);
      END_STATE();
    case 490:
      ACCEPT_TOKEN(anon_sym_escalate_after);
      END_STATE();
    case 491:
      ACCEPT_TOKEN(anon_sym_LBRACK);
      END_STATE();
    case 492:
      ACCEPT_TOKEN(anon_sym_RBRACK);
      END_STATE();
    case 493:
      ACCEPT_TOKEN(anon_sym_public);
      END_STATE();
    case 494:
      ACCEPT_TOKEN(anon_sym_local_only);
      END_STATE();
    case 495:
      ACCEPT_TOKEN(anon_sym_single_pass);
      END_STATE();
    case 496:
      ACCEPT_TOKEN(anon_sym_single_pass);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 497:
      ACCEPT_TOKEN(anon_sym_refine);
      END_STATE();
    case 498:
      ACCEPT_TOKEN(anon_sym_refine);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 499:
      ACCEPT_TOKEN(anon_sym_true);
      END_STATE();
    case 500:
      ACCEPT_TOKEN(anon_sym_false);
      END_STATE();
    case 501:
      ACCEPT_TOKEN(sym_string);
      END_STATE();
    case 502:
      ACCEPT_TOKEN(sym_raw_string);
      END_STATE();
    case 503:
      ACCEPT_TOKEN(sym_float);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(503);
      END_STATE();
    case 504:
      ACCEPT_TOKEN(sym_integer);
      if (lookahead == '.') ADVANCE(421);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(504);
      END_STATE();
    case 505:
      ACCEPT_TOKEN(sym_integer);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(505);
      END_STATE();
    case 506:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '_') ADVANCE(556);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 507:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(560);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 508:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(554);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 509:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(538);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 510:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(545);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 511:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(564);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 512:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(562);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 513:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(530);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 514:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(565);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 515:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(509);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 516:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'd') ADVANCE(533);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 517:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(513);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 518:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(507);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 519:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(542);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 520:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(437);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 521:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(526);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 522:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(498);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 523:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(506);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 524:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(514);
      if (lookahead == 'p') ADVANCE(519);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 525:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(508);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 526:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'f') ADVANCE(536);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 527:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'g') ADVANCE(441);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 528:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'g') ADVANCE(539);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 529:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(525);
      if (lookahead == 'o') ADVANCE(548);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 530:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(510);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 531:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(566);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 532:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(515);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 533:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(544);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 534:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(540);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 535:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(543);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 536:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(546);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 537:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(553);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 538:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(445);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 539:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(523);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 540:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(527);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 541:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(443);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 542:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(561);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 543:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(528);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 544:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(511);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 545:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(532);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 546:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(522);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 547:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(534);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 548:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(557);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 549:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(558);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 550:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(555);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 551:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(547);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 552:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(550);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 553:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(541);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 554:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(435);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 555:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(475);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 556:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(512);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 557:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(516);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 558:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(439);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 559:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(496);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 560:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(551);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 561:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(531);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 562:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(559);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 563:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(537);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 564:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(549);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 565:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'u') ADVANCE(563);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 566:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'v') ADVANCE(520);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 567:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'x') ADVANCE(524);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    case 568:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(568);
      END_STATE();
    default:
      return false;
  }
}

static const TSLexMode ts_lex_modes[STATE_COUNT] = {
  [0] = {.lex_state = 0},
  [1] = {.lex_state = 0},
  [2] = {.lex_state = 10},
  [3] = {.lex_state = 10},
  [4] = {.lex_state = 10},
  [5] = {.lex_state = 9},
  [6] = {.lex_state = 9},
  [7] = {.lex_state = 0},
  [8] = {.lex_state = 0},
  [9] = {.lex_state = 0},
  [10] = {.lex_state = 0},
  [11] = {.lex_state = 0},
  [12] = {.lex_state = 9},
  [13] = {.lex_state = 9},
  [14] = {.lex_state = 9},
  [15] = {.lex_state = 0},
  [16] = {.lex_state = 0},
  [17] = {.lex_state = 9},
  [18] = {.lex_state = 9},
  [19] = {.lex_state = 2},
  [20] = {.lex_state = 10},
  [21] = {.lex_state = 0},
  [22] = {.lex_state = 0},
  [23] = {.lex_state = 0},
  [24] = {.lex_state = 0},
  [25] = {.lex_state = 0},
  [26] = {.lex_state = 10},
  [27] = {.lex_state = 0},
  [28] = {.lex_state = 10},
  [29] = {.lex_state = 0},
  [30] = {.lex_state = 0},
  [31] = {.lex_state = 0},
  [32] = {.lex_state = 9},
  [33] = {.lex_state = 9},
  [34] = {.lex_state = 0},
  [35] = {.lex_state = 0},
  [36] = {.lex_state = 9},
  [37] = {.lex_state = 0},
  [38] = {.lex_state = 9},
  [39] = {.lex_state = 9},
  [40] = {.lex_state = 9},
  [41] = {.lex_state = 9},
  [42] = {.lex_state = 0},
  [43] = {.lex_state = 10},
  [44] = {.lex_state = 10},
  [45] = {.lex_state = 2},
  [46] = {.lex_state = 0},
  [47] = {.lex_state = 10},
  [48] = {.lex_state = 2},
  [49] = {.lex_state = 10},
  [50] = {.lex_state = 10},
  [51] = {.lex_state = 10},
  [52] = {.lex_state = 10},
  [53] = {.lex_state = 0},
  [54] = {.lex_state = 0},
  [55] = {.lex_state = 0},
  [56] = {.lex_state = 1},
  [57] = {.lex_state = 0},
  [58] = {.lex_state = 0},
  [59] = {.lex_state = 0},
  [60] = {.lex_state = 0},
  [61] = {.lex_state = 11},
  [62] = {.lex_state = 0},
  [63] = {.lex_state = 1},
  [64] = {.lex_state = 0},
  [65] = {.lex_state = 0},
  [66] = {.lex_state = 11},
  [67] = {.lex_state = 0},
  [68] = {.lex_state = 0},
  [69] = {.lex_state = 1},
  [70] = {.lex_state = 0},
  [71] = {.lex_state = 11},
  [72] = {.lex_state = 0},
  [73] = {.lex_state = 0},
  [74] = {.lex_state = 11},
  [75] = {.lex_state = 0},
  [76] = {.lex_state = 11},
  [77] = {.lex_state = 9},
  [78] = {.lex_state = 9},
  [79] = {.lex_state = 9},
  [80] = {.lex_state = 0},
  [81] = {.lex_state = 0},
  [82] = {.lex_state = 0},
  [83] = {.lex_state = 8},
  [84] = {.lex_state = 8},
  [85] = {.lex_state = 11},
  [86] = {.lex_state = 0},
  [87] = {.lex_state = 8},
  [88] = {.lex_state = 0},
  [89] = {.lex_state = 0},
  [90] = {.lex_state = 9},
  [91] = {.lex_state = 1},
  [92] = {.lex_state = 1},
  [93] = {.lex_state = 12},
  [94] = {.lex_state = 1},
  [95] = {.lex_state = 1},
  [96] = {.lex_state = 1},
  [97] = {.lex_state = 0},
  [98] = {.lex_state = 0},
  [99] = {.lex_state = 0},
  [100] = {.lex_state = 0},
  [101] = {.lex_state = 0},
  [102] = {.lex_state = 0},
  [103] = {.lex_state = 1},
  [104] = {.lex_state = 0},
  [105] = {.lex_state = 1},
  [106] = {.lex_state = 1},
  [107] = {.lex_state = 0},
  [108] = {.lex_state = 0},
  [109] = {.lex_state = 8},
  [110] = {.lex_state = 0},
  [111] = {.lex_state = 0},
  [112] = {.lex_state = 0},
  [113] = {.lex_state = 0},
  [114] = {.lex_state = 8},
  [115] = {.lex_state = 0},
  [116] = {.lex_state = 1},
  [117] = {.lex_state = 0},
  [118] = {.lex_state = 0},
  [119] = {.lex_state = 0},
  [120] = {.lex_state = 0},
  [121] = {.lex_state = 0},
  [122] = {.lex_state = 1},
  [123] = {.lex_state = 0},
  [124] = {.lex_state = 0},
  [125] = {.lex_state = 1},
  [126] = {.lex_state = 1},
  [127] = {.lex_state = 10},
  [128] = {.lex_state = 0},
  [129] = {.lex_state = 0},
  [130] = {.lex_state = 9},
  [131] = {.lex_state = 10},
  [132] = {.lex_state = 1},
  [133] = {.lex_state = 0},
  [134] = {.lex_state = 0},
  [135] = {.lex_state = 1},
  [136] = {.lex_state = 1},
  [137] = {.lex_state = 0},
  [138] = {.lex_state = 10},
  [139] = {.lex_state = 1},
  [140] = {.lex_state = 1},
  [141] = {.lex_state = 0},
  [142] = {.lex_state = 0},
  [143] = {.lex_state = 0},
  [144] = {.lex_state = 10},
  [145] = {.lex_state = 0},
  [146] = {.lex_state = 10},
};

static const uint16_t ts_parse_table[LARGE_STATE_COUNT][SYMBOL_COUNT] = {
  [0] = {
    [ts_builtin_sym_end] = ACTIONS(1),
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
    [sym_source_file] = STATE(123),
    [sym__definition] = STATE(11),
    [sym_client_declaration] = STATE(11),
    [sym_vars_block] = STATE(11),
    [sym_tier_alias_declaration] = STATE(11),
    [sym_prompt_declaration] = STATE(11),
    [sym_agent_declaration] = STATE(11),
    [sym_workflow_declaration] = STATE(11),
    [aux_sym_source_file_repeat1] = STATE(11),
    [ts_builtin_sym_end] = ACTIONS(5),
    [sym_comment] = ACTIONS(3),
    [anon_sym_client] = ACTIONS(7),
    [anon_sym_tier] = ACTIONS(9),
    [anon_sym_vars] = ACTIONS(11),
    [anon_sym_prompt] = ACTIONS(13),
    [anon_sym_agent] = ACTIONS(15),
    [anon_sym_workflow] = ACTIONS(17),
  },
};

static const uint16_t ts_small_parse_table[] = {
  [0] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(21), 1,
      anon_sym_test,
    ACTIONS(19), 23,
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
  [32] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(23), 18,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
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
  [56] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(25), 18,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
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
  [80] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(27), 18,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
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
  [104] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(29), 18,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
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
  [128] = 14,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(11), 1,
      anon_sym_vars,
    ACTIONS(31), 1,
      anon_sym_client,
    ACTIONS(33), 1,
      anon_sym_RBRACE,
    ACTIONS(35), 1,
      anon_sym_tier,
    ACTIONS(37), 1,
      anon_sym_prompt,
    ACTIONS(39), 1,
      anon_sym_description,
    ACTIONS(41), 1,
      anon_sym_depends_on,
    ACTIONS(43), 1,
      anon_sym_max_retries,
    ACTIONS(45), 1,
      anon_sym_scope,
    ACTIONS(47), 1,
      anon_sym_memory,
    ACTIONS(49), 1,
      anon_sym_context,
    STATE(8), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(21), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [175] = 14,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(51), 1,
      anon_sym_client,
    ACTIONS(54), 1,
      anon_sym_RBRACE,
    ACTIONS(56), 1,
      anon_sym_tier,
    ACTIONS(59), 1,
      anon_sym_vars,
    ACTIONS(62), 1,
      anon_sym_prompt,
    ACTIONS(65), 1,
      anon_sym_description,
    ACTIONS(68), 1,
      anon_sym_depends_on,
    ACTIONS(71), 1,
      anon_sym_max_retries,
    ACTIONS(74), 1,
      anon_sym_scope,
    ACTIONS(77), 1,
      anon_sym_memory,
    ACTIONS(80), 1,
      anon_sym_context,
    STATE(8), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(21), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [222] = 14,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(11), 1,
      anon_sym_vars,
    ACTIONS(31), 1,
      anon_sym_client,
    ACTIONS(35), 1,
      anon_sym_tier,
    ACTIONS(37), 1,
      anon_sym_prompt,
    ACTIONS(39), 1,
      anon_sym_description,
    ACTIONS(41), 1,
      anon_sym_depends_on,
    ACTIONS(43), 1,
      anon_sym_max_retries,
    ACTIONS(45), 1,
      anon_sym_scope,
    ACTIONS(47), 1,
      anon_sym_memory,
    ACTIONS(49), 1,
      anon_sym_context,
    ACTIONS(83), 1,
      anon_sym_RBRACE,
    STATE(7), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(21), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [269] = 9,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(85), 1,
      ts_builtin_sym_end,
    ACTIONS(87), 1,
      anon_sym_client,
    ACTIONS(90), 1,
      anon_sym_tier,
    ACTIONS(93), 1,
      anon_sym_vars,
    ACTIONS(96), 1,
      anon_sym_prompt,
    ACTIONS(99), 1,
      anon_sym_agent,
    ACTIONS(102), 1,
      anon_sym_workflow,
    STATE(10), 8,
      sym__definition,
      sym_client_declaration,
      sym_vars_block,
      sym_tier_alias_declaration,
      sym_prompt_declaration,
      sym_agent_declaration,
      sym_workflow_declaration,
      aux_sym_source_file_repeat1,
  [304] = 9,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(7), 1,
      anon_sym_client,
    ACTIONS(9), 1,
      anon_sym_tier,
    ACTIONS(11), 1,
      anon_sym_vars,
    ACTIONS(13), 1,
      anon_sym_prompt,
    ACTIONS(15), 1,
      anon_sym_agent,
    ACTIONS(17), 1,
      anon_sym_workflow,
    ACTIONS(105), 1,
      ts_builtin_sym_end,
    STATE(10), 8,
      sym__definition,
      sym_client_declaration,
      sym_vars_block,
      sym_tier_alias_declaration,
      sym_prompt_declaration,
      sym_agent_declaration,
      sym_workflow_declaration,
      aux_sym_source_file_repeat1,
  [339] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(47), 1,
      anon_sym_memory,
    ACTIONS(107), 1,
      anon_sym_RBRACE,
    ACTIONS(111), 1,
      anon_sym_verify,
    ACTIONS(113), 1,
      anon_sym_steps,
    ACTIONS(115), 1,
      anon_sym_strategy,
    ACTIONS(117), 1,
      anon_sym_test_first,
    STATE(14), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(41), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(109), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [375] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(119), 1,
      anon_sym_RBRACE,
    ACTIONS(124), 1,
      anon_sym_memory,
    ACTIONS(127), 1,
      anon_sym_verify,
    ACTIONS(130), 1,
      anon_sym_steps,
    ACTIONS(133), 1,
      anon_sym_strategy,
    ACTIONS(136), 1,
      anon_sym_test_first,
    STATE(13), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(41), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(121), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [411] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(47), 1,
      anon_sym_memory,
    ACTIONS(111), 1,
      anon_sym_verify,
    ACTIONS(113), 1,
      anon_sym_steps,
    ACTIONS(115), 1,
      anon_sym_strategy,
    ACTIONS(117), 1,
      anon_sym_test_first,
    ACTIONS(139), 1,
      anon_sym_RBRACE,
    STATE(13), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(41), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(109), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [447] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(141), 14,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
      anon_sym_workflow,
  [467] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(143), 14,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
      anon_sym_workflow,
  [487] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(145), 14,
      anon_sym_RBRACE,
      anon_sym_owned,
      anon_sym_read_only,
      anon_sym_impact_scope,
      anon_sym_read_ns,
      anon_sym_write_ns,
      anon_sym_importance,
      anon_sym_staleness_sources,
      anon_sym_read_query,
      anon_sym_read_limit,
      anon_sym_write_content,
      anon_sym_callers_of,
      anon_sym_tests_for,
      anon_sym_depth,
  [507] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(147), 14,
      anon_sym_RBRACE,
      anon_sym_owned,
      anon_sym_read_only,
      anon_sym_impact_scope,
      anon_sym_read_ns,
      anon_sym_write_ns,
      anon_sym_importance,
      anon_sym_staleness_sources,
      anon_sym_read_query,
      anon_sym_read_limit,
      anon_sym_write_content,
      anon_sym_callers_of,
      anon_sym_tests_for,
      anon_sym_depth,
  [527] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(153), 1,
      sym_identifier,
    ACTIONS(151), 2,
      sym_string,
      sym_raw_string,
    STATE(72), 3,
      sym__effort_value,
      sym_tier_value,
      sym__string_value,
    ACTIONS(149), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [551] = 8,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(155), 1,
      anon_sym_RBRACE,
    ACTIONS(157), 1,
      anon_sym_agents,
    ACTIONS(161), 1,
      anon_sym_strict_judge,
    ACTIONS(163), 1,
      anon_sym_until,
    STATE(47), 1,
      sym_until_clause,
    STATE(28), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(159), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [580] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(165), 11,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [597] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(167), 11,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [614] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(169), 11,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [631] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(171), 11,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [648] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(173), 11,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [665] = 8,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(157), 1,
      anon_sym_agents,
    ACTIONS(161), 1,
      anon_sym_strict_judge,
    ACTIONS(163), 1,
      anon_sym_until,
    ACTIONS(175), 1,
      anon_sym_RBRACE,
    STATE(47), 1,
      sym_until_clause,
    STATE(20), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(159), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [694] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(177), 11,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [711] = 8,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(179), 1,
      anon_sym_RBRACE,
    ACTIONS(181), 1,
      anon_sym_agents,
    ACTIONS(187), 1,
      anon_sym_strict_judge,
    ACTIONS(190), 1,
      anon_sym_until,
    STATE(47), 1,
      sym_until_clause,
    STATE(28), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(184), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [740] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(193), 11,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [757] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(195), 1,
      anon_sym_RBRACE,
    ACTIONS(197), 1,
      anon_sym_tier,
    ACTIONS(200), 1,
      anon_sym_model,
    ACTIONS(203), 1,
      anon_sym_effort,
    ACTIONS(206), 1,
      anon_sym_privacy,
    ACTIONS(209), 1,
      anon_sym_default,
    ACTIONS(212), 1,
      anon_sym_extra,
    STATE(64), 1,
      sym_extra_block,
    STATE(30), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [789] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(215), 1,
      anon_sym_RBRACE,
    ACTIONS(221), 1,
      anon_sym_importance,
    ACTIONS(223), 1,
      anon_sym_read_limit,
    ACTIONS(217), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(37), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(219), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [815] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(225), 10,
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
  [831] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(227), 10,
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
  [847] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(221), 1,
      anon_sym_importance,
    ACTIONS(223), 1,
      anon_sym_read_limit,
    ACTIONS(229), 1,
      anon_sym_RBRACE,
    ACTIONS(217), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(31), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(219), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [873] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(231), 1,
      anon_sym_RBRACE,
    ACTIONS(233), 1,
      anon_sym_tier,
    ACTIONS(235), 1,
      anon_sym_model,
    ACTIONS(237), 1,
      anon_sym_effort,
    ACTIONS(239), 1,
      anon_sym_privacy,
    ACTIONS(241), 1,
      anon_sym_default,
    ACTIONS(243), 1,
      anon_sym_extra,
    STATE(64), 1,
      sym_extra_block,
    STATE(30), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [905] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(245), 10,
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
  [921] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(247), 1,
      anon_sym_RBRACE,
    ACTIONS(255), 1,
      anon_sym_importance,
    ACTIONS(258), 1,
      anon_sym_read_limit,
    ACTIONS(249), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(37), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(252), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [947] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(261), 10,
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
  [963] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(263), 10,
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
  [979] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(265), 10,
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
  [995] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(267), 10,
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
  [1011] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(233), 1,
      anon_sym_tier,
    ACTIONS(235), 1,
      anon_sym_model,
    ACTIONS(237), 1,
      anon_sym_effort,
    ACTIONS(239), 1,
      anon_sym_privacy,
    ACTIONS(241), 1,
      anon_sym_default,
    ACTIONS(243), 1,
      anon_sym_extra,
    ACTIONS(269), 1,
      anon_sym_RBRACE,
    STATE(64), 1,
      sym_extra_block,
    STATE(35), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [1043] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(271), 8,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [1057] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(273), 8,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [1071] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(140), 1,
      sym_tier_alias_name,
    ACTIONS(275), 7,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
      sym_identifier,
  [1087] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(277), 8,
      anon_sym_RBRACE,
      anon_sym_read_ns,
      anon_sym_write_ns,
      anon_sym_importance,
      anon_sym_staleness_sources,
      anon_sym_read_query,
      anon_sym_read_limit,
      anon_sym_write_content,
  [1101] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(279), 8,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [1115] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(22), 1,
      sym_tier_alias_name,
    ACTIONS(281), 7,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
      sym_identifier,
  [1131] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(283), 8,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [1145] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(285), 8,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [1159] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(287), 8,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [1173] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(289), 8,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
      anon_sym_strict_judge,
      anon_sym_until,
  [1187] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(291), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1200] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(293), 7,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1213] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(295), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1226] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(297), 1,
      anon_sym_RBRACE,
    STATE(104), 1,
      sym__string_value,
    STATE(63), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(299), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1245] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(301), 7,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1258] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(303), 7,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1271] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(72), 1,
      sym_tier_value,
    ACTIONS(305), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [1286] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(307), 7,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1299] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(309), 1,
      anon_sym_RBRACE,
    STATE(66), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(311), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1316] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(313), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1329] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(315), 1,
      anon_sym_RBRACE,
    STATE(104), 1,
      sym__string_value,
    STATE(63), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(317), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1348] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(320), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1361] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(322), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1374] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(324), 1,
      anon_sym_RBRACE,
    STATE(66), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(326), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1391] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(329), 7,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1404] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(331), 1,
      anon_sym_LBRACE,
    ACTIONS(333), 1,
      anon_sym_agent,
    ACTIONS(335), 1,
      anon_sym_command,
    STATE(49), 4,
      sym__until_condition,
      sym_until_verify,
      sym_until_agent,
      sym_until_command,
  [1423] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(337), 1,
      anon_sym_RBRACE,
    STATE(104), 1,
      sym__string_value,
    STATE(56), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(299), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1442] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(339), 7,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1455] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(341), 1,
      anon_sym_RBRACE,
    STATE(74), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(311), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1472] = 2,
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
  [1485] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(345), 7,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1498] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(347), 1,
      anon_sym_RBRACE,
    STATE(66), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(311), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1515] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(349), 7,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1528] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(351), 1,
      anon_sym_RBRACE,
    STATE(61), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(311), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1545] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(353), 1,
      anon_sym_RBRACE,
    ACTIONS(357), 1,
      anon_sym_depth,
    ACTIONS(355), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(78), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1563] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(359), 1,
      anon_sym_RBRACE,
    ACTIONS(364), 1,
      anon_sym_depth,
    ACTIONS(361), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(78), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1581] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(357), 1,
      anon_sym_depth,
    ACTIONS(367), 1,
      anon_sym_RBRACE,
    ACTIONS(355), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(77), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1599] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(369), 1,
      anon_sym_RBRACE,
    ACTIONS(373), 1,
      anon_sym_impact_scope,
    ACTIONS(371), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(81), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1617] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(373), 1,
      anon_sym_impact_scope,
    ACTIONS(375), 1,
      anon_sym_RBRACE,
    ACTIONS(371), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(82), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1635] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(377), 1,
      anon_sym_RBRACE,
    ACTIONS(382), 1,
      anon_sym_impact_scope,
    ACTIONS(379), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(82), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1653] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(385), 1,
      anon_sym_loop,
    ACTIONS(387), 1,
      anon_sym_RBRACK,
    ACTIONS(389), 1,
      sym_identifier,
    STATE(87), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [1670] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(391), 1,
      anon_sym_loop,
    ACTIONS(394), 1,
      anon_sym_RBRACK,
    ACTIONS(396), 1,
      sym_identifier,
    STATE(84), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [1687] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(399), 5,
      anon_sym_RBRACE,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1698] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(401), 1,
      anon_sym_RBRACK,
    ACTIONS(403), 2,
      sym_string,
      sym_raw_string,
    STATE(88), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [1713] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(385), 1,
      anon_sym_loop,
    ACTIONS(405), 1,
      anon_sym_RBRACK,
    ACTIONS(407), 1,
      sym_identifier,
    STATE(84), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [1730] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(409), 1,
      anon_sym_RBRACK,
    ACTIONS(411), 2,
      sym_string,
      sym_raw_string,
    STATE(88), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [1745] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(414), 1,
      anon_sym_RBRACK,
    ACTIONS(416), 2,
      sym_string,
      sym_raw_string,
    STATE(86), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [1760] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(418), 4,
      anon_sym_RBRACE,
      anon_sym_callers_of,
      anon_sym_tests_for,
      anon_sym_depth,
  [1770] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(420), 1,
      anon_sym_RBRACE,
    ACTIONS(422), 1,
      sym_identifier,
    STATE(91), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [1784] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(425), 1,
      anon_sym_RBRACE,
    ACTIONS(427), 1,
      sym_identifier,
    STATE(91), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [1798] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(39), 1,
      sym_strategy_value,
    ACTIONS(429), 3,
      anon_sym_single_pass,
      anon_sym_refine,
      sym_identifier,
  [1810] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(431), 4,
      anon_sym_RBRACE,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1820] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(427), 1,
      sym_identifier,
    ACTIONS(433), 1,
      anon_sym_RBRACE,
    STATE(92), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [1834] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(22), 1,
      sym__string_value,
    ACTIONS(435), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1846] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(437), 4,
      anon_sym_RBRACE,
      anon_sym_owned,
      anon_sym_read_only,
      anon_sym_impact_scope,
  [1856] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(72), 1,
      sym__string_value,
    ACTIONS(151), 2,
      sym_string,
      sym_raw_string,
  [1867] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(85), 1,
      sym_boolean,
    ACTIONS(439), 2,
      anon_sym_true,
      anon_sym_false,
  [1878] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(39), 1,
      sym_boolean,
    ACTIONS(439), 2,
      anon_sym_true,
      anon_sym_false,
  [1889] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(116), 1,
      sym__string_value,
    ACTIONS(441), 2,
      sym_string,
      sym_raw_string,
  [1900] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(72), 1,
      sym_privacy_value,
    ACTIONS(443), 2,
      anon_sym_public,
      anon_sym_local_only,
  [1911] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(445), 1,
      anon_sym_RBRACK,
    ACTIONS(447), 1,
      sym_identifier,
    STATE(103), 1,
      aux_sym_identifier_list_repeat1,
  [1924] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(94), 1,
      sym__string_value,
    ACTIONS(450), 2,
      sym_string,
      sym_raw_string,
  [1935] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(452), 1,
      anon_sym_RBRACK,
    ACTIONS(454), 1,
      sym_identifier,
    STATE(106), 1,
      aux_sym_identifier_list_repeat1,
  [1948] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(456), 1,
      anon_sym_RBRACK,
    ACTIONS(458), 1,
      sym_identifier,
    STATE(103), 1,
      aux_sym_identifier_list_repeat1,
  [1961] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(97), 1,
      sym_boolean,
    ACTIONS(439), 2,
      anon_sym_true,
      anon_sym_false,
  [1972] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(22), 1,
      sym__string_value,
    ACTIONS(435), 2,
      sym_string,
      sym_raw_string,
  [1983] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(462), 1,
      anon_sym_RBRACK,
    ACTIONS(460), 2,
      anon_sym_loop,
      sym_identifier,
  [1994] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(57), 1,
      sym__string_value,
    ACTIONS(464), 2,
      sym_string,
      sym_raw_string,
  [2005] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(43), 1,
      sym_boolean,
    ACTIONS(439), 2,
      anon_sym_true,
      anon_sym_false,
  [2016] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(46), 1,
      sym__string_value,
    ACTIONS(466), 2,
      sym_string,
      sym_raw_string,
  [2027] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(44), 1,
      sym__string_value,
    ACTIONS(468), 2,
      sym_string,
      sym_raw_string,
  [2038] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(472), 1,
      anon_sym_RBRACK,
    ACTIONS(470), 2,
      anon_sym_loop,
      sym_identifier,
  [2049] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(474), 1,
      anon_sym_LBRACK,
    STATE(39), 1,
      sym_step_list,
  [2059] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(476), 2,
      anon_sym_RBRACE,
      sym_identifier,
  [2067] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(478), 1,
      anon_sym_LBRACK,
    STATE(97), 1,
      sym_string_list,
  [2077] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(480), 1,
      anon_sym_LBRACK,
    STATE(43), 1,
      sym_identifier_list,
  [2087] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(478), 1,
      anon_sym_LBRACK,
    STATE(46), 1,
      sym_string_list,
  [2097] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(478), 1,
      anon_sym_LBRACK,
    STATE(90), 1,
      sym_string_list,
  [2107] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(480), 1,
      anon_sym_LBRACK,
    STATE(22), 1,
      sym_identifier_list,
  [2117] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(482), 1,
      sym_identifier,
  [2124] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(484), 1,
      ts_builtin_sym_end,
  [2131] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(486), 1,
      anon_sym_LBRACE,
  [2138] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(488), 1,
      sym_identifier,
  [2145] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(193), 1,
      sym_identifier,
  [2152] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(490), 1,
      sym_integer,
  [2159] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(492), 1,
      anon_sym_LBRACE,
  [2166] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(494), 1,
      anon_sym_LBRACE,
  [2173] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(466), 1,
      sym_float,
  [2180] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(466), 1,
      sym_integer,
  [2187] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(496), 1,
      sym_identifier,
  [2194] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(498), 1,
      anon_sym_LBRACE,
  [2201] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(500), 1,
      anon_sym_LBRACE,
  [2208] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(502), 1,
      sym_identifier,
  [2215] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(504), 1,
      sym_identifier,
  [2222] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(506), 1,
      anon_sym_LBRACE,
  [2229] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(508), 1,
      sym_integer,
  [2236] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(435), 1,
      sym_identifier,
  [2243] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(510), 1,
      sym_identifier,
  [2250] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(512), 1,
      anon_sym_LBRACE,
  [2257] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(514), 1,
      anon_sym_LBRACE,
  [2264] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(516), 1,
      anon_sym_LBRACE,
  [2271] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(518), 1,
      sym_integer,
  [2278] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(520), 1,
      anon_sym_LBRACE,
  [2285] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(435), 1,
      sym_integer,
};

static const uint32_t ts_small_parse_table_map[] = {
  [SMALL_STATE(2)] = 0,
  [SMALL_STATE(3)] = 32,
  [SMALL_STATE(4)] = 56,
  [SMALL_STATE(5)] = 80,
  [SMALL_STATE(6)] = 104,
  [SMALL_STATE(7)] = 128,
  [SMALL_STATE(8)] = 175,
  [SMALL_STATE(9)] = 222,
  [SMALL_STATE(10)] = 269,
  [SMALL_STATE(11)] = 304,
  [SMALL_STATE(12)] = 339,
  [SMALL_STATE(13)] = 375,
  [SMALL_STATE(14)] = 411,
  [SMALL_STATE(15)] = 447,
  [SMALL_STATE(16)] = 467,
  [SMALL_STATE(17)] = 487,
  [SMALL_STATE(18)] = 507,
  [SMALL_STATE(19)] = 527,
  [SMALL_STATE(20)] = 551,
  [SMALL_STATE(21)] = 580,
  [SMALL_STATE(22)] = 597,
  [SMALL_STATE(23)] = 614,
  [SMALL_STATE(24)] = 631,
  [SMALL_STATE(25)] = 648,
  [SMALL_STATE(26)] = 665,
  [SMALL_STATE(27)] = 694,
  [SMALL_STATE(28)] = 711,
  [SMALL_STATE(29)] = 740,
  [SMALL_STATE(30)] = 757,
  [SMALL_STATE(31)] = 789,
  [SMALL_STATE(32)] = 815,
  [SMALL_STATE(33)] = 831,
  [SMALL_STATE(34)] = 847,
  [SMALL_STATE(35)] = 873,
  [SMALL_STATE(36)] = 905,
  [SMALL_STATE(37)] = 921,
  [SMALL_STATE(38)] = 947,
  [SMALL_STATE(39)] = 963,
  [SMALL_STATE(40)] = 979,
  [SMALL_STATE(41)] = 995,
  [SMALL_STATE(42)] = 1011,
  [SMALL_STATE(43)] = 1043,
  [SMALL_STATE(44)] = 1057,
  [SMALL_STATE(45)] = 1071,
  [SMALL_STATE(46)] = 1087,
  [SMALL_STATE(47)] = 1101,
  [SMALL_STATE(48)] = 1115,
  [SMALL_STATE(49)] = 1131,
  [SMALL_STATE(50)] = 1145,
  [SMALL_STATE(51)] = 1159,
  [SMALL_STATE(52)] = 1173,
  [SMALL_STATE(53)] = 1187,
  [SMALL_STATE(54)] = 1200,
  [SMALL_STATE(55)] = 1213,
  [SMALL_STATE(56)] = 1226,
  [SMALL_STATE(57)] = 1245,
  [SMALL_STATE(58)] = 1258,
  [SMALL_STATE(59)] = 1271,
  [SMALL_STATE(60)] = 1286,
  [SMALL_STATE(61)] = 1299,
  [SMALL_STATE(62)] = 1316,
  [SMALL_STATE(63)] = 1329,
  [SMALL_STATE(64)] = 1348,
  [SMALL_STATE(65)] = 1361,
  [SMALL_STATE(66)] = 1374,
  [SMALL_STATE(67)] = 1391,
  [SMALL_STATE(68)] = 1404,
  [SMALL_STATE(69)] = 1423,
  [SMALL_STATE(70)] = 1442,
  [SMALL_STATE(71)] = 1455,
  [SMALL_STATE(72)] = 1472,
  [SMALL_STATE(73)] = 1485,
  [SMALL_STATE(74)] = 1498,
  [SMALL_STATE(75)] = 1515,
  [SMALL_STATE(76)] = 1528,
  [SMALL_STATE(77)] = 1545,
  [SMALL_STATE(78)] = 1563,
  [SMALL_STATE(79)] = 1581,
  [SMALL_STATE(80)] = 1599,
  [SMALL_STATE(81)] = 1617,
  [SMALL_STATE(82)] = 1635,
  [SMALL_STATE(83)] = 1653,
  [SMALL_STATE(84)] = 1670,
  [SMALL_STATE(85)] = 1687,
  [SMALL_STATE(86)] = 1698,
  [SMALL_STATE(87)] = 1713,
  [SMALL_STATE(88)] = 1730,
  [SMALL_STATE(89)] = 1745,
  [SMALL_STATE(90)] = 1760,
  [SMALL_STATE(91)] = 1770,
  [SMALL_STATE(92)] = 1784,
  [SMALL_STATE(93)] = 1798,
  [SMALL_STATE(94)] = 1810,
  [SMALL_STATE(95)] = 1820,
  [SMALL_STATE(96)] = 1834,
  [SMALL_STATE(97)] = 1846,
  [SMALL_STATE(98)] = 1856,
  [SMALL_STATE(99)] = 1867,
  [SMALL_STATE(100)] = 1878,
  [SMALL_STATE(101)] = 1889,
  [SMALL_STATE(102)] = 1900,
  [SMALL_STATE(103)] = 1911,
  [SMALL_STATE(104)] = 1924,
  [SMALL_STATE(105)] = 1935,
  [SMALL_STATE(106)] = 1948,
  [SMALL_STATE(107)] = 1961,
  [SMALL_STATE(108)] = 1972,
  [SMALL_STATE(109)] = 1983,
  [SMALL_STATE(110)] = 1994,
  [SMALL_STATE(111)] = 2005,
  [SMALL_STATE(112)] = 2016,
  [SMALL_STATE(113)] = 2027,
  [SMALL_STATE(114)] = 2038,
  [SMALL_STATE(115)] = 2049,
  [SMALL_STATE(116)] = 2059,
  [SMALL_STATE(117)] = 2067,
  [SMALL_STATE(118)] = 2077,
  [SMALL_STATE(119)] = 2087,
  [SMALL_STATE(120)] = 2097,
  [SMALL_STATE(121)] = 2107,
  [SMALL_STATE(122)] = 2117,
  [SMALL_STATE(123)] = 2124,
  [SMALL_STATE(124)] = 2131,
  [SMALL_STATE(125)] = 2138,
  [SMALL_STATE(126)] = 2145,
  [SMALL_STATE(127)] = 2152,
  [SMALL_STATE(128)] = 2159,
  [SMALL_STATE(129)] = 2166,
  [SMALL_STATE(130)] = 2173,
  [SMALL_STATE(131)] = 2180,
  [SMALL_STATE(132)] = 2187,
  [SMALL_STATE(133)] = 2194,
  [SMALL_STATE(134)] = 2201,
  [SMALL_STATE(135)] = 2208,
  [SMALL_STATE(136)] = 2215,
  [SMALL_STATE(137)] = 2222,
  [SMALL_STATE(138)] = 2229,
  [SMALL_STATE(139)] = 2236,
  [SMALL_STATE(140)] = 2243,
  [SMALL_STATE(141)] = 2250,
  [SMALL_STATE(142)] = 2257,
  [SMALL_STATE(143)] = 2264,
  [SMALL_STATE(144)] = 2271,
  [SMALL_STATE(145)] = 2278,
  [SMALL_STATE(146)] = 2285,
};

static const TSParseActionEntry ts_parse_actions[] = {
  [0] = {.entry = {.count = 0, .reusable = false}},
  [1] = {.entry = {.count = 1, .reusable = false}}, RECOVER(),
  [3] = {.entry = {.count = 1, .reusable = true}}, SHIFT_EXTRA(),
  [5] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 0, 0, 0),
  [7] = {.entry = {.count = 1, .reusable = true}}, SHIFT(125),
  [9] = {.entry = {.count = 1, .reusable = true}}, SHIFT(45),
  [11] = {.entry = {.count = 1, .reusable = true}}, SHIFT(141),
  [13] = {.entry = {.count = 1, .reusable = true}}, SHIFT(135),
  [15] = {.entry = {.count = 1, .reusable = true}}, SHIFT(132),
  [17] = {.entry = {.count = 1, .reusable = true}}, SHIFT(122),
  [19] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_boolean, 1, 0, 0),
  [21] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_boolean, 1, 0, 0),
  [23] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 3, 0, 0),
  [25] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 2, 0, 0),
  [27] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_block, 3, 0, 0),
  [29] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_block, 4, 0, 0),
  [31] = {.entry = {.count = 1, .reusable = true}}, SHIFT(139),
  [33] = {.entry = {.count = 1, .reusable = true}}, SHIFT(73),
  [35] = {.entry = {.count = 1, .reusable = true}}, SHIFT(48),
  [37] = {.entry = {.count = 1, .reusable = true}}, SHIFT(96),
  [39] = {.entry = {.count = 1, .reusable = true}}, SHIFT(108),
  [41] = {.entry = {.count = 1, .reusable = true}}, SHIFT(121),
  [43] = {.entry = {.count = 1, .reusable = true}}, SHIFT(146),
  [45] = {.entry = {.count = 1, .reusable = true}}, SHIFT(142),
  [47] = {.entry = {.count = 1, .reusable = true}}, SHIFT(143),
  [49] = {.entry = {.count = 1, .reusable = true}}, SHIFT(133),
  [51] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(139),
  [54] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0),
  [56] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(48),
  [59] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(141),
  [62] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(96),
  [65] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(108),
  [68] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(121),
  [71] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(146),
  [74] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(142),
  [77] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(143),
  [80] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(133),
  [83] = {.entry = {.count = 1, .reusable = true}}, SHIFT(67),
  [85] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0),
  [87] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(125),
  [90] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(45),
  [93] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(141),
  [96] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(135),
  [99] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(132),
  [102] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(122),
  [105] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 1, 0, 0),
  [107] = {.entry = {.count = 1, .reusable = true}}, SHIFT(60),
  [109] = {.entry = {.count = 1, .reusable = true}}, SHIFT(144),
  [111] = {.entry = {.count = 1, .reusable = true}}, SHIFT(137),
  [113] = {.entry = {.count = 1, .reusable = true}}, SHIFT(115),
  [115] = {.entry = {.count = 1, .reusable = true}}, SHIFT(93),
  [117] = {.entry = {.count = 1, .reusable = true}}, SHIFT(100),
  [119] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0),
  [121] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(144),
  [124] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(143),
  [127] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(137),
  [130] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(115),
  [133] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(93),
  [136] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(100),
  [139] = {.entry = {.count = 1, .reusable = true}}, SHIFT(54),
  [141] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_block, 3, 0, 0),
  [143] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_block, 4, 0, 0),
  [145] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 2, 0, 0),
  [147] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 3, 0, 0),
  [149] = {.entry = {.count = 1, .reusable = false}}, SHIFT(53),
  [151] = {.entry = {.count = 1, .reusable = true}}, SHIFT(72),
  [153] = {.entry = {.count = 1, .reusable = false}}, SHIFT(72),
  [155] = {.entry = {.count = 1, .reusable = true}}, SHIFT(114),
  [157] = {.entry = {.count = 1, .reusable = true}}, SHIFT(118),
  [159] = {.entry = {.count = 1, .reusable = true}}, SHIFT(127),
  [161] = {.entry = {.count = 1, .reusable = true}}, SHIFT(111),
  [163] = {.entry = {.count = 1, .reusable = true}}, SHIFT(68),
  [165] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 1, 0, 0),
  [167] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 2, 0, 0),
  [169] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 3, 0, 0),
  [171] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_block, 3, 0, 0),
  [173] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 4, 0, 0),
  [175] = {.entry = {.count = 1, .reusable = true}}, SHIFT(109),
  [177] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_block, 4, 0, 0),
  [179] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0),
  [181] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(118),
  [184] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(127),
  [187] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(111),
  [190] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(68),
  [193] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_alias_name, 1, 0, 0),
  [195] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0),
  [197] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(59),
  [200] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(98),
  [203] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(19),
  [206] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(102),
  [209] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(64),
  [212] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(124),
  [215] = {.entry = {.count = 1, .reusable = true}}, SHIFT(6),
  [217] = {.entry = {.count = 1, .reusable = true}}, SHIFT(119),
  [219] = {.entry = {.count = 1, .reusable = true}}, SHIFT(112),
  [221] = {.entry = {.count = 1, .reusable = true}}, SHIFT(130),
  [223] = {.entry = {.count = 1, .reusable = true}}, SHIFT(131),
  [225] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_block, 3, 0, 0),
  [227] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_step_list, 2, 0, 0),
  [229] = {.entry = {.count = 1, .reusable = true}}, SHIFT(5),
  [231] = {.entry = {.count = 1, .reusable = true}}, SHIFT(70),
  [233] = {.entry = {.count = 1, .reusable = true}}, SHIFT(59),
  [235] = {.entry = {.count = 1, .reusable = true}}, SHIFT(98),
  [237] = {.entry = {.count = 1, .reusable = true}}, SHIFT(19),
  [239] = {.entry = {.count = 1, .reusable = true}}, SHIFT(102),
  [241] = {.entry = {.count = 1, .reusable = true}}, SHIFT(64),
  [243] = {.entry = {.count = 1, .reusable = true}}, SHIFT(124),
  [245] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_block, 4, 0, 0),
  [247] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0),
  [249] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(119),
  [252] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(112),
  [255] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(130),
  [258] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(131),
  [261] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_step_list, 3, 0, 0),
  [263] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 2, 0, 0),
  [265] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_strategy_value, 1, 0, 0),
  [267] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 1, 0, 0),
  [269] = {.entry = {.count = 1, .reusable = true}}, SHIFT(58),
  [271] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_field, 2, 0, 0),
  [273] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_command, 2, 0, 0),
  [275] = {.entry = {.count = 1, .reusable = false}}, SHIFT(126),
  [277] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_field, 2, 0, 0),
  [279] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_field, 1, 0, 0),
  [281] = {.entry = {.count = 1, .reusable = false}}, SHIFT(29),
  [283] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_clause, 2, 0, 0),
  [285] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_verify, 2, 0, 0),
  [287] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_agent, 2, 0, 0),
  [289] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_verify, 3, 0, 0),
  [291] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_value, 1, 0, 0),
  [293] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 5, 0, 0),
  [295] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_block, 3, 0, 0),
  [297] = {.entry = {.count = 1, .reusable = true}}, SHIFT(62),
  [299] = {.entry = {.count = 1, .reusable = true}}, SHIFT(104),
  [301] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_prompt_declaration, 3, 0, 0),
  [303] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 4, 0, 0),
  [305] = {.entry = {.count = 1, .reusable = true}}, SHIFT(53),
  [307] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 4, 0, 0),
  [309] = {.entry = {.count = 1, .reusable = true}}, SHIFT(36),
  [311] = {.entry = {.count = 1, .reusable = true}}, SHIFT(99),
  [313] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_block, 4, 0, 0),
  [315] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_extra_block_repeat1, 2, 0, 0),
  [317] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_extra_block_repeat1, 2, 0, 0), SHIFT_REPEAT(104),
  [320] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_field, 1, 0, 0),
  [322] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_privacy_value, 1, 0, 0),
  [324] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_verify_block_repeat1, 2, 0, 0),
  [326] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_verify_block_repeat1, 2, 0, 0), SHIFT_REPEAT(99),
  [329] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 4, 0, 0),
  [331] = {.entry = {.count = 1, .reusable = true}}, SHIFT(71),
  [333] = {.entry = {.count = 1, .reusable = true}}, SHIFT(136),
  [335] = {.entry = {.count = 1, .reusable = true}}, SHIFT(113),
  [337] = {.entry = {.count = 1, .reusable = true}}, SHIFT(55),
  [339] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 5, 0, 0),
  [341] = {.entry = {.count = 1, .reusable = true}}, SHIFT(50),
  [343] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_field, 2, 0, 0),
  [345] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 5, 0, 0),
  [347] = {.entry = {.count = 1, .reusable = true}}, SHIFT(52),
  [349] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_alias_declaration, 3, 0, 0),
  [351] = {.entry = {.count = 1, .reusable = true}}, SHIFT(32),
  [353] = {.entry = {.count = 1, .reusable = true}}, SHIFT(27),
  [355] = {.entry = {.count = 1, .reusable = true}}, SHIFT(120),
  [357] = {.entry = {.count = 1, .reusable = true}}, SHIFT(138),
  [359] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0),
  [361] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0), SHIFT_REPEAT(120),
  [364] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0), SHIFT_REPEAT(138),
  [367] = {.entry = {.count = 1, .reusable = true}}, SHIFT(24),
  [369] = {.entry = {.count = 1, .reusable = true}}, SHIFT(23),
  [371] = {.entry = {.count = 1, .reusable = true}}, SHIFT(117),
  [373] = {.entry = {.count = 1, .reusable = true}}, SHIFT(107),
  [375] = {.entry = {.count = 1, .reusable = true}}, SHIFT(25),
  [377] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0),
  [379] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(117),
  [382] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(107),
  [385] = {.entry = {.count = 1, .reusable = false}}, SHIFT(145),
  [387] = {.entry = {.count = 1, .reusable = true}}, SHIFT(33),
  [389] = {.entry = {.count = 1, .reusable = false}}, SHIFT(87),
  [391] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0), SHIFT_REPEAT(145),
  [394] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0),
  [396] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0), SHIFT_REPEAT(84),
  [399] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_field, 2, 0, 0),
  [401] = {.entry = {.count = 1, .reusable = true}}, SHIFT(18),
  [403] = {.entry = {.count = 1, .reusable = true}}, SHIFT(88),
  [405] = {.entry = {.count = 1, .reusable = true}}, SHIFT(38),
  [407] = {.entry = {.count = 1, .reusable = false}}, SHIFT(84),
  [409] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0),
  [411] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0), SHIFT_REPEAT(88),
  [414] = {.entry = {.count = 1, .reusable = true}}, SHIFT(17),
  [416] = {.entry = {.count = 1, .reusable = true}}, SHIFT(86),
  [418] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_field, 2, 0, 0),
  [420] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_vars_block_repeat1, 2, 0, 0),
  [422] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_vars_block_repeat1, 2, 0, 0), SHIFT_REPEAT(101),
  [425] = {.entry = {.count = 1, .reusable = true}}, SHIFT(16),
  [427] = {.entry = {.count = 1, .reusable = true}}, SHIFT(101),
  [429] = {.entry = {.count = 1, .reusable = false}}, SHIFT(40),
  [431] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_pair, 2, 0, 0),
  [433] = {.entry = {.count = 1, .reusable = true}}, SHIFT(15),
  [435] = {.entry = {.count = 1, .reusable = true}}, SHIFT(22),
  [437] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_field, 2, 0, 0),
  [439] = {.entry = {.count = 1, .reusable = true}}, SHIFT(2),
  [441] = {.entry = {.count = 1, .reusable = true}}, SHIFT(116),
  [443] = {.entry = {.count = 1, .reusable = true}}, SHIFT(65),
  [445] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0),
  [447] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0), SHIFT_REPEAT(103),
  [450] = {.entry = {.count = 1, .reusable = true}}, SHIFT(94),
  [452] = {.entry = {.count = 1, .reusable = true}}, SHIFT(4),
  [454] = {.entry = {.count = 1, .reusable = true}}, SHIFT(106),
  [456] = {.entry = {.count = 1, .reusable = true}}, SHIFT(3),
  [458] = {.entry = {.count = 1, .reusable = true}}, SHIFT(103),
  [460] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_loop_block, 3, 0, 0),
  [462] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_block, 3, 0, 0),
  [464] = {.entry = {.count = 1, .reusable = true}}, SHIFT(57),
  [466] = {.entry = {.count = 1, .reusable = true}}, SHIFT(46),
  [468] = {.entry = {.count = 1, .reusable = true}}, SHIFT(44),
  [470] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_loop_block, 4, 0, 0),
  [472] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_block, 4, 0, 0),
  [474] = {.entry = {.count = 1, .reusable = true}}, SHIFT(83),
  [476] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_pair, 2, 0, 0),
  [478] = {.entry = {.count = 1, .reusable = true}}, SHIFT(89),
  [480] = {.entry = {.count = 1, .reusable = true}}, SHIFT(105),
  [482] = {.entry = {.count = 1, .reusable = true}}, SHIFT(129),
  [484] = {.entry = {.count = 1, .reusable = true}},  ACCEPT_INPUT(),
  [486] = {.entry = {.count = 1, .reusable = true}}, SHIFT(69),
  [488] = {.entry = {.count = 1, .reusable = true}}, SHIFT(134),
  [490] = {.entry = {.count = 1, .reusable = true}}, SHIFT(43),
  [492] = {.entry = {.count = 1, .reusable = true}}, SHIFT(9),
  [494] = {.entry = {.count = 1, .reusable = true}}, SHIFT(12),
  [496] = {.entry = {.count = 1, .reusable = true}}, SHIFT(128),
  [498] = {.entry = {.count = 1, .reusable = true}}, SHIFT(79),
  [500] = {.entry = {.count = 1, .reusable = true}}, SHIFT(42),
  [502] = {.entry = {.count = 1, .reusable = true}}, SHIFT(110),
  [504] = {.entry = {.count = 1, .reusable = true}}, SHIFT(51),
  [506] = {.entry = {.count = 1, .reusable = true}}, SHIFT(76),
  [508] = {.entry = {.count = 1, .reusable = true}}, SHIFT(90),
  [510] = {.entry = {.count = 1, .reusable = true}}, SHIFT(75),
  [512] = {.entry = {.count = 1, .reusable = true}}, SHIFT(95),
  [514] = {.entry = {.count = 1, .reusable = true}}, SHIFT(80),
  [516] = {.entry = {.count = 1, .reusable = true}}, SHIFT(34),
  [518] = {.entry = {.count = 1, .reusable = true}}, SHIFT(39),
  [520] = {.entry = {.count = 1, .reusable = true}}, SHIFT(26),
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
