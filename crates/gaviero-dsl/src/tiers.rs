//! Load `tier <alias> <client>` bindings from a profile file for CLI overrides.

use std::collections::HashSet;
use std::path::Path;

use miette::NamedSource;

use crate::ast::{Item, Script};
use crate::error::{DslError, DslErrors};
use crate::resolver;

/// Parse a `.gaviero` tiers profile: top-level `tier <alias> <client-ref>` lines only.
///
/// `include "..."` is allowed so profiles can live beside shared libraries; other
/// top-level items (`client`, `agent`, `workflow`, …) are rejected.
pub fn load_tier_overrides(path: &Path) -> Result<Vec<(String, String)>, miette::Report> {
    let (script, sources) = resolver::resolve(path).map_err(miette::Report::new)?;
    extract_tier_overrides(&script, &sources).map_err(miette::Report::new)
}

fn extract_tier_overrides(
    script: &Script,
    sources: &[(String, String)],
) -> Result<Vec<(String, String)>, DslErrors> {
    let src_for = |fid: u32| -> NamedSource<String> {
        let (filename, content) = &sources[fid as usize];
        NamedSource::new(filename, content.clone())
    };

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut errors = Vec::new();

    for item in &script.items {
        match item {
            Item::TierAlias(ta) => {
                if !seen.insert(ta.name.clone()) {
                    errors.push(DslError::Compile {
                        src: src_for(ta.file_id),
                        span: (
                            ta.name_span.start,
                            ta.name_span.end.saturating_sub(ta.name_span.start).max(1),
                        )
                            .into(),
                        reason: format!("duplicate tier alias `{}`", ta.name),
                    });
                    continue;
                }
                out.push((ta.name.clone(), ta.client_ref.clone()));
            }
            Item::Vars(_) => {}
            Item::Include(inc) => {
                errors.push(DslError::Compile {
                    src: src_for(0),
                    span: (
                        inc.span.start,
                        inc.span.end.saturating_sub(inc.span.start).max(1),
                    )
                        .into(),
                    reason:
                        "unexpected `include` after resolution in tiers profile — file a bug"
                            .into(),
                });
            }
            other => {
                let (span, label) = match other {
                    Item::Client(c) => (c.span, "client"),
                    Item::Agent(a) => (a.span, "agent"),
                    Item::Workflow(w) => (w.span, "workflow"),
                    Item::Prompt(p) => (p.span, "prompt"),
                    Item::TierAlias(_) => unreachable!(),
                    Item::Vars(_) | Item::Include(_) => unreachable!(),
                };
                errors.push(DslError::Compile {
                    src: src_for(0),
                    span: (span.start, span.end.saturating_sub(span.start).max(1)).into(),
                    reason: format!(
                        "tiers profile must contain only `tier` declarations, found `{label}`"
                    ),
                });
            }
        }
    }

    if !errors.is_empty() {
        return Err(DslErrors::new(errors));
    }
    if out.is_empty() {
        return Err(DslErrors::single(DslError::Compile {
            src: src_for(0),
            span: (0, 0).into(),
            reason: "tiers profile contains no `tier <alias> <client>` bindings".into(),
        }));
    }
    Ok(out)
}
