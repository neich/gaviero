use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "gaviero-relay", version, about = "Gaviero Remote relay (run behind nginx)")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    serve: ServeArgs,
}

#[derive(Subcommand)]
enum Command {
    /// Print a new random secret (put it in the --secret-file).
    GenSecret,
}

#[derive(clap::Args)]
struct ServeArgs {
    /// HTTP listener for /v1/host, /v1/data, /v1/phone/stream (nginx https proxies here).
    #[arg(long, default_value = "127.0.0.1:7080")]
    http: SocketAddr,
    /// Raw TLS splice listener (nginx stream forwards every non-relay SNI here).
    #[arg(long, default_value = "127.0.0.1:7443")]
    splice: SocketAddr,
    /// File holding the shared secret (first line).
    #[arg(long)]
    secret_file: Option<PathBuf>,
    /// The relay's own name; routes must be `<label>.<public-host>`.
    #[arg(long)]
    public_host: Option<String>,
    /// Expect a PROXY v1 header on the splice port (nginx `proxy_protocol on`).
    #[arg(long)]
    proxy_protocol: bool,
    /// Hours a notification is kept for phones that reconnect later.
    #[arg(long, default_value_t = 6)]
    note_ttl_hours: u64,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    if let Some(Command::GenSecret) = cli.command {
        println!("{}", gaviero_relay::generate_secret());
        return std::process::ExitCode::SUCCESS;
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "gaviero_relay=info".into()),
        )
        .init();

    let args = cli.serve;
    let Some(path) = args.secret_file else {
        eprintln!("--secret-file is required (create one with `gaviero-relay gen-secret`)");
        return std::process::ExitCode::from(2);
    };
    let secret = match std::fs::read_to_string(&path) {
        Ok(text) => text.lines().next().unwrap_or("").trim().to_string(),
        Err(e) => {
            eprintln!("cannot read {}: {e}", path.display());
            return std::process::ExitCode::from(2);
        }
    };
    let mut cfg = gaviero_relay::Config::new(secret);
    cfg.http_addr = args.http;
    cfg.splice_addr = args.splice;
    cfg.public_host = args.public_host.map(|h| h.trim().trim_end_matches('.').to_ascii_lowercase());
    cfg.proxy_protocol = args.proxy_protocol;
    cfg.note_ttl = Duration::from_secs(args.note_ttl_hours * 3600);

    match gaviero_relay::start(cfg).await {
        Ok(running) => {
            tokio::select! {
                _ = running.wait() => std::process::ExitCode::FAILURE,
                _ = tokio::signal::ctrl_c() => std::process::ExitCode::SUCCESS,
            }
        }
        Err(e) => {
            eprintln!("gaviero-relay: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
