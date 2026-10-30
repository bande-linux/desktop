use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub const BANNER_DURATION: Duration = Duration::from_secs(5);
pub const BANNER_LINGER: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<(String, String)>,
    pub urgency: Urgency,
    pub desktop_entry: Option<String>,
    pub resident: bool,
    pub transient: bool,
}

impl Notification {
    pub fn group(&self) -> &str {
        self.desktop_entry.as_deref().unwrap_or(&self.app_name)
    }

    fn offers(&self, action_key: &str) -> bool {
        self.actions.iter().any(|(key, _)| key == action_key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timeout {
    ServerDefault,
    Never,
    After(Duration),
}

impl Timeout {
    pub fn from_milliseconds(expire_timeout: i32) -> Self {
        match expire_timeout {
            0 => Self::Never,
            milliseconds if milliseconds > 0 => {
                Self::After(Duration::from_millis(milliseconds.unsigned_abs().into()))
            }
            _ => Self::ServerDefault,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Banner {
    Hidden,
    Until(Instant),
    UntilDismissed,
}

#[derive(Debug)]
struct Entry {
    notification: Notification,
    sequence: u64,
    banner: Banner,
    expires_at: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stack {
    pub group: String,
    pub notifications: Vec<Notification>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Presentation {
    pub banners: Vec<Stack>,
    pub center: Vec<Stack>,
    pub do_not_disturb: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invoked {
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenBanner {
    pub closed: bool,
}

#[derive(Debug, Default)]
pub struct Center {
    last_id: u32,
    sequence: u64,
    entries: BTreeMap<u32, Entry>,
    do_not_disturb: bool,
    banners_held: bool,
}

impl Center {
    pub fn post(&mut self, mut notification: Notification, timeout: Timeout, now: Instant) -> u32 {
        if notification.id == 0 {
            notification.id = self.fresh_id();
        }
        let id = notification.id;
        let shown = !self.do_not_disturb || notification.urgency == Urgency::Critical;
        let banner = match (shown, timeout) {
            (false, _) => Banner::Hidden,
            (true, Timeout::After(after)) => Banner::Until(now + after),
            (true, _) if notification.urgency == Urgency::Critical => Banner::UntilDismissed,
            (true, _) => Banner::Until(now + BANNER_DURATION),
        };
        let expires_at = match timeout {
            Timeout::After(after) => Some(now + after),
            _ if notification.transient => match banner {
                Banner::Until(deadline) => Some(deadline),
                Banner::UntilDismissed => None,
                Banner::Hidden => Some(now + BANNER_DURATION),
            },
            _ => None,
        };
        self.sequence += 1;
        self.entries.insert(
            id,
            Entry {
                notification,
                sequence: self.sequence,
                banner,
                expires_at,
            },
        );
        id
    }

    pub fn get(&self, id: u32) -> Option<&Notification> {
        self.entries.get(&id).map(|entry| &entry.notification)
    }

    pub fn notifications(&self) -> impl Iterator<Item = &Notification> {
        self.entries.values().map(|entry| &entry.notification)
    }

    pub fn close(&mut self, id: u32) -> bool {
        self.entries.remove(&id).is_some()
    }

    pub fn invoke(&mut self, id: u32, action_key: &str) -> Option<Invoked> {
        let entry = self.entries.get(&id)?;
        if !entry.notification.offers(action_key) {
            return None;
        }
        let closed = !entry.notification.resident;
        if closed {
            self.entries.remove(&id);
        }
        Some(Invoked { closed })
    }

    pub fn clear(&mut self, group: &str) -> Vec<u32> {
        let cleared: Vec<u32> = self
            .entries
            .values()
            .filter(|entry| entry.notification.group() == group)
            .map(|entry| entry.notification.id)
            .collect();
        for id in &cleared {
            self.entries.remove(id);
        }
        cleared
    }

    pub fn set_do_not_disturb(&mut self, enabled: bool) {
        self.do_not_disturb = enabled;
        if enabled {
            for entry in self.entries.values_mut() {
                if entry.notification.urgency != Urgency::Critical {
                    entry.banner = Banner::Hidden;
                }
            }
        }
    }

    pub fn do_not_disturb(&self) -> bool {
        self.do_not_disturb
    }

    pub fn clear_all(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.entries).into_keys().collect()
    }

    pub fn hide_banner(&mut self, id: u32) -> Option<HiddenBanner> {
        let entry = self.entries.get_mut(&id)?;
        if entry.banner == Banner::Hidden {
            return None;
        }
        if entry.notification.transient {
            self.entries.remove(&id);
            return Some(HiddenBanner { closed: true });
        }
        entry.banner = Banner::Hidden;
        Some(HiddenBanner { closed: false })
    }

    pub fn hold_banners(&mut self, held: bool, now: Instant) {
        if self.banners_held && !held {
            let linger = now + BANNER_LINGER;
            for entry in self.entries.values_mut() {
                if let Banner::Until(deadline) = entry.banner {
                    let deadline = deadline.max(linger);
                    entry.banner = Banner::Until(deadline);
                    entry.expires_at = entry.expires_at.map(|expiry| expiry.max(deadline));
                }
            }
        }
        self.banners_held = held;
    }

    fn paused(&self, entry: &Entry) -> bool {
        self.banners_held && entry.banner != Banner::Hidden
    }

    pub fn advance(&mut self, now: Instant) -> Vec<u32> {
        let expired: Vec<u32> = self
            .entries
            .values()
            .filter(|entry| !self.paused(entry))
            .filter(|entry| entry.expires_at.is_some_and(|deadline| deadline <= now))
            .map(|entry| entry.notification.id)
            .collect();
        for id in &expired {
            self.entries.remove(id);
        }
        let held = self.banners_held;
        for entry in self.entries.values_mut() {
            if !held && matches!(entry.banner, Banner::Until(deadline) if deadline <= now) {
                entry.banner = Banner::Hidden;
            }
        }
        expired
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.entries
            .values()
            .filter(|entry| !self.paused(entry))
            .flat_map(|entry| {
                let banner = match entry.banner {
                    Banner::Until(deadline) => Some(deadline),
                    _ => None,
                };
                [banner, entry.expires_at]
            })
            .flatten()
            .min()
    }

    pub fn presentation(&self) -> Presentation {
        Presentation {
            banners: self.stacks(|entry| entry.banner != Banner::Hidden),
            center: self.stacks(|entry| !entry.notification.transient),
            do_not_disturb: self.do_not_disturb,
        }
    }

    fn stacks(&self, include: impl Fn(&Entry) -> bool) -> Vec<Stack> {
        let mut entries: Vec<&Entry> = self.entries.values().filter(|e| include(e)).collect();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.sequence));
        let mut stacks: Vec<Stack> = Vec::new();
        for entry in entries {
            let group = entry.notification.group();
            match stacks.iter_mut().find(|stack| stack.group == group) {
                Some(stack) => stack.notifications.push(entry.notification.clone()),
                None => stacks.push(Stack {
                    group: group.to_owned(),
                    notifications: vec![entry.notification.clone()],
                }),
            }
        }
        stacks
    }

    fn fresh_id(&mut self) -> u32 {
        loop {
            self.last_id = self.last_id.wrapping_add(1);
            if self.last_id != 0 && !self.entries.contains_key(&self.last_id) {
                return self.last_id;
            }
        }
    }
}
