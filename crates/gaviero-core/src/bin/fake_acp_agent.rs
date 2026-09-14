//! Scripted ACP agent for `dsh:` tests. Not a user-facing binary.

#[path = "../agent_session/agent_client_protocol/fake.rs"]
mod fake;

#[tokio::main]
async fn main() {
    let scenario = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("GAVIERO_FAKE_ACP").ok())
        .unwrap_or_else(|| "happy".to_string());
    if let Err(e) =
        fake::serve_stdio(&scenario).await
    {
        eprintln!("fake-acp-agent: {e:#}");
        std::process::exit(1);
    }
}
