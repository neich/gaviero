//! Indent capture types, scopes, and accumulation logic.
//!
//! Implements Helix's capture vocabulary for full `indents.scm` compatibility.

use std::collections::HashMap;

/// The type of an indent capture from a query match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndentCaptureType {
    /// +1 indent level. Non-stacking: multiple on same line = still +1.
    /// Cancelled by `Outdent` on the same line.
    Indent,
    /// -1 indent level. Non-stacking, same cancellation rules as Indent.
    Outdent,
    /// +1 indent, stacks with other IndentAlways on the same line.
    /// If present, Indent on the same line is ignored.
    IndentAlways,
    /// -1 indent, stacks. Net = IndentAlways count - OutdentAlways count.
    OutdentAlways,
    /// Align contents to the column of the paired Anchor.
    Align,
    /// Defines the column for Align. Must appear in the same query pattern.
    Anchor,
    /// Extend the captured node's range downward through subsequent
    /// more-indented lines. Essential for Python.
    Extend,
    /// Block one Extend from applying.
    ExtendPreventOnce,
}

/// The scope of an indent capture — which lines of the node it applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentScope {
    /// Applies to all lines of the node except its first line.
    Tail,
    /// Applies to all lines of the node including the first.
    All,
}

impl IndentCaptureType {
    /// Parse a capture name string into a capture type.
    /// Returns None for unrecognized names.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "indent" => Some(Self::Indent),
            "outdent" => Some(Self::Outdent),
            "indent.always" => Some(Self::IndentAlways),
            "outdent.always" => Some(Self::OutdentAlways),
            "align" => Some(Self::Align),
            "anchor" => Some(Self::Anchor),
            "extend" => Some(Self::Extend),
            "extend.prevent-once" => Some(Self::ExtendPreventOnce),
            _ => None,
        }
    }

    /// Default scope for this capture type.
    pub fn default_scope(&self) -> IndentScope {
        match self {
            Self::Indent | Self::IndentAlways => IndentScope::Tail,
            Self::Outdent | Self::OutdentAlways => IndentScope::All,
            // Align/Anchor/Extend don't use the scope system directly
            Self::Align | Self::Anchor => IndentScope::All,
            Self::Extend | Self::ExtendPreventOnce => IndentScope::All,
        }
    }
}

/// A single indent capture collected during tree traversal.
#[derive(Debug, Clone)]
pub struct IndentCapture {
    pub capture_type: IndentCaptureType,
    pub scope: IndentScope,
    /// The line where this capture takes effect.
    pub effective_line: usize,
    /// The first line of the captured node (used for scope=Tail filtering).
    pub node_first_line: usize,
    /// Column of the node start (used for @anchor).
    pub column: usize,
}

/// Accumulates indent captures and computes the net indent level.
///
/// Groups captures by the line where they take effect, then applies
/// the stacking/cancellation rules from the Helix model:
///
/// Within each line:
/// - If any `IndentAlways` is present: ignore `Indent`. Net = Σ(IndentAlways) - Σ(OutdentAlways).
/// - Else: if both `Indent` and `Outdent` are present, they cancel to 0.
///   Otherwise: net = min(1, indent_count) - min(1, outdent_count).
pub struct LineAccumulator {
    /// Captures grouped by effective line.
    lines: HashMap<usize, Vec<IndentCapture>>,
}

impl LineAccumulator {
    pub fn new() -> Self {
        Self {
            lines: HashMap::new(),
        }
    }

    /// Add a capture, applying scope filtering.
    ///
    /// `new_line` is the line being indented. If `scope == Tail` and
    /// `new_line == node_first_line`, the capture is skipped (tail scope
    /// excludes the node's first line).
    pub fn add(&mut self, capture: IndentCapture, new_line: usize) {
        // Scope filtering: tail scope excludes the node's first line
        if capture.scope == IndentScope::Tail && new_line == capture.node_first_line {
            return;
        }
        self.lines
            .entry(capture.effective_line)
            .or_default()
            .push(capture);
    }

    /// Compute the total indent level from all accumulated captures.
    pub fn compute_level(&self) -> i32 {
        let mut total: i32 = 0;

        for captures in self.lines.values() {
            total += Self::compute_line_level(captures);
        }

        total
    }

    /// Compute the net indent contribution from captures on a single line.
    fn compute_line_level(captures: &[IndentCapture]) -> i32 {
        let has_indent_always = captures
            .iter()
            .any(|c| c.capture_type == IndentCaptureType::IndentAlways);
        let has_outdent_always = captures
            .iter()
            .any(|c| c.capture_type == IndentCaptureType::OutdentAlways);

        if has_indent_always || has_outdent_always {
            // Stacking mode: count IndentAlways and OutdentAlways
            // Indent (non-always) is ignored when IndentAlways is present
            let indent_always_count = captures
                .iter()
                .filter(|c| c.capture_type == IndentCaptureType::IndentAlways)
                .count() as i32;
            let outdent_always_count = captures
                .iter()
                .filter(|c| c.capture_type == IndentCaptureType::OutdentAlways)
                .count() as i32;
            indent_always_count - outdent_always_count
        } else {
            // Non-stacking mode: Indent and Outdent each contribute at most 1
            let has_indent = captures
                .iter()
                .any(|c| c.capture_type == IndentCaptureType::Indent);
            let has_outdent = captures
                .iter()
                .any(|c| c.capture_type == IndentCaptureType::Outdent);

            if has_indent && has_outdent {
                // Cancel each other
                0
            } else if has_indent {
                1
            } else if has_outdent {
                -1
            } else {
                0
            }
        }
    }

    /// Find an @align capture paired with an @anchor, if present.
    /// Returns the anchor column.
    pub fn find_alignment(&self) -> Option<usize> {
        for captures in self.lines.values() {
            let has_align = captures
                .iter()
                .any(|c| c.capture_type == IndentCaptureType::Align);
            if has_align {
                // Find the paired anchor
                if let Some(anchor) = captures
                    .iter()
                    .find(|c| c.capture_type == IndentCaptureType::Anchor)
                {
                    return Some(anchor.column);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(ct: IndentCaptureType, line: usize) -> IndentCapture {
        IndentCapture {
            capture_type: ct,
            scope: ct.default_scope(),
            effective_line: line,
            node_first_line: 0,
            column: 0,
        }
    }

    fn capture_at(ct: IndentCaptureType, eff_line: usize, first_line: usize) -> IndentCapture {
        IndentCapture {
            capture_type: ct,
            scope: ct.default_scope(),
            effective_line: eff_line,
            node_first_line: first_line,
            column: 0,
        }
    }

    #[test]
    fn test_single_indent() {
        let mut acc = LineAccumulator::new();
        acc.add(capture(IndentCaptureType::Indent, 5), 10);
        assert_eq!(acc.compute_level(), 1);
    }

    #[test]
    fn test_single_outdent() {
        let mut acc = LineAccumulator::new();
        acc.add(capture(IndentCaptureType::Outdent, 5), 10);
        assert_eq!(acc.compute_level(), -1);
    }

    #[test]
    fn test_indent_outdent_cancel() {
        let mut acc = LineAccumulator::new();
        acc.add(capture(IndentCaptureType::Indent, 5), 10);
        acc.add(capture(IndentCaptureType::Outdent, 5), 10);
        assert_eq!(acc.compute_level(), 0);
    }

    #[test]
    fn test_multiple_indent_same_line_no_stack() {
        let mut acc = LineAccumulator::new();
        acc.add(capture(IndentCaptureType::Indent, 5), 10);
        acc.add(capture(IndentCaptureType::Indent, 5), 10);
        // Non-stacking: still +1
        assert_eq!(acc.compute_level(), 1);
    }

    #[test]
    fn test_indent_always_stacks() {
        let mut acc = LineAccumulator::new();
        acc.add(capture(IndentCaptureType::IndentAlways, 5), 10);
        acc.add(capture(IndentCaptureType::IndentAlways, 5), 10);
        assert_eq!(acc.compute_level(), 2);
    }

    #[test]
    fn test_indent_always_suppresses_indent() {
        let mut acc = LineAccumulator::new();
        acc.add(capture(IndentCaptureType::IndentAlways, 5), 10);
        acc.add(capture(IndentCaptureType::Indent, 5), 10);
        // IndentAlways present → Indent ignored, net = 1 IndentAlways
        assert_eq!(acc.compute_level(), 1);
    }

    #[test]
    fn test_indent_always_vs_outdent_always() {
        let mut acc = LineAccumulator::new();
        acc.add(capture(IndentCaptureType::IndentAlways, 5), 10);
        acc.add(capture(IndentCaptureType::IndentAlways, 5), 10);
        acc.add(capture(IndentCaptureType::OutdentAlways, 5), 10);
        // 2 - 1 = 1
        assert_eq!(acc.compute_level(), 1);
    }

    #[test]
    fn test_different_lines_accumulate() {
        let mut acc = LineAccumulator::new();
        acc.add(capture(IndentCaptureType::Indent, 3), 10);
        acc.add(capture(IndentCaptureType::Indent, 5), 10);
        // Different lines: each contributes +1
        assert_eq!(acc.compute_level(), 2);
    }

    #[test]
    fn test_scope_tail_filters_first_line() {
        let mut acc = LineAccumulator::new();
        // Capture on node starting at line 5, scope=Tail
        // If new_line is also 5 (the node's first line), capture is skipped
        acc.add(capture_at(IndentCaptureType::Indent, 5, 5), 5);
        assert_eq!(acc.compute_level(), 0);
    }

    #[test]
    fn test_scope_tail_allows_non_first_line() {
        let mut acc = LineAccumulator::new();
        // Node starts at line 3, capture effective at line 3
        // new_line is 5 (not the first line) — should be included
        acc.add(capture_at(IndentCaptureType::Indent, 3, 3), 5);
        assert_eq!(acc.compute_level(), 1);
    }

    #[test]
    fn test_scope_all_includes_first_line() {
        let mut acc = LineAccumulator::new();
        let mut cap = capture_at(IndentCaptureType::Outdent, 5, 5);
        cap.scope = IndentScope::All; // Outdent default is All
        acc.add(cap, 5);
        // Scope All includes first line
        assert_eq!(acc.compute_level(), -1);
    }

    #[test]
    fn test_capture_type_from_name() {
        assert_eq!(IndentCaptureType::from_name("indent"), Some(IndentCaptureType::Indent));
        assert_eq!(IndentCaptureType::from_name("outdent"), Some(IndentCaptureType::Outdent));
        assert_eq!(IndentCaptureType::from_name("indent.always"), Some(IndentCaptureType::IndentAlways));
        assert_eq!(IndentCaptureType::from_name("extend"), Some(IndentCaptureType::Extend));
        assert_eq!(IndentCaptureType::from_name("unknown"), None);
    }
}
