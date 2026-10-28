use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use zbus::blocking::Connection;

use crate::idle::{Command, IdleMachine, Policy};
use crate::logind::{ManagerProxyBlocking, SessionProxyBlocking};

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
}

impl IdleController {
    pub fn new(policy: Policy, execute: impl Fn(Command) + Send + Sync + 'static) -> Self {
        Self {
            machine: Mutex::new(IdleMachine::new(policy)),
            execute: Box::new(execute),
        }
    }

    fn run(&self, step: impl FnOnce(&mut IdleMachine) -> Vec<Command>) {
        let commands = step(&mut self.machine.lock().unwrap_or_else(PoisonError::into_inner));
        for command in commands {
            (self.execute)(command);
        }
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
    let path = ManagerProxyBlocking::new(system)?.get_session("auto")?;
    let session = SessionProxyBlocking::builder(system).path(path)?.build()?;
    let changes = session.receive_locked_hint_changed();
    controller.set_locked(session.locked_hint()?);
    let controller = Arc::clone(controller);
    thread::spawn(move || {
        for change in changes {
            if let Ok(locked) = change.get() {
                controller.set_locked(locked);
            }
        }
    });
    Ok(())
}

pub fn logind_executor(
    manager: ManagerProxyBlocking<'static>,
    session: SessionProxyBlocking<'static>,
) -> impl Fn(Command) + Send + Sync + 'static {
    move |command| {
        let result = match command {
            Command::Lock => session.lock(),
            Command::Suspend => match manager.can_suspend() {
                Ok(answer) if answer == "yes" => manager.suspend(false),
                Ok(_) => Ok(()),
                Err(error) => Err(error),
            },
            Command::Dim | Command::Undim | Command::RestartIdleTimers => Ok(()),
        };
        if let Err(error) = result {
            eprintln!("unbound-power: cannot carry out {command:?}: {error}");
        }
    }
}
