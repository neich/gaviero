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
#define STATE_COUNT 118
#define LARGE_STATE_COUNT 2
#define SYMBOL_COUNT 108
#define ALIAS_COUNT 0
#define TOKEN_COUNT 65
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
  anon_sym_privacy = 7,
  anon_sym_agent = 8,
  anon_sym_description = 9,
  anon_sym_depends_on = 10,
  anon_sym_prompt = 11,
  anon_sym_max_retries = 12,
  anon_sym_scope = 13,
  anon_sym_owned = 14,
  anon_sym_read_only = 15,
  anon_sym_impact_scope = 16,
  anon_sym_memory = 17,
  anon_sym_read_ns = 18,
  anon_sym_write_ns = 19,
  anon_sym_importance = 20,
  anon_sym_staleness_sources = 21,
  anon_sym_read_query = 22,
  anon_sym_read_limit = 23,
  anon_sym_write_content = 24,
  anon_sym_verify = 25,
  anon_sym_compile = 26,
  anon_sym_clippy = 27,
  anon_sym_test = 28,
  anon_sym_impact_tests = 29,
  anon_sym_context = 30,
  anon_sym_callers_of = 31,
  anon_sym_tests_for = 32,
  anon_sym_depth = 33,
  anon_sym_loop = 34,
  anon_sym_agents = 35,
  anon_sym_max_iterations = 36,
  anon_sym_until = 37,
  anon_sym_command = 38,
  anon_sym_workflow = 39,
  anon_sym_steps = 40,
  anon_sym_max_parallel = 41,
  anon_sym_strategy = 42,
  anon_sym_test_first = 43,
  anon_sym_attempts = 44,
  anon_sym_escalate_after = 45,
  anon_sym_LBRACK = 46,
  anon_sym_RBRACK = 47,
  anon_sym_cheap = 48,
  anon_sym_expensive = 49,
  anon_sym_coordinator = 50,
  anon_sym_reasoning = 51,
  anon_sym_execution = 52,
  anon_sym_mechanical = 53,
  anon_sym_public = 54,
  anon_sym_local_only = 55,
  anon_sym_single_pass = 56,
  anon_sym_refine = 57,
  anon_sym_true = 58,
  anon_sym_false = 59,
  sym_string = 60,
  sym_raw_string = 61,
  sym_float = 62,
  sym_integer = 63,
  sym_identifier = 64,
  sym_source_file = 65,
  sym__definition = 66,
  sym_client_declaration = 67,
  sym_client_field = 68,
  sym_agent_declaration = 69,
  sym_agent_field = 70,
  sym_scope_block = 71,
  sym_scope_field = 72,
  sym_memory_block = 73,
  sym_memory_field = 74,
  sym_verify_block = 75,
  sym_verify_field = 76,
  sym_context_block = 77,
  sym_context_field = 78,
  sym_loop_block = 79,
  sym_loop_field = 80,
  sym_until_clause = 81,
  sym__until_condition = 82,
  sym_until_verify = 83,
  sym_until_agent = 84,
  sym_until_command = 85,
  sym_workflow_declaration = 86,
  sym_workflow_field = 87,
  sym_step_list = 88,
  sym_string_list = 89,
  sym_identifier_list = 90,
  sym_tier_value = 91,
  sym_privacy_value = 92,
  sym_strategy_value = 93,
  sym_boolean = 94,
  sym__string_value = 95,
  aux_sym_source_file_repeat1 = 96,
  aux_sym_client_declaration_repeat1 = 97,
  aux_sym_agent_declaration_repeat1 = 98,
  aux_sym_scope_block_repeat1 = 99,
  aux_sym_memory_block_repeat1 = 100,
  aux_sym_verify_block_repeat1 = 101,
  aux_sym_context_block_repeat1 = 102,
  aux_sym_loop_block_repeat1 = 103,
  aux_sym_workflow_declaration_repeat1 = 104,
  aux_sym_step_list_repeat1 = 105,
  aux_sym_string_list_repeat1 = 106,
  aux_sym_identifier_list_repeat1 = 107,
};

static const char * const ts_symbol_names[] = {
  [ts_builtin_sym_end] = "end",
  [sym_comment] = "comment",
  [anon_sym_client] = "client",
  [anon_sym_LBRACE] = "{",
  [anon_sym_RBRACE] = "}",
  [anon_sym_tier] = "tier",
  [anon_sym_model] = "model",
  [anon_sym_privacy] = "privacy",
  [anon_sym_agent] = "agent",
  [anon_sym_description] = "description",
  [anon_sym_depends_on] = "depends_on",
  [anon_sym_prompt] = "prompt",
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
  [anon_sym_cheap] = "cheap",
  [anon_sym_expensive] = "expensive",
  [anon_sym_coordinator] = "coordinator",
  [anon_sym_reasoning] = "reasoning",
  [anon_sym_execution] = "execution",
  [anon_sym_mechanical] = "mechanical",
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
  [anon_sym_privacy] = anon_sym_privacy,
  [anon_sym_agent] = anon_sym_agent,
  [anon_sym_description] = anon_sym_description,
  [anon_sym_depends_on] = anon_sym_depends_on,
  [anon_sym_prompt] = anon_sym_prompt,
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
  [anon_sym_cheap] = anon_sym_cheap,
  [anon_sym_expensive] = anon_sym_expensive,
  [anon_sym_coordinator] = anon_sym_coordinator,
  [anon_sym_reasoning] = anon_sym_reasoning,
  [anon_sym_execution] = anon_sym_execution,
  [anon_sym_mechanical] = anon_sym_mechanical,
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
  [anon_sym_privacy] = {
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
  [anon_sym_prompt] = {
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
};

static bool ts_lex(TSLexer *lexer, TSStateId state) {
  START_LEXER();
  eof = lexer->eof(lexer);
  switch (state) {
    case 0:
      if (eof) ADVANCE(370);
      ADVANCE_MAP(
        '"', 1,
        '#', 3,
        '/', 12,
        '[', 419,
        ']', 420,
        'a', 142,
        'c', 29,
        'd', 86,
        'e', 289,
        'f', 34,
        'i', 193,
        'l', 230,
        'm', 31,
        'o', 357,
        'p', 268,
        'r', 87,
        's', 62,
        't', 88,
        'u', 209,
        'v', 89,
        'w', 232,
        '{', 373,
        '}', 374,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(0);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(438);
      END_STATE();
    case 1:
      if (lookahead == '"') ADVANCE(435);
      if (lookahead != 0) ADVANCE(1);
      END_STATE();
    case 2:
      if (lookahead == '"') ADVANCE(4);
      if (lookahead != 0) ADVANCE(2);
      END_STATE();
    case 3:
      if (lookahead == '"') ADVANCE(2);
      END_STATE();
    case 4:
      if (lookahead == '#') ADVANCE(436);
      if (lookahead != 0) ADVANCE(2);
      END_STATE();
    case 5:
      if (lookahead == '.') ADVANCE(369);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(5);
      END_STATE();
    case 6:
      if (lookahead == '/') ADVANCE(12);
      if (lookahead == ']') ADVANCE(420);
      if (lookahead == 'l') ADVANCE(453);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(6);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 7:
      if (lookahead == '/') ADVANCE(12);
      if (lookahead == ']') ADVANCE(420);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(7);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 8:
      ADVANCE_MAP(
        '/', 12,
        'a', 316,
        'c', 30,
        'd', 86,
        'e', 288,
        'i', 204,
        'm', 49,
        'o', 357,
        'p', 282,
        'r', 120,
        's', 63,
        't', 132,
        'v', 89,
        'w', 274,
        '}', 374,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(8);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(5);
      END_STATE();
    case 9:
      ADVANCE_MAP(
        '/', 12,
        'a', 145,
        'c', 172,
        'd', 99,
        'e', 288,
        'i', 198,
        'm', 32,
        'o', 357,
        'p', 282,
        'r', 116,
        's', 64,
        't', 130,
        'u', 209,
        'v', 89,
        '}', 374,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(9);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(439);
      END_STATE();
    case 10:
      if (lookahead == '/') ADVANCE(12);
      if (lookahead == 'c') ADVANCE(187);
      if (lookahead == 'i') ADVANCE(203);
      if (lookahead == 't') ADVANCE(134);
      if (lookahead == '}') ADVANCE(374);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(10);
      END_STATE();
    case 11:
      if (lookahead == '/') ADVANCE(12);
      if (lookahead == 'r') ADVANCE(442);
      if (lookahead == 's') ADVANCE(447);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(11);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 12:
      if (lookahead == '/') ADVANCE(371);
      END_STATE();
    case 13:
      if (lookahead == '_') ADVANCE(166);
      END_STATE();
    case 14:
      if (lookahead == '_') ADVANCE(185);
      END_STATE();
    case 15:
      if (lookahead == '_') ADVANCE(67);
      END_STATE();
    case 16:
      if (lookahead == '_') ADVANCE(302);
      END_STATE();
    case 17:
      if (lookahead == '_') ADVANCE(255);
      END_STATE();
    case 18:
      if (lookahead == '_') ADVANCE(138);
      if (lookahead == 's') ADVANCE(19);
      END_STATE();
    case 19:
      if (lookahead == '_') ADVANCE(139);
      END_STATE();
    case 20:
      if (lookahead == '_') ADVANCE(235);
      END_STATE();
    case 21:
      if (lookahead == '_') ADVANCE(259);
      END_STATE();
    case 22:
      if (lookahead == '_') ADVANCE(43);
      END_STATE();
    case 23:
      if (lookahead == '_') ADVANCE(303);
      END_STATE();
    case 24:
      if (lookahead == '_') ADVANCE(301);
      END_STATE();
    case 25:
      if (lookahead == '_') ADVANCE(247);
      END_STATE();
    case 26:
      if (lookahead == '_') ADVANCE(239);
      END_STATE();
    case 27:
      if (lookahead == '_') ADVANCE(237);
      END_STATE();
    case 28:
      if (lookahead == '_') ADVANCE(341);
      END_STATE();
    case 29:
      if (lookahead == 'a') ADVANCE(178);
      if (lookahead == 'h') ADVANCE(98);
      if (lookahead == 'l') ADVANCE(151);
      if (lookahead == 'o') ADVANCE(194);
      END_STATE();
    case 30:
      if (lookahead == 'a') ADVANCE(178);
      if (lookahead == 'l') ADVANCE(170);
      if (lookahead == 'o') ADVANCE(228);
      END_STATE();
    case 31:
      if (lookahead == 'a') ADVANCE(358);
      if (lookahead == 'e') ADVANCE(60);
      if (lookahead == 'o') ADVANCE(83);
      END_STATE();
    case 32:
      if (lookahead == 'a') ADVANCE(358);
      if (lookahead == 'e') ADVANCE(195);
      END_STATE();
    case 33:
      if (lookahead == 'a') ADVANCE(81);
      if (lookahead == 'f') ADVANCE(162);
      END_STATE();
    case 34:
      if (lookahead == 'a') ADVANCE(177);
      END_STATE();
    case 35:
      if (lookahead == 'a') ADVANCE(68);
      END_STATE();
    case 36:
      if (lookahead == 'a') ADVANCE(68);
      if (lookahead == 'o') ADVANCE(279);
      END_STATE();
    case 37:
      if (lookahead == 'a') ADVANCE(192);
      END_STATE();
    case 38:
      if (lookahead == 'a') ADVANCE(66);
      END_STATE();
    case 39:
      if (lookahead == 'a') ADVANCE(252);
      END_STATE();
    case 40:
      if (lookahead == 'a') ADVANCE(221);
      END_STATE();
    case 41:
      if (lookahead == 'a') ADVANCE(179);
      END_STATE();
    case 42:
      if (lookahead == 'a') ADVANCE(85);
      END_STATE();
    case 43:
      if (lookahead == 'a') ADVANCE(140);
      END_STATE();
    case 44:
      if (lookahead == 'a') ADVANCE(80);
      END_STATE();
    case 45:
      if (lookahead == 'a') ADVANCE(287);
      END_STATE();
    case 46:
      if (lookahead == 'a') ADVANCE(217);
      END_STATE();
    case 47:
      if (lookahead == 'a') ADVANCE(345);
      END_STATE();
    case 48:
      if (lookahead == 'a') ADVANCE(175);
      END_STATE();
    case 49:
      if (lookahead == 'a') ADVANCE(360);
      if (lookahead == 'e') ADVANCE(195);
      END_STATE();
    case 50:
      if (lookahead == 'a') ADVANCE(186);
      if (lookahead == 'e') ADVANCE(254);
      if (lookahead == 'r') ADVANCE(53);
      END_STATE();
    case 51:
      if (lookahead == 'a') ADVANCE(215);
      END_STATE();
    case 52:
      if (lookahead == 'a') ADVANCE(307);
      END_STATE();
    case 53:
      if (lookahead == 'a') ADVANCE(338);
      END_STATE();
    case 54:
      if (lookahead == 'a') ADVANCE(340);
      END_STATE();
    case 55:
      if (lookahead == 'a') ADVANCE(191);
      END_STATE();
    case 56:
      if (lookahead == 'a') ADVANCE(73);
      if (lookahead == 'o') ADVANCE(279);
      END_STATE();
    case 57:
      if (lookahead == 'a') ADVANCE(75);
      END_STATE();
    case 58:
      if (lookahead == 'a') ADVANCE(349);
      END_STATE();
    case 59:
      if (lookahead == 'b') ADVANCE(182);
      END_STATE();
    case 60:
      if (lookahead == 'c') ADVANCE(147);
      if (lookahead == 'm') ADVANCE(236);
      END_STATE();
    case 61:
      if (lookahead == 'c') ADVANCE(427);
      END_STATE();
    case 62:
      if (lookahead == 'c') ADVANCE(233);
      if (lookahead == 'i') ADVANCE(205);
      if (lookahead == 't') ADVANCE(50);
      END_STATE();
    case 63:
      if (lookahead == 'c') ADVANCE(233);
      if (lookahead == 't') ADVANCE(50);
      END_STATE();
    case 64:
      if (lookahead == 'c') ADVANCE(233);
      if (lookahead == 't') ADVANCE(106);
      END_STATE();
    case 65:
      if (lookahead == 'c') ADVANCE(353);
      END_STATE();
    case 66:
      if (lookahead == 'c') ADVANCE(364);
      END_STATE();
    case 67:
      if (lookahead == 'c') ADVANCE(248);
      if (lookahead == 'n') ADVANCE(293);
      END_STATE();
    case 68:
      if (lookahead == 'c') ADVANCE(329);
      END_STATE();
    case 69:
      if (lookahead == 'c') ADVANCE(96);
      END_STATE();
    case 70:
      if (lookahead == 'c') ADVANCE(123);
      END_STATE();
    case 71:
      if (lookahead == 'c') ADVANCE(37);
      END_STATE();
    case 72:
      if (lookahead == 'c') ADVANCE(283);
      END_STATE();
    case 73:
      if (lookahead == 'c') ADVANCE(343);
      END_STATE();
    case 74:
      if (lookahead == 'c') ADVANCE(41);
      if (lookahead == 'o') ADVANCE(251);
      END_STATE();
    case 75:
      if (lookahead == 'c') ADVANCE(333);
      END_STATE();
    case 76:
      if (lookahead == 'c') ADVANCE(48);
      END_STATE();
    case 77:
      if (lookahead == 'c') ADVANCE(245);
      END_STATE();
    case 78:
      if (lookahead == 'd') ADVANCE(384);
      END_STATE();
    case 79:
      if (lookahead == 'd') ADVANCE(411);
      END_STATE();
    case 80:
      if (lookahead == 'd') ADVANCE(14);
      END_STATE();
    case 81:
      if (lookahead == 'd') ADVANCE(14);
      if (lookahead == 's') ADVANCE(246);
      END_STATE();
    case 82:
      if (lookahead == 'd') ADVANCE(154);
      END_STATE();
    case 83:
      if (lookahead == 'd') ADVANCE(110);
      END_STATE();
    case 84:
      if (lookahead == 'd') ADVANCE(315);
      END_STATE();
    case 85:
      if (lookahead == 'd') ADVANCE(27);
      END_STATE();
    case 86:
      if (lookahead == 'e') ADVANCE(249);
      END_STATE();
    case 87:
      if (lookahead == 'e') ADVANCE(33);
      END_STATE();
    case 88:
      if (lookahead == 'e') ADVANCE(300);
      if (lookahead == 'i') ADVANCE(107);
      if (lookahead == 'r') ADVANCE(350);
      END_STATE();
    case 89:
      if (lookahead == 'e') ADVANCE(277);
      END_STATE();
    case 90:
      if (lookahead == 'e') ADVANCE(433);
      END_STATE();
    case 91:
      if (lookahead == 'e') ADVANCE(434);
      END_STATE();
    case 92:
      if (lookahead == 'e') ADVANCE(383);
      END_STATE();
    case 93:
      if (lookahead == 'e') ADVANCE(431);
      END_STATE();
    case 94:
      if (lookahead == 'e') ADVANCE(396);
      END_STATE();
    case 95:
      if (lookahead == 'e') ADVANCE(422);
      END_STATE();
    case 96:
      if (lookahead == 'e') ADVANCE(390);
      END_STATE();
    case 97:
      if (lookahead == 'e') ADVANCE(386);
      END_STATE();
    case 98:
      if (lookahead == 'e') ADVANCE(39);
      END_STATE();
    case 99:
      if (lookahead == 'e') ADVANCE(264);
      END_STATE();
    case 100:
      if (lookahead == 'e') ADVANCE(359);
      END_STATE();
    case 101:
      if (lookahead == 'e') ADVANCE(143);
      END_STATE();
    case 102:
      if (lookahead == 'e') ADVANCE(65);
      if (lookahead == 'p') ADVANCE(108);
      END_STATE();
    case 103:
      if (lookahead == 'e') ADVANCE(78);
      END_STATE();
    case 104:
      if (lookahead == 'e') ADVANCE(212);
      END_STATE();
    case 105:
      if (lookahead == 'e') ADVANCE(212);
      if (lookahead == 't') ADVANCE(146);
      END_STATE();
    case 106:
      if (lookahead == 'e') ADVANCE(254);
      if (lookahead == 'r') ADVANCE(53);
      END_STATE();
    case 107:
      if (lookahead == 'e') ADVANCE(270);
      END_STATE();
    case 108:
      if (lookahead == 'e') ADVANCE(213);
      END_STATE();
    case 109:
      if (lookahead == 'e') ADVANCE(15);
      END_STATE();
    case 110:
      if (lookahead == 'e') ADVANCE(173);
      END_STATE();
    case 111:
      if (lookahead == 'e') ADVANCE(278);
      END_STATE();
    case 112:
      if (lookahead == 'e') ADVANCE(21);
      END_STATE();
    case 113:
      if (lookahead == 'e') ADVANCE(305);
      END_STATE();
    case 114:
      if (lookahead == 'e') ADVANCE(22);
      END_STATE();
    case 115:
      if (lookahead == 'e') ADVANCE(280);
      END_STATE();
    case 116:
      if (lookahead == 'e') ADVANCE(42);
      END_STATE();
    case 117:
      if (lookahead == 'e') ADVANCE(344);
      END_STATE();
    case 118:
      if (lookahead == 'e') ADVANCE(294);
      END_STATE();
    case 119:
      if (lookahead == 'e') ADVANCE(281);
      END_STATE();
    case 120:
      if (lookahead == 'e') ADVANCE(44);
      END_STATE();
    case 121:
      if (lookahead == 'e') ADVANCE(176);
      END_STATE();
    case 122:
      if (lookahead == 'e') ADVANCE(273);
      END_STATE();
    case 123:
      if (lookahead == 'e') ADVANCE(298);
      END_STATE();
    case 124:
      if (lookahead == 'e') ADVANCE(211);
      END_STATE();
    case 125:
      if (lookahead == 'e') ADVANCE(202);
      END_STATE();
    case 126:
      if (lookahead == 'e') ADVANCE(214);
      END_STATE();
    case 127:
      if (lookahead == 'e') ADVANCE(214);
      if (lookahead == 'p') ADVANCE(253);
      END_STATE();
    case 128:
      if (lookahead == 'e') ADVANCE(310);
      END_STATE();
    case 129:
      if (lookahead == 'e') ADVANCE(227);
      END_STATE();
    case 130:
      if (lookahead == 'e') ADVANCE(311);
      END_STATE();
    case 131:
      if (lookahead == 'e') ADVANCE(224);
      END_STATE();
    case 132:
      if (lookahead == 'e') ADVANCE(312);
      END_STATE();
    case 133:
      if (lookahead == 'e') ADVANCE(225);
      END_STATE();
    case 134:
      if (lookahead == 'e') ADVANCE(313);
      END_STATE();
    case 135:
      if (lookahead == 'f') ADVANCE(403);
      END_STATE();
    case 136:
      if (lookahead == 'f') ADVANCE(363);
      END_STATE();
    case 137:
      if (lookahead == 'f') ADVANCE(180);
      END_STATE();
    case 138:
      if (lookahead == 'f') ADVANCE(158);
      END_STATE();
    case 139:
      if (lookahead == 'f') ADVANCE(240);
      END_STATE();
    case 140:
      if (lookahead == 'f') ADVANCE(342);
      END_STATE();
    case 141:
      if (lookahead == 'g') ADVANCE(424);
      END_STATE();
    case 142:
      if (lookahead == 'g') ADVANCE(124);
      if (lookahead == 't') ADVANCE(330);
      END_STATE();
    case 143:
      if (lookahead == 'g') ADVANCE(365);
      END_STATE();
    case 144:
      if (lookahead == 'g') ADVANCE(188);
      END_STATE();
    case 145:
      if (lookahead == 'g') ADVANCE(133);
      if (lookahead == 't') ADVANCE(330);
      END_STATE();
    case 146:
      if (lookahead == 'h') ADVANCE(405);
      END_STATE();
    case 147:
      if (lookahead == 'h') ADVANCE(40);
      END_STATE();
    case 148:
      if (lookahead == 'i') ADVANCE(354);
      if (lookahead == 'o') ADVANCE(197);
      END_STATE();
    case 149:
      if (lookahead == 'i') ADVANCE(136);
      END_STATE();
    case 150:
      if (lookahead == 'i') ADVANCE(201);
      END_STATE();
    case 151:
      if (lookahead == 'i') ADVANCE(127);
      END_STATE();
    case 152:
      if (lookahead == 'i') ADVANCE(355);
      END_STATE();
    case 153:
      if (lookahead == 'i') ADVANCE(61);
      END_STATE();
    case 154:
      if (lookahead == 'i') ADVANCE(216);
      END_STATE();
    case 155:
      if (lookahead == 'i') ADVANCE(174);
      END_STATE();
    case 156:
      if (lookahead == 'i') ADVANCE(266);
      END_STATE();
    case 157:
      if (lookahead == 'i') ADVANCE(210);
      END_STATE();
    case 158:
      if (lookahead == 'i') ADVANCE(285);
      END_STATE();
    case 159:
      if (lookahead == 'i') ADVANCE(257);
      END_STATE();
    case 160:
      if (lookahead == 'i') ADVANCE(322);
      END_STATE();
    case 161:
      if (lookahead == 'i') ADVANCE(118);
      END_STATE();
    case 162:
      if (lookahead == 'i') ADVANCE(226);
      END_STATE();
    case 163:
      if (lookahead == 'i') ADVANCE(336);
      END_STATE();
    case 164:
      if (lookahead == 'i') ADVANCE(238);
      END_STATE();
    case 165:
      if (lookahead == 'i') ADVANCE(189);
      END_STATE();
    case 166:
      if (lookahead == 'i') ADVANCE(339);
      if (lookahead == 'p') ADVANCE(45);
      if (lookahead == 'r') ADVANCE(117);
      END_STATE();
    case 167:
      if (lookahead == 'i') ADVANCE(241);
      END_STATE();
    case 168:
      if (lookahead == 'i') ADVANCE(243);
      END_STATE();
    case 169:
      if (lookahead == 'i') ADVANCE(76);
      END_STATE();
    case 170:
      if (lookahead == 'i') ADVANCE(126);
      END_STATE();
    case 171:
      if (lookahead == 'k') ADVANCE(137);
      END_STATE();
    case 172:
      if (lookahead == 'l') ADVANCE(151);
      if (lookahead == 'o') ADVANCE(200);
      END_STATE();
    case 173:
      if (lookahead == 'l') ADVANCE(376);
      END_STATE();
    case 174:
      if (lookahead == 'l') ADVANCE(410);
      END_STATE();
    case 175:
      if (lookahead == 'l') ADVANCE(426);
      END_STATE();
    case 176:
      if (lookahead == 'l') ADVANCE(414);
      END_STATE();
    case 177:
      if (lookahead == 'l') ADVANCE(308);
      END_STATE();
    case 178:
      if (lookahead == 'l') ADVANCE(184);
      END_STATE();
    case 179:
      if (lookahead == 'l') ADVANCE(25);
      END_STATE();
    case 180:
      if (lookahead == 'l') ADVANCE(231);
      END_STATE();
    case 181:
      if (lookahead == 'l') ADVANCE(366);
      END_STATE();
    case 182:
      if (lookahead == 'l') ADVANCE(153);
      END_STATE();
    case 183:
      if (lookahead == 'l') ADVANCE(367);
      END_STATE();
    case 184:
      if (lookahead == 'l') ADVANCE(111);
      END_STATE();
    case 185:
      if (lookahead == 'l') ADVANCE(150);
      if (lookahead == 'n') ADVANCE(291);
      if (lookahead == 'o') ADVANCE(219);
      if (lookahead == 'q') ADVANCE(352);
      END_STATE();
    case 186:
      if (lookahead == 'l') ADVANCE(129);
      END_STATE();
    case 187:
      if (lookahead == 'l') ADVANCE(159);
      if (lookahead == 'o') ADVANCE(199);
      END_STATE();
    case 188:
      if (lookahead == 'l') ADVANCE(112);
      END_STATE();
    case 189:
      if (lookahead == 'l') ADVANCE(94);
      END_STATE();
    case 190:
      if (lookahead == 'l') ADVANCE(121);
      END_STATE();
    case 191:
      if (lookahead == 'l') ADVANCE(190);
      END_STATE();
    case 192:
      if (lookahead == 'l') ADVANCE(54);
      END_STATE();
    case 193:
      if (lookahead == 'm') ADVANCE(250);
      END_STATE();
    case 194:
      if (lookahead == 'm') ADVANCE(196);
      if (lookahead == 'n') ADVANCE(334);
      if (lookahead == 'o') ADVANCE(275);
      END_STATE();
    case 195:
      if (lookahead == 'm') ADVANCE(236);
      END_STATE();
    case 196:
      if (lookahead == 'm') ADVANCE(51);
      if (lookahead == 'p') ADVANCE(165);
      END_STATE();
    case 197:
      if (lookahead == 'm') ADVANCE(258);
      END_STATE();
    case 198:
      if (lookahead == 'm') ADVANCE(261);
      END_STATE();
    case 199:
      if (lookahead == 'm') ADVANCE(256);
      END_STATE();
    case 200:
      if (lookahead == 'm') ADVANCE(256);
      if (lookahead == 'n') ADVANCE(334);
      END_STATE();
    case 201:
      if (lookahead == 'm') ADVANCE(160);
      END_STATE();
    case 202:
      if (lookahead == 'm') ADVANCE(260);
      END_STATE();
    case 203:
      if (lookahead == 'm') ADVANCE(265);
      END_STATE();
    case 204:
      if (lookahead == 'm') ADVANCE(267);
      END_STATE();
    case 205:
      if (lookahead == 'n') ADVANCE(144);
      END_STATE();
    case 206:
      if (lookahead == 'n') ADVANCE(425);
      END_STATE();
    case 207:
      if (lookahead == 'n') ADVANCE(380);
      END_STATE();
    case 208:
      if (lookahead == 'n') ADVANCE(379);
      END_STATE();
    case 209:
      if (lookahead == 'n') ADVANCE(328);
      END_STATE();
    case 210:
      if (lookahead == 'n') ADVANCE(141);
      END_STATE();
    case 211:
      if (lookahead == 'n') ADVANCE(318);
      END_STATE();
    case 212:
      if (lookahead == 'n') ADVANCE(84);
      END_STATE();
    case 213:
      if (lookahead == 'n') ADVANCE(306);
      END_STATE();
    case 214:
      if (lookahead == 'n') ADVANCE(319);
      END_STATE();
    case 215:
      if (lookahead == 'n') ADVANCE(79);
      END_STATE();
    case 216:
      if (lookahead == 'n') ADVANCE(47);
      END_STATE();
    case 217:
      if (lookahead == 'n') ADVANCE(69);
      END_STATE();
    case 218:
      if (lookahead == 'n') ADVANCE(103);
      END_STATE();
    case 219:
      if (lookahead == 'n') ADVANCE(181);
      END_STATE();
    case 220:
      if (lookahead == 'n') ADVANCE(183);
      END_STATE();
    case 221:
      if (lookahead == 'n') ADVANCE(169);
      END_STATE();
    case 222:
      if (lookahead == 'n') ADVANCE(157);
      END_STATE();
    case 223:
      if (lookahead == 'n') ADVANCE(297);
      END_STATE();
    case 224:
      if (lookahead == 'n') ADVANCE(324);
      END_STATE();
    case 225:
      if (lookahead == 'n') ADVANCE(337);
      END_STATE();
    case 226:
      if (lookahead == 'n') ADVANCE(93);
      END_STATE();
    case 227:
      if (lookahead == 'n') ADVANCE(113);
      END_STATE();
    case 228:
      if (lookahead == 'n') ADVANCE(334);
      END_STATE();
    case 229:
      if (lookahead == 'n') ADVANCE(347);
      END_STATE();
    case 230:
      if (lookahead == 'o') ADVANCE(74);
      END_STATE();
    case 231:
      if (lookahead == 'o') ADVANCE(356);
      END_STATE();
    case 232:
      if (lookahead == 'o') ADVANCE(269);
      if (lookahead == 'r') ADVANCE(163);
      END_STATE();
    case 233:
      if (lookahead == 'o') ADVANCE(262);
      END_STATE();
    case 234:
      if (lookahead == 'o') ADVANCE(351);
      END_STATE();
    case 235:
      if (lookahead == 'o') ADVANCE(135);
      END_STATE();
    case 236:
      if (lookahead == 'o') ADVANCE(276);
      END_STATE();
    case 237:
      if (lookahead == 'o') ADVANCE(219);
      END_STATE();
    case 238:
      if (lookahead == 'o') ADVANCE(206);
      END_STATE();
    case 239:
      if (lookahead == 'o') ADVANCE(207);
      END_STATE();
    case 240:
      if (lookahead == 'o') ADVANCE(271);
      END_STATE();
    case 241:
      if (lookahead == 'o') ADVANCE(208);
      END_STATE();
    case 242:
      if (lookahead == 'o') ADVANCE(272);
      END_STATE();
    case 243:
      if (lookahead == 'o') ADVANCE(223);
      END_STATE();
    case 244:
      if (lookahead == 'o') ADVANCE(197);
      END_STATE();
    case 245:
      if (lookahead == 'o') ADVANCE(263);
      END_STATE();
    case 246:
      if (lookahead == 'o') ADVANCE(222);
      END_STATE();
    case 247:
      if (lookahead == 'o') ADVANCE(220);
      END_STATE();
    case 248:
      if (lookahead == 'o') ADVANCE(229);
      END_STATE();
    case 249:
      if (lookahead == 'p') ADVANCE(105);
      if (lookahead == 's') ADVANCE(72);
      END_STATE();
    case 250:
      if (lookahead == 'p') ADVANCE(36);
      END_STATE();
    case 251:
      if (lookahead == 'p') ADVANCE(406);
      END_STATE();
    case 252:
      if (lookahead == 'p') ADVANCE(421);
      END_STATE();
    case 253:
      if (lookahead == 'p') ADVANCE(361);
      END_STATE();
    case 254:
      if (lookahead == 'p') ADVANCE(290);
      END_STATE();
    case 255:
      if (lookahead == 'p') ADVANCE(45);
      if (lookahead == 'r') ADVANCE(117);
      END_STATE();
    case 256:
      if (lookahead == 'p') ADVANCE(165);
      END_STATE();
    case 257:
      if (lookahead == 'p') ADVANCE(253);
      END_STATE();
    case 258:
      if (lookahead == 'p') ADVANCE(320);
      END_STATE();
    case 259:
      if (lookahead == 'p') ADVANCE(52);
      END_STATE();
    case 260:
      if (lookahead == 'p') ADVANCE(331);
      END_STATE();
    case 261:
      if (lookahead == 'p') ADVANCE(35);
      END_STATE();
    case 262:
      if (lookahead == 'p') ADVANCE(92);
      END_STATE();
    case 263:
      if (lookahead == 'p') ADVANCE(97);
      END_STATE();
    case 264:
      if (lookahead == 'p') ADVANCE(104);
      if (lookahead == 's') ADVANCE(72);
      END_STATE();
    case 265:
      if (lookahead == 'p') ADVANCE(57);
      END_STATE();
    case 266:
      if (lookahead == 'p') ADVANCE(348);
      END_STATE();
    case 267:
      if (lookahead == 'p') ADVANCE(56);
      END_STATE();
    case 268:
      if (lookahead == 'r') ADVANCE(148);
      if (lookahead == 'u') ADVANCE(59);
      END_STATE();
    case 269:
      if (lookahead == 'r') ADVANCE(171);
      END_STATE();
    case 270:
      if (lookahead == 'r') ADVANCE(375);
      END_STATE();
    case 271:
      if (lookahead == 'r') ADVANCE(404);
      END_STATE();
    case 272:
      if (lookahead == 'r') ADVANCE(423);
      END_STATE();
    case 273:
      if (lookahead == 'r') ADVANCE(418);
      END_STATE();
    case 274:
      if (lookahead == 'r') ADVANCE(163);
      END_STATE();
    case 275:
      if (lookahead == 'r') ADVANCE(82);
      END_STATE();
    case 276:
      if (lookahead == 'r') ADVANCE(362);
      END_STATE();
    case 277:
      if (lookahead == 'r') ADVANCE(149);
      END_STATE();
    case 278:
      if (lookahead == 'r') ADVANCE(314);
      END_STATE();
    case 279:
      if (lookahead == 'r') ADVANCE(346);
      END_STATE();
    case 280:
      if (lookahead == 'r') ADVANCE(58);
      END_STATE();
    case 281:
      if (lookahead == 'r') ADVANCE(368);
      END_STATE();
    case 282:
      if (lookahead == 'r') ADVANCE(244);
      END_STATE();
    case 283:
      if (lookahead == 'r') ADVANCE(156);
      END_STATE();
    case 284:
      if (lookahead == 'r') ADVANCE(161);
      END_STATE();
    case 285:
      if (lookahead == 'r') ADVANCE(309);
      END_STATE();
    case 286:
      if (lookahead == 'r') ADVANCE(70);
      END_STATE();
    case 287:
      if (lookahead == 'r') ADVANCE(55);
      END_STATE();
    case 288:
      if (lookahead == 's') ADVANCE(71);
      END_STATE();
    case 289:
      if (lookahead == 's') ADVANCE(71);
      if (lookahead == 'x') ADVANCE(102);
      END_STATE();
    case 290:
      if (lookahead == 's') ADVANCE(413);
      END_STATE();
    case 291:
      if (lookahead == 's') ADVANCE(388);
      END_STATE();
    case 292:
      if (lookahead == 's') ADVANCE(417);
      END_STATE();
    case 293:
      if (lookahead == 's') ADVANCE(389);
      END_STATE();
    case 294:
      if (lookahead == 's') ADVANCE(382);
      END_STATE();
    case 295:
      if (lookahead == 's') ADVANCE(429);
      END_STATE();
    case 296:
      if (lookahead == 's') ADVANCE(401);
      END_STATE();
    case 297:
      if (lookahead == 's') ADVANCE(409);
      END_STATE();
    case 298:
      if (lookahead == 's') ADVANCE(391);
      END_STATE();
    case 299:
      if (lookahead == 's') ADVANCE(408);
      END_STATE();
    case 300:
      if (lookahead == 's') ADVANCE(317);
      END_STATE();
    case 301:
      if (lookahead == 's') ADVANCE(77);
      END_STATE();
    case 302:
      if (lookahead == 's') ADVANCE(77);
      if (lookahead == 't') ADVANCE(128);
      END_STATE();
    case 303:
      if (lookahead == 's') ADVANCE(234);
      END_STATE();
    case 304:
      if (lookahead == 's') ADVANCE(23);
      END_STATE();
    case 305:
      if (lookahead == 's') ADVANCE(304);
      END_STATE();
    case 306:
      if (lookahead == 's') ADVANCE(152);
      END_STATE();
    case 307:
      if (lookahead == 's') ADVANCE(295);
      END_STATE();
    case 308:
      if (lookahead == 's') ADVANCE(91);
      END_STATE();
    case 309:
      if (lookahead == 's') ADVANCE(323);
      END_STATE();
    case 310:
      if (lookahead == 's') ADVANCE(335);
      END_STATE();
    case 311:
      if (lookahead == 's') ADVANCE(325);
      END_STATE();
    case 312:
      if (lookahead == 's') ADVANCE(326);
      END_STATE();
    case 313:
      if (lookahead == 's') ADVANCE(327);
      END_STATE();
    case 314:
      if (lookahead == 's') ADVANCE(20);
      END_STATE();
    case 315:
      if (lookahead == 's') ADVANCE(26);
      END_STATE();
    case 316:
      if (lookahead == 't') ADVANCE(330);
      END_STATE();
    case 317:
      if (lookahead == 't') ADVANCE(400);
      END_STATE();
    case 318:
      if (lookahead == 't') ADVANCE(378);
      END_STATE();
    case 319:
      if (lookahead == 't') ADVANCE(372);
      END_STATE();
    case 320:
      if (lookahead == 't') ADVANCE(381);
      END_STATE();
    case 321:
      if (lookahead == 't') ADVANCE(402);
      END_STATE();
    case 322:
      if (lookahead == 't') ADVANCE(393);
      END_STATE();
    case 323:
      if (lookahead == 't') ADVANCE(416);
      END_STATE();
    case 324:
      if (lookahead == 't') ADVANCE(394);
      END_STATE();
    case 325:
      if (lookahead == 't') ADVANCE(399);
      END_STATE();
    case 326:
      if (lookahead == 't') ADVANCE(18);
      END_STATE();
    case 327:
      if (lookahead == 't') ADVANCE(398);
      END_STATE();
    case 328:
      if (lookahead == 't') ADVANCE(155);
      END_STATE();
    case 329:
      if (lookahead == 't') ADVANCE(16);
      END_STATE();
    case 330:
      if (lookahead == 't') ADVANCE(125);
      END_STATE();
    case 331:
      if (lookahead == 't') ADVANCE(292);
      END_STATE();
    case 332:
      if (lookahead == 't') ADVANCE(164);
      END_STATE();
    case 333:
      if (lookahead == 't') ADVANCE(28);
      END_STATE();
    case 334:
      if (lookahead == 't') ADVANCE(100);
      END_STATE();
    case 335:
      if (lookahead == 't') ADVANCE(296);
      END_STATE();
    case 336:
      if (lookahead == 't') ADVANCE(109);
      END_STATE();
    case 337:
      if (lookahead == 't') ADVANCE(299);
      END_STATE();
    case 338:
      if (lookahead == 't') ADVANCE(101);
      END_STATE();
    case 339:
      if (lookahead == 't') ADVANCE(115);
      END_STATE();
    case 340:
      if (lookahead == 't') ADVANCE(114);
      END_STATE();
    case 341:
      if (lookahead == 't') ADVANCE(128);
      END_STATE();
    case 342:
      if (lookahead == 't') ADVANCE(122);
      END_STATE();
    case 343:
      if (lookahead == 't') ADVANCE(24);
      END_STATE();
    case 344:
      if (lookahead == 't') ADVANCE(284);
      END_STATE();
    case 345:
      if (lookahead == 't') ADVANCE(242);
      END_STATE();
    case 346:
      if (lookahead == 't') ADVANCE(46);
      END_STATE();
    case 347:
      if (lookahead == 't') ADVANCE(131);
      END_STATE();
    case 348:
      if (lookahead == 't') ADVANCE(167);
      END_STATE();
    case 349:
      if (lookahead == 't') ADVANCE(168);
      END_STATE();
    case 350:
      if (lookahead == 'u') ADVANCE(90);
      END_STATE();
    case 351:
      if (lookahead == 'u') ADVANCE(286);
      END_STATE();
    case 352:
      if (lookahead == 'u') ADVANCE(119);
      END_STATE();
    case 353:
      if (lookahead == 'u') ADVANCE(332);
      END_STATE();
    case 354:
      if (lookahead == 'v') ADVANCE(38);
      END_STATE();
    case 355:
      if (lookahead == 'v') ADVANCE(95);
      END_STATE();
    case 356:
      if (lookahead == 'w') ADVANCE(412);
      END_STATE();
    case 357:
      if (lookahead == 'w') ADVANCE(218);
      END_STATE();
    case 358:
      if (lookahead == 'x') ADVANCE(13);
      END_STATE();
    case 359:
      if (lookahead == 'x') ADVANCE(321);
      END_STATE();
    case 360:
      if (lookahead == 'x') ADVANCE(17);
      END_STATE();
    case 361:
      if (lookahead == 'y') ADVANCE(397);
      END_STATE();
    case 362:
      if (lookahead == 'y') ADVANCE(387);
      END_STATE();
    case 363:
      if (lookahead == 'y') ADVANCE(395);
      END_STATE();
    case 364:
      if (lookahead == 'y') ADVANCE(377);
      END_STATE();
    case 365:
      if (lookahead == 'y') ADVANCE(415);
      END_STATE();
    case 366:
      if (lookahead == 'y') ADVANCE(385);
      END_STATE();
    case 367:
      if (lookahead == 'y') ADVANCE(428);
      END_STATE();
    case 368:
      if (lookahead == 'y') ADVANCE(392);
      END_STATE();
    case 369:
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(437);
      END_STATE();
    case 370:
      ACCEPT_TOKEN(ts_builtin_sym_end);
      END_STATE();
    case 371:
      ACCEPT_TOKEN(sym_comment);
      if (lookahead != 0 &&
          lookahead != '\n') ADVANCE(371);
      END_STATE();
    case 372:
      ACCEPT_TOKEN(anon_sym_client);
      END_STATE();
    case 373:
      ACCEPT_TOKEN(anon_sym_LBRACE);
      END_STATE();
    case 374:
      ACCEPT_TOKEN(anon_sym_RBRACE);
      END_STATE();
    case 375:
      ACCEPT_TOKEN(anon_sym_tier);
      END_STATE();
    case 376:
      ACCEPT_TOKEN(anon_sym_model);
      END_STATE();
    case 377:
      ACCEPT_TOKEN(anon_sym_privacy);
      END_STATE();
    case 378:
      ACCEPT_TOKEN(anon_sym_agent);
      END_STATE();
    case 379:
      ACCEPT_TOKEN(anon_sym_description);
      END_STATE();
    case 380:
      ACCEPT_TOKEN(anon_sym_depends_on);
      END_STATE();
    case 381:
      ACCEPT_TOKEN(anon_sym_prompt);
      END_STATE();
    case 382:
      ACCEPT_TOKEN(anon_sym_max_retries);
      END_STATE();
    case 383:
      ACCEPT_TOKEN(anon_sym_scope);
      END_STATE();
    case 384:
      ACCEPT_TOKEN(anon_sym_owned);
      END_STATE();
    case 385:
      ACCEPT_TOKEN(anon_sym_read_only);
      END_STATE();
    case 386:
      ACCEPT_TOKEN(anon_sym_impact_scope);
      END_STATE();
    case 387:
      ACCEPT_TOKEN(anon_sym_memory);
      END_STATE();
    case 388:
      ACCEPT_TOKEN(anon_sym_read_ns);
      END_STATE();
    case 389:
      ACCEPT_TOKEN(anon_sym_write_ns);
      END_STATE();
    case 390:
      ACCEPT_TOKEN(anon_sym_importance);
      END_STATE();
    case 391:
      ACCEPT_TOKEN(anon_sym_staleness_sources);
      END_STATE();
    case 392:
      ACCEPT_TOKEN(anon_sym_read_query);
      END_STATE();
    case 393:
      ACCEPT_TOKEN(anon_sym_read_limit);
      END_STATE();
    case 394:
      ACCEPT_TOKEN(anon_sym_write_content);
      END_STATE();
    case 395:
      ACCEPT_TOKEN(anon_sym_verify);
      END_STATE();
    case 396:
      ACCEPT_TOKEN(anon_sym_compile);
      END_STATE();
    case 397:
      ACCEPT_TOKEN(anon_sym_clippy);
      END_STATE();
    case 398:
      ACCEPT_TOKEN(anon_sym_test);
      END_STATE();
    case 399:
      ACCEPT_TOKEN(anon_sym_test);
      if (lookahead == '_') ADVANCE(138);
      END_STATE();
    case 400:
      ACCEPT_TOKEN(anon_sym_test);
      if (lookahead == '_') ADVANCE(138);
      if (lookahead == 's') ADVANCE(19);
      END_STATE();
    case 401:
      ACCEPT_TOKEN(anon_sym_impact_tests);
      END_STATE();
    case 402:
      ACCEPT_TOKEN(anon_sym_context);
      END_STATE();
    case 403:
      ACCEPT_TOKEN(anon_sym_callers_of);
      END_STATE();
    case 404:
      ACCEPT_TOKEN(anon_sym_tests_for);
      END_STATE();
    case 405:
      ACCEPT_TOKEN(anon_sym_depth);
      END_STATE();
    case 406:
      ACCEPT_TOKEN(anon_sym_loop);
      END_STATE();
    case 407:
      ACCEPT_TOKEN(anon_sym_loop);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 408:
      ACCEPT_TOKEN(anon_sym_agents);
      END_STATE();
    case 409:
      ACCEPT_TOKEN(anon_sym_max_iterations);
      END_STATE();
    case 410:
      ACCEPT_TOKEN(anon_sym_until);
      END_STATE();
    case 411:
      ACCEPT_TOKEN(anon_sym_command);
      END_STATE();
    case 412:
      ACCEPT_TOKEN(anon_sym_workflow);
      END_STATE();
    case 413:
      ACCEPT_TOKEN(anon_sym_steps);
      END_STATE();
    case 414:
      ACCEPT_TOKEN(anon_sym_max_parallel);
      END_STATE();
    case 415:
      ACCEPT_TOKEN(anon_sym_strategy);
      END_STATE();
    case 416:
      ACCEPT_TOKEN(anon_sym_test_first);
      END_STATE();
    case 417:
      ACCEPT_TOKEN(anon_sym_attempts);
      END_STATE();
    case 418:
      ACCEPT_TOKEN(anon_sym_escalate_after);
      END_STATE();
    case 419:
      ACCEPT_TOKEN(anon_sym_LBRACK);
      END_STATE();
    case 420:
      ACCEPT_TOKEN(anon_sym_RBRACK);
      END_STATE();
    case 421:
      ACCEPT_TOKEN(anon_sym_cheap);
      END_STATE();
    case 422:
      ACCEPT_TOKEN(anon_sym_expensive);
      END_STATE();
    case 423:
      ACCEPT_TOKEN(anon_sym_coordinator);
      END_STATE();
    case 424:
      ACCEPT_TOKEN(anon_sym_reasoning);
      END_STATE();
    case 425:
      ACCEPT_TOKEN(anon_sym_execution);
      END_STATE();
    case 426:
      ACCEPT_TOKEN(anon_sym_mechanical);
      END_STATE();
    case 427:
      ACCEPT_TOKEN(anon_sym_public);
      END_STATE();
    case 428:
      ACCEPT_TOKEN(anon_sym_local_only);
      END_STATE();
    case 429:
      ACCEPT_TOKEN(anon_sym_single_pass);
      END_STATE();
    case 430:
      ACCEPT_TOKEN(anon_sym_single_pass);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 431:
      ACCEPT_TOKEN(anon_sym_refine);
      END_STATE();
    case 432:
      ACCEPT_TOKEN(anon_sym_refine);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 433:
      ACCEPT_TOKEN(anon_sym_true);
      END_STATE();
    case 434:
      ACCEPT_TOKEN(anon_sym_false);
      END_STATE();
    case 435:
      ACCEPT_TOKEN(sym_string);
      END_STATE();
    case 436:
      ACCEPT_TOKEN(sym_raw_string);
      END_STATE();
    case 437:
      ACCEPT_TOKEN(sym_float);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(437);
      END_STATE();
    case 438:
      ACCEPT_TOKEN(sym_integer);
      if (lookahead == '.') ADVANCE(369);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(438);
      END_STATE();
    case 439:
      ACCEPT_TOKEN(sym_integer);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(439);
      END_STATE();
    case 440:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '_') ADVANCE(455);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 441:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(457);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 442:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(445);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 443:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(432);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 444:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(440);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 445:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'f') ADVANCE(448);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 446:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'g') ADVANCE(449);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 447:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(450);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 448:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(451);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 449:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(444);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 450:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(446);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 451:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(443);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 452:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(454);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 453:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(452);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 454:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(407);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 455:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(441);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 456:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(430);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 457:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(456);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    case 458:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(458);
      END_STATE();
    default:
      return false;
  }
}

static const TSLexMode ts_lex_modes[STATE_COUNT] = {
  [0] = {.lex_state = 0},
  [1] = {.lex_state = 0},
  [2] = {.lex_state = 9},
  [3] = {.lex_state = 8},
  [4] = {.lex_state = 8},
  [5] = {.lex_state = 8},
  [6] = {.lex_state = 0},
  [7] = {.lex_state = 8},
  [8] = {.lex_state = 0},
  [9] = {.lex_state = 8},
  [10] = {.lex_state = 0},
  [11] = {.lex_state = 8},
  [12] = {.lex_state = 8},
  [13] = {.lex_state = 9},
  [14] = {.lex_state = 9},
  [15] = {.lex_state = 0},
  [16] = {.lex_state = 8},
  [17] = {.lex_state = 8},
  [18] = {.lex_state = 8},
  [19] = {.lex_state = 8},
  [20] = {.lex_state = 0},
  [21] = {.lex_state = 0},
  [22] = {.lex_state = 8},
  [23] = {.lex_state = 8},
  [24] = {.lex_state = 8},
  [25] = {.lex_state = 0},
  [26] = {.lex_state = 0},
  [27] = {.lex_state = 0},
  [28] = {.lex_state = 0},
  [29] = {.lex_state = 0},
  [30] = {.lex_state = 0},
  [31] = {.lex_state = 0},
  [32] = {.lex_state = 0},
  [33] = {.lex_state = 0},
  [34] = {.lex_state = 10},
  [35] = {.lex_state = 10},
  [36] = {.lex_state = 10},
  [37] = {.lex_state = 10},
  [38] = {.lex_state = 9},
  [39] = {.lex_state = 0},
  [40] = {.lex_state = 0},
  [41] = {.lex_state = 9},
  [42] = {.lex_state = 10},
  [43] = {.lex_state = 9},
  [44] = {.lex_state = 0},
  [45] = {.lex_state = 0},
  [46] = {.lex_state = 8},
  [47] = {.lex_state = 0},
  [48] = {.lex_state = 0},
  [49] = {.lex_state = 0},
  [50] = {.lex_state = 0},
  [51] = {.lex_state = 8},
  [52] = {.lex_state = 8},
  [53] = {.lex_state = 6},
  [54] = {.lex_state = 10},
  [55] = {.lex_state = 6},
  [56] = {.lex_state = 0},
  [57] = {.lex_state = 0},
  [58] = {.lex_state = 0},
  [59] = {.lex_state = 6},
  [60] = {.lex_state = 0},
  [61] = {.lex_state = 11},
  [62] = {.lex_state = 0},
  [63] = {.lex_state = 0},
  [64] = {.lex_state = 0},
  [65] = {.lex_state = 0},
  [66] = {.lex_state = 9},
  [67] = {.lex_state = 8},
  [68] = {.lex_state = 0},
  [69] = {.lex_state = 0},
  [70] = {.lex_state = 9},
  [71] = {.lex_state = 0},
  [72] = {.lex_state = 9},
  [73] = {.lex_state = 0},
  [74] = {.lex_state = 9},
  [75] = {.lex_state = 0},
  [76] = {.lex_state = 9},
  [77] = {.lex_state = 9},
  [78] = {.lex_state = 9},
  [79] = {.lex_state = 0},
  [80] = {.lex_state = 7},
  [81] = {.lex_state = 7},
  [82] = {.lex_state = 0},
  [83] = {.lex_state = 0},
  [84] = {.lex_state = 0},
  [85] = {.lex_state = 0},
  [86] = {.lex_state = 6},
  [87] = {.lex_state = 0},
  [88] = {.lex_state = 0},
  [89] = {.lex_state = 0},
  [90] = {.lex_state = 6},
  [91] = {.lex_state = 7},
  [92] = {.lex_state = 0},
  [93] = {.lex_state = 0},
  [94] = {.lex_state = 0},
  [95] = {.lex_state = 0},
  [96] = {.lex_state = 0},
  [97] = {.lex_state = 0},
  [98] = {.lex_state = 7},
  [99] = {.lex_state = 0},
  [100] = {.lex_state = 9},
  [101] = {.lex_state = 9},
  [102] = {.lex_state = 7},
  [103] = {.lex_state = 9},
  [104] = {.lex_state = 0},
  [105] = {.lex_state = 9},
  [106] = {.lex_state = 7},
  [107] = {.lex_state = 0},
  [108] = {.lex_state = 7},
  [109] = {.lex_state = 0},
  [110] = {.lex_state = 8},
  [111] = {.lex_state = 7},
  [112] = {.lex_state = 9},
  [113] = {.lex_state = 0},
  [114] = {.lex_state = 0},
  [115] = {.lex_state = 0},
  [116] = {.lex_state = 0},
  [117] = {.lex_state = 0},
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
    [anon_sym_privacy] = ACTIONS(1),
    [anon_sym_agent] = ACTIONS(1),
    [anon_sym_description] = ACTIONS(1),
    [anon_sym_depends_on] = ACTIONS(1),
    [anon_sym_prompt] = ACTIONS(1),
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
    [anon_sym_cheap] = ACTIONS(1),
    [anon_sym_expensive] = ACTIONS(1),
    [anon_sym_coordinator] = ACTIONS(1),
    [anon_sym_reasoning] = ACTIONS(1),
    [anon_sym_execution] = ACTIONS(1),
    [anon_sym_mechanical] = ACTIONS(1),
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
    [sym_source_file] = STATE(99),
    [sym__definition] = STATE(32),
    [sym_client_declaration] = STATE(32),
    [sym_agent_declaration] = STATE(32),
    [sym_workflow_declaration] = STATE(32),
    [aux_sym_source_file_repeat1] = STATE(32),
    [ts_builtin_sym_end] = ACTIONS(5),
    [sym_comment] = ACTIONS(3),
    [anon_sym_client] = ACTIONS(7),
    [anon_sym_agent] = ACTIONS(9),
    [anon_sym_workflow] = ACTIONS(11),
  },
};

static const uint16_t ts_small_parse_table[] = {
  [0] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(15), 1,
      anon_sym_test,
    ACTIONS(13), 16,
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
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [25] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(17), 16,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
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
  [47] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(19), 16,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
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
  [69] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(21), 14,
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
  [89] = 11,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(23), 1,
      anon_sym_client,
    ACTIONS(25), 1,
      anon_sym_RBRACE,
    ACTIONS(29), 1,
      anon_sym_depends_on,
    ACTIONS(31), 1,
      anon_sym_max_retries,
    ACTIONS(33), 1,
      anon_sym_scope,
    ACTIONS(35), 1,
      anon_sym_memory,
    ACTIONS(37), 1,
      anon_sym_context,
    ACTIONS(27), 2,
      anon_sym_description,
      anon_sym_prompt,
    STATE(8), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(28), 3,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [127] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(35), 1,
      anon_sym_memory,
    ACTIONS(39), 1,
      anon_sym_RBRACE,
    ACTIONS(43), 1,
      anon_sym_verify,
    ACTIONS(45), 1,
      anon_sym_steps,
    ACTIONS(47), 1,
      anon_sym_strategy,
    ACTIONS(49), 1,
      anon_sym_test_first,
    STATE(9), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(16), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(41), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [163] = 11,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(23), 1,
      anon_sym_client,
    ACTIONS(29), 1,
      anon_sym_depends_on,
    ACTIONS(31), 1,
      anon_sym_max_retries,
    ACTIONS(33), 1,
      anon_sym_scope,
    ACTIONS(35), 1,
      anon_sym_memory,
    ACTIONS(37), 1,
      anon_sym_context,
    ACTIONS(51), 1,
      anon_sym_RBRACE,
    ACTIONS(27), 2,
      anon_sym_description,
      anon_sym_prompt,
    STATE(10), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(28), 3,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [201] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(35), 1,
      anon_sym_memory,
    ACTIONS(43), 1,
      anon_sym_verify,
    ACTIONS(45), 1,
      anon_sym_steps,
    ACTIONS(47), 1,
      anon_sym_strategy,
    ACTIONS(49), 1,
      anon_sym_test_first,
    ACTIONS(53), 1,
      anon_sym_RBRACE,
    STATE(11), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(16), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(41), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [237] = 11,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(55), 1,
      anon_sym_client,
    ACTIONS(58), 1,
      anon_sym_RBRACE,
    ACTIONS(63), 1,
      anon_sym_depends_on,
    ACTIONS(66), 1,
      anon_sym_max_retries,
    ACTIONS(69), 1,
      anon_sym_scope,
    ACTIONS(72), 1,
      anon_sym_memory,
    ACTIONS(75), 1,
      anon_sym_context,
    ACTIONS(60), 2,
      anon_sym_description,
      anon_sym_prompt,
    STATE(10), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(28), 3,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [275] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(78), 1,
      anon_sym_RBRACE,
    ACTIONS(83), 1,
      anon_sym_memory,
    ACTIONS(86), 1,
      anon_sym_verify,
    ACTIONS(89), 1,
      anon_sym_steps,
    ACTIONS(92), 1,
      anon_sym_strategy,
    ACTIONS(95), 1,
      anon_sym_test_first,
    STATE(11), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(16), 2,
      sym_memory_block,
      sym_verify_block,
    ACTIONS(80), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [311] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(98), 14,
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
  [331] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(100), 12,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_until,
  [349] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(102), 12,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_until,
  [367] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(104), 1,
      anon_sym_RBRACE,
    ACTIONS(110), 1,
      anon_sym_importance,
    ACTIONS(112), 1,
      anon_sym_read_limit,
    ACTIONS(106), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(20), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(108), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [393] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(114), 10,
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
  [409] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(116), 10,
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
  [425] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(118), 10,
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
  [441] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(120), 10,
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
  [457] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(110), 1,
      anon_sym_importance,
    ACTIONS(112), 1,
      anon_sym_read_limit,
    ACTIONS(122), 1,
      anon_sym_RBRACE,
    ACTIONS(106), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(21), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(108), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [483] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(124), 1,
      anon_sym_RBRACE,
    ACTIONS(132), 1,
      anon_sym_importance,
    ACTIONS(135), 1,
      anon_sym_read_limit,
    ACTIONS(126), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(21), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(129), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [509] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(138), 10,
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
  [525] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(140), 10,
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
  [541] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(142), 10,
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
  [557] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(144), 9,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [572] = 6,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(146), 1,
      ts_builtin_sym_end,
    ACTIONS(148), 1,
      anon_sym_client,
    ACTIONS(151), 1,
      anon_sym_agent,
    ACTIONS(154), 1,
      anon_sym_workflow,
    STATE(26), 5,
      sym__definition,
      sym_client_declaration,
      sym_agent_declaration,
      sym_workflow_declaration,
      aux_sym_source_file_repeat1,
  [595] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(157), 9,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [610] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(159), 9,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [625] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(161), 9,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [640] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(163), 9,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [655] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(165), 9,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_memory,
      anon_sym_context,
  [670] = 6,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(7), 1,
      anon_sym_client,
    ACTIONS(9), 1,
      anon_sym_agent,
    ACTIONS(11), 1,
      anon_sym_workflow,
    ACTIONS(167), 1,
      ts_builtin_sym_end,
    STATE(26), 5,
      sym__definition,
      sym_client_declaration,
      sym_agent_declaration,
      sym_workflow_declaration,
      aux_sym_source_file_repeat1,
  [693] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(169), 8,
      anon_sym_RBRACE,
      anon_sym_read_ns,
      anon_sym_write_ns,
      anon_sym_importance,
      anon_sym_staleness_sources,
      anon_sym_read_query,
      anon_sym_read_limit,
      anon_sym_write_content,
  [707] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(171), 1,
      anon_sym_RBRACE,
    STATE(37), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(173), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [724] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(175), 1,
      anon_sym_RBRACE,
    STATE(34), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(173), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [741] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(177), 1,
      anon_sym_RBRACE,
    STATE(37), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(173), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [758] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(179), 1,
      anon_sym_RBRACE,
    STATE(37), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(181), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [775] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(184), 1,
      anon_sym_RBRACE,
    ACTIONS(186), 1,
      anon_sym_agents,
    ACTIONS(188), 1,
      anon_sym_max_iterations,
    ACTIONS(190), 1,
      anon_sym_until,
    STATE(66), 1,
      sym_until_clause,
    STATE(41), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
  [798] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(65), 1,
      sym_tier_value,
    ACTIONS(192), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [813] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(194), 1,
      anon_sym_LBRACE,
    ACTIONS(196), 1,
      anon_sym_agent,
    ACTIONS(198), 1,
      anon_sym_command,
    STATE(72), 4,
      sym__until_condition,
      sym_until_verify,
      sym_until_agent,
      sym_until_command,
  [832] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(186), 1,
      anon_sym_agents,
    ACTIONS(188), 1,
      anon_sym_max_iterations,
    ACTIONS(190), 1,
      anon_sym_until,
    ACTIONS(200), 1,
      anon_sym_RBRACE,
    STATE(66), 1,
      sym_until_clause,
    STATE(43), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
  [855] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(202), 1,
      anon_sym_RBRACE,
    STATE(36), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(173), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [872] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(204), 1,
      anon_sym_RBRACE,
    ACTIONS(206), 1,
      anon_sym_agents,
    ACTIONS(209), 1,
      anon_sym_max_iterations,
    ACTIONS(212), 1,
      anon_sym_until,
    STATE(66), 1,
      sym_until_clause,
    STATE(43), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
  [895] = 6,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(215), 1,
      anon_sym_RBRACE,
    ACTIONS(217), 1,
      anon_sym_tier,
    ACTIONS(219), 1,
      anon_sym_model,
    ACTIONS(221), 1,
      anon_sym_privacy,
    STATE(45), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [915] = 6,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(223), 1,
      anon_sym_RBRACE,
    ACTIONS(225), 1,
      anon_sym_tier,
    ACTIONS(228), 1,
      anon_sym_model,
    ACTIONS(231), 1,
      anon_sym_privacy,
    STATE(45), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [935] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(234), 1,
      anon_sym_RBRACE,
    ACTIONS(239), 1,
      anon_sym_depth,
    ACTIONS(236), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(46), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [953] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(242), 1,
      anon_sym_RBRACE,
    ACTIONS(247), 1,
      anon_sym_impact_scope,
    ACTIONS(244), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(47), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [971] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(250), 1,
      anon_sym_RBRACE,
    ACTIONS(254), 1,
      anon_sym_impact_scope,
    ACTIONS(252), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(47), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [989] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(254), 1,
      anon_sym_impact_scope,
    ACTIONS(256), 1,
      anon_sym_RBRACE,
    ACTIONS(252), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(48), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [1007] = 6,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(217), 1,
      anon_sym_tier,
    ACTIONS(219), 1,
      anon_sym_model,
    ACTIONS(221), 1,
      anon_sym_privacy,
    ACTIONS(258), 1,
      anon_sym_RBRACE,
    STATE(44), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [1027] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(260), 1,
      anon_sym_RBRACE,
    ACTIONS(264), 1,
      anon_sym_depth,
    ACTIONS(262), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(46), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1045] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(264), 1,
      anon_sym_depth,
    ACTIONS(266), 1,
      anon_sym_RBRACE,
    ACTIONS(262), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(51), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [1063] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(268), 1,
      anon_sym_loop,
    ACTIONS(270), 1,
      anon_sym_RBRACK,
    ACTIONS(272), 1,
      sym_identifier,
    STATE(59), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [1080] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(274), 5,
      anon_sym_RBRACE,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1091] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(276), 1,
      anon_sym_loop,
    ACTIONS(279), 1,
      anon_sym_RBRACK,
    ACTIONS(281), 1,
      sym_identifier,
    STATE(55), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [1108] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(284), 1,
      anon_sym_RBRACK,
    ACTIONS(286), 2,
      sym_string,
      sym_raw_string,
    STATE(57), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [1123] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(288), 1,
      anon_sym_RBRACK,
    ACTIONS(290), 2,
      sym_string,
      sym_raw_string,
    STATE(58), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [1138] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(292), 1,
      anon_sym_RBRACK,
    ACTIONS(294), 2,
      sym_string,
      sym_raw_string,
    STATE(58), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [1153] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(268), 1,
      anon_sym_loop,
    ACTIONS(297), 1,
      anon_sym_RBRACK,
    ACTIONS(299), 1,
      sym_identifier,
    STATE(55), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [1170] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(301), 4,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_agent,
      anon_sym_workflow,
  [1180] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(17), 1,
      sym_strategy_value,
    ACTIONS(303), 3,
      anon_sym_single_pass,
      anon_sym_refine,
      sym_identifier,
  [1192] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(305), 4,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_agent,
      anon_sym_workflow,
  [1202] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(307), 4,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_agent,
      anon_sym_workflow,
  [1212] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(309), 4,
      anon_sym_RBRACE,
      anon_sym_owned,
      anon_sym_read_only,
      anon_sym_impact_scope,
  [1222] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(311), 4,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_privacy,
  [1232] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(313), 4,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_until,
  [1242] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(315), 4,
      anon_sym_RBRACE,
      anon_sym_callers_of,
      anon_sym_tests_for,
      anon_sym_depth,
  [1252] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(317), 4,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_privacy,
  [1262] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(319), 4,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_agent,
      anon_sym_workflow,
  [1272] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(321), 4,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_until,
  [1282] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(323), 4,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_agent,
      anon_sym_workflow,
  [1292] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(325), 4,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_until,
  [1302] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(327), 4,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_agent,
      anon_sym_workflow,
  [1312] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(329), 4,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_until,
  [1322] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(331), 4,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_privacy,
  [1332] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(333), 4,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_until,
  [1342] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(335), 4,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_until,
  [1352] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(337), 4,
      anon_sym_RBRACE,
      anon_sym_agents,
      anon_sym_max_iterations,
      anon_sym_until,
  [1362] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(64), 1,
      sym_boolean,
    ACTIONS(339), 2,
      anon_sym_true,
      anon_sym_false,
  [1373] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(341), 1,
      anon_sym_RBRACK,
    ACTIONS(343), 1,
      sym_identifier,
    STATE(80), 1,
      aux_sym_identifier_list_repeat1,
  [1386] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(346), 1,
      anon_sym_RBRACK,
    ACTIONS(348), 1,
      sym_identifier,
    STATE(80), 1,
      aux_sym_identifier_list_repeat1,
  [1399] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(17), 1,
      sym_boolean,
    ACTIONS(339), 2,
      anon_sym_true,
      anon_sym_false,
  [1410] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(65), 1,
      sym_privacy_value,
    ACTIONS(350), 2,
      anon_sym_public,
      anon_sym_local_only,
  [1421] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(65), 1,
      sym__string_value,
    ACTIONS(352), 2,
      sym_string,
      sym_raw_string,
  [1432] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(33), 1,
      sym__string_value,
    ACTIONS(354), 2,
      sym_string,
      sym_raw_string,
  [1443] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(358), 1,
      anon_sym_RBRACK,
    ACTIONS(356), 2,
      anon_sym_loop,
      sym_identifier,
  [1454] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(54), 1,
      sym_boolean,
    ACTIONS(339), 2,
      anon_sym_true,
      anon_sym_false,
  [1465] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(27), 1,
      sym__string_value,
    ACTIONS(360), 2,
      sym_string,
      sym_raw_string,
  [1476] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(77), 1,
      sym__string_value,
    ACTIONS(362), 2,
      sym_string,
      sym_raw_string,
  [1487] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(366), 1,
      anon_sym_RBRACK,
    ACTIONS(364), 2,
      anon_sym_loop,
      sym_identifier,
  [1498] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(368), 1,
      anon_sym_RBRACK,
    ACTIONS(370), 1,
      sym_identifier,
    STATE(81), 1,
      aux_sym_identifier_list_repeat1,
  [1511] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(372), 1,
      anon_sym_LBRACK,
    STATE(17), 1,
      sym_step_list,
  [1521] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(374), 1,
      anon_sym_LBRACK,
    STATE(64), 1,
      sym_string_list,
  [1531] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(374), 1,
      anon_sym_LBRACK,
    STATE(33), 1,
      sym_string_list,
  [1541] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(374), 1,
      anon_sym_LBRACK,
    STATE(67), 1,
      sym_string_list,
  [1551] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(376), 1,
      anon_sym_LBRACK,
    STATE(70), 1,
      sym_identifier_list,
  [1561] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(376), 1,
      anon_sym_LBRACK,
    STATE(27), 1,
      sym_identifier_list,
  [1571] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(378), 1,
      sym_identifier,
  [1578] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(380), 1,
      ts_builtin_sym_end,
  [1585] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(382), 1,
      sym_integer,
  [1592] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(354), 1,
      sym_integer,
  [1599] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(384), 1,
      sym_identifier,
  [1606] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(386), 1,
      sym_integer,
  [1613] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(388), 1,
      anon_sym_LBRACE,
  [1620] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(390), 1,
      sym_integer,
  [1627] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(360), 1,
      sym_identifier,
  [1634] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(392), 1,
      anon_sym_LBRACE,
  [1641] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(394), 1,
      sym_identifier,
  [1648] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(396), 1,
      anon_sym_LBRACE,
  [1655] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(354), 1,
      sym_float,
  [1662] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(398), 1,
      sym_identifier,
  [1669] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(360), 1,
      sym_integer,
  [1676] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(400), 1,
      anon_sym_LBRACE,
  [1683] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(402), 1,
      anon_sym_LBRACE,
  [1690] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(404), 1,
      anon_sym_LBRACE,
  [1697] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(406), 1,
      anon_sym_LBRACE,
  [1704] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(408), 1,
      anon_sym_LBRACE,
};

static const uint32_t ts_small_parse_table_map[] = {
  [SMALL_STATE(2)] = 0,
  [SMALL_STATE(3)] = 25,
  [SMALL_STATE(4)] = 47,
  [SMALL_STATE(5)] = 69,
  [SMALL_STATE(6)] = 89,
  [SMALL_STATE(7)] = 127,
  [SMALL_STATE(8)] = 163,
  [SMALL_STATE(9)] = 201,
  [SMALL_STATE(10)] = 237,
  [SMALL_STATE(11)] = 275,
  [SMALL_STATE(12)] = 311,
  [SMALL_STATE(13)] = 331,
  [SMALL_STATE(14)] = 349,
  [SMALL_STATE(15)] = 367,
  [SMALL_STATE(16)] = 393,
  [SMALL_STATE(17)] = 409,
  [SMALL_STATE(18)] = 425,
  [SMALL_STATE(19)] = 441,
  [SMALL_STATE(20)] = 457,
  [SMALL_STATE(21)] = 483,
  [SMALL_STATE(22)] = 509,
  [SMALL_STATE(23)] = 525,
  [SMALL_STATE(24)] = 541,
  [SMALL_STATE(25)] = 557,
  [SMALL_STATE(26)] = 572,
  [SMALL_STATE(27)] = 595,
  [SMALL_STATE(28)] = 610,
  [SMALL_STATE(29)] = 625,
  [SMALL_STATE(30)] = 640,
  [SMALL_STATE(31)] = 655,
  [SMALL_STATE(32)] = 670,
  [SMALL_STATE(33)] = 693,
  [SMALL_STATE(34)] = 707,
  [SMALL_STATE(35)] = 724,
  [SMALL_STATE(36)] = 741,
  [SMALL_STATE(37)] = 758,
  [SMALL_STATE(38)] = 775,
  [SMALL_STATE(39)] = 798,
  [SMALL_STATE(40)] = 813,
  [SMALL_STATE(41)] = 832,
  [SMALL_STATE(42)] = 855,
  [SMALL_STATE(43)] = 872,
  [SMALL_STATE(44)] = 895,
  [SMALL_STATE(45)] = 915,
  [SMALL_STATE(46)] = 935,
  [SMALL_STATE(47)] = 953,
  [SMALL_STATE(48)] = 971,
  [SMALL_STATE(49)] = 989,
  [SMALL_STATE(50)] = 1007,
  [SMALL_STATE(51)] = 1027,
  [SMALL_STATE(52)] = 1045,
  [SMALL_STATE(53)] = 1063,
  [SMALL_STATE(54)] = 1080,
  [SMALL_STATE(55)] = 1091,
  [SMALL_STATE(56)] = 1108,
  [SMALL_STATE(57)] = 1123,
  [SMALL_STATE(58)] = 1138,
  [SMALL_STATE(59)] = 1153,
  [SMALL_STATE(60)] = 1170,
  [SMALL_STATE(61)] = 1180,
  [SMALL_STATE(62)] = 1192,
  [SMALL_STATE(63)] = 1202,
  [SMALL_STATE(64)] = 1212,
  [SMALL_STATE(65)] = 1222,
  [SMALL_STATE(66)] = 1232,
  [SMALL_STATE(67)] = 1242,
  [SMALL_STATE(68)] = 1252,
  [SMALL_STATE(69)] = 1262,
  [SMALL_STATE(70)] = 1272,
  [SMALL_STATE(71)] = 1282,
  [SMALL_STATE(72)] = 1292,
  [SMALL_STATE(73)] = 1302,
  [SMALL_STATE(74)] = 1312,
  [SMALL_STATE(75)] = 1322,
  [SMALL_STATE(76)] = 1332,
  [SMALL_STATE(77)] = 1342,
  [SMALL_STATE(78)] = 1352,
  [SMALL_STATE(79)] = 1362,
  [SMALL_STATE(80)] = 1373,
  [SMALL_STATE(81)] = 1386,
  [SMALL_STATE(82)] = 1399,
  [SMALL_STATE(83)] = 1410,
  [SMALL_STATE(84)] = 1421,
  [SMALL_STATE(85)] = 1432,
  [SMALL_STATE(86)] = 1443,
  [SMALL_STATE(87)] = 1454,
  [SMALL_STATE(88)] = 1465,
  [SMALL_STATE(89)] = 1476,
  [SMALL_STATE(90)] = 1487,
  [SMALL_STATE(91)] = 1498,
  [SMALL_STATE(92)] = 1511,
  [SMALL_STATE(93)] = 1521,
  [SMALL_STATE(94)] = 1531,
  [SMALL_STATE(95)] = 1541,
  [SMALL_STATE(96)] = 1551,
  [SMALL_STATE(97)] = 1561,
  [SMALL_STATE(98)] = 1571,
  [SMALL_STATE(99)] = 1578,
  [SMALL_STATE(100)] = 1585,
  [SMALL_STATE(101)] = 1592,
  [SMALL_STATE(102)] = 1599,
  [SMALL_STATE(103)] = 1606,
  [SMALL_STATE(104)] = 1613,
  [SMALL_STATE(105)] = 1620,
  [SMALL_STATE(106)] = 1627,
  [SMALL_STATE(107)] = 1634,
  [SMALL_STATE(108)] = 1641,
  [SMALL_STATE(109)] = 1648,
  [SMALL_STATE(110)] = 1655,
  [SMALL_STATE(111)] = 1662,
  [SMALL_STATE(112)] = 1669,
  [SMALL_STATE(113)] = 1676,
  [SMALL_STATE(114)] = 1683,
  [SMALL_STATE(115)] = 1690,
  [SMALL_STATE(116)] = 1697,
  [SMALL_STATE(117)] = 1704,
};

static const TSParseActionEntry ts_parse_actions[] = {
  [0] = {.entry = {.count = 0, .reusable = false}},
  [1] = {.entry = {.count = 1, .reusable = false}}, RECOVER(),
  [3] = {.entry = {.count = 1, .reusable = true}}, SHIFT_EXTRA(),
  [5] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 0, 0, 0),
  [7] = {.entry = {.count = 1, .reusable = true}}, SHIFT(111),
  [9] = {.entry = {.count = 1, .reusable = true}}, SHIFT(102),
  [11] = {.entry = {.count = 1, .reusable = true}}, SHIFT(108),
  [13] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_boolean, 1, 0, 0),
  [15] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_boolean, 1, 0, 0),
  [17] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_block, 3, 0, 0),
  [19] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_block, 4, 0, 0),
  [21] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 3, 0, 0),
  [23] = {.entry = {.count = 1, .reusable = true}}, SHIFT(106),
  [25] = {.entry = {.count = 1, .reusable = true}}, SHIFT(73),
  [27] = {.entry = {.count = 1, .reusable = true}}, SHIFT(88),
  [29] = {.entry = {.count = 1, .reusable = true}}, SHIFT(97),
  [31] = {.entry = {.count = 1, .reusable = true}}, SHIFT(112),
  [33] = {.entry = {.count = 1, .reusable = true}}, SHIFT(114),
  [35] = {.entry = {.count = 1, .reusable = true}}, SHIFT(115),
  [37] = {.entry = {.count = 1, .reusable = true}}, SHIFT(116),
  [39] = {.entry = {.count = 1, .reusable = true}}, SHIFT(62),
  [41] = {.entry = {.count = 1, .reusable = true}}, SHIFT(103),
  [43] = {.entry = {.count = 1, .reusable = true}}, SHIFT(109),
  [45] = {.entry = {.count = 1, .reusable = true}}, SHIFT(92),
  [47] = {.entry = {.count = 1, .reusable = true}}, SHIFT(61),
  [49] = {.entry = {.count = 1, .reusable = true}}, SHIFT(82),
  [51] = {.entry = {.count = 1, .reusable = true}}, SHIFT(63),
  [53] = {.entry = {.count = 1, .reusable = true}}, SHIFT(71),
  [55] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(106),
  [58] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0),
  [60] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(88),
  [63] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(97),
  [66] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(112),
  [69] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(114),
  [72] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(115),
  [75] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(116),
  [78] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0),
  [80] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(103),
  [83] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(115),
  [86] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(109),
  [89] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(92),
  [92] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(61),
  [95] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(82),
  [98] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 2, 0, 0),
  [100] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 3, 0, 0),
  [102] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 2, 0, 0),
  [104] = {.entry = {.count = 1, .reusable = true}}, SHIFT(3),
  [106] = {.entry = {.count = 1, .reusable = true}}, SHIFT(94),
  [108] = {.entry = {.count = 1, .reusable = true}}, SHIFT(85),
  [110] = {.entry = {.count = 1, .reusable = true}}, SHIFT(110),
  [112] = {.entry = {.count = 1, .reusable = true}}, SHIFT(101),
  [114] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 1, 0, 0),
  [116] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 2, 0, 0),
  [118] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_strategy_value, 1, 0, 0),
  [120] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_block, 3, 0, 0),
  [122] = {.entry = {.count = 1, .reusable = true}}, SHIFT(4),
  [124] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0),
  [126] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(94),
  [129] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(85),
  [132] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(110),
  [135] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(101),
  [138] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_block, 4, 0, 0),
  [140] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_step_list, 2, 0, 0),
  [142] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_step_list, 3, 0, 0),
  [144] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_block, 3, 0, 0),
  [146] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0),
  [148] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(111),
  [151] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(102),
  [154] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(108),
  [157] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 2, 0, 0),
  [159] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 1, 0, 0),
  [161] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 4, 0, 0),
  [163] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 3, 0, 0),
  [165] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_block, 4, 0, 0),
  [167] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 1, 0, 0),
  [169] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_field, 2, 0, 0),
  [171] = {.entry = {.count = 1, .reusable = true}}, SHIFT(22),
  [173] = {.entry = {.count = 1, .reusable = true}}, SHIFT(87),
  [175] = {.entry = {.count = 1, .reusable = true}}, SHIFT(19),
  [177] = {.entry = {.count = 1, .reusable = true}}, SHIFT(78),
  [179] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_verify_block_repeat1, 2, 0, 0),
  [181] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_verify_block_repeat1, 2, 0, 0), SHIFT_REPEAT(87),
  [184] = {.entry = {.count = 1, .reusable = true}}, SHIFT(86),
  [186] = {.entry = {.count = 1, .reusable = true}}, SHIFT(96),
  [188] = {.entry = {.count = 1, .reusable = true}}, SHIFT(100),
  [190] = {.entry = {.count = 1, .reusable = true}}, SHIFT(40),
  [192] = {.entry = {.count = 1, .reusable = true}}, SHIFT(75),
  [194] = {.entry = {.count = 1, .reusable = true}}, SHIFT(42),
  [196] = {.entry = {.count = 1, .reusable = true}}, SHIFT(98),
  [198] = {.entry = {.count = 1, .reusable = true}}, SHIFT(89),
  [200] = {.entry = {.count = 1, .reusable = true}}, SHIFT(90),
  [202] = {.entry = {.count = 1, .reusable = true}}, SHIFT(74),
  [204] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0),
  [206] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(96),
  [209] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(100),
  [212] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(40),
  [215] = {.entry = {.count = 1, .reusable = true}}, SHIFT(60),
  [217] = {.entry = {.count = 1, .reusable = true}}, SHIFT(39),
  [219] = {.entry = {.count = 1, .reusable = true}}, SHIFT(84),
  [221] = {.entry = {.count = 1, .reusable = true}}, SHIFT(83),
  [223] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0),
  [225] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(39),
  [228] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(84),
  [231] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(83),
  [234] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0),
  [236] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0), SHIFT_REPEAT(95),
  [239] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0), SHIFT_REPEAT(105),
  [242] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0),
  [244] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(93),
  [247] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(79),
  [250] = {.entry = {.count = 1, .reusable = true}}, SHIFT(29),
  [252] = {.entry = {.count = 1, .reusable = true}}, SHIFT(93),
  [254] = {.entry = {.count = 1, .reusable = true}}, SHIFT(79),
  [256] = {.entry = {.count = 1, .reusable = true}}, SHIFT(30),
  [258] = {.entry = {.count = 1, .reusable = true}}, SHIFT(69),
  [260] = {.entry = {.count = 1, .reusable = true}}, SHIFT(31),
  [262] = {.entry = {.count = 1, .reusable = true}}, SHIFT(95),
  [264] = {.entry = {.count = 1, .reusable = true}}, SHIFT(105),
  [266] = {.entry = {.count = 1, .reusable = true}}, SHIFT(25),
  [268] = {.entry = {.count = 1, .reusable = false}}, SHIFT(113),
  [270] = {.entry = {.count = 1, .reusable = true}}, SHIFT(23),
  [272] = {.entry = {.count = 1, .reusable = false}}, SHIFT(59),
  [274] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_field, 2, 0, 0),
  [276] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0), SHIFT_REPEAT(113),
  [279] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0),
  [281] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0), SHIFT_REPEAT(55),
  [284] = {.entry = {.count = 1, .reusable = true}}, SHIFT(12),
  [286] = {.entry = {.count = 1, .reusable = true}}, SHIFT(57),
  [288] = {.entry = {.count = 1, .reusable = true}}, SHIFT(5),
  [290] = {.entry = {.count = 1, .reusable = true}}, SHIFT(58),
  [292] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0),
  [294] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0), SHIFT_REPEAT(58),
  [297] = {.entry = {.count = 1, .reusable = true}}, SHIFT(24),
  [299] = {.entry = {.count = 1, .reusable = false}}, SHIFT(55),
  [301] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 5, 0, 0),
  [303] = {.entry = {.count = 1, .reusable = false}}, SHIFT(18),
  [305] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 4, 0, 0),
  [307] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 5, 0, 0),
  [309] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_field, 2, 0, 0),
  [311] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_field, 2, 0, 0),
  [313] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_field, 1, 0, 0),
  [315] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_field, 2, 0, 0),
  [317] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_privacy_value, 1, 0, 0),
  [319] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 4, 0, 0),
  [321] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_field, 2, 0, 0),
  [323] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 5, 0, 0),
  [325] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_clause, 2, 0, 0),
  [327] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 4, 0, 0),
  [329] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_verify, 2, 0, 0),
  [331] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_value, 1, 0, 0),
  [333] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_agent, 2, 0, 0),
  [335] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_command, 2, 0, 0),
  [337] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_verify, 3, 0, 0),
  [339] = {.entry = {.count = 1, .reusable = true}}, SHIFT(2),
  [341] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0),
  [343] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0), SHIFT_REPEAT(80),
  [346] = {.entry = {.count = 1, .reusable = true}}, SHIFT(13),
  [348] = {.entry = {.count = 1, .reusable = true}}, SHIFT(80),
  [350] = {.entry = {.count = 1, .reusable = true}}, SHIFT(68),
  [352] = {.entry = {.count = 1, .reusable = true}}, SHIFT(65),
  [354] = {.entry = {.count = 1, .reusable = true}}, SHIFT(33),
  [356] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_loop_block, 3, 0, 0),
  [358] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_block, 3, 0, 0),
  [360] = {.entry = {.count = 1, .reusable = true}}, SHIFT(27),
  [362] = {.entry = {.count = 1, .reusable = true}}, SHIFT(77),
  [364] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_loop_block, 4, 0, 0),
  [366] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_block, 4, 0, 0),
  [368] = {.entry = {.count = 1, .reusable = true}}, SHIFT(14),
  [370] = {.entry = {.count = 1, .reusable = true}}, SHIFT(81),
  [372] = {.entry = {.count = 1, .reusable = true}}, SHIFT(53),
  [374] = {.entry = {.count = 1, .reusable = true}}, SHIFT(56),
  [376] = {.entry = {.count = 1, .reusable = true}}, SHIFT(91),
  [378] = {.entry = {.count = 1, .reusable = true}}, SHIFT(76),
  [380] = {.entry = {.count = 1, .reusable = true}},  ACCEPT_INPUT(),
  [382] = {.entry = {.count = 1, .reusable = true}}, SHIFT(70),
  [384] = {.entry = {.count = 1, .reusable = true}}, SHIFT(107),
  [386] = {.entry = {.count = 1, .reusable = true}}, SHIFT(17),
  [388] = {.entry = {.count = 1, .reusable = true}}, SHIFT(7),
  [390] = {.entry = {.count = 1, .reusable = true}}, SHIFT(67),
  [392] = {.entry = {.count = 1, .reusable = true}}, SHIFT(6),
  [394] = {.entry = {.count = 1, .reusable = true}}, SHIFT(104),
  [396] = {.entry = {.count = 1, .reusable = true}}, SHIFT(35),
  [398] = {.entry = {.count = 1, .reusable = true}}, SHIFT(117),
  [400] = {.entry = {.count = 1, .reusable = true}}, SHIFT(38),
  [402] = {.entry = {.count = 1, .reusable = true}}, SHIFT(49),
  [404] = {.entry = {.count = 1, .reusable = true}}, SHIFT(15),
  [406] = {.entry = {.count = 1, .reusable = true}}, SHIFT(52),
  [408] = {.entry = {.count = 1, .reusable = true}}, SHIFT(50),
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
