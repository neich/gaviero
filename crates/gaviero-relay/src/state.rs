//! Shared relay state. Every lock is a short `std::sync::Mutex` section and
//! is never held across an `.await`.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::ws::WebSocket;
use subtle::ConstantTimeEq;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::config::Config;
use crate::proto::{NotifyKind, PhoneEvent, RelayMsg, RouteInfo};

struct RouteEntry {
    conn_id: u64,
    tx: mpsc::Sender<RelayMsg>,
    info: RouteInfo,
}

struct StoredNote {
    at: Instant,
    event: PhoneEvent,
    id: u64,
}

pub(crate) struct Relay {
    pub cfg: Config,
    routes: Mutex<HashMap<String, RouteEntry>>,
    pending: Mutex<HashMap<String, oneshot::Sender<WebSocket>>>,
    notes: Mutex<VecDeque<StoredNote>>,
    sessions_per_ip: Mutex<HashMap<IpAddr, usize>>,
    next_note: AtomicU64,
    next_conn: AtomicU64,
    pub events: broadcast::Sender<PhoneEvent>,
    /// Random per process start; notification ids are only comparable
    /// within one epoch.
    pub epoch: String,
}

impl Relay {
    pub fn new(cfg: Config) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            cfg,
            routes: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            notes: Mutex::new(VecDeque::new()),
            sessions_per_ip: Mutex::new(HashMap::new()),
            next_note: AtomicU64::new(1),
            next_conn: AtomicU64::new(1),
            events,
            epoch: crate::util::random_hex(8),
        }
    }

    /// `Authorization: Bearer <secret>`, compared in constant time.
    pub fn authorized(&self, header: Option<&str>) -> bool {
        let Some(token) = header.and_then(|h| h.strip_prefix("Bearer ")) else {
            return false;
        };
        let (a, b) = (token.trim().as_bytes(), self.cfg.secret.as_bytes());
        a.len() == b.len() && bool::from(a.ct_eq(b))
    }

    pub fn next_conn_id(&self) -> u64 {
        self.next_conn.fetch_add(1, Ordering::Relaxed)
    }

    // ── Routes ──────────────────────────────────────────────────────

    /// Newest registration of a route wins (a TUI restart replaces its
    /// stale entry immediately instead of waiting for the old socket to die).
    pub fn register(
        &self,
        conn_id: u64,
        tx: &mpsc::Sender<RelayMsg>,
        info: RouteInfo,
    ) -> Result<(), String> {
        crate::proto::validate_route(&info.route, self.cfg.public_host.as_deref())?;
        let mut routes = self.routes.lock().unwrap();
        let owned = routes.values().filter(|r| r.conn_id == conn_id).count();
        if owned >= self.cfg.max_routes_per_host && !routes.contains_key(&info.route) {
            return Err("too many routes on one connection".into());
        }
        routes.insert(
            info.route.clone(),
            RouteEntry {
                conn_id,
                tx: tx.clone(),
                info,
            },
        );
        Ok(())
    }

    pub fn unregister(&self, conn_id: u64, route: &str) {
        let mut routes = self.routes.lock().unwrap();
        if routes.get(route).is_some_and(|r| r.conn_id == conn_id) {
            routes.remove(route);
        }
    }

    /// Drop every route a closed control connection owned.
    pub fn drop_conn(&self, conn_id: u64) -> bool {
        let mut routes = self.routes.lock().unwrap();
        let before = routes.len();
        routes.retain(|_, r| r.conn_id != conn_id);
        routes.len() != before
    }

    pub fn lookup(&self, route: &str) -> Option<mpsc::Sender<RelayMsg>> {
        self.routes.lock().unwrap().get(route).map(|r| r.tx.clone())
    }

    pub fn presence(&self) -> PhoneEvent {
        let mut routes: Vec<RouteInfo> = self
            .routes
            .lock()
            .unwrap()
            .values()
            .map(|r| r.info.clone())
            .collect();
        routes.sort_by(|a, b| a.route.cmp(&b.route));
        PhoneEvent::Presence {
            epoch: self.epoch.clone(),
            routes,
        }
    }

    pub fn broadcast_presence(&self) {
        let _ = self.events.send(self.presence());
    }

    // ── Pending phone sessions ──────────────────────────────────────

    pub fn add_pending(&self, nonce: String, tx: oneshot::Sender<WebSocket>) {
        self.pending.lock().unwrap().insert(nonce, tx);
    }

    pub fn take_pending(&self, nonce: &str) -> Option<oneshot::Sender<WebSocket>> {
        self.pending.lock().unwrap().remove(nonce)
    }

    // ── Notifications ───────────────────────────────────────────────

    pub fn push_note(&self, machine: String, workspace_id: String, conv: String, kind: NotifyKind) {
        let id = self.next_note.fetch_add(1, Ordering::Relaxed);
        let event = PhoneEvent::Notify {
            id,
            ts: crate::util::now_unix(),
            machine,
            workspace_id,
            conv,
            kind,
        };
        {
            let mut notes = self.notes.lock().unwrap();
            Self::expire(&mut notes, &self.cfg);
            notes.push_back(StoredNote {
                at: Instant::now(),
                event: event.clone(),
                id,
            });
            while notes.len() > self.cfg.note_capacity {
                notes.pop_front();
            }
        }
        let _ = self.events.send(event);
    }

    /// The highest notification id this process has issued (0 if none).
    pub fn max_note_id(&self) -> u64 {
        self.next_note.load(Ordering::Relaxed).saturating_sub(1)
    }

    /// Unexpired notifications with `id > since`, oldest first.
    pub fn notes_since(&self, since: u64) -> Vec<PhoneEvent> {
        let mut notes = self.notes.lock().unwrap();
        Self::expire(&mut notes, &self.cfg);
        notes
            .iter()
            .filter(|n| n.id > since)
            .map(|n| n.event.clone())
            .collect()
    }

    fn expire(notes: &mut VecDeque<StoredNote>, cfg: &Config) {
        while notes.front().is_some_and(|n| n.at.elapsed() > cfg.note_ttl) {
            notes.pop_front();
        }
    }

    // ── Per-IP session limit ────────────────────────────────────────

    pub fn session_guard(self: &Arc<Self>, ip: IpAddr) -> Option<SessionGuard> {
        let mut map = self.sessions_per_ip.lock().unwrap();
        let count = map.entry(ip).or_insert(0);
        if *count >= self.cfg.max_sessions_per_ip {
            return None;
        }
        *count += 1;
        Some(SessionGuard {
            relay: self.clone(),
            ip,
        })
    }
}

/// Releases one per-IP session slot on drop.
pub(crate) struct SessionGuard {
    relay: Arc<Relay>,
    ip: IpAddr,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let mut map = self.relay.sessions_per_ip.lock().unwrap();
        if let Some(count) = map.get_mut(&self.ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(&self.ip);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay() -> Relay {
        let mut cfg = Config::new("s3cret");
        cfg.note_capacity = 3;
        Relay::new(cfg)
    }

    fn info(route: &str) -> RouteInfo {
        RouteInfo {
            route: route.into(),
            machine: "desk".into(),
            workspace_id: "w".into(),
            display_name: "ws".into(),
        }
    }

    #[test]
    fn bearer_auth_is_exact() {
        let r = relay();
        assert!(r.authorized(Some("Bearer s3cret")));
        assert!(!r.authorized(Some("Bearer s3cre")));
        assert!(!r.authorized(Some("s3cret")));
        assert!(!r.authorized(None));
    }

    #[test]
    fn newest_registration_wins_and_closing_drops_only_own_routes() {
        let r = relay();
        let (tx1, _rx1) = mpsc::channel(1);
        let (tx2, _rx2) = mpsc::channel(1);
        r.register(1, &tx1, info("a.x.test")).unwrap();
        r.register(2, &tx2, info("a.x.test")).unwrap();
        assert!(r.lookup("a.x.test").unwrap().same_channel(&tx2));
        assert!(!r.drop_conn(1), "conn 1 no longer owns the route");
        assert!(r.lookup("a.x.test").is_some());
        assert!(r.drop_conn(2));
        assert!(r.lookup("a.x.test").is_none());
    }

    #[test]
    fn notes_are_capped_and_replayed_after_since() {
        let r = relay();
        for i in 0..5 {
            r.push_note("m".into(), "w".into(), format!("c{i}"), NotifyKind::AgentFinished);
        }
        let all = r.notes_since(0);
        assert_eq!(all.len(), 3, "capacity 3 keeps the newest");
        let ids: Vec<u64> = all
            .iter()
            .map(|e| match e {
                PhoneEvent::Notify { id, .. } => *id,
                _ => 0,
            })
            .collect();
        assert_eq!(ids, vec![3, 4, 5]);
        assert_eq!(r.notes_since(4).len(), 1);
    }
}
