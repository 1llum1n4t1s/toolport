//! Fixed-field, best-effort timing for low-frequency connection work.
use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static TRACE: OnceLock<u128> = OnceLock::new();
static NEXT_SPAN: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    AdapterStart,
    PrivateStart,
    Discovery,
    ElectionLock,
    DescriptorLock,
    Spawn,
    Readiness,
    LifetimeOpen,
    TcpConnect,
    FirstInput,
    InitializeHeaders,
    InitializeResponse,
}
impl Stage {
    fn label(self) -> &'static str {
        match self {
            Self::AdapterStart => "adapter_start",
            Self::PrivateStart => "private_start",
            Self::Discovery => "discovery",
            Self::ElectionLock => "election_lock",
            Self::DescriptorLock => "descriptor_lock",
            Self::Spawn => "spawn",
            Self::Readiness => "readiness",
            Self::LifetimeOpen => "lifetime_open",
            Self::TcpConnect => "tcp_connect",
            Self::FirstInput => "first_input",
            Self::InitializeHeaders => "initialize_headers",
            Self::InitializeResponse => "initialize_response",
        }
    }
}

#[derive(Clone, Copy)]
enum Outcome {
    Pending,
    Ok,
    Error,
    Aborted,
}
impl Outcome {
    fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Aborted => "aborted",
        }
    }
}

/// Callers cannot supply text. Persistence retains gatewaylog's best-effort policy.
pub(crate) struct Span {
    stage: Stage,
    id: u64,
    started: Instant,
    finished: bool,
}
impl Span {
    pub(crate) fn start(stage: Stage) -> Self {
        let span = Self {
            stage,
            id: NEXT_SPAN.fetch_add(1, Ordering::Relaxed),
            started: Instant::now(),
            finished: false,
        };
        span.emit(Outcome::Pending);
        span
    }
    pub(crate) fn finish<T, E>(self, result: Result<T, E>) -> Result<T, E> {
        self.end(result.is_ok());
        result
    }
    pub(crate) fn end(mut self, success: bool) {
        self.emit(if success { Outcome::Ok } else { Outcome::Error });
        self.finished = true;
    }
    fn emit(&self, outcome: Outcome) {
        let trace = *TRACE.get_or_init(|| {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            (nanos << 32) | u128::from(std::process::id())
        });
        let line = format_line(
            trace,
            self.id,
            self.stage,
            outcome,
            self.started.elapsed().as_millis(),
            std::process::id(),
        );
        #[cfg(test)]
        if let Some(dir) = crate::registry::test_data_dir_override() {
            crate::gatewaylog::queue_at(&dir.join("gateway.log"), &line);
        }
        #[cfg(not(test))]
        crate::gatewaylog::append(&line);
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if !self.finished {
            self.emit(Outcome::Aborted);
        }
    }
}
fn format_line(
    trace: u128,
    span: u64,
    stage: Stage,
    outcome: Outcome,
    elapsed_ms: u128,
    pid: u32,
) -> String {
    let event = if matches!(outcome, Outcome::Pending) {
        "start"
    } else {
        "end"
    };
    format!("startup: trace={trace} span={span} stage={} event={event} result={} elapsed_ms={elapsed_ms} pid={pid}", stage.label(), outcome.label())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_without_a_test_directory_does_not_resolve_one() {
        let _lock = crate::registry::data_dir_test_lock();
        assert!(crate::registry::test_data_dir_override().is_none());
        let result: Result<(), ()> = Span::start(Stage::Discovery).finish(Err(()));
        assert!(result.is_err());
        drop(Span::start(Stage::FirstInput));
    }

    #[test]
    fn result_and_drop_emit_one_terminal_record() {
        let _lock = crate::registry::data_dir_test_lock();
        let dir = std::env::temp_dir().join(format!(
            "toolport-startup-trace-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let data = crate::registry::DataDirOverride::set(&dir);
        let mut spans = Vec::new();
        for success in [true, false] {
            let span = Span::start(Stage::InitializeResponse);
            spans.push((span.id, if success { "ok" } else { "error" }));
            assert_eq!(
                span.finish(if success { Ok(()) } else { Err(()) }).is_ok(),
                success
            );
        }
        let span = Span::start(Stage::FirstInput);
        spans.push((span.id, "aborted"));
        drop(span);
        assert!(crate::telemetry::flush_for_test(
            std::time::Duration::from_secs(5)
        ));
        let log = std::fs::read_to_string(dir.join("gateway.log")).unwrap();
        for (id, result) in spans {
            let marker = format!(" span={id} ");
            let records: Vec<_> = log.lines().filter(|line| line.contains(&marker)).collect();
            assert_eq!(records.len(), 2);
            assert!(records[0].contains("event=start result=pending"));
            assert!(records[1].contains(&format!("event=end result={result}")));
        }
        drop(data);
        assert!(crate::telemetry::retire_dir_for_test(
            &dir,
            std::time::Duration::from_secs(5)
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn fixed_schema_covers_all_outcomes_without_caller_text() {
        for (outcome, label, event) in [
            (Outcome::Pending, "pending", "start"),
            (Outcome::Ok, "ok", "end"),
            (Outcome::Error, "error", "end"),
            (Outcome::Aborted, "aborted", "end"),
        ] {
            assert_eq!(format_line(1, 2, Stage::InitializeResponse, outcome, 3, 4), format!("startup: trace=1 span=2 stage=initialize_response event={event} result={label} elapsed_ms=3 pid=4"));
        }
    }
}
