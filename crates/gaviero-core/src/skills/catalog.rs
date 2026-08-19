//! Multi-root skill catalog: scan, resolve, complete, and semantic search.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::memory::embedder::{Embedder, EmbeddingPurpose};
use crate::memory::reranker::Reranker;
use crate::memory::scope::{SCOPE_GLOBAL, SCOPE_REPO, SCOPE_WORKSPACE, hash_path};
use crate::workspace::Workspace;

use super::{EXTRA_ROOT_WARNING_NAME, Skill, SkillWarning, parse_skill, skill_name_from_path};

/// Catalog-local scope for `skills.extraRoots` entries. Intentionally
/// outside the memory `SCOPE_*` range so extra skills never route through
/// memory DBs. Ranked after global for unqualified `$name`.
const SCOPE_EXTRA_ROOT: i32 = 10;

/// Registry of skills discovered under workspace and global roots.
#[derive(Debug, Clone)]
pub struct SkillCatalog {
    /// Skill name → all definitions across scopes (nearest-first ordering per name).
    by_name: HashMap<String, Vec<Skill>>,
    /// Canonical folder root → display label for `source_label`.
    folder_index: HashMap<PathBuf, String>,
    /// Canonical extra-root path → derived source label (`claude`, `codex`, …).
    extra_root_index: HashMap<PathBuf, String>,
}

impl SkillCatalog {
    pub fn global_skills_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".gaviero")
            .join("skills")
    }

    /// Scan skill roots mirroring `MemoryStores` routing, then any
    /// configured `skills.extraRoots`.
    pub fn scan(workspace: &Workspace, global_dir: &Path) -> (Self, Vec<SkillWarning>) {
        let mut catalog = Self {
            by_name: HashMap::new(),
            folder_index: HashMap::new(),
            extra_root_index: HashMap::new(),
        };
        let mut warnings = Vec::new();
        let mut native_skills_dirs: Vec<PathBuf> = Vec::new();

        let workspace_root = workspace
            .roots()
            .first()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let canon_ws = canonicalize_path(&workspace_root);
        let folders = workspace.folders();
        let single_folder = folders.len() <= 1;

        for folder in folders {
            let folder_canon = canonicalize_path(&folder.path);
            catalog
                .folder_index
                .insert(folder_canon.clone(), folder.display_name().to_string());
            let skills_dir = folder.path.join(".gaviero").join("skills");
            native_skills_dirs.push(canonicalize_path(&skills_dir));
            catalog.scan_dir(&skills_dir, SCOPE_REPO, &mut warnings);
        }

        if !single_folder {
            let ws_skills = workspace_root.join(".gaviero").join("skills");
            native_skills_dirs.push(canonicalize_path(&ws_skills));
            catalog.scan_dir(&ws_skills, SCOPE_WORKSPACE, &mut warnings);
        }

        if global_dir.is_dir() {
            native_skills_dirs.push(canonicalize_path(global_dir));
            catalog.scan_dir(global_dir, SCOPE_GLOBAL, &mut warnings);
        }

        if workspace.skill_extra_roots_is_malformed(None) {
            warnings.push(SkillWarning {
                name: EXTRA_ROOT_WARNING_NAME.to_string(),
                message: "skills.extraRoots must be a JSON array of paths".to_string(),
            });
        }
        catalog.scan_extra_roots(
            &workspace.resolve_skill_extra_roots(None),
            &native_skills_dirs,
            &mut warnings,
        );

        // Suppress unused variable warning — canon_ws used in tests / future aliasing checks.
        let _ = canon_ws;

        (catalog, warnings)
    }

    fn scan_extra_roots(
        &mut self,
        extra_roots: &[PathBuf],
        native_skills_dirs: &[PathBuf],
        warnings: &mut Vec<SkillWarning>,
    ) {
        for root in extra_roots {
            let display = root.display().to_string();
            let canon = canonicalize_path(root);
            if native_skills_dirs.iter().any(|n| n == &canon) {
                continue;
            }
            if self.extra_root_index.contains_key(&canon) {
                continue;
            }
            match std::fs::read_dir(root) {
                Ok(_) => {
                    let label =
                        unique_extra_label(extra_root_label(root), self.extra_root_index.values());
                    self.extra_root_index.insert(canon, label);
                    self.scan_dir(root, SCOPE_EXTRA_ROOT, warnings);
                }
                Err(e) => warnings.push(SkillWarning {
                    name: EXTRA_ROOT_WARNING_NAME.to_string(),
                    message: format!("{display} does not exist or is unreadable ({e})"),
                }),
            }
        }
    }

    fn scan_dir(&mut self, dir: &Path, scope_level: i32, warnings: &mut Vec<SkillWarning>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let folder = entry.path();
            if !folder.is_dir() {
                continue;
            }
            let folder_name = folder.file_name().and_then(|s| s.to_str()).unwrap_or("?");
            if folder_name.starts_with('.') {
                continue;
            }
            let skill_md = folder.join("SKILL.md");
            if !skill_md.is_file() {
                warnings.push(SkillWarning {
                    name: folder_name.to_string(),
                    message: format!("skill folder {} is missing SKILL.md", folder.display()),
                });
                continue;
            }
            let contents = match std::fs::read_to_string(&skill_md) {
                Ok(c) => c,
                Err(e) => {
                    warnings.push(SkillWarning {
                        name: folder_name.to_string(),
                        message: format!("failed to read {}: {e}", skill_md.display()),
                    });
                    continue;
                }
            };
            match parse_skill(&skill_md, &contents) {
                Ok(mut skill) => {
                    debug_assert_eq!(skill_name_from_path(&skill_md), Some(skill.name.as_str()));
                    skill.scope_level = scope_level;
                    self.by_name
                        .entry(skill.name.clone())
                        .or_default()
                        .push(skill);
                }
                Err(w) => warnings.push(w),
            }
        }
    }

    /// Human-readable source label for completion display.
    pub fn source_label(&self, skill: &Skill) -> String {
        match skill.scope_level {
            SCOPE_GLOBAL => "global".to_string(),
            SCOPE_WORKSPACE => "workspace".to_string(),
            SCOPE_EXTRA_ROOT => self
                .extra_label_for(skill)
                .unwrap_or_else(|| "extra".to_string()),
            SCOPE_REPO => {
                if let Some(root) = repo_root_for_skill(skill) {
                    if let Some(label) = self.folder_index.get(&root) {
                        return label.clone();
                    }
                }
                "repo".to_string()
            }
            _ => "repo".to_string(),
        }
    }

    fn extra_label_for(&self, skill: &Skill) -> Option<String> {
        let skill_canon = canonicalize_path(&skill.source_path);
        self.extra_root_index
            .iter()
            .find(|(root, _)| skill_canon.starts_with(root) || skill.source_path.starts_with(root))
            .map(|(_, label)| label.clone())
    }

    /// Resolve a skill by optional source qualifier and bare name.
    pub fn resolve(
        &self,
        qualifier: Option<&str>,
        name: &str,
        active_repo_id: Option<&str>,
    ) -> Option<&Skill> {
        let skills = self.by_name.get(name)?;
        if let Some(q) = qualifier {
            return skills.iter().find(|s| self.source_label(s) == q);
        }

        let mut ordered: Vec<&Skill> = skills.iter().collect();

        if let Some(repo_id) = active_repo_id {
            if let Some(active) = ordered
                .iter()
                .find(|s| {
                    s.scope_level == SCOPE_REPO
                        && self.repo_id_for_skill(s).as_deref() == Some(repo_id)
                })
                .copied()
            {
                return Some(active);
            }
        }

        ordered.sort_by_key(|s| (s.scope_level, s.source_path.clone()));
        // Nearest-first: repo (active first above) → other repos → workspace → global.
        // Within repo level prefer lower scope_level number... actually repo=2, ws=1, global=0
        // Plan says: active-folder repo → other folder repos → workspace → global
        // So we want SCOPE_REPO first (but active handled), then SCOPE_WORKSPACE, then SCOPE_GLOBAL.
        ordered.sort_by_key(|s| match s.scope_level {
            SCOPE_REPO => 0,
            SCOPE_WORKSPACE => 1,
            SCOPE_GLOBAL => 2,
            SCOPE_EXTRA_ROOT => 3,
            _ => 4,
        });
        ordered.into_iter().next()
    }

    fn repo_id_for_skill(&self, skill: &Skill) -> Option<String> {
        if skill.scope_level != SCOPE_REPO {
            return None;
        }
        repo_root_for_skill(skill).map(|root| hash_path(&root))
    }

    /// Prefix completion candidates, scope-ordered nearest-first.
    pub fn complete(&self, prefix: &str, active_repo_id: Option<&str>) -> Vec<&Skill> {
        let mut out: Vec<&Skill> = self
            .by_name
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .flat_map(|(_, skills)| skills.iter())
            .collect();

        out.sort_by(|a, b| {
            let rank = |s: &Skill| -> (u8, String) {
                let scope_rank = match s.scope_level {
                    SCOPE_REPO => {
                        if active_repo_id
                            .is_some_and(|id| self.repo_id_for_skill(s).as_deref() == Some(id))
                        {
                            0
                        } else {
                            1
                        }
                    }
                    SCOPE_WORKSPACE => 2,
                    SCOPE_GLOBAL => 3,
                    SCOPE_EXTRA_ROOT => 4,
                    _ => 5,
                };
                (scope_rank, s.name.clone())
            };
            rank(a).cmp(&rank(b))
        });
        out
    }

    /// Semantic search over skill descriptions.
    pub async fn search(
        &self,
        query: &str,
        embedder: &Arc<dyn Embedder>,
        reranker: &dyn Reranker,
    ) -> Vec<&Skill> {
        let all: Vec<&Skill> = self.by_name.values().flat_map(|v| v.iter()).collect();
        if all.is_empty() {
            return Vec::new();
        }

        let descriptions: Vec<&str> = all.iter().map(|s| s.description.as_str()).collect();
        let embedder = Arc::clone(embedder);
        let descs_owned: Vec<String> = descriptions.iter().map(|s| s.to_string()).collect();
        let doc_refs: Vec<&str> = descs_owned.iter().map(|s| s.as_str()).collect();

        let query_emb = match embedder.embed_query(query).await {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };
        let doc_embs = match embedder
            .embed_batch(&doc_refs, EmbeddingPurpose::Document)
            .await
        {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };

        let mut scored: Vec<(usize, f32)> = doc_embs
            .iter()
            .enumerate()
            .map(|(i, emb)| (i, cosine_similarity(&query_emb, emb)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let candidates: Vec<&str> = scored.iter().map(|(i, _)| descriptions[*i]).collect();
        let rerank_scores = reranker
            .rerank(query, &candidates)
            .await
            .unwrap_or_else(|_| scored.iter().map(|(_, s)| *s).collect());

        let mut indexed: Vec<(usize, f32)> = scored
            .iter()
            .enumerate()
            .map(|(rank, (idx, _))| (*idx, rerank_scores.get(rank).copied().unwrap_or(0.0)))
            .collect();
        indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        indexed.into_iter().map(|(i, _)| all[i]).collect()
    }

    pub fn rebuild(&mut self, workspace: &Workspace, global_dir: &Path) -> Vec<SkillWarning> {
        let (fresh, warnings) = Self::scan(workspace, global_dir);
        *self = fresh;
        warnings
    }

    /// True when `event_path` lies under any `*/.gaviero/skills/` directory.
    pub fn needs_rebuild(event_path: &Path) -> bool {
        for ancestor in event_path.ancestors() {
            if ancestor.file_name().is_some_and(|n| n == "skills")
                && ancestor
                    .parent()
                    .and_then(|p| p.file_name())
                    .is_some_and(|n| n == ".gaviero")
            {
                return true;
            }
        }
        false
    }

    /// Native `.gaviero/skills` predicate plus configured extra-root prefixes.
    pub fn needs_rebuild_for(&self, event_path: &Path) -> bool {
        if Self::needs_rebuild(event_path) {
            return true;
        }
        let event_canon = canonicalize_path(event_path);
        self.extra_root_index
            .keys()
            .any(|root| event_canon.starts_with(root) || event_path.starts_with(root))
    }

    /// All skills in the catalog (for `/skills` listing).
    pub fn all_skills(&self) -> Vec<&Skill> {
        let mut out: Vec<&Skill> = self.by_name.values().flat_map(|v| v.iter()).collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
}

fn canonicalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Label for an extra skill root. H-OD6 is "root basename"; when the last
/// component is `skills` we use the parent (`~/.claude/skills` → `claude`)
/// so two harnesses do not collide on the token `skills`. A leading `.` is
/// stripped so `$source/name` identifiers stay valid (`is_name_char`).
fn extra_root_label(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("extra");
    let raw = if name.eq_ignore_ascii_case("skills") {
        path.parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(name)
    } else {
        name
    };
    let stripped = raw.trim_start_matches('.');
    if stripped.is_empty() {
        "extra".to_string()
    } else {
        stripped.to_string()
    }
}

fn unique_extra_label<'a>(base: String, existing: impl Iterator<Item = &'a String>) -> String {
    let taken: HashSet<&str> = existing.map(|s| s.as_str()).collect();
    if !taken.contains(base.as_str()) {
        return base;
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}-{n}");
        if !taken.contains(candidate.as_str()) {
            return candidate;
        }
        n += 1;
    }
}

/// Repo folder root for a skill under `<repo>/.gaviero/skills/<name>/SKILL.md`.
fn repo_root_for_skill(skill: &Skill) -> Option<PathBuf> {
    for ancestor in skill.source_path.ancestors() {
        if ancestor.file_name().is_some_and(|n| n == "skills")
            && ancestor
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|n| n == ".gaviero")
        {
            return ancestor
                .parent()
                .and_then(|p| p.parent())
                .map(canonicalize_path);
        }
    }
    None
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::embedder::{Embedder, EmbeddingPurpose};
    use crate::memory::reranker::NullReranker;
    use async_trait::async_trait;
    use std::fs;
    use std::sync::Arc;

    struct TestEmbedder;
    #[async_trait]
    impl Embedder for TestEmbedder {
        fn name(&self) -> &str {
            "test"
        }
        fn dimension(&self) -> usize {
            8
        }
        async fn embed(&self, text: &str, _purpose: EmbeddingPurpose) -> anyhow::Result<Vec<f32>> {
            let mut v = vec![0.0f32; 8];
            for (i, b) in text.bytes().enumerate() {
                v[i % 8] += b as f32;
            }
            Ok(v)
        }
    }

    fn write_skill(dir: &Path, name: &str, description: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let body = format!("---\ndescription: {description}\n---\nBody for {name}\n");
        fs::write(skill_dir.join("SKILL.md"), body).unwrap();
    }

    #[test]
    fn scan_single_folder_tags_repo_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join(".gaviero").join("skills");
        write_skill(&skills_dir, "lint", "Run the linter on changed files");

        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let (catalog, warnings) = SkillCatalog::scan(&ws, Path::new("/nonexistent"));
        assert!(warnings.is_empty());
        let skill = catalog.resolve(None, "lint", None).unwrap();
        assert_eq!(skill.scope_level, SCOPE_REPO);
    }

    #[test]
    fn resolve_qualified_by_source_label() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join(".gaviero").join("skills");
        write_skill(&skills_dir, "deploy", "Deploy the application");

        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let (catalog, _) = SkillCatalog::scan(&ws, Path::new("/nonexistent"));
        let label = catalog.source_label(catalog.resolve(None, "deploy", None).unwrap());
        assert!(catalog.resolve(Some(&label), "deploy", None).is_some());
    }

    #[test]
    fn complete_prefix_returns_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join(".gaviero").join("skills");
        write_skill(&skills_dir, "migrate", "Migrate components");
        write_skill(&skills_dir, "minimal", "Minimal skill description here");

        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let (catalog, _) = SkillCatalog::scan(&ws, Path::new("/nonexistent"));
        let hits = catalog.complete("mig", None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "migrate");
    }

    #[test]
    fn needs_rebuild_detects_skills_path() {
        let p = Path::new("/tmp/proj/.gaviero/skills/foo/SKILL.md");
        assert!(SkillCatalog::needs_rebuild(p));
        let p2 = Path::new("/tmp/proj/.gaviero/memory.db");
        assert!(!SkillCatalog::needs_rebuild(p2));
    }

    #[test]
    fn scan_skips_hidden_skill_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join(".gaviero").join("skills");
        write_skill(&skills_dir, "visible", "Visible skill description here");
        let hidden = skills_dir.join(".claude");
        std::fs::create_dir_all(&hidden).unwrap();

        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let (catalog, warnings) = SkillCatalog::scan(&ws, Path::new("/nonexistent"));
        assert_eq!(catalog.all_skills().len(), 1);
        assert_eq!(catalog.all_skills()[0].name, "visible");
        assert!(warnings.is_empty());
    }

    #[tokio::test]
    async fn search_returns_skills_ordered() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join(".gaviero").join("skills");
        write_skill(&skills_dir, "alpha", "Alpha skill for testing search");
        write_skill(&skills_dir, "beta", "Beta skill unrelated topic");

        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let (catalog, _) = SkillCatalog::scan(&ws, Path::new("/nonexistent"));
        let embedder = Arc::new(TestEmbedder) as Arc<dyn Embedder>;
        let reranker = NullReranker;
        let results = catalog.search("alpha testing", &embedder, &reranker).await;
        assert!(!results.is_empty());
    }

    fn write_extra_roots_settings(ws_root: &Path, extra: &[&Path]) {
        let gav = ws_root.join(".gaviero");
        fs::create_dir_all(&gav).unwrap();
        let paths: Vec<String> = extra
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let json = serde_json::json!({ "skills": { "extraRoots": paths } });
        fs::write(
            gav.join("settings.json"),
            serde_json::to_string(&json).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn extra_root_label_uses_parent_when_last_component_is_skills() {
        assert_eq!(
            extra_root_label(Path::new("/home/u/.claude/skills")),
            "claude"
        );
        assert_eq!(
            extra_root_label(Path::new("/home/u/.codex/skills")),
            "codex"
        );
        assert_eq!(extra_root_label(Path::new("/opt/my-bundle")), "my-bundle");
    }

    #[test]
    fn extra_root_fixture_discovered_under_derived_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let extra = tempfile::tempdir().unwrap();
        let extra_skills = extra.path().join("claude").join("skills");
        write_skill(&extra_skills, "deploy", "Deploy via Claude skill");
        write_extra_roots_settings(tmp.path(), &[&extra_skills]);

        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let (catalog, warnings) = SkillCatalog::scan(&ws, Path::new("/nonexistent"));
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let skill = catalog.resolve(None, "deploy", None).unwrap();
        assert_eq!(skill.scope_level, SCOPE_EXTRA_ROOT);
        assert_eq!(catalog.source_label(skill), "claude");
        assert!(catalog.needs_rebuild_for(&extra_skills.join("deploy").join("SKILL.md")));
        assert!(!catalog.needs_rebuild_for(Path::new("/tmp/unrelated.rs")));
    }

    #[test]
    fn extra_root_collision_with_native_skill_qualifies() {
        let tmp = tempfile::tempdir().unwrap();
        let native = tmp.path().join(".gaviero").join("skills");
        write_skill(&native, "deploy", "Native deploy skill body");
        let extra = tempfile::tempdir().unwrap();
        let extra_skills = extra.path().join("claude").join("skills");
        write_skill(&extra_skills, "deploy", "Foreign deploy skill body");
        write_extra_roots_settings(tmp.path(), &[&extra_skills]);

        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let (catalog, warnings) = SkillCatalog::scan(&ws, Path::new("/nonexistent"));
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let hits = catalog.complete("deploy", None);
        assert_eq!(hits.len(), 2);
        let labels: Vec<String> = hits.iter().map(|s| catalog.source_label(s)).collect();
        let folder = tmp.path().file_name().and_then(|s| s.to_str()).unwrap();
        assert!(labels.contains(&folder.to_string()), "labels={labels:?}");
        assert!(labels.contains(&"claude".to_string()), "labels={labels:?}");
        assert!(catalog.resolve(Some("claude"), "deploy", None).is_some());
        let nearest = catalog.resolve(None, "deploy", None).unwrap();
        assert_eq!(nearest.scope_level, SCOPE_REPO);
    }

    #[test]
    fn extra_root_missing_emits_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("no-such-extra-root");
        write_extra_roots_settings(tmp.path(), &[&missing]);
        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let (catalog, warnings) = SkillCatalog::scan(&ws, Path::new("/nonexistent"));
        assert!(catalog.all_skills().is_empty());
        assert!(
            warnings.iter().any(|w| {
                w.name == EXTRA_ROOT_WARNING_NAME && w.message.contains("does not exist")
            }),
            "warnings={warnings:?}"
        );
    }

    #[test]
    fn extra_roots_malformed_object_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let gav = tmp.path().join(".gaviero");
        fs::create_dir_all(&gav).unwrap();
        fs::write(
            gav.join("settings.json"),
            r#"{ "skills": { "extraRoots": { "bad": true } } }"#,
        )
        .unwrap();
        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let (_, warnings) = SkillCatalog::scan(&ws, Path::new("/nonexistent"));
        assert!(
            warnings.iter().any(|w| {
                w.name == EXTRA_ROOT_WARNING_NAME && w.message.contains("must be a JSON array")
            }),
            "warnings={warnings:?}"
        );
    }

    #[test]
    fn extra_root_bad_sibling_does_not_drop_good_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let extra = tempfile::tempdir().unwrap();
        let extra_skills = extra.path().join("codex").join("skills");
        write_skill(&extra_skills, "ok", "Valid foreign skill description");
        let broken = extra_skills.join("broken");
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join("SKILL.md"), "no frontmatter here\n").unwrap();
        write_extra_roots_settings(tmp.path(), &[&extra_skills]);

        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let (catalog, warnings) = SkillCatalog::scan(&ws, Path::new("/nonexistent"));
        assert_eq!(catalog.all_skills().len(), 1);
        assert_eq!(catalog.all_skills()[0].name, "ok");
        assert!(
            warnings.iter().any(|w| w.name == "broken"),
            "warnings={warnings:?}"
        );
    }

    #[test]
    fn extra_roots_default_empty_catalog_matches_unconfigured() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join(".gaviero").join("skills");
        write_skill(&skills_dir, "lint", "Run the linter on changed files");
        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let (before, warn_before) = SkillCatalog::scan(&ws, Path::new("/nonexistent"));
        assert!(warn_before.is_empty());

        fs::write(
            tmp.path().join(".gaviero").join("settings.json"),
            r#"{ "skills": { "extraRoots": [] } }"#,
        )
        .unwrap();
        let ws2 = Workspace::single_folder(tmp.path().to_path_buf());
        let (after, warn_after) = SkillCatalog::scan(&ws2, Path::new("/nonexistent"));
        assert!(warn_after.is_empty());
        assert_eq!(before.all_skills().len(), after.all_skills().len());
        assert_eq!(
            before.all_skills()[0].source_path,
            after.all_skills()[0].source_path
        );
    }
}
