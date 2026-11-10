//! Portable input: crossterm events, re-encoded as VT bytes by the host.
//! The Unix input path (Windows uses raw console records instead).

use super::HostEvent;
use crossterm::event::Event;
use tokio::sync::mpsc::UnboundedSender;

/// Read crossterm events on a std thread until reading fails or the host
/// loop is gone.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn spawn_crossterm_reader(tx: UnboundedSender<HostEvent>) {
    let spawned = std::thread::Builder::new()
        .name("session-host-input".into())
        .spawn(move || {
            while let Ok(event) = crossterm::event::read() {
                let event = match event {
                    Event::Resize(..) => HostEvent::Resize,
                    other => HostEvent::Term(other),
                };
                if tx.send(event).is_err() {
                    return;
                }
            }
        });
    if let Err(e) = spawned {
        tracing::error!("could not start the input reader: {e}");
    }
}
