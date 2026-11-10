//! One session: a `gaviero` child in a pseudo-terminal, its emulated screen,
//! and the status it reports through its window title.

use super::{CONTROL_ENV, HOST_ENV, HostEvent, encode};
use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

/// Title prefix a session child uses to report its status (`main.rs`).
const STATUS_TITLE_PREFIX: &str = "gaviero-session:";

/// Keeps the last window title (OSC 0 / OSC 2) the child set.
#[derive(Default)]
pub(crate) struct TitleCb {
    pub title: Option<String>,
}

impl vt100::Callbacks for TitleCb {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = Some(String::from_utf8_lossy(title).into_owned());
    }
}

/// What a session is doing, from its status title: the two circles on
/// its tab (`render::tab_badges`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Activity {
    /// Waiting for the user: a permission prompt, an agent's question or a
    /// pending turn review (`attention`).
    pub waiting: bool,
    /// An agent or a swarm script is running.
    pub agent: bool,
    /// A command is running in one of the session's shells.
    pub shell: bool,
}

/// The activity a child reports in its window title
/// (`gaviero-session:<idle|busy|attention>[:work,...]`); anything else is
/// none. A child that reports `busy` with no detail (an older build) counts
/// as an agent running.
pub(crate) fn parse_activity(title: Option<&str>) -> Activity {
    let Some((status, detail)) = status_fields(title) else {
        return Activity::default();
    };
    let has = |keyword: &str| detail.split(',').any(|k| k == keyword);
    let agent = has("agent") || has("swarm");
    let shell = has("terminal");
    Activity {
        waiting: status == "attention",
        agent: agent || (status == "busy" && !shell),
        shell,
    }
}

/// Which of a background tab's work ended between `before` and `after`:
/// `(agent, shell)`. Each sets that tab's "finished" circle until the tab is
/// made current.
pub(crate) fn finished(before: Activity, after: Activity) -> (bool, bool) {
    (before.agent && !after.agent, before.shell && !after.shell)
}

/// What closing the child now would cut off, from its status title, as the
/// phrases the close guard shows. A child that reports `busy` with no
/// detail (an older build) has "a task" running.
pub(crate) fn parse_work(title: Option<&str>) -> Vec<&'static str> {
    let Some((status, detail)) = status_fields(title) else {
        return Vec::new();
    };
    let mut work: Vec<&'static str> = detail
        .split(',')
        .filter_map(|keyword| match keyword {
            "agent" => Some("an agent is running"),
            "swarm" => Some("a swarm is running"),
            "terminal" => Some("a shell is running a command"),
            "unsaved" => Some("files are not saved"),
            _ => None,
        })
        .collect();
    if work.is_empty() && status == "busy" {
        work.push("a task is running");
    }
    work
}

/// `(status, detail)` of a status title; detail is empty when absent.
fn status_fields(title: Option<&str>) -> Option<(&str, &str)> {
    let rest = title?.strip_prefix(STATUS_TITLE_PREFIX)?;
    Some(rest.split_once(':').unwrap_or((rest, "")))
}

/// The tab label for `path`: a workspace file's stem, a folder's name;
/// ` (2)`, ` (3)`, … when `taken` already holds that label.
pub(crate) fn label_for(path: &Path, taken: &[String]) -> String {
    let is_workspace = path
        .extension()
        .is_some_and(|ext| ext == "gaviero-workspace");
    let name = if is_workspace {
        path.file_stem()
    } else {
        path.file_name()
    };
    let base = name
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base} ({n})"))
        .find(|label| !taken.contains(label))
        .expect("an unused label")
}

pub(crate) struct Session {
    pub id: u64,
    pub path: PathBuf,
    pub label: String,
    pub parser: vt100::Parser<TitleCb>,
    master: Box<dyn MasterPty + Send>,
    input: std::sync::mpsc::Sender<Vec<u8>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pub started: Instant,
    pub launch_state: PathBuf,
    pub control: PathBuf,
    pub activity: Activity,
    /// What closing this session now would cut off (`parse_work`).
    pub work: Vec<&'static str>,
    /// An agent turn / a shell command ended while this tab was not
    /// current; cleared when it is made current.
    pub agent_done_unseen: bool,
    pub shell_done_unseen: bool,
    /// Exited for a restart too soon after starting; a delayed
    /// [`HostEvent::Relaunch`] is on its way.
    pub restart_pending: bool,
}

/// Start `gaviero <path>` in a `rows` × `cols` pseudo-terminal. Output and
/// end-of-output arrive on `tx` as [`HostEvent::Output`] / [`HostEvent::Exited`].
pub(crate) fn spawn(
    id: u64,
    path: &Path,
    label: String,
    rows: u16,
    cols: u16,
    state_dir: &Path,
    tx: UnboundedSender<HostEvent>,
) -> Result<Session> {
    let rows = rows.max(2);
    let cols = cols.max(10);
    let launch_state = state_dir.join(format!("{id}.launch"));
    let control = state_dir.join(format!("{id}.ctl"));

    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("opening a pseudo-terminal")?;

    let exe = std::env::current_exe().context("locating the gaviero executable")?;
    let mut cmd = CommandBuilder::new(exe);
    cmd.arg(path);
    let cwd = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    cmd.cwd(cwd);
    for (key, value) in crate::supervisor::child_env(&launch_state) {
        cmd.env(key, value);
    }
    cmd.env(HOST_ENV, "1");
    cmd.env(CONTROL_ENV, &control);

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("starting gaviero for {}", path.display()))?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("cloning the pseudo-terminal reader")?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("taking the pseudo-terminal writer")?;

    std::thread::Builder::new()
        .name(format!("session-{id}-reader"))
        .spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => {
                        let _ = tx.send(HostEvent::Exited { id, code: None });
                        break;
                    }
                    Ok(n) => {
                        let bytes = buf[..n].to_vec();
                        if tx.send(HostEvent::Output { id, bytes }).is_err() {
                            break;
                        }
                    }
                }
            }
        })
        .context("starting the session reader")?;

    // A child that stops reading its input must never block the host loop,
    // so writes happen on their own thread.
    let (input, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::Builder::new()
        .name(format!("session-{id}-writer"))
        .spawn(move || {
            while let Ok(bytes) = input_rx.recv() {
                if writer.write_all(&bytes).and_then(|_| writer.flush()).is_err() {
                    break;
                }
            }
        })
        .context("starting the session writer")?;

    Ok(Session {
        id,
        path: path.to_path_buf(),
        label,
        parser: vt100::Parser::new_with_callbacks(rows, cols, 0, TitleCb::default()),
        master: pair.master,
        input,
        child,
        started: Instant::now(),
        launch_state,
        control,
        activity: Activity::default(),
        work: Vec::new(),
        agent_done_unseen: false,
        shell_done_unseen: false,
        restart_pending: false,
    })
}

impl Session {
    /// Queue bytes for the child's input.
    pub fn send(&self, bytes: Vec<u8>) {
        let _ = self.input.send(bytes);
    }

    /// Feed child output into the screen, answer its terminal queries, and
    /// update its activity from its title.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        let replies = encode::query_replies(bytes, self.parser.screen().cursor_position());
        if !replies.is_empty() {
            self.send(replies);
        }
        let title = self.parser.callbacks().title.as_deref();
        self.activity = parse_activity(title);
        self.work = parse_work(title);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(2);
        let cols = cols.max(10);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.parser.screen_mut().set_size(rows, cols);
    }

    /// The child's exit code once it has exited; `None` while it runs.
    /// Never blocks: a pseudo console may report end-of-output before (or
    /// long after) its child is gone.
    pub fn exit_code(&mut self) -> Option<u32> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.exit_code()),
            Ok(None) => None,
            // The handle is unusable: treat it as gone.
            Err(_) => Some(1),
        }
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_parses_from_the_title() {
        let activity = |t| parse_activity(Some(t));
        assert_eq!(activity("gaviero-session:idle"), Activity::default());
        assert_eq!(activity("gaviero-session:other"), Activity::default());
        assert_eq!(parse_activity(Some("vim")), Activity::default());
        assert_eq!(parse_activity(None), Activity::default());
        assert_eq!(
            activity("gaviero-session:busy:agent,terminal"),
            Activity { waiting: false, agent: true, shell: true }
        );
        assert_eq!(
            activity("gaviero-session:busy:swarm,unsaved"),
            Activity { waiting: false, agent: true, shell: false }
        );
        assert_eq!(
            activity("gaviero-session:busy:terminal"),
            Activity { waiting: false, agent: false, shell: true }
        );
        // A permission prompt or question while the agent runs.
        assert_eq!(
            activity("gaviero-session:attention:agent"),
            Activity { waiting: true, agent: true, shell: false }
        );
        // A pending turn review after the turn.
        assert_eq!(
            activity("gaviero-session:attention"),
            Activity { waiting: true, agent: false, shell: false }
        );
        // An older child: busy, no detail.
        assert_eq!(
            activity("gaviero-session:busy"),
            Activity { waiting: false, agent: true, shell: false }
        );
        // Unsaved files alone are no activity.
        assert_eq!(activity("gaviero-session:idle:unsaved"), Activity::default());
    }

    #[test]
    fn work_that_ends_is_reported_finished() {
        let both = Activity { waiting: false, agent: true, shell: true };
        let none = Activity::default();
        assert_eq!(finished(both, none), (true, true));
        assert_eq!(finished(none, both), (false, false));
        assert_eq!(finished(both, both), (false, false));
        let shell_only = Activity { agent: false, ..both };
        assert_eq!(finished(both, shell_only), (true, false));
        // The turn ends into a turn review: the agent finished.
        let review = Activity { waiting: true, agent: false, shell: false };
        assert_eq!(finished(both, review), (true, true));
    }

    #[test]
    fn work_in_progress_parses_from_the_title() {
        assert_eq!(
            parse_work(Some("gaviero-session:busy:agent,terminal")),
            vec!["an agent is running", "a shell is running a command"]
        );
        assert_eq!(
            parse_work(Some("gaviero-session:idle:unsaved")),
            vec!["files are not saved"]
        );
        assert_eq!(
            parse_work(Some("gaviero-session:attention:swarm")),
            vec!["a swarm is running"]
        );
        assert!(parse_work(Some("gaviero-session:idle")).is_empty());
        // An older child: busy, no detail.
        assert_eq!(
            parse_work(Some("gaviero-session:busy")),
            vec!["a task is running"]
        );
        assert!(parse_work(Some("vim")).is_empty());
        assert!(parse_work(None).is_empty());
    }

    #[test]
    fn labels_use_the_folder_name_and_workspace_stem() {
        let folder = Path::new("C:\\work").join("alpha");
        assert_eq!(label_for(&folder, &[]), "alpha");
        let ws = Path::new("C:\\work").join("team.gaviero-workspace");
        assert_eq!(label_for(&ws, &[]), "team");
    }

    #[test]
    fn duplicate_labels_are_numbered() {
        let folder = Path::new("C:\\a").join("alpha");
        let taken = vec!["alpha".to_string()];
        assert_eq!(label_for(&folder, &taken), "alpha (2)");
        let taken = vec!["alpha".to_string(), "alpha (2)".to_string()];
        assert_eq!(label_for(&folder, &taken), "alpha (3)");
    }
}
