//! Pattern sugar: expand high-level workflow patterns into plan fields.
//!
//! ## Supported
//!
//! - `pattern map_reduce { discover … reduce … max_spawn … }` → one
//!   [`FanoutOp`] with `after_unit = discover` and the given `max_spawn`
//!   (or [`DEFAULT_MAX_SPAWN`] when omitted).
//!
//! ## Not sugar
//!
//! - **Consensus** — use existing `loop { reviewers … consensus_mode … }`
//!   (see `examples/generic_consensus.gaviero`). There is no
//!   `pattern consensus` expansion in v1.

use gaviero_core::swarm::plan::{DEFAULT_MAX_SPAWN, FanoutOp};

use crate::ast::{MapReducePattern, PatternDecl, Span};

/// Error from expanding a [`PatternDecl`] (missing/invalid fields).
#[derive(Debug, Clone)]
pub struct PatternExpandError {
    pub span: Span,
    pub reason: String,
}

/// Expand a workflow pattern into [`FanoutOp`]s for `CompiledPlan.fanout_ops`.
pub fn expand_fanout_ops(pattern: &PatternDecl) -> Result<Vec<FanoutOp>, PatternExpandError> {
    match pattern {
        PatternDecl::MapReduce(mr) => expand_map_reduce(mr),
    }
}

fn expand_map_reduce(mr: &MapReducePattern) -> Result<Vec<FanoutOp>, PatternExpandError> {
    if mr.discover.0.is_empty() {
        return Err(PatternExpandError {
            span: mr.span,
            reason: "`pattern map_reduce` requires `discover <agent>`".into(),
        });
    }
    if mr.reduce.0.is_empty() {
        return Err(PatternExpandError {
            span: mr.span,
            reason: "`pattern map_reduce` requires `reduce <agent>`".into(),
        });
    }

    let max_spawn = mr.max_spawn.map(|(n, _)| n).unwrap_or(DEFAULT_MAX_SPAWN);

    Ok(vec![FanoutOp {
        after_unit: mr.discover.0.clone(),
        max_spawn,
        default_model: None,
        barrier_id: None,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use chumsky::span::SimpleSpan;

    fn span() -> Span {
        SimpleSpan::from(0..1)
    }

    #[test]
    fn map_reduce_emits_one_fanout_op() {
        let pat = PatternDecl::MapReduce(MapReducePattern {
            discover: ("discoverer".into(), span()),
            reduce: ("aggregator".into(), span()),
            max_spawn: Some((8, span())),
            span: span(),
        });
        let ops = expand_fanout_ops(&pat).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].after_unit, "discoverer");
        assert_eq!(ops[0].max_spawn, 8);
        assert!(ops[0].default_model.is_none());
        assert!(ops[0].barrier_id.is_none());
    }

    #[test]
    fn map_reduce_defaults_max_spawn() {
        let pat = PatternDecl::MapReduce(MapReducePattern {
            discover: ("d".into(), span()),
            reduce: ("r".into(), span()),
            max_spawn: None,
            span: span(),
        });
        let ops = expand_fanout_ops(&pat).unwrap();
        assert_eq!(ops[0].max_spawn, DEFAULT_MAX_SPAWN);
    }

    #[test]
    fn map_reduce_requires_discover() {
        let pat = PatternDecl::MapReduce(MapReducePattern {
            discover: (String::new(), span()),
            reduce: ("r".into(), span()),
            max_spawn: None,
            span: span(),
        });
        assert!(expand_fanout_ops(&pat).is_err());
    }
}
