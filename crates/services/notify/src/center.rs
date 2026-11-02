use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::Picture;

pub const BANNER_DURATION: Duration = Duration::from_secs(5);
pub const BANNER_LINGER: Duration = Duration::from_secs(2);
pub const MAX_NOTIFICATIONS: usize = 200;

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
    pub picture: Option<Picture>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Banners {
    #[default]
    Temporary,
    Persistent,
    Off,
}

impl Banners {
    pub const ALL: [Self; 3] = [Self::Temporary, Self::Persistent, Self::Off];

    pub fn keyword(self) -> &'static str {
        match self {
            Self::Temporary => "temporary",
            Self::Persistent => "persistent",
            Self::Off => "off",
        }
    }

    pub fn parse(keyword: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|banners| banners.keyword() == keyword)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppRule {
    pub allow: bool,
    pub banners: Banners,
    pub in_center: bool,
    pub sound: bool,
}

impl Default for AppRule {
    fn default() -> Self {
        Self {
            allow: true,
            banners: Banners::Temporary,
            in_center: true,
            sound: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub group: String,
    pub name: String,
    pub rule: AppRule,
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
    pub screen_shared: bool,
    pub full_screen: bool,
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
    screen_shared: bool,
    full_screen: bool,
    banners_held: bool,
    apps: BTreeMap<String, App>,
}

impl Center {
    pub fn apps(&self) -> impl Iterator<Item = &App> {
        self.apps.values()
    }

    pub fn remember(&mut self, app: App) {
        self.apps.insert(app.group.clone(), app);
    }

    pub fn set_rule(&mut self, group: &str, rule: AppRule) -> bool {
        match self.apps.get_mut(group) {
            Some(app) => {
                app.rule = rule;
                true
            }
            None => false,
        }
    }

    fn rule_for(&mut self, notification: &Notification) -> AppRule {
        let group = notification.group().to_owned();
        let app = self.apps.entry(group.clone()).or_insert_with(|| App {
            group,
            name: notification.app_name.clone(),
            rule: AppRule::default(),
        });
        if app.name.is_empty() {
            app.name.clone_from(&notification.app_name);
        }
        app.rule
    }

    pub fn post(&mut self, mut notification: Notification, timeout: Timeout, now: Instant) -> u32 {
        if notification.id == 0 {
            notification.id = self.fresh_id();
        }
        let id = notification.id;
        let rule = self.rule_for(&notification);
        if !rule.allow {
            self.entries.remove(&id);
            return id;
        }
        notification.transient |= !rule.in_center;
        let shown = (!self.quiet() && rule.banners != Banners::Off)
            || notification.urgency == Urgency::Critical;
        let banner = match (shown, timeout) {
            (false, _) => Banner::Hidden,
            (true, Timeout::ServerDefault) if rule.banners == Banners::Persistent => {
                Banner::UntilDismissed
            }
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

    pub fn plays_sound(&self, id: u32) -> bool {
        self.get(id).is_some_and(|notification| {
            self.apps
                .get(notification.group())
                .is_none_or(|app| app.rule.sound)
        })
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
        self.hide_quieted_banners();
    }

    pub fn do_not_disturb(&self) -> bool {
        self.do_not_disturb
    }

    pub fn set_screen_shared(&mut self, shared: bool) {
        self.screen_shared = shared;
        self.hide_quieted_banners();
    }

    pub fn set_full_screen(&mut self, full_screen: bool) {
        self.full_screen = full_screen;
        self.hide_quieted_banners();
    }

    pub fn quiet(&self) -> bool {
        self.do_not_disturb || self.screen_shared || self.full_screen
    }

    fn hide_quieted_banners(&mut self) {
        if !self.quiet() {
            return;
        }
        for entry in self.entries.values_mut() {
            if entry.notification.urgency != Urgency::Critical {
                entry.banner = Banner::Hidden;
            }
        }
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
        let mut expired: Vec<u32> = self
            .entries
            .values()
            .filter(|entry| !self.paused(entry))
            .filter(|entry| match entry.expires_at {
                Some(deadline) => deadline <= now,
                None => entry.notification.transient && entry.banner == Banner::Hidden,
            })
            .map(|entry| entry.notification.id)
            .collect();
        for id in &expired {
            self.entries.remove(id);
        }
        if self.entries.len() > MAX_NOTIFICATIONS {
            let mut oldest: Vec<(u64, u32)> = self
                .entries
                .values()
                .map(|entry| (entry.sequence, entry.notification.id))
                .collect();
            oldest.sort_unstable();
            for (_, id) in oldest.iter().take(self.entries.len() - MAX_NOTIFICATIONS) {
                self.entries.remove(id);
                expired.push(*id);
            }
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
            screen_shared: self.screen_shared,
            full_screen: self.full_screen,
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
