//! Test runner: execute project test suites and parse results.
//!
//! Catches behavioral regressions — code that parses correctly and looks right
//! but doesn't actually work. Zero LLM cost.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use super::{ParsedTestResults, TestFailure, TestReport};

/// Configuration for the test runner.
#[derive(Debug, Clone)]
pub struct TestRunnerConfig {
    /// Command to execute (e.g., "cargo test", "npm test").
    pub command: Option<String>,
    /// Working directory (defaults to workspace root).
    pub cwd: Option<PathBuf>,
    /// Timeout in seconds (default: 300 = 5 minutes).
    pub timeout_secs: u64,
    /// Environment variables to inherit from the parent process.
    pub inherit_env: Vec<String>,
    /// Additional environment variables to set.
    pub extra_env: HashMap<String, String>,
    /// Whether to run only tests related to modified files.
    pub targeted: bool,
    /// Auto-detect test command from project files.
    pub auto_detect: bool,
}

impl Default for TestRunnerConfig {
    fn default() -> Self {
        Self {
            command: None,
            cwd: None,
            timeout_secs: 300,
            inherit_env: default_inherit_env(),
            extra_env: HashMap::new(),
            targeted: true,
            auto_detect: true,
        }
    }
}

fn default_inherit_env() -> Vec<String> {
    [
        "PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME", "GOPATH",
        "JAVA_HOME", "NODE_PATH", "PYTHONPATH", "VIRTUAL_ENV",
        "CONDA_PREFIX", "DATABASE_URL", "RUST_LOG", "RUST_BACKTRACE", "CI",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Run the test suite and return a report.
pub async fn run(
    config: &TestRunnerConfig,
    modified_files: &[PathBuf],
    workspace_root: &Path,
) -> Result<TestReport> {
    let cwd = config.cwd.as_deref().unwrap_or(workspace_root);

    // 1. Resolve test command
    let base_command = resolve_command(config, cwd)?;
    let Some(base_command) = base_command else {
        return Ok(TestReport {
            exit_code: 0,
            passed: true,
            stdout: String::new(),
            stderr: "No test suite detected".into(),
            duration: Duration::ZERO,
            targeted_filter: None,
            parsed_results: None,
        });
    };

    // 2. Build targeted filter
    let (command, filter) = if config.targeted && !modified_files.is_empty() {
        build_targeted_command(&base_command, modified_files, cwd)
    } else {
        (base_command.clone(), None)
    };

    // 3. Build environment
    let mut env: HashMap<String, String> = HashMap::new();
    for key in &config.inherit_env {
        if let Ok(val) = std::env::var(key) {
            env.insert(key.clone(), val);
        }
    }
    for (k, v) in &config.extra_env {
        env.insert(k.clone(), v.clone());
    }

    // 4. Execute
    let start = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(config.timeout_secs),
        run_command(&command, cwd, &env),
    )
    .await;

    let duration = start.elapsed();

    match result {
        Ok(Ok((exit_code, stdout, stderr))) => {
            let parsed = parse_test_output(&stdout, &stderr, &base_command);
            Ok(TestReport {
                exit_code,
                passed: exit_code == 0,
                stdout,
                stderr,
                duration,
                targeted_filter: filter,
                parsed_results: parsed,
            })
        }
        Ok(Err(e)) => Ok(TestReport {
            exit_code: -1,
            passed: false,
            stdout: String::new(),
            stderr: format!("Test execution error: {}", e),
            duration,
            targeted_filter: filter,
            parsed_results: None,
        }),
        Err(_) => Ok(TestReport {
            exit_code: -1,
            passed: false,
            stdout: String::new(),
            stderr: format!("Test execution timed out after {}s", config.timeout_secs),
            duration,
            targeted_filter: filter,
            parsed_results: None,
        }),
    }
}

/// Resolve the test command from config or auto-detection.
fn resolve_command(config: &TestRunnerConfig, cwd: &Path) -> Result<Option<String>> {
    // Priority 1: explicit command
    if let Some(ref cmd) = config.command {
        return Ok(Some(cmd.clone()));
    }

    // Priority 2: auto-detect
    if !config.auto_detect {
        return Ok(None);
    }

    Ok(auto_detect_test_command(cwd))
}

/// Auto-detect test command from project files.
pub fn auto_detect_test_command(project_root: &Path) -> Option<String> {
    // Cargo.toml → cargo test
    if project_root.join("Cargo.toml").exists() {
        return Some("cargo test".into());
    }

    // package.json with "test" script → npm test
    if let Ok(content) = std::fs::read_to_string(project_root.join("package.json")) {
        if let Ok(pkg) = serde_json::from_str::<serde_json::Value>(&content) {
            if pkg.get("scripts").and_then(|s| s.get("test")).is_some() {
                return Some("npm test".into());
            }
        }
    }

    // pytest markers
    for name in &["pytest.ini", "setup.cfg", "pyproject.toml"] {
        if project_root.join(name).exists() {
            if let Ok(content) = std::fs::read_to_string(project_root.join(name)) {
                if content.contains("[tool.pytest") || content.contains("[pytest]") || *name == "pytest.ini" {
                    return Some("pytest".into());
                }
            }
        }
    }

    // Gradle
    if project_root.join("build.gradle").exists() || project_root.join("build.gradle.kts").exists() {
        return Some("gradle test".into());
    }

    // Makefile with test target
    if let Ok(content) = std::fs::read_to_string(project_root.join("Makefile")) {
        if content.contains("\ntest:") || content.starts_with("test:") {
            return Some("make test".into());
        }
    }

    None
}

/// Build a targeted test command filtering to modified-file-related tests.
fn build_targeted_command(
    base_command: &str,
    modified_files: &[PathBuf],
    _cwd: &Path,
) -> (String, Option<String>) {
    if base_command.starts_with("cargo test") {
        // Rust: extract module paths from file paths
        let filters: Vec<String> = modified_files
            .iter()
            .filter_map(|f| {
                let s = f.to_string_lossy();
                if s.ends_with(".rs") {
                    // src/auth/strategy.rs → auth::strategy
                    let stripped = s
                        .strip_prefix("src/")
                        .or_else(|| s.strip_prefix("tests/"))
                        .unwrap_or(&s);
                    let module = stripped
                        .strip_suffix(".rs")
                        .unwrap_or(stripped)
                        .replace('/', "::");
                    Some(module)
                } else {
                    None
                }
            })
            .collect();

        if filters.is_empty() {
            return (base_command.into(), None);
        }

        let filter = filters.join(" ");
        (format!("{} {}", base_command, filter), Some(filter))
    } else if base_command.contains("jest") || base_command.contains("npm test") {
        // JavaScript: match test files by convention
        let patterns: Vec<String> = modified_files
            .iter()
            .filter_map(|f| {
                f.file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            })
            .collect();

        if patterns.is_empty() {
            return (base_command.into(), None);
        }

        let pattern = patterns.join("|");
        let filter = format!("--testPathPattern='{}'", pattern);
        (format!("{} {}", base_command, filter), Some(pattern))
    } else if base_command.starts_with("pytest") {
        // Python: match test files
        let test_files: Vec<String> = modified_files
            .iter()
            .filter_map(|f| {
                let name = f.file_name()?.to_str()?;
                if name.ends_with(".py") {
                    let test_name = if name.starts_with("test_") {
                        f.to_string_lossy().to_string()
                    } else {
                        let parent = f.parent().unwrap_or(Path::new("tests"));
                        parent
                            .join(format!("test_{}", name))
                            .to_string_lossy()
                            .to_string()
                    };
                    Some(test_name)
                } else {
                    None
                }
            })
            .collect();

        if test_files.is_empty() {
            return (base_command.into(), None);
        }

        let filter = test_files.join(" ");
        (format!("{} {}", base_command, filter), Some(filter))
    } else {
        (base_command.into(), None)
    }
}

/// Run a shell command and capture output.
async fn run_command(
    command: &str,
    cwd: &Path,
    env: &HashMap<String, String>,
) -> Result<(i32, String, String)> {
    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .env_clear()
        .envs(env)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("spawning test command")?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok((exit_code, stdout, stderr))
}

/// Parse test output into structured results (best-effort).
fn parse_test_output(stdout: &str, stderr: &str, command: &str) -> Option<ParsedTestResults> {
    let combined = format!("{}\n{}", stdout, stderr);

    if command.contains("cargo test") {
        parse_rust_test_output(&combined)
    } else if command.contains("pytest") {
        parse_pytest_output(&combined)
    } else if command.contains("jest") || command.contains("npm test") {
        parse_jest_output(&combined)
    } else {
        None
    }
}

fn parse_rust_test_output(output: &str) -> Option<ParsedTestResults> {
    // Parse "test result: ok. N passed; M failed; K ignored"
    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let mut total_ignored = 0usize;

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("test result:") {
            let rest = rest.trim();
            // Skip "ok." or "FAILED."
            let rest = rest
                .strip_prefix("ok.")
                .or_else(|| rest.strip_prefix("FAILED."))
                .unwrap_or(rest)
                .trim();
            // Parse "N passed; M failed; K ignored" or "K filtered out"
            for part in rest.split(';') {
                let part = part.trim();
                if let Some(n) = extract_number_before(part, " passed") {
                    total_passed += n;
                } else if let Some(n) = extract_number_before(part, " failed") {
                    total_failed += n;
                } else if let Some(n) = extract_number_before(part, " ignored") {
                    total_ignored += n;
                } else if let Some(n) = extract_number_before(part, " filtered out") {
                    total_ignored += n;
                }
            }
        }
    }

    if total_passed == 0 && total_failed == 0 {
        return None;
    }

    // Collect failure names
    let mut failures = Vec::new();
    for line in output.lines() {
        if line.starts_with("test ") && line.contains(" ... FAILED") {
            if let Some(name) = line.strip_prefix("test ").and_then(|s| s.split(" ... ").next()) {
                failures.push(TestFailure {
                    test_name: name.to_string(),
                    message: String::new(),
                    file: None,
                    line: None,
                });
            }
        }
    }

    Some(ParsedTestResults {
        total: total_passed + total_failed,
        passed: total_passed,
        failed: total_failed,
        skipped: total_ignored,
        failures,
    })
}

fn parse_pytest_output(output: &str) -> Option<ParsedTestResults> {
    let passed = find_number_before_keyword(output, " passed")?;
    let failed = find_number_before_keyword(output, " failed").unwrap_or(0);

    Some(ParsedTestResults {
        total: passed + failed,
        passed,
        failed,
        skipped: 0,
        failures: Vec::new(),
    })
}

fn parse_jest_output(output: &str) -> Option<ParsedTestResults> {
    let passed = find_number_before_keyword(output, " passed")?;
    let failed = find_number_before_keyword(output, " failed").unwrap_or(0);

    Some(ParsedTestResults {
        total: passed + failed,
        passed,
        failed,
        skipped: 0,
        failures: Vec::new(),
    })
}

/// Extract the number immediately before a keyword suffix in a string fragment.
/// E.g. "5 passed" with suffix " passed" → Some(5)
fn extract_number_before(s: &str, suffix: &str) -> Option<usize> {
    let s = s.trim();
    s.strip_suffix(suffix)?.trim().parse().ok()
}

/// Find a number before a keyword anywhere in the text.
/// E.g. "12 passed, 3 failed" with keyword " passed" → Some(12)
fn find_number_before_keyword(text: &str, keyword: &str) -> Option<usize> {
    for (pos, _) in text.match_indices(keyword) {
        // Walk backwards to find digits
        let before = &text[..pos];
        let num_str: String = before.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect();
        if let Ok(n) = num_str.parse::<usize>() {
            return Some(n);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_detect_cargo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
        assert_eq!(auto_detect_test_command(dir.path()), Some("cargo test".into()));
    }

    #[test]
    fn test_auto_detect_npm() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts": {"test": "jest"}}"#,
        )
        .unwrap();
        assert_eq!(auto_detect_test_command(dir.path()), Some("npm test".into()));
    }

    #[test]
    fn test_auto_detect_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(auto_detect_test_command(dir.path()), None);
    }

    #[test]
    fn test_auto_detect_makefile() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Makefile"), "all:\n\techo hi\n\ntest:\n\techo test\n")
            .unwrap();
        assert_eq!(auto_detect_test_command(dir.path()), Some("make test".into()));
    }

    #[test]
    fn test_parse_rust_output_ok() {
        let output = "running 5 tests\n\
            test tests::test_a ... ok\n\
            test tests::test_b ... ok\n\
            test tests::test_c ... ok\n\
            test tests::test_d ... ok\n\
            test tests::test_e ... ok\n\n\
            test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";

        let parsed = parse_rust_test_output(output).unwrap();
        assert_eq!(parsed.passed, 5);
        assert_eq!(parsed.failed, 0);
        assert!(parsed.failures.is_empty());
    }

    #[test]
    fn test_parse_rust_output_failures() {
        let output = "running 3 tests\n\
            test tests::test_a ... ok\n\
            test tests::test_b ... FAILED\n\
            test tests::test_c ... ok\n\n\
            test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n";

        let parsed = parse_rust_test_output(output).unwrap();
        assert_eq!(parsed.passed, 2);
        assert_eq!(parsed.failed, 1);
        assert_eq!(parsed.failures.len(), 1);
        assert_eq!(parsed.failures[0].test_name, "tests::test_b");
    }

    #[test]
    fn test_parse_rust_output_multiple_suites() {
        let output = "test result: ok. 10 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out\n\
            test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";

        let parsed = parse_rust_test_output(output).unwrap();
        assert_eq!(parsed.passed, 15);
        assert_eq!(parsed.failed, 0);
        assert_eq!(parsed.skipped, 1);
    }

    #[test]
    fn test_parse_pytest_output() {
        let output = "===== 12 passed, 3 failed in 1.5s =====\n";
        let parsed = parse_pytest_output(output).unwrap();
        assert_eq!(parsed.passed, 12);
        assert_eq!(parsed.failed, 3);
    }

    #[test]
    fn test_parse_jest_output() {
        let output = "Tests:  2 failed, 8 passed, 10 total\n";
        let parsed = parse_jest_output(output).unwrap();
        assert_eq!(parsed.passed, 8);
        assert_eq!(parsed.failed, 2);
    }

    #[test]
    fn test_targeted_cargo() {
        let files = vec![
            PathBuf::from("src/auth/strategy.rs"),
            PathBuf::from("src/api/handler.rs"),
        ];
        let (cmd, filter) = build_targeted_command("cargo test", &files, Path::new("."));
        assert!(cmd.contains("auth::strategy"));
        assert!(cmd.contains("api::handler"));
        assert!(filter.is_some());
    }

    #[test]
    fn test_targeted_pytest() {
        let files = vec![PathBuf::from("src/auth.py")];
        let (cmd, filter) = build_targeted_command("pytest", &files, Path::new("."));
        assert!(cmd.contains("test_auth.py"));
        assert!(filter.is_some());
    }

    #[test]
    fn test_targeted_no_matching_files() {
        let files = vec![PathBuf::from("README.md")];
        let (cmd, filter) = build_targeted_command("cargo test", &files, Path::new("."));
        assert_eq!(cmd, "cargo test");
        assert!(filter.is_none());
    }

    #[test]
    fn test_default_inherit_env() {
        let env = default_inherit_env();
        assert!(env.contains(&"PATH".to_string()));
        assert!(env.contains(&"HOME".to_string()));
        assert!(env.contains(&"CARGO_HOME".to_string()));
    }

    #[tokio::test]
    async fn test_run_no_test_suite() {
        let dir = tempfile::tempdir().unwrap();
        let config = TestRunnerConfig {
            auto_detect: true,
            ..Default::default()
        };
        let report = run(&config, &[], dir.path()).await.unwrap();
        assert!(report.passed);
        assert!(report.stderr.contains("No test suite"));
    }
}
