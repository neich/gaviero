//! Provider-parity drift guard.
//!
//! `plans/provider-parity` §3.4, decision 5. This asserts the **declared**
//! capability table (`ProviderProfile`, filled by `build_provider_profile`)
//! against the **artifacts it claims to describe** — the per-vendor config
//! files `mcp::config_synth` actually renders, and the enforcement/prompt
//! mechanisms each provider really has.
//!
//! The point is that drift becomes a failing test instead of a finding in the
//! third review. The concrete bug this class already produced: the table said
//! `cursor: context7_allowed = true` while the synthesizer reached Cursor's
//! context7 by **hardcoding a URL** and never reading the row, so the table was
//! inert (fixed in §2.9). The vendor-config assertions below fail if an emitter
//! ever goes back to deciding for itself.
//!
//! Honest scope note: because §2.9 wired every emitter *to* the table, the
//! rendered-config-vs-row comparison is **consistent by construction today**.
//! It is load-bearing on the next edit, exactly as the §2.9 mutation test is.
//! What is *not* tautological here is the second half of each test: that the
//! declared row matches the mechanism the provider actually uses.
//!
//! Runs offline — no binary, no network, no `E2E_AGENT_MODEL`.

use gaviero_core::context_planner::types::McpCapabilities;
use gaviero_core::context_planner::{
    ModelSpec, PromptKind, Provider, ProviderProfile, RuntimeConfig, ToolEnforcement,
    build_provider_profile,
};
use gaviero_core::mcp::config_synth::{
    McpConfigSynth, claude_mcp_config_json, codex_mcp_config_toml, cursor_mcp_config_json,
};

/// Every provider arm, as the model spec an operator would type.
const PROVIDER_SPECS: &[&str] = &[
    "claude:sonnet",
    "codex:x",
    "codex-app-server:gpt-5",
    "cursor:x",
    "dsh:x",
    "deepseek:x",
    "ollama:x",
];

fn profile(spec: &str) -> ProviderProfile {
    build_provider_profile(&ModelSpec::parse(spec), &RuntimeConfig::default())
}

/// A synth with context7 on and nothing else exotic, so a vendor config that
/// omits context7 is omitting it by decision rather than by missing input.
fn synth_for(dir: &std::path::Path) -> McpConfigSynth {
    let mut synth = McpConfigSynth::default();
    synth.worktree = dir.to_path_buf();
    synth.enabled = true;
    synth.gaviero_enabled = true;
    synth.context7.enabled = true;
    assert!(
        synth.context7.enabled && synth.permissions.server_allowed("context7"),
        "fixture precondition: context7 must survive the permission gate, \
         or this file asserts nothing"
    );
    synth
}

/// The rendered config text for one synthesizer vendor key.
fn render(synth: &McpConfigSynth, vendor: &str) -> String {
    match vendor {
        "claude" => claude_mcp_config_json(synth).expect("claude .mcp.json renders"),
        "cursor" => cursor_mcp_config_json(synth).expect("cursor .mcp.json renders"),
        "codex" => codex_mcp_config_toml(synth).expect("codex config.toml renders"),
        other => panic!("unknown synth vendor {other}"),
    }
}

/// The three vendor keys `Provider::synth_vendor` can name.
const VENDORS: &[&str] = &["claude", "cursor", "codex"];

/// The rendered config for a vendor must contain a context7 server entry iff
/// that vendor's declared row allows context7.
#[test]
fn rendered_vendor_configs_agree_with_the_declared_row() {
    let dir = tempfile::tempdir().expect("tempdir");
    let synth = synth_for(dir.path());

    for vendor in VENDORS {
        let declared = McpCapabilities::for_synth_vendor(vendor);
        let text = render(&synth, vendor);
        assert_eq!(
            text.contains("context7"),
            declared.context7,
            "{vendor}: declared context7={} but the rendered config \
             {} a context7 entry. Config text:\n{text}",
            declared.context7,
            if text.contains("context7") {
                "contains"
            } else {
                "omits"
            },
        );

        // The row the emitter consults must itself be the row the provider
        // arms declare — otherwise the config is faithful to a lookup that
        // disagrees with the table.
        let owners: Vec<Provider> = Provider::ALL
            .iter()
            .copied()
            .filter(|p| p.synth_vendor() == Some(vendor))
            .collect();
        assert!(
            !owners.is_empty(),
            "{vendor} has no owning Provider arm, so the reverse lookup \
             `for_synth_vendor` is reaching its permissive fallback"
        );
        for owner in owners {
            assert_eq!(
                owner.mcp_capabilities(),
                declared,
                "{owner:?} and the {vendor} vendor lookup disagree"
            );
        }
    }
}

/// The provider rows and the vendor reverse-lookup are one answer, and only
/// the config-file vendors have a vendor at all.
#[test]
fn provider_rows_and_the_vendor_lookup_share_one_source() {
    for provider in Provider::ALL {
        if let Some(vendor) = provider.synth_vendor() {
            assert_eq!(
                provider.mcp_capabilities(),
                McpCapabilities::for_synth_vendor(vendor),
                "{provider:?} ({vendor}) — forward and reverse lookups disagree"
            );
            assert!(
                VENDORS.contains(&vendor),
                "{provider:?} names vendor {vendor}, which this test does not render"
            );
        }
    }

    // Exactly the providers that never receive a synthesized config file:
    // the two in-process loops, and dsh (an ACP session, not a config file).
    let mut vendorless: Vec<String> = Provider::ALL
        .iter()
        .filter(|p| p.synth_vendor().is_none())
        .map(|p| format!("{p:?}"))
        .collect();
    vendorless.sort();
    assert_eq!(vendorless, vec!["Deepseek", "Dsh", "Ollama"]);

    // A vendor the table does not name must not be silently muted (§2.9).
    assert!(McpCapabilities::for_synth_vendor("not-a-vendor").context7);
}

/// The enforcement and prompt axes must name the mechanism the provider
/// actually has — this is the half of the file that cannot be satisfied by
/// wiring the emitters to the table.
///
/// Decision 1 (dsh is declared unenforced, not attempted) and decision 3
/// (every model reaches a multi-choice ask tool) both land here.
#[test]
fn declared_enforcement_and_prompt_axes_match_the_real_mechanism() {
    let expected: &[(&str, ToolEnforcement, PromptKind)] = &[
        // Build-time argv; AskUserQuestion + y/n.
        ("claude:sonnet", ToolEnforcement::Argv, PromptKind::MultiChoice),
        // `codex exec` is non-interactive and carries no tool list.
        ("codex:x", ToolEnforcement::Unenforced, PromptKind::None),
        // Runtime host approval; y/n only.
        (
            "codex-app-server:gpt-5",
            ToolEnforcement::RuntimeHost,
            PromptKind::YesNo,
        ),
        // Build-time deny rules; headless `-p` has no prompt channel.
        (
            "cursor:x",
            ToolEnforcement::GeneratedConfig,
            PromptKind::None,
        ),
        // Decision 1: dsh keeps its y/n channel but no tool policing.
        ("dsh:x", ToolEnforcement::Unenforced, PromptKind::YesNo),
        // Registry membership; decision 3 gave both a multi-choice ask tool.
        (
            "deepseek:x",
            ToolEnforcement::RegistryMembership,
            PromptKind::MultiChoice,
        ),
        (
            "ollama:x",
            ToolEnforcement::RegistryMembership,
            PromptKind::MultiChoice,
        ),
    ];

    assert_eq!(
        expected.len(),
        PROVIDER_SPECS.len(),
        "every provider spec must be covered by an enforcement expectation"
    );

    for (spec, enforcement, prompt) in expected {
        let p = profile(spec);
        assert_eq!(p.tool_enforcement, *enforcement, "{spec}: enforcement");
        assert_eq!(p.prompt_kind, *prompt, "{spec}: prompt kind");
        assert_eq!(
            p.prompt_kind.has_multi_choice(),
            *prompt == PromptKind::MultiChoice,
            "{spec}: has_multi_choice disagrees with the kind"
        );
    }
}

/// Phase 6 (decision 1): the structurally unenforced rows are the only ones
/// that disclose a warning, and they do disclose one — a declaration the user
/// cannot see is not a declaration.
#[test]
fn only_unenforced_providers_disclose_a_warning() {
    let mut disclosing: Vec<String> = Vec::new();
    for spec in PROVIDER_SPECS {
        let p = profile(spec);
        match p.tool_enforcement.ui_disclosure(&p.provider) {
            Some(msg) => {
                assert!(
                    msg.contains(&p.provider),
                    "{spec}: the disclosure must name the provider: {msg}"
                );
                assert_eq!(
                    p.tool_enforcement,
                    ToolEnforcement::Unenforced,
                    "{spec}: disclosed a warning while claiming enforcement"
                );
                disclosing.push(p.provider.clone());
            }
            None => assert_ne!(
                p.tool_enforcement,
                ToolEnforcement::Unenforced,
                "{spec}: unenforced but silent — the user is never told"
            ),
        }
    }
    assert_eq!(disclosing, vec!["codex".to_string(), "dsh".to_string()]);
}

/// The declared MCP transport must agree with whether a `mcpServers` entry is
/// even meaningful — the regression §2.10 records (an in-process provider was
/// about to be handed a context7 *MCP server* once 2d made `context7` true for
/// every row).
#[test]
fn in_process_rows_do_not_claim_an_mcp_server_transport() {
    for spec in PROVIDER_SPECS {
        let p = profile(spec);
        let caps = p.mcp_capabilities();
        assert_eq!(
            caps.uses_mcp_servers(),
            caps.transport != gaviero_core::context_planner::McpTransport::InProcess,
            "{spec}: uses_mcp_servers disagrees with the declared transport"
        );
    }

    // The in-process loops reach context7 as a *native tool* (§2.10), so they
    // allow the capability without having an MCP transport to receive it on.
    for spec in ["deepseek:x", "ollama:x"] {
        let caps = profile(spec).mcp_capabilities();
        assert!(caps.context7, "{spec} should reach context7 natively");
        assert!(
            !caps.uses_mcp_servers(),
            "{spec} must not register an MCP server entry"
        );
    }
}
