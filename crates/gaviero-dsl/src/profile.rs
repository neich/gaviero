//! Model profiles: the one place a tier gets its model.
//!
//! A profile is a `.gaviero` file of `client` and `tier` declarations (plus
//! `include`s of the same). It is self-contained: its tiers may only point at
//! its own clients, and those clients are renamed on merge, so a script
//! cannot depend on them by name. Scripts declare neither clients nor tier
//! lines ([`reject_model_tables`]); they reach a profile through
//! `tier <name>` alone.

use std::collections::HashSet;

use gaviero_core::script_profiles::SelectedProfile;
use gaviero_core::swarm::backend::effort::{is_known_effort, known_effort_names};
use miette::NamedSource;

use crate::ast::{ClientDecl, Item, Script, Span, TierAlias};
use crate::error::{DslError, DslErrors};
use crate::resolver;

/// One run's change to what a tier means: another client of the same
/// profile (`--tier-client TIER=CLIENT`), another effort
/// (`--tier-effort TIER=EFFORT`), or both. Models stay declared only in the
/// profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierOverride {
    pub tier: String,
    /// A client the profile declares; `None` keeps the tier's own.
    pub client: Option<String>,
    /// An effort name; `None` keeps the client's.
    pub effort: Option<String>,
}

impl TierOverride {
    /// Merge `--tier-client` and `--tier-effort` pairs into one override per
    /// tier, in order of first mention.
    pub fn from_pairs(clients: &[(String, String)], efforts: &[(String, String)]) -> Vec<Self> {
        let mut overrides: Vec<Self> = Vec::new();
        let mut entry = |tier: &str| -> usize {
            match overrides.iter().position(|o| o.tier == tier) {
                Some(i) => i,
                None => {
                    overrides.push(Self {
                        tier: tier.to_string(),
                        client: None,
                        effort: None,
                    });
                    overrides.len() - 1
                }
            }
        };
        let mut set = Vec::new();
        for (tier, client) in clients {
            set.push((entry(tier), Some(client.clone()), None));
        }
        for (tier, effort) in efforts {
            set.push((entry(tier), None, Some(effort.clone())));
        }
        for (i, client, effort) in set {
            if client.is_some() {
                overrides[i].client = client;
            }
            if effort.is_some() {
                overrides[i].effort = effort;
            }
        }
        overrides
    }
}

/// Name a profile client takes inside the merged script.
pub fn profile_client_name(client: &str) -> String {
    format!("__profile_{client}")
}

/// Reject `client { }` and `tier <alias> <client>` in a script (or a file it
/// includes): what a tier means belongs to the model profile, never to the
/// script.
pub(crate) fn reject_model_tables(
    script: &Script,
    sources: &[(String, String)],
) -> Result<(), DslErrors> {
    const WHY: &str = "models are chosen in one place: a model profile \
                       (.gaviero/dsl/profiles/<name>.gaviero) maps tiers to clients, \
                       and agents pick a tier with `tier <name>`";
    let mut errors = Vec::new();
    for item in &script.items {
        let (file_id, span, what) = match item {
            Item::Client(c) => (c.file_id, c.span, format!("`client {}`", c.name)),
            Item::TierAlias(t) => (
                t.file_id,
                t.span,
                format!("`tier {} {}`", t.name, t.client_ref),
            ),
            _ => continue,
        };
        let (filename, content) = &sources[file_id as usize];
        errors.push(DslError::Compile {
            src: NamedSource::new(filename, content.clone()),
            span: (span.start, span.end.saturating_sub(span.start).max(1)).into(),
            reason: format!("a script cannot declare {what} — {WHY}"),
        });
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(DslErrors::new(errors))
    }
}

/// Merge `selected` into `script`. Diagnostics for the profile point at the
/// profile's own files, which are appended to `sources`.
///
/// `overrides` change tiers for this run only, to other clients of the same
/// profile and/or another effort: the profile stays the one place a model
/// is declared. A tier the profile lacks is added (it needs a client).
pub(crate) fn apply_profile(
    script: &mut Script,
    sources: &mut Vec<(String, String)>,
    selected: &SelectedProfile,
    overrides: &[TierOverride],
) -> Result<(), DslErrors> {
    let (profile, profile_sources) = resolver::resolve(&selected.path)?;
    let (mut clients, mut tiers) = split_profile(profile, &profile_sources)?;
    rebind_tiers(&mut clients, &mut tiers, overrides)?;

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

/// Apply each override: point the tier at the named client, then, for an
/// effort, at a copy of its client with that effort (named
/// `<client>__<tier>`, so the profile's own client and every other tier on
/// it are untouched). A rebound tier keeps the span of the line it replaces;
/// an added tier takes its client's.
fn rebind_tiers(
    clients: &mut Vec<ClientDecl>,
    tiers: &mut Vec<TierAlias>,
    overrides: &[TierOverride],
) -> Result<(), DslErrors> {
    let mut errors = Vec::new();
    let error = |reason: String| DslError::Compile {
        src: NamedSource::new("<run>", String::new()),
        span: (0, 0).into(),
        reason,
    };
    for o in overrides {
        let tier = &o.tier;
        let client_name = match (&o.client, tiers.iter().find(|t| t.name == *tier)) {
            (Some(name), _) => name.clone(),
            (None, Some(existing)) => existing.client_ref.clone(),
            (None, None) => {
                errors.push(error(format!(
                    "--tier-effort {tier}=…: the profile has no tier `{tier}`; \
                     give it a client with --tier-client {tier}=CLIENT"
                )));
                continue;
            }
        };
        let Some(client) = clients.iter().find(|c| c.name == client_name).cloned() else {
            let known: Vec<&str> = clients.iter().map(|c| c.name.as_str()).collect();
            errors.push(error(format!(
                "--tier-client {tier}={client_name}: the profile has no client \
                 `{client_name}` (it has: {})",
                known.join(", ")
            )));
            continue;
        };
        let bound = match &o.effort {
            None => client.name.clone(),
            Some(effort) if !is_known_effort(effort) => {
                errors.push(error(format!(
                    "--tier-effort {tier}={effort}: unknown effort (known: {})",
                    known_effort_names()
                )));
                continue;
            }
            Some(effort) => {
                let mut copy = client.clone();
                copy.name = format!("{}__{tier}", client.name);
                copy.effort = Some((effort.clone(), client.name_span));
                clients.retain(|c| c.name != copy.name);
                let name = copy.name.clone();
                clients.push(copy);
                name
            }
        };
        match tiers.iter_mut().find(|t| t.name == *tier) {
            Some(existing) => existing.client_ref = bound,
            None => tiers.push(TierAlias {
                name: tier.clone(),
                name_span: client.name_span,
                client_ref: bound,
                client_ref_span: client.name_span,
                span: client.span,
                file_id: client.file_id,
            }),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(DslErrors::new(errors))
    }
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
        if client.is_default {
            errors.push(error_at(
                client.file_id,
                client.name_span,
                format!(
                    "client `{}` is marked `default`, which no longer means anything — \
                     every agent declares a tier; remove `default`",
                    client.name
                ),
            ));
        }
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
