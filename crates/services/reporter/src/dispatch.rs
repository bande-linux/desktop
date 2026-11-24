use std::error::Error;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::checkin;
use crate::config::Config;
use crate::http::Session;
use crate::policy::{Choice, Policy};
use crate::send::{self, Outcome, Sender};
use crate::spool::{Spool, Status};
use crate::throttle::{self, Ledger};
use crate::wait::{Waiter, Wakes, Woken};

const FIRST_RETRY: Duration = Duration::from_secs(60);
const LONGEST_RETRY: Duration = Duration::from_secs(6 * 60 * 60);

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
        Outcome::Stored | Outcome::OverBudget | Outcome::Withdrawn => Pause::None,
        Outcome::Unauthorized | Outcome::Revoked => Pause::UntilConfigChanges,
        Outcome::Later(wait) => Pause::Server(wait.unwrap_or(retry)),
        Outcome::TooLarge | Outcome::UnknownToken | Outcome::BadFormat => Pause::Server(retry),
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
        if !spool.checkin_waiting() {
            return None;
        }
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

fn send_waiting(
    session: Session,
    spool: &Spool,
    config: &Config,
    waiting: &[String],
    retry: Duration,
) -> Pause {
    let mut sender = Sender::new(session, spool);
    let mut ledger = Ledger::load(spool.root());
    let policy = sender.identity.policy.clone();
    let asked: Vec<&String> = waiting.iter().filter(|id| spool.asked(id)).collect();
    let until_run = ledger.next_run_in(&policy);
    let metered = throttle::is_metered();
    let due = until_run.is_zero() && !metered && config.send;
    let checkin = due && spool.checkin_waiting();
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
            if !spool.is_queued(id) {
                continue;
            }
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

fn forget(session: &Session, spool: &Spool, retry: Duration) -> Pause {
    let Some(request) = spool.forget_waiting() else {
        return Pause::None;
    };
    match send::forget(session, &request) {
        Outcome::Stored => {
            if let Err(error) = spool.forgotten() {
                tracing::warn!("cannot note that the server forgot this computer: {error}");
                return Pause::Server(retry);
            }
            Pause::None
        }
        outcome => pause_for(outcome, retry),
    }
}

pub fn run(spool_root: &Path, config_path: &Path) -> Result<(), Box<dyn Error>> {
    run_over(spool_root, config_path, Session::with_report_server)
}

fn run_over(
    spool_root: &Path,
    config_path: &Path,
    connect: impl Fn() -> Session,
) -> Result<(), Box<dyn Error>> {
    let spool = Spool::open(spool_root)?;
    let mut waiter = Waiter::new(config_path, &spool.outbox());
    let mut retry = FIRST_RETRY;
    loop {
        let waiting = spool.waiting()?;
        if waiting.is_empty() && !spool.checkin_waiting() && spool.forget_waiting().is_none() {
            return Ok(());
        }
        let config = match Config::load(config_path) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!("cannot read the report settings: {error}");
                Config::default()
            }
        };
        let session = connect();
        let pause = match forget(&session, &spool, retry) {
            Pause::None => send_waiting(session, &spool, &config, &waiting, retry),
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

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::sync::mpsc::{self, Receiver};

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::config;
    use crate::identity::Identity;
    use crate::serve::set_sending;

    const REPORT: &str = "POST /v1/reports";
    const ENROLL: [&str; 2] = ["GET /v1/enroll", "POST /v1/enroll"];
    const FORGET: &str = "DELETE /v1/me";

    fn answer(stream: &TcpStream, asked: impl Fn(&str) -> io::Result<()>) -> io::Result<String> {
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        reader.read_line(&mut request)?;
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            match line.trim().split_once(':') {
                Some((name, value)) if name.eq_ignore_ascii_case("content-length") => {
                    length = value.trim().parse().map_err(io::Error::other)?;
                }
                Some(_) => {}
                None => break,
            }
        }
        io::copy(&mut reader.take(length), &mut io::sink())?;
        let request = request.trim_end_matches(" HTTP/1.1\r\n").to_owned();
        asked(&request)?;
        let (status, body) = match request.as_str() {
            "GET /v1/enroll" => (200, json!({"challenge": "c", "difficulty": 0})),
            "POST /v1/enroll" => (201, json!({"machine": "m", "token": "t"})),
            _ => (201, json!({"policy": {"min_interval": 1}})),
        };
        let body = body.to_string();
        let mut stream = stream;
        write!(
            stream,
            "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        Ok(request)
    }

    struct Computer {
        _folder: TempDir,
        spool: Spool,
        config: PathBuf,
    }

    impl Computer {
        fn new() -> io::Result<Self> {
            let folder = tempfile::tempdir()?;
            Ok(Self {
                spool: Spool::open(&folder.path().join("reports"))?,
                config: folder.path().join("reports.conf"),
                _folder: folder,
            })
        }

        fn stored(&self, signature: &str) -> io::Result<String> {
            let manifest =
                json!({"kind": "crash", "program": "bande-files", "signature": signature});
            let manifest = manifest.as_object().cloned().unwrap_or_default();
            self.spool.store(manifest, Vec::new(), true)
        }

        fn requests_until_idle(
            &self,
            switch_off_at: Option<&'static str>,
        ) -> Result<Vec<String>, Box<dyn Error>> {
            let (root, config) = (self.spool.root().to_path_buf(), self.config.clone());
            let (address, requests) = server(move |request| {
                if switch_off_at == Some(request) {
                    set_sending(&root, &config, false)?;
                }
                Ok(())
            })?;
            run_over(self.spool.root(), &self.config, || {
                Session::unencrypted(&address)
            })?;
            Ok(requests.try_iter().collect::<io::Result<_>>()?)
        }
    }

    fn server(
        asked: impl Fn(&str) -> io::Result<()> + Send + 'static,
    ) -> io::Result<(String, Receiver<io::Result<String>>)> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = format!("http://{}", listener.local_addr()?);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let request = stream.and_then(|stream| answer(&stream, &asked));
                if sender.send(request).is_err() {
                    return;
                }
            }
        });
        Ok((address, receiver))
    }

    #[test]
    fn while_sending_is_off_only_a_report_sent_by_hand_leaves() -> Result<(), Box<dyn Error>> {
        let computer = Computer::new()?;
        let spool = &computer.spool;
        let kept = computer.stored("a")?;
        spool.queue_checkin()?;
        assert!(computer.requests_until_idle(None)?.is_empty());
        assert!(spool.waiting()?.is_empty());
        assert_eq!(spool.read(&kept)?.status, Status::Local);

        let sent = computer.stored("b")?;
        spool.queue_by_hand(&sent)?;
        spool.queue(&kept)?;
        assert_eq!(
            computer.requests_until_idle(None)?,
            [ENROLL[0], ENROLL[1], REPORT]
        );
        assert_eq!(spool.read(&sent)?.status, Status::Sent);
        assert_eq!(spool.read(&kept)?.status, Status::Local);
        Ok(())
    }

    #[test]
    fn switching_off_while_sending_stops_it_and_forgets_the_computer() -> Result<(), Box<dyn Error>>
    {
        for (switch_off_at, expected) in [
            (ENROLL[0], vec![ENROLL[0]]),
            (ENROLL[1], vec![ENROLL[0], ENROLL[1], FORGET]),
            (REPORT, vec![ENROLL[0], ENROLL[1], REPORT, FORGET]),
        ] {
            let computer = Computer::new()?;
            config::set_sending(&computer.config, true)?;
            computer.stored("a")?;
            computer.stored("b")?;
            assert_eq!(computer.requests_until_idle(Some(switch_off_at))?, expected);
            assert!(Identity::load(computer.spool.root()).is_none());
        }
        Ok(())
    }
}
