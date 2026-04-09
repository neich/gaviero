//! Privacy scanner: evaluates PrivacyLevel for files based on glob patterns.
//!
//! The scanner overrides coordinator-suggested privacy levels to LocalOnly
//! when file paths match configured patterns. This is a safety net — the
//! privacy decision is never purely LLM-determined.

use crate::types::PrivacyLevel;
use crate::swarm::models::WorkUnit;

/// Evaluates privacy levels based on configured glob patterns.
#[derive(Debug, Clone)]
pub struct PrivacyScanner {
    patterns: Vec<GlobPattern>,
}

/// A simple glob pattern matcher (supports `*` and `**`).
#[derive(Debug, Clone)]
struct GlobPattern {
    raw: String,
}

impl GlobPattern {
    fn new(pattern: &str) -> Self {
        Self { raw: pattern.to_string() }
    }

    /// Check if a path matches this pattern.
    ///
    /// Supports:
    /// - `*` matches anything within a path segment
    /// - `**` matches any number of path segments
    /// - Exact prefix matching for directory patterns
    fn matches(&self, path: &str) -> bool {
        let pattern = self.raw.trim_start_matches("./");
        let path = path.trim_start_matches("./");

        // Handle ** patterns by extracting literal segments between **'s
        // and checking that all literal segments appear in order in the path.
        if pattern.contains("**") {
            // Split pattern on ** and / to get the literal segments
            // E.g. "**/clinical/**" → literal segments: ["clinical"]
            // E.g. "data/**/secret/**" → literal segments: ["data", "secret"]
            let segments: Vec<&str> = pattern
                .split("**")
                .flat_map(|part| part.split('/'))
                .filter(|s| !s.is_empty() && *s != "*")
                .collect();

            if segments.is_empty() {
                return true; // Pattern is just "**" — matches everything
            }

            // Check that all literal segments appear as path components in order
            let path_parts: Vec<&str> = path.split('/').collect();
            let mut pi = 0;
            for seg in &segments {
                let found = path_parts[pi..].iter().position(|p| p == seg);
                match found {
                    Some(idx) => pi += idx + 1,
                    None => return false,
                }
            }
            return true;
        }

        // Handle * wildcard in simple patterns
        if pattern.contains('*') {
            let parts: Vec<&str> = pattern.split('*').collect();
            if parts.len() == 2 {
                return path.starts_with(parts[0]) && path.ends_with(parts[1]);
            }
        }

        // Directory prefix matching
        if pattern.ends_with('/') {
            return path.starts_with(pattern) || path.starts_with(pattern.trim_end_matches('/'));
        }

        // Exact match
        path == pattern
    }
}

impl PrivacyScanner {
    /// Create a scanner from a list of glob patterns.
    ///
    /// Patterns are typically from `agent.routing.privacyPatterns` in settings.
    pub fn new(patterns: &[String]) -> Self {
        Self {
            patterns: patterns.iter().map(|p| GlobPattern::new(p)).collect(),
        }
    }

    /// Check if any file in a WorkUnit's scope matches privacy patterns.
    ///
    /// Returns `LocalOnly` if any owned or read-only path matches,
    /// otherwise returns the unit's existing privacy level.
    pub fn classify(&self, unit: &WorkUnit) -> PrivacyLevel {
        // If already LocalOnly, keep it
        if unit.privacy == PrivacyLevel::LocalOnly {
            return PrivacyLevel::LocalOnly;
        }

        // Check all paths in scope
        for path in unit.scope.owned_paths.iter().chain(unit.scope.read_only_paths.iter()) {
            if self.matches_any(path) {
                return PrivacyLevel::LocalOnly;
            }
        }

        unit.privacy
    }

    /// Apply privacy overrides to all units in a list.
    ///
    /// This mutates units in-place, upgrading privacy from Public to LocalOnly
    /// where patterns match. Never downgrades from LocalOnly.
    pub fn apply_overrides(&self, units: &mut [WorkUnit]) {
        for unit in units {
            let classified = self.classify(unit);
            if classified == PrivacyLevel::LocalOnly {
                unit.privacy = PrivacyLevel::LocalOnly;
            }
        }
    }

    /// Check if a path matches any configured privacy pattern.
    pub fn matches_any(&self, path: &str) -> bool {
        self.patterns.iter().any(|p| p.matches(path))
    }

    /// Return true if the scanner has any patterns configured.
    pub fn has_patterns(&self) -> bool {
        !self.patterns.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileScope;
    use std::collections::HashMap;

    fn make_unit(owned: &[&str], privacy: PrivacyLevel) -> WorkUnit {
        WorkUnit {
            id: "test".into(),
            description: "test".into(),
            scope: FileScope {
                owned_paths: owned.iter().map(|s| s.to_string()).collect(),
                read_only_paths: vec![],
                interface_contracts: HashMap::new(),
            },
            depends_on: vec![],
            backend: Default::default(),
            model: None,
            tier: Default::default(),
            privacy,
            coordinator_instructions: String::new(),
            estimated_tokens: 0,
            max_retries: 1,
            escalation_tier: None,
            read_namespaces: None,
            write_namespace: None,
            memory_importance: None,
            staleness_sources: vec![],
            memory_read_query: None,
            memory_read_limit: None,
            memory_write_content: None,
            impact_scope: false,
            context_callers_of: vec![],
            context_tests_for: vec![],
            context_depth: 2,
        }
    }

    #[test]
    fn test_double_star_pattern() {
        let scanner = PrivacyScanner::new(&["**/clinical/**".into()]);
        assert!(scanner.matches_any("data/clinical/patient.csv"));
        assert!(scanner.matches_any("src/clinical/analysis.py"));
        assert!(!scanner.matches_any("src/auth/login.rs"));
    }

    #[test]
    fn test_directory_pattern() {
        let scanner = PrivacyScanner::new(&["data/grading/".into()]);
        assert!(scanner.matches_any("data/grading/scores.csv"));
        assert!(scanner.matches_any("data/grading/report.pdf"));
        assert!(!scanner.matches_any("data/public/report.pdf"));
    }

    #[test]
    fn test_wildcard_pattern() {
        let scanner = PrivacyScanner::new(&["*.env".into()]);
        assert!(scanner.matches_any("production.env"));
        assert!(scanner.matches_any(".env"));
        assert!(!scanner.matches_any("src/config.rs"));
    }

    #[test]
    fn test_exact_match() {
        let scanner = PrivacyScanner::new(&["secrets.json".into()]);
        assert!(scanner.matches_any("secrets.json"));
        assert!(!scanner.matches_any("config.json"));
    }

    #[test]
    fn test_classify_public_stays_public() {
        let scanner = PrivacyScanner::new(&["**/clinical/**".into()]);
        let unit = make_unit(&["src/auth/login.rs"], PrivacyLevel::Public);
        assert_eq!(scanner.classify(&unit), PrivacyLevel::Public);
    }

    #[test]
    fn test_classify_overrides_to_local_only() {
        let scanner = PrivacyScanner::new(&["**/clinical/**".into()]);
        let unit = make_unit(&["data/clinical/analysis.py"], PrivacyLevel::Public);
        assert_eq!(scanner.classify(&unit), PrivacyLevel::LocalOnly);
    }

    #[test]
    fn test_classify_never_downgrades() {
        let scanner = PrivacyScanner::new(&[]); // No patterns
        let unit = make_unit(&["data/clinical/analysis.py"], PrivacyLevel::LocalOnly);
        assert_eq!(scanner.classify(&unit), PrivacyLevel::LocalOnly);
    }

    #[test]
    fn test_apply_overrides() {
        let scanner = PrivacyScanner::new(&["**/clinical/**".into()]);
        let mut units = vec![
            make_unit(&["src/auth.rs"], PrivacyLevel::Public),
            make_unit(&["data/clinical/data.csv"], PrivacyLevel::Public),
        ];
        scanner.apply_overrides(&mut units);
        assert_eq!(units[0].privacy, PrivacyLevel::Public);
        assert_eq!(units[1].privacy, PrivacyLevel::LocalOnly);
    }

    #[test]
    fn test_empty_scanner() {
        let scanner = PrivacyScanner::new(&[]);
        assert!(!scanner.has_patterns());
        let unit = make_unit(&["anything.rs"], PrivacyLevel::Public);
        assert_eq!(scanner.classify(&unit), PrivacyLevel::Public);
    }

    #[test]
    fn test_multiple_patterns() {
        let scanner = PrivacyScanner::new(&[
            "**/clinical/**".into(),
            "**/grading/**".into(),
            "*.env".into(),
        ]);
        assert!(scanner.matches_any("data/clinical/x.csv"));
        assert!(scanner.matches_any("src/grading/calc.py"));
        assert!(scanner.matches_any("production.env"));
        assert!(!scanner.matches_any("src/lib.rs"));
    }
}
