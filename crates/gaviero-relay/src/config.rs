use std::net::SocketAddr;
use std::time::Duration;

/// Relay settings. `Debug` never prints the secret.
#[derive(Clone)]
pub struct Config {
    /// Control/data WebSockets and the phone stream (behind nginx's HTTPS).
    pub http_addr: SocketAddr,
    /// Raw TLS from phones, routed by SNI (behind nginx `stream`).
    pub splice_addr: SocketAddr,
    /// Shared bearer secret for hosts and phones. The relay is not the
    /// security boundary — end-to-end TLS with the pinned CA is — so one
    /// secret only gates who may use the relay at all.
    pub secret: String,
    /// When set, routes must be `<label>.<public_host>`.
    pub public_host: Option<String>,
    /// Expect a PROXY v1 line before the ClientHello on the splice port
    /// (nginx `stream { proxy_protocol on; }`).
    pub proxy_protocol: bool,
    /// How long notifications are kept for phones that reconnect.
    pub note_ttl: Duration,
    pub note_capacity: usize,
    /// Phone waiting for its host to open the data channel.
    pub pending_timeout: Duration,
    /// Time allowed for the PROXY line and the ClientHello.
    pub hello_timeout: Duration,
    /// Spliced session with no bytes either way (the protocol pings every
    /// 20 s, so this only fires on a dead peer).
    pub idle_timeout: Duration,
    /// Control-channel pings and phone-stream keepalive lines.
    pub keepalive: Duration,
    pub max_hello_bytes: usize,
    pub max_sessions_per_ip: usize,
    pub max_routes_per_host: usize,
}

impl Config {
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            http_addr: ([127, 0, 0, 1], 7080).into(),
            splice_addr: ([127, 0, 0, 1], 7443).into(),
            secret: secret.into(),
            public_host: None,
            proxy_protocol: false,
            note_ttl: Duration::from_secs(6 * 3600),
            note_capacity: 200,
            pending_timeout: Duration::from_secs(10),
            hello_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(180),
            keepalive: Duration::from_secs(25),
            max_hello_bytes: 16 * 1024,
            max_sessions_per_ip: 32,
            max_routes_per_host: 32,
        }
    }
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("http_addr", &self.http_addr)
            .field("splice_addr", &self.splice_addr)
            .field("secret", &"<redacted>")
            .field("public_host", &self.public_host)
            .field("proxy_protocol", &self.proxy_protocol)
            .field("note_ttl", &self.note_ttl)
            .finish_non_exhaustive()
    }
}
