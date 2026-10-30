use unbound_settings_panes::{DIM_CHOICES, LOCK_CHOICES, SettingId, Value, WITH_DISPLAY};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Idle {
    pub dim: u64,
    pub lock: u64,
    pub sleep: Option<u64>,
}

fn with_display() -> Value {
    Value::Keyword(WITH_DISPLAY.to_owned())
}

fn at_most(choices: &[u64], limit: u64) -> u64 {
    choices
        .iter()
        .copied()
        .filter(|choice| *choice <= limit)
        .max()
        .unwrap_or(limit)
}

fn at_least(choices: &[u64], limit: u64) -> u64 {
    choices
        .iter()
        .copied()
        .filter(|choice| *choice >= limit)
        .min()
        .unwrap_or(limit)
}

impl Idle {
    fn sleeps_before(&self, lock: u64) -> bool {
        self.sleep.is_some_and(|sleep| sleep > 0 && sleep < lock)
    }

    fn raise_lock(&self, lock: u64, writes: &mut Vec<(SettingId, Value)>) {
        if self.sleeps_before(lock) {
            writes.push((SettingId::SleepAfter, with_display()));
        }
        writes.push((SettingId::LockAfter, Value::Seconds(lock)));
    }

    fn lower_lock(&self, lock: u64, writes: &mut Vec<(SettingId, Value)>) {
        if self.dim > lock {
            writes.push((
                SettingId::DimAfter,
                Value::Seconds(at_most(&DIM_CHOICES, lock)),
            ));
        }
        writes.push((SettingId::LockAfter, Value::Seconds(lock)));
    }

    pub fn writes(&self, id: SettingId, value: &Value) -> Vec<(SettingId, Value)> {
        let mut writes = Vec::new();
        match (id, value) {
            (SettingId::DimAfter, Value::Seconds(dim)) => {
                if self.lock < *dim {
                    self.raise_lock(at_least(&LOCK_CHOICES, *dim), &mut writes);
                }
            }
            (SettingId::LockAfter, Value::Seconds(lock)) if *lock > self.lock => {
                self.raise_lock(*lock, &mut writes);
                return writes;
            }
            (SettingId::LockAfter, Value::Seconds(lock)) => {
                self.lower_lock(*lock, &mut writes);
                return writes;
            }
            (SettingId::SleepAfter, Value::Seconds(sleep)) if *sleep > 0 && *sleep < self.lock => {
                self.lower_lock(at_most(&LOCK_CHOICES, *sleep), &mut writes);
            }
            _ => {}
        }
        writes.push((id, value.clone()));
        writes
    }
}
