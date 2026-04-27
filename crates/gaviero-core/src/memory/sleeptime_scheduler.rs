//! Tier B / B5 sleeptime auto-trigger.
//!
//! Closes the gap diagnosed in the tier review: the `SleeptimeConfig`
//! fields `min_idle_minutes` / `weekly_force_run` /
//! `first_run_require_confirm` were defined but read by no scheduler.
//! This module spawns a long-lived task that watches:
//!
//! - `last_sleeptime_at` (stamped into `_gaviero_meta` by the writer
//!   task on every successful live run), and
//! - the writer's queue depth (a cheap busy/idle proxy — when the
//!   queue is empty for `min_idle_minutes` we treat the system as
//!   idle without a separate activity stream).
//!
//! Trigger policy:
//!
//! 1. If `last_sleeptime_at` is `None` and `first_run_require_confirm`
//!    is `true`, the scheduler stays idle until the user has run an
//!    explicit `--sleep` (which stamps the timestamp). This honours
//!    the plan's "first-run protection" without an interactive prompt.
//! 2. Otherwise: if `now - last_sleeptime_at >= 24h` AND the writer
//!    queue has been empty for `min_idle_minutes`, enqueue
//!    `WriterMessage::Sleeptime`.
//! 3. Independently, if `weekly_force_run` is `true` and
//!    `now - last_sleeptime_at >= 7 days`, enqueue regardless of idle
//!    state. The plan's "weekly force" — guarantees forward progress
//!    on systems where the queue is rarely empty.
//!
//! The scheduler never blocks; everything goes through the writer
//! task. Failures (writer task gone, meta read errors) log at
//! `warn!` and the scheduler continues running.
//!
//! Lifecycle: this module is a long-running process feature. Spawned
//! by the TUI bootstrap; the CLI does not spawn it (single-shot
//! subcommands never reach the idle threshold).

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::sleeptime::SleeptimeConfig;
use super::stores::MemoryStores;
use super::writer::{WriterHandle, WriterMessage};

const META_LAST_SLEEPTIME_AT: &str = "last_sleeptime_at";
const WEEKLY_FORCE_THRESHOLD_HOURS: i64 = 24 * 7;
const ROUTINE_THRESHOLD_HOURS: i64 = 24;

/// Sleeptime scheduler handle. Drop the handle to stop the task — the
/// scheduler observes a `Stop` signal sent via the bundled control
/// channel (or, equivalently, the receiver being closed when the
/// `SleeptimeScheduler` is dropped).
pub struct SleeptimeScheduler {
    join: JoinHandle<()>,
    stop_tx: mpsc::UnboundedSender<()>,
}

impl SleeptimeScheduler {
    /// Spawn the scheduler against the workspace store.
    ///
    /// `tick` is the polling cadence — exposed for tests; production
    /// callers should pass `Duration::from_secs(60)`. The scheduler
    /// otherwise owns its sleep loop.
    pub fn spawn(
        stores: Arc<MemoryStores>,
        writer: WriterHandle,
        cfg: SleeptimeConfig,
        tick: Duration,
    ) -> Self {
        let (stop_tx, stop_rx) = mpsc::unbounded_channel();
        let join = tokio::spawn(scheduler_loop(stores, writer, cfg, tick, stop_rx));
        Self { join, stop_tx }
    }

    /// Cooperative shutdown. Returns once the scheduler task has
    /// observed the stop signal and exited.
    pub async fn shutdown(self) {
        let _ = self.stop_tx.send(());
        let _ = self.join.await;
    }
}

async fn scheduler_loop(
    stores: Arc<MemoryStores>,
    writer: WriterHandle,
    cfg: SleeptimeConfig,
    tick: Duration,
    mut stop_rx: mpsc::UnboundedReceiver<()>,
) {
    if !cfg.enabled {
        tracing::info!(target: "memory_sleeptime_scheduler", "scheduler disabled by config — exiting");
        return;
    }

    // Track the last time we observed a non-empty queue. The
    // scheduler treats `now - last_busy_at >= min_idle_minutes` as
    // "idle." Initialised to `now` so a freshly-spawned scheduler
    // doesn't fire immediately.
    let mut last_busy_at = Utc::now();

    loop {
        tokio::select! {
            _ = stop_rx.recv() => {
                tracing::info!(target: "memory_sleeptime_scheduler", "stop signal received");
                return;
            }
            _ = tokio::time::sleep(tick) => {}
        }

        if writer.queue_depth() > 0 {
            last_busy_at = Utc::now();
            continue;
        }
        if !writer.is_alive() {
            tracing::warn!(target: "memory_sleeptime_scheduler", "writer task gone — exiting");
            return;
        }

        let last_sleeptime_at = read_last_sleeptime_at(&stores).await;
        let decision = decide_trigger(
            &cfg,
            last_sleeptime_at,
            last_busy_at,
            Utc::now(),
        );
        match decision {
            Decision::Skip { reason } => {
                tracing::trace!(target: "memory_sleeptime_scheduler", reason, "skip tick");
            }
            Decision::Trigger { kind } => {
                tracing::info!(
                    target: "memory_sleeptime_scheduler",
                    trigger = kind.as_str(),
                    "enqueueing Sleeptime"
                );
                if let Err(e) = writer.enqueue(WriterMessage::Sleeptime {
                    payload: json!({}),
                }) {
                    tracing::warn!(
                        target: "memory_sleeptime_scheduler",
                        error = %e,
                        "Sleeptime enqueue failed (writer terminated?)"
                    );
                    return;
                }
                // Optimistically advance so we don't enqueue again on
                // the very next tick before the writer drains. The
                // writer stamps the authoritative timestamp on
                // success, which we'll re-read next tick.
                last_busy_at = Utc::now();
            }
        }
    }
}

async fn read_last_sleeptime_at(stores: &MemoryStores) -> Option<DateTime<Utc>> {
    let store = stores.workspace().clone();
    match store.get_meta_value(META_LAST_SLEEPTIME_AT).await {
        Ok(Some(s)) => DateTime::parse_from_rfc3339(&s)
            .ok()
            .map(|d| d.with_timezone(&Utc)),
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(
                target: "memory_sleeptime_scheduler",
                error = %e,
                "failed to read last_sleeptime_at"
            );
            None
        }
    }
}

#[derive(Debug, PartialEq)]
enum Decision {
    Skip { reason: &'static str },
    Trigger { kind: TriggerKind },
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum TriggerKind {
    Routine,
    WeeklyForce,
}

impl TriggerKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Routine => "routine",
            Self::WeeklyForce => "weekly_force",
        }
    }
}

/// Pure decision function — extracted for unit testability so we can
/// drive the scheduler logic deterministically without sleeping.
fn decide_trigger(
    cfg: &SleeptimeConfig,
    last_sleeptime_at: Option<DateTime<Utc>>,
    last_busy_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Decision {
    if let Some(ts) = last_sleeptime_at {
        let hours = (now - ts).num_hours();
        if cfg.weekly_force_run && hours >= WEEKLY_FORCE_THRESHOLD_HOURS {
            return Decision::Trigger {
                kind: TriggerKind::WeeklyForce,
            };
        }
        if hours < ROUTINE_THRESHOLD_HOURS {
            return Decision::Skip {
                reason: "within 24h of last sleeptime",
            };
        }
        let idle_min = (now - last_busy_at).num_minutes() as usize;
        if idle_min < cfg.min_idle_minutes {
            return Decision::Skip {
                reason: "system not idle long enough",
            };
        }
        Decision::Trigger {
            kind: TriggerKind::Routine,
        }
    } else if cfg.first_run_require_confirm {
        Decision::Skip {
            reason: "first run pending user `--sleep` confirmation",
        }
    } else {
        // No prior sleeptime, no first-run gate: treat as routine
        // once we've observed enough idle time.
        let idle_min = (now - last_busy_at).num_minutes() as usize;
        if idle_min < cfg.min_idle_minutes {
            return Decision::Skip {
                reason: "system not idle long enough (first run)",
            };
        }
        Decision::Trigger {
            kind: TriggerKind::Routine,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(min_idle: usize, weekly: bool, first_run_confirm: bool) -> SleeptimeConfig {
        SleeptimeConfig {
            min_idle_minutes: min_idle,
            weekly_force_run: weekly,
            first_run_require_confirm: first_run_confirm,
            ..Default::default()
        }
    }

    fn iso(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn first_run_with_confirm_required_skips() {
        let now = iso("2026-04-26T10:00:00Z");
        let busy = iso("2026-04-26T09:00:00Z"); // idle 60min
        let d = decide_trigger(&cfg(10, true, true), None, busy, now);
        assert!(matches!(d, Decision::Skip { .. }));
    }

    #[test]
    fn first_run_without_confirm_triggers_when_idle_long_enough() {
        let now = iso("2026-04-26T10:00:00Z");
        let busy = iso("2026-04-26T09:00:00Z"); // idle 60min
        let d = decide_trigger(&cfg(10, true, false), None, busy, now);
        assert_eq!(
            d,
            Decision::Trigger {
                kind: TriggerKind::Routine
            }
        );
    }

    #[test]
    fn within_24h_of_last_run_is_skipped_even_if_idle() {
        let last = iso("2026-04-26T05:00:00Z"); // 5h ago
        let now = iso("2026-04-26T10:00:00Z");
        let busy = iso("2026-04-26T09:00:00Z"); // idle 60min
        let d = decide_trigger(&cfg(10, true, true), Some(last), busy, now);
        assert!(matches!(d, Decision::Skip { .. }));
    }

    #[test]
    fn over_24h_and_idle_triggers_routine() {
        let last = iso("2026-04-25T08:00:00Z"); // 26h ago
        let now = iso("2026-04-26T10:00:00Z");
        let busy = iso("2026-04-26T09:30:00Z"); // idle 30min
        let d = decide_trigger(&cfg(10, true, true), Some(last), busy, now);
        assert_eq!(
            d,
            Decision::Trigger {
                kind: TriggerKind::Routine
            }
        );
    }

    #[test]
    fn over_24h_but_busy_skips() {
        let last = iso("2026-04-25T08:00:00Z"); // 26h ago
        let now = iso("2026-04-26T10:00:00Z");
        let busy = iso("2026-04-26T09:55:00Z"); // idle 5min
        let d = decide_trigger(&cfg(10, true, true), Some(last), busy, now);
        assert!(matches!(d, Decision::Skip { .. }));
    }

    #[test]
    fn over_7d_triggers_weekly_force_even_when_busy() {
        // Weekly force ignores idle gating — simulates a system that
        // never quiets down for 10 minutes.
        let last = iso("2026-04-19T10:00:00Z"); // 7d ago
        let now = iso("2026-04-26T10:00:00Z");
        let busy = iso("2026-04-26T09:59:00Z"); // idle 1min
        let d = decide_trigger(&cfg(10, true, true), Some(last), busy, now);
        assert_eq!(
            d,
            Decision::Trigger {
                kind: TriggerKind::WeeklyForce
            }
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scheduler_does_not_enqueue_when_first_run_confirm_required() {
        // End-to-end: fresh in-memory stores (no `last_sleeptime_at`),
        // scheduler with `first_run_require_confirm = true` should
        // never enqueue. We run for ~5 ticks of wall time and verify
        // the writer queue stayed at its initial depth.
        let services = super::super::MemoryServices::for_tests_in_memory().unwrap();
        let depth_before = services.writer.queue_depth();
        let scheduler = SleeptimeScheduler::spawn(
            services.stores.clone(),
            services.writer.clone(),
            cfg(0, true, true),
            Duration::from_millis(5),
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
        scheduler.shutdown().await;
        assert_eq!(services.writer.queue_depth(), depth_before);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scheduler_disabled_config_exits_immediately() {
        let services = super::super::MemoryServices::for_tests_in_memory().unwrap();
        let mut config = cfg(0, true, false);
        config.enabled = false;
        let scheduler = SleeptimeScheduler::spawn(
            services.stores.clone(),
            services.writer.clone(),
            config,
            Duration::from_millis(5),
        );
        // The task should observe `enabled == false` and return at
        // the top of the loop. `shutdown` therefore completes
        // promptly without needing the stop signal.
        tokio::time::timeout(Duration::from_millis(200), scheduler.shutdown())
            .await
            .expect("disabled scheduler should exit promptly");
    }

    #[test]
    fn weekly_force_disabled_falls_back_to_routine_gating() {
        let last = iso("2026-04-19T10:00:00Z"); // 7d ago — would force-trigger
        let now = iso("2026-04-26T10:00:00Z");
        let busy = iso("2026-04-26T09:59:00Z"); // idle 1min
        let d = decide_trigger(&cfg(10, /* weekly */ false, true), Some(last), busy, now);
        // > 24h but not idle long enough, weekly_force off → skip.
        assert!(matches!(d, Decision::Skip { .. }));
    }
}
