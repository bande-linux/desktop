use std::time::Duration;

pub const LOCKED_DISPLAY_OFF_AFTER: Duration = Duration::from_secs(60);
pub const DIM_AHEAD: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub screen_saver_after: Option<Duration>,
    pub display_off_after: Option<Duration>,
    pub require_password_after: Duration,
    pub suspend_after: Option<Duration>,
}

impl Policy {
    fn rest_after(&self) -> Option<Duration> {
        match (self.screen_saver_after, self.display_off_after) {
            (Some(saver), Some(off)) => Some(saver.min(off)),
            (saver, off) => saver.or(off),
        }
    }

    fn dim_after(&self) -> Option<Duration> {
        self.rest_after()
            .map(|rest| rest.saturating_sub(DIM_AHEAD).max(Duration::from_secs(1)))
    }

    fn password_after(&self) -> Option<Duration> {
        self.rest_after()
            .map(|rest| rest + self.require_password_after)
    }

    fn suspend_after(&self) -> Option<Duration> {
        self.suspend_after.map(|suspend| {
            self.display_off_after
                .map_or(suspend, |off| suspend.max(off))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Active,
    Dimmed,
    Resting,
    Suspended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Dim,
    Undim,
    Lock,
    RequirePassword(bool),
    DisplayOff,
    DisplayOn,
    Suspend,
    RestartIdleTimers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inhibitor {
    ScreenSaver,
    Logind,
}

#[derive(Debug)]
pub struct IdleMachine {
    policy: Policy,
    stage: Stage,
    dimmed: bool,
    display_off: bool,
    locked: bool,
    locked_when_idle: bool,
    password_required: bool,
    inhibitors: Vec<Inhibitor>,
}

impl IdleMachine {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            stage: Stage::Active,
            dimmed: false,
            display_off: false,
            locked: false,
            locked_when_idle: false,
            password_required: true,
            inhibitors: Vec::new(),
        }
    }

    pub fn thresholds(&self) -> Vec<Duration> {
        let mut thresholds: Vec<Duration> = [
            Some(LOCKED_DISPLAY_OFF_AFTER),
            self.policy.dim_after(),
            self.policy.rest_after(),
            self.policy.password_after(),
            self.policy.display_off_after,
            self.policy.suspend_after(),
        ]
        .into_iter()
        .flatten()
        .collect();
        thresholds.sort_unstable();
        thresholds.dedup();
        thresholds
    }

    pub fn idle_for(&mut self, idle: Duration) -> Vec<Command> {
        if !self.inhibitors.is_empty() {
            return Vec::new();
        }
        let reached = |after: Option<Duration>| after.is_some_and(|after| idle >= after);
        let mut commands = Vec::new();
        if self.stage < Stage::Dimmed && reached(self.policy.dim_after()) {
            self.stage = Stage::Dimmed;
            self.dimmed = true;
            commands.push(Command::Dim);
        }
        if self.stage < Stage::Resting && reached(self.policy.rest_after()) {
            self.stage = Stage::Resting;
            if !self.locked {
                self.locked = true;
                self.locked_when_idle = true;
                if !self.policy.require_password_after.is_zero() {
                    self.password_required = false;
                    commands.push(Command::RequirePassword(false));
                }
                commands.push(Command::Lock);
            }
        }
        if !self.password_required && reached(self.policy.password_after()) {
            self.password_required = true;
            commands.push(Command::RequirePassword(true));
        }
        if !self.display_off
            && (reached(self.policy.display_off_after)
                || (self.locked && !self.locked_when_idle && idle >= LOCKED_DISPLAY_OFF_AFTER))
        {
            self.display_off = true;
            commands.push(Command::DisplayOff);
        }
        if self.stage < Stage::Suspended && reached(self.policy.suspend_after()) {
            self.stage = Stage::Suspended;
            commands.push(Command::Suspend);
        }
        commands
    }

    pub fn resumed(&mut self) -> Vec<Command> {
        self.stage = Stage::Active;
        self.undim()
    }

    pub fn set_inhibited(&mut self, inhibitor: Inhibitor, held: bool) -> Vec<Command> {
        let was_inhibited = !self.inhibitors.is_empty();
        self.inhibitors.retain(|kind| *kind != inhibitor);
        if held {
            self.inhibitors.push(inhibitor);
        }
        let inhibited = !self.inhibitors.is_empty();
        if inhibited == was_inhibited {
            return Vec::new();
        }
        if inhibited {
            if self.stage == Stage::Dimmed {
                self.stage = Stage::Active;
            }
            return self.undim();
        }
        self.stage = Stage::Active;
        vec![Command::RestartIdleTimers]
    }

    pub fn set_locked(&mut self, locked: bool) -> Vec<Command> {
        self.locked = locked;
        if locked {
            return Vec::new();
        }
        self.locked_when_idle = false;
        if std::mem::replace(&mut self.password_required, true) {
            return Vec::new();
        }
        vec![Command::RequirePassword(true)]
    }

    pub fn set_policy(&mut self, policy: Policy) -> Vec<Command> {
        if policy == self.policy {
            return Vec::new();
        }
        self.policy = policy;
        vec![Command::RestartIdleTimers]
    }

    fn undim(&mut self) -> Vec<Command> {
        let mut commands = Vec::new();
        if std::mem::take(&mut self.dimmed) {
            commands.push(Command::Undim);
        }
        if std::mem::take(&mut self.display_off) {
            commands.push(Command::DisplayOn);
        }
        commands
    }
}
