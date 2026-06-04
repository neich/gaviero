# Draft research plan: adaptive sparse attention for long-context code models

## Context

We want to study whether learned sparse attention patterns can match dense
transformer quality on repository-scale code contexts (128k+ tokens) while
cutting inference FLOPs by ≥40%. This document is a **research plan draft**
for multi-agent refinement — not an implementation specification.

## Initial objectives (draft)

1. Characterize which long-range dependencies in code corpora are structurally
   predictable (imports, call graph hops, doc references) vs. require dense attention.
2. Compare static sparsity masks (syntax/tree-based) against learned routing
   trained with a distillation objective from a dense teacher.
3. Establish reproducible benchmarks on public code-LM eval suites plus an
   internal held-out repo slice.

## Constraints

- Training budget capped at roughly 8×A100-weeks for the first study tranche.
- Must report latency and memory at 32k, 64k, and 128k context on identical hardware.
- All claims about prior work must be verifiable via Semantic Scholar or primary sources.

## Open questions for refinement

- Which teacher model scale is sufficient for distillation without dominating cost?
- How to separate "sparse attention helps" from "smaller effective receptive field hurts"?
- What stopping rule defines success vs. abandon for the programme?

## Success criteria (provisional)

- Sparse variant within 2% of dense teacher on SWE-bench-lite and a custom
  long-file completion set, with ≥40% attention FLOP reduction at 64k context.
- Ablations published with seeds, configs, and negative results documented.
