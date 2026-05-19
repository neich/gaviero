use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Well-known setting keys. Use these constants instead of raw strings
/// to get compile-time typo detection.
pub mod settings {
    pub const TAB_SIZE: &str = "editor.tabSize";
    pub const INSERT_SPACES: &str = "editor.insertSpaces";
    pub const FORMAT_ON_SAVE: &str = "editor.formatOnSave";
    pub const FILES_EXCLUDE: &str = "files.exclude";
    pub const FILE_TREE_WIDTH: &str = "panels.fileTree.width";
    pub const SIDE_PANEL_WIDTH: &str = "panels.sidePanel.width";
    pub const TERMINAL_SPLIT_PERCENT: &str = "panels.terminal.splitPercent";
    pub const GIT_TREE_ALLOW_LIST: &str = "git.treeAllowList";

    // Agent / Claude settings
    pub const AGENT_MODEL: &str = "agent.model";
    pub const AGENT_EFFORT: &str = "agent.effort";
    pub const AGENT_MAX_TOKENS: &str = "agent.maxTokens";
    pub const AGENT_OLLAMA_BASE_URL: &str = "agent.ollamaBaseUrl";
    /// Token budget for graph-based source-code context injection in simple chat. 0 disables.
    pub const AGENT_GRAPH_BUDGET_TOKENS: &str = "agent.graphBudgetTokens";

    pub const AGENT_TOPOLOGY_ENABLED: &str = "agent.topology.enabled";
    pub const AGENT_TOPOLOGY_MAX_DEPTH: &str = "agent.topology.maxDepth";
    pub const AGENT_TOPOLOGY_MAX_DIRS: &str = "agent.topology.maxDirs";
    pub const AGENT_TOPOLOGY_TOKEN_BUDGET: &str = "agent.topology.tokenBudget";
    /// Tool surface offered to the Claude subprocess via `--tools`.
    /// Anything outside this list is unavailable to the agent regardless
    /// of `--allowedTools`. Default omits `Bash` so agent writes flow
    /// through the Write Gate; opt in per-workspace to enable shell.
    pub const AGENT_AVAILABLE_TOOLS: &str = "agent.availableTools";
    /// Subset of `agent.availableTools` auto-approved without a permission
    /// prompt (passed via `--allowedTools`). Anything available but not
    /// approved triggers a `PermissionRequest` that the host must answer.
    pub const AGENT_APPROVED_TOOLS: &str = "agent.approvedTools";

    // Memory settings
    /// The namespace to write memories to.
    pub const MEMORY_NAMESPACE: &str = "memory.namespace";
    /// Additional namespaces to search when reading (the write namespace is always included).
    pub const MEMORY_READ_NAMESPACES: &str = "memory.readNamespaces";

    // Coordinator settings (tier routing)
    pub const COORDINATOR_MODEL: &str = "agent.coordinator.model";
    pub const COORDINATOR_MAX_CONTEXT_TOKENS: &str = "agent.coordinator.maxContextTokens";

    // Tier settings
    pub const TIER_REASONING_MODEL: &str = "agent.tiers.reasoning.model";
    pub const TIER_REASONING_MAX_PARALLEL: &str = "agent.tiers.reasoning.maxParallel";
    pub const TIER_EXECUTION_MODEL: &str = "agent.tiers.execution.model";
    pub const TIER_EXECUTION_MAX_PARALLEL: &str = "agent.tiers.execution.maxParallel";
    pub const TIER_MECHANICAL_ENABLED: &str = "agent.tiers.mechanical.enabled";
    pub const TIER_MECHANICAL_BACKEND: &str = "agent.tiers.mechanical.backend";
    pub const TIER_MECHANICAL_MODEL: &str = "agent.tiers.mechanical.ollamaModel";
    pub const TIER_MECHANICAL_BASE_URL: &str = "agent.tiers.mechanical.ollamaBaseUrl";
    pub const TIER_MECHANICAL_MAX_PARALLEL: &str = "agent.tiers.mechanical.maxParallel";

    // Routing settings
    pub const ROUTING_PRIVACY_PATTERNS: &str = "agent.routing.privacyPatterns";
    pub const ROUTING_ESCALATION_ENABLED: &str = "agent.routing.escalationEnabled";
    pub const ROUTING_COST_BUDGET: &str = "agent.routing.costBudget";

    // Verification settings
    pub const VERIFICATION_DEFAULT_STRATEGY: &str = "agent.verification.defaultStrategy";
    pub const VERIFICATION_TEST_COMMAND: &str = "agent.verification.testSuite.command";
    pub const VERIFICATION_TEST_TIMEOUT: &str = "agent.verification.testSuite.timeout";

    // Memory enrichment settings
    pub const MEMORY_ENRICH_COORDINATOR: &str = "agent.memory.enrichCoordinator";
    pub const MEMORY_ENRICH_AGENTS: &str = "agent.memory.enrichAgents";
    pub const MEMORY_COORDINATOR_LIMIT: &str = "agent.memory.coordinatorMemoryLimit";
    pub const MEMORY_AGENT_LIMIT: &str = "agent.memory.agentMemoryLimit";

    // Chat memory injection (Tier S / S1)
    pub const MEMORY_CHAT_INJECTION_ENABLED: &str = "memory.chatInjection.enabled";
    pub const MEMORY_CHAT_INJECTION_SCOPES: &str = "memory.chatInjection.scopes";
    pub const MEMORY_CHAT_INJECTION_MAX_ITEMS: &str = "memory.chatInjection.maxItems";
    pub const MEMORY_CHAT_INJECTION_TOKEN_BUDGET: &str = "memory.chatInjection.tokenBudget";
    pub const MEMORY_CHAT_INJECTION_MIN_SIM: &str = "memory.chatInjection.minSimilarity";

    // Retrieval-manifest persistence (Tier S / S4)
    pub const MEMORY_MANIFESTS_ENABLED: &str = "memory.manifests.enabled";
    pub const MEMORY_MANIFESTS_RETENTION_DAYS: &str = "memory.manifests.retentionDays";
    pub const MEMORY_MANIFESTS_CAPTURE_POOL: &str = "memory.manifests.captureCandidatePool";

    // Per-turn extractor (Tier S / S3)
    pub const MEMORY_EXTRACTOR_ENABLED: &str = "memory.extractor.enabled";
    pub const MEMORY_EXTRACTOR_MODEL: &str = "memory.extractor.model";
    pub const MEMORY_EXTRACTOR_EVERY_N_TURNS: &str = "memory.extractor.everyNTurns";
    pub const MEMORY_EXTRACTOR_MAX_PER_TURN: &str = "memory.extractor.maxExtractionsPerTurn";

    // Tier B / B4 — recency floor + decay-exempt types
    pub const MEMORY_SCORING_RECENCY_FLOOR: &str = "memory.scoring.recencyFloor";
    pub const MEMORY_SCORING_DECAY_EXEMPT_TYPES: &str = "memory.scoring.decayExemptTypes";

    // Tier B / B1 — embedder selection + re-embed batch size
    pub const MEMORY_EMBEDDER_MODEL: &str = "memory.embedder.model";
    pub const MEMORY_EMBEDDER_REEMBED_BATCH_SIZE: &str = "memory.embedder.reembedBatchSize";

    // Tier B / B2 — cross-encoder reranker
    pub const MEMORY_RERANKER_ENABLED: &str = "memory.reranker.enabled";
    pub const MEMORY_RERANKER_MODEL: &str = "memory.reranker.model";
    pub const MEMORY_RERANKER_POOL_SIZE: &str = "memory.reranker.candidatePoolSize";
    pub const MEMORY_RERANKER_BLEND_WEIGHT: &str = "memory.reranker.blendWeight";
    pub const MEMORY_RERANKER_MAX_LATENCY_MS: &str = "memory.reranker.maxLatencyMs";

    // Tier C / C3-C4 — repo-map specificity and typed edge settings
    pub const REPO_MAP_SPECIFICITY_ENABLED: &str = "repoMap.specificity.enabled";
    pub const REPO_MAP_SPECIFICITY_STOP_SYMBOL_THRESHOLD: &str =
        "repoMap.specificity.stopSymbolThreshold";
    pub const REPO_MAP_TYPED_EDGES_ENABLED: &str = "repoMap.edges.typed";
    /// C4: per-intent edge weight overrides. Keyed by intent name
    /// (`"impact"`, `"callers"`, `"tests"`, `"implementations"`,
    /// `"all"`); each value is a JSON object whose keys are
    /// camelCase EdgeKind names (`"calls"`, `"imports"`,
    /// `"implements"`, `"defines"`, `"testOf"`,
    /// `"referencesDocstringOf"`, `"declaresContractWith"`) and whose
    /// values are floats clamped to `[0.0, 1.0]`. Missing intents and
    /// missing kinds keep the plan's preset.
    ///
    /// Example settings.json fragment:
    /// ```json
    /// "repoMap.edges.weights": {
    ///     "impact":  { "imports": 0.7, "testOf": 0.1 },
    ///     "callers": { "imports": 0.4 }
    /// }
    /// ```
    pub const REPO_MAP_EDGE_WEIGHTS: &str = "repoMap.edges.weights";

    // Tier B / B3 — merged multi-scope retrieval mode + per-scope cap
    pub const MEMORY_RETRIEVAL_MODE: &str = "memory.retrieval.mode";
    pub const MEMORY_RETRIEVAL_PER_SCOPE_TOP_K: &str = "memory.retrieval.perScopeTopK";
    pub const MEMORY_RETRIEVAL_MAX_MERGED_POOL: &str = "memory.retrieval.maxMergedPool";

    // Tier B / B5 — session consolidator + sleeptime
    pub const MEMORY_SESSION_CONSOLIDATE_ON_CLOSE: &str = "memory.session.consolidateOnClose";
    pub const MEMORY_SESSION_IDLE_TIMEOUT_SEC: &str = "memory.session.idleTimeoutSec";
    pub const MEMORY_SLEEPTIME_ENABLED: &str = "memory.sleeptime.enabled";
    pub const MEMORY_SLEEPTIME_MIN_IDLE_MINUTES: &str = "memory.sleeptime.minIdleMinutes";
    pub const MEMORY_SLEEPTIME_WEEKLY_FORCE_RUN: &str = "memory.sleeptime.weeklyForceRun";
    pub const MEMORY_SLEEPTIME_NEAR_DUP_THRESHOLD: &str = "memory.sleeptime.nearDupThreshold";
    pub const MEMORY_SLEEPTIME_FIRST_RUN_REQUIRE_CONFIRM: &str =
        "memory.sleeptime.firstRunRequireConfirm";

    // Tier B / B6 — retrieval-use telemetry
    pub const MEMORY_TELEMETRY_ENABLED: &str = "memory.telemetry.enabled";
    pub const MEMORY_TELEMETRY_USED_THRESHOLD: &str = "memory.telemetry.usedThreshold";
    pub const MEMORY_TELEMETRY_PARTIAL_THRESHOLD: &str = "memory.telemetry.partialThreshold";
    pub const MEMORY_TELEMETRY_MIN_INJECTIONS_FOR_TRUST: &str =
        "memory.telemetry.minInjectionsForTrustAdjust";
    pub const MEMORY_TELEMETRY_TRUST_ADJUST_DELTA: &str = "memory.telemetry.trustAdjustDelta";
    pub const MEMORY_TELEMETRY_MIN_RESPONSE_TOKENS: &str = "memory.telemetry.minResponseTokens";

    // /remember scope defaults + UX (Tier A / A2)
    pub const MEMORY_REMEMBER_DEFAULT_SCOPE: &str = "memory.remember.defaultScope";
    pub const MEMORY_REMEMBER_SHOW_SCOPE_BADGE: &str = "memory.remember.showScopeBadge";
    pub const MEMORY_REMEMBER_SHOW_SIMILARITY_ON_REINFORCE: &str =
        "memory.remember.showSimilarityOnReinforce";

    // /forget + audit table retention (Tier C / C2)
    pub const MEMORY_FORGET_USER_RETENTION_DAYS: &str = "memory.forget.userDeletionRetentionDays";
    pub const MEMORY_FORGET_SLEEP_RETENTION_DAYS: &str =
        "memory.forget.sleeptimePruneRetentionDays";
    pub const MEMORY_FORGET_REQUIRE_CONFIRM_BULK: &str = "memory.forget.requireConfirmForBulk";
    pub const MEMORY_FORGET_ALLOW_HISTORY_REDACTION: &str =
        "memory.forget.allowHistoryRedaction";

    // MCP server + external-memory migration (Tier A / A5)
    pub const MCP_GAVIERO_ENABLED: &str = "mcp.gavieroServer.enabled";
    pub const MCP_GAVIERO_EXPOSED_TOOLS: &str = "mcp.gavieroServer.exposedTools";
    pub const MCP_GAVIERO_DISABLE_EXTERNAL: &str = "mcp.gavieroServer.disableExternalMemory";
    pub const MCP_GAVIERO_SHIM_BINARY: &str = "mcp.gavieroServer.shimBinary";
    pub const MCP_GAVIERO_CODEX_TRUST: &str = "mcp.gavieroServer.codexTrust";

    // context7 docs-lookup MCP server (default-on; injected into every
    // swarm worktree's .mcp.json + .codex/config.toml alongside the
    // gaviero shim). Disable for offline or privacy-sensitive work.
    pub const MCP_CONTEXT7_ENABLED: &str = "mcp.context7.enabled";
    pub const MCP_CONTEXT7_COMMAND: &str = "mcp.context7.command";
    pub const MCP_CONTEXT7_ARGS: &str = "mcp.context7.args";

    // TUI memory panel (Tier A / A4)
    pub const UI_MEMORY_PANEL_RECENT_WINDOW_HOURS: &str = "ui.memoryPanel.recentWindowHours";
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceFolder {
    pub path: PathBuf,
    #[serde(default)]
    pub name: Option<String>,
}

impl WorkspaceFolder {
    pub fn display_name(&self) -> &str {
        self.name
            .as_deref()
            .unwrap_or_else(|| self.path.file_name().and_then(|n| n.to_str()).unwrap_or(""))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WorkspaceFile {
    folders: Vec<WorkspaceFolder>,
    #[serde(default)]
    settings: serde_json::Value,
}

#[derive(Clone, Debug)]
pub struct Workspace {
    folders: Vec<WorkspaceFolder>,
    workspace_settings: serde_json::Value,
    workspace_path: Option<PathBuf>,
    /// Cached per-folder `.gaviero/settings.json` contents (keyed by folder root).
    folder_settings_cache: HashMap<PathBuf, serde_json::Value>,
    /// Cached user-level `~/.config/gaviero/settings.json`.
    user_settings_cache: Option<serde_json::Value>,
}

impl Workspace {
    /// Load a `.gaviero-workspace` file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).context("reading .gaviero-workspace file")?;
        let file: WorkspaceFile =
            serde_json::from_str(&content).context("parsing .gaviero-workspace file")?;

        let mut ws = Self {
            folders: file.folders,
            workspace_settings: file.settings,
            workspace_path: Some(path.to_path_buf()),
            folder_settings_cache: HashMap::new(),
            user_settings_cache: None,
        };
        ws.reload_settings_cache();
        Ok(ws)
    }

    /// Create a temporary single-root workspace (no file on disk).
    pub fn single_folder(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        // In single-folder mode, load .gaviero/settings.json as workspace settings
        // so that resolve_setting(key, None) finds them.
        let settings_path = path.join(".gaviero").join("settings.json");
        let workspace_settings = match std::fs::read_to_string(&settings_path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(val) => {
                    tracing::info!("Loaded settings from {}", settings_path.display());
                    val
                }
                Err(e) => {
                    tracing::warn!(
                        "Invalid JSON in {}: {} — all settings will use defaults",
                        settings_path.display(),
                        e
                    );
                    serde_json::Value::Object(serde_json::Map::new())
                }
            },
            Err(e) => {
                tracing::warn!(
                    "Settings not found at {} ({}): all settings will use defaults",
                    settings_path.display(),
                    e
                );
                serde_json::Value::Object(serde_json::Map::new())
            }
        };

        let mut ws = Self {
            folders: vec![WorkspaceFolder {
                path,
                name: Some(name),
            }],
            workspace_settings,
            workspace_path: None,
            folder_settings_cache: HashMap::new(),
            user_settings_cache: None,
        };
        ws.reload_settings_cache();
        ws
    }

    /// Ensure `.gaviero/settings.json` exists for all workspace roots.
    /// Creates the directory and a default settings file if missing.
    pub fn ensure_settings(&mut self) {
        for folder in &self.folders {
            let gaviero_dir = folder.path.join(".gaviero");
            let settings_path = gaviero_dir.join("settings.json");

            if settings_path.exists() {
                continue;
            }

            // Derive namespace from folder name
            let namespace = folder
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("default")
                .to_string();

            let defaults = serde_json::json!({
                "files": {
                    "exclude": {
                        ".DS_Store": true,
                        ".cache": true,
                        ".gradle": true,
                        ".idea": true,
                        ".mvn": true,
                        ".mypy_cache": true,
                        ".next": true,
                        ".nuxt": true,
                        ".parcel-cache": true,
                        ".pytest_cache": true,
                        ".tox": true,
                        ".venv": true,
                        "Thumbs.db": true,
                        "__pycache__": true,
                        "build": true,
                        "coverage": true,
                        "dist": true,
                        "node_modules": true,
                        "out": true,
                        "target": true,
                        "venv": true
                    }
                },
                "git": {
                    "treeAllowList": ["config", "description", "HEAD", "hooks", "info"]
                },
                "memory": {
                    "namespace": namespace,
                    "readNamespaces": []
                },
                "panels": {
                    "fileTree": { "width": 25 },
                    "sidePanel": { "width": 50 },
                    "layouts": {
                        "1": [15, 60, 25],
                        "2": [15, 40, 45],
                        "3": [0, 100, 0],
                        "4": [0, 60, 40]
                    }
                }
            });

            if let Err(e) = std::fs::create_dir_all(&gaviero_dir) {
                tracing::warn!("failed to create {}: {}", gaviero_dir.display(), e);
                continue;
            }

            let content = serde_json::to_string_pretty(&defaults).unwrap_or_default();
            if let Err(e) = std::fs::write(&settings_path, &content) {
                tracing::warn!("failed to write {}: {}", settings_path.display(), e);
                continue;
            }

            tracing::info!("created default settings at {}", settings_path.display());

            // Reload into workspace_settings and cache so the current session uses them
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                self.workspace_settings = val.clone();
                self.folder_settings_cache.insert(folder.path.clone(), val);
            }
        }
    }

    /// Return all workspace root paths.
    pub fn roots(&self) -> Vec<&Path> {
        self.folders.iter().map(|f| f.path.as_path()).collect()
    }

    /// Return paths to the `.claude` and `.gaviero` configuration folders
    /// near the workspace file. Each name is searched first in the directory
    /// containing the workspace file, then one level up. The first existing
    /// match per name wins. Empty in single-folder mode (no workspace file).
    ///
    /// Candidates whose parent directory is itself a workspace folder root
    /// are skipped — the `.claude`/`.gaviero` dir is already visible inside
    /// that folder's tree, so adding it again would duplicate the entry.
    pub fn config_roots(&self) -> Vec<PathBuf> {
        let Some(workspace_dir) = self.workspace_path.as_deref().and_then(Path::parent) else {
            return Vec::new();
        };
        let parent_dir = workspace_dir.parent();
        let folder_roots: Vec<PathBuf> = self
            .folders
            .iter()
            .map(|f| canonicalize_path(&f.path))
            .collect();
        let is_folder_root = |dir: &Path| -> bool {
            let canonical = canonicalize_path(dir);
            folder_roots.iter().any(|r| r == &canonical)
        };
        let resolve = |name: &str| -> Option<PathBuf> {
            if !is_folder_root(workspace_dir) {
                let same = workspace_dir.join(name);
                if same.is_dir() {
                    return Some(same);
                }
            }
            let up_dir = parent_dir?;
            if !is_folder_root(up_dir) {
                let up = up_dir.join(name);
                if up.is_dir() {
                    return Some(up);
                }
            }
            None
        };
        let mut out = Vec::new();
        if let Some(p) = resolve(".claude") {
            out.push(p);
        }
        if let Some(p) = resolve(".gaviero") {
            out.push(p);
        }
        out
    }

    /// Return workspace folders.
    pub fn folders(&self) -> &[WorkspaceFolder] {
        &self.folders
    }

    /// Return the workspace folder root that contains `path`, picking
    /// the longest match when folders are nested. Used by the memory
    /// registry to route a file's reads/writes to the correct
    /// per-folder DB.
    ///
    /// Comparison uses canonical paths so symlinks resolve correctly.
    pub fn folder_for_path(&self, path: &Path) -> Option<&Path> {
        let target = canonicalize_path(path);
        let mut best: Option<(&Path, usize)> = None;
        for folder in &self.folders {
            let folder_canonical = canonicalize_path(&folder.path);
            if target.starts_with(&folder_canonical) {
                let len = folder_canonical.as_os_str().len();
                if best.map(|(_, l)| len > l).unwrap_or(true) {
                    best = Some((folder.path.as_path(), len));
                }
            }
        }
        best.map(|(p, _)| p)
    }

    /// Worktree-aware variant of [`Self::folder_for_path`]. When `path`
    /// lives inside a `.gaviero/worktrees/{id}/` subtree, walks up to
    /// the parent folder so memory writes from inside a swarm worktree
    /// land in the parent repo's DB (and survive worktree cleanup).
    pub fn folder_for_worktree_path(&self, path: &Path) -> Option<&Path> {
        let stripped = strip_worktree_segment(path);
        self.folder_for_path(stripped.as_ref().unwrap_or(&path.to_path_buf()))
    }

    /// Add a root folder and optionally save to disk.
    pub fn add_root(&mut self, path: PathBuf, name: Option<String>) {
        self.folders.push(WorkspaceFolder { path, name });
    }

    /// Save the workspace to its file (if it has one).
    pub fn save(&self) -> Result<()> {
        let path = self
            .workspace_path
            .as_ref()
            .context("no workspace file path (single-folder mode)")?;
        let file = WorkspaceFile {
            folders: self.folders.clone(),
            settings: self.workspace_settings.clone(),
        };
        let content = serde_json::to_string_pretty(&file)?;
        std::fs::write(path, content).context("writing workspace file")?;
        Ok(())
    }

    /// Persist a dotted-key setting into `<root>/.gaviero/settings.json`.
    /// Creates the file (and parent dir) if missing. Refreshes the
    /// cached folder settings so subsequent `resolve_setting` calls see
    /// the new value without a reload.
    pub fn save_folder_setting(
        &mut self,
        root: &Path,
        key: &str,
        value: serde_json::Value,
    ) -> Result<()> {
        let gaviero_dir = root.join(".gaviero");
        std::fs::create_dir_all(&gaviero_dir)
            .with_context(|| format!("creating {}", gaviero_dir.display()))?;
        let path = gaviero_dir.join("settings.json");
        let mut doc: serde_json::Value = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|_| serde_json::json!({})),
            Err(_) => serde_json::json!({}),
        };
        dot_set(&mut doc, key, value);
        let content = serde_json::to_string_pretty(&doc)?;
        std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
        self.folder_settings_cache.insert(root.to_path_buf(), doc);
        Ok(())
    }

    /// Reload cached settings from disk. Call when the file watcher reports
    /// changes to any `settings.json` file.
    pub fn reload_settings_cache(&mut self) {
        // Cache per-folder settings
        self.folder_settings_cache.clear();
        for folder in &self.folders {
            let settings_path = folder.path.join(".gaviero").join("settings.json");
            if let Ok(content) = std::fs::read_to_string(&settings_path)
                && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
            {
                self.folder_settings_cache.insert(folder.path.clone(), val);
            }
        }

        // Cache user-level settings
        self.user_settings_cache = dirs::config_dir().and_then(|config_dir| {
            let user_settings_path = config_dir.join("gaviero").join("settings.json");
            let content = std::fs::read_to_string(&user_settings_path).ok()?;
            match serde_json::from_str(&content) {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(
                        "failed to parse user settings {}: {}",
                        user_settings_path.display(),
                        e
                    );
                    None
                }
            }
        });
    }

    /// Resolve a setting using the cascade:
    /// 1. Per-folder `.gaviero/settings.json` (if root provided)
    /// 2. Workspace-level settings
    /// 3. User-level `~/.config/gaviero/settings.json`
    /// 4. Hardcoded defaults
    pub fn resolve_setting(&self, key: &str, root: Option<&Path>) -> serde_json::Value {
        // 1. Per-folder settings (from cache)
        if let Some(root) = root
            && let Some(settings) = self.folder_settings_cache.get(root)
            && let Some(val) = dot_get(settings, key)
        {
            return val.clone();
        }

        // 2. Workspace-level settings
        if let Some(val) = dot_get(&self.workspace_settings, key) {
            return val.clone();
        }

        // 3. User-level settings (from cache)
        if let Some(ref settings) = self.user_settings_cache
            && let Some(val) = dot_get(settings, key)
        {
            return val.clone();
        }

        // 4. Hardcoded defaults
        hardcoded_default(key)
    }

    /// Resolve a language-specific setting.
    /// Checks `[language].key` before `key` at each cascade level.
    pub fn resolve_language_setting(
        &self,
        key: &str,
        language: &str,
        root: Option<&Path>,
    ) -> serde_json::Value {
        let lang_key = format!("[{language}].{key}");
        let val = self.resolve_setting(&lang_key, root);
        if !val.is_null() {
            return val;
        }
        self.resolve_setting(key, root)
    }

    /// Resolve the write namespace (where new memories are stored).
    ///
    /// Cascade: per-folder `memory.namespace` → workspace-level → folder name fallback.
    pub fn resolve_namespace(&self, root: Option<&Path>) -> String {
        let val = self.resolve_setting(settings::MEMORY_NAMESPACE, root);
        if let Some(ns) = val.as_str() {
            return ns.to_string();
        }
        // Fallback: derive from the first folder name
        let folder = root.or_else(|| self.roots().first().copied());
        folder
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("default")
            .to_string()
    }

    /// Resolve the full chat-injection config from the settings cascade.
    ///
    /// The returned `ChatInjectionConfig` is the effective config after per-
    /// folder → workspace → user → defaults resolution. Callers at the chat
    /// entry point hand this to `memory::retrieve_for_chat`.
    /// Resolve shallow `<repo_topology>` settings for the workspace root.
    pub fn resolve_topology_config(&self, root: Option<&Path>) -> crate::repo_map::TopologyConfig {
        let enabled = self
            .resolve_setting(settings::AGENT_TOPOLOGY_ENABLED, root)
            .as_bool()
            .unwrap_or(true);
        let max_depth = self
            .resolve_setting(settings::AGENT_TOPOLOGY_MAX_DEPTH, root)
            .as_u64()
            .unwrap_or(2)
            .min(u8::MAX as u64) as u8;
        let max_dirs = self
            .resolve_setting(settings::AGENT_TOPOLOGY_MAX_DIRS, root)
            .as_u64()
            .unwrap_or(64) as usize;
        let max_token_budget = self
            .resolve_setting(settings::AGENT_TOPOLOGY_TOKEN_BUDGET, root)
            .as_u64()
            .unwrap_or(600) as usize;
        crate::repo_map::TopologyConfig {
            enabled,
            max_depth,
            max_dirs,
            max_token_budget,
        }
    }

    pub fn resolve_chat_injection_config(
        &self,
        root: Option<&Path>,
    ) -> crate::memory::ChatInjectionConfig {
        let enabled = self
            .resolve_setting(settings::MEMORY_CHAT_INJECTION_ENABLED, root)
            .as_bool()
            .unwrap_or(true);

        let scope_names: Vec<String> = self
            .resolve_setting(settings::MEMORY_CHAT_INJECTION_SCOPES, root)
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let scopes = crate::memory::ScopeMix::from_names(&scope_names);

        let max_items = self
            .resolve_setting(settings::MEMORY_CHAT_INJECTION_MAX_ITEMS, root)
            .as_u64()
            .unwrap_or(8) as usize;
        let token_budget = self
            .resolve_setting(settings::MEMORY_CHAT_INJECTION_TOKEN_BUDGET, root)
            .as_u64()
            .unwrap_or(1000) as usize;
        let min_similarity = self
            .resolve_setting(settings::MEMORY_CHAT_INJECTION_MIN_SIM, root)
            .as_f64()
            .unwrap_or(0.3) as f32;

        crate::memory::ChatInjectionConfig {
            enabled,
            scopes,
            max_items,
            token_budget,
            min_similarity,
        }
    }

    /// Resolve the agent tool surface for the given workspace root.
    ///
    /// Returns `(available, approved)`. `available` becomes `--tools`
    /// on the Claude subprocess; `approved` becomes `--allowedTools`
    /// (auto-approved). Anything in `approved` not also in `available`
    /// is dropped — Claude rejects unknown tools in `--allowedTools`.
    pub fn resolve_agent_tools(&self, root: Option<&Path>) -> (Vec<String>, Vec<String>) {
        let available: Vec<String> = self
            .resolve_setting(settings::AGENT_AVAILABLE_TOOLS, root)
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let approved: Vec<String> = self
            .resolve_setting(settings::AGENT_APPROVED_TOOLS, root)
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .filter(|name| available.iter().any(|a| a == name))
                    .collect()
            })
            .unwrap_or_default();
        (available, approved)
    }

    /// Tier B / B3: resolve the retrieval-engine config (mode + pool
    /// caps) from `memory.retrieval.*`. The returned struct is what the
    /// central `retrieve_ranked` engine consumes.
    pub fn resolve_retrieval_config(&self, root: Option<&Path>) -> crate::memory::RetrievalConfig {
        let mode = self
            .resolve_setting(settings::MEMORY_RETRIEVAL_MODE, root)
            .as_str()
            .map(crate::memory::RetrievalMode::parse)
            .unwrap_or(crate::memory::RetrievalMode::Merged);
        let per_scope_top_k = self
            .resolve_setting(settings::MEMORY_RETRIEVAL_PER_SCOPE_TOP_K, root)
            .as_u64()
            .unwrap_or(20) as usize;
        let max_merged_pool = self
            .resolve_setting(settings::MEMORY_RETRIEVAL_MAX_MERGED_POOL, root)
            .as_u64()
            .unwrap_or(50) as usize;
        crate::memory::RetrievalConfig {
            mode,
            per_scope_top_k,
            max_merged_pool,
        }
    }

    /// Tier B / B2: resolve the reranker stage config from
    /// `memory.reranker.*`.
    /// C3: resolve `repoMap.specificity.*` settings into a
    /// [`crate::repo_map::SpecificityConfig`]. Used by callers that want
    /// the configured behavior instead of the hardcoded default.
    pub fn resolve_specificity_config(
        &self,
        root: Option<&Path>,
    ) -> crate::repo_map::SpecificityConfig {
        let enabled = self
            .resolve_setting(settings::REPO_MAP_SPECIFICITY_ENABLED, root)
            .as_bool()
            .unwrap_or(true);
        let stop_symbol_threshold = self
            .resolve_setting(settings::REPO_MAP_SPECIFICITY_STOP_SYMBOL_THRESHOLD, root)
            .as_f64()
            .unwrap_or(0.5);
        crate::repo_map::SpecificityConfig {
            enabled,
            stop_symbol_threshold,
        }
    }

    /// C4: resolve `repoMap.edges.weights.<intent>` settings into an
    /// [`crate::repo_map::store::EdgeWeights`] map for the requested
    /// mode. Missing settings produce the plan-default preset.
    pub fn resolve_edge_weights(
        &self,
        mode: crate::repo_map::store::BlastRadiusMode,
        root: Option<&Path>,
    ) -> crate::repo_map::store::EdgeWeights {
        let mut weights = crate::repo_map::store::EdgeWeights::default_for(mode);
        let raw = self.resolve_setting(settings::REPO_MAP_EDGE_WEIGHTS, root);
        let Some(map) = raw.as_object() else {
            return weights;
        };
        let intent_key = mode.as_str();
        if let Some(intent_overrides) = map.get(intent_key).and_then(|v| v.as_object()) {
            weights.apply_overrides(intent_overrides);
        }
        weights
    }

    /// Resolve edge weights for every supported mode in one call so
    /// the MCP server can stash them at construction time and avoid
    /// re-walking the settings cascade per `blast_radius` invocation.
    pub fn resolve_all_edge_weights(
        &self,
        root: Option<&Path>,
    ) -> std::collections::HashMap<
        crate::repo_map::store::BlastRadiusMode,
        crate::repo_map::store::EdgeWeights,
    > {
        use crate::repo_map::store::BlastRadiusMode;
        let modes = [
            BlastRadiusMode::Impact,
            BlastRadiusMode::Callers,
            BlastRadiusMode::Tests,
            BlastRadiusMode::Implementations,
            BlastRadiusMode::All,
        ];
        modes
            .into_iter()
            .map(|m| (m, self.resolve_edge_weights(m, root)))
            .collect()
    }

    pub fn resolve_rerank_config(&self, root: Option<&Path>) -> crate::memory::RerankConfig {
        let enabled = self
            .resolve_setting(settings::MEMORY_RERANKER_ENABLED, root)
            .as_bool()
            .unwrap_or(false);
        let pool_size = self
            .resolve_setting(settings::MEMORY_RERANKER_POOL_SIZE, root)
            .as_u64()
            .unwrap_or(50) as usize;
        let blend_weight = self
            .resolve_setting(settings::MEMORY_RERANKER_BLEND_WEIGHT, root)
            .as_f64()
            .unwrap_or(0.6) as f32;
        let max_latency_ms = self
            .resolve_setting(settings::MEMORY_RERANKER_MAX_LATENCY_MS, root)
            .as_u64()
            .unwrap_or(200);
        crate::memory::RerankConfig {
            enabled,
            pool_size,
            blend_weight,
            max_latency_ms,
        }
    }

    /// Resolve the read namespaces (searched when querying memory).
    ///
    /// Always includes the write namespace. Additional namespaces come from
    /// `memory.readNamespaces` in settings (a JSON array of strings).
    pub fn resolve_read_namespaces(&self, root: Option<&Path>) -> Vec<String> {
        let write_ns = self.resolve_namespace(root);
        let mut namespaces = vec![write_ns];

        let val = self.resolve_setting(settings::MEMORY_READ_NAMESPACES, root);
        if let Some(arr) = val.as_array() {
            for item in arr {
                if let Some(ns) = item.as_str() {
                    let ns = ns.to_string();
                    if !namespaces.contains(&ns) {
                        namespaces.push(ns);
                    }
                }
            }
        }

        namespaces
    }

    /// List all distinct namespaces across workspace folders.
    pub fn all_namespaces(&self) -> Vec<(String, PathBuf)> {
        self.folders
            .iter()
            .map(|f| {
                let ns = self.resolve_namespace(Some(&f.path));
                (ns, f.path.clone())
            })
            .collect()
    }
}

/// Resolve a dot-notation key like `"editor.tabSize"` into a nested JSON value.
fn dot_get<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    let parts: Vec<&str> = key.splitn(2, '.').collect();
    match value.get(parts[0]) {
        Some(inner) if parts.len() == 2 => dot_get(inner, parts[1]),
        Some(inner) if parts.len() == 1 => Some(inner),
        _ => None,
    }
}

fn dot_set(target: &mut serde_json::Value, key: &str, value: serde_json::Value) {
    let parts: Vec<&str> = key.splitn(2, '.').collect();
    if !target.is_object() {
        *target = serde_json::Value::Object(serde_json::Map::new());
    }
    let obj = target.as_object_mut().expect("ensured object above");
    if parts.len() == 1 {
        obj.insert(parts[0].to_string(), value);
    } else {
        let child = obj
            .entry(parts[0].to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        dot_set(child, parts[1], value);
    }
}

fn hardcoded_default(key: &str) -> serde_json::Value {
    match key {
        settings::TAB_SIZE => serde_json::json!(4),
        settings::INSERT_SPACES => serde_json::json!(true),
        settings::FORMAT_ON_SAVE => serde_json::json!(false),
        settings::FILES_EXCLUDE => serde_json::json!({}),
        settings::FILE_TREE_WIDTH => serde_json::json!(30),
        settings::SIDE_PANEL_WIDTH => serde_json::json!(40),
        settings::TERMINAL_SPLIT_PERCENT => serde_json::json!(30),
        settings::AGENT_MODEL => serde_json::json!("claude:sonnet"),
        settings::AGENT_EFFORT => serde_json::json!("off"),
        settings::AGENT_MAX_TOKENS => serde_json::json!(16384),
        settings::AGENT_GRAPH_BUDGET_TOKENS => serde_json::json!(12000),
        settings::AGENT_AVAILABLE_TOOLS => {
            serde_json::json!(["Read", "Glob", "Grep", "Write", "Edit", "MultiEdit"])
        }
        settings::AGENT_APPROVED_TOOLS => {
            serde_json::json!(["Read", "Glob", "Grep"])
        }

        // Chat memory injection (S1)
        settings::MEMORY_CHAT_INJECTION_ENABLED => serde_json::json!(true),
        settings::MEMORY_CHAT_INJECTION_SCOPES => {
            serde_json::json!(["workspace", "repo", "module"])
        }
        settings::MEMORY_CHAT_INJECTION_MAX_ITEMS => serde_json::json!(8),
        settings::MEMORY_CHAT_INJECTION_TOKEN_BUDGET => serde_json::json!(1000),
        settings::MEMORY_CHAT_INJECTION_MIN_SIM => serde_json::json!(0.3),

        // Retrieval manifests (S4)
        settings::MEMORY_MANIFESTS_ENABLED => serde_json::json!(true),
        settings::MEMORY_MANIFESTS_RETENTION_DAYS => serde_json::json!(30),
        settings::MEMORY_MANIFESTS_CAPTURE_POOL => serde_json::json!(true),

        // Per-turn extractor (S3)
        settings::MEMORY_EXTRACTOR_ENABLED => serde_json::json!(true),
        settings::MEMORY_EXTRACTOR_MODEL => serde_json::json!(""),
        settings::MEMORY_EXTRACTOR_EVERY_N_TURNS => serde_json::json!(1),
        settings::MEMORY_EXTRACTOR_MAX_PER_TURN => serde_json::json!(5),

        // Recency floor + decay-exempt types (B4)
        settings::MEMORY_SCORING_RECENCY_FLOOR => serde_json::json!(0.35),
        settings::MEMORY_SCORING_DECAY_EXEMPT_TYPES => serde_json::json!([
            "decision",
            "convention",
            "invariant",
            "preference",
            "gotcha"
        ]),

        // Embedder selection (B1).
        // Default = "nomic" until B1g flips after the eval ablation.
        // Accepted: "nomic" | "gte-modernbert".
        settings::MEMORY_EMBEDDER_MODEL => serde_json::json!("nomic"),
        settings::MEMORY_EMBEDDER_REEMBED_BATCH_SIZE => serde_json::json!(32),

        // Cross-encoder reranker (B2).
        // Disabled by default until the B2f ablation gate is green.
        settings::MEMORY_RERANKER_ENABLED => serde_json::json!(false),
        settings::MEMORY_RERANKER_MODEL => serde_json::json!("gte-reranker-modernbert"),
        settings::MEMORY_RERANKER_POOL_SIZE => serde_json::json!(50),
        settings::MEMORY_RERANKER_BLEND_WEIGHT => serde_json::json!(0.6),
        settings::MEMORY_RERANKER_MAX_LATENCY_MS => serde_json::json!(200),

        // Repo-map graph precision (C3/C4).
        settings::REPO_MAP_SPECIFICITY_ENABLED => serde_json::json!(true),
        settings::REPO_MAP_SPECIFICITY_STOP_SYMBOL_THRESHOLD => serde_json::json!(0.5),
        settings::REPO_MAP_TYPED_EDGES_ENABLED => serde_json::json!(true),

        // Merged multi-scope retrieval (B3).
        // "merged" → no 0.70 early-exit, every scope flows into one
        // pool (the new default). "cascade" → legacy kill-switch path,
        // kept one release for debugging.
        settings::MEMORY_RETRIEVAL_MODE => serde_json::json!("merged"),
        settings::MEMORY_RETRIEVAL_PER_SCOPE_TOP_K => serde_json::json!(20),
        settings::MEMORY_RETRIEVAL_MAX_MERGED_POOL => serde_json::json!(50),

        // /remember UX (A2)
        settings::MEMORY_REMEMBER_DEFAULT_SCOPE => serde_json::json!("repo"),
        settings::MEMORY_REMEMBER_SHOW_SCOPE_BADGE => serde_json::json!(true),
        settings::MEMORY_REMEMBER_SHOW_SIMILARITY_ON_REINFORCE => serde_json::json!(true),

        // /forget + audit retention (Tier C / C2)
        settings::MEMORY_FORGET_USER_RETENTION_DAYS => serde_json::json!(30),
        settings::MEMORY_FORGET_SLEEP_RETENTION_DAYS => serde_json::json!(14),
        settings::MEMORY_FORGET_REQUIRE_CONFIRM_BULK => serde_json::json!(true),
        settings::MEMORY_FORGET_ALLOW_HISTORY_REDACTION => serde_json::json!(true),

        // MCP server (A5)
        settings::MCP_GAVIERO_ENABLED => serde_json::json!(true),
        settings::MCP_GAVIERO_EXPOSED_TOOLS => {
            serde_json::json!(["memory_search", "blast_radius", "node_doc"])
        }
        settings::MCP_GAVIERO_DISABLE_EXTERNAL => serde_json::json!(true),
        settings::MCP_GAVIERO_SHIM_BINARY => serde_json::json!("gaviero-mcp-shim"),
        settings::MCP_GAVIERO_CODEX_TRUST => serde_json::json!("unknown"),

        // context7 docs-lookup MCP server: default-on; uses `npx` so it
        // works out of the box on any Node-equipped host. Users without
        // Node, or running offline, set enabled=false to suppress.
        settings::MCP_CONTEXT7_ENABLED => serde_json::json!(true),
        settings::MCP_CONTEXT7_COMMAND => serde_json::json!("npx"),
        settings::MCP_CONTEXT7_ARGS => serde_json::json!(["-y", "@upstash/context7-mcp"]),

        // Memory panel (A4)
        settings::UI_MEMORY_PANEL_RECENT_WINDOW_HOURS => serde_json::json!(24),

        // Tier B / B5 — session consolidator + sleeptime
        settings::MEMORY_SESSION_CONSOLIDATE_ON_CLOSE => serde_json::json!(true),
        settings::MEMORY_SESSION_IDLE_TIMEOUT_SEC => serde_json::json!(90),
        settings::MEMORY_SLEEPTIME_ENABLED => serde_json::json!(true),
        settings::MEMORY_SLEEPTIME_MIN_IDLE_MINUTES => serde_json::json!(10),
        settings::MEMORY_SLEEPTIME_WEEKLY_FORCE_RUN => serde_json::json!(true),
        settings::MEMORY_SLEEPTIME_NEAR_DUP_THRESHOLD => serde_json::json!(0.92),
        settings::MEMORY_SLEEPTIME_FIRST_RUN_REQUIRE_CONFIRM => serde_json::json!(true),

        // Tier B / B6 — retrieval-use telemetry
        settings::MEMORY_TELEMETRY_ENABLED => serde_json::json!(true),
        settings::MEMORY_TELEMETRY_USED_THRESHOLD => serde_json::json!(0.55),
        settings::MEMORY_TELEMETRY_PARTIAL_THRESHOLD => serde_json::json!(0.35),
        settings::MEMORY_TELEMETRY_MIN_INJECTIONS_FOR_TRUST => serde_json::json!(5),
        settings::MEMORY_TELEMETRY_TRUST_ADJUST_DELTA => serde_json::json!(0.05),
        settings::MEMORY_TELEMETRY_MIN_RESPONSE_TOKENS => serde_json::json!(20),

        _ => serde_json::Value::Null,
    }
}

fn canonicalize_path(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// If `path` lives inside a `.gaviero/worktrees/{id}/...` subtree,
/// return the parent of that segment (i.e., the folder that owns the
/// worktree). Otherwise `None`.
fn strip_worktree_segment(path: &Path) -> Option<PathBuf> {
    let components: Vec<_> = path.components().collect();
    for i in 0..components.len().saturating_sub(2) {
        if components[i].as_os_str() == ".gaviero" && components[i + 1].as_os_str() == "worktrees" {
            let parent: PathBuf = components[..i].iter().collect();
            return Some(parent);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_single_folder_workspace() {
        let ws = Workspace::single_folder(PathBuf::from("/tmp/test-project"));
        assert_eq!(ws.roots().len(), 1);
        assert_eq!(ws.roots()[0], Path::new("/tmp/test-project"));
    }

    #[test]
    fn test_load_workspace_file() {
        let dir = tempfile::tempdir().unwrap();
        let ws_path = dir.path().join("test.gaviero-workspace");
        let content = r#"{
            "folders": [
                { "path": "/home/user/project-a", "name": "Project A" },
                { "path": "/home/user/project-b" },
                { "path": "/home/user/project-c", "name": "Project C" }
            ],
            "settings": {
                "editor": { "tabSize": 2 }
            }
        }"#;
        fs::write(&ws_path, content).unwrap();

        let ws = Workspace::load(&ws_path).unwrap();
        assert_eq!(ws.roots().len(), 3);
        assert_eq!(ws.folders()[0].display_name(), "Project A");
        assert_eq!(ws.folders()[1].display_name(), "project-b");
    }

    #[test]
    fn test_resolve_setting_workspace_level() {
        let ws = Workspace {
            folders: vec![],
            workspace_settings: serde_json::json!({
                "editor": { "tabSize": 2 }
            }),
            workspace_path: None,
            folder_settings_cache: HashMap::new(),
            user_settings_cache: None,
        };
        assert_eq!(
            ws.resolve_setting("editor.tabSize", None),
            serde_json::json!(2)
        );
    }

    #[test]
    fn test_resolve_setting_falls_to_default() {
        let ws = Workspace::single_folder(PathBuf::from("/tmp/test"));
        assert_eq!(
            ws.resolve_setting("editor.tabSize", None),
            serde_json::json!(4)
        );
    }

    #[test]
    fn test_resolve_setting_folder_override() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let gaviero_dir = root.join(".gaviero");
        fs::create_dir_all(&gaviero_dir).unwrap();
        fs::write(
            gaviero_dir.join("settings.json"),
            r#"{ "editor": { "tabSize": 8 } }"#,
        )
        .unwrap();

        let mut ws = Workspace {
            folders: vec![WorkspaceFolder {
                path: root.to_path_buf(),
                name: None,
            }],
            workspace_settings: serde_json::json!({
                "editor": { "tabSize": 2 }
            }),
            workspace_path: None,
            folder_settings_cache: HashMap::new(),
            user_settings_cache: None,
        };
        ws.reload_settings_cache();
        assert_eq!(
            ws.resolve_setting("editor.tabSize", Some(root)),
            serde_json::json!(8)
        );
    }

    #[test]
    fn test_add_root_and_save() {
        let dir = tempfile::tempdir().unwrap();
        let ws_path = dir.path().join("test.gaviero-workspace");
        fs::write(&ws_path, r#"{"folders":[],"settings":{}}"#).unwrap();

        let mut ws = Workspace::load(&ws_path).unwrap();
        ws.add_root(PathBuf::from("/new/root"), Some("New Root".into()));
        ws.save().unwrap();

        let ws2 = Workspace::load(&ws_path).unwrap();
        assert_eq!(ws2.roots().len(), 1);
    }

    #[test]
    fn config_roots_finds_dot_dirs_at_workspace_file_level() {
        let dir = tempfile::tempdir().unwrap();
        let ws_path = dir.path().join("project.gaviero-workspace");
        fs::write(&ws_path, r#"{"folders":[],"settings":{}}"#).unwrap();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::create_dir_all(dir.path().join(".gaviero")).unwrap();

        let ws = Workspace::load(&ws_path).unwrap();
        let roots = ws.config_roots();
        assert_eq!(roots.len(), 2);
        assert!(roots[0].ends_with(".claude"));
        assert!(roots[1].ends_with(".gaviero"));
    }

    #[test]
    fn config_roots_falls_back_to_parent_level() {
        let outer = tempfile::tempdir().unwrap();
        let inner = outer.path().join("nested");
        fs::create_dir_all(&inner).unwrap();
        let ws_path = inner.join("project.gaviero-workspace");
        fs::write(&ws_path, r#"{"folders":[],"settings":{}}"#).unwrap();
        fs::create_dir_all(outer.path().join(".gaviero")).unwrap();

        let ws = Workspace::load(&ws_path).unwrap();
        let roots = ws.config_roots();
        assert_eq!(roots.len(), 1);
        assert!(roots[0].starts_with(outer.path()));
        assert!(roots[0].ends_with(".gaviero"));
    }

    #[test]
    fn config_roots_prefers_same_level_over_parent() {
        let outer = tempfile::tempdir().unwrap();
        let inner = outer.path().join("nested");
        fs::create_dir_all(&inner).unwrap();
        let ws_path = inner.join("project.gaviero-workspace");
        fs::write(&ws_path, r#"{"folders":[],"settings":{}}"#).unwrap();
        fs::create_dir_all(outer.path().join(".claude")).unwrap();
        fs::create_dir_all(inner.join(".claude")).unwrap();

        let ws = Workspace::load(&ws_path).unwrap();
        let roots = ws.config_roots();
        assert_eq!(roots.len(), 1);
        assert!(roots[0].starts_with(&inner));
    }

    #[test]
    fn config_roots_empty_for_single_folder_mode() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
        let ws = Workspace::single_folder(dir.path().to_path_buf());
        assert!(ws.config_roots().is_empty());
    }

    #[test]
    fn config_roots_skips_dirs_already_inside_a_folder_root() {
        let dir = tempfile::tempdir().unwrap();
        let ws_path = dir.path().join("project.gaviero-workspace");
        let folder = canonicalize_path(dir.path()).to_string_lossy().to_string();
        let body = format!(
            r#"{{"folders":[{{"path":"{}"}}],"settings":{{}}}}"#,
            folder.replace('\\', "\\\\")
        );
        fs::write(&ws_path, body).unwrap();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::create_dir_all(dir.path().join(".gaviero")).unwrap();

        let ws = Workspace::load(&ws_path).unwrap();
        assert!(
            ws.config_roots().is_empty(),
            "config dirs at folder root should not be re-listed"
        );
    }

    #[test]
    fn config_roots_skips_parent_level_dirs_already_inside_a_folder_root() {
        let outer = tempfile::tempdir().unwrap();
        let inner = outer.path().join("nested");
        fs::create_dir_all(&inner).unwrap();
        let ws_path = inner.join("project.gaviero-workspace");
        let folder = canonicalize_path(outer.path()).to_string_lossy().to_string();
        let body = format!(
            r#"{{"folders":[{{"path":"{}"}}],"settings":{{}}}}"#,
            folder.replace('\\', "\\\\")
        );
        fs::write(&ws_path, body).unwrap();
        fs::create_dir_all(outer.path().join(".gaviero")).unwrap();

        let ws = Workspace::load(&ws_path).unwrap();
        assert!(
            ws.config_roots().is_empty(),
            "parent-level config dir already inside a folder root should not be re-listed"
        );
    }

    #[test]
    fn config_roots_kept_when_no_folder_root_owns_them() {
        let dir = tempfile::tempdir().unwrap();
        let ws_path = dir.path().join("project.gaviero-workspace");
        let other = dir.path().join("other");
        fs::create_dir_all(&other).unwrap();
        let folder = canonicalize_path(&other).to_string_lossy().to_string();
        let body = format!(
            r#"{{"folders":[{{"path":"{}"}}],"settings":{{}}}}"#,
            folder.replace('\\', "\\\\")
        );
        fs::write(&ws_path, body).unwrap();
        fs::create_dir_all(dir.path().join(".gaviero")).unwrap();

        let ws = Workspace::load(&ws_path).unwrap();
        let roots = ws.config_roots();
        assert_eq!(roots.len(), 1);
        assert!(roots[0].ends_with(".gaviero"));
    }

    #[test]
    fn test_dot_get() {
        let val = serde_json::json!({"a": {"b": {"c": 42}}});
        assert_eq!(dot_get(&val, "a.b.c"), Some(&serde_json::json!(42)));
        assert_eq!(dot_get(&val, "a.b"), Some(&serde_json::json!({"c": 42})));
        assert_eq!(dot_get(&val, "x.y"), None);
    }

    #[test]
    fn folder_for_path_picks_longest_match() {
        let outer = tempfile::tempdir().unwrap();
        let inner = outer.path().join("nested");
        fs::create_dir_all(&inner).unwrap();

        let mut ws = Workspace::single_folder(outer.path().to_path_buf());
        ws.add_root(inner.clone(), None);

        let file_in_inner = inner.join("file.rs");
        fs::write(&file_in_inner, "").unwrap();
        let resolved = ws.folder_for_path(&file_in_inner).unwrap();
        // Both folders contain file_in_inner; longer (inner) wins.
        let resolved_canonical = canonicalize_path(resolved);
        assert_eq!(resolved_canonical, canonicalize_path(&inner));
    }

    #[test]
    fn folder_for_path_returns_none_when_outside() {
        let outer = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let ws = Workspace::single_folder(outer.path().to_path_buf());
        let file_outside = other.path().join("file.rs");
        fs::write(&file_outside, "").unwrap();
        assert!(ws.folder_for_path(&file_outside).is_none());
    }

    #[test]
    fn folder_for_worktree_path_strips_worktree_segment() {
        let folder = tempfile::tempdir().unwrap();
        let worktree_file = folder.path().join(".gaviero/worktrees/abc123/src/lib.rs");
        std::fs::create_dir_all(worktree_file.parent().unwrap()).unwrap();
        std::fs::write(&worktree_file, "").unwrap();

        let ws = Workspace::single_folder(folder.path().to_path_buf());
        let resolved = ws.folder_for_worktree_path(&worktree_file).unwrap();
        assert_eq!(
            canonicalize_path(resolved),
            canonicalize_path(folder.path())
        );
    }

    #[test]
    fn folder_for_worktree_path_falls_back_to_normal_lookup() {
        let folder = tempfile::tempdir().unwrap();
        let normal_file = folder.path().join("src/lib.rs");
        std::fs::create_dir_all(normal_file.parent().unwrap()).unwrap();
        std::fs::write(&normal_file, "").unwrap();

        let ws = Workspace::single_folder(folder.path().to_path_buf());
        let resolved = ws.folder_for_worktree_path(&normal_file).unwrap();
        assert_eq!(
            canonicalize_path(resolved),
            canonicalize_path(folder.path())
        );
    }

    #[test]
    fn strip_worktree_segment_no_match() {
        let p = PathBuf::from("/foo/bar/.gaviero/settings.json");
        assert!(strip_worktree_segment(&p).is_none());
    }
}
