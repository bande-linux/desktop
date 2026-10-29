use std::io::{self, IsTerminal};

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

const DEFAULT_LEVEL: &str = "info";

pub fn init(identifier: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_LEVEL));
    let registry = tracing_subscriber::registry().with(filter);
    let journal = stderr_is_journal()
        .then(tracing_journald::layer)
        .and_then(Result::ok);
    let _ = match journal {
        Some(journal) => registry
            .with(journal.with_syslog_identifier(identifier.to_owned()))
            .try_init(),
        None => registry
            .with(
                fmt::layer()
                    .with_writer(io::stderr)
                    .with_ansi(io::stderr().is_terminal()),
            )
            .try_init(),
    };
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

#[cfg(test)]
mod tests {
    use super::is_journal_stream;

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
