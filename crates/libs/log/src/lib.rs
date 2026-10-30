use std::io;

use tracing_subscriber::filter::{FilterFn, LevelFilter, Targets};
use tracing_subscriber::fmt;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

const FILTER_VARIABLE: &str = "RUST_LOG";
const PANIC_TARGET: &str = "panic";

pub fn init(identifier: &str) {
    let filter = targets(std::env::var(FILTER_VARIABLE).ok().as_deref());
    let journal = tracing_journald::layer()
        .ok()
        .map(|journal| journal.with_syslog_identifier(identifier.to_owned()));
    let journaled = journal.is_some();
    let stderr_is_journal = stderr_is_journal();
    let stderr = (!journaled || !stderr_is_journal).then(|| {
        fmt::layer()
            .with_writer(io::stderr)
            .with_ansi(io::IsTerminal::is_terminal(&io::stderr()))
            .with_filter(FilterFn::new(|metadata| metadata.target() != PANIC_TARGET))
    });
    if tracing_subscriber::registry()
        .with(filter)
        .with(journal)
        .with(stderr)
        .try_init()
        .is_ok()
        && journaled
    {
        journal_panics(!stderr_is_journal);
    }
}

fn journal_panics(also_report: bool) {
    let report = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        tracing::error!(target: PANIC_TARGET, "{panic}");
        if also_report {
            report(panic);
        }
    }));
}

fn stderr_is_journal() -> bool {
    let Ok(stat) = rustix::fs::fstat(io::stderr()) else {
        return false;
    };
    is_journal_stream(
        std::env::var("JOURNAL_STREAM").ok().as_deref(),
        stat.st_dev,
        stat.st_ino,
    )
}

fn is_journal_stream(journal_stream: Option<&str>, device: u64, inode: u64) -> bool {
    journal_stream
        .and_then(|value| value.split_once(':'))
        .is_some_and(|(stream_device, stream_inode)| {
            stream_device.parse() == Ok(device) && stream_inode.parse() == Ok(inode)
        })
}

fn targets(directives: Option<&str>) -> Targets {
    directives
        .and_then(|directives| directives.parse().ok())
        .unwrap_or_else(|| Targets::new().with_default(LevelFilter::INFO))
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::filter::LevelFilter;

    use super::{is_journal_stream, targets};

    #[test]
    fn rust_log_directives_set_the_level_per_target() {
        let filter = targets(Some("warn,unbound_dock=debug"));
        assert_eq!(filter.default_level(), Some(LevelFilter::WARN));
        assert!(filter.would_enable("unbound_dock::view", &tracing::Level::DEBUG));
        assert!(!filter.would_enable("zbus", &tracing::Level::INFO));
    }

    #[test]
    fn without_valid_directives_info_and_above_is_logged() {
        for directives in [None, Some("=nonsense=")] {
            let filter = targets(directives);
            assert_eq!(filter.default_level(), Some(LevelFilter::INFO));
        }
    }

    #[test]
    fn stderr_matching_the_journal_stream_goes_to_the_journal() {
        assert!(is_journal_stream(Some("8:44711"), 8, 44711));
    }

    #[test]
    fn a_different_stream_or_none_stays_on_stderr() {
        assert!(!is_journal_stream(Some("8:44711"), 8, 44712));
        assert!(!is_journal_stream(Some("9:44711"), 8, 44711));
        assert!(!is_journal_stream(None, 8, 44711));
        assert!(!is_journal_stream(Some("8"), 8, 44711));
        assert!(!is_journal_stream(Some("x:44711"), 8, 44711));
    }
}
