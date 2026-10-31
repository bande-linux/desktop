use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub dim_after: Duration,
    pub lock_after: Duration,
    pub suspend_after: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Active,
    Dimmed,
    Locked,
    Suspended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Dim,
    Undim,
    Lock,
    Suspend,
    RestartIdleTimers,
}

#[derive(Debug)]
pub struct IdleMachine {
    policy: Policy,
    stage: Stage,
    dimmed: bool,
    locked: bool,
    inhibited: bool,
}

impl IdleMachine {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            stage: Stage::Active,
            dimmed: false,
            locked: false,
            inhibited: false,
        }
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    pub fn thresholds(&self) -> Vec<Duration> {
        let mut thresholds: Vec<Duration> = [
            Some(self.policy.dim_after),
            Some(self.policy.lock_after),
            self.policy.suspend_after,
        ]
        .into_iter()
        .flatten()
        .collect();
        thresholds.sort_unstable();
        thresholds.dedup();
        thresholds
    }

    pub fn idle_for(&mut self, idle: Duration) -> Vec<Command> {
        if self.inhibited {
            return Vec::new();
        }
        let mut commands = Vec::new();
        if self.stage < Stage::Dimmed && idle >= self.policy.dim_after {
            self.stage = Stage::Dimmed;
            self.dimmed = true;
            commands.push(Command::Dim);
        }
        if self.stage < Stage::Locked && idle >= self.policy.lock_after {
            self.stage = Stage::Locked;
            if !self.locked {
                self.locked = true;
                commands.push(Command::Lock);
            }
        }
        if self.stage < Stage::Suspended
            && self
                .policy
                .suspend_after
                .is_some_and(|suspend_after| idle >= suspend_after)
        {
            self.stage = Stage::Suspended;
            commands.push(Command::Suspend);
        }
        commands
    }

    pub fn resumed(&mut self) -> Vec<Command> {
        self.stage = Stage::Active;
        self.undim()
    }

    pub fn set_inhibited(&mut self, inhibited: bool) -> Vec<Command> {
        if inhibited == self.inhibited {
            return Vec::new();
        }
        self.inhibited = inhibited;
        if inhibited {
            if self.stage == Stage::Dimmed {
                self.stage = Stage::Active;
            }
            return self.undim();
        }
        self.stage = Stage::Active;
        vec![Command::RestartIdleTimers]
    }

    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
    }

    pub fn set_policy(&mut self, policy: Policy) -> Vec<Command> {
        if policy == self.policy {
            return Vec::new();
        }
        self.policy = policy;
        vec![Command::RestartIdleTimers]
    }

    fn undim(&mut self) -> Vec<Command> {
        if !self.dimmed {
            return Vec::new();
        }
        self.dimmed = false;
        vec![Command::Undim]
    }
}
