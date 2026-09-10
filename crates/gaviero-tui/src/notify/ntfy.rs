//! Always-on ntfy publisher (remote-ntfy v1).
//!
//! POSTs JSON to `{server}/` on a spawned task so the TUI event loop never
//! waits on HTTP. Failures are `tracing::warn!` only — no chat banner, no
//! token/topic in logs.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gaviero_core::workspace::{Workspace, settings, user_settings_path};
use serde::Serialize;

use super::NotifyEvent;
use crate::app::App;

const DEFAULT_SERVER: &str = "https://ntfy.sh";
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(5);
const TOPIC_HEX_CHARS: usize = 32;

/// Resolved ntfy preferences. `token` is a secret — never log it, never
/// put it in `/remote` status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NtfyConfig {
    pub enabled: bool,
    pub server: String,
    pub topic: String,
    pub token: String,
    pub agent_finished: bool,
    pub agent_waiting: bool,
}

impl Default for NtfyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server: DEFAULT_SERVER.to_string(),
            topic: String::new(),
            token: String::new(),
            agent_finished: true,
            agent_waiting: true,
        }
    }
}

/// `~/.gaviero/ntfy/topic` — same home convention as `~/.gaviero/remote/token`.
pub fn default_topic_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".gaviero").join("ntfy").join("topic"))
}

/// Master switch only — does not mint a topic. Safe for `/remote` status.
pub fn ntfy_enabled(workspace: &Workspace, root: Option<&Path>) -> bool {
    workspace
        .resolve_setting(settings::NOTIFICATIONS_NTFY_ENABLED, root)
        .as_bool()
        .unwrap_or(false)
}

pub fn resolve_ntfy_config(
    workspace: &Workspace,
    root: Option<&Path>,
    topic_file: Option<&Path>,
) -> NtfyConfig {
    let enabled = ntfy_enabled(workspace, root);
    let server_raw = workspace
        .resolve_setting(settings::NOTIFICATIONS_NTFY_SERVER, root)
        .as_str()
        .unwrap_or(DEFAULT_SERVER)
        .trim()
        .trim_end_matches('/')
        .to_string();
    let server = if server_raw.is_empty() {
        DEFAULT_SERVER.to_string()
    } else {
        server_raw
    };
    let mut topic = workspace
        .resolve_setting(settings::NOTIFICATIONS_NTFY_TOPIC, root)
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let token = workspace
        .resolve_setting(settings::NOTIFICATIONS_NTFY_TOKEN, root)
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let agent_finished = workspace
        .resolve_setting(settings::NOTIFICATIONS_NTFY_AGENT_FINISHED, root)
        .as_bool()
        .unwrap_or(true);
    let agent_waiting = workspace
        .resolve_setting(settings::NOTIFICATIONS_NTFY_AGENT_WAITING, root)
        .as_bool()
        .unwrap_or(true);

    if enabled && topic.is_empty() {
        match topic_file
            .map(Path::to_path_buf)
            .or_else(default_topic_path)
        {
            Some(path) => match load_or_mint_topic(&path) {
                Ok(minted) => topic = minted,
                Err(e) => {
                    tracing::warn!(error = %e, "ntfy: could not mint topic file — skip publish");
                }
            },
            None => tracing::warn!("ntfy: no home directory to store topic — skip publish"),
        }
    }

    NtfyConfig {
        enabled,
        server,
        topic,
        token,
        agent_finished,
        agent_waiting,
    }
}

fn load_or_mint_topic(path: &Path) -> Result<String, String> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    let topic: String = gaviero_remote::pairing::generate_token()
        .chars()
        .take(TOPIC_HEX_CHARS)
        .collect();
    write_topic_at(path, &topic)?;
    Ok(topic)
}

/// Owner-only permissions where supported; on Windows inherit the parent
/// ACL the same way `~/.gaviero/remote/token` does.
fn write_topic_at(path: &Path, topic: &str) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| "ntfy topic path has no parent".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, topic).map_err(|e| format!("writing ntfy topic: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("replacing ntfy topic: {e}"))?;
    #[cfg(windows)]
    {
        if let Ok(meta) = std::fs::metadata(dir)
            && !meta.permissions().readonly()
        {
            tracing::info!(
                dir = %dir.display(),
                "ntfy topic stored with inherited directory ACL — restrict this folder if the machine is shared"
            );
        }
    }
    Ok(())
}

/// Title shown in the ntfy notification (not the desktop toast).
pub fn ntfy_title(event: NotifyEvent, workspace_display: &str) -> String {
    let name = if workspace_display.trim().is_empty() {
        "gaviero"
    } else {
        workspace_display.trim()
    };
    match event {
        NotifyEvent::AgentFinished => format!("{name} · Agent finished"),
        NotifyEvent::AgentWaiting => format!("{name} · Agent needs you"),
    }
}

/// `gaviero-remote://open?host=&workspace=&conv=` — identity for tap, not
/// conversation content.
pub fn ntfy_click_url(host: &str, workspace_id: &str, conv_id: &str) -> String {
    format!(
        "gaviero-remote://open?host={}&workspace={}&conv={}",
        percent_encode_query(host),
        percent_encode_query(workspace_id),
        percent_encode_query(conv_id),
    )
}

fn percent_encode_query(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Fire-and-forget POST. Never blocks the caller; never uses
/// `reqwest::blocking`.
pub fn publish_ntfy(config: &NtfyConfig, event: NotifyEvent, title: &str, body: &str, click: &str) {
    if !config.enabled {
        return;
    }
    match event {
        NotifyEvent::AgentFinished if !config.agent_finished => return,
        NotifyEvent::AgentWaiting if !config.agent_waiting => return,
        _ => {}
    }
    if config.topic.is_empty() {
        tracing::warn!("ntfy: enabled but topic is empty — skip publish");
        return;
    }

    let payload = NtfyJson {
        topic: config.topic.clone(),
        title: title.to_string(),
        message: body.to_string(),
        tags: match event {
            NotifyEvent::AgentFinished => vec!["white_check_mark".to_string()],
            NotifyEvent::AgentWaiting => vec!["question".to_string()],
        },
        priority: match event {
            NotifyEvent::AgentFinished => 3,
            NotifyEvent::AgentWaiting => 4,
        },
        click: click.to_string(),
    };
    let url = format!("{}/", config.server.trim_end_matches('/'));
    let token = config.token.clone();

    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(async move {
                if let Err(status) = post_ntfy(&url, token.as_str(), &payload).await {
                    tracing::warn!(status, "ntfy publish failed");
                }
            });
        }
        Err(_) => tracing::warn!("ntfy: no tokio runtime — skip publish"),
    }
}

#[derive(Debug, Serialize)]
struct NtfyJson {
    topic: String,
    title: String,
    message: String,
    tags: Vec<String>,
    priority: u8,
    click: String,
}

/// Returns `Ok(())` on 2xx. On transport error or non-2xx, `Err` is a
/// status code (`0` for transport) — never the topic or token.
async fn post_ntfy(url: &str, token: &str, payload: &NtfyJson) -> Result<(), u16> {
    let client = reqwest::Client::builder()
        .timeout(PUBLISH_TIMEOUT)
        .build()
        .map_err(|_| 0u16)?;
    let mut req = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(payload);
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await.map_err(|_| 0u16)?;
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(status.as_u16())
    }
}

fn server_is_ntfy_sh(server: &str) -> bool {
    let rest = server
        .trim()
        .trim_end_matches('/')
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    rest.eq_ignore_ascii_case("ntfy.sh")
}

fn subscribe_url(server: &str, topic: &str) -> String {
    format!("{}/{topic}", server.trim_end_matches('/'))
}

/// `/ntfy` / `/ntfy hide`. File reads + QR only — no HTTP from the event loop.
pub fn handle_ntfy_command(app: &mut App, line: &str) {
    let arg = line
        .trim()
        .strip_prefix("/ntfy")
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    if arg == "hide" {
        let idx = app.chat_state.active_conv;
        let conv = &mut app.chat_state.conversations[idx];
        for msg in &mut conv.messages {
            if msg.content.contains("ntfy subscribe URL") {
                msg.content = "[ntfy QR hidden — run /ntfy to show it again]".to_string();
            }
        }
        app.chat_state
            .add_system_message("ntfy: subscribe QR cleared from this transcript.");
        return;
    }

    let roots = app.workspace.roots();
    let root = roots.first().copied();
    let config = resolve_ntfy_config(&app.workspace, root, None);

    if !config.enabled {
        let settings_file = user_settings_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "~/.gaviero/settings.json".to_string());
        app.chat_state.add_system_message(&format!(
            "ntfy is off (notifications.ntfy.enabled defaults to false).\n\
             Set it in {settings_file} so every TUI on this \
             machine shares one topic:\n\n\
             {{\n  \"notifications\": {{\n    \"ntfy\": {{ \"enabled\": true }}\n  }}\n}}\n\n\
             Then restart gaviero and run /ntfy again. Do not put the topic \
             in a screenshot of /remote — that status only says \"ntfy: on\"."
        ));
        return;
    }

    if config.topic.is_empty() {
        app.chat_state.add_system_message(
            "ntfy is enabled but the topic could not be minted (no home \
             directory, or ~/.gaviero/ntfy/topic was unwritable). Fix that \
             and restart. To rotate later: delete ~/.gaviero/ntfy/topic and \
             any notifications.ntfy.topic setting, then restart — do not \
             rotate from this command (it would desync every phone).",
        );
        return;
    }

    let sub = subscribe_url(&config.server, &config.topic);
    let mut report = format!(
        "ntfy: enabled\nServer: {}\nTopic: {}\nSubscribe: {sub}\n",
        config.server, config.topic
    );
    if server_is_ntfy_sh(&config.server) {
        report.push_str(&format!("ntfy app: ntfy://ntfy.sh/{}\n", config.topic));
    }
    report.push_str(
        "\nInstall the ntfy app on the phone, then scan this ntfy subscribe URL \
         (same tailnet is not required for ntfy.sh):\n\n",
    );
    match crate::app::remote_setup::render_qr(&sub) {
        Ok(qr) => report.push_str(&qr),
        Err(e) => report.push_str(&format!("QR: {e}\n")),
    }
    report.push_str(&format!(
        "\n{sub}\n(Topic shown only in this message. /ntfy hide clears it.)\n\
         To rotate: delete ~/.gaviero/ntfy/topic and any notifications.ntfy.topic \
         setting, then restart gaviero. There is no /ntfy rotate — that would \
         desync every subscribed phone."
    ));
    app.chat_state.add_system_message(&report);
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn cfg(enabled: bool, server: &str, topic: &str) -> NtfyConfig {
        NtfyConfig {
            enabled,
            server: server.trim_end_matches('/').to_string(),
            topic: topic.to_string(),
            token: String::new(),
            agent_finished: true,
            agent_waiting: true,
        }
    }

    #[test]
    fn click_url_encodes_query_values() {
        let url = ntfy_click_url("neichtop.tail9b1d28.ts.net", "0b7d245998c0e8c3", "conv/1");
        assert_eq!(
            url,
            "gaviero-remote://open?host=neichtop.tail9b1d28.ts.net&workspace=0b7d245998c0e8c3&conv=conv%2F1"
        );
    }

    #[test]
    fn titles_use_workspace_display() {
        assert_eq!(
            ntfy_title(NotifyEvent::AgentFinished, "gaviero"),
            "gaviero · Agent finished"
        );
        assert_eq!(
            ntfy_title(NotifyEvent::AgentWaiting, ""),
            "gaviero · Agent needs you"
        );
    }

    #[test]
    fn mint_writes_32_hex_chars_to_injected_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("topic");
        let a = load_or_mint_topic(&path).expect("mint");
        let b = load_or_mint_topic(&path).expect("reload");
        assert_eq!(a, b, "second call must reuse the file");
        assert_eq!(a.len(), TOPIC_HEX_CHARS);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), a);
    }

    #[test]
    fn resolve_nested_settings_and_injects_topic_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
        std::fs::write(
            dir.path().join(".gaviero").join("settings.json"),
            r#"{
              "notifications": {
                "ntfy": { "enabled": true, "server": "https://ntfy.example/" }
              }
            }"#,
        )
        .unwrap();
        let ws = Workspace::single_folder(dir.path().to_path_buf());
        let topic_path = dir.path().join("injected-topic");
        let c = resolve_ntfy_config(&ws, None, Some(&topic_path));
        assert!(c.enabled);
        assert_eq!(c.server, "https://ntfy.example", "trailing slash stripped");
        assert_eq!(c.topic.len(), TOPIC_HEX_CHARS);
        assert!(topic_path.is_file(), "tests must not touch real home");
        assert!(c.agent_finished && c.agent_waiting);
    }

    #[test]
    fn disabled_config_does_not_mint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws = Workspace::single_folder(dir.path().to_path_buf());
        let topic_path = dir.path().join("should-not-exist");
        let c = resolve_ntfy_config(&ws, None, Some(&topic_path));
        assert!(!c.enabled);
        assert!(c.topic.is_empty());
        assert!(!topic_path.exists());
    }

    #[test]
    fn disabled_publish_is_a_noop_without_runtime() {
        publish_ntfy(
            &cfg(false, "http://127.0.0.1:1", "topic"),
            NotifyEvent::AgentFinished,
            "title",
            "body",
            "click",
        );
    }

    #[tokio::test]
    async fn enabled_publishes_once() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let click = ntfy_click_url("host.ts.net", "abc", "c1");
        publish_ntfy(
            &cfg(true, &server.uri(), "secrettopic"),
            NotifyEvent::AgentFinished,
            "gaviero · Agent finished",
            "Agent finished (claude:sonnet) — no file changes",
            &click,
        );

        let mut body = String::new();
        for _ in 0..50 {
            let reqs = server.received_requests().await.unwrap();
            if let Some(req) = reqs.first() {
                body = String::from_utf8_lossy(&req.body).into_owned();
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(body.contains("\"topic\":\"secrettopic\""), "{body}");
        assert!(body.contains("\"priority\":3"), "{body}");
        assert!(body.contains("white_check_mark"), "{body}");
        assert!(body.contains(&click), "{body}");
        assert!(!body.contains("Authorization"), "{body}");
    }

    #[tokio::test]
    async fn disabled_publishes_zero() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        publish_ntfy(
            &cfg(false, &server.uri(), "secrettopic"),
            NotifyEvent::AgentFinished,
            "title",
            "body",
            "click",
        );
        tokio::time::sleep(Duration::from_millis(80)).await;
    }

    #[tokio::test]
    async fn per_event_switch_skips_finished() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let mut c = cfg(true, &server.uri(), "secrettopic");
        c.agent_finished = false;
        publish_ntfy(&c, NotifyEvent::AgentFinished, "t", "b", "c");
        tokio::time::sleep(Duration::from_millis(80)).await;
    }

    #[tokio::test]
    async fn waiting_uses_priority_4_and_optional_bearer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("authorization", "Bearer tk_test"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let mut c = cfg(true, &server.uri(), "secrettopic");
        c.token = "tk_test".into();
        publish_ntfy(&c, NotifyEvent::AgentWaiting, "t", "b", "click");
        for _ in 0..50 {
            if !server.received_requests().await.unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1);
        let body = String::from_utf8_lossy(&reqs[0].body);
        assert!(body.contains("\"priority\":4"), "{body}");
        assert!(body.contains("question"), "{body}");
    }

    #[test]
    fn ntfy_sh_detect_ignores_scheme_and_slash() {
        assert!(server_is_ntfy_sh("https://ntfy.sh"));
        assert!(server_is_ntfy_sh("https://ntfy.sh/"));
        assert!(!server_is_ntfy_sh("https://ntfy.example"));
    }
}
