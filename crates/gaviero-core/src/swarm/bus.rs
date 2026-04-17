//! Inter-agent communication bus.
//!
//! Provides broadcast messaging (all agents see it) and per-agent inboxes
//! (targeted messages). Used by the swarm pipeline for coordination.

use std::collections::HashMap;

use tokio::sync::{broadcast, mpsc};

/// A message on the agent bus.
#[derive(Debug, Clone)]
pub struct BusMessage {
    pub from: String,
    pub to: Option<String>, // None = broadcast
    pub content: String,
}

/// Inter-agent communication bus.
pub struct AgentBus {
    /// Broadcast channel — all agents receive these.
    broadcast_tx: broadcast::Sender<BusMessage>,
    /// Per-agent inboxes for targeted messages.
    inboxes: HashMap<String, mpsc::UnboundedSender<BusMessage>>,
}

impl AgentBus {
    pub fn new() -> Self {
        const BUS_CHANNEL_CAPACITY: usize = 256;
        let (broadcast_tx, _) = broadcast::channel(BUS_CHANNEL_CAPACITY);
        Self {
            broadcast_tx,
            inboxes: HashMap::new(),
        }
    }

    /// Register an agent and return its broadcast receiver + inbox receiver.
    pub fn register(
        &mut self,
        agent_id: &str,
    ) -> (
        broadcast::Receiver<BusMessage>,
        mpsc::UnboundedReceiver<BusMessage>,
    ) {
        let broadcast_rx = self.broadcast_tx.subscribe();
        let (inbox_tx, inbox_rx) = mpsc::unbounded_channel();
        self.inboxes.insert(agent_id.to_string(), inbox_tx);
        (broadcast_rx, inbox_rx)
    }

    /// Send a broadcast message to all agents.
    pub fn broadcast(&self, from: &str, content: &str) {
        let _ = self.broadcast_tx.send(BusMessage {
            from: from.to_string(),
            to: None,
            content: content.to_string(),
        });
    }

    /// Send a targeted message to a specific agent's inbox.
    pub fn send_to(&self, from: &str, to: &str, content: &str) -> bool {
        if let Some(tx) = self.inboxes.get(to) {
            tx.send(BusMessage {
                from: from.to_string(),
                to: Some(to.to_string()),
                content: content.to_string(),
            })
            .is_ok()
        } else {
            false
        }
    }

    /// Unregister an agent (drops its inbox sender).
    pub fn unregister(&mut self, agent_id: &str) {
        self.inboxes.remove(agent_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_broadcast() {
        let mut bus = AgentBus::new();
        let (mut rx1, _inbox1) = bus.register("agent-1");
        let (mut rx2, _inbox2) = bus.register("agent-2");

        bus.broadcast("agent-1", "hello everyone");

        let msg1 = rx1.recv().await.unwrap();
        assert_eq!(msg1.content, "hello everyone");
        assert_eq!(msg1.from, "agent-1");

        let msg2 = rx2.recv().await.unwrap();
        assert_eq!(msg2.content, "hello everyone");
    }

    #[tokio::test]
    async fn test_targeted_message() {
        let mut bus = AgentBus::new();
        let (_rx1, mut inbox1) = bus.register("agent-1");
        let (_rx2, mut inbox2) = bus.register("agent-2");

        assert!(bus.send_to("agent-1", "agent-2", "just for you"));
        assert!(!bus.send_to("agent-1", "agent-3", "nonexistent")); // no such agent

        let msg = inbox2.recv().await.unwrap();
        assert_eq!(msg.content, "just for you");
        assert_eq!(msg.to, Some("agent-2".to_string()));

        // agent-1's inbox should be empty
        assert!(inbox1.try_recv().is_err());
    }
}
