use std::time::Duration;

pub const LOCKED_DISPLAY_OFF_AFTER: Duration = Duration::from_secs(60);
pub const DIM_AHEAD: Duration = Duration::from_secs(10);
pub const LOCK_SETTLES_IN: Duration = Duration::from_secs(5);
pub const STREAMS_HOLD_ON_BATTERY: Duration = Duration::from_secs(30 * 60);
pub const SLEEP_AGAIN_AFTER_WAKE: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub screen_saver_after: Option<Duration>,
    pub display_off_after: Option<Duration>,
    pub require_password_after: Duration,
    pub suspend_after: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policies {
    pub on_power: Policy,
    pub on_battery: Policy,
}

impl Policies {
    fn for_source(&self, on_battery: bool) -> Policy {
        match on_battery {
            true => self.on_battery,
            false => self.on_power,
        }
    }
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

    pub fn computer_sleeps_after(&self) -> Option<Duration> {
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
    IdleHint(bool),
    RestartIdleTimers,
    RestartSleepTimer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inhibitor {
    ScreenSaver,
    Logind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assertion {
    Streams,
    Applications,
    Logind,
}

#[derive(Debug)]
pub struct IdleMachine {
    policies: Policies,
    policy: Policy,
    stage: Stage,
    dimmed: bool,
    display_off: bool,
    locked: bool,
    locked_when_idle: bool,
    lock_pending: bool,
    lock_sent: Option<Duration>,
    others_lock: bool,
    asleep: bool,
    display_off_when_locked: bool,
    woke_locked: bool,
    grace_ends: Option<Duration>,
    inhibitors: Vec<Inhibitor>,
    assertions: Vec<Assertion>,
    sleep_overdue: bool,
    idle_hint: bool,
    on_battery: bool,
    battery_low: bool,
}

impl IdleMachine {
    pub fn new(policies: Policies) -> Self {
        Self {
            policies,
            policy: policies.on_power,
            stage: Stage::Active,
            dimmed: false,
            display_off: false,
            locked: false,
            locked_when_idle: false,
            lock_pending: false,
            lock_sent: None,
            others_lock: false,
            asleep: false,
            display_off_when_locked: false,
            woke_locked: false,
            grace_ends: None,
            inhibitors: Vec::new(),
            assertions: Vec::new(),
            sleep_overdue: false,
            idle_hint: false,
            on_battery: false,
            battery_low: false,
        }
    }

    pub fn thresholds(&self) -> Vec<Duration> {
        let mut thresholds: Vec<Duration> = [
            Some(LOCKED_DISPLAY_OFF_AFTER),
            self.policy
                .computer_sleeps_after()
                .map(|_| SLEEP_AGAIN_AFTER_WAKE),
            self.policy.dim_after(),
            self.policy.rest_after(),
            self.policy.password_after(),
            self.policy.display_off_after,
            self.policy
                .display_off_after
                .map(|off| off + LOCK_SETTLES_IN),
            self.policy.computer_sleeps_after(),
            self.policy
                .computer_sleeps_after()
                .map(|after| after + STREAMS_HOLD_ON_BATTERY),
        ]
        .into_iter()
        .flatten()
        .collect();
        thresholds.sort_unstable();
        thresholds.dedup();
        thresholds
    }

    pub fn password_required(&self, now: Duration) -> bool {
        self.grace_ends.is_none_or(|ends| now >= ends)
    }

    fn lock_failed(&self, now: Duration) -> bool {
        self.lock_sent
            .is_some_and(|sent| now >= sent + LOCK_SETTLES_IN)
    }

    fn rests_locked(&self, idle: Duration) -> bool {
        self.locked && !self.locked_when_idle && idle >= LOCKED_DISPLAY_OFF_AFTER
    }

    pub fn idle_for(&mut self, idle: Duration, now: Duration) -> Vec<Command> {
        if !self.inhibitors.is_empty() {
            if !self.display_off && self.rests_locked(idle) {
                self.display_off = true;
                return vec![Command::DisplayOff];
            }
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
            self.idle_hint = true;
            commands.push(Command::IdleHint(true));
            if !self.locked || self.lock_failed(now) {
                let unlocked = !std::mem::replace(&mut self.locked, true);
                self.locked_when_idle = true;
                self.lock_pending = true;
                self.lock_sent = Some(now);
                if unlocked
                    && !self.policy.require_password_after.is_zero()
                    && !self.asleep
                    && !self.others_lock
                {
                    self.grace_ends = self
                        .policy
                        .password_after()
                        .map(|after| (now + after).saturating_sub(idle));
                    commands.push(Command::RequirePassword(false));
                }
                commands.push(Command::Lock);
            }
        }
        if reached(self.policy.password_after()) {
            commands.extend(self.require_password());
        }
        if !self.display_off && (reached(self.policy.display_off_after) || self.rests_locked(idle))
        {
            let lock_overdue = reached(
                self.policy
                    .display_off_after
                    .map(|off| off + LOCK_SETTLES_IN),
            );
            if self.lock_sent.is_some() && !lock_overdue {
                self.display_off_when_locked = true;
            } else {
                self.display_off = true;
                commands.push(Command::DisplayOff);
            }
        }
        if self.stage < Stage::Suspended && reached(self.sleep_after()) {
            if !self.kept_awake(idle) {
                self.stage = Stage::Suspended;
                self.sleep_overdue = false;
                commands.push(Command::Suspend);
            } else {
                self.sleep_overdue = true;
            }
        }
        commands
    }

    pub fn sleep_after(&self) -> Option<Duration> {
        let after = self.policy.computer_sleeps_after()?;
        Some(if self.woke_locked && self.locked {
            after.min(SLEEP_AGAIN_AFTER_WAKE)
        } else {
            after
        })
    }

    pub fn sleeping(&mut self) -> Vec<Command> {
        self.asleep = true;
        self.require_password()
    }

    pub fn woke(&mut self) -> Vec<Command> {
        self.asleep = false;
        self.locked_when_idle = false;
        self.woke_locked = self.locked;
        Vec::new()
    }

    pub fn lock_requested(&mut self) -> Vec<Command> {
        if std::mem::take(&mut self.lock_pending) {
            return Vec::new();
        }
        self.others_lock = true;
        self.require_password()
    }

    fn require_password(&mut self) -> Vec<Command> {
        match self.grace_ends.take() {
            Some(_) => vec![Command::RequirePassword(true)],
            None => Vec::new(),
        }
    }

    pub fn resumed(&mut self, now: Duration) -> Vec<Command> {
        self.stage = Stage::Active;
        self.sleep_overdue = false;
        let mut commands = Vec::new();
        if self.lock_failed(now) {
            self.locked = false;
            self.locked_when_idle = false;
            self.lock_pending = false;
            self.lock_sent = None;
            commands = self.require_password();
        }
        commands.extend(self.undim());
        commands
    }

    fn kept_awake(&self, idle: Duration) -> bool {
        let streams_spent = self.on_battery
            && (self.battery_low
                || self
                    .policy
                    .computer_sleeps_after()
                    .is_some_and(|after| idle >= after + STREAMS_HOLD_ON_BATTERY));
        self.assertions
            .iter()
            .any(|assertion| *assertion != Assertion::Streams || !streams_spent)
    }

    pub fn set_power_source(&mut self, on_battery: bool, battery_low: bool) -> Vec<Command> {
        self.on_battery = on_battery;
        self.battery_low = battery_low;
        let policy = self.policies.for_source(on_battery);
        if policy != self.policy {
            return self.follow(policy);
        }
        if self.sleep_overdue && !self.kept_awake(Duration::ZERO) {
            self.sleep_overdue = false;
            return vec![Command::RestartSleepTimer];
        }
        Vec::new()
    }

    pub fn set_asserted(&mut self, assertion: Assertion, held: bool) -> Vec<Command> {
        self.assertions.retain(|kind| *kind != assertion);
        if held {
            self.assertions.push(assertion);
        }
        if !self.kept_awake(Duration::ZERO) && std::mem::take(&mut self.sleep_overdue) {
            return vec![Command::RestartSleepTimer];
        }
        Vec::new()
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
            if self.locked {
                return Vec::new();
            }
            return self.undim();
        }
        self.stage = Stage::Active;
        self.sleep_overdue = false;
        vec![Command::RestartIdleTimers]
    }

    pub fn set_locked(&mut self, locked: bool) -> Vec<Command> {
        self.locked = locked;
        self.lock_sent = None;
        let display_off = std::mem::take(&mut self.display_off_when_locked);
        if locked {
            if display_off && !self.display_off {
                self.display_off = true;
                return vec![Command::DisplayOff];
            }
            return Vec::new();
        }
        self.locked_when_idle = false;
        self.lock_pending = false;
        self.others_lock = false;
        self.woke_locked = false;
        self.require_password()
    }

    pub fn set_policies(&mut self, policies: Policies) -> Vec<Command> {
        self.policies = policies;
        self.follow(policies.for_source(self.on_battery))
    }

    fn follow(&mut self, policy: Policy) -> Vec<Command> {
        if policy == self.policy {
            return Vec::new();
        }
        self.policy = policy;
        self.sleep_overdue = false;
        vec![Command::RestartIdleTimers]
    }

    fn undim(&mut self) -> Vec<Command> {
        self.display_off_when_locked = false;
        let mut commands = Vec::new();
        if std::mem::take(&mut self.dimmed) {
            commands.push(Command::Undim);
        }
        if std::mem::take(&mut self.display_off) {
            commands.push(Command::DisplayOn);
        }
        if std::mem::take(&mut self.idle_hint) {
            commands.push(Command::IdleHint(false));
        }
        commands
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minutes(minutes: u64) -> Duration {
        Duration::from_secs(minutes * 60)
    }

    fn grace_machine() -> IdleMachine {
        let policy = Policy {
            screen_saver_after: Some(minutes(5)),
            display_off_after: Some(minutes(10)),
            require_password_after: minutes(5),
            suspend_after: Some(minutes(10)),
        };
        IdleMachine::new(Policies {
            on_power: policy,
            on_battery: policy,
        })
    }

    #[test]
    fn sleep_or_a_lock_request_during_the_password_grace_period_requires_the_password() {
        let booted = minutes(100);
        for asleep in [true, false] {
            let mut machine = grace_machine();
            let saver = machine.idle_for(minutes(5), booted);
            assert!(saver.contains(&Command::RequirePassword(false)));
            assert!(!machine.password_required(booted));
            assert!(machine.lock_requested().is_empty());
            machine.set_locked(true);
            let required = match asleep {
                true => machine.sleeping(),
                false => machine.lock_requested(),
            };
            assert_eq!(required, vec![Command::RequirePassword(true)]);
            assert!(machine.password_required(booted));
        }
    }

    #[test]
    fn the_grace_period_ends_on_time_and_never_covers_a_lock_the_user_started() {
        let locked_at = minutes(100);
        let mut machine = grace_machine();
        machine.idle_for(minutes(5), locked_at);
        machine.resumed(locked_at + Duration::from_secs(1));
        machine.set_locked(true);
        machine.set_inhibited(Inhibitor::ScreenSaver, true);
        assert!(
            machine
                .idle_for(minutes(10), locked_at + minutes(5))
                .is_empty()
        );
        assert!(!machine.password_required(locked_at + minutes(5) - Duration::from_secs(1)));
        assert!(machine.password_required(locked_at + minutes(5)));

        let mut machine = grace_machine();
        machine.idle_for(minutes(5), locked_at);
        machine.set_inhibited(Inhibitor::ScreenSaver, true);
        machine.set_inhibited(Inhibitor::ScreenSaver, false);
        let relocked = machine.idle_for(minutes(5), locked_at + minutes(1));
        assert!(relocked.contains(&Command::Lock));
        assert!(machine.password_required(locked_at + minutes(5)));

        let mut machine = grace_machine();
        assert!(machine.lock_requested().is_empty());
        machine.idle_for(minutes(5), locked_at);
        assert!(machine.password_required(locked_at));
    }
}
