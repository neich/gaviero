//! AcpSessionFactory — manages one-shot and persistent Claude subprocess sessions.
//!
//! One-shot mode: spawns `claude --print`, writes stdin, closes, reads stdout.
//! Persistent mode: spawns `claude` (no --print), keeps stdin open, `ResultEvent`
//! signals end-of-turn. Slash commands like `/compact` forwarded as raw stdin lines.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use super::protocol::{StreamEvent, parse_stream_line};
use super::session::{AcpSession, AgentOptions};

/// Session lifecycle mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMode {
    /// Spawn `claude --print`, write stdin, close, read stdout, child exits.
    OneShot,
    /// Spawn `claude` (no --print), keep alive, bidirectional stdin/stdout.
    Persistent,
}

/// A persistent session handle — keeps the child alive across turns.
pub struct PersistentSession {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
    line_buf: String,
    model: String,
    alive: bool,
}

impl PersistentSession {
    /// Send a prompt and read the response until `ResultEvent`.
    ///
    /// Does NOT close stdin — the session stays alive for subsequent turns.
    pub async fn send_prompt(&mut self, prompt: &str) -> Result<Vec<StreamEvent>> {
        if !self.alive {
            anyhow::bail!("persistent session is dead");
        }

        // Write prompt + newline to stdin
        self.stdin
            .write_all(prompt.as_bytes())
            .await
            .context("writing prompt to persistent session")?;
        self.stdin
            .write_all(b"\n")
            .await
            .context("writing newline to persistent session")?;
        self.stdin.flush().await.context("flushing stdin")?;

        // Read events until ResultEvent
        let mut events = Vec::new();
        loop {
            match self.next_event().await {
                Ok(Some(event)) => {
                    let is_result = matches!(&event, StreamEvent::ResultEvent { .. });
                    events.push(event);
                    if is_result {
                        break; // End of turn
                    }
                }
                Ok(None) => {
                    // EOF — child died
                    self.alive = false;
                    break;
                }
                Err(e) => {
                    tracing::warn!("Persistent session read error: {}", e);
                    self.alive = false;
                    break;
                }
            }
        }

        Ok(events)
    }

    /// Forward a slash command (e.g., `/compact`, `/model sonnet`).
    pub async fn send_command(&mut self, command: &str) -> Result<Vec<StreamEvent>> {
        self.send_prompt(command).await
    }

    /// Read the next NDJSON event.
    async fn next_event(&mut self) -> Result<Option<StreamEvent>> {
        use tokio::io::AsyncBufReadExt;
        loop {
            self.line_buf.clear();
            let bytes = self
                .stdout
                .read_line(&mut self.line_buf)
                .await
                .context("reading persistent session stdout")?;
            if bytes == 0 {
                return Ok(None);
            }
            let line = self.line_buf.trim();
            if line.is_empty() {
                continue;
            }
            match parse_stream_line(line) {
                Ok(event) => return Ok(Some(event)),
                Err(_) => continue,
            }
        }
    }

    pub fn is_alive(&self) -> bool {
        self.alive
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Kill the persistent session.
    pub fn kill(&mut self) {
        let _ = self.child.start_kill();
        self.alive = false;
    }
}

impl Drop for PersistentSession {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Factory for creating and managing ACP sessions.
///
/// Manages both one-shot and persistent sessions. Persistent sessions are
/// keyed by purpose string (e.g., "chat", "coordinator:abc123").
pub struct AcpSessionFactory {
    default_options: AgentOptions,
    persistent_sessions: Arc<Mutex<HashMap<String, PersistentSession>>>,
}

impl AcpSessionFactory {
    pub fn new(default_options: AgentOptions) -> Self {
        Self {
            default_options,
            persistent_sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a one-shot session: spawn `claude --print`, write prompt, close stdin.
    ///
    /// This wraps the existing `AcpSession::spawn()` with no behavioral change.
    /// All listed tools are both available and auto-approved.
    pub fn one_shot(
        &self,
        model: &str,
        cwd: &Path,
        prompt: &str,
        system_prompt: &str,
        tools: &[&str],
    ) -> Result<AcpSession> {
        AcpSession::spawn(
            model,
            cwd,
            prompt,
            system_prompt,
            tools,
            tools,
            &self.default_options,
            &[],
        )
    }

    /// Create or retrieve a persistent session.
    ///
    /// If a session with the given key is alive, returns a reference to it.
    /// If it died or was never created, spawns a new one.
    pub async fn persistent(
        &self,
        key: &str,
        model: &str,
        cwd: &Path,
        system_prompt: &str,
        tools: &[&str],
    ) -> Result<()> {
        let mut sessions = self.persistent_sessions.lock().await;

        // Check if existing session is alive
        if let Some(session) = sessions.get(key) {
            if session.is_alive() {
                return Ok(());
            }
        }

        // Spawn new persistent session (no --print)
        let session = spawn_persistent(model, cwd, system_prompt, tools, &self.default_options)?;
        sessions.insert(key.to_string(), session);
        Ok(())
    }

    /// Send a prompt to a persistent session and collect the response events.
    pub async fn send_to_persistent(&self, key: &str, prompt: &str) -> Result<Vec<StreamEvent>> {
        let mut sessions = self.persistent_sessions.lock().await;
        let session = sessions
            .get_mut(key)
            .ok_or_else(|| anyhow::anyhow!("no persistent session with key '{}'", key))?;
        session.send_prompt(prompt).await
    }

    /// Send a slash command (e.g., `/compact`) to a persistent session.
    pub async fn send_command(&self, key: &str, command: &str) -> Result<Vec<StreamEvent>> {
        let mut sessions = self.persistent_sessions.lock().await;
        let session = sessions
            .get_mut(key)
            .ok_or_else(|| anyhow::anyhow!("no persistent session with key '{}'", key))?;
        session.send_command(command).await
    }

    /// Check if a persistent session is alive.
    pub async fn is_session_alive(&self, key: &str) -> bool {
        let sessions = self.persistent_sessions.lock().await;
        sessions.get(key).map(|s| s.is_alive()).unwrap_or(false)
    }

    /// Kill a persistent session.
    pub async fn kill_session(&self, key: &str) {
        let mut sessions = self.persistent_sessions.lock().await;
        if let Some(session) = sessions.get_mut(key) {
            session.kill();
        }
        sessions.remove(key);
    }

    /// Kill all persistent sessions. Called on workspace close or TUI quit.
    pub async fn kill_all(&self) {
        let mut sessions = self.persistent_sessions.lock().await;
        for (_, session) in sessions.iter_mut() {
            session.kill();
        }
        sessions.clear();
    }

    /// List all active persistent session keys.
    pub async fn active_sessions(&self) -> Vec<String> {
        let sessions = self.persistent_sessions.lock().await;
        sessions
            .iter()
            .filter(|(_, s)| s.is_alive())
            .map(|(k, _)| k.clone())
            .collect()
    }
}

/// Spawn a persistent Claude session (no --print, stdin kept open).
fn spawn_persistent(
    model: &str,
    cwd: &Path,
    system_prompt: &str,
    tools: &[&str],
    options: &AgentOptions,
) -> Result<PersistentSession> {
    use std::process::Stdio;

    let tools_str = tools.join(",");

    let mut cmd = tokio::process::Command::new("claude");
    // No --print — persistent conversation mode
    cmd.arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--include-partial-messages")
        .arg("--model")
        .arg(model);

    if !options.effort.is_empty() && options.effort != "off" {
        cmd.arg("--effort").arg(&options.effort);
    }

    if !system_prompt.is_empty() {
        cmd.arg("--append-system-prompt").arg(system_prompt);
    }

    if !tools_str.is_empty() {
        cmd.arg("--allowedTools").arg(&tools_str);
    }

    cmd.arg("--add-dir").arg(cwd);

    cmd.current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null()) // Persistent sessions don't capture stderr
        .stdin(Stdio::piped());

    tracing::info!(
        "Spawning persistent claude session: model={}, cwd={}",
        model,
        cwd.display()
    );

    let mut child = cmd.spawn().map_err(|e| {
        if e.raw_os_error() == Some(7) {
            anyhow::anyhow!(
                "spawning persistent claude subprocess: argument list too long.\n\
                 Lower `agent.graphBudgetTokens`, drop some @file refs, or switch to a codex/ollama model."
            )
        } else if matches!(e.kind(), std::io::ErrorKind::NotFound) {
            anyhow::anyhow!(
                "spawning persistent claude subprocess: {e}\n\
                 The `claude` CLI binary was not found on PATH. \
                 Install it from https://docs.anthropic.com/claude/docs/claude-code, \
                 or switch provider by setting agent.model to a `codex:...` / `ollama:...` spec."
            )
        } else {
            anyhow::anyhow!("spawning persistent claude subprocess: {e}")
        }
    })?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to capture persistent session stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to capture persistent session stdout"))?;

    Ok(PersistentSession {
        child,
        stdin,
        stdout: tokio::io::BufReader::new(stdout),
        line_buf: String::new(),
        model: model.to_string(),
        alive: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_mode_eq() {
        assert_eq!(SessionMode::OneShot, SessionMode::OneShot);
        assert_ne!(SessionMode::OneShot, SessionMode::Persistent);
    }

    #[tokio::test]
    async fn test_factory_no_persistent_session() {
        let factory = AcpSessionFactory::new(AgentOptions::default());
        assert!(!factory.is_session_alive("nonexistent").await);
    }

    #[tokio::test]
    async fn test_factory_kill_nonexistent_session() {
        let factory = AcpSessionFactory::new(AgentOptions::default());
        factory.kill_session("nonexistent").await; // Should not panic
    }

    #[tokio::test]
    async fn test_factory_active_sessions_empty() {
        let factory = AcpSessionFactory::new(AgentOptions::default());
        let active = factory.active_sessions().await;
        assert!(active.is_empty());
    }

    #[tokio::test]
    async fn test_factory_kill_all_empty() {
        let factory = AcpSessionFactory::new(AgentOptions::default());
        factory.kill_all().await; // Should not panic
    }

    #[tokio::test]
    async fn test_send_to_nonexistent_persistent_fails() {
        let factory = AcpSessionFactory::new(AgentOptions::default());
        let result = factory.send_to_persistent("missing", "hello").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no persistent session")
        );
    }
}
