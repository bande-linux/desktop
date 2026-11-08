pub mod fault;
mod ring;

use std::io;
use std::path::Path;

use tracing_subscriber::filter::{FilterFn, LevelFilter, Targets};
use tracing_subscriber::fmt;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

const FILTER_VARIABLE: &str = "RUST_LOG";
const PANIC_TARGET: &str = "panic";
const OWN_TARGETS: &str = "bande";
const HOLDS_PASSWORDS: [&str; 6] = [
    "bande-clipboard",
    "bande-greeter",
    "bande-lock",
    "bande-polkit-agent",
    "bande-secrets",
    "bande-secrets-prompt",
];
const UNPREFIXED_TARGETS: [&str; 2] = ["pam_bande_secrets", "xdg_desktop_portal_bande"];
const DBUS_TARGET: &str = "zbus";

fn name_process(identifier: &str) {
    if let Ok(name) = std::ffi::CString::new(identifier)
        && let Err(error) = rustix::thread::set_name(&name)
    {
        eprintln!("cannot name the process {identifier}: {error}");
    }
}

pub fn init(identifier: &str) {
    init_quieting(identifier, &[]);
}

pub fn init_quieting(identifier: &str, chatty: &[&str]) {
    name_process(identifier);
    let filter = chatty.iter().fold(
        targets(std::env::var(FILTER_VARIABLE).ok().as_deref()),
        |filter, target| filter.with_target(*target, LevelFilter::WARN),
    );
    let in_memory = (!HOLDS_PASSWORDS.contains(&identifier))
        .then(|| ring::Ring::open(identifier).ok())
        .flatten()
        .map(|ring| {
            let filter = chatty.iter().fold(
                Targets::new()
                    .with_default(LevelFilter::INFO)
                    .with_target(OWN_TARGETS, LevelFilter::DEBUG),
                |filter, target| filter.with_target(*target, LevelFilter::WARN),
            );
            ring::RingLayer(ring).with_filter(filter)
        });
    let journal = tracing_journald::layer().ok().map(|journal| {
        journal
            .with_syslog_identifier(identifier.to_owned())
            .with_filter(filter.clone())
    });
    let journaled = journal.is_some();
    let stderr_is_journal = stderr_is_journal();
    let stderr_is_console =
        std::fs::read_link("/proc/self/fd/2").is_ok_and(|path| is_virtual_console(&path));
    let stderr = (!journaled || !(stderr_is_journal || stderr_is_console)).then(|| {
        fmt::layer()
            .with_writer(io::stderr)
            .with_ansi(io::IsTerminal::is_terminal(&io::stderr()))
            .with_filter(FilterFn::new(|metadata| metadata.target() != PANIC_TARGET))
            .with_filter(filter)
    });
    if tracing_subscriber::registry()
        .with(in_memory)
        .with(journal)
        .with(stderr)
        .try_init()
        .is_ok()
        && journaled
    {
        journal_panics(!stderr_is_journal);
    }
}

fn is_virtual_console(path: &Path) -> bool {
    path.to_str()
        .and_then(|path| path.strip_prefix("/dev/tty"))
        .is_some_and(|number| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        })
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
        .unwrap_or_else(quiet_dependencies)
}

fn quiet_dependencies() -> Targets {
    UNPREFIXED_TARGETS.iter().chain([&OWN_TARGETS]).fold(
        Targets::new()
            .with_default(LevelFilter::WARN)
            .with_target(DBUS_TARGET, LevelFilter::ERROR),
        |filter, target| filter.with_target(*target, LevelFilter::INFO),
    )
}
