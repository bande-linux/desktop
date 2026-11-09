use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bande_reporter::checkin;
use bande_reporter::config::Config;
use bande_reporter::policy::{Choice, Policy};
use bande_reporter::send::{self, Outcome, Sender};
use bande_reporter::spool::{Attachment, Spool, Status};
use bande_reporter::throttle::{self, Ledger};
use bande_reporter::wait::{Waiter, Wakes, Woken};
use serde_json::{Map, Value};

const SPOOL: &str = "/var/lib/bande/reports";
const CONFIG: &str = "/etc/bande/reports.conf";
const FIRST_RETRY: Duration = Duration::from_secs(60);
const LONGEST_RETRY: Duration = Duration::from_secs(6 * 60 * 60);
const USAGE: &str = "usage: bande-reporter [--spool DIR] [--config FILE] (send | serve | watch | resources | list | store MANIFEST [ATTACHMENT...] | queue ID | delete ID)";

struct Paths {
    spool: PathBuf,
    config: PathBuf,
}

enum Pause {
    None,
    Network,
    Server(Duration),
    UntilConfigChanges,
    Interval(Duration),
    Metered,
    Held { next_day: bool, power: bool },
}

fn pause_for(outcome: Outcome, retry: Duration) -> Pause {
    match outcome {
        Outcome::Stored | Outcome::OverBudget => Pause::None,
        Outcome::Unauthorized | Outcome::Revoked => Pause::UntilConfigChanges,
        Outcome::Later(wait) => Pause::Server(wait.unwrap_or(retry)),
        Outcome::NotSupported | Outcome::TooLarge | Outcome::UnknownToken | Outcome::BadFormat => {
            Pause::Server(retry)
        }
        Outcome::NoRoute(error) => {
            if matches!(
                error.kind(),
                io::ErrorKind::InvalidData | io::ErrorKind::Other
            ) {
                tracing::warn!("cannot talk to the report server: {error}");
            } else {
                tracing::debug!("cannot reach the report server: {error}");
            }
            Pause::Network
        }
    }
}

fn check_in(spool: &Spool, sender: &mut Sender, retry: Duration) -> Option<Pause> {
    for day in checkin::due_days(spool.root()) {
        let payload = match checkin::payload(day) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!("cannot make the daily check-in: {error}");
                break;
            }
        };
        if checkin::is_empty(&payload) {
            if let Err(error) = checkin::sent(spool.root(), day) {
                tracing::warn!("cannot note the check-in for {day}: {error}");
                break;
            }
            continue;
        }
        let recorded = match sender.checkin(&payload) {
            Outcome::Stored => checkin::sent(spool.root(), day),
            Outcome::BadFormat | Outcome::TooLarge => {
                tracing::warn!("the server refused the check-in for {day}");
                checkin::sent(spool.root(), day)
            }
            Outcome::NotSupported => {
                if let Err(error) = checkin::not_yet_supported(spool.root()) {
                    tracing::warn!("cannot note when to check in again: {error}");
                }
                break;
            }
            outcome => return Some(pause_for(outcome, retry)),
        };
        if let Err(error) = recorded {
            tracing::warn!("cannot note the check-in for {day}: {error}");
            break;
        }
    }
    if let Err(error) = spool.unqueue_checkin() {
        tracing::warn!("cannot take the check-in out of the outbox: {error}");
    }
    None
}

fn send_waiting(spool: &Spool, config: &Config, waiting: &[String], retry: Duration) -> Pause {
    let Some(mut sender) = Sender::new(config, spool) else {
        return Pause::UntilConfigChanges;
    };
    let mut ledger = Ledger::load(spool.root());
    let policy = sender.identity.policy.clone();
    let asked: Vec<&String> = waiting.iter().filter(|id| spool.asked(id)).collect();
    let until_run = ledger.next_run_in(&policy);
    let metered = throttle::is_metered();
    let due = until_run.is_zero() && !metered && config.send;
    let checkin = due && spool.checkin_waiting() && config.sends_automatically();
    let batch: Vec<&String> = if due {
        waiting.iter().collect()
    } else {
        asked.clone()
    };
    if !config.send {
        for id in waiting.iter().filter(|id| !asked.contains(id)) {
            settle(spool, id, None);
        }
        if let Err(error) = spool.unqueue_checkin() {
            tracing::warn!("cannot take the check-in out of the outbox: {error}");
        }
    }
    if batch.is_empty() && !checkin {
        return match (config.send, metered) {
            (false, _) => Pause::None,
            (true, true) => Pause::Metered,
            (true, false) => next_run(spool, waiting, &mut ledger, &policy, until_run),
        };
    }
    if let Err(outcome) = sender.prepare() {
        return pause_for(outcome, retry);
    }
    let pause = (|| {
        if checkin && let Some(pause) = check_in(spool, &mut sender, retry) {
            return pause;
        }
        let policy = sender.identity.policy.clone();
        let mut mains = None;
        let mut held = (false, false);
        for id in batch {
            let Ok(report) = spool.read(id) else {
                settle(spool, id, Some(Status::Rejected));
                continue;
            };
            let field = |name| {
                report
                    .manifest
                    .get(name)
                    .and_then(Value::as_str)
                    .unwrap_or("")
            };
            let kind = field("kind").to_owned();
            let by_hand = asked.contains(&id);
            let counts_only = match policy.choose(&kind, field("signature"), id) {
                Choice::KeepHere if !by_hand => {
                    settle(spool, id, None);
                    continue;
                }
                Choice::CountsOnly => true,
                Choice::Send | Choice::KeepHere => false,
            };
            if !by_hand
                && !counts_only
                && report.status != Status::Sent
                && send::attachment_bytes(&report) > throttle::LARGE_ATTACHMENTS
                && !*mains.get_or_insert_with(throttle::on_mains)
            {
                held.1 = true;
                continue;
            }
            let (outcome, bytes) = sender.send(&report, counts_only, |bytes| {
                by_hand || ledger.allows(&policy, &kind, bytes)
            });
            match outcome {
                Outcome::Stored => {
                    ledger.spend(&kind, bytes);
                    settle(spool, id, Some(Status::Sent));
                }
                Outcome::OverBudget => held.0 = true,
                Outcome::BadFormat | Outcome::TooLarge => {
                    tracing::warn!("the server refused report {id}");
                    settle(spool, id, Some(Status::Rejected));
                }
                outcome => return pause_for(outcome, retry),
            }
        }
        match held {
            (false, false) => Pause::None,
            (next_day, power) => Pause::Held { next_day, power },
        }
    })();
    if sender.contacted {
        ledger.ran();
    }
    if let Err(error) = ledger.save(spool.root()) {
        tracing::warn!("cannot note what was sent today: {error}");
    }
    pause
}

fn next_run(
    spool: &Spool,
    waiting: &[String],
    ledger: &mut Ledger,
    policy: &Policy,
    until_run: Duration,
) -> Pause {
    let mut mains = None;
    let mut held = (false, false);
    for report in waiting.iter().filter_map(|id| spool.read(id).ok()) {
        let kind = report
            .manifest
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !ledger.allows(policy, kind, 0) {
            held.0 = true;
        } else if report.status != Status::Sent
            && send::attachment_bytes(&report) > throttle::LARGE_ATTACHMENTS
            && !*mains.get_or_insert_with(throttle::on_mains)
        {
            held.1 = true;
        } else {
            return Pause::Interval(until_run);
        }
    }
    match held {
        (false, false) => Pause::Interval(until_run),
        (next_day, power) => Pause::Held { next_day, power },
    }
}

fn settle(spool: &Spool, id: &str, status: Option<Status>) {
    let settled = match status {
        Some(status) => spool.settle(id, status),
        None => spool.unqueue(id),
    };
    if let Err(error) = settled {
        tracing::warn!("cannot record the state of report {id}: {error}");
        if let Err(error) = spool.unqueue(id) {
            tracing::warn!("cannot take report {id} out of the outbox: {error}");
        }
    }
}

fn forget(spool: &Spool, config: &Config, retry: Duration) -> Pause {
    let Some(request) = spool.forget_waiting() else {
        return Pause::None;
    };
    match send::forget(&request, config.ca.as_deref()) {
        Outcome::Stored => {
            if let Err(error) = spool.forgotten() {
                tracing::warn!("cannot note that the server forgot this computer: {error}");
            }
            Pause::None
        }
        outcome => pause_for(outcome, retry),
    }
}

fn run_sender(paths: &Paths) -> Result<(), Box<dyn Error>> {
    let spool = Spool::open(&paths.spool)?;
    let mut waiter = Waiter::new(&paths.config, &spool.outbox());
    let mut retry = FIRST_RETRY;
    loop {
        let waiting = spool.waiting()?;
        let forgetting = spool.forget_waiting().is_some();
        if waiting.is_empty() && !spool.checkin_waiting() && !forgetting {
            return Ok(());
        }
        let config = match Config::load(&paths.config) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!("cannot read the report settings: {error}");
                Config::default()
            }
        };
        let pause = match forget(&spool, &config, retry) {
            Pause::None if config.has_server() => send_waiting(&spool, &config, &waiting, retry),
            Pause::None if forgetting => Pause::None,
            Pause::None => Pause::UntilConfigChanges,
            pause => pause,
        };
        let backs_off = matches!(pause, Pause::Network | Pause::Server(_));
        let at = |wait: Duration| Some(Instant::now() + throttle::jittered(wait));
        let (deadline, wakes) = match pause {
            Pause::None => {
                retry = FIRST_RETRY;
                continue;
            }
            Pause::UntilConfigChanges => (None, Wakes::default()),
            Pause::Server(wait) => (at(wait), Wakes::default()),
            Pause::Network => (
                at(retry),
                Wakes {
                    network: true,
                    ..Wakes::default()
                },
            ),
            Pause::Interval(wait) => (Some(Instant::now() + wait), Wakes::default()),
            Pause::Metered => (
                None,
                Wakes {
                    network: true,
                    ..Wakes::default()
                },
            ),
            Pause::Held { next_day, power } => (
                next_day.then(|| Instant::now() + throttle::until_next_day()),
                Wakes {
                    power,
                    ..Wakes::default()
                },
            ),
        };
        let woken = waiter.wait(
            deadline,
            Wakes {
                outbox: !backs_off,
                ..wakes
            },
        )?;
        if backs_off {
            retry = match woken {
                Woken::Deadline => (retry * 2).min(LONGEST_RETRY),
                Woken::Network | Woken::Config | Woken::Outbox | Woken::Power => FIRST_RETRY,
            };
        }
    }
}

fn list(paths: &Paths) -> Result<(), Box<dyn Error>> {
    let spool = Spool::open(&paths.spool)?;
    let waiting = spool.waiting()?;
    for report in spool.all() {
        let field = |name| {
            report
                .manifest
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or("-")
        };
        let status = match report.status {
            Status::Sent => "sent",
            Status::Rejected => "rejected",
            Status::Local if waiting.contains(&report.id) => "waiting",
            Status::Local => "local",
        };
        println!(
            "{} {} {} {status} x{} {} bytes",
            report.id,
            field("time"),
            field("kind"),
            report.occurrences,
            report.bytes,
        );
    }
    Ok(())
}

fn store(paths: &Paths, manifest: &Path, attachments: &[String]) -> Result<(), Box<dyn Error>> {
    let spool = Spool::open(&paths.spool)?;
    let manifest: Map<String, Value> = serde_json::from_slice(&fs::read(manifest)?)?;
    let mut read = Vec::with_capacity(attachments.len());
    for file in attachments {
        let name = Path::new(file)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, file.clone()))?;
        read.push(Attachment {
            name: name.to_owned(),
            bytes: fs::read(file)?,
        });
    }
    let automatic = Config::load(&paths.config)?.sends_automatically();
    println!("{}", spool.store(manifest, read, automatic)?);
    Ok(())
}

fn run(arguments: &[String]) -> Result<(), Box<dyn Error>> {
    let mut paths = Paths {
        spool: PathBuf::from(SPOOL),
        config: PathBuf::from(CONFIG),
    };
    let mut rest = arguments;
    while let [flag, value, tail @ ..] = rest {
        match flag.as_str() {
            "--spool" => paths.spool = PathBuf::from(value),
            "--config" => paths.config = PathBuf::from(value),
            _ => break,
        }
        rest = tail;
    }
    match rest {
        [command] if command == "send" => run_sender(&paths),
        [command] if command == "list" => list(&paths),
        [command, unit] if command == "collect-failures" => {
            Ok(bande_reporter::diagnose::ask_for_collection(unit)?)
        }
        [command] if command == "serve" => bande_reporter::serve::run(&paths.spool, &paths.config),
        [command] if command == "watch" => bande_reporter::watch::run(&paths.spool, &paths.config),
        [command] if command == "resources" => {
            bande_reporter::resources::run(&paths.spool, &paths.config)
        }
        [command, manifest, attachments @ ..] if command == "store" => {
            store(&paths, Path::new(manifest), attachments)
        }
        [command, id] if command == "queue" => Ok(Spool::open(&paths.spool)?.queue_by_hand(id)?),
        [command, id] if command == "delete" => Ok(Spool::open(&paths.spool)?.delete(id)?),
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    bande_log::init("bande-reporter");
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
