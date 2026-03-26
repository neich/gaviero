//! Mock backend for testing.
//!
//! Emits a configurable sequence of [`UnifiedStreamEvent`]s without
//! touching any external process or network.

use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;
use futures::Stream;
use tokio_stream::wrappers::ReceiverStream;

use super::{AgentBackend, Capabilities, CompletionRequest, UnifiedStreamEvent};

/// A test double that emits a pre-configured event sequence.
pub struct MockBackend {
    name: String,
    capabilities: Capabilities,
    events: Vec<UnifiedStreamEvent>,
    health_ok: bool,
    delay: Option<Duration>,
}

impl MockBackend {
    /// Create a mock with the given name and event sequence.
    /// Health check succeeds by default.
    pub fn new(name: &str, events: Vec<UnifiedStreamEvent>) -> Self {
        Self {
            name: name.to_string(),
            capabilities: Capabilities::default(),
            events,
            health_ok: true,
            delay: None,
        }
    }

    /// Set custom capabilities.
    pub fn with_capabilities(mut self, caps: Capabilities) -> Self {
        self.capabilities = caps;
        self
    }

    /// Make health_check fail.
    pub fn with_health_failure(mut self) -> Self {
        self.health_ok = false;
        self
    }

    /// Add a delay between events (useful for timeout testing).
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }
}

#[async_trait::async_trait]
impl AgentBackend for MockBackend {
    async fn stream_completion(
        &self,
        _request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let events = self.events.clone();
        let delay = self.delay;

        tokio::spawn(async move {
            for event in events {
                if let Some(d) = delay {
                    tokio::time::sleep(d).await;
                }
                if tx.send(Ok(event)).await.is_err() {
                    break; // receiver dropped
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities.clone()
    }

    fn name(&self) -> &str {
        &self.name
    }

    async fn health_check(&self) -> Result<()> {
        if self.health_ok {
            Ok(())
        } else {
            anyhow::bail!("mock health check failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::backend::StopReason;
    use futures::StreamExt;
    use std::path::PathBuf;

    // Test 9: MockBackend error emission
    #[tokio::test]
    async fn test_mock_error_emission() {
        let events = vec![
            UnifiedStreamEvent::Error("timeout".into()),
            UnifiedStreamEvent::Done(StopReason::Timeout),
        ];
        let backend = MockBackend::new("err-mock", events);

        let req = CompletionRequest {
            prompt: "test".into(),
            system_prompt: None,
            workspace_root: PathBuf::from("/tmp"),
            allowed_tools: vec![],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
        };

        let mut stream = backend.stream_completion(req).await.unwrap();
        let e1 = stream.next().await.unwrap().unwrap();
        let e2 = stream.next().await.unwrap().unwrap();
        assert_eq!(e1, UnifiedStreamEvent::Error("timeout".into()));
        assert_eq!(e2, UnifiedStreamEvent::Done(StopReason::Timeout));
        assert!(stream.next().await.is_none());
    }

    // Test 10: MockBackend empty stream
    #[tokio::test]
    async fn test_mock_empty_stream() {
        let backend = MockBackend::new("empty", vec![]);

        let req = CompletionRequest {
            prompt: "test".into(),
            system_prompt: None,
            workspace_root: PathBuf::from("/tmp"),
            allowed_tools: vec![],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
        };

        let mut stream = backend.stream_completion(req).await.unwrap();
        // Stream should end immediately
        assert!(stream.next().await.is_none());
    }

    // Health check failure test
    #[tokio::test]
    async fn test_mock_health_failure() {
        let backend = MockBackend::new("sick", vec![]).with_health_failure();
        assert!(backend.health_check().await.is_err());
    }
}
