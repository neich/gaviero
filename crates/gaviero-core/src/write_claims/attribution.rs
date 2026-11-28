//! The decision rule: whose change is this?

use std::path::Path;

use super::journal::{Before, Claim, WriterInfo, WriterKind, claim_key};

/// Every writer relevant to a window, read once.
#[derive(Debug, Clone, Default)]
pub struct JournalSnapshot {
    pub writers: Vec<WriterRecord>,
}

#[derive(Debug, Clone)]
pub struct WriterRecord {
    pub info: WriterInfo,
    /// `None` while the writer runs.
    pub effective_end_ns: Option<i64>,
    pub claims: Vec<Claim>,
}

/// How a changed file relates to one writer's window. Label lists are
/// sorted and deduplicated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Only this writer claimed it.
    Own,
    /// This writer and these others claimed it.
    Shared(Vec<String>),
    /// Another writer provably made the change.
    Foreign(String),
    /// Nobody proved it; these agent writers were active over the path.
    Unclaimed(Vec<String>),
}

/// A claim shows the write changed the file: its before-state is known and
/// differs from the after-state.
pub fn proves_change(before: &Before, after: Option<&str>) -> bool {
    match before {
        Before::Unknown => false,
        Before::Absent => after.is_some(),
        Before::Hash(h) => after != Some(h.as_str()),
    }
}

fn sorted_labels<'a>(labels: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = labels.map(str::to_string).collect();
    out.sort();
    out.dedup();
    out
}

impl JournalSnapshot {
    /// Classify the change of `path` (now holding `current`, `None` =
    /// absent) for writer `own_id`, whose window is
    /// `window_start_ns..=until_ns`.
    pub fn classify(
        &self,
        own_id: &str,
        window_start_ns: i64,
        until_ns: i64,
        path: &Path,
        current: Option<&str>,
    ) -> Verdict {
        let key = claim_key(path);
        let in_window =
            |c: &Claim| c.key == key && window_start_ns <= c.at_ns && c.at_ns <= until_ns;
        let claimed = |w: &WriterRecord| w.claims.iter().any(in_window);

        if self
            .writers
            .iter()
            .any(|w| w.info.id == own_id && claimed(w))
        {
            let others = sorted_labels(
                self.writers
                    .iter()
                    .filter(|w| w.info.id != own_id && claimed(w))
                    .map(|w| w.info.label.as_str()),
            );
            return if others.is_empty() {
                Verdict::Own
            } else {
                Verdict::Shared(others)
            };
        }

        let latest = self
            .writers
            .iter()
            .filter(|w| w.info.id != own_id)
            .flat_map(|w| w.claims.iter().filter(|c| in_window(c)).map(move |c| (w, c)))
            .max_by_key(|(_, c)| c.at_ns);
        if let Some((w, c)) = latest
            && c.after.as_deref() == current
            && (w.info.kind == WriterKind::Host
                || w.info.started_ns <= window_start_ns
                || proves_change(&c.before, c.after.as_deref()))
        {
            return Verdict::Foreign(w.info.label.clone());
        }

        Verdict::Unclaimed(sorted_labels(
            self.writers
                .iter()
                .filter(|w| {
                    w.info.id != own_id
                        && w.info.kind != WriterKind::Host
                        && w.info.started_ns <= until_ns
                        && w.effective_end_ns.is_none_or(|e| e >= window_start_ns)
                        && (w
                            .info
                            .roots
                            .iter()
                            .any(|r| key.starts_with(&format!("{}/", claim_key(r))))
                            || claimed(w))
                })
                .map(|w| w.info.label.as_str()),
        ))
    }
}
