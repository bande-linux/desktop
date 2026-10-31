use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use std::io::{self, Write};
use std::os::unix::net::UnixStream;

use zbus::blocking::Connection;

use crate::backlight::{Backlight, dimmed};
use crate::idle::{Command, IdleMachine, Policy};
use crate::logind::{self, ManagerProxyBlocking, SessionProxyBlocking};

#[zbus::proxy(
    interface = "org.unbound.Config",
    default_service = "org.unbound.Config",
    default_path = "/org/unbound/Config"
)]
pub trait Config {
    #[zbus(property)]
    fn idle_dim_after(&self) -> zbus::Result<u64>;

    #[zbus(property)]
    fn idle_lock_after(&self) -> zbus::Result<u64>;

    #[zbus(property)]
    fn idle_suspend_after(&self) -> zbus::Result<u64>;
}

type Executor = Box<dyn Fn(Command) + Send + Sync>;

pub struct IdleController {
    machine: Mutex<IdleMachine>,
    execute: Executor,
    timer_listeners: Mutex<Vec<UnixStream>>,
}

impl IdleController {
    pub fn new(policy: Policy, execute: impl Fn(Command) + Send + Sync + 'static) -> Self {
        Self {
            machine: Mutex::new(IdleMachine::new(policy)),
            execute: Box::new(execute),
            timer_listeners: Mutex::new(Vec::new()),
        }
    }

    fn run(&self, step: impl FnOnce(&mut IdleMachine) -> Vec<Command>) {
        let commands = step(&mut self.machine.lock().unwrap_or_else(PoisonError::into_inner));
        for command in commands {
            if command == Command::RestartIdleTimers {
                self.restart_idle_timers();
            }
            (self.execute)(command);
        }
    }

    fn restart_idle_timers(&self) {
        self.timer_listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain_mut(|listener| listener.write_all(&[1]).is_ok());
    }

    pub fn idle_for(&self, idle: Duration) {
        self.run(|machine| machine.idle_for(idle));
    }

    pub fn resumed(&self) {
        self.run(IdleMachine::resumed);
    }

    pub fn set_inhibited(&self, inhibited: bool) {
        self.run(|machine| machine.set_inhibited(inhibited));
    }

    pub fn set_locked(&self, locked: bool) {
        self.run(|machine| {
            machine.set_locked(locked);
            Vec::new()
        });
    }

    pub fn set_policy(&self, policy: Policy) {
        self.run(|machine| machine.set_policy(policy));
        self.restart_idle_timers();
    }

    pub fn idle_timer_restarts(&self) -> io::Result<UnixStream> {
        let (changes, listener) = UnixStream::pair()?;
        listener.set_nonblocking(true)?;
        self.timer_listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(listener);
        Ok(changes)
    }

    pub fn thresholds(&self) -> Vec<Duration> {
        self.machine
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .thresholds()
    }
}

fn policy(config: &ConfigProxyBlocking<'_>) -> zbus::Result<Policy> {
    let suspend_after = config.idle_suspend_after()?;
    Ok(Policy {
        dim_after: Duration::from_secs(config.idle_dim_after()?),
        lock_after: Duration::from_secs(config.idle_lock_after()?),
        suspend_after: (suspend_after > 0).then(|| Duration::from_secs(suspend_after)),
    })
}

pub fn configured_policy(session: &Connection) -> zbus::Result<Policy> {
    policy(&ConfigProxyBlocking::new(session)?)
}

pub fn follow_config(session: &Connection, controller: &Arc<IdleController>) -> zbus::Result<()> {
    let config = ConfigProxyBlocking::new(session)?;
    let changes = [
        config.receive_idle_dim_after_changed(),
        config.receive_idle_lock_after_changed(),
        config.receive_idle_suspend_after_changed(),
    ];
    for change in changes {
        let config = config.clone();
        let controller = Arc::clone(controller);
        thread::spawn(move || {
            for _ in change {
                if let Ok(policy) = policy(&config) {
                    controller.set_policy(policy);
                }
            }
        });
    }
    controller.set_policy(policy(&config)?);
    Ok(())
}

pub fn follow_lock(system: &Connection, controller: &Arc<IdleController>) -> zbus::Result<()> {
    let controller = Arc::clone(controller);
    logind::follow_locked_hint(system, move |locked| controller.set_locked(locked))
}

pub fn logind_executor(
    manager: ManagerProxyBlocking<'static>,
    session: SessionProxyBlocking<'static>,
    backlight: Option<Backlight>,
) -> impl Fn(Command) + Send + Sync + 'static {
    let undimmed = Mutex::new(None);
    move |command| {
        let set_backlight = |level: u32| match &backlight {
            Some(backlight) => session.set_brightness("backlight", &backlight.name, level),
            None => Ok(()),
        };
        let result = match command {
            Command::Lock => session.lock(),
            Command::Suspend => match manager.can_suspend() {
                Ok(answer) if answer == "yes" => manager.suspend(false),
                Ok(_) => Ok(()),
                Err(error) => Err(error),
            },
            Command::Dim => match backlight.as_ref().map(Backlight::brightness) {
                Some(Ok(level)) => match dimmed(level) {
                    Some(dim) => {
                        *undimmed.lock().unwrap_or_else(PoisonError::into_inner) = Some(level);
                        set_backlight(dim)
                    }
                    None => Ok(()),
                },
                Some(Err(error)) => Err(zbus::Error::InputOutput(error.into())),
                None => Ok(()),
            },
            Command::Undim => {
                let level = undimmed
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                level.map_or(Ok(()), set_backlight)
            }
            Command::RestartIdleTimers => Ok(()),
        };
        if let Err(error) = result {
            tracing::warn!("cannot carry out {command:?}: {error}");
        }
    }
}
