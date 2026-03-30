#include "tree_sitter/parser.h"

#if defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic ignored "-Wmissing-field-initializers"
#endif

#define LANGUAGE_VERSION 14
#define STATE_COUNT 59
#define LARGE_STATE_COUNT 2
#define SYMBOL_COUNT 53
#define ALIAS_COUNT 0
#define TOKEN_COUNT 31
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
  anon_sym_workflow = 16,
  anon_sym_steps = 17,
  anon_sym_max_parallel = 18,
  anon_sym_LBRACK = 19,
  anon_sym_RBRACK = 20,
  anon_sym_coordinator = 21,
  anon_sym_reasoning = 22,
  anon_sym_execution = 23,
  anon_sym_mechanical = 24,
  anon_sym_public = 25,
  anon_sym_local_only = 26,
  sym_string = 27,
  sym_raw_string = 28,
  sym_integer = 29,
  sym_identifier = 30,
  sym_source_file = 31,
  sym__definition = 32,
  sym_client_declaration = 33,
  sym_client_field = 34,
  sym_agent_declaration = 35,
  sym_agent_field = 36,
  sym_scope_block = 37,
  sym_scope_field = 38,
  sym_workflow_declaration = 39,
  sym_workflow_field = 40,
  sym_string_list = 41,
  sym_identifier_list = 42,
  sym_tier_value = 43,
  sym_privacy_value = 44,
  sym__string_value = 45,
  aux_sym_source_file_repeat1 = 46,
  aux_sym_client_declaration_repeat1 = 47,
  aux_sym_agent_declaration_repeat1 = 48,
  aux_sym_scope_block_repeat1 = 49,
  aux_sym_workflow_declaration_repeat1 = 50,
  aux_sym_string_list_repeat1 = 51,
  aux_sym_identifier_list_repeat1 = 52,
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
  [anon_sym_workflow] = "workflow",
  [anon_sym_steps] = "steps",
  [anon_sym_max_parallel] = "max_parallel",
  [anon_sym_LBRACK] = "[",
  [anon_sym_RBRACK] = "]",
  [anon_sym_coordinator] = "coordinator",
  [anon_sym_reasoning] = "reasoning",
  [anon_sym_execution] = "execution",
  [anon_sym_mechanical] = "mechanical",
  [anon_sym_public] = "public",
  [anon_sym_local_only] = "local_only",
  [sym_string] = "string",
  [sym_raw_string] = "raw_string",
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
  [sym_workflow_declaration] = "workflow_declaration",
  [sym_workflow_field] = "workflow_field",
  [sym_string_list] = "string_list",
  [sym_identifier_list] = "identifier_list",
  [sym_tier_value] = "tier_value",
  [sym_privacy_value] = "privacy_value",
  [sym__string_value] = "_string_value",
  [aux_sym_source_file_repeat1] = "source_file_repeat1",
  [aux_sym_client_declaration_repeat1] = "client_declaration_repeat1",
  [aux_sym_agent_declaration_repeat1] = "agent_declaration_repeat1",
  [aux_sym_scope_block_repeat1] = "scope_block_repeat1",
  [aux_sym_workflow_declaration_repeat1] = "workflow_declaration_repeat1",
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
  [anon_sym_workflow] = anon_sym_workflow,
  [anon_sym_steps] = anon_sym_steps,
  [anon_sym_max_parallel] = anon_sym_max_parallel,
  [anon_sym_LBRACK] = anon_sym_LBRACK,
  [anon_sym_RBRACK] = anon_sym_RBRACK,
  [anon_sym_coordinator] = anon_sym_coordinator,
  [anon_sym_reasoning] = anon_sym_reasoning,
  [anon_sym_execution] = anon_sym_execution,
  [anon_sym_mechanical] = anon_sym_mechanical,
  [anon_sym_public] = anon_sym_public,
  [anon_sym_local_only] = anon_sym_local_only,
  [sym_string] = sym_string,
  [sym_raw_string] = sym_raw_string,
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
  [sym_workflow_declaration] = sym_workflow_declaration,
  [sym_workflow_field] = sym_workflow_field,
  [sym_string_list] = sym_string_list,
  [sym_identifier_list] = sym_identifier_list,
  [sym_tier_value] = sym_tier_value,
  [sym_privacy_value] = sym_privacy_value,
  [sym__string_value] = sym__string_value,
  [aux_sym_source_file_repeat1] = aux_sym_source_file_repeat1,
  [aux_sym_client_declaration_repeat1] = aux_sym_client_declaration_repeat1,
  [aux_sym_agent_declaration_repeat1] = aux_sym_agent_declaration_repeat1,
  [aux_sym_scope_block_repeat1] = aux_sym_scope_block_repeat1,
  [aux_sym_workflow_declaration_repeat1] = aux_sym_workflow_declaration_repeat1,
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
  [anon_sym_LBRACK] = {
    .visible = true,
    .named = false,
  },
  [anon_sym_RBRACK] = {
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
  [sym_string] = {
    .visible = true,
    .named = true,
  },
  [sym_raw_string] = {
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
  [sym_workflow_declaration] = {
    .visible = true,
    .named = true,
  },
  [sym_workflow_field] = {
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
  [aux_sym_workflow_declaration_repeat1] = {
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
};

static bool ts_lex(TSLexer *lexer, TSStateId state) {
  START_LEXER();
  eof = lexer->eof(lexer);
  switch (state) {
    case 0:
      if (eof) ADVANCE(134);
      ADVANCE_MAP(
        '"', 1,
        '#', 3,
        '/', 6,
        '[', 153,
        ']', 154,
        'a', 50,
        'c', 73,
        'd', 34,
        'e', 130,
        'l', 89,
        'm', 12,
        'o', 127,
        'p', 107,
        'r', 35,
        's', 26,
        't', 53,
        'w', 90,
        '{', 137,
        '}', 138,
      );
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(0);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(163);
      END_STATE();
    case 1:
      if (lookahead == '"') ADVANCE(161);
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
      if (lookahead == '#') ADVANCE(162);
      if (lookahead != 0) ADVANCE(2);
      END_STATE();
    case 5:
      if (lookahead == '/') ADVANCE(6);
      if (lookahead == ']') ADVANCE(154);
      if (('\t' <= lookahead && lookahead <= '\r') ||
          lookahead == ' ') SKIP(5);
      if (('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(164);
      END_STATE();
    case 6:
      if (lookahead == '/') ADVANCE(135);
      END_STATE();
    case 7:
      if (lookahead == '_') ADVANCE(103);
      END_STATE();
    case 8:
      if (lookahead == '_') ADVANCE(96);
      END_STATE();
    case 9:
      if (lookahead == '_') ADVANCE(97);
      END_STATE();
    case 10:
      if (lookahead == '_') ADVANCE(99);
      END_STATE();
    case 11:
      if (lookahead == 'a') ADVANCE(30);
      END_STATE();
    case 12:
      if (lookahead == 'a') ADVANCE(129);
      if (lookahead == 'e') ADVANCE(21);
      if (lookahead == 'o') ADVANCE(33);
      END_STATE();
    case 13:
      if (lookahead == 'a') ADVANCE(74);
      END_STATE();
    case 14:
      if (lookahead == 'a') ADVANCE(23);
      END_STATE();
    case 15:
      if (lookahead == 'a') ADVANCE(84);
      END_STATE();
    case 16:
      if (lookahead == 'a') ADVANCE(113);
      END_STATE();
    case 17:
      if (lookahead == 'a') ADVANCE(123);
      END_STATE();
    case 18:
      if (lookahead == 'a') ADVANCE(71);
      END_STATE();
    case 19:
      if (lookahead == 'a') ADVANCE(65);
      END_STATE();
    case 20:
      if (lookahead == 'b') ADVANCE(67);
      END_STATE();
    case 21:
      if (lookahead == 'c') ADVANCE(51);
      END_STATE();
    case 22:
      if (lookahead == 'c') ADVANCE(125);
      END_STATE();
    case 23:
      if (lookahead == 'c') ADVANCE(131);
      END_STATE();
    case 24:
      if (lookahead == 'c') ADVANCE(159);
      END_STATE();
    case 25:
      if (lookahead == 'c') ADVANCE(13);
      END_STATE();
    case 26:
      if (lookahead == 'c') ADVANCE(91);
      if (lookahead == 't') ADVANCE(38);
      END_STATE();
    case 27:
      if (lookahead == 'c') ADVANCE(112);
      END_STATE();
    case 28:
      if (lookahead == 'c') ADVANCE(19);
      END_STATE();
    case 29:
      if (lookahead == 'd') ADVANCE(148);
      END_STATE();
    case 30:
      if (lookahead == 'd') ADVANCE(8);
      if (lookahead == 's') ADVANCE(94);
      END_STATE();
    case 31:
      if (lookahead == 'd') ADVANCE(117);
      END_STATE();
    case 32:
      if (lookahead == 'd') ADVANCE(56);
      END_STATE();
    case 33:
      if (lookahead == 'd') ADVANCE(42);
      END_STATE();
    case 34:
      if (lookahead == 'e') ADVANCE(104);
      END_STATE();
    case 35:
      if (lookahead == 'e') ADVANCE(11);
      END_STATE();
    case 36:
      if (lookahead == 'e') ADVANCE(147);
      END_STATE();
    case 37:
      if (lookahead == 'e') ADVANCE(76);
      END_STATE();
    case 38:
      if (lookahead == 'e') ADVANCE(101);
      END_STATE();
    case 39:
      if (lookahead == 'e') ADVANCE(29);
      END_STATE();
    case 40:
      if (lookahead == 'e') ADVANCE(81);
      END_STATE();
    case 41:
      if (lookahead == 'e') ADVANCE(109);
      END_STATE();
    case 42:
      if (lookahead == 'e') ADVANCE(64);
      END_STATE();
    case 43:
      if (lookahead == 'e') ADVANCE(116);
      END_STATE();
    case 44:
      if (lookahead == 'e') ADVANCE(22);
      END_STATE();
    case 45:
      if (lookahead == 'e') ADVANCE(82);
      END_STATE();
    case 46:
      if (lookahead == 'e') ADVANCE(122);
      END_STATE();
    case 47:
      if (lookahead == 'e') ADVANCE(66);
      END_STATE();
    case 48:
      if (lookahead == 'f') ADVANCE(70);
      END_STATE();
    case 49:
      if (lookahead == 'g') ADVANCE(156);
      END_STATE();
    case 50:
      if (lookahead == 'g') ADVANCE(37);
      END_STATE();
    case 51:
      if (lookahead == 'h') ADVANCE(15);
      END_STATE();
    case 52:
      if (lookahead == 'i') ADVANCE(126);
      if (lookahead == 'o') ADVANCE(75);
      END_STATE();
    case 53:
      if (lookahead == 'i') ADVANCE(41);
      END_STATE();
    case 54:
      if (lookahead == 'i') ADVANCE(24);
      END_STATE();
    case 55:
      if (lookahead == 'i') ADVANCE(40);
      END_STATE();
    case 56:
      if (lookahead == 'i') ADVANCE(85);
      END_STATE();
    case 57:
      if (lookahead == 'i') ADVANCE(80);
      END_STATE();
    case 58:
      if (lookahead == 'i') ADVANCE(43);
      END_STATE();
    case 59:
      if (lookahead == 'i') ADVANCE(106);
      END_STATE();
    case 60:
      if (lookahead == 'i') ADVANCE(98);
      END_STATE();
    case 61:
      if (lookahead == 'i') ADVANCE(100);
      END_STATE();
    case 62:
      if (lookahead == 'i') ADVANCE(28);
      END_STATE();
    case 63:
      if (lookahead == 'k') ADVANCE(48);
      END_STATE();
    case 64:
      if (lookahead == 'l') ADVANCE(140);
      END_STATE();
    case 65:
      if (lookahead == 'l') ADVANCE(158);
      END_STATE();
    case 66:
      if (lookahead == 'l') ADVANCE(152);
      END_STATE();
    case 67:
      if (lookahead == 'l') ADVANCE(54);
      END_STATE();
    case 68:
      if (lookahead == 'l') ADVANCE(132);
      END_STATE();
    case 69:
      if (lookahead == 'l') ADVANCE(133);
      END_STATE();
    case 70:
      if (lookahead == 'l') ADVANCE(92);
      END_STATE();
    case 71:
      if (lookahead == 'l') ADVANCE(72);
      END_STATE();
    case 72:
      if (lookahead == 'l') ADVANCE(47);
      END_STATE();
    case 73:
      if (lookahead == 'l') ADVANCE(55);
      if (lookahead == 'o') ADVANCE(93);
      END_STATE();
    case 74:
      if (lookahead == 'l') ADVANCE(9);
      END_STATE();
    case 75:
      if (lookahead == 'm') ADVANCE(102);
      END_STATE();
    case 76:
      if (lookahead == 'n') ADVANCE(118);
      END_STATE();
    case 77:
      if (lookahead == 'n') ADVANCE(157);
      END_STATE();
    case 78:
      if (lookahead == 'n') ADVANCE(144);
      END_STATE();
    case 79:
      if (lookahead == 'n') ADVANCE(143);
      END_STATE();
    case 80:
      if (lookahead == 'n') ADVANCE(49);
      END_STATE();
    case 81:
      if (lookahead == 'n') ADVANCE(119);
      END_STATE();
    case 82:
      if (lookahead == 'n') ADVANCE(31);
      END_STATE();
    case 83:
      if (lookahead == 'n') ADVANCE(68);
      END_STATE();
    case 84:
      if (lookahead == 'n') ADVANCE(62);
      END_STATE();
    case 85:
      if (lookahead == 'n') ADVANCE(17);
      END_STATE();
    case 86:
      if (lookahead == 'n') ADVANCE(69);
      END_STATE();
    case 87:
      if (lookahead == 'n') ADVANCE(39);
      END_STATE();
    case 88:
      if (lookahead == 'n') ADVANCE(57);
      END_STATE();
    case 89:
      if (lookahead == 'o') ADVANCE(25);
      END_STATE();
    case 90:
      if (lookahead == 'o') ADVANCE(108);
      END_STATE();
    case 91:
      if (lookahead == 'o') ADVANCE(105);
      END_STATE();
    case 92:
      if (lookahead == 'o') ADVANCE(128);
      END_STATE();
    case 93:
      if (lookahead == 'o') ADVANCE(111);
      END_STATE();
    case 94:
      if (lookahead == 'o') ADVANCE(88);
      END_STATE();
    case 95:
      if (lookahead == 'o') ADVANCE(110);
      END_STATE();
    case 96:
      if (lookahead == 'o') ADVANCE(83);
      END_STATE();
    case 97:
      if (lookahead == 'o') ADVANCE(86);
      END_STATE();
    case 98:
      if (lookahead == 'o') ADVANCE(77);
      END_STATE();
    case 99:
      if (lookahead == 'o') ADVANCE(78);
      END_STATE();
    case 100:
      if (lookahead == 'o') ADVANCE(79);
      END_STATE();
    case 101:
      if (lookahead == 'p') ADVANCE(115);
      END_STATE();
    case 102:
      if (lookahead == 'p') ADVANCE(120);
      END_STATE();
    case 103:
      if (lookahead == 'p') ADVANCE(16);
      if (lookahead == 'r') ADVANCE(46);
      END_STATE();
    case 104:
      if (lookahead == 'p') ADVANCE(45);
      if (lookahead == 's') ADVANCE(27);
      END_STATE();
    case 105:
      if (lookahead == 'p') ADVANCE(36);
      END_STATE();
    case 106:
      if (lookahead == 'p') ADVANCE(124);
      END_STATE();
    case 107:
      if (lookahead == 'r') ADVANCE(52);
      if (lookahead == 'u') ADVANCE(20);
      END_STATE();
    case 108:
      if (lookahead == 'r') ADVANCE(63);
      END_STATE();
    case 109:
      if (lookahead == 'r') ADVANCE(139);
      END_STATE();
    case 110:
      if (lookahead == 'r') ADVANCE(155);
      END_STATE();
    case 111:
      if (lookahead == 'r') ADVANCE(32);
      END_STATE();
    case 112:
      if (lookahead == 'r') ADVANCE(59);
      END_STATE();
    case 113:
      if (lookahead == 'r') ADVANCE(18);
      END_STATE();
    case 114:
      if (lookahead == 'r') ADVANCE(58);
      END_STATE();
    case 115:
      if (lookahead == 's') ADVANCE(151);
      END_STATE();
    case 116:
      if (lookahead == 's') ADVANCE(146);
      END_STATE();
    case 117:
      if (lookahead == 's') ADVANCE(10);
      END_STATE();
    case 118:
      if (lookahead == 't') ADVANCE(142);
      END_STATE();
    case 119:
      if (lookahead == 't') ADVANCE(136);
      END_STATE();
    case 120:
      if (lookahead == 't') ADVANCE(145);
      END_STATE();
    case 121:
      if (lookahead == 't') ADVANCE(60);
      END_STATE();
    case 122:
      if (lookahead == 't') ADVANCE(114);
      END_STATE();
    case 123:
      if (lookahead == 't') ADVANCE(95);
      END_STATE();
    case 124:
      if (lookahead == 't') ADVANCE(61);
      END_STATE();
    case 125:
      if (lookahead == 'u') ADVANCE(121);
      END_STATE();
    case 126:
      if (lookahead == 'v') ADVANCE(14);
      END_STATE();
    case 127:
      if (lookahead == 'w') ADVANCE(87);
      END_STATE();
    case 128:
      if (lookahead == 'w') ADVANCE(150);
      END_STATE();
    case 129:
      if (lookahead == 'x') ADVANCE(7);
      END_STATE();
    case 130:
      if (lookahead == 'x') ADVANCE(44);
      END_STATE();
    case 131:
      if (lookahead == 'y') ADVANCE(141);
      END_STATE();
    case 132:
      if (lookahead == 'y') ADVANCE(149);
      END_STATE();
    case 133:
      if (lookahead == 'y') ADVANCE(160);
      END_STATE();
    case 134:
      ACCEPT_TOKEN(ts_builtin_sym_end);
      END_STATE();
    case 135:
      ACCEPT_TOKEN(sym_comment);
      if (lookahead != 0 &&
          lookahead != '\n') ADVANCE(135);
      END_STATE();
    case 136:
      ACCEPT_TOKEN(anon_sym_client);
      END_STATE();
    case 137:
      ACCEPT_TOKEN(anon_sym_LBRACE);
      END_STATE();
    case 138:
      ACCEPT_TOKEN(anon_sym_RBRACE);
      END_STATE();
    case 139:
      ACCEPT_TOKEN(anon_sym_tier);
      END_STATE();
    case 140:
      ACCEPT_TOKEN(anon_sym_model);
      END_STATE();
    case 141:
      ACCEPT_TOKEN(anon_sym_privacy);
      END_STATE();
    case 142:
      ACCEPT_TOKEN(anon_sym_agent);
      END_STATE();
    case 143:
      ACCEPT_TOKEN(anon_sym_description);
      END_STATE();
    case 144:
      ACCEPT_TOKEN(anon_sym_depends_on);
      END_STATE();
    case 145:
      ACCEPT_TOKEN(anon_sym_prompt);
      END_STATE();
    case 146:
      ACCEPT_TOKEN(anon_sym_max_retries);
      END_STATE();
    case 147:
      ACCEPT_TOKEN(anon_sym_scope);
      END_STATE();
    case 148:
      ACCEPT_TOKEN(anon_sym_owned);
      END_STATE();
    case 149:
      ACCEPT_TOKEN(anon_sym_read_only);
      END_STATE();
    case 150:
      ACCEPT_TOKEN(anon_sym_workflow);
      END_STATE();
    case 151:
      ACCEPT_TOKEN(anon_sym_steps);
      END_STATE();
    case 152:
      ACCEPT_TOKEN(anon_sym_max_parallel);
      END_STATE();
    case 153:
      ACCEPT_TOKEN(anon_sym_LBRACK);
      END_STATE();
    case 154:
      ACCEPT_TOKEN(anon_sym_RBRACK);
      END_STATE();
    case 155:
      ACCEPT_TOKEN(anon_sym_coordinator);
      END_STATE();
    case 156:
      ACCEPT_TOKEN(anon_sym_reasoning);
      END_STATE();
    case 157:
      ACCEPT_TOKEN(anon_sym_execution);
      END_STATE();
    case 158:
      ACCEPT_TOKEN(anon_sym_mechanical);
      END_STATE();
    case 159:
      ACCEPT_TOKEN(anon_sym_public);
      END_STATE();
    case 160:
      ACCEPT_TOKEN(anon_sym_local_only);
      END_STATE();
    case 161:
      ACCEPT_TOKEN(sym_string);
      END_STATE();
    case 162:
      ACCEPT_TOKEN(sym_raw_string);
      END_STATE();
    case 163:
      ACCEPT_TOKEN(sym_integer);
      if (('0' <= lookahead && lookahead <= '9')) ADVANCE(163);
      END_STATE();
    case 164:
      ACCEPT_TOKEN(sym_identifier);
      if (lookahead == '-' ||
          ('0' <= lookahead && lookahead <= '9') ||
          ('A' <= lookahead && lookahead <= 'Z') ||
          lookahead == '_' ||
          ('a' <= lookahead && lookahead <= 'z')) ADVANCE(164);
      END_STATE();
    default:
      return false;
  }
}

static const TSLexMode ts_lex_modes[STATE_COUNT] = {
  [0] = {.lex_state = 0},
  [1] = {.lex_state = 0},
  [2] = {.lex_state = 0},
  [3] = {.lex_state = 0},
  [4] = {.lex_state = 0},
  [5] = {.lex_state = 0},
  [6] = {.lex_state = 0},
  [7] = {.lex_state = 0},
  [8] = {.lex_state = 0},
  [9] = {.lex_state = 0},
  [10] = {.lex_state = 0},
  [11] = {.lex_state = 0},
  [12] = {.lex_state = 0},
  [13] = {.lex_state = 0},
  [14] = {.lex_state = 0},
  [15] = {.lex_state = 0},
  [16] = {.lex_state = 0},
  [17] = {.lex_state = 0},
  [18] = {.lex_state = 0},
  [19] = {.lex_state = 0},
  [20] = {.lex_state = 0},
  [21] = {.lex_state = 0},
  [22] = {.lex_state = 0},
  [23] = {.lex_state = 0},
  [24] = {.lex_state = 0},
  [25] = {.lex_state = 0},
  [26] = {.lex_state = 0},
  [27] = {.lex_state = 0},
  [28] = {.lex_state = 0},
  [29] = {.lex_state = 0},
  [30] = {.lex_state = 0},
  [31] = {.lex_state = 0},
  [32] = {.lex_state = 0},
  [33] = {.lex_state = 0},
  [34] = {.lex_state = 0},
  [35] = {.lex_state = 5},
  [36] = {.lex_state = 0},
  [37] = {.lex_state = 0},
  [38] = {.lex_state = 5},
  [39] = {.lex_state = 5},
  [40] = {.lex_state = 0},
  [41] = {.lex_state = 0},
  [42] = {.lex_state = 0},
  [43] = {.lex_state = 0},
  [44] = {.lex_state = 0},
  [45] = {.lex_state = 0},
  [46] = {.lex_state = 0},
  [47] = {.lex_state = 0},
  [48] = {.lex_state = 0},
  [49] = {.lex_state = 5},
  [50] = {.lex_state = 0},
  [51] = {.lex_state = 0},
  [52] = {.lex_state = 5},
  [53] = {.lex_state = 0},
  [54] = {.lex_state = 0},
  [55] = {.lex_state = 0},
  [56] = {.lex_state = 0},
  [57] = {.lex_state = 5},
  [58] = {.lex_state = 5},
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
    [anon_sym_workflow] = ACTIONS(1),
    [anon_sym_steps] = ACTIONS(1),
    [anon_sym_max_parallel] = ACTIONS(1),
    [anon_sym_LBRACK] = ACTIONS(1),
    [anon_sym_RBRACK] = ACTIONS(1),
    [anon_sym_coordinator] = ACTIONS(1),
    [anon_sym_reasoning] = ACTIONS(1),
    [anon_sym_execution] = ACTIONS(1),
    [anon_sym_mechanical] = ACTIONS(1),
    [anon_sym_public] = ACTIONS(1),
    [anon_sym_local_only] = ACTIONS(1),
    [sym_string] = ACTIONS(1),
    [sym_raw_string] = ACTIONS(1),
    [sym_integer] = ACTIONS(1),
  },
  [1] = {
    [sym_source_file] = STATE(54),
    [sym__definition] = STATE(8),
    [sym_client_declaration] = STATE(8),
    [sym_agent_declaration] = STATE(8),
    [sym_workflow_declaration] = STATE(8),
    [aux_sym_source_file_repeat1] = STATE(8),
    [ts_builtin_sym_end] = ACTIONS(5),
    [sym_comment] = ACTIONS(3),
    [anon_sym_client] = ACTIONS(7),
    [anon_sym_agent] = ACTIONS(9),
    [anon_sym_workflow] = ACTIONS(11),
  },
};

static const uint16_t ts_small_parse_table[] = {
  [0] = 9,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(13), 1,
      anon_sym_client,
    ACTIONS(15), 1,
      anon_sym_RBRACE,
    ACTIONS(19), 1,
      anon_sym_depends_on,
    ACTIONS(21), 1,
      anon_sym_max_retries,
    ACTIONS(23), 1,
      anon_sym_scope,
    STATE(9), 1,
      sym_scope_block,
    ACTIONS(17), 2,
      anon_sym_description,
      anon_sym_prompt,
    STATE(3), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
  [30] = 9,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(13), 1,
      anon_sym_client,
    ACTIONS(19), 1,
      anon_sym_depends_on,
    ACTIONS(21), 1,
      anon_sym_max_retries,
    ACTIONS(23), 1,
      anon_sym_scope,
    ACTIONS(25), 1,
      anon_sym_RBRACE,
    STATE(9), 1,
      sym_scope_block,
    ACTIONS(17), 2,
      anon_sym_description,
      anon_sym_prompt,
    STATE(4), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
  [60] = 9,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(27), 1,
      anon_sym_client,
    ACTIONS(30), 1,
      anon_sym_RBRACE,
    ACTIONS(35), 1,
      anon_sym_depends_on,
    ACTIONS(38), 1,
      anon_sym_max_retries,
    ACTIONS(41), 1,
      anon_sym_scope,
    STATE(9), 1,
      sym_scope_block,
    ACTIONS(32), 2,
      anon_sym_description,
      anon_sym_prompt,
    STATE(4), 2,
      sym_agent_field,
      aux_sym_agent_declaration_repeat1,
  [90] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(44), 9,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_steps,
      anon_sym_max_parallel,
  [105] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(46), 9,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
      anon_sym_max_retries,
      anon_sym_scope,
      anon_sym_steps,
      anon_sym_max_parallel,
  [120] = 6,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(48), 1,
      ts_builtin_sym_end,
    ACTIONS(50), 1,
      anon_sym_client,
    ACTIONS(53), 1,
      anon_sym_agent,
    ACTIONS(56), 1,
      anon_sym_workflow,
    STATE(7), 5,
      sym__definition,
      sym_client_declaration,
      sym_agent_declaration,
      sym_workflow_declaration,
      aux_sym_source_file_repeat1,
  [143] = 6,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(7), 1,
      anon_sym_client,
    ACTIONS(9), 1,
      anon_sym_agent,
    ACTIONS(11), 1,
      anon_sym_workflow,
    ACTIONS(59), 1,
      ts_builtin_sym_end,
    STATE(7), 5,
      sym__definition,
      sym_client_declaration,
      sym_agent_declaration,
      sym_workflow_declaration,
      aux_sym_source_file_repeat1,
  [166] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(61), 7,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
      anon_sym_max_retries,
      anon_sym_scope,
  [179] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(63), 7,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
      anon_sym_max_retries,
      anon_sym_scope,
  [192] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(65), 7,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
      anon_sym_max_retries,
      anon_sym_scope,
  [205] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(67), 7,
      anon_sym_client,
      anon_sym_RBRACE,
      anon_sym_description,
      anon_sym_depends_on,
      anon_sym_prompt,
      anon_sym_max_retries,
      anon_sym_scope,
  [218] = 6,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(69), 1,
      anon_sym_RBRACE,
    ACTIONS(71), 1,
      anon_sym_tier,
    ACTIONS(74), 1,
      anon_sym_model,
    ACTIONS(77), 1,
      anon_sym_privacy,
    STATE(13), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [238] = 6,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(80), 1,
      anon_sym_RBRACE,
    ACTIONS(82), 1,
      anon_sym_tier,
    ACTIONS(84), 1,
      anon_sym_model,
    ACTIONS(86), 1,
      anon_sym_privacy,
    STATE(15), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [258] = 6,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(82), 1,
      anon_sym_tier,
    ACTIONS(84), 1,
      anon_sym_model,
    ACTIONS(86), 1,
      anon_sym_privacy,
    ACTIONS(88), 1,
      anon_sym_RBRACE,
    STATE(13), 2,
      sym_client_field,
      aux_sym_client_declaration_repeat1,
  [278] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(90), 1,
      anon_sym_RBRACE,
    ACTIONS(92), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(21), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [293] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(94), 1,
      anon_sym_RBRACK,
    ACTIONS(96), 2,
      sym_string,
      sym_raw_string,
    STATE(17), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [308] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(99), 1,
      anon_sym_RBRACE,
    ACTIONS(101), 1,
      anon_sym_steps,
    ACTIONS(103), 1,
      anon_sym_max_parallel,
    STATE(23), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
  [325] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(101), 1,
      anon_sym_steps,
    ACTIONS(103), 1,
      anon_sym_max_parallel,
    ACTIONS(105), 1,
      anon_sym_RBRACE,
    STATE(18), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
  [342] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(107), 1,
      anon_sym_RBRACE,
    ACTIONS(92), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(16), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [357] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(109), 1,
      anon_sym_RBRACE,
    ACTIONS(111), 2,
      anon_sym_owned,
      anon_sym_read_only,
    STATE(21), 2,
      sym_scope_field,
      aux_sym_scope_block_repeat1,
  [372] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(28), 1,
      sym_tier_value,
    ACTIONS(114), 4,
      anon_sym_coordinator,
      anon_sym_reasoning,
      anon_sym_execution,
      anon_sym_mechanical,
  [385] = 5,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(116), 1,
      anon_sym_RBRACE,
    ACTIONS(118), 1,
      anon_sym_steps,
    ACTIONS(121), 1,
      anon_sym_max_parallel,
    STATE(23), 2,
      sym_workflow_field,
      aux_sym_workflow_declaration_repeat1,
  [402] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(124), 1,
      anon_sym_RBRACK,
    ACTIONS(126), 2,
      sym_string,
      sym_raw_string,
    STATE(17), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [417] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(128), 1,
      anon_sym_RBRACK,
    ACTIONS(130), 2,
      sym_string,
      sym_raw_string,
    STATE(24), 2,
      sym__string_value,
      aux_sym_string_list_repeat1,
  [432] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(132), 4,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_agent,
      anon_sym_workflow,
  [442] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(134), 4,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_agent,
      anon_sym_workflow,
  [452] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(136), 4,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_privacy,
  [462] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(138), 4,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_privacy,
  [472] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(140), 4,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_agent,
      anon_sym_workflow,
  [482] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(142), 4,
      anon_sym_RBRACE,
      anon_sym_tier,
      anon_sym_model,
      anon_sym_privacy,
  [492] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(144), 4,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_agent,
      anon_sym_workflow,
  [502] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(146), 4,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_agent,
      anon_sym_workflow,
  [512] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(148), 4,
      ts_builtin_sym_end,
      anon_sym_client,
      anon_sym_agent,
      anon_sym_workflow,
  [522] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(150), 1,
      anon_sym_RBRACK,
    ACTIONS(152), 1,
      sym_identifier,
    STATE(38), 1,
      aux_sym_identifier_list_repeat1,
  [535] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(154), 3,
      anon_sym_RBRACE,
      anon_sym_steps,
      anon_sym_max_parallel,
  [544] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(11), 1,
      sym__string_value,
    ACTIONS(156), 2,
      sym_string,
      sym_raw_string,
  [555] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(158), 1,
      anon_sym_RBRACK,
    ACTIONS(160), 1,
      sym_identifier,
    STATE(39), 1,
      aux_sym_identifier_list_repeat1,
  [568] = 4,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(162), 1,
      anon_sym_RBRACK,
    ACTIONS(164), 1,
      sym_identifier,
    STATE(39), 1,
      aux_sym_identifier_list_repeat1,
  [581] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(28), 1,
      sym__string_value,
    ACTIONS(167), 2,
      sym_string,
      sym_raw_string,
  [592] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(169), 3,
      anon_sym_RBRACE,
      anon_sym_owned,
      anon_sym_read_only,
  [601] = 3,
    ACTIONS(3), 1,
      sym_comment,
    STATE(28), 1,
      sym_privacy_value,
    ACTIONS(171), 2,
      anon_sym_public,
      anon_sym_local_only,
  [612] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(173), 3,
      anon_sym_RBRACE,
      anon_sym_owned,
      anon_sym_read_only,
  [621] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(175), 3,
      anon_sym_RBRACE,
      anon_sym_owned,
      anon_sym_read_only,
  [630] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(177), 1,
      anon_sym_LBRACK,
    STATE(36), 1,
      sym_identifier_list,
  [640] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(177), 1,
      anon_sym_LBRACK,
    STATE(11), 1,
      sym_identifier_list,
  [650] = 3,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(179), 1,
      anon_sym_LBRACK,
    STATE(41), 1,
      sym_string_list,
  [660] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(181), 1,
      anon_sym_LBRACE,
  [667] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(156), 1,
      sym_identifier,
  [674] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(183), 1,
      sym_integer,
  [681] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(185), 1,
      anon_sym_LBRACE,
  [688] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(187), 1,
      sym_identifier,
  [695] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(156), 1,
      sym_integer,
  [702] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(189), 1,
      ts_builtin_sym_end,
  [709] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(191), 1,
      anon_sym_LBRACE,
  [716] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(193), 1,
      anon_sym_LBRACE,
  [723] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(195), 1,
      sym_identifier,
  [730] = 2,
    ACTIONS(3), 1,
      sym_comment,
    ACTIONS(197), 1,
      sym_identifier,
};

static const uint32_t ts_small_parse_table_map[] = {
  [SMALL_STATE(2)] = 0,
  [SMALL_STATE(3)] = 30,
  [SMALL_STATE(4)] = 60,
  [SMALL_STATE(5)] = 90,
  [SMALL_STATE(6)] = 105,
  [SMALL_STATE(7)] = 120,
  [SMALL_STATE(8)] = 143,
  [SMALL_STATE(9)] = 166,
  [SMALL_STATE(10)] = 179,
  [SMALL_STATE(11)] = 192,
  [SMALL_STATE(12)] = 205,
  [SMALL_STATE(13)] = 218,
  [SMALL_STATE(14)] = 238,
  [SMALL_STATE(15)] = 258,
  [SMALL_STATE(16)] = 278,
  [SMALL_STATE(17)] = 293,
  [SMALL_STATE(18)] = 308,
  [SMALL_STATE(19)] = 325,
  [SMALL_STATE(20)] = 342,
  [SMALL_STATE(21)] = 357,
  [SMALL_STATE(22)] = 372,
  [SMALL_STATE(23)] = 385,
  [SMALL_STATE(24)] = 402,
  [SMALL_STATE(25)] = 417,
  [SMALL_STATE(26)] = 432,
  [SMALL_STATE(27)] = 442,
  [SMALL_STATE(28)] = 452,
  [SMALL_STATE(29)] = 462,
  [SMALL_STATE(30)] = 472,
  [SMALL_STATE(31)] = 482,
  [SMALL_STATE(32)] = 492,
  [SMALL_STATE(33)] = 502,
  [SMALL_STATE(34)] = 512,
  [SMALL_STATE(35)] = 522,
  [SMALL_STATE(36)] = 535,
  [SMALL_STATE(37)] = 544,
  [SMALL_STATE(38)] = 555,
  [SMALL_STATE(39)] = 568,
  [SMALL_STATE(40)] = 581,
  [SMALL_STATE(41)] = 592,
  [SMALL_STATE(42)] = 601,
  [SMALL_STATE(43)] = 612,
  [SMALL_STATE(44)] = 621,
  [SMALL_STATE(45)] = 630,
  [SMALL_STATE(46)] = 640,
  [SMALL_STATE(47)] = 650,
  [SMALL_STATE(48)] = 660,
  [SMALL_STATE(49)] = 667,
  [SMALL_STATE(50)] = 674,
  [SMALL_STATE(51)] = 681,
  [SMALL_STATE(52)] = 688,
  [SMALL_STATE(53)] = 695,
  [SMALL_STATE(54)] = 702,
  [SMALL_STATE(55)] = 709,
  [SMALL_STATE(56)] = 716,
  [SMALL_STATE(57)] = 723,
  [SMALL_STATE(58)] = 730,
};

static const TSParseActionEntry ts_parse_actions[] = {
  [0] = {.entry = {.count = 0, .reusable = false}},
  [1] = {.entry = {.count = 1, .reusable = false}}, RECOVER(),
  [3] = {.entry = {.count = 1, .reusable = true}}, SHIFT_EXTRA(),
  [5] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 0, 0, 0),
  [7] = {.entry = {.count = 1, .reusable = true}}, SHIFT(57),
  [9] = {.entry = {.count = 1, .reusable = true}}, SHIFT(58),
  [11] = {.entry = {.count = 1, .reusable = true}}, SHIFT(52),
  [13] = {.entry = {.count = 1, .reusable = true}}, SHIFT(49),
  [15] = {.entry = {.count = 1, .reusable = true}}, SHIFT(26),
  [17] = {.entry = {.count = 1, .reusable = true}}, SHIFT(37),
  [19] = {.entry = {.count = 1, .reusable = true}}, SHIFT(46),
  [21] = {.entry = {.count = 1, .reusable = true}}, SHIFT(53),
  [23] = {.entry = {.count = 1, .reusable = true}}, SHIFT(56),
  [25] = {.entry = {.count = 1, .reusable = true}}, SHIFT(33),
  [27] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(49),
  [30] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0),
  [32] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(37),
  [35] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(46),
  [38] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(53),
  [41] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_agent_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(56),
  [44] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 3, 0, 0),
  [46] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_identifier_list, 2, 0, 0),
  [48] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0),
  [50] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(57),
  [53] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(58),
  [56] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_source_file_repeat1, 2, 0, 0), SHIFT_REPEAT(52),
  [59] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_source_file, 1, 0, 0),
  [61] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 1, 0, 0),
  [63] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 3, 0, 0),
  [65] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_field, 2, 0, 0),
  [67] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_block, 4, 0, 0),
  [69] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0),
  [71] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(22),
  [74] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(40),
  [77] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_client_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(42),
  [80] = {.entry = {.count = 1, .reusable = true}}, SHIFT(34),
  [82] = {.entry = {.count = 1, .reusable = true}}, SHIFT(22),
  [84] = {.entry = {.count = 1, .reusable = true}}, SHIFT(40),
  [86] = {.entry = {.count = 1, .reusable = true}}, SHIFT(42),
  [88] = {.entry = {.count = 1, .reusable = true}}, SHIFT(30),
  [90] = {.entry = {.count = 1, .reusable = true}}, SHIFT(12),
  [92] = {.entry = {.count = 1, .reusable = true}}, SHIFT(47),
  [94] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0),
  [96] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_string_list_repeat1, 2, 0, 0), SHIFT_REPEAT(17),
  [99] = {.entry = {.count = 1, .reusable = true}}, SHIFT(32),
  [101] = {.entry = {.count = 1, .reusable = true}}, SHIFT(45),
  [103] = {.entry = {.count = 1, .reusable = true}}, SHIFT(50),
  [105] = {.entry = {.count = 1, .reusable = true}}, SHIFT(27),
  [107] = {.entry = {.count = 1, .reusable = true}}, SHIFT(10),
  [109] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0),
  [111] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_scope_block_repeat1, 2, 0, 0), SHIFT_REPEAT(47),
  [114] = {.entry = {.count = 1, .reusable = true}}, SHIFT(31),
  [116] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0),
  [118] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(45),
  [121] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_workflow_declaration_repeat1, 2, 0, 0), SHIFT_REPEAT(50),
  [124] = {.entry = {.count = 1, .reusable = true}}, SHIFT(44),
  [126] = {.entry = {.count = 1, .reusable = true}}, SHIFT(17),
  [128] = {.entry = {.count = 1, .reusable = true}}, SHIFT(43),
  [130] = {.entry = {.count = 1, .reusable = true}}, SHIFT(24),
  [132] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 4, 0, 0),
  [134] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 4, 0, 0),
  [136] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_field, 2, 0, 0),
  [138] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_privacy_value, 1, 0, 0),
  [140] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 5, 0, 0),
  [142] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_tier_value, 1, 0, 0),
  [144] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_declaration, 5, 0, 0),
  [146] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_agent_declaration, 5, 0, 0),
  [148] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_client_declaration, 4, 0, 0),
  [150] = {.entry = {.count = 1, .reusable = true}}, SHIFT(6),
  [152] = {.entry = {.count = 1, .reusable = true}}, SHIFT(38),
  [154] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_workflow_field, 2, 0, 0),
  [156] = {.entry = {.count = 1, .reusable = true}}, SHIFT(11),
  [158] = {.entry = {.count = 1, .reusable = true}}, SHIFT(5),
  [160] = {.entry = {.count = 1, .reusable = true}}, SHIFT(39),
  [162] = {.entry = {.count = 1, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0),
  [164] = {.entry = {.count = 2, .reusable = true}}, REDUCE(aux_sym_identifier_list_repeat1, 2, 0, 0), SHIFT_REPEAT(39),
  [167] = {.entry = {.count = 1, .reusable = true}}, SHIFT(28),
  [169] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_scope_field, 2, 0, 0),
  [171] = {.entry = {.count = 1, .reusable = true}}, SHIFT(29),
  [173] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 2, 0, 0),
  [175] = {.entry = {.count = 1, .reusable = true}}, REDUCE(sym_string_list, 3, 0, 0),
  [177] = {.entry = {.count = 1, .reusable = true}}, SHIFT(35),
  [179] = {.entry = {.count = 1, .reusable = true}}, SHIFT(25),
  [181] = {.entry = {.count = 1, .reusable = true}}, SHIFT(2),
  [183] = {.entry = {.count = 1, .reusable = true}}, SHIFT(36),
  [185] = {.entry = {.count = 1, .reusable = true}}, SHIFT(14),
  [187] = {.entry = {.count = 1, .reusable = true}}, SHIFT(55),
  [189] = {.entry = {.count = 1, .reusable = true}},  ACCEPT_INPUT(),
  [191] = {.entry = {.count = 1, .reusable = true}}, SHIFT(19),
  [193] = {.entry = {.count = 1, .reusable = true}}, SHIFT(20),
  [195] = {.entry = {.count = 1, .reusable = true}}, SHIFT(51),
  [197] = {.entry = {.count = 1, .reusable = true}}, SHIFT(48),
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
