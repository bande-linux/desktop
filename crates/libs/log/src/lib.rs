use std::io::{self, IsTerminal};

use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const FILTER_VARIABLE: &str = "RUST_LOG";

pub fn init(identifier: &str) {
    let filter = targets(std::env::var(FILTER_VARIABLE).ok().as_deref());
    let journal = tracing_journald::layer()
        .ok()
        .map(|journal| journal.with_syslog_identifier(identifier.to_owned()));
    let terminal = io::stderr().is_terminal();
    let stderr = (journal.is_none() || terminal)
        .then(|| fmt::layer().with_writer(io::stderr).with_ansi(terminal));
    if tracing_subscriber::registry()
        .with(filter)
        .with(journal)
        .with(stderr)
        .try_init()
        .is_ok()
    {
        log_panics();
    }
}

fn log_panics() {
    let report = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        tracing::error!("{panic}");
        report(panic);
    }));
}

fn targets(directives: Option<&str>) -> Targets {
    directives
        .and_then(|directives| directives.parse().ok())
        .unwrap_or_else(|| Targets::new().with_default(LevelFilter::INFO))
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::filter::LevelFilter;

    use super::targets;

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
}
