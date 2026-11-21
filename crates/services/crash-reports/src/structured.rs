use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use bande_reporter::config::Config;
use bande_reporter::diagnose::previous_boot_tail;
use bande_reporter::elfcore::read_core;
use bande_reporter::facts;
use bande_reporter::fault::{Fault, push_text};
use bande_reporter::scrub::Scrubber;
use bande_reporter::spool::{Attachment, Spool};
use bande_reporter::stacks::parse_coredump;
use jiff::Timestamp;
use serde_json::json;

use crate::journal::{self, Entry};

const SPOOL: &str = "/var/lib/bande/reports";
const CONFIG: &str = "/etc/bande/reports.conf";
const HANG_KINDS: [&str; 4] = ["app-hang", "force-quit", "compositor-stall", "shell-stall"];
const HANG_MERGE_SECONDS: i64 = 40;
const PREVIOUS_BOOT: &str = "-1";
const QUIT_SIGNAL: &str = "SIGQUIT";
const PANIC_PREFIX: &str = "panicked at ";

fn config() -> Config {
    Config::load(Path::new(CONFIG)).unwrap_or_default()
}

fn panic_location(text: &str) -> Option<String> {
    let rest = text.split(PANIC_PREFIX).nth(1)?;
    let end = rest.find([' ', '\n']).unwrap_or(rest.len());
    let location = rest[..end].trim_end_matches(':');
    (location.starts_with("crates/") || location.contains(".rs:")).then(|| location.to_owned())
}

fn hang_already_reported(spool: &Spool, program: &str, time: Timestamp) -> bool {
    spool.all().iter().any(|report| {
        let field = |name| {
            report
                .manifest
                .get(name)
                .and_then(serde_json::Value::as_str)
        };
        field("program") == Some(program)
            && field("kind").is_some_and(|kind| HANG_KINDS.contains(&kind))
            && field("time")
                .and_then(|time| time.parse::<Timestamp>().ok())
                .is_some_and(|then| {
                    (time.as_second() - then.as_second()).abs() < HANG_MERGE_SECONDS
                })
    })
}

fn crash(entry: &Entry, spool: &Spool, config: &Config) -> Result<(), Box<dyn Error>> {
    let program = entry
        .field("COREDUMP_EXE")
        .as_deref()
        .and_then(|path| path.rsplit('/').next())
        .map(str::to_owned)
        .or_else(|| entry.field("COREDUMP_COMM"))
        .unwrap_or_default();
    let pid = entry.field("COREDUMP_PID").unwrap_or_default();
    let signal = entry.field("COREDUMP_SIGNAL_NAME").unwrap_or_default();
    let time = entry
        .timestamp("COREDUMP_TIMESTAMP")
        .unwrap_or_else(|| entry.time());
    if signal == QUIT_SIGNAL && hang_already_reported(spool, &program, time) {
        return Ok(());
    }
    let mut fault = Fault::new(if signal == QUIT_SIGNAL {
        "not-responding"
    } else {
        "crash"
    });
    fault.time = time;
    fault.owner = entry.number("COREDUMP_UID").filter(|uid| *uid != 0);
    fault.pid = Some(pid.clone());
    fault.program = Some(program);
    fault.executable = entry.field("COREDUMP_EXE").map(PathBuf::from);
    fault.detail.insert("signal".to_owned(), json!(signal));
    let unit = entry
        .field("COREDUMP_USER_UNIT")
        .map(|unit| ("USER_UNIT", unit))
        .or_else(|| entry.field("COREDUMP_UNIT").map(|unit| ("UNIT", unit)));
    let termination = if signal == QUIT_SIGNAL {
        Some("force-quit")
    } else {
        unit.as_ref()
            .filter(|(field, unit)| journal::killed_by_watchdog(field, unit, time))
            .map(|_| "watchdog")
    };
    fault
        .detail
        .insert("termination".to_owned(), json!(termination));
    fault
        .detail
        .insert("unit".to_owned(), json!(unit.map(|(_, unit)| unit)));
    if let Some(panic) = journal::panic_of(&pid, time) {
        fault.panic_location = panic_location(&panic);
        fault.detail.insert("panic".to_owned(), json!(panic));
    }
    let message = entry.field("MESSAGE").unwrap_or_default();
    let mut crashed_tid = None;
    let mut mapped_files = Vec::new();
    if let Ok(mut dump) = Command::new("coredumpctl")
        .args(["dump", "--quiet", &pid])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
    {
        if let Some(core) = dump.stdout.take() {
            match read_core(core, config.stack_memory) {
                Ok((facts, slice)) => {
                    let sent_by_another = facts.signal_code.is_some_and(|code| code <= 0);
                    crashed_tid = if sent_by_another {
                        pid.parse().ok()
                    } else {
                        facts.crashed_tid
                    };
                    mapped_files.clone_from(&facts.mapped_files);
                    fault
                        .detail
                        .insert("registers".to_owned(), facts.registers_json());
                    fault
                        .detail
                        .insert("signal_code".to_owned(), json!(facts.signal_code));
                    fault.detail.insert(
                        "fault_address".to_owned(),
                        json!(facts.fault_address.map(|a| format!("{a:#x}"))),
                    );
                    if let Some(bytes) = slice {
                        fault.attachments.push(Attachment {
                            name: "stack-memory.core".to_owned(),
                            bytes,
                        });
                    }
                }
                Err(error) => tracing::warn!("cannot read the core dump: {error}"),
            }
        }
        if let Err(error) = dump.kill() {
            tracing::debug!("cannot stop coredumpctl: {error}");
        }
        if let Err(error) = dump.wait() {
            tracing::debug!("cannot wait for coredumpctl: {error}");
        }
    }
    fault.stacks = parse_coredump(&message, crashed_tid);
    fault.stacks.add_mapped_files(&mapped_files);
    fault.signature_text = Some(signal);
    fault.store(spool, config, Path::new(SPOOL))?;
    Ok(())
}

fn undumpable(entry: &Entry, spool: &Spool, config: &Config) -> Result<(), Box<dyn Error>> {
    let message = entry.field("MESSAGE").unwrap_or_default();
    let mut fault = Fault::new("panic");
    fault.time = entry.time();
    fault.owner = entry.number("_UID").filter(|uid| *uid != 0);
    fault.program = entry.field("SYSLOG_IDENTIFIER");
    fault.panic_location = panic_location(&message);
    fault.detail.insert("panic".to_owned(), json!(message));
    fault.store(spool, config, Path::new(SPOOL))?;
    Ok(())
}

fn failed_unit(entry: &Entry, spool: &Spool, config: &Config) -> Result<(), Box<dyn Error>> {
    let unit = entry.field("USER_UNIT").or_else(|| entry.field("UNIT"));
    let result = entry.field("UNIT_RESULT");
    let mut fault = Fault::new("failed-unit");
    fault.time = entry.time();
    fault.owner = entry
        .field("USER_UNIT")
        .and_then(|_| entry.number("_UID"))
        .filter(|uid| *uid != 0);
    fault.signature_text = Some(format!("{unit:?} {result:?}"));
    let termination = match result.as_deref() {
        Some("watchdog") => Some("watchdog"),
        Some("timeout") => Some("stop-timeout"),
        Some("oom-kill") => {
            facts::add_memory_details(&mut fault.detail);
            match unit
                .as_deref()
                .and_then(|unit| facts::oomd_reason(unit, fault.time.as_second()))
            {
                Some(reason) => {
                    fault.detail.insert("limit".to_owned(), json!(reason));
                    Some("oomd")
                }
                None => Some("kernel-oom"),
            }
        }
        _ => None,
    };
    fault
        .detail
        .insert("termination".to_owned(), json!(termination));
    fault.program = unit;
    fault.detail.insert("result".to_owned(), json!(result));
    fault.store(spool, config, Path::new(SPOOL))?;
    Ok(())
}

pub fn queue_checkin_if_due() {
    match Spool::open(Path::new(SPOOL)) {
        Ok(spool) => bande_reporter::checkin::queue_if_due(&spool, &config()),
        Err(error) => tracing::warn!("cannot open the report spool: {error}"),
    }
}

pub fn of_entry(entry: &Entry) -> Result<(), Box<dyn Error>> {
    let spool = Spool::open(Path::new(SPOOL))?;
    let config = config();
    if entry.field("MESSAGE_ID").as_deref() == Some(journal::COREDUMP_MESSAGE) {
        crash(entry, &spool, &config)
    } else if entry.field("TARGET").as_deref() == Some(journal::PANIC_TARGET) {
        undumpable(entry, &spool, &config)
    } else {
        failed_unit(entry, &spool, &config)
    }
}

pub fn of_previous_boot(saved_kernel_log: &str, time: Timestamp) -> Result<(), Box<dyn Error>> {
    let spool = Spool::open(Path::new(SPOOL))?;
    let scrubber = Scrubber::for_this_computer();
    let panicked = !saved_kernel_log.trim().is_empty();
    let mut fault = Fault::new(if panicked {
        "kernel-panic"
    } else {
        "unexpected-restart"
    });
    fault.time = time;
    fault.program = Some("kernel".to_owned());
    fault.boot = Some(PREVIOUS_BOOT.to_owned());
    fault.signature_text = saved_kernel_log
        .lines()
        .find(|line| line.contains("Kernel panic"))
        .map(|line| {
            line.split_once("Kernel panic")
                .map_or(line, |(_, rest)| rest)
                .split_whitespace()
                .filter(|word| !word.bytes().any(|byte| byte.is_ascii_digit()))
                .collect::<Vec<_>>()
                .join(" ")
        });
    push_text(
        &mut fault.attachments,
        "pstore.txt",
        scrubber.text(saved_kernel_log),
    );
    previous_boot_tail(&mut fault, &scrubber, Path::new(SPOOL));
    fault.store(&spool, &config(), Path::new(SPOOL))?;
    Ok(())
}
