//! Merge a named model profile into a resolved script.
//!
//! A profile is a `.gaviero` file of `client` and `tier` declarations (plus
//! `include`s of the same). It is self-contained: its tiers may only point at
//! its own clients, and those clients are renamed on merge, so a profile never
//! collides with what the script declares and a script cannot depend on it by
//! client name. Scripts reach a profile through `tier <name>` alone.

use std::collections::HashSet;

use gaviero_core::script_profiles::{ProfilePrecedence, SelectedProfile};
use miette::NamedSource;

use crate::ast::{ClientDecl, Item, Script, Span, TierAlias};
use crate::error::{DslError, DslErrors};
use crate::resolver;

/// Name a profile client takes inside the merged script.
pub fn profile_client_name(client: &str) -> String {
    format!("__profile_{client}")
}

/// Merge `selected` into `script`. Diagnostics for the profile point at the
/// profile's own files, which are appended to `sources`.
pub(crate) fn apply_profile(
    script: &mut Script,
    sources: &mut Vec<(String, String)>,
    selected: &SelectedProfile,
) -> Result<(), DslErrors> {
    let (profile, profile_sources) = resolver::resolve(&selected.path)?;
    let (mut clients, mut tiers) = split_profile(profile, &profile_sources)?;

    match selected.precedence {
        ProfilePrecedence::OverridesScript => {
            let bound: HashSet<&str> = tiers.iter().map(|t| t.name.as_str()).collect();
            script
                .items
                .retain(|i| !matches!(i, Item::TierAlias(t) if bound.contains(t.name.as_str())));
            if clients.iter().any(|c| c.is_default) {
                for item in &mut script.items {
                    if let Item::Client(c) = item {
                        c.is_default = false;
                    }
                }
            }
        }
        ProfilePrecedence::FillsGaps => {
            let bound: HashSet<String> = script
                .items
                .iter()
                .filter_map(|i| match i {
                    Item::TierAlias(t) => Some(t.name.clone()),
                    _ => None,
                })
                .collect();
            tiers.retain(|t| !bound.contains(&t.name));
            let script_has_default = script
                .items
                .iter()
                .any(|i| matches!(i, Item::Client(c) if c.is_default));
            if script_has_default {
                for client in &mut clients {
                    client.is_default = false;
                }
            }
        }
    }

    let offset = sources.len() as u32;
    sources.extend(profile_sources);
    for mut client in clients {
        client.name = profile_client_name(&client.name);
        client.file_id += offset;
        script.items.push(Item::Client(client));
    }
    for mut tier in tiers {
        tier.client_ref = profile_client_name(&tier.client_ref);
        tier.file_id += offset;
        script.items.push(Item::TierAlias(tier));
    }
    Ok(())
}

/// Separate a resolved profile into its clients and tiers, rejecting anything
/// that would make it more than a model table.
fn split_profile(
    profile: Script,
    sources: &[(String, String)],
) -> Result<(Vec<ClientDecl>, Vec<TierAlias>), DslErrors> {
    let error_at = |file_id: u32, span: Span, reason: String| {
        let (filename, content) = &sources[file_id as usize];
        DslError::Compile {
            src: NamedSource::new(filename, content.clone()),
            span: (span.start, span.end.saturating_sub(span.start).max(1)).into(),
            reason,
        }
    };

    let mut clients = Vec::new();
    let mut tiers = Vec::new();
    let mut errors = Vec::new();
    let mut not_allowed = |file_id: u32, span: Span, label: &str| {
        errors.push(error_at(
            file_id,
            span,
            format!("a profile may only declare `client` and `tier`, found `{label}`"),
        ));
    };

    for item in profile.items {
        match item {
            Item::Client(c) => clients.push(c),
            Item::TierAlias(t) => tiers.push(t),
            Item::Agent(a) => not_allowed(a.file_id, a.span, "agent"),
            Item::Workflow(w) => not_allowed(w.file_id, w.span, "workflow"),
            Item::Prompt(p) => not_allowed(p.file_id, p.span, "prompt"),
            Item::Vars(_) => not_allowed(0, (0..0).into(), "vars"),
            Item::Include(inc) => not_allowed(0, inc.span, "include"),
        }
    }

    let mut declared = HashSet::new();
    for client in &clients {
        if !declared.insert(client.name.as_str()) {
            errors.push(error_at(
                client.file_id,
                client.name_span,
                format!("duplicate client name `{}`", client.name),
            ));
        }
    }
    for tier in &tiers {
        if !declared.contains(tier.client_ref.as_str()) {
            errors.push(error_at(
                tier.file_id,
                tier.client_ref_span,
                format!(
                    "tier `{}` references client `{}`, which this profile does not declare",
                    tier.name, tier.client_ref
                ),
            ));
        }
    }
    if tiers.is_empty() {
        errors.push(error_at(
            0,
            (0..0).into(),
            "profile contains no `tier <name> <client>` bindings".into(),
        ));
    }

    if errors.is_empty() {
        Ok((clients, tiers))
    } else {
        Err(DslErrors::new(errors))
    }
}
