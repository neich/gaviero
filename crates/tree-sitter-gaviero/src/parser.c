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
#define STATE_COUNT 179
#define LARGE_STATE_COUNT 2
#define SYMBOL_COUNT 155
#define ALIAS_COUNT 0
#define TOKEN_COUNT 91
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
  anon_sym_execution_mode = 72,
  anon_sym_strategy = 73,
  anon_sym_test_first = 74,
  anon_sym_attempts = 75,
  anon_sym_escalate_after = 76,
  anon_sym_param = 77,
  anon_sym_public = 78,
  anon_sym_local_only = 79,
  anon_sym_single_pass = 80,
  anon_sym_refine = 81,
  anon_sym_repo = 82,
  anon_sym_document = 83,
  anon_sym_true = 84,
  anon_sym_false = 85,
  sym_string = 86,
  sym_raw_string = 87,
  sym_float = 88,
  sym_integer = 89,
  sym_identifier = 90,
  sym_source_file = 91,
  sym__definition = 92,
  sym_include_declaration = 93,
  sym_client_declaration = 94,
  sym_client_field = 95,
  sym__effort_value = 96,
  sym_extra_block = 97,
  sym_extra_pair = 98,
  sym_vars_block = 99,
  sym_vars_pair = 100,
  sym_tier_alias_declaration = 101,
  sym_tier_alias_name = 102,
  sym_prompt_declaration = 103,
  sym_agent_declaration = 104,
  sym_agent_field = 105,
  sym_scope_block = 106,
  sym_scope_field = 107,
  sym_memory_block = 108,
  sym_memory_field = 109,
  sym_verify_block = 110,
  sym_verify_field = 111,
  sym_context_block = 112,
  sym_context_field = 113,
  sym_loop_block = 114,
  sym_loop_field = 115,
  sym_reviewer_list = 116,
  sym_reviewer_entry = 117,
  sym_reviewer_field = 118,
  sym_consensus_mode_value = 119,
  sym_branch_chain_value = 120,
  sym_until_clause = 121,
  sym__until_condition = 122,
  sym_until_verify = 123,
  sym_until_agent = 124,
  sym_until_command = 125,
  sym_workflow_declaration = 126,
  sym_workflow_field = 127,
  sym_param_declaration = 128,
  sym_param_client_block = 129,
  sym_step_list = 130,
  sym_string_list = 131,
  sym_identifier_list = 132,
  sym_tier_value = 133,
  sym_privacy_value = 134,
  sym_strategy_value = 135,
  sym_execution_mode_value = 136,
  sym_boolean = 137,
  sym__string_value = 138,
  aux_sym_source_file_repeat1 = 139,
  aux_sym_client_declaration_repeat1 = 140,
  aux_sym_extra_block_repeat1 = 141,
  aux_sym_vars_block_repeat1 = 142,
  aux_sym_agent_declaration_repeat1 = 143,
  aux_sym_scope_block_repeat1 = 144,
  aux_sym_memory_block_repeat1 = 145,
  aux_sym_verify_block_repeat1 = 146,
  aux_sym_context_block_repeat1 = 147,
  aux_sym_loop_block_repeat1 = 148,
  aux_sym_reviewer_list_repeat1 = 149,
  aux_sym_reviewer_entry_repeat1 = 150,
  aux_sym_workflow_declaration_repeat1 = 151,
  aux_sym_step_list_repeat1 = 152,
  aux_sym_string_list_repeat1 = 153,
  aux_sym_identifier_list_repeat1 = 154,
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
  [anon_sym_execution_mode] = "execution_mode",
  [anon_sym_strategy] = "strategy",
  [anon_sym_test_first] = "test_first",
  [anon_sym_attempts] = "attempts",
  [anon_sym_escalate_after] = "escalate_after",
  [anon_sym_param] = "param",
  [anon_sym_public] = "public",
  [anon_sym_local_only] = "local_only",
  [anon_sym_single_pass] = "single_pass",
  [anon_sym_refine] = "refine",
  [anon_sym_repo] = "repo",
  [anon_sym_document] = "document",
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
  [sym_param_client_block] = "param_client_block",
  [sym_step_list] = "step_list",
  [sym_string_list] = "string_list",
  [sym_identifier_list] = "identifier_list",
  [sym_tier_value] = "tier_value",
  [sym_privacy_value] = "privacy_value",
  [sym_strategy_value] = "strategy_value",
  [sym_execution_mode_value] = "execution_mode_value",
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
  [anon_sym_execution_mode] = anon_sym_execution_mode,
  [anon_sym_strategy] = anon_sym_strategy,
  [anon_sym_test_first] = anon_sym_test_first,
  [anon_sym_attempts] = anon_sym_attempts,
  [anon_sym_escalate_after] = anon_sym_escalate_after,
  [anon_sym_param] = anon_sym_param,
  [anon_sym_public] = anon_sym_public,
  [anon_sym_local_only] = anon_sym_local_only,
  [anon_sym_single_pass] = anon_sym_single_pass,
  [anon_sym_refine] = anon_sym_refine,
  [anon_sym_repo] = anon_sym_repo,
  [anon_sym_document] = anon_sym_document,
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
  [sym_param_client_block] = sym_param_client_block,
  [sym_step_list] = sym_step_list,
  [sym_string_list] = sym_string_list,
  [sym_identifier_list] = sym_identifier_list,
  [sym_tier_value] = sym_tier_value,
  [sym_privacy_value] = sym_privacy_value,
  [sym_strategy_value] = sym_strategy_value,
  [sym_execution_mode_value] = sym_execution_mode_value,
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
  [anon_sym_execution_mode] = {
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
  [anon_sym_repo] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_document] = {
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
  [sym_param_client_block] = {
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
  [sym_execution_mode_value] = {
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
  [166] = 25,
  [167] = 167,
  [168] = 168,
  [169] = 169,
  [170] = 170,
  [171] = 171,
  [172] = 172,
  [173] = 173,
  [174] = 174,
  [175] = 175,
  [176] = 176,
  [177] = 177,
  [178] = 178,
};

static bool ts_lex(TSLexer *lexer, TSStateId state) {
  START_LEXER();
  eof = lexer->eof(lexer);
  switch (state) {
    case 0:
      if (eof) ADVANCE(544);
      ADVANCE_MAP(
        '"', 3,
        '#', 4,
        '/', 13,
        '[', 613,
        ']', 614,
        'a', 216,
        'b', 405,
        'c', 39,
        'd', 133,
        'e', 205,
        'f', 48,
        'i', 118,
        'j', 511,
        'l', 347,
        'm', 41,
        'n', 350,
        'o', 527,
        'p', 46,
        'r', 134,
        's', 95,
        't', 135,
        'u', 317,
        'v', 50,
        'w', 354,
        '{', 548,
        '}', 549,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(0);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(646);
      END_STATE();
    case 1:
      if (lookahead == '"') ADVANCE(3);
      if (lookahead == '#') ADVANCE(4);
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == '[') ADVANCE(613);
      if (lookahead == ']') ADVANCE(614);
      if (lookahead == '}') ADVANCE(549);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(1);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 2:
      if (lookahead == '"') ADVANCE(3);
      if (lookahead == '#') ADVANCE(4);
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'c') ADVANCE(671);
      if (lookahead == 'e') ADVANCE(709);
      if (lookahead == 'm') ADVANCE(659);
      if (lookahead == 'r') ADVANCE(660);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(2);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 3:
      if (lookahead == '"') ADVANCE(643);
      if (lookahead == '\\') ADVANCE(542);
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
      if (lookahead == '#') ADVANCE(644);
      if (lookahead != 0) ADVANCE(5);
      END_STATE();
    case 7:
      if (lookahead == '.') ADVANCE(541);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(7);
      END_STATE();
    case 8:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == ']') ADVANCE(614);
      if (lookahead == 'l') ADVANCE(694);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(8);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 9:
      ADVANCE_MAP(
        '/', 13,
        'a', 221,
        'b', 405,
        'c', 263,
        'd', 151,
        'e', 440,
        'i', 298,
        'j', 511,
        'm', 42,
        'o', 527,
        'p', 80,
        'r', 152,
        's', 96,
        't', 190,
        'u', 317,
        'v', 50,
        '}', 549,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(9);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(647);
      END_STATE();
    case 10:
      ADVANCE_MAP(
        '/', 13,
        'a', 221,
        'b', 405,
        'c', 367,
        'e', 440,
        'i', 479,
        'j', 511,
        'm', 42,
        'p', 79,
        'r', 160,
        's', 478,
        't', 197,
        'u', 317,
        'v', 161,
        '}', 549,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(10);
      END_STATE();
    case 11:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'c') ADVANCE(284);
      if (lookahead == 'i') ADVANCE(311);
      if (lookahead == 't') ADVANCE(199);
      if (lookahead == '}') ADVANCE(549);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(11);
      END_STATE();
    case 12:
      if (lookahead == '/') ADVANCE(13);
      if (lookahead == 'r') ADVANCE(663);
      if (lookahead == 's') ADVANCE(677);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(12);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 13:
      if (lookahead == '/') ADVANCE(546);
      END_STATE();
    case 14:
      if (lookahead == '_') ADVANCE(251);
      END_STATE();
    case 15:
      if (lookahead == '_') ADVANCE(280);
      END_STATE();
    case 16:
      if (lookahead == '_') ADVANCE(103);
      END_STATE();
    case 17:
      if (lookahead == '_') ADVANCE(259);
      END_STATE();
    case 18:
      if (lookahead == '_') ADVANCE(249);
      END_STATE();
    case 19:
      if (lookahead == '_') ADVANCE(385);
      END_STATE();
    case 20:
      if (lookahead == '_') ADVANCE(445);
      END_STATE();
    case 21:
      if (lookahead == '_') ADVANCE(211);
      END_STATE();
    case 22:
      if (lookahead == '_') ADVANCE(211);
      if (lookahead == 's') ADVANCE(37);
      END_STATE();
    case 23:
      if (lookahead == '_') ADVANCE(449);
      END_STATE();
    case 24:
      if (lookahead == '_') ADVANCE(299);
      END_STATE();
    case 25:
      if (lookahead == '_') ADVANCE(355);
      END_STATE();
    case 26:
      if (lookahead == '_') ADVANCE(110);
      END_STATE();
    case 27:
      if (lookahead == '_') ADVANCE(353);
      END_STATE();
    case 28:
      if (lookahead == '_') ADVANCE(388);
      END_STATE();
    case 29:
      if (lookahead == '_') ADVANCE(57);
      END_STATE();
    case 30:
      if (lookahead == '_') ADVANCE(448);
      END_STATE();
    case 31:
      if (lookahead == '_') ADVANCE(444);
      END_STATE();
    case 32:
      if (lookahead == '_') ADVANCE(373);
      END_STATE();
    case 33:
      if (lookahead == '_') ADVANCE(493);
      END_STATE();
    case 34:
      if (lookahead == '_') ADVANCE(364);
      END_STATE();
    case 35:
      if (lookahead == '_') ADVANCE(360);
      END_STATE();
    case 36:
      if (lookahead == '_') ADVANCE(499);
      END_STATE();
    case 37:
      if (lookahead == '_') ADVANCE(212);
      END_STATE();
    case 38:
      if (lookahead == '_') ADVANCE(308);
      END_STATE();
    case 39:
      if (lookahead == 'a') ADVANCE(269);
      if (lookahead == 'h') ADVANCE(158);
      if (lookahead == 'l') ADVANCE(226);
      if (lookahead == 'o') ADVANCE(293);
      END_STATE();
    case 40:
      if (lookahead == 'a') ADVANCE(269);
      if (lookahead == 'l') ADVANCE(258);
      if (lookahead == 'o') ADVANCE(345);
      END_STATE();
    case 41:
      if (lookahead == 'a') ADVANCE(529);
      if (lookahead == 'e') ADVANCE(92);
      if (lookahead == 'o') ADVANCE(125);
      END_STATE();
    case 42:
      if (lookahead == 'a') ADVANCE(529);
      if (lookahead == 'e') ADVANCE(305);
      END_STATE();
    case 43:
      if (lookahead == 'a') ADVANCE(124);
      if (lookahead == 'f') ADVANCE(247);
      if (lookahead == 'p') ADVANCE(349);
      if (lookahead == 'v') ADVANCE(238);
      END_STATE();
    case 44:
      if (lookahead == 'a') ADVANCE(91);
      if (lookahead == 'e') ADVANCE(382);
      if (lookahead == 'r') ADVANCE(75);
      END_STATE();
    case 45:
      if (lookahead == 'a') ADVANCE(555);
      END_STATE();
    case 46:
      if (lookahead == 'a') ADVANCE(399);
      if (lookahead == 'r') ADVANCE(227);
      if (lookahead == 'u') ADVANCE(89);
      END_STATE();
    case 47:
      if (lookahead == 'a') ADVANCE(399);
      if (lookahead == 'r') ADVANCE(348);
      END_STATE();
    case 48:
      if (lookahead == 'a') ADVANCE(264);
      END_STATE();
    case 49:
      if (lookahead == 'a') ADVANCE(90);
      if (lookahead == 'e') ADVANCE(382);
      if (lookahead == 'r') ADVANCE(77);
      END_STATE();
    case 50:
      if (lookahead == 'a') ADVANCE(406);
      if (lookahead == 'e') ADVANCE(412);
      END_STATE();
    case 51:
      if (lookahead == 'a') ADVANCE(294);
      END_STATE();
    case 52:
      if (lookahead == 'a') ADVANCE(294);
      if (lookahead == 't') ADVANCE(250);
      END_STATE();
    case 53:
      if (lookahead == 'a') ADVANCE(514);
      END_STATE();
    case 54:
      if (lookahead == 'a') ADVANCE(322);
      END_STATE();
    case 55:
      if (lookahead == 'a') ADVANCE(102);
      END_STATE();
    case 56:
      if (lookahead == 'a') ADVANCE(102);
      if (lookahead == 'o') ADVANCE(414);
      END_STATE();
    case 57:
      if (lookahead == 'a') ADVANCE(213);
      END_STATE();
    case 58:
      if (lookahead == 'a') ADVANCE(380);
      END_STATE();
    case 59:
      if (lookahead == 'a') ADVANCE(273);
      END_STATE();
    case 60:
      if (lookahead == 'a') ADVANCE(100);
      END_STATE();
    case 61:
      if (lookahead == 'a') ADVANCE(333);
      END_STATE();
    case 62:
      if (lookahead == 'a') ADVANCE(271);
      END_STATE();
    case 63:
      if (lookahead == 'a') ADVANCE(132);
      if (lookahead == 'v') ADVANCE(238);
      END_STATE();
    case 64:
      if (lookahead == 'a') ADVANCE(123);
      END_STATE();
    case 65:
      if (lookahead == 'a') ADVANCE(277);
      END_STATE();
    case 66:
      if (lookahead == 'a') ADVANCE(420);
      END_STATE();
    case 67:
      if (lookahead == 'a') ADVANCE(267);
      END_STATE();
    case 68:
      if (lookahead == 'a') ADVANCE(504);
      END_STATE();
    case 69:
      if (lookahead == 'a') ADVANCE(240);
      END_STATE();
    case 70:
      if (lookahead == 'a') ADVANCE(531);
      if (lookahead == 'e') ADVANCE(305);
      END_STATE();
    case 71:
      if (lookahead == 'a') ADVANCE(330);
      END_STATE();
    case 72:
      if (lookahead == 'a') ADVANCE(325);
      END_STATE();
    case 73:
      if (lookahead == 'a') ADVANCE(453);
      END_STATE();
    case 74:
      if (lookahead == 'a') ADVANCE(425);
      END_STATE();
    case 75:
      if (lookahead == 'a') ADVANCE(494);
      if (lookahead == 'i') ADVANCE(104);
      END_STATE();
    case 76:
      if (lookahead == 'a') ADVANCE(494);
      if (lookahead == 'i') ADVANCE(106);
      END_STATE();
    case 77:
      if (lookahead == 'a') ADVANCE(494);
      if (lookahead == 'i') ADVANCE(112);
      END_STATE();
    case 78:
      if (lookahead == 'a') ADVANCE(105);
      if (lookahead == 'o') ADVANCE(414);
      END_STATE();
    case 79:
      if (lookahead == 'a') ADVANCE(419);
      END_STATE();
    case 80:
      if (lookahead == 'a') ADVANCE(419);
      if (lookahead == 'r') ADVANCE(348);
      END_STATE();
    case 81:
      if (lookahead == 'a') ADVANCE(497);
      END_STATE();
    case 82:
      if (lookahead == 'a') ADVANCE(279);
      if (lookahead == 'e') ADVANCE(382);
      if (lookahead == 'r') ADVANCE(76);
      END_STATE();
    case 83:
      if (lookahead == 'a') ADVANCE(498);
      END_STATE();
    case 84:
      if (lookahead == 'a') ADVANCE(500);
      END_STATE();
    case 85:
      if (lookahead == 'a') ADVANCE(501);
      END_STATE();
    case 86:
      if (lookahead == 'a') ADVANCE(289);
      END_STATE();
    case 87:
      if (lookahead == 'a') ADVANCE(114);
      END_STATE();
    case 88:
      if (lookahead == 'a') ADVANCE(509);
      END_STATE();
    case 89:
      if (lookahead == 'b') ADVANCE(276);
      END_STATE();
    case 90:
      if (lookahead == 'b') ADVANCE(246);
      END_STATE();
    case 91:
      if (lookahead == 'b') ADVANCE(246);
      if (lookahead == 'c') ADVANCE(262);
      if (lookahead == 'l') ADVANCE(198);
      END_STATE();
    case 92:
      if (lookahead == 'c') ADVANCE(225);
      if (lookahead == 'm') ADVANCE(361);
      END_STATE();
    case 93:
      if (lookahead == 'c') ADVANCE(633);
      END_STATE();
    case 94:
      if (lookahead == 'c') ADVANCE(512);
      END_STATE();
    case 95:
      if (lookahead == 'c') ADVANCE(352);
      if (lookahead == 'i') ADVANCE(318);
      if (lookahead == 't') ADVANCE(44);
      END_STATE();
    case 96:
      if (lookahead == 'c') ADVANCE(352);
      if (lookahead == 't') ADVANCE(49);
      END_STATE();
    case 97:
      if (lookahead == 'c') ADVANCE(352);
      if (lookahead == 't') ADVANCE(82);
      END_STATE();
    case 98:
      if (lookahead == 'c') ADVANCE(223);
      END_STATE();
    case 99:
      if (lookahead == 'c') ADVANCE(285);
      END_STATE();
    case 100:
      if (lookahead == 'c') ADVANCE(535);
      END_STATE();
    case 101:
      if (lookahead == 'c') ADVANCE(517);
      END_STATE();
    case 102:
      if (lookahead == 'c') ADVANCE(483);
      END_STATE();
    case 103:
      if (lookahead == 'c') ADVANCE(376);
      if (lookahead == 'n') ADVANCE(431);
      END_STATE();
    case 104:
      if (lookahead == 'c') ADVANCE(464);
      END_STATE();
    case 105:
      if (lookahead == 'c') ADVANCE(502);
      END_STATE();
    case 106:
      if (lookahead == 'c') ADVANCE(476);
      END_STATE();
    case 107:
      if (lookahead == 'c') ADVANCE(146);
      END_STATE();
    case 108:
      if (lookahead == 'c') ADVANCE(187);
      END_STATE();
    case 109:
      if (lookahead == 'c') ADVANCE(59);
      END_STATE();
    case 110:
      if (lookahead == 'c') ADVANCE(224);
      END_STATE();
    case 111:
      if (lookahead == 'c') ADVANCE(417);
      END_STATE();
    case 112:
      if (lookahead == 'c') ADVANCE(488);
      END_STATE();
    case 113:
      if (lookahead == 'c') ADVANCE(62);
      if (lookahead == 'o') ADVANCE(379);
      END_STATE();
    case 114:
      if (lookahead == 'c') ADVANCE(492);
      END_STATE();
    case 115:
      if (lookahead == 'c') ADVANCE(67);
      END_STATE();
    case 116:
      if (lookahead == 'c') ADVANCE(370);
      END_STATE();
    case 117:
      if (lookahead == 'c') ADVANCE(522);
      END_STATE();
    case 118:
      if (lookahead == 'd') ADVANCE(615);
      if (lookahead == 'm') ADVANCE(378);
      if (lookahead == 'n') ADVANCE(99);
      if (lookahead == 't') ADVANCE(167);
      END_STATE();
    case 119:
      if (lookahead == 'd') ADVANCE(218);
      END_STATE();
    case 120:
      if (lookahead == 'd') ADVANCE(578);
      END_STATE();
    case 121:
      if (lookahead == 'd') ADVANCE(623);
      END_STATE();
    case 122:
      if (lookahead == 'd') ADVANCE(620);
      END_STATE();
    case 123:
      if (lookahead == 'd') ADVANCE(15);
      END_STATE();
    case 124:
      if (lookahead == 'd') ADVANCE(15);
      if (lookahead == 's') ADVANCE(372);
      END_STATE();
    case 125:
      if (lookahead == 'd') ADVANCE(173);
      END_STATE();
    case 126:
      if (lookahead == 'd') ADVANCE(458);
      END_STATE();
    case 127:
      if (lookahead == 'd') ADVANCE(233);
      END_STATE();
    case 128:
      if (lookahead == 'd') ADVANCE(143);
      END_STATE();
    case 129:
      if (lookahead == 'd') ADVANCE(149);
      END_STATE();
    case 130:
      if (lookahead == 'd') ADVANCE(153);
      END_STATE();
    case 131:
      if (lookahead == 'd') ADVANCE(219);
      END_STATE();
    case 132:
      if (lookahead == 'd') ADVANCE(35);
      END_STATE();
    case 133:
      if (lookahead == 'e') ADVANCE(208);
      if (lookahead == 'o') ADVANCE(94);
      END_STATE();
    case 134:
      if (lookahead == 'e') ADVANCE(43);
      END_STATE();
    case 135:
      if (lookahead == 'e') ADVANCE(295);
      if (lookahead == 'i') ADVANCE(172);
      if (lookahead == 'o') ADVANCE(356);
      if (lookahead == 'r') ADVANCE(513);
      END_STATE();
    case 136:
      if (lookahead == 'e') ADVANCE(621);
      END_STATE();
    case 137:
      if (lookahead == 'e') ADVANCE(641);
      END_STATE();
    case 138:
      if (lookahead == 'e') ADVANCE(642);
      END_STATE();
    case 139:
      if (lookahead == 'e') ADVANCE(577);
      END_STATE();
    case 140:
      if (lookahead == 'e') ADVANCE(637);
      END_STATE();
    case 141:
      if (lookahead == 'e') ADVANCE(590);
      END_STATE();
    case 142:
      if (lookahead == 'e') ADVANCE(619);
      END_STATE();
    case 143:
      if (lookahead == 'e') ADVANCE(545);
      END_STATE();
    case 144:
      if (lookahead == 'e') ADVANCE(576);
      END_STATE();
    case 145:
      if (lookahead == 'e') ADVANCE(559);
      END_STATE();
    case 146:
      if (lookahead == 'e') ADVANCE(584);
      END_STATE();
    case 147:
      if (lookahead == 'e') ADVANCE(580);
      END_STATE();
    case 148:
      if (lookahead == 'e') ADVANCE(611);
      END_STATE();
    case 149:
      if (lookahead == 'e') ADVANCE(606);
      END_STATE();
    case 150:
      if (lookahead == 'e') ADVANCE(605);
      END_STATE();
    case 151:
      if (lookahead == 'e') ADVANCE(391);
      END_STATE();
    case 152:
      if (lookahead == 'e') ADVANCE(63);
      END_STATE();
    case 153:
      if (lookahead == 'e') ADVANCE(627);
      END_STATE();
    case 154:
      if (lookahead == 'e') ADVANCE(575);
      END_STATE();
    case 155:
      if (lookahead == 'e') ADVANCE(528);
      END_STATE();
    case 156:
      if (lookahead == 'e') ADVANCE(530);
      END_STATE();
    case 157:
      if (lookahead == 'e') ADVANCE(377);
      END_STATE();
    case 158:
      if (lookahead == 'e') ADVANCE(58);
      END_STATE();
    case 159:
      if (lookahead == 'e') ADVANCE(217);
      END_STATE();
    case 160:
      if (lookahead == 'e') ADVANCE(523);
      END_STATE();
    case 161:
      if (lookahead == 'e') ADVANCE(412);
      END_STATE();
    case 162:
      if (lookahead == 'e') ADVANCE(120);
      END_STATE();
    case 163:
      if (lookahead == 'e') ADVANCE(33);
      END_STATE();
    case 164:
      if (lookahead == 'e') ADVANCE(323);
      END_STATE();
    case 165:
      if (lookahead == 'e') ADVANCE(323);
      if (lookahead == 't') ADVANCE(222);
      END_STATE();
    case 166:
      if (lookahead == 'e') ADVANCE(324);
      if (lookahead == 'l') ADVANCE(363);
      END_STATE();
    case 167:
      if (lookahead == 'e') ADVANCE(407);
      END_STATE();
    case 168:
      if (lookahead == 'e') ADVANCE(328);
      END_STATE();
    case 169:
      if (lookahead == 'e') ADVANCE(214);
      END_STATE();
    case 170:
      if (lookahead == 'e') ADVANCE(122);
      END_STATE();
    case 171:
      if (lookahead == 'e') ADVANCE(16);
      END_STATE();
    case 172:
      if (lookahead == 'e') ADVANCE(401);
      END_STATE();
    case 173:
      if (lookahead == 'e') ADVANCE(265);
      END_STATE();
    case 174:
      if (lookahead == 'e') ADVANCE(28);
      END_STATE();
    case 175:
      if (lookahead == 'e') ADVANCE(357);
      END_STATE();
    case 176:
      if (lookahead == 'e') ADVANCE(413);
      END_STATE();
    case 177:
      if (lookahead == 'e') ADVANCE(452);
      END_STATE();
    case 178:
      if (lookahead == 'e') ADVANCE(29);
      END_STATE();
    case 179:
      if (lookahead == 'e') ADVANCE(418);
      END_STATE();
    case 180:
      if (lookahead == 'e') ADVANCE(503);
      END_STATE();
    case 181:
      if (lookahead == 'e') ADVANCE(64);
      END_STATE();
    case 182:
      if (lookahead == 'e') ADVANCE(433);
      END_STATE();
    case 183:
      if (lookahead == 'e') ADVANCE(415);
      END_STATE();
    case 184:
      if (lookahead == 'e') ADVANCE(268);
      END_STATE();
    case 185:
      if (lookahead == 'e') ADVANCE(18);
      END_STATE();
    case 186:
      if (lookahead == 'e') ADVANCE(404);
      END_STATE();
    case 187:
      if (lookahead == 'e') ADVANCE(437);
      END_STATE();
    case 188:
      if (lookahead == 'e') ADVANCE(319);
      END_STATE();
    case 189:
      if (lookahead == 'e') ADVANCE(101);
      if (lookahead == 'p') ADVANCE(166);
      if (lookahead == 't') ADVANCE(410);
      END_STATE();
    case 190:
      if (lookahead == 'e') ADVANCE(296);
      if (lookahead == 'i') ADVANCE(172);
      if (lookahead == 'o') ADVANCE(356);
      END_STATE();
    case 191:
      if (lookahead == 'e') ADVANCE(321);
      END_STATE();
    case 192:
      if (lookahead == 'e') ADVANCE(321);
      if (lookahead == 'p') ADVANCE(381);
      END_STATE();
    case 193:
      if (lookahead == 'e') ADVANCE(310);
      if (lookahead == 'i') ADVANCE(172);
      if (lookahead == 'o') ADVANCE(356);
      END_STATE();
    case 194:
      if (lookahead == 'e') ADVANCE(457);
      END_STATE();
    case 195:
      if (lookahead == 'e') ADVANCE(416);
      END_STATE();
    case 196:
      if (lookahead == 'e') ADVANCE(334);
      END_STATE();
    case 197:
      if (lookahead == 'e') ADVANCE(312);
      END_STATE();
    case 198:
      if (lookahead == 'e') ADVANCE(340);
      END_STATE();
    case 199:
      if (lookahead == 'e') ADVANCE(455);
      END_STATE();
    case 200:
      if (lookahead == 'e') ADVANCE(339);
      END_STATE();
    case 201:
      if (lookahead == 'e') ADVANCE(343);
      END_STATE();
    case 202:
      if (lookahead == 'e') ADVANCE(306);
      END_STATE();
    case 203:
      if (lookahead == 'e') ADVANCE(117);
      END_STATE();
    case 204:
      if (lookahead == 'e') ADVANCE(117);
      if (lookahead == 'p') ADVANCE(288);
      END_STATE();
    case 205:
      if (lookahead == 'f') ADVANCE(209);
      if (lookahead == 's') ADVANCE(109);
      if (lookahead == 'x') ADVANCE(189);
      END_STATE();
    case 206:
      if (lookahead == 'f') ADVANCE(597);
      END_STATE();
    case 207:
      if (lookahead == 'f') ADVANCE(534);
      END_STATE();
    case 208:
      if (lookahead == 'f') ADVANCE(53);
      if (lookahead == 'p') ADVANCE(165);
      if (lookahead == 's') ADVANCE(111);
      END_STATE();
    case 209:
      if (lookahead == 'f') ADVANCE(359);
      END_STATE();
    case 210:
      if (lookahead == 'f') ADVANCE(274);
      END_STATE();
    case 211:
      if (lookahead == 'f') ADVANCE(239);
      END_STATE();
    case 212:
      if (lookahead == 'f') ADVANCE(368);
      END_STATE();
    case 213:
      if (lookahead == 'f') ADVANCE(505);
      END_STATE();
    case 214:
      if (lookahead == 'f') ADVANCE(253);
      END_STATE();
    case 215:
      if (lookahead == 'g') ADVANCE(563);
      END_STATE();
    case 216:
      if (lookahead == 'g') ADVANCE(188);
      if (lookahead == 't') ADVANCE(480);
      END_STATE();
    case 217:
      if (lookahead == 'g') ADVANCE(536);
      END_STATE();
    case 218:
      if (lookahead == 'g') ADVANCE(163);
      END_STATE();
    case 219:
      if (lookahead == 'g') ADVANCE(148);
      END_STATE();
    case 220:
      if (lookahead == 'g') ADVANCE(282);
      END_STATE();
    case 221:
      if (lookahead == 'g') ADVANCE(201);
      if (lookahead == 't') ADVANCE(480);
      END_STATE();
    case 222:
      if (lookahead == 'h') ADVANCE(599);
      END_STATE();
    case 223:
      if (lookahead == 'h') ADVANCE(26);
      END_STATE();
    case 224:
      if (lookahead == 'h') ADVANCE(69);
      END_STATE();
    case 225:
      if (lookahead == 'h') ADVANCE(61);
      END_STATE();
    case 226:
      if (lookahead == 'i') ADVANCE(192);
      END_STATE();
    case 227:
      if (lookahead == 'i') ADVANCE(524);
      if (lookahead == 'o') ADVANCE(297);
      END_STATE();
    case 228:
      if (lookahead == 'i') ADVANCE(207);
      END_STATE();
    case 229:
      if (lookahead == 'i') ADVANCE(525);
      END_STATE();
    case 230:
      if (lookahead == 'i') ADVANCE(302);
      END_STATE();
    case 231:
      if (lookahead == 'i') ADVANCE(303);
      END_STATE();
    case 232:
      if (lookahead == 'i') ADVANCE(93);
      END_STATE();
    case 233:
      if (lookahead == 'i') ADVANCE(329);
      END_STATE();
    case 234:
      if (lookahead == 'i') ADVANCE(395);
      END_STATE();
    case 235:
      if (lookahead == 'i') ADVANCE(266);
      END_STATE();
    case 236:
      if (lookahead == 'i') ADVANCE(320);
      END_STATE();
    case 237:
      if (lookahead == 'i') ADVANCE(384);
      END_STATE();
    case 238:
      if (lookahead == 'i') ADVANCE(155);
      END_STATE();
    case 239:
      if (lookahead == 'i') ADVANCE(423);
      END_STATE();
    case 240:
      if (lookahead == 'i') ADVANCE(316);
      END_STATE();
    case 241:
      if (lookahead == 'i') ADVANCE(482);
      END_STATE();
    case 242:
      if (lookahead == 'i') ADVANCE(469);
      END_STATE();
    case 243:
      if (lookahead == 'i') ADVANCE(472);
      END_STATE();
    case 244:
      if (lookahead == 'i') ADVANCE(182);
      END_STATE();
    case 245:
      if (lookahead == 'i') ADVANCE(489);
      END_STATE();
    case 246:
      if (lookahead == 'i') ADVANCE(281);
      END_STATE();
    case 247:
      if (lookahead == 'i') ADVANCE(336);
      END_STATE();
    case 248:
      if (lookahead == 'i') ADVANCE(283);
      END_STATE();
    case 249:
      if (lookahead == 'i') ADVANCE(338);
      if (lookahead == 'r') ADVANCE(169);
      END_STATE();
    case 250:
      if (lookahead == 'i') ADVANCE(65);
      END_STATE();
    case 251:
      if (lookahead == 'i') ADVANCE(496);
      if (lookahead == 'p') ADVANCE(74);
      if (lookahead == 'r') ADVANCE(180);
      END_STATE();
    case 252:
      if (lookahead == 'i') ADVANCE(362);
      END_STATE();
    case 253:
      if (lookahead == 'i') ADVANCE(341);
      END_STATE();
    case 254:
      if (lookahead == 'i') ADVANCE(365);
      END_STATE();
    case 255:
      if (lookahead == 'i') ADVANCE(371);
      END_STATE();
    case 256:
      if (lookahead == 'i') ADVANCE(366);
      END_STATE();
    case 257:
      if (lookahead == 'i') ADVANCE(115);
      END_STATE();
    case 258:
      if (lookahead == 'i') ADVANCE(191);
      END_STATE();
    case 259:
      if (lookahead == 'j') ADVANCE(521);
      END_STATE();
    case 260:
      if (lookahead == 'k') ADVANCE(618);
      END_STATE();
    case 261:
      if (lookahead == 'k') ADVANCE(210);
      END_STATE();
    case 262:
      if (lookahead == 'k') ADVANCE(170);
      END_STATE();
    case 263:
      if (lookahead == 'l') ADVANCE(226);
      if (lookahead == 'o') ADVANCE(301);
      END_STATE();
    case 264:
      if (lookahead == 'l') ADVANCE(447);
      END_STATE();
    case 265:
      if (lookahead == 'l') ADVANCE(551);
      END_STATE();
    case 266:
      if (lookahead == 'l') ADVANCE(622);
      END_STATE();
    case 267:
      if (lookahead == 'l') ADVANCE(567);
      END_STATE();
    case 268:
      if (lookahead == 'l') ADVANCE(626);
      END_STATE();
    case 269:
      if (lookahead == 'l') ADVANCE(287);
      END_STATE();
    case 270:
      if (lookahead == 'l') ADVANCE(428);
      END_STATE();
    case 271:
      if (lookahead == 'l') ADVANCE(32);
      END_STATE();
    case 272:
      if (lookahead == 'l') ADVANCE(537);
      END_STATE();
    case 273:
      if (lookahead == 'l') ADVANCE(81);
      END_STATE();
    case 274:
      if (lookahead == 'l') ADVANCE(351);
      END_STATE();
    case 275:
      if (lookahead == 'l') ADVANCE(539);
      END_STATE();
    case 276:
      if (lookahead == 'l') ADVANCE(232);
      END_STATE();
    case 277:
      if (lookahead == 'l') ADVANCE(27);
      END_STATE();
    case 278:
      if (lookahead == 'l') ADVANCE(466);
      END_STATE();
    case 279:
      if (lookahead == 'l') ADVANCE(198);
      END_STATE();
    case 280:
      if (lookahead == 'l') ADVANCE(230);
      if (lookahead == 'n') ADVANCE(429);
      if (lookahead == 'o') ADVANCE(331);
      if (lookahead == 'q') ADVANCE(520);
      END_STATE();
    case 281:
      if (lookahead == 'l') ADVANCE(241);
      END_STATE();
    case 282:
      if (lookahead == 'l') ADVANCE(174);
      END_STATE();
    case 283:
      if (lookahead == 'l') ADVANCE(141);
      END_STATE();
    case 284:
      if (lookahead == 'l') ADVANCE(237);
      if (lookahead == 'o') ADVANCE(300);
      END_STATE();
    case 285:
      if (lookahead == 'l') ADVANCE(518);
      END_STATE();
    case 286:
      if (lookahead == 'l') ADVANCE(184);
      END_STATE();
    case 287:
      if (lookahead == 'l') ADVANCE(176);
      END_STATE();
    case 288:
      if (lookahead == 'l') ADVANCE(363);
      END_STATE();
    case 289:
      if (lookahead == 'l') ADVANCE(286);
      END_STATE();
    case 290:
      if (lookahead == 'l') ADVANCE(83);
      END_STATE();
    case 291:
      if (lookahead == 'l') ADVANCE(84);
      END_STATE();
    case 292:
      if (lookahead == 'l') ADVANCE(85);
      END_STATE();
    case 293:
      if (lookahead == 'm') ADVANCE(304);
      if (lookahead == 'n') ADVANCE(443);
      if (lookahead == 'o') ADVANCE(409);
      END_STATE();
    case 294:
      if (lookahead == 'm') ADVANCE(632);
      END_STATE();
    case 295:
      if (lookahead == 'm') ADVANCE(394);
      if (lookahead == 's') ADVANCE(459);
      END_STATE();
    case 296:
      if (lookahead == 'm') ADVANCE(394);
      if (lookahead == 's') ADVANCE(474);
      END_STATE();
    case 297:
      if (lookahead == 'm') ADVANCE(386);
      END_STATE();
    case 298:
      if (lookahead == 'm') ADVANCE(392);
      if (lookahead == 't') ADVANCE(167);
      END_STATE();
    case 299:
      if (lookahead == 'm') ADVANCE(374);
      END_STATE();
    case 300:
      if (lookahead == 'm') ADVANCE(383);
      END_STATE();
    case 301:
      if (lookahead == 'm') ADVANCE(383);
      if (lookahead == 'n') ADVANCE(443);
      END_STATE();
    case 302:
      if (lookahead == 'm') ADVANCE(242);
      END_STATE();
    case 303:
      if (lookahead == 'm') ADVANCE(175);
      END_STATE();
    case 304:
      if (lookahead == 'm') ADVANCE(72);
      if (lookahead == 'p') ADVANCE(248);
      END_STATE();
    case 305:
      if (lookahead == 'm') ADVANCE(361);
      END_STATE();
    case 306:
      if (lookahead == 'm') ADVANCE(387);
      END_STATE();
    case 307:
      if (lookahead == 'm') ADVANCE(196);
      END_STATE();
    case 308:
      if (lookahead == 'm') ADVANCE(375);
      END_STATE();
    case 309:
      if (lookahead == 'm') ADVANCE(393);
      if (lookahead == 'n') ADVANCE(99);
      END_STATE();
    case 310:
      if (lookahead == 'm') ADVANCE(397);
      if (lookahead == 's') ADVANCE(475);
      END_STATE();
    case 311:
      if (lookahead == 'm') ADVANCE(396);
      END_STATE();
    case 312:
      if (lookahead == 'm') ADVANCE(398);
      if (lookahead == 's') ADVANCE(490);
      END_STATE();
    case 313:
      if (lookahead == 'n') ADVANCE(565);
      END_STATE();
    case 314:
      if (lookahead == 'n') ADVANCE(572);
      END_STATE();
    case 315:
      if (lookahead == 'n') ADVANCE(571);
      END_STATE();
    case 316:
      if (lookahead == 'n') ADVANCE(612);
      END_STATE();
    case 317:
      if (lookahead == 'n') ADVANCE(481);
      END_STATE();
    case 318:
      if (lookahead == 'n') ADVANCE(220);
      END_STATE();
    case 319:
      if (lookahead == 'n') ADVANCE(460);
      END_STATE();
    case 320:
      if (lookahead == 'n') ADVANCE(215);
      END_STATE();
    case 321:
      if (lookahead == 'n') ADVANCE(461);
      END_STATE();
    case 322:
      if (lookahead == 'n') ADVANCE(98);
      END_STATE();
    case 323:
      if (lookahead == 'n') ADVANCE(126);
      END_STATE();
    case 324:
      if (lookahead == 'n') ADVANCE(450);
      END_STATE();
    case 325:
      if (lookahead == 'n') ADVANCE(121);
      END_STATE();
    case 326:
      if (lookahead == 'n') ADVANCE(136);
      END_STATE();
    case 327:
      if (lookahead == 'n') ADVANCE(162);
      END_STATE();
    case 328:
      if (lookahead == 'n') ADVANCE(441);
      END_STATE();
    case 329:
      if (lookahead == 'n') ADVANCE(68);
      END_STATE();
    case 330:
      if (lookahead == 'n') ADVANCE(107);
      END_STATE();
    case 331:
      if (lookahead == 'n') ADVANCE(272);
      END_STATE();
    case 332:
      if (lookahead == 'n') ADVANCE(275);
      END_STATE();
    case 333:
      if (lookahead == 'n') ADVANCE(257);
      END_STATE();
    case 334:
      if (lookahead == 'n') ADVANCE(467);
      END_STATE();
    case 335:
      if (lookahead == 'n') ADVANCE(236);
      END_STATE();
    case 336:
      if (lookahead == 'n') ADVANCE(140);
      END_STATE();
    case 337:
      if (lookahead == 'n') ADVANCE(436);
      END_STATE();
    case 338:
      if (lookahead == 'n') ADVANCE(243);
      END_STATE();
    case 339:
      if (lookahead == 'n') ADVANCE(473);
      END_STATE();
    case 340:
      if (lookahead == 'n') ADVANCE(177);
      END_STATE();
    case 341:
      if (lookahead == 'n') ADVANCE(150);
      END_STATE();
    case 342:
      if (lookahead == 'n') ADVANCE(442);
      END_STATE();
    case 343:
      if (lookahead == 'n') ADVANCE(495);
      END_STATE();
    case 344:
      if (lookahead == 'n') ADVANCE(508);
      END_STATE();
    case 345:
      if (lookahead == 'n') ADVANCE(485);
      END_STATE();
    case 346:
      if (lookahead == 'n') ADVANCE(38);
      END_STATE();
    case 347:
      if (lookahead == 'o') ADVANCE(113);
      END_STATE();
    case 348:
      if (lookahead == 'o') ADVANCE(297);
      END_STATE();
    case 349:
      if (lookahead == 'o') ADVANCE(639);
      END_STATE();
    case 350:
      if (lookahead == 'o') ADVANCE(326);
      END_STATE();
    case 351:
      if (lookahead == 'o') ADVANCE(526);
      END_STATE();
    case 352:
      if (lookahead == 'o') ADVANCE(389);
      END_STATE();
    case 353:
      if (lookahead == 'o') ADVANCE(260);
      END_STATE();
    case 354:
      if (lookahead == 'o') ADVANCE(400);
      if (lookahead == 'r') ADVANCE(245);
      END_STATE();
    case 355:
      if (lookahead == 'o') ADVANCE(206);
      END_STATE();
    case 356:
      if (lookahead == 'o') ADVANCE(270);
      END_STATE();
    case 357:
      if (lookahead == 'o') ADVANCE(516);
      END_STATE();
    case 358:
      if (lookahead == 'o') ADVANCE(515);
      END_STATE();
    case 359:
      if (lookahead == 'o') ADVANCE(411);
      END_STATE();
    case 360:
      if (lookahead == 'o') ADVANCE(331);
      END_STATE();
    case 361:
      if (lookahead == 'o') ADVANCE(408);
      END_STATE();
    case 362:
      if (lookahead == 'o') ADVANCE(313);
      END_STATE();
    case 363:
      if (lookahead == 'o') ADVANCE(421);
      END_STATE();
    case 364:
      if (lookahead == 'o') ADVANCE(314);
      END_STATE();
    case 365:
      if (lookahead == 'o') ADVANCE(315);
      END_STATE();
    case 366:
      if (lookahead == 'o') ADVANCE(346);
      END_STATE();
    case 367:
      if (lookahead == 'o') ADVANCE(342);
      END_STATE();
    case 368:
      if (lookahead == 'o') ADVANCE(402);
      END_STATE();
    case 369:
      if (lookahead == 'o') ADVANCE(403);
      END_STATE();
    case 370:
      if (lookahead == 'o') ADVANCE(390);
      END_STATE();
    case 371:
      if (lookahead == 'o') ADVANCE(337);
      END_STATE();
    case 372:
      if (lookahead == 'o') ADVANCE(335);
      END_STATE();
    case 373:
      if (lookahead == 'o') ADVANCE(332);
      END_STATE();
    case 374:
      if (lookahead == 'o') ADVANCE(129);
      END_STATE();
    case 375:
      if (lookahead == 'o') ADVANCE(130);
      END_STATE();
    case 376:
      if (lookahead == 'o') ADVANCE(344);
      END_STATE();
    case 377:
      if (lookahead == 'p') ADVANCE(165);
      if (lookahead == 's') ADVANCE(111);
      END_STATE();
    case 378:
      if (lookahead == 'p') ADVANCE(56);
      END_STATE();
    case 379:
      if (lookahead == 'p') ADVANCE(600);
      END_STATE();
    case 380:
      if (lookahead == 'p') ADVANCE(557);
      END_STATE();
    case 381:
      if (lookahead == 'p') ADVANCE(532);
      END_STATE();
    case 382:
      if (lookahead == 'p') ADVANCE(427);
      END_STATE();
    case 383:
      if (lookahead == 'p') ADVANCE(248);
      END_STATE();
    case 384:
      if (lookahead == 'p') ADVANCE(381);
      END_STATE();
    case 385:
      if (lookahead == 'p') ADVANCE(74);
      if (lookahead == 'r') ADVANCE(180);
      END_STATE();
    case 386:
      if (lookahead == 'p') ADVANCE(463);
      END_STATE();
    case 387:
      if (lookahead == 'p') ADVANCE(486);
      END_STATE();
    case 388:
      if (lookahead == 'p') ADVANCE(73);
      END_STATE();
    case 389:
      if (lookahead == 'p') ADVANCE(139);
      END_STATE();
    case 390:
      if (lookahead == 'p') ADVANCE(147);
      END_STATE();
    case 391:
      if (lookahead == 'p') ADVANCE(164);
      if (lookahead == 's') ADVANCE(111);
      END_STATE();
    case 392:
      if (lookahead == 'p') ADVANCE(55);
      END_STATE();
    case 393:
      if (lookahead == 'p') ADVANCE(78);
      END_STATE();
    case 394:
      if (lookahead == 'p') ADVANCE(290);
      END_STATE();
    case 395:
      if (lookahead == 'p') ADVANCE(507);
      END_STATE();
    case 396:
      if (lookahead == 'p') ADVANCE(87);
      END_STATE();
    case 397:
      if (lookahead == 'p') ADVANCE(291);
      END_STATE();
    case 398:
      if (lookahead == 'p') ADVANCE(292);
      END_STATE();
    case 399:
      if (lookahead == 'r') ADVANCE(52);
      END_STATE();
    case 400:
      if (lookahead == 'r') ADVANCE(261);
      END_STATE();
    case 401:
      if (lookahead == 'r') ADVANCE(550);
      END_STATE();
    case 402:
      if (lookahead == 'r') ADVANCE(598);
      END_STATE();
    case 403:
      if (lookahead == 'r') ADVANCE(561);
      END_STATE();
    case 404:
      if (lookahead == 'r') ADVANCE(631);
      END_STATE();
    case 405:
      if (lookahead == 'r') ADVANCE(54);
      END_STATE();
    case 406:
      if (lookahead == 'r') ADVANCE(426);
      END_STATE();
    case 407:
      if (lookahead == 'r') ADVANCE(23);
      END_STATE();
    case 408:
      if (lookahead == 'r') ADVANCE(533);
      END_STATE();
    case 409:
      if (lookahead == 'r') ADVANCE(127);
      END_STATE();
    case 410:
      if (lookahead == 'r') ADVANCE(45);
      END_STATE();
    case 411:
      if (lookahead == 'r') ADVANCE(462);
      END_STATE();
    case 412:
      if (lookahead == 'r') ADVANCE(228);
      END_STATE();
    case 413:
      if (lookahead == 'r') ADVANCE(446);
      END_STATE();
    case 414:
      if (lookahead == 'r') ADVANCE(506);
      END_STATE();
    case 415:
      if (lookahead == 'r') ADVANCE(540);
      END_STATE();
    case 416:
      if (lookahead == 'r') ADVANCE(88);
      END_STATE();
    case 417:
      if (lookahead == 'r') ADVANCE(234);
      END_STATE();
    case 418:
      if (lookahead == 'r') ADVANCE(432);
      END_STATE();
    case 419:
      if (lookahead == 'r') ADVANCE(51);
      END_STATE();
    case 420:
      if (lookahead == 'r') ADVANCE(468);
      END_STATE();
    case 421:
      if (lookahead == 'r') ADVANCE(142);
      END_STATE();
    case 422:
      if (lookahead == 'r') ADVANCE(244);
      END_STATE();
    case 423:
      if (lookahead == 'r') ADVANCE(454);
      END_STATE();
    case 424:
      if (lookahead == 'r') ADVANCE(108);
      END_STATE();
    case 425:
      if (lookahead == 'r') ADVANCE(86);
      END_STATE();
    case 426:
      if (lookahead == 's') ADVANCE(556);
      END_STATE();
    case 427:
      if (lookahead == 's') ADVANCE(625);
      END_STATE();
    case 428:
      if (lookahead == 's') ADVANCE(574);
      END_STATE();
    case 429:
      if (lookahead == 's') ADVANCE(582);
      END_STATE();
    case 430:
      if (lookahead == 's') ADVANCE(630);
      END_STATE();
    case 431:
      if (lookahead == 's') ADVANCE(583);
      END_STATE();
    case 432:
      if (lookahead == 's') ADVANCE(603);
      END_STATE();
    case 433:
      if (lookahead == 's') ADVANCE(573);
      END_STATE();
    case 434:
      if (lookahead == 's') ADVANCE(635);
      END_STATE();
    case 435:
      if (lookahead == 's') ADVANCE(595);
      END_STATE();
    case 436:
      if (lookahead == 's') ADVANCE(607);
      END_STATE();
    case 437:
      if (lookahead == 's') ADVANCE(585);
      END_STATE();
    case 438:
      if (lookahead == 's') ADVANCE(602);
      END_STATE();
    case 439:
      if (lookahead == 's') ADVANCE(109);
      if (lookahead == 'x') ADVANCE(204);
      END_STATE();
    case 440:
      if (lookahead == 's') ADVANCE(109);
      if (lookahead == 'x') ADVANCE(203);
      END_STATE();
    case 441:
      if (lookahead == 's') ADVANCE(519);
      END_STATE();
    case 442:
      if (lookahead == 's') ADVANCE(168);
      END_STATE();
    case 443:
      if (lookahead == 's') ADVANCE(168);
      if (lookahead == 't') ADVANCE(156);
      END_STATE();
    case 444:
      if (lookahead == 's') ADVANCE(116);
      END_STATE();
    case 445:
      if (lookahead == 's') ADVANCE(116);
      if (lookahead == 't') ADVANCE(194);
      END_STATE();
    case 446:
      if (lookahead == 's') ADVANCE(25);
      END_STATE();
    case 447:
      if (lookahead == 's') ADVANCE(138);
      END_STATE();
    case 448:
      if (lookahead == 's') ADVANCE(358);
      END_STATE();
    case 449:
      if (lookahead == 's') ADVANCE(484);
      END_STATE();
    case 450:
      if (lookahead == 's') ADVANCE(229);
      END_STATE();
    case 451:
      if (lookahead == 's') ADVANCE(24);
      END_STATE();
    case 452:
      if (lookahead == 's') ADVANCE(456);
      END_STATE();
    case 453:
      if (lookahead == 's') ADVANCE(434);
      END_STATE();
    case 454:
      if (lookahead == 's') ADVANCE(470);
      END_STATE();
    case 455:
      if (lookahead == 's') ADVANCE(477);
      END_STATE();
    case 456:
      if (lookahead == 's') ADVANCE(30);
      END_STATE();
    case 457:
      if (lookahead == 's') ADVANCE(491);
      END_STATE();
    case 458:
      if (lookahead == 's') ADVANCE(34);
      END_STATE();
    case 459:
      if (lookahead == 't') ADVANCE(594);
      END_STATE();
    case 460:
      if (lookahead == 't') ADVANCE(570);
      END_STATE();
    case 461:
      if (lookahead == 't') ADVANCE(547);
      END_STATE();
    case 462:
      if (lookahead == 't') ADVANCE(552);
      END_STATE();
    case 463:
      if (lookahead == 't') ADVANCE(569);
      END_STATE();
    case 464:
      if (lookahead == 't') ADVANCE(617);
      END_STATE();
    case 465:
      if (lookahead == 't') ADVANCE(596);
      END_STATE();
    case 466:
      if (lookahead == 't') ADVANCE(554);
      END_STATE();
    case 467:
      if (lookahead == 't') ADVANCE(640);
      END_STATE();
    case 468:
      if (lookahead == 't') ADVANCE(608);
      END_STATE();
    case 469:
      if (lookahead == 't') ADVANCE(587);
      END_STATE();
    case 470:
      if (lookahead == 't') ADVANCE(629);
      END_STATE();
    case 471:
      if (lookahead == 't') ADVANCE(610);
      END_STATE();
    case 472:
      if (lookahead == 't') ADVANCE(604);
      END_STATE();
    case 473:
      if (lookahead == 't') ADVANCE(588);
      END_STATE();
    case 474:
      if (lookahead == 't') ADVANCE(593);
      END_STATE();
    case 475:
      if (lookahead == 't') ADVANCE(22);
      END_STATE();
    case 476:
      if (lookahead == 't') ADVANCE(616);
      END_STATE();
    case 477:
      if (lookahead == 't') ADVANCE(592);
      END_STATE();
    case 478:
      if (lookahead == 't') ADVANCE(49);
      END_STATE();
    case 479:
      if (lookahead == 't') ADVANCE(167);
      END_STATE();
    case 480:
      if (lookahead == 't') ADVANCE(202);
      END_STATE();
    case 481:
      if (lookahead == 't') ADVANCE(235);
      END_STATE();
    case 482:
      if (lookahead == 't') ADVANCE(538);
      END_STATE();
    case 483:
      if (lookahead == 't') ADVANCE(20);
      END_STATE();
    case 484:
      if (lookahead == 't') ADVANCE(66);
      END_STATE();
    case 485:
      if (lookahead == 't') ADVANCE(156);
      END_STATE();
    case 486:
      if (lookahead == 't') ADVANCE(430);
      END_STATE();
    case 487:
      if (lookahead == 't') ADVANCE(252);
      END_STATE();
    case 488:
      if (lookahead == 't') ADVANCE(17);
      END_STATE();
    case 489:
      if (lookahead == 't') ADVANCE(171);
      END_STATE();
    case 490:
      if (lookahead == 't') ADVANCE(21);
      END_STATE();
    case 491:
      if (lookahead == 't') ADVANCE(435);
      END_STATE();
    case 492:
      if (lookahead == 't') ADVANCE(36);
      END_STATE();
    case 493:
      if (lookahead == 't') ADVANCE(231);
      END_STATE();
    case 494:
      if (lookahead == 't') ADVANCE(159);
      END_STATE();
    case 495:
      if (lookahead == 't') ADVANCE(438);
      END_STATE();
    case 496:
      if (lookahead == 't') ADVANCE(195);
      END_STATE();
    case 497:
      if (lookahead == 't') ADVANCE(178);
      END_STATE();
    case 498:
      if (lookahead == 't') ADVANCE(144);
      END_STATE();
    case 499:
      if (lookahead == 't') ADVANCE(194);
      END_STATE();
    case 500:
      if (lookahead == 't') ADVANCE(154);
      END_STATE();
    case 501:
      if (lookahead == 't') ADVANCE(185);
      END_STATE();
    case 502:
      if (lookahead == 't') ADVANCE(31);
      END_STATE();
    case 503:
      if (lookahead == 't') ADVANCE(422);
      END_STATE();
    case 504:
      if (lookahead == 't') ADVANCE(369);
      END_STATE();
    case 505:
      if (lookahead == 't') ADVANCE(186);
      END_STATE();
    case 506:
      if (lookahead == 't') ADVANCE(71);
      END_STATE();
    case 507:
      if (lookahead == 't') ADVANCE(254);
      END_STATE();
    case 508:
      if (lookahead == 't') ADVANCE(200);
      END_STATE();
    case 509:
      if (lookahead == 't') ADVANCE(255);
      END_STATE();
    case 510:
      if (lookahead == 't') ADVANCE(256);
      END_STATE();
    case 511:
      if (lookahead == 'u') ADVANCE(119);
      END_STATE();
    case 512:
      if (lookahead == 'u') ADVANCE(307);
      END_STATE();
    case 513:
      if (lookahead == 'u') ADVANCE(137);
      END_STATE();
    case 514:
      if (lookahead == 'u') ADVANCE(278);
      END_STATE();
    case 515:
      if (lookahead == 'u') ADVANCE(424);
      END_STATE();
    case 516:
      if (lookahead == 'u') ADVANCE(471);
      END_STATE();
    case 517:
      if (lookahead == 'u') ADVANCE(487);
      END_STATE();
    case 518:
      if (lookahead == 'u') ADVANCE(128);
      END_STATE();
    case 519:
      if (lookahead == 'u') ADVANCE(451);
      END_STATE();
    case 520:
      if (lookahead == 'u') ADVANCE(183);
      END_STATE();
    case 521:
      if (lookahead == 'u') ADVANCE(131);
      END_STATE();
    case 522:
      if (lookahead == 'u') ADVANCE(510);
      END_STATE();
    case 523:
      if (lookahead == 'v') ADVANCE(238);
      END_STATE();
    case 524:
      if (lookahead == 'v') ADVANCE(60);
      END_STATE();
    case 525:
      if (lookahead == 'v') ADVANCE(145);
      END_STATE();
    case 526:
      if (lookahead == 'w') ADVANCE(624);
      END_STATE();
    case 527:
      if (lookahead == 'w') ADVANCE(327);
      END_STATE();
    case 528:
      if (lookahead == 'w') ADVANCE(179);
      END_STATE();
    case 529:
      if (lookahead == 'x') ADVANCE(14);
      END_STATE();
    case 530:
      if (lookahead == 'x') ADVANCE(465);
      END_STATE();
    case 531:
      if (lookahead == 'x') ADVANCE(19);
      END_STATE();
    case 532:
      if (lookahead == 'y') ADVANCE(591);
      END_STATE();
    case 533:
      if (lookahead == 'y') ADVANCE(581);
      END_STATE();
    case 534:
      if (lookahead == 'y') ADVANCE(589);
      END_STATE();
    case 535:
      if (lookahead == 'y') ADVANCE(553);
      END_STATE();
    case 536:
      if (lookahead == 'y') ADVANCE(628);
      END_STATE();
    case 537:
      if (lookahead == 'y') ADVANCE(579);
      END_STATE();
    case 538:
      if (lookahead == 'y') ADVANCE(609);
      END_STATE();
    case 539:
      if (lookahead == 'y') ADVANCE(634);
      END_STATE();
    case 540:
      if (lookahead == 'y') ADVANCE(586);
      END_STATE();
    case 541:
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(645);
      END_STATE();
    case 542:
      if (lookahead != 0 &&
          lookahead != '\n') ADVANCE(3);
      END_STATE();
    case 543:
      if (eof) ADVANCE(544);
      ADVANCE_MAP(
        '/', 13,
        '[', 613,
        'a', 216,
        'c', 40,
        'd', 157,
        'e', 439,
        'i', 309,
        'm', 70,
        'o', 527,
        'p', 47,
        'r', 181,
        's', 97,
        't', 193,
        'v', 50,
        'w', 354,
        '{', 548,
        '}', 549,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(543);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(7);
      END_STATE();
    case 544:
      ACCEPT_TOKEN(ts_builtin_sym_end);
      END_STATE();
    case 545:
      ACCEPT_TOKEN(anon_sym_include);
      END_STATE();
    case 546:
      ACCEPT_TOKEN(sym_comment);
      if (lookahead != 0 &&
          lookahead != '\n') ADVANCE(546);
      END_STATE();
    case 547:
      ACCEPT_TOKEN(anon_sym_client);
      END_STATE();
    case 548:
      ACCEPT_TOKEN(anon_sym_LBRACE);
      END_STATE();
    case 549:
      ACCEPT_TOKEN(anon_sym_RBRACE);
      END_STATE();
    case 550:
      ACCEPT_TOKEN(anon_sym_tier);
      END_STATE();
    case 551:
      ACCEPT_TOKEN(anon_sym_model);
      END_STATE();
    case 552:
      ACCEPT_TOKEN(anon_sym_effort);
      END_STATE();
    case 553:
      ACCEPT_TOKEN(anon_sym_privacy);
      END_STATE();
    case 554:
      ACCEPT_TOKEN(anon_sym_default);
      END_STATE();
    case 555:
      ACCEPT_TOKEN(anon_sym_extra);
      END_STATE();
    case 556:
      ACCEPT_TOKEN(anon_sym_vars);
      END_STATE();
    case 557:
      ACCEPT_TOKEN(anon_sym_cheap);
      END_STATE();
    case 558:
      ACCEPT_TOKEN(anon_sym_cheap);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 559:
      ACCEPT_TOKEN(anon_sym_expensive);
      END_STATE();
    case 560:
      ACCEPT_TOKEN(anon_sym_expensive);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 561:
      ACCEPT_TOKEN(anon_sym_coordinator);
      END_STATE();
    case 562:
      ACCEPT_TOKEN(anon_sym_coordinator);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 563:
      ACCEPT_TOKEN(anon_sym_reasoning);
      END_STATE();
    case 564:
      ACCEPT_TOKEN(anon_sym_reasoning);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 565:
      ACCEPT_TOKEN(anon_sym_execution);
      END_STATE();
    case 566:
      ACCEPT_TOKEN(anon_sym_execution);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 567:
      ACCEPT_TOKEN(anon_sym_mechanical);
      END_STATE();
    case 568:
      ACCEPT_TOKEN(anon_sym_mechanical);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 569:
      ACCEPT_TOKEN(anon_sym_prompt);
      END_STATE();
    case 570:
      ACCEPT_TOKEN(anon_sym_agent);
      END_STATE();
    case 571:
      ACCEPT_TOKEN(anon_sym_description);
      END_STATE();
    case 572:
      ACCEPT_TOKEN(anon_sym_depends_on);
      END_STATE();
    case 573:
      ACCEPT_TOKEN(anon_sym_max_retries);
      END_STATE();
    case 574:
      ACCEPT_TOKEN(anon_sym_tools);
      END_STATE();
    case 575:
      ACCEPT_TOKEN(anon_sym_template);
      END_STATE();
    case 576:
      ACCEPT_TOKEN(anon_sym_template);
      if (lookahead == '_') ADVANCE(249);
      END_STATE();
    case 577:
      ACCEPT_TOKEN(anon_sym_scope);
      END_STATE();
    case 578:
      ACCEPT_TOKEN(anon_sym_owned);
      END_STATE();
    case 579:
      ACCEPT_TOKEN(anon_sym_read_only);
      END_STATE();
    case 580:
      ACCEPT_TOKEN(anon_sym_impact_scope);
      END_STATE();
    case 581:
      ACCEPT_TOKEN(anon_sym_memory);
      END_STATE();
    case 582:
      ACCEPT_TOKEN(anon_sym_read_ns);
      END_STATE();
    case 583:
      ACCEPT_TOKEN(anon_sym_write_ns);
      END_STATE();
    case 584:
      ACCEPT_TOKEN(anon_sym_importance);
      END_STATE();
    case 585:
      ACCEPT_TOKEN(anon_sym_staleness_sources);
      END_STATE();
    case 586:
      ACCEPT_TOKEN(anon_sym_read_query);
      END_STATE();
    case 587:
      ACCEPT_TOKEN(anon_sym_read_limit);
      END_STATE();
    case 588:
      ACCEPT_TOKEN(anon_sym_write_content);
      END_STATE();
    case 589:
      ACCEPT_TOKEN(anon_sym_verify);
      END_STATE();
    case 590:
      ACCEPT_TOKEN(anon_sym_compile);
      END_STATE();
    case 591:
      ACCEPT_TOKEN(anon_sym_clippy);
      END_STATE();
    case 592:
      ACCEPT_TOKEN(anon_sym_test);
      END_STATE();
    case 593:
      ACCEPT_TOKEN(anon_sym_test);
      if (lookahead == '_') ADVANCE(211);
      END_STATE();
    case 594:
      ACCEPT_TOKEN(anon_sym_test);
      if (lookahead == '_') ADVANCE(211);
      if (lookahead == 's') ADVANCE(37);
      END_STATE();
    case 595:
      ACCEPT_TOKEN(anon_sym_impact_tests);
      END_STATE();
    case 596:
      ACCEPT_TOKEN(anon_sym_context);
      END_STATE();
    case 597:
      ACCEPT_TOKEN(anon_sym_callers_of);
      END_STATE();
    case 598:
      ACCEPT_TOKEN(anon_sym_tests_for);
      END_STATE();
    case 599:
      ACCEPT_TOKEN(anon_sym_depth);
      END_STATE();
    case 600:
      ACCEPT_TOKEN(anon_sym_loop);
      END_STATE();
    case 601:
      ACCEPT_TOKEN(anon_sym_loop);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 602:
      ACCEPT_TOKEN(anon_sym_agents);
      END_STATE();
    case 603:
      ACCEPT_TOKEN(anon_sym_reviewers);
      END_STATE();
    case 604:
      ACCEPT_TOKEN(anon_sym_template_init);
      END_STATE();
    case 605:
      ACCEPT_TOKEN(anon_sym_template_refine);
      END_STATE();
    case 606:
      ACCEPT_TOKEN(anon_sym_consensus_mode);
      END_STATE();
    case 607:
      ACCEPT_TOKEN(anon_sym_max_iterations);
      END_STATE();
    case 608:
      ACCEPT_TOKEN(anon_sym_iter_start);
      END_STATE();
    case 609:
      ACCEPT_TOKEN(anon_sym_stability);
      END_STATE();
    case 610:
      ACCEPT_TOKEN(anon_sym_judge_timeout);
      END_STATE();
    case 611:
      ACCEPT_TOKEN(anon_sym_strict_judge);
      END_STATE();
    case 612:
      ACCEPT_TOKEN(anon_sym_branch_chain);
      END_STATE();
    case 613:
      ACCEPT_TOKEN(anon_sym_LBRACK);
      END_STATE();
    case 614:
      ACCEPT_TOKEN(anon_sym_RBRACK);
      END_STATE();
    case 615:
      ACCEPT_TOKEN(anon_sym_id);
      END_STATE();
    case 616:
      ACCEPT_TOKEN(anon_sym_strict);
      END_STATE();
    case 617:
      ACCEPT_TOKEN(anon_sym_strict);
      if (lookahead == '_') ADVANCE(259);
      END_STATE();
    case 618:
      ACCEPT_TOKEN(anon_sym_partial_ok);
      END_STATE();
    case 619:
      ACCEPT_TOKEN(anon_sym_explore);
      END_STATE();
    case 620:
      ACCEPT_TOKEN(anon_sym_stacked);
      END_STATE();
    case 621:
      ACCEPT_TOKEN(anon_sym_none);
      END_STATE();
    case 622:
      ACCEPT_TOKEN(anon_sym_until);
      END_STATE();
    case 623:
      ACCEPT_TOKEN(anon_sym_command);
      END_STATE();
    case 624:
      ACCEPT_TOKEN(anon_sym_workflow);
      END_STATE();
    case 625:
      ACCEPT_TOKEN(anon_sym_steps);
      END_STATE();
    case 626:
      ACCEPT_TOKEN(anon_sym_max_parallel);
      END_STATE();
    case 627:
      ACCEPT_TOKEN(anon_sym_execution_mode);
      END_STATE();
    case 628:
      ACCEPT_TOKEN(anon_sym_strategy);
      END_STATE();
    case 629:
      ACCEPT_TOKEN(anon_sym_test_first);
      END_STATE();
    case 630:
      ACCEPT_TOKEN(anon_sym_attempts);
      END_STATE();
    case 631:
      ACCEPT_TOKEN(anon_sym_escalate_after);
      END_STATE();
    case 632:
      ACCEPT_TOKEN(anon_sym_param);
      END_STATE();
    case 633:
      ACCEPT_TOKEN(anon_sym_public);
      END_STATE();
    case 634:
      ACCEPT_TOKEN(anon_sym_local_only);
      END_STATE();
    case 635:
      ACCEPT_TOKEN(anon_sym_single_pass);
      END_STATE();
    case 636:
      ACCEPT_TOKEN(anon_sym_single_pass);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 637:
      ACCEPT_TOKEN(anon_sym_refine);
      END_STATE();
    case 638:
      ACCEPT_TOKEN(anon_sym_refine);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 639:
      ACCEPT_TOKEN(anon_sym_repo);
      END_STATE();
    case 640:
      ACCEPT_TOKEN(anon_sym_document);
      END_STATE();
    case 641:
      ACCEPT_TOKEN(anon_sym_true);
      END_STATE();
    case 642:
      ACCEPT_TOKEN(anon_sym_false);
      END_STATE();
    case 643:
      ACCEPT_TOKEN(sym_string);
      END_STATE();
    case 644:
      ACCEPT_TOKEN(sym_raw_string);
      END_STATE();
    case 645:
      ACCEPT_TOKEN(sym_float);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(645);
      END_STATE();
    case 646:
      ACCEPT_TOKEN(sym_integer);
      if (lookahead == '.') ADVANCE(541);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(646);
      END_STATE();
    case 647:
      ACCEPT_TOKEN(sym_integer);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(647);
      END_STATE();
    case 648:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '_') ADVANCE(698);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 649:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(702);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 650:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(696);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 651:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(680);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 652:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(687);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 653:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(706);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 654:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'a') ADVANCE(704);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('b' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 655:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(672);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 656:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(707);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 657:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'c') ADVANCE(651);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 658:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'd') ADVANCE(675);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 659:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(655);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 660:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(649);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 661:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(684);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 662:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(560);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 663:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(668);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 664:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(638);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 665:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(648);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 666:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(656);
      if (lookahead == 'p') ADVANCE(661);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 667:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'e') ADVANCE(650);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 668:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'f') ADVANCE(678);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 669:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'g') ADVANCE(564);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 670:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'g') ADVANCE(681);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 671:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(667);
      if (lookahead == 'o') ADVANCE(690);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 672:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'h') ADVANCE(652);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 673:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(708);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 674:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(657);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 675:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(686);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 676:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(682);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 677:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(685);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 678:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(688);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 679:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'i') ADVANCE(695);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 680:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(568);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 681:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'l') ADVANCE(665);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 682:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(669);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 683:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(566);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 684:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(703);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 685:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(670);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 686:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(653);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 687:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(674);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 688:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(664);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 689:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'n') ADVANCE(676);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 690:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(699);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 691:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(700);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 692:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(697);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 693:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(689);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 694:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(692);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 695:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'o') ADVANCE(683);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 696:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(558);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 697:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(601);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 698:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'p') ADVANCE(654);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 699:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(658);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 700:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'r') ADVANCE(562);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 701:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(636);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 702:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(693);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 703:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(673);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 704:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 's') ADVANCE(701);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 705:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(679);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 706:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 't') ADVANCE(691);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 707:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'u') ADVANCE(705);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 708:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'v') ADVANCE(662);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 709:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == 'x') ADVANCE(666);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    case 710:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(710);
      END_STATE();
    default:
      return false;
  }
}

static const TSLexMode ts_lex_modes[STATE_COUNT] = {
  [0] = {.lex_state = 0},
  [1] = {.lex_state = 0},
  [2] = {.lex_state = 9},
  [3] = {.lex_state = 543},
  [4] = {.lex_state = 543},
  [5] = {.lex_state = 9},
  [6] = {.lex_state = 9},
  [7] = {.lex_state = 10},
  [8] = {.lex_state = 10},
  [9] = {.lex_state = 543},
  [10] = {.lex_state = 543},
  [11] = {.lex_state = 543},
  [12] = {.lex_state = 543},
  [13] = {.lex_state = 543},
  [14] = {.lex_state = 543},
  [15] = {.lex_state = 543},
  [16] = {.lex_state = 0},
  [17] = {.lex_state = 543},
  [18] = {.lex_state = 543},
  [19] = {.lex_state = 0},
  [20] = {.lex_state = 543},
  [21] = {.lex_state = 10},
  [22] = {.lex_state = 10},
  [23] = {.lex_state = 543},
  [24] = {.lex_state = 10},
  [25] = {.lex_state = 543},
  [26] = {.lex_state = 10},
  [27] = {.lex_state = 543},
  [28] = {.lex_state = 543},
  [29] = {.lex_state = 543},
  [30] = {.lex_state = 543},
  [31] = {.lex_state = 543},
  [32] = {.lex_state = 10},
  [33] = {.lex_state = 10},
  [34] = {.lex_state = 10},
  [35] = {.lex_state = 10},
  [36] = {.lex_state = 10},
  [37] = {.lex_state = 10},
  [38] = {.lex_state = 10},
  [39] = {.lex_state = 10},
  [40] = {.lex_state = 543},
  [41] = {.lex_state = 543},
  [42] = {.lex_state = 543},
  [43] = {.lex_state = 543},
  [44] = {.lex_state = 543},
  [45] = {.lex_state = 543},
  [46] = {.lex_state = 543},
  [47] = {.lex_state = 543},
  [48] = {.lex_state = 2},
  [49] = {.lex_state = 543},
  [50] = {.lex_state = 543},
  [51] = {.lex_state = 543},
  [52] = {.lex_state = 2},
  [53] = {.lex_state = 543},
  [54] = {.lex_state = 0},
  [55] = {.lex_state = 0},
  [56] = {.lex_state = 0},
  [57] = {.lex_state = 0},
  [58] = {.lex_state = 0},
  [59] = {.lex_state = 0},
  [60] = {.lex_state = 0},
  [61] = {.lex_state = 0},
  [62] = {.lex_state = 0},
  [63] = {.lex_state = 2},
  [64] = {.lex_state = 2},
  [65] = {.lex_state = 0},
  [66] = {.lex_state = 0},
  [67] = {.lex_state = 0},
  [68] = {.lex_state = 0},
  [69] = {.lex_state = 0},
  [70] = {.lex_state = 0},
  [71] = {.lex_state = 0},
  [72] = {.lex_state = 0},
  [73] = {.lex_state = 0},
  [74] = {.lex_state = 0},
  [75] = {.lex_state = 1},
  [76] = {.lex_state = 0},
  [77] = {.lex_state = 11},
  [78] = {.lex_state = 0},
  [79] = {.lex_state = 11},
  [80] = {.lex_state = 0},
  [81] = {.lex_state = 1},
  [82] = {.lex_state = 0},
  [83] = {.lex_state = 11},
  [84] = {.lex_state = 1},
  [85] = {.lex_state = 0},
  [86] = {.lex_state = 0},
  [87] = {.lex_state = 0},
  [88] = {.lex_state = 11},
  [89] = {.lex_state = 11},
  [90] = {.lex_state = 0},
  [91] = {.lex_state = 543},
  [92] = {.lex_state = 543},
  [93] = {.lex_state = 0},
  [94] = {.lex_state = 543},
  [95] = {.lex_state = 0},
  [96] = {.lex_state = 0},
  [97] = {.lex_state = 0},
  [98] = {.lex_state = 0},
  [99] = {.lex_state = 8},
  [100] = {.lex_state = 8},
  [101] = {.lex_state = 0},
  [102] = {.lex_state = 0},
  [103] = {.lex_state = 8},
  [104] = {.lex_state = 11},
  [105] = {.lex_state = 0},
  [106] = {.lex_state = 0},
  [107] = {.lex_state = 0},
  [108] = {.lex_state = 0},
  [109] = {.lex_state = 1},
  [110] = {.lex_state = 1},
  [111] = {.lex_state = 1},
  [112] = {.lex_state = 0},
  [113] = {.lex_state = 543},
  [114] = {.lex_state = 12},
  [115] = {.lex_state = 0},
  [116] = {.lex_state = 543},
  [117] = {.lex_state = 1},
  [118] = {.lex_state = 1},
  [119] = {.lex_state = 0},
  [120] = {.lex_state = 0},
  [121] = {.lex_state = 0},
  [122] = {.lex_state = 1},
  [123] = {.lex_state = 0},
  [124] = {.lex_state = 0},
  [125] = {.lex_state = 0},
  [126] = {.lex_state = 0},
  [127] = {.lex_state = 0},
  [128] = {.lex_state = 8},
  [129] = {.lex_state = 1},
  [130] = {.lex_state = 0},
  [131] = {.lex_state = 0},
  [132] = {.lex_state = 0},
  [133] = {.lex_state = 0},
  [134] = {.lex_state = 0},
  [135] = {.lex_state = 0},
  [136] = {.lex_state = 0},
  [137] = {.lex_state = 8},
  [138] = {.lex_state = 1},
  [139] = {.lex_state = 1},
  [140] = {.lex_state = 0},
  [141] = {.lex_state = 0},
  [142] = {.lex_state = 1},
  [143] = {.lex_state = 0},
  [144] = {.lex_state = 0},
  [145] = {.lex_state = 0},
  [146] = {.lex_state = 0},
  [147] = {.lex_state = 0},
  [148] = {.lex_state = 0},
  [149] = {.lex_state = 0},
  [150] = {.lex_state = 0},
  [151] = {.lex_state = 0},
  [152] = {.lex_state = 1},
  [153] = {.lex_state = 1},
  [154] = {.lex_state = 0},
  [155] = {.lex_state = 9},
  [156] = {.lex_state = 1},
  [157] = {.lex_state = 0},
  [158] = {.lex_state = 1},
  [159] = {.lex_state = 9},
  [160] = {.lex_state = 543},
  [161] = {.lex_state = 9},
  [162] = {.lex_state = 9},
  [163] = {.lex_state = 0},
  [164] = {.lex_state = 0},
  [165] = {.lex_state = 1},
  [166] = {.lex_state = 1},
  [167] = {.lex_state = 1},
  [168] = {.lex_state = 0},
  [169] = {.lex_state = 0},
  [170] = {.lex_state = 0},
  [171] = {.lex_state = 0},
  [172] = {.lex_state = 1},
  [173] = {.lex_state = 0},
  [174] = {.lex_state = 9},
  [175] = {.lex_state = 0},
  [176] = {.lex_state = 1},
  [177] = {.lex_state = 0},
  [178] = {.lex_state = 1},
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
    [anon_sym_repo] = ACTIONS(1),
    [anon_sym_document] = ACTIONS(1),
    [anon_sym_true] = ACTIONS(1),
    [anon_sym_false] = ACTIONS(1),
    [sym_string] = ACTIONS(1),
    [sym_raw_string] = ACTIONS(1),
    [sym_float] = ACTIONS(1),
    [sym_integer] = ACTIONS(1),
  },
  [1] = {
    [sym_source_file] = STATE(163),
    [sym__definition] = STATE(19),
    [sym_include_declaration] = STATE(19),
    [sym_client_declaration] = STATE(19),
    [sym_vars_block] = STATE(19),
    [sym_tier_alias_declaration] = STATE(19),
    [sym_prompt_declaration] = STATE(19),
    [sym_agent_declaration] = STATE(19),
    [sym_workflow_declaration] = STATE(19),
    [aux_sym_source_file_repeat1] = STATE(19),
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
    ACTIONS(21), 39,
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
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [49] = 2,
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
  [81] = 2,
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
  [113] = 3,
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
  [146] = 3,
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
  [179] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(37), 24,
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
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [209] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(39), 24,
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
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [239] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(41), 22,
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
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [267] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(43), 22,
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
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [295] = 16,
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
    STATE(12), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(31), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [348] = 16,
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
    STATE(13), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
    STATE(31), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [401] = 16,
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
    STATE(31), 4,
      sym_vars_block,
      sym_scope_block,
      sym_memory_block,
      sym_context_block,
  [454] = 2,
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
  [477] = 12,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(65), 1,
      anon_sym_memory,
    ACTIONS(111), 1,
      anon_sym_RBRACE,
    ACTIONS(115), 1,
      anon_sym_verify,
    ACTIONS(117), 1,
      anon_sym_steps,
    ACTIONS(119), 1,
      anon_sym_execution_mode,
    ACTIONS(121), 1,
      anon_sym_strategy,
    ACTIONS(123), 1,
      anon_sym_test_first,
    ACTIONS(125), 1,
      anon_sym_param,
    STATE(17), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(41), 3,
      sym_memory_block,
      sym_verify_block,
      sym_param_declaration,
    ACTIONS(113), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [520] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(127), 1,
      ts_builtin_sym_end,
    ACTIONS(129), 1,
      anon_sym_include,
    ACTIONS(132), 1,
      anon_sym_client,
    ACTIONS(135), 1,
      anon_sym_tier,
    ACTIONS(138), 1,
      anon_sym_vars,
    ACTIONS(141), 1,
      anon_sym_prompt,
    ACTIONS(144), 1,
      anon_sym_agent,
    ACTIONS(147), 1,
      anon_sym_workflow,
    STATE(16), 9,
      sym__definition,
      sym_include_declaration,
      sym_client_declaration,
      sym_vars_block,
      sym_tier_alias_declaration,
      sym_prompt_declaration,
      sym_agent_declaration,
      sym_workflow_declaration,
      aux_sym_source_file_repeat1,
  [559] = 12,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(150), 1,
      anon_sym_RBRACE,
    ACTIONS(155), 1,
      anon_sym_memory,
    ACTIONS(158), 1,
      anon_sym_verify,
    ACTIONS(161), 1,
      anon_sym_steps,
    ACTIONS(164), 1,
      anon_sym_execution_mode,
    ACTIONS(167), 1,
      anon_sym_strategy,
    ACTIONS(170), 1,
      anon_sym_test_first,
    ACTIONS(173), 1,
      anon_sym_param,
    STATE(17), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(41), 3,
      sym_memory_block,
      sym_verify_block,
      sym_param_declaration,
    ACTIONS(152), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [602] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(176), 17,
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
  [625] = 10,
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
    ACTIONS(178), 1,
      ts_builtin_sym_end,
    STATE(16), 9,
      sym__definition,
      sym_include_declaration,
      sym_client_declaration,
      sym_vars_block,
      sym_tier_alias_declaration,
      sym_prompt_declaration,
      sym_agent_declaration,
      sym_workflow_declaration,
      aux_sym_source_file_repeat1,
  [664] = 12,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(65), 1,
      anon_sym_memory,
    ACTIONS(115), 1,
      anon_sym_verify,
    ACTIONS(117), 1,
      anon_sym_steps,
    ACTIONS(119), 1,
      anon_sym_execution_mode,
    ACTIONS(121), 1,
      anon_sym_strategy,
    ACTIONS(123), 1,
      anon_sym_test_first,
    ACTIONS(125), 1,
      anon_sym_param,
    ACTIONS(180), 1,
      anon_sym_RBRACE,
    STATE(15), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
    STATE(41), 3,
      sym_memory_block,
      sym_verify_block,
      sym_param_declaration,
    ACTIONS(113), 4,
      anon_sym_max_retries,
      anon_sym_max_parallel,
      anon_sym_attempts,
      anon_sym_escalate_after,
  [707] = 12,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(182), 1,
      anon_sym_RBRACE,
    ACTIONS(184), 1,
      anon_sym_agents,
    ACTIONS(187), 1,
      anon_sym_reviewers,
    ACTIONS(193), 1,
      anon_sym_consensus_mode,
    ACTIONS(199), 1,
      anon_sym_strict_judge,
    ACTIONS(202), 1,
      anon_sym_branch_chain,
    ACTIONS(205), 1,
      anon_sym_until,
    STATE(32), 1,
      sym_until_clause,
    ACTIONS(190), 2,
      anon_sym_template_init,
      anon_sym_template_refine,
    STATE(21), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(196), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [749] = 12,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(208), 1,
      anon_sym_RBRACE,
    ACTIONS(210), 1,
      anon_sym_agents,
    ACTIONS(212), 1,
      anon_sym_reviewers,
    ACTIONS(216), 1,
      anon_sym_consensus_mode,
    ACTIONS(220), 1,
      anon_sym_strict_judge,
    ACTIONS(222), 1,
      anon_sym_branch_chain,
    ACTIONS(224), 1,
      anon_sym_until,
    STATE(32), 1,
      sym_until_clause,
    ACTIONS(214), 2,
      anon_sym_template_init,
      anon_sym_template_refine,
    STATE(24), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(218), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [791] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(226), 1,
      anon_sym_LBRACE,
    ACTIONS(230), 1,
      anon_sym_LBRACK,
    STATE(51), 2,
      sym_reviewer_list,
      sym_param_client_block,
    ACTIONS(228), 12,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [819] = 12,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(210), 1,
      anon_sym_agents,
    ACTIONS(212), 1,
      anon_sym_reviewers,
    ACTIONS(216), 1,
      anon_sym_consensus_mode,
    ACTIONS(220), 1,
      anon_sym_strict_judge,
    ACTIONS(222), 1,
      anon_sym_branch_chain,
    ACTIONS(224), 1,
      anon_sym_until,
    ACTIONS(232), 1,
      anon_sym_RBRACE,
    STATE(32), 1,
      sym_until_clause,
    ACTIONS(214), 2,
      anon_sym_template_init,
      anon_sym_template_refine,
    STATE(21), 2,
      sym_loop_field,
      aux_sym_loop_block_repeat1,
    ACTIONS(218), 4,
      anon_sym_max_iterations,
      anon_sym_iter_start,
      anon_sym_stability,
      anon_sym_judge_timeout,
  [861] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(234), 13,
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
  [880] = 2,
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
  [899] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(238), 13,
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
    ACTIONS(240), 13,
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
    ACTIONS(242), 13,
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
  [975] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(246), 13,
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
  [994] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(248), 13,
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
  [1013] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(250), 13,
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
    ACTIONS(252), 13,
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
    ACTIONS(254), 13,
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
    ACTIONS(256), 13,
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
    ACTIONS(258), 13,
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
    ACTIONS(260), 13,
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
    ACTIONS(262), 13,
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
  [1146] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(264), 13,
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
  [1165] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(266), 12,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [1183] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(268), 12,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [1201] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(270), 12,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [1219] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(272), 12,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [1237] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(274), 12,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [1255] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(276), 12,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [1273] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(278), 12,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [1291] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(284), 1,
      sym_identifier,
    ACTIONS(282), 2,
      sym_string,
      sym_raw_string,
    STATE(107), 3,
      sym__effort_value,
      sym_tier_value,
      sym__string_value,
    ACTIONS(280), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [1315] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(286), 12,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [1333] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(288), 12,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [1351] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(290), 12,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [1369] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(294), 1,
      sym_identifier,
    ACTIONS(292), 2,
      sym_string,
      sym_raw_string,
    STATE(87), 3,
      sym__effort_value,
      sym_tier_value,
      sym__string_value,
    ACTIONS(280), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [1393] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(296), 12,
      anon_sym_RBRACE,
      anon_sym_max_retries,
      anon_sym_memory,
      anon_sym_verify,
      anon_sym_steps,
      anon_sym_max_parallel,
      anon_sym_execution_mode,
      anon_sym_strategy,
      anon_sym_test_first,
      anon_sym_attempts,
      anon_sym_escalate_after,
      anon_sym_param,
  [1411] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(298), 1,
      anon_sym_RBRACE,
    ACTIONS(304), 1,
      anon_sym_importance,
    ACTIONS(306), 1,
      anon_sym_read_limit,
    ACTIONS(300), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(56), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(302), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [1437] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(308), 1,
      anon_sym_RBRACE,
    ACTIONS(310), 1,
      anon_sym_tier,
    ACTIONS(312), 1,
      anon_sym_model,
    ACTIONS(314), 1,
      anon_sym_effort,
    ACTIONS(316), 1,
      anon_sym_privacy,
    ACTIONS(318), 1,
      anon_sym_default,
    ACTIONS(320), 1,
      anon_sym_extra,
    STATE(78), 1,
      sym_extra_block,
    STATE(57), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [1469] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(304), 1,
      anon_sym_importance,
    ACTIONS(306), 1,
      anon_sym_read_limit,
    ACTIONS(322), 1,
      anon_sym_RBRACE,
    ACTIONS(300), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(61), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(302), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [1495] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(324), 1,
      anon_sym_RBRACE,
    ACTIONS(326), 1,
      anon_sym_tier,
    ACTIONS(329), 1,
      anon_sym_model,
    ACTIONS(332), 1,
      anon_sym_effort,
    ACTIONS(335), 1,
      anon_sym_privacy,
    ACTIONS(338), 1,
      anon_sym_default,
    ACTIONS(341), 1,
      anon_sym_extra,
    STATE(78), 1,
      sym_extra_block,
    STATE(57), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [1527] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(310), 1,
      anon_sym_tier,
    ACTIONS(312), 1,
      anon_sym_model,
    ACTIONS(314), 1,
      anon_sym_effort,
    ACTIONS(316), 1,
      anon_sym_privacy,
    ACTIONS(318), 1,
      anon_sym_default,
    ACTIONS(320), 1,
      anon_sym_extra,
    ACTIONS(344), 1,
      anon_sym_RBRACE,
    STATE(78), 1,
      sym_extra_block,
    STATE(55), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [1559] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(310), 1,
      anon_sym_tier,
    ACTIONS(312), 1,
      anon_sym_model,
    ACTIONS(314), 1,
      anon_sym_effort,
    ACTIONS(316), 1,
      anon_sym_privacy,
    ACTIONS(318), 1,
      anon_sym_default,
    ACTIONS(320), 1,
      anon_sym_extra,
    ACTIONS(346), 1,
      anon_sym_RBRACE,
    STATE(78), 1,
      sym_extra_block,
    STATE(60), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [1591] = 10,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(310), 1,
      anon_sym_tier,
    ACTIONS(312), 1,
      anon_sym_model,
    ACTIONS(314), 1,
      anon_sym_effort,
    ACTIONS(316), 1,
      anon_sym_privacy,
    ACTIONS(318), 1,
      anon_sym_default,
    ACTIONS(320), 1,
      anon_sym_extra,
    ACTIONS(348), 1,
      anon_sym_RBRACE,
    STATE(78), 1,
      sym_extra_block,
    STATE(57), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [1623] = 7,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(350), 1,
      anon_sym_RBRACE,
    ACTIONS(358), 1,
      anon_sym_importance,
    ACTIONS(361), 1,
      anon_sym_read_limit,
    ACTIONS(352), 2,
      anon_sym_read_ns,
      anon_sym_staleness_sources,
    STATE(61), 2,
      sym_memory_field,
      aux_sym_memory_block_repeat1,
    ACTIONS(355), 3,
      anon_sym_write_ns,
      anon_sym_read_query,
      anon_sym_write_content,
  [1649] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(364), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1663] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(27), 1,
      sym_tier_alias_name,
    ACTIONS(366), 7,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
      sym_identifier,
  [1679] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(176), 1,
      sym_tier_alias_name,
    ACTIONS(368), 7,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
      sym_identifier,
  [1695] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(370), 8,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
      anon_sym_id,
  [1709] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(372), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1723] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(374), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1737] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(376), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1751] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(378), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1765] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(380), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1779] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(382), 8,
      anon_sym_RBRACE,
      anon_sym_read_ns,
      anon_sym_write_ns,
      anon_sym_importance,
      anon_sym_staleness_sources,
      anon_sym_read_query,
      anon_sym_read_limit,
      anon_sym_write_content,
  [1793] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(384), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1807] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(386), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1821] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(388), 8,
      ts_builtin_sym_end,
      anon_sym_include,
      anon_sym_client,
      anon_sym_tier,
      anon_sym_vars,
      anon_sym_prompt,
      anon_sym_agent,
      anon_sym_workflow,
  [1835] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(390), 1,
      anon_sym_RBRACE,
    STATE(130), 1,
      sym__string_value,
    STATE(81), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(392), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1854] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(394), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1867] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(396), 1,
      anon_sym_RBRACE,
    STATE(79), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(398), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1884] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(400), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1897] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(402), 1,
      anon_sym_RBRACE,
    STATE(83), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(398), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1914] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(404), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [1927] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(406), 1,
      anon_sym_RBRACE,
    STATE(130), 1,
      sym__string_value,
    STATE(81), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(408), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [1946] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(411), 1,
      anon_sym_LBRACE,
    ACTIONS(413), 1,
      anon_sym_agent,
    ACTIONS(415), 1,
      anon_sym_command,
    STATE(36), 4,
      sym__until_condition,
      sym_until_verify,
      sym_until_agent,
      sym_until_command,
  [1965] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(417), 1,
      anon_sym_RBRACE,
    STATE(83), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(419), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [1982] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(422), 1,
      anon_sym_RBRACE,
    STATE(130), 1,
      sym__string_value,
    STATE(75), 2,
      sym_extra_pair,
      aux_sym_extra_block_repeat1,
    ACTIONS(392), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [2001] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(424), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [2014] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(87), 1,
      sym_tier_value,
    ACTIONS(426), 6,
      anon_sym_cheap,
      anon_sym_expensive,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [2029] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(428), 7,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_privacy,
      anon_sym_default,
      anon_sym_extra,
  [2042] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(430), 1,
      anon_sym_RBRACE,
    STATE(89), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(398), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [2059] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(432), 1,
      anon_sym_RBRACE,
    STATE(83), 2,
      sym_verify_field,
      aux_sym_verify_block_repeat1,
    ACTIONS(398), 4,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [2076] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(434), 1,
      anon_sym_RBRACE,
    ACTIONS(438), 1,
      anon_sym_impact_scope,
    ACTIONS(436), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(93), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [2094] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(440), 1,
      anon_sym_RBRACE,
    ACTIONS(444), 1,
      anon_sym_depth,
    ACTIONS(442), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(92), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [2112] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(444), 1,
      anon_sym_depth,
    ACTIONS(446), 1,
      anon_sym_RBRACE,
    ACTIONS(442), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(94), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [2130] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(448), 1,
      anon_sym_RBRACE,
    ACTIONS(453), 1,
      anon_sym_impact_scope,
    ACTIONS(450), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(93), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [2148] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(456), 1,
      anon_sym_RBRACE,
    ACTIONS(461), 1,
      anon_sym_depth,
    ACTIONS(458), 2,
      anon_sym_callers_of,
      anon_sym_tests_for,
    STATE(94), 2,
      sym_context_field,
      aux_sym_context_block_repeat1,
  [2166] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(464), 1,
      anon_sym_RBRACE,
    ACTIONS(469), 1,
      anon_sym_effort,
    ACTIONS(466), 2,
      anon_sym_model,
      anon_sym_id,
    STATE(95), 2,
      sym_reviewer_field,
      aux_sym_reviewer_entry_repeat1,
  [2184] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(472), 1,
      anon_sym_RBRACE,
    ACTIONS(476), 1,
      anon_sym_effort,
    ACTIONS(474), 2,
      anon_sym_model,
      anon_sym_id,
    STATE(97), 2,
      sym_reviewer_field,
      aux_sym_reviewer_entry_repeat1,
  [2202] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(476), 1,
      anon_sym_effort,
    ACTIONS(478), 1,
      anon_sym_RBRACE,
    ACTIONS(474), 2,
      anon_sym_model,
      anon_sym_id,
    STATE(95), 2,
      sym_reviewer_field,
      aux_sym_reviewer_entry_repeat1,
  [2220] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(438), 1,
      anon_sym_impact_scope,
    ACTIONS(480), 1,
      anon_sym_RBRACE,
    ACTIONS(436), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(90), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [2238] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(482), 1,
      anon_sym_loop,
    ACTIONS(484), 1,
      anon_sym_RBRACK,
    ACTIONS(486), 1,
      sym_identifier,
    STATE(100), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [2255] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(482), 1,
      anon_sym_loop,
    ACTIONS(488), 1,
      anon_sym_RBRACK,
    ACTIONS(490), 1,
      sym_identifier,
    STATE(103), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [2272] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(492), 1,
      anon_sym_RBRACK,
    ACTIONS(494), 2,
      sym_string,
      sym_raw_string,
    STATE(102), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [2287] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(496), 1,
      anon_sym_RBRACK,
    ACTIONS(498), 2,
      sym_string,
      sym_raw_string,
    STATE(102), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [2302] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(501), 1,
      anon_sym_loop,
    ACTIONS(504), 1,
      anon_sym_RBRACK,
    ACTIONS(506), 1,
      sym_identifier,
    STATE(103), 2,
      sym_loop_block,
      aux_sym_step_list_repeat1,
  [2319] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(509), 5,
      anon_sym_RBRACE,
      anon_sym_compile,
      anon_sym_clippy,
      anon_sym_test,
      anon_sym_impact_tests,
  [2330] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(511), 1,
      anon_sym_RBRACK,
    ACTIONS(513), 2,
      sym_string,
      sym_raw_string,
    STATE(101), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [2345] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(515), 1,
      anon_sym_LBRACE,
    ACTIONS(518), 1,
      anon_sym_RBRACK,
    STATE(106), 2,
      sym_reviewer_entry,
      aux_sym_reviewer_list_repeat1,
  [2359] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(520), 4,
      anon_sym_RBRACE,
      anon_sym_model,
      anon_sym_effort,
      anon_sym_id,
  [2369] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(522), 1,
      anon_sym_LBRACE,
    ACTIONS(524), 1,
      anon_sym_RBRACK,
    STATE(115), 2,
      sym_reviewer_entry,
      aux_sym_reviewer_list_repeat1,
  [2383] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(526), 4,
      anon_sym_RBRACE,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [2393] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(528), 1,
      anon_sym_RBRACE,
    ACTIONS(530), 1,
      sym_identifier,
    STATE(117), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [2407] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(532), 1,
      anon_sym_RBRACE,
    ACTIONS(534), 1,
      sym_identifier,
    STATE(111), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [2421] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(537), 4,
      anon_sym_RBRACE,
      anon_sym_owned,
      anon_sym_read_only,
      anon_sym_impact_scope,
  [2431] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(539), 4,
      anon_sym_RBRACE,
      anon_sym_callers_of,
      anon_sym_tests_for,
      anon_sym_depth,
  [2441] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(42), 1,
      sym_strategy_value,
    ACTIONS(541), 3,
      anon_sym_single_pass,
      anon_sym_refine,
      sym_identifier,
  [2453] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(522), 1,
      anon_sym_LBRACE,
    ACTIONS(543), 1,
      anon_sym_RBRACK,
    STATE(106), 2,
      sym_reviewer_entry,
      aux_sym_reviewer_list_repeat1,
  [2467] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(33), 1,
      sym_consensus_mode_value,
    ACTIONS(545), 3,
      anon_sym_strict,
      anon_sym_partial_ok,
      anon_sym_explore,
  [2479] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(530), 1,
      sym_identifier,
    ACTIONS(547), 1,
      anon_sym_RBRACE,
    STATE(111), 2,
      sym_vars_pair,
      aux_sym_vars_block_repeat1,
  [2493] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(27), 1,
      sym__string_value,
    ACTIONS(549), 3,
      sym_string,
      sym_raw_string,
      sym_identifier,
  [2505] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(87), 1,
      sym__string_value,
    ACTIONS(292), 2,
      sym_string,
      sym_raw_string,
  [2516] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(67), 1,
      sym__string_value,
    ACTIONS(551), 2,
      sym_string,
      sym_raw_string,
  [2527] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(87), 1,
      sym_privacy_value,
    ACTIONS(553), 2,
      anon_sym_public,
      anon_sym_local_only,
  [2538] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(555), 1,
      anon_sym_RBRACK,
    ACTIONS(557), 1,
      sym_identifier,
    STATE(139), 1,
      aux_sym_identifier_list_repeat1,
  [2551] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(66), 1,
      sym__string_value,
    ACTIONS(559), 2,
      sym_string,
      sym_raw_string,
  [2562] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(42), 1,
      sym_execution_mode_value,
    ACTIONS(561), 2,
      anon_sym_repo,
      anon_sym_document,
  [2573] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(27), 1,
      sym_boolean,
    ACTIONS(563), 2,
      anon_sym_true,
      anon_sym_false,
  [2584] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(42), 1,
      sym_boolean,
    ACTIONS(563), 2,
      anon_sym_true,
      anon_sym_false,
  [2595] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(112), 1,
      sym_boolean,
    ACTIONS(563), 2,
      anon_sym_true,
      anon_sym_false,
  [2606] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(567), 1,
      anon_sym_RBRACK,
    ACTIONS(565), 2,
      anon_sym_loop,
      sym_identifier,
  [2617] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(230), 1,
      anon_sym_LBRACK,
    ACTIONS(569), 1,
      sym_identifier,
    STATE(33), 1,
      sym_reviewer_list,
  [2630] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(109), 1,
      sym__string_value,
    ACTIONS(571), 2,
      sym_string,
      sym_raw_string,
  [2641] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(33), 1,
      sym_boolean,
    ACTIONS(563), 2,
      anon_sym_true,
      anon_sym_false,
  [2652] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(33), 1,
      sym_branch_chain_value,
    ACTIONS(573), 2,
      anon_sym_stacked,
      anon_sym_none,
  [2663] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(142), 1,
      sym__string_value,
    ACTIONS(575), 2,
      sym_string,
      sym_raw_string,
  [2674] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(107), 1,
      sym__string_value,
    ACTIONS(282), 2,
      sym_string,
      sym_raw_string,
  [2685] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(71), 1,
      sym__string_value,
    ACTIONS(577), 2,
      sym_string,
      sym_raw_string,
  [2696] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(39), 1,
      sym__string_value,
    ACTIONS(579), 2,
      sym_string,
      sym_raw_string,
  [2707] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(583), 1,
      anon_sym_RBRACK,
    ACTIONS(581), 2,
      anon_sym_loop,
      sym_identifier,
  [2718] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(585), 1,
      anon_sym_RBRACK,
    ACTIONS(587), 1,
      sym_identifier,
    STATE(122), 1,
      aux_sym_identifier_list_repeat1,
  [2731] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(589), 1,
      anon_sym_RBRACK,
    ACTIONS(591), 1,
      sym_identifier,
    STATE(139), 1,
      aux_sym_identifier_list_repeat1,
  [2744] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(27), 1,
      sym__string_value,
    ACTIONS(549), 2,
      sym_string,
      sym_raw_string,
  [2755] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(104), 1,
      sym_boolean,
    ACTIONS(563), 2,
      anon_sym_true,
      anon_sym_false,
  [2766] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(594), 2,
      anon_sym_RBRACE,
      sym_identifier,
  [2774] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(596), 1,
      anon_sym_LBRACK,
    STATE(33), 1,
      sym_identifier_list,
  [2784] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(598), 2,
      anon_sym_LBRACE,
      anon_sym_RBRACK,
  [2792] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(600), 1,
      anon_sym_LBRACK,
    STATE(42), 1,
      sym_step_list,
  [2802] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(602), 1,
      anon_sym_LBRACK,
    STATE(71), 1,
      sym_string_list,
  [2812] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(602), 1,
      anon_sym_LBRACK,
    STATE(27), 1,
      sym_string_list,
  [2822] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(602), 1,
      anon_sym_LBRACK,
    STATE(112), 1,
      sym_string_list,
  [2832] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(604), 2,
      anon_sym_LBRACE,
      anon_sym_RBRACK,
  [2840] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(602), 1,
      anon_sym_LBRACK,
    STATE(113), 1,
      sym_string_list,
  [2850] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(596), 1,
      anon_sym_LBRACK,
    STATE(27), 1,
      sym_identifier_list,
  [2860] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(606), 1,
      sym_identifier,
  [2867] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(608), 1,
      sym_identifier,
  [2874] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(610), 1,
      anon_sym_LBRACE,
  [2881] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(612), 1,
      sym_integer,
  [2888] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(569), 1,
      sym_identifier,
  [2895] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(614), 1,
      anon_sym_LBRACE,
  [2902] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(549), 1,
      sym_identifier,
  [2909] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(569), 1,
      sym_integer,
  [2916] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(577), 1,
      sym_float,
  [2923] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(577), 1,
      sym_integer,
  [2930] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(549), 1,
      sym_integer,
  [2937] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(616), 1,
      ts_builtin_sym_end,
  [2944] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(618), 1,
      anon_sym_LBRACE,
  [2951] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(620), 1,
      sym_identifier,
  [2958] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(234), 1,
      sym_identifier,
  [2965] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(622), 1,
      sym_identifier,
  [2972] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(624), 1,
      anon_sym_LBRACE,
  [2979] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(626), 1,
      anon_sym_LBRACE,
  [2986] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(628), 1,
      anon_sym_LBRACE,
  [2993] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(630), 1,
      anon_sym_LBRACE,
  [3000] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(632), 1,
      sym_identifier,
  [3007] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(634), 1,
      anon_sym_LBRACE,
  [3014] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(636), 1,
      sym_integer,
  [3021] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(638), 1,
      anon_sym_LBRACE,
  [3028] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(640), 1,
      sym_identifier,
  [3035] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(642), 1,
      anon_sym_LBRACE,
  [3042] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(644), 1,
      sym_identifier,
};

static const uint32_t ts_small_parse_table_map[] = {
  [SMALL_STATE(2)] = 0,
  [SMALL_STATE(3)] = 49,
  [SMALL_STATE(4)] = 81,
  [SMALL_STATE(5)] = 113,
  [SMALL_STATE(6)] = 146,
  [SMALL_STATE(7)] = 179,
  [SMALL_STATE(8)] = 209,
  [SMALL_STATE(9)] = 239,
  [SMALL_STATE(10)] = 267,
  [SMALL_STATE(11)] = 295,
  [SMALL_STATE(12)] = 348,
  [SMALL_STATE(13)] = 401,
  [SMALL_STATE(14)] = 454,
  [SMALL_STATE(15)] = 477,
  [SMALL_STATE(16)] = 520,
  [SMALL_STATE(17)] = 559,
  [SMALL_STATE(18)] = 602,
  [SMALL_STATE(19)] = 625,
  [SMALL_STATE(20)] = 664,
  [SMALL_STATE(21)] = 707,
  [SMALL_STATE(22)] = 749,
  [SMALL_STATE(23)] = 791,
  [SMALL_STATE(24)] = 819,
  [SMALL_STATE(25)] = 861,
  [SMALL_STATE(26)] = 880,
  [SMALL_STATE(27)] = 899,
  [SMALL_STATE(28)] = 918,
  [SMALL_STATE(29)] = 937,
  [SMALL_STATE(30)] = 956,
  [SMALL_STATE(31)] = 975,
  [SMALL_STATE(32)] = 994,
  [SMALL_STATE(33)] = 1013,
  [SMALL_STATE(34)] = 1032,
  [SMALL_STATE(35)] = 1051,
  [SMALL_STATE(36)] = 1070,
  [SMALL_STATE(37)] = 1089,
  [SMALL_STATE(38)] = 1108,
  [SMALL_STATE(39)] = 1127,
  [SMALL_STATE(40)] = 1146,
  [SMALL_STATE(41)] = 1165,
  [SMALL_STATE(42)] = 1183,
  [SMALL_STATE(43)] = 1201,
  [SMALL_STATE(44)] = 1219,
  [SMALL_STATE(45)] = 1237,
  [SMALL_STATE(46)] = 1255,
  [SMALL_STATE(47)] = 1273,
  [SMALL_STATE(48)] = 1291,
  [SMALL_STATE(49)] = 1315,
  [SMALL_STATE(50)] = 1333,
  [SMALL_STATE(51)] = 1351,
  [SMALL_STATE(52)] = 1369,
  [SMALL_STATE(53)] = 1393,
  [SMALL_STATE(54)] = 1411,
  [SMALL_STATE(55)] = 1437,
  [SMALL_STATE(56)] = 1469,
  [SMALL_STATE(57)] = 1495,
  [SMALL_STATE(58)] = 1527,
  [SMALL_STATE(59)] = 1559,
  [SMALL_STATE(60)] = 1591,
  [SMALL_STATE(61)] = 1623,
  [SMALL_STATE(62)] = 1649,
  [SMALL_STATE(63)] = 1663,
  [SMALL_STATE(64)] = 1679,
  [SMALL_STATE(65)] = 1695,
  [SMALL_STATE(66)] = 1709,
  [SMALL_STATE(67)] = 1723,
  [SMALL_STATE(68)] = 1737,
  [SMALL_STATE(69)] = 1751,
  [SMALL_STATE(70)] = 1765,
  [SMALL_STATE(71)] = 1779,
  [SMALL_STATE(72)] = 1793,
  [SMALL_STATE(73)] = 1807,
  [SMALL_STATE(74)] = 1821,
  [SMALL_STATE(75)] = 1835,
  [SMALL_STATE(76)] = 1854,
  [SMALL_STATE(77)] = 1867,
  [SMALL_STATE(78)] = 1884,
  [SMALL_STATE(79)] = 1897,
  [SMALL_STATE(80)] = 1914,
  [SMALL_STATE(81)] = 1927,
  [SMALL_STATE(82)] = 1946,
  [SMALL_STATE(83)] = 1965,
  [SMALL_STATE(84)] = 1982,
  [SMALL_STATE(85)] = 2001,
  [SMALL_STATE(86)] = 2014,
  [SMALL_STATE(87)] = 2029,
  [SMALL_STATE(88)] = 2042,
  [SMALL_STATE(89)] = 2059,
  [SMALL_STATE(90)] = 2076,
  [SMALL_STATE(91)] = 2094,
  [SMALL_STATE(92)] = 2112,
  [SMALL_STATE(93)] = 2130,
  [SMALL_STATE(94)] = 2148,
  [SMALL_STATE(95)] = 2166,
  [SMALL_STATE(96)] = 2184,
  [SMALL_STATE(97)] = 2202,
  [SMALL_STATE(98)] = 2220,
  [SMALL_STATE(99)] = 2238,
  [SMALL_STATE(100)] = 2255,
  [SMALL_STATE(101)] = 2272,
  [SMALL_STATE(102)] = 2287,
  [SMALL_STATE(103)] = 2302,
  [SMALL_STATE(104)] = 2319,
  [SMALL_STATE(105)] = 2330,
  [SMALL_STATE(106)] = 2345,
  [SMALL_STATE(107)] = 2359,
  [SMALL_STATE(108)] = 2369,
  [SMALL_STATE(109)] = 2383,
  [SMALL_STATE(110)] = 2393,
  [SMALL_STATE(111)] = 2407,
  [SMALL_STATE(112)] = 2421,
  [SMALL_STATE(113)] = 2431,
  [SMALL_STATE(114)] = 2441,
  [SMALL_STATE(115)] = 2453,
  [SMALL_STATE(116)] = 2467,
  [SMALL_STATE(117)] = 2479,
  [SMALL_STATE(118)] = 2493,
  [SMALL_STATE(119)] = 2505,
  [SMALL_STATE(120)] = 2516,
  [SMALL_STATE(121)] = 2527,
  [SMALL_STATE(122)] = 2538,
  [SMALL_STATE(123)] = 2551,
  [SMALL_STATE(124)] = 2562,
  [SMALL_STATE(125)] = 2573,
  [SMALL_STATE(126)] = 2584,
  [SMALL_STATE(127)] = 2595,
  [SMALL_STATE(128)] = 2606,
  [SMALL_STATE(129)] = 2617,
  [SMALL_STATE(130)] = 2630,
  [SMALL_STATE(131)] = 2641,
  [SMALL_STATE(132)] = 2652,
  [SMALL_STATE(133)] = 2663,
  [SMALL_STATE(134)] = 2674,
  [SMALL_STATE(135)] = 2685,
  [SMALL_STATE(136)] = 2696,
  [SMALL_STATE(137)] = 2707,
  [SMALL_STATE(138)] = 2718,
  [SMALL_STATE(139)] = 2731,
  [SMALL_STATE(140)] = 2744,
  [SMALL_STATE(141)] = 2755,
  [SMALL_STATE(142)] = 2766,
  [SMALL_STATE(143)] = 2774,
  [SMALL_STATE(144)] = 2784,
  [SMALL_STATE(145)] = 2792,
  [SMALL_STATE(146)] = 2802,
  [SMALL_STATE(147)] = 2812,
  [SMALL_STATE(148)] = 2822,
  [SMALL_STATE(149)] = 2832,
  [SMALL_STATE(150)] = 2840,
  [SMALL_STATE(151)] = 2850,
  [SMALL_STATE(152)] = 2860,
  [SMALL_STATE(153)] = 2867,
  [SMALL_STATE(154)] = 2874,
  [SMALL_STATE(155)] = 2881,
  [SMALL_STATE(156)] = 2888,
  [SMALL_STATE(157)] = 2895,
  [SMALL_STATE(158)] = 2902,
  [SMALL_STATE(159)] = 2909,
  [SMALL_STATE(160)] = 2916,
  [SMALL_STATE(161)] = 2923,
  [SMALL_STATE(162)] = 2930,
  [SMALL_STATE(163)] = 2937,
  [SMALL_STATE(164)] = 2944,
  [SMALL_STATE(165)] = 2951,
  [SMALL_STATE(166)] = 2958,
  [SMALL_STATE(167)] = 2965,
  [SMALL_STATE(168)] = 2972,
  [SMALL_STATE(169)] = 2979,
  [SMALL_STATE(170)] = 2986,
  [SMALL_STATE(171)] = 2993,
  [SMALL_STATE(172)] = 3000,
  [SMALL_STATE(173)] = 3007,
  [SMALL_STATE(174)] = 3014,
  [SMALL_STATE(175)] = 3021,
  [SMALL_STATE(176)] = 3028,
  [SMALL_STATE(177)] = 3035,
  [SMALL_STATE(178)] = 3042,
};

static const TSParseActionEntry ts_parse_actions[] = {
  [0] = {.entry = {.count = 0, .reusable = false}},
  [1] = {.entry = {.count = 1, .reusable = false}}, RECOVER(),
  [3] = {.entry = {.count = 1, .reusable = true}}, SHIFT_EXTRA(),
  [5] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 0, 0, 0),
  [7] = {.entry = {.count = 1, .reusable = true}}, SHIFT(120),
  [9] = {.entry = {.count = 1, .reusable = true}}, SHIFT(167),
  [11] = {.entry = {.count = 1, .reusable = true}}, SHIFT(64),
  [13] = {.entry = {.count = 1, .reusable = true}}, SHIFT(169),
  [15] = {.entry = {.count = 1, .reusable = true}}, SHIFT(152),
  [17] = {.entry = {.count = 1, .reusable = true}}, SHIFT(178),
  [19] = {.entry = {.count = 1, .reusable = true}}, SHIFT(153),
  [21] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_boolean, 1, 0, 0),
  [23] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_boolean, 1, 0, 0),
  [25] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 3, 0, 0),
  [27] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 2, 0, 0),
  [29] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 2, 0, 0),
  [31] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_identifier_list, 2, 0, 0),
  [33] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 3, 0, 0),
  [35] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_identifier_list, 3, 0, 0),
  [37] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_list, 3, 0, 0),
  [39] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_list, 2, 0, 0),
  [41] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_block, 4, 0, 0),
  [43] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_block, 3, 0, 0),
  [45] = {.entry = {.count = 1, .reusable = true}}, SHIFT(158),
  [47] = {.entry = {.count = 1, .reusable = true}}, SHIFT(62),
  [49] = {.entry = {.count = 1, .reusable = true}}, SHIFT(63),
  [51] = {.entry = {.count = 1, .reusable = true}}, SHIFT(118),
  [53] = {.entry = {.count = 1, .reusable = true}}, SHIFT(140),
  [55] = {.entry = {.count = 1, .reusable = true}}, SHIFT(151),
  [57] = {.entry = {.count = 1, .reusable = true}}, SHIFT(162),
  [59] = {.entry = {.count = 1, .reusable = true}}, SHIFT(147),
  [61] = {.entry = {.count = 1, .reusable = true}}, SHIFT(125),
  [63] = {.entry = {.count = 1, .reusable = true}}, SHIFT(164),
  [65] = {.entry = {.count = 1, .reusable = true}}, SHIFT(170),
  [67] = {.entry = {.count = 1, .reusable = true}}, SHIFT(154),
  [69] = {.entry = {.count = 1, .reusable = true}}, SHIFT(72),
  [71] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(158),
  [74] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0),
  [76] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(63),
  [79] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(169),
  [82] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(118),
  [85] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(140),
  [88] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(151),
  [91] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(162),
  [94] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(147),
  [97] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(125),
  [100] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(164),
  [103] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(170),
  [106] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(154),
  [109] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_block, 3, 0, 0),
  [111] = {.entry = {.count = 1, .reusable = true}}, SHIFT(73),
  [113] = {.entry = {.count = 1, .reusable = true}}, SHIFT(155),
  [115] = {.entry = {.count = 1, .reusable = true}}, SHIFT(171),
  [117] = {.entry = {.count = 1, .reusable = true}}, SHIFT(145),
  [119] = {.entry = {.count = 1, .reusable = true}}, SHIFT(124),
  [121] = {.entry = {.count = 1, .reusable = true}}, SHIFT(114),
  [123] = {.entry = {.count = 1, .reusable = true}}, SHIFT(126),
  [125] = {.entry = {.count = 1, .reusable = true}}, SHIFT(172),
  [127] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0),
  [129] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(120),
  [132] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(167),
  [135] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(64),
  [138] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(169),
  [141] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(152),
  [144] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(178),
  [147] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(153),
  [150] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0),
  [152] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(155),
  [155] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(170),
  [158] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(171),
  [161] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(145),
  [164] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(124),
  [167] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(114),
  [170] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(126),
  [173] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(172),
  [176] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_block, 4, 0, 0),
  [178] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 1, 0, 0),
  [180] = {.entry = {.count = 1, .reusable = true}}, SHIFT(70),
  [182] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0),
  [184] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(143),
  [187] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(129),
  [190] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(156),
  [193] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(116),
  [196] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(159),
  [199] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(131),
  [202] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(132),
  [205] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_loop_block_repeat1, 2, 0, 0), SHIFT_REPEAT(82),
  [208] = {.entry = {.count = 1, .reusable = true}}, SHIFT(128),
  [210] = {.entry = {.count = 1, .reusable = true}}, SHIFT(143),
  [212] = {.entry = {.count = 1, .reusable = true}}, SHIFT(129),
  [214] = {.entry = {.count = 1, .reusable = true}}, SHIFT(156),
  [216] = {.entry = {.count = 1, .reusable = true}}, SHIFT(116),
  [218] = {.entry = {.count = 1, .reusable = true}}, SHIFT(159),
  [220] = {.entry = {.count = 1, .reusable = true}}, SHIFT(131),
  [222] = {.entry = {.count = 1, .reusable = true}}, SHIFT(132),
  [224] = {.entry = {.count = 1, .reusable = true}}, SHIFT(82),
  [226] = {.entry = {.count = 1, .reusable = true}}, SHIFT(58),
  [228] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_param_declaration, 2, 0, 0),
  [230] = {.entry = {.count = 1, .reusable = true}}, SHIFT(108),
  [232] = {.entry = {.count = 1, .reusable = true}}, SHIFT(137),
  [234] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_alias_name, 1, 0, 0),
  [236] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_verify, 3, 0, 0),
  [238] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 2, 0, 0),
  [240] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 3, 0, 0),
  [242] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_block, 3, 0, 0),
  [244] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 4, 0, 0),
  [246] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 1, 0, 0),
  [248] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_field, 1, 0, 0),
  [250] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_field, 2, 0, 0),
  [252] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_consensus_mode_value, 1, 0, 0),
  [254] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_branch_chain_value, 1, 0, 0),
  [256] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_clause, 2, 0, 0),
  [258] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_verify, 2, 0, 0),
  [260] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_agent, 2, 0, 0),
  [262] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_until_command, 2, 0, 0),
  [264] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_block, 4, 0, 0),
  [266] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 1, 0, 0),
  [268] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 2, 0, 0),
  [270] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_block, 4, 0, 0),
  [272] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_param_client_block, 2, 0, 0),
  [274] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_step_list, 3, 0, 0),
  [276] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_execution_mode_value, 1, 0, 0),
  [278] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_param_client_block, 3, 0, 0),
  [280] = {.entry = {.count = 1, .reusable = false}}, SHIFT(65),
  [282] = {.entry = {.count = 1, .reusable = true}}, SHIFT(107),
  [284] = {.entry = {.count = 1, .reusable = false}}, SHIFT(107),
  [286] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_block, 3, 0, 0),
  [288] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_step_list, 2, 0, 0),
  [290] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_param_declaration, 3, 0, 0),
  [292] = {.entry = {.count = 1, .reusable = true}}, SHIFT(87),
  [294] = {.entry = {.count = 1, .reusable = false}}, SHIFT(87),
  [296] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_strategy_value, 1, 0, 0),
  [298] = {.entry = {.count = 1, .reusable = true}}, SHIFT(10),
  [300] = {.entry = {.count = 1, .reusable = true}}, SHIFT(146),
  [302] = {.entry = {.count = 1, .reusable = true}}, SHIFT(135),
  [304] = {.entry = {.count = 1, .reusable = true}}, SHIFT(160),
  [306] = {.entry = {.count = 1, .reusable = true}}, SHIFT(161),
  [308] = {.entry = {.count = 1, .reusable = true}}, SHIFT(47),
  [310] = {.entry = {.count = 1, .reusable = true}}, SHIFT(86),
  [312] = {.entry = {.count = 1, .reusable = true}}, SHIFT(119),
  [314] = {.entry = {.count = 1, .reusable = true}}, SHIFT(52),
  [316] = {.entry = {.count = 1, .reusable = true}}, SHIFT(121),
  [318] = {.entry = {.count = 1, .reusable = true}}, SHIFT(78),
  [320] = {.entry = {.count = 1, .reusable = true}}, SHIFT(173),
  [322] = {.entry = {.count = 1, .reusable = true}}, SHIFT(9),
  [324] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0),
  [326] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(86),
  [329] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(119),
  [332] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(52),
  [335] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(121),
  [338] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(78),
  [341] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(173),
  [344] = {.entry = {.count = 1, .reusable = true}}, SHIFT(44),
  [346] = {.entry = {.count = 1, .reusable = true}}, SHIFT(68),
  [348] = {.entry = {.count = 1, .reusable = true}}, SHIFT(69),
  [350] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0),
  [352] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(146),
  [355] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(135),
  [358] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(160),
  [361] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_memory_block_repeat1, 2, 0, 0), SHIFT_REPEAT(161),
  [364] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 4, 0, 0),
  [366] = {.entry = {.count = 1, .reusable = false}}, SHIFT(25),
  [368] = {.entry = {.count = 1, .reusable = false}}, SHIFT(166),
  [370] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_value, 1, 0, 0),
  [372] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_prompt_declaration, 3, 0, 0),
  [374] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_include_declaration, 2, 0, 0),
  [376] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 4, 0, 0),
  [378] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 5, 0, 0),
  [380] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 4, 0, 0),
  [382] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_memory_field, 2, 0, 0),
  [384] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 5, 0, 0),
  [386] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 5, 0, 0),
  [388] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_alias_declaration, 3, 0, 0),
  [390] = {.entry = {.count = 1, .reusable = true}}, SHIFT(80),
  [392] = {.entry = {.count = 1, .reusable = true}}, SHIFT(130),
  [394] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_block, 3, 0, 0),
  [396] = {.entry = {.count = 1, .reusable = true}}, SHIFT(49),
  [398] = {.entry = {.count = 1, .reusable = true}}, SHIFT(141),
  [400] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_field, 1, 0, 0),
  [402] = {.entry = {.count = 1, .reusable = true}}, SHIFT(43),
  [404] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_block, 4, 0, 0),
  [406] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_extra_block_repeat1, 2, 0, 0),
  [408] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_extra_block_repeat1, 2, 0, 0), SHIFT_REPEAT(130),
  [411] = {.entry = {.count = 1, .reusable = true}}, SHIFT(88),
  [413] = {.entry = {.count = 1, .reusable = true}}, SHIFT(165),
  [415] = {.entry = {.count = 1, .reusable = true}}, SHIFT(136),
  [417] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_verify_block_repeat1, 2, 0, 0),
  [419] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_verify_block_repeat1, 2, 0, 0), SHIFT_REPEAT(141),
  [422] = {.entry = {.count = 1, .reusable = true}}, SHIFT(76),
  [424] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_privacy_value, 1, 0, 0),
  [426] = {.entry = {.count = 1, .reusable = true}}, SHIFT(65),
  [428] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_field, 2, 0, 0),
  [430] = {.entry = {.count = 1, .reusable = true}}, SHIFT(37),
  [432] = {.entry = {.count = 1, .reusable = true}}, SHIFT(26),
  [434] = {.entry = {.count = 1, .reusable = true}}, SHIFT(30),
  [436] = {.entry = {.count = 1, .reusable = true}}, SHIFT(148),
  [438] = {.entry = {.count = 1, .reusable = true}}, SHIFT(127),
  [440] = {.entry = {.count = 1, .reusable = true}}, SHIFT(29),
  [442] = {.entry = {.count = 1, .reusable = true}}, SHIFT(150),
  [444] = {.entry = {.count = 1, .reusable = true}}, SHIFT(174),
  [446] = {.entry = {.count = 1, .reusable = true}}, SHIFT(40),
  [448] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0),
  [450] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(148),
  [453] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(127),
  [456] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0),
  [458] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0), SHIFT_REPEAT(150),
  [461] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_context_block_repeat1, 2, 0, 0), SHIFT_REPEAT(174),
  [464] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_reviewer_entry_repeat1, 2, 0, 0),
  [466] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_reviewer_entry_repeat1, 2, 0, 0), SHIFT_REPEAT(134),
  [469] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_reviewer_entry_repeat1, 2, 0, 0), SHIFT_REPEAT(48),
  [472] = {.entry = {.count = 1, .reusable = true}}, SHIFT(144),
  [474] = {.entry = {.count = 1, .reusable = true}}, SHIFT(134),
  [476] = {.entry = {.count = 1, .reusable = true}}, SHIFT(48),
  [478] = {.entry = {.count = 1, .reusable = true}}, SHIFT(149),
  [480] = {.entry = {.count = 1, .reusable = true}}, SHIFT(28),
  [482] = {.entry = {.count = 1, .reusable = false}}, SHIFT(175),
  [484] = {.entry = {.count = 1, .reusable = true}}, SHIFT(50),
  [486] = {.entry = {.count = 1, .reusable = false}}, SHIFT(100),
  [488] = {.entry = {.count = 1, .reusable = true}}, SHIFT(45),
  [490] = {.entry = {.count = 1, .reusable = false}}, SHIFT(103),
  [492] = {.entry = {.count = 1, .reusable = true}}, SHIFT(3),
  [494] = {.entry = {.count = 1, .reusable = true}}, SHIFT(102),
  [496] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0),
  [498] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0), SHIFT_REPEAT(102),
  [501] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0), SHIFT_REPEAT(175),
  [504] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0),
  [506] = {.entry = {.count = 2, .reusable = false}}, REDUCE(aux_sym_step_list_repeat1, 2, 0, 0), SHIFT_REPEAT(103),
  [509] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_verify_field, 2, 0, 0),
  [511] = {.entry = {.count = 1, .reusable = true}}, SHIFT(4),
  [513] = {.entry = {.count = 1, .reusable = true}}, SHIFT(101),
  [515] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_reviewer_list_repeat1, 2, 0, 0), SHIFT_REPEAT(96),
  [518] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_reviewer_list_repeat1, 2, 0, 0),
  [520] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_field, 2, 0, 0),
  [522] = {.entry = {.count = 1, .reusable = true}}, SHIFT(96),
  [524] = {.entry = {.count = 1, .reusable = true}}, SHIFT(8),
  [526] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_extra_pair, 2, 0, 0),
  [528] = {.entry = {.count = 1, .reusable = true}}, SHIFT(14),
  [530] = {.entry = {.count = 1, .reusable = true}}, SHIFT(133),
  [532] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_vars_block_repeat1, 2, 0, 0),
  [534] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_vars_block_repeat1, 2, 0, 0), SHIFT_REPEAT(133),
  [537] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_field, 2, 0, 0),
  [539] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_context_field, 2, 0, 0),
  [541] = {.entry = {.count = 1, .reusable = false}}, SHIFT(53),
  [543] = {.entry = {.count = 1, .reusable = true}}, SHIFT(7),
  [545] = {.entry = {.count = 1, .reusable = true}}, SHIFT(34),
  [547] = {.entry = {.count = 1, .reusable = true}}, SHIFT(18),
  [549] = {.entry = {.count = 1, .reusable = true}}, SHIFT(27),
  [551] = {.entry = {.count = 1, .reusable = true}}, SHIFT(67),
  [553] = {.entry = {.count = 1, .reusable = true}}, SHIFT(85),
  [555] = {.entry = {.count = 1, .reusable = true}}, SHIFT(6),
  [557] = {.entry = {.count = 1, .reusable = true}}, SHIFT(139),
  [559] = {.entry = {.count = 1, .reusable = true}}, SHIFT(66),
  [561] = {.entry = {.count = 1, .reusable = true}}, SHIFT(46),
  [563] = {.entry = {.count = 1, .reusable = true}}, SHIFT(2),
  [565] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_loop_block, 3, 0, 0),
  [567] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_block, 3, 0, 0),
  [569] = {.entry = {.count = 1, .reusable = true}}, SHIFT(33),
  [571] = {.entry = {.count = 1, .reusable = true}}, SHIFT(109),
  [573] = {.entry = {.count = 1, .reusable = true}}, SHIFT(35),
  [575] = {.entry = {.count = 1, .reusable = true}}, SHIFT(142),
  [577] = {.entry = {.count = 1, .reusable = true}}, SHIFT(71),
  [579] = {.entry = {.count = 1, .reusable = true}}, SHIFT(39),
  [581] = {.entry = {.count = 1, .reusable = false}}, REDUCE(sym_loop_block, 4, 0, 0),
  [583] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_loop_block, 4, 0, 0),
  [585] = {.entry = {.count = 1, .reusable = true}}, SHIFT(5),
  [587] = {.entry = {.count = 1, .reusable = true}}, SHIFT(122),
  [589] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0),
  [591] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0), SHIFT_REPEAT(139),
  [594] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_vars_pair, 2, 0, 0),
  [596] = {.entry = {.count = 1, .reusable = true}}, SHIFT(138),
  [598] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_entry, 2, 0, 0),
  [600] = {.entry = {.count = 1, .reusable = true}}, SHIFT(99),
  [602] = {.entry = {.count = 1, .reusable = true}}, SHIFT(105),
  [604] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_reviewer_entry, 3, 0, 0),
  [606] = {.entry = {.count = 1, .reusable = true}}, SHIFT(123),
  [608] = {.entry = {.count = 1, .reusable = true}}, SHIFT(168),
  [610] = {.entry = {.count = 1, .reusable = true}}, SHIFT(91),
  [612] = {.entry = {.count = 1, .reusable = true}}, SHIFT(42),
  [614] = {.entry = {.count = 1, .reusable = true}}, SHIFT(11),
  [616] = {.entry = {.count = 1, .reusable = true}},  ACCEPT_INPUT(),
  [618] = {.entry = {.count = 1, .reusable = true}}, SHIFT(98),
  [620] = {.entry = {.count = 1, .reusable = true}}, SHIFT(38),
  [622] = {.entry = {.count = 1, .reusable = true}}, SHIFT(177),
  [624] = {.entry = {.count = 1, .reusable = true}}, SHIFT(20),
  [626] = {.entry = {.count = 1, .reusable = true}}, SHIFT(110),
  [628] = {.entry = {.count = 1, .reusable = true}}, SHIFT(54),
  [630] = {.entry = {.count = 1, .reusable = true}}, SHIFT(77),
  [632] = {.entry = {.count = 1, .reusable = true}}, SHIFT(23),
  [634] = {.entry = {.count = 1, .reusable = true}}, SHIFT(84),
  [636] = {.entry = {.count = 1, .reusable = true}}, SHIFT(113),
  [638] = {.entry = {.count = 1, .reusable = true}}, SHIFT(22),
  [640] = {.entry = {.count = 1, .reusable = true}}, SHIFT(74),
  [642] = {.entry = {.count = 1, .reusable = true}}, SHIFT(59),
  [644] = {.entry = {.count = 1, .reusable = true}}, SHIFT(157),
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
