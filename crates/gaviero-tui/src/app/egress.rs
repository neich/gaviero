//! Egress proxy wiring: install [`gaviero_core::egress::EgressService`] at
//! startup and surface its warnings and `ask` prompts in the chat.

use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;

use gaviero_core::egress::{
    EgressConfig, EgressEvent, EgressObserver, EgressService, NetworkMode,
};

use super::App;
use crate::event::Event;

/// Forwards proxy decisions into the TUI event loop.
pub(crate) struct TuiEgressObserver {
    pub tx: mpsc::UnboundedSender<Event>,
}

impl EgressObserver for TuiEgressObserver {
    fn on_egress(&self, event: &EgressEvent) {
        let _ = self.tx.send(Event::Egress(event.clone()));
    }

    fn ask(&self, provider: &str, host: &str, respond: tokio::sync::oneshot::Sender<bool>) {
        let _ = self.tx.send(Event::EgressAsk {
            provider: provider.to_string(),
            host: host.to_string(),
            respond,
        });
    }
}

/// Install the process-wide egress proxy for `root` unless
/// `agent.network.mode` is `off`. Must run inside the tokio runtime.
pub(crate) fn install(app: &App, root: &Path, tx: mpsc::UnboundedSender<Event>) {
    let config = EgressConfig::from_workspace(&app.workspace, root);
    if config.mode == NetworkMode::Off {
        return;
    }
    gaviero_core::egress::install(EgressService::new(
        config,
        Arc::new(TuiEgressObserver { tx }),
    ));
}

/// `Event::Egress`: warn once per provider/host that is not allowlisted.
pub(crate) fn on_egress(app: &mut App, event: &EgressEvent) {
    if !event.first_seen || event.decision == "allowed" {
        return;
    }
    app.chat_state.add_system_message(&format!(
        "Egress: {} → {}:{} ({}; not in agent.network.allow)",
        event.provider, event.host, event.port, event.decision
    ));
}

/// `Event::EgressAsk`: re-raise as a permission request on the active
/// conversation so the user answers in the usual overlay. No conversation →
/// deny.
pub(crate) fn on_ask(
    app: &mut App,
    provider: String,
    host: String,
    respond: tokio::sync::oneshot::Sender<bool>,
) {
    let Some(conv_id) = app
        .chat_state
        .conversations
        .get(app.chat_state.active_conv)
        .map(|c| c.id.clone())
    else {
        let _ = respond.send(false);
        return;
    };
    let (decision_tx, decision_rx) =
        tokio::sync::oneshot::channel::<gaviero_core::observer::PermissionDecision>();
    tokio::spawn(async move {
        let allow = decision_rx.await.map(|d| d.is_allow()).unwrap_or(false);
        let _ = respond.send(allow);
    });
    let _ = app.event_tx.send(Event::PermissionRequest {
        conv_id,
        tool_name: "Network".to_string(),
        description: format!("{provider} wants to connect to {host}"),
        input: serde_json::json!({ "host": host, "provider": provider }),
        respond: decision_tx,
    });
}
