//! The run log: what a job's handler logs with `tracing` while it runs is kept with the run, by attempt, and shown
//! with it. Lines are cut at 2 KiB; a process sends at most 1,000 lines and 1 MiB a second, and drops the rest.
//! Lines logged while handling a request are not kept this way.
//!
//! `serve()` installs this. A program that sets up its own `tracing` subscriber and calls `serve_async` adds
//! [`run_log_layer`] to it.

use std::fmt::Write as _;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use celobus_proto as pb;
use pb::plugin_message;
use tokio::sync::mpsc;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

/// The longest line kept.
pub const MAX_LINE: usize = 2048;
/// Lines and bytes a process sends a second.
pub const LINES_PER_SECOND: u32 = 1_000;
pub const BYTES_PER_SECOND: usize = 1 << 20;

/// Where a run's lines go: the run's request id and the session.
#[derive(Clone)]
pub(crate) struct RunLog {
    pub(crate) request_id: u64,
    pub(crate) tx: mpsc::Sender<pb::PluginMessage>,
}

tokio::task_local! {
    pub(crate) static RUN: RunLog;
}

/// A fixed one-second window of lines and bytes.
pub(crate) struct Limiter {
    window: Option<Instant>,
    lines: u32,
    bytes: usize,
}

impl Limiter {
    pub(crate) const fn new() -> Self {
        Self {
            window: None,
            lines: 0,
            bytes: 0,
        }
    }

    /// Whether a line of `bytes` may go at `now`.
    pub(crate) fn admit(&mut self, now: Instant, bytes: usize) -> bool {
        match self.window {
            Some(start) if now.duration_since(start) < Duration::from_secs(1) => {}
            _ => {
                self.window = Some(now);
                self.lines = 0;
                self.bytes = 0;
            }
        }
        if self.lines >= LINES_PER_SECOND || self.bytes + bytes > BYTES_PER_SECOND {
            return false;
        }
        self.lines += 1;
        self.bytes += bytes;
        true
    }
}

static LIMIT: Mutex<Limiter> = Mutex::new(Limiter::new());

/// The `tracing` layer that sends a job run's log lines with the run.
pub struct RunLogLayer {
    _private: (),
}

/// The layer to add to a program's own `tracing` subscriber so that its jobs' runs keep their logs.
pub fn run_log_layer() -> RunLogLayer {
    RunLogLayer { _private: () }
}

impl<S: Subscriber> Layer<S> for RunLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let _ = RUN.try_with(|run| {
            let msg = line(event);
            let admitted = LIMIT
                .lock()
                .map(|mut l| l.admit(Instant::now(), msg.len()))
                .unwrap_or(false);
            if admitted {
                let _ = run
                    .tx
                    .try_send(plugin_log(run.request_id, *event.metadata().level(), msg));
            }
        });
    }
}

pub(crate) fn plugin_log(request_id: u64, level: Level, message: String) -> pb::PluginMessage {
    use pb::plugin_log::Level as L;
    let level = match level {
        Level::ERROR => L::Error,
        Level::WARN => L::Warn,
        Level::INFO => L::Info,
        _ => L::Debug,
    };
    pb::PluginMessage {
        kind: Some(plugin_message::Kind::PluginLog(pb::PluginLog {
            request_id,
            level: level as i32,
            message,
            at_unix_ms: chrono::Utc::now().timestamp_millis(),
        })),
    }
}

/// The event's message, then its other fields as `key=value`, cut at [`MAX_LINE`].
fn line(event: &Event<'_>) -> String {
    let mut v = Line::default();
    event.record(&mut v);
    let mut out = v.message;
    if !v.fields.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&v.fields);
    }
    crate::jobs::cut(&out, MAX_LINE)
}

#[derive(Default)]
struct Line {
    message: String,
    fields: String,
}

impl Line {
    fn field(&mut self, name: &str, value: std::fmt::Arguments<'_>) {
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        let _ = write!(self.fields, "{name}={value}");
    }
}

impl Visit for Line {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.field(field.name(), format_args!("{value:?}"));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.field(field.name(), format_args!("{value:?}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt as _;

    #[test]
    fn the_limit_is_per_second() {
        let mut l = Limiter::new();
        let t = Instant::now();
        for _ in 0..LINES_PER_SECOND {
            assert!(l.admit(t, 10));
        }
        assert!(!l.admit(t + Duration::from_millis(999), 10));
        assert!(l.admit(t + Duration::from_secs(1), 10));
        let mut l = Limiter::new();
        assert!(l.admit(t, BYTES_PER_SECOND - 1));
        assert!(!l.admit(t, 2));
        assert!(l.admit(t, 1));
    }

    fn collect(request_id: Option<u64>, log: impl FnOnce()) -> Vec<(u64, i32, String)> {
        let (tx, mut rx) = mpsc::channel(16);
        let subscriber = tracing_subscriber::registry().with(run_log_layer());
        tracing::subscriber::with_default(subscriber, || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            rt.block_on(async {
                match request_id {
                    Some(request_id) => RUN.scope(RunLog { request_id, tx }, async { log() }).await,
                    None => log(),
                }
            });
        });
        let mut out = vec![];
        while let Ok(m) = rx.try_recv() {
            if let Some(plugin_message::Kind::PluginLog(l)) = m.kind {
                out.push((l.request_id, l.level, l.message));
            }
        }
        out
    }

    #[test]
    fn lines_of_a_run_are_sent_with_its_request_id() {
        let lines = collect(Some(9), || {
            tracing::info!(order = "SO-1", n = 3, "reminded");
            tracing::error!("no mail");
            tracing::trace!("detail");
        });
        assert_eq!(
            lines,
            vec![
                (
                    9,
                    pb::plugin_log::Level::Info as i32,
                    "reminded order=\"SO-1\" n=3".into()
                ),
                (9, pb::plugin_log::Level::Error as i32, "no mail".into()),
                (9, pb::plugin_log::Level::Debug as i32, "detail".into()),
            ]
        );
    }

    #[test]
    fn lines_outside_a_run_are_not_sent() {
        assert!(collect(None, || tracing::info!("a request's line")).is_empty());
    }

    #[test]
    fn long_lines_are_cut_at_2_kib() {
        let long = "é".repeat(2000);
        let lines = collect(Some(1), || tracing::warn!("{long}"));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].2.len() <= MAX_LINE && lines[0].2.len() >= MAX_LINE - 1);
        assert!(lines[0].2.chars().all(|c| c == 'é'));
    }
}
