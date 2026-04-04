use std::collections::{HashMap, HashSet, VecDeque};

use super::models::WorkUnit;

/// Errors from scope validation.
#[derive(Debug, Clone)]
pub struct ScopeError {
    pub unit_a: String,
    pub unit_b: String,
    pub overlapping_paths: Vec<String>,
}

impl std::fmt::Display for ScopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "scope overlap between '{}' and '{}': [{}]",
            self.unit_a,
            self.unit_b,
            self.overlapping_paths.join(", ")
        )
    }
}

/// Errors from dependency validation.
#[derive(Debug, Clone)]
pub struct CycleError {
    pub cycle: Vec<String>,
}

impl std::fmt::Display for CycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "dependency cycle: {}", self.cycle.join(" -> "))
    }
}

/// Validate that no two work units have overlapping owned_paths.
///
/// O(n²) pairwise check. Each pair of paths is tested for:
/// - Exact match (both files)
/// - Prefix containment (one or both are directories ending with '/')
pub fn validate_scopes(units: &[WorkUnit]) -> Vec<ScopeError> {
    let mut errors = Vec::new();

    for i in 0..units.len() {
        for j in (i + 1)..units.len() {
            let overlaps = find_overlapping_paths(
                &units[i].scope.owned_paths,
                &units[j].scope.owned_paths,
            );
            if !overlaps.is_empty() {
                errors.push(ScopeError {
                    unit_a: units[i].id.clone(),
                    unit_b: units[j].id.clone(),
                    overlapping_paths: overlaps,
                });
            }
        }
    }

    errors
}

/// Check if two sets of owned paths overlap.
fn find_overlapping_paths(paths_a: &[String], paths_b: &[String]) -> Vec<String> {
    let mut overlaps = Vec::new();

    for a in paths_a {
        for b in paths_b {
            if paths_overlap(a, b) {
                overlaps.push(format!("{} <-> {}", a, b));
            }
        }
    }

    overlaps
}

/// Check if two paths overlap (exact match, or one is a prefix of the other).
fn paths_overlap(a: &str, b: &str) -> bool {
    let a = crate::types::normalize_path(a);
    let b = crate::types::normalize_path(b);
    let a = a.as_str();
    let b = b.as_str();

    if a == b {
        return true;
    }

    // Directory prefix: "src/" contains "src/main.rs"
    let a_is_dir = a.ends_with('/');
    let b_is_dir = b.ends_with('/');

    if a_is_dir && (b.starts_with(a) || b == a.trim_end_matches('/')) {
        return true;
    }
    if b_is_dir && (a.starts_with(b) || a == b.trim_end_matches('/')) {
        return true;
    }

    false
}

/// Compute dependency tiers using Kahn's algorithm (topological sort).
///
/// Returns tiers where each tier is a set of work unit IDs that can execute
/// in parallel (all their dependencies are in earlier tiers).
///
/// Returns `Err(CycleError)` if a dependency cycle is detected.
pub fn dependency_tiers(units: &[WorkUnit]) -> Result<Vec<Vec<String>>, CycleError> {
    let ids: HashSet<&str> = units.iter().map(|u| u.id.as_str()).collect();

    // Build adjacency list and in-degree map
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for unit in units {
        in_degree.entry(unit.id.as_str()).or_insert(0);
        for dep in &unit.depends_on {
            if ids.contains(dep.as_str()) {
                *in_degree.entry(unit.id.as_str()).or_insert(0) += 1;
                dependents.entry(dep.as_str()).or_default().push(&unit.id);
            }
        }
    }

    // Kahn's algorithm
    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|&(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();

    let mut tiers: Vec<Vec<String>> = Vec::new();
    let mut processed = 0;

    while !queue.is_empty() {
        let tier: Vec<String> = queue.drain(..).map(|s| s.to_string()).collect();

        // Reduce in-degree for dependents
        let mut next_queue = VecDeque::new();
        for id in &tier {
            if let Some(deps) = dependents.get(id.as_str()) {
                for &dep_id in deps {
                    let deg = in_degree.get_mut(dep_id).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        next_queue.push_back(dep_id);
                    }
                }
            }
        }

        processed += tier.len();
        tiers.push(tier);
        queue = next_queue;
    }

    if processed != units.len() {
        // Cycle detected — find nodes still with in-degree > 0
        let cycle: Vec<String> = in_degree
            .iter()
            .filter(|&(_, &deg)| deg > 0)
            .map(|(&id, _)| id.to_string())
            .collect();
        return Err(CycleError { cycle });
    }

    Ok(tiers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FileScope;
    use std::collections::HashMap;

    fn scope(owned: &[&str]) -> FileScope {
        FileScope {
            owned_paths: owned.iter().map(|s| s.to_string()).collect(),
            read_only_paths: Vec::new(),
            interface_contracts: HashMap::new(),
        }
    }

    fn unit(id: &str, owned: &[&str], deps: &[&str]) -> WorkUnit {
        WorkUnit {
            id: id.to_string(),
            description: format!("Task {}", id),
            scope: scope(owned),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            backend: Default::default(),
            model: None,
            tier: Default::default(),
            privacy: Default::default(),
            coordinator_instructions: String::new(),
            estimated_tokens: 0,
            max_retries: 1,
            escalation_tier: None,
            read_namespaces: None,
            write_namespace: None,
            memory_importance: None,
            staleness_sources: vec![],
        }
    }

    // ── Scope validation tests ──────────────────────────────────

    #[test]
    fn test_no_overlap_clean_separation() {
        let units = vec![
            unit("a", &["src/auth/"], &[]),
            unit("b", &["src/api/"], &[]),
            unit("c", &["src/db/"], &[]),
        ];
        assert!(validate_scopes(&units).is_empty());
    }

    #[test]
    fn test_overlap_duplicate_paths() {
        let units = vec![
            unit("a", &["src/main.rs"], &[]),
            unit("b", &["src/main.rs"], &[]),
        ];
        let errors = validate_scopes(&units);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].unit_a, "a");
        assert_eq!(errors[0].unit_b, "b");
    }

    #[test]
    fn test_overlap_prefix_containment() {
        let units = vec![
            unit("a", &["src/"], &[]),
            unit("b", &["src/auth/login.rs"], &[]),
        ];
        let errors = validate_scopes(&units);
        assert_eq!(errors.len(), 1);
    }

    // ── Dependency tier tests ───────────────────────────────────

    #[test]
    fn test_linear_chain() {
        let units = vec![
            unit("a", &["src/a/"], &[]),
            unit("b", &["src/b/"], &["a"]),
            unit("c", &["src/c/"], &["b"]),
        ];
        let tiers = dependency_tiers(&units).unwrap();
        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0], vec!["a"]);
        assert_eq!(tiers[1], vec!["b"]);
        assert_eq!(tiers[2], vec!["c"]);
    }

    #[test]
    fn test_diamond_dependency() {
        // a -> b, a -> c, b -> d, c -> d
        let units = vec![
            unit("a", &["src/a/"], &[]),
            unit("b", &["src/b/"], &["a"]),
            unit("c", &["src/c/"], &["a"]),
            unit("d", &["src/d/"], &["b", "c"]),
        ];
        let tiers = dependency_tiers(&units).unwrap();
        assert_eq!(tiers.len(), 3);
        assert!(tiers[0].contains(&"a".to_string()));
        // b and c should be in the same tier
        assert!(tiers[1].contains(&"b".to_string()));
        assert!(tiers[1].contains(&"c".to_string()));
        assert!(tiers[2].contains(&"d".to_string()));
    }

    #[test]
    fn test_no_dependencies_single_tier() {
        let units = vec![
            unit("a", &["src/a/"], &[]),
            unit("b", &["src/b/"], &[]),
            unit("c", &["src/c/"], &[]),
        ];
        let tiers = dependency_tiers(&units).unwrap();
        assert_eq!(tiers.len(), 1);
        assert_eq!(tiers[0].len(), 3);
    }

    #[test]
    fn test_cycle_detected() {
        let units = vec![
            unit("a", &["src/a/"], &["b"]),
            unit("b", &["src/b/"], &["a"]),
        ];
        let err = dependency_tiers(&units).unwrap_err();
        assert!(err.cycle.contains(&"a".to_string()));
        assert!(err.cycle.contains(&"b".to_string()));
    }
}
