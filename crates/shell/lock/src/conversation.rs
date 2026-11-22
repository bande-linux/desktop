use std::time::{Duration, Instant};

use bande_l10n::{tr, trn};

pub const MESSAGE_LIMIT: usize = 400;
const SHOWN_MESSAGES: usize = 3;
const FREE_ATTEMPTS: u32 = 4;
const FIRST_WAIT: Duration = Duration::from_secs(5);
const LONGEST_WAIT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub text: String,
    pub visible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    WrongAnswer,
    PasswordExpired,
    AccountExpired,
    Denied,
    TooManyAttempts,
    Unknown,
    Unavailable,
}

const PAM_PERM_DENIED: i32 = 6;
const PAM_AUTH_ERR: i32 = 7;
const PAM_CRED_INSUFFICIENT: i32 = 8;
const PAM_AUTHINFO_UNAVAIL: i32 = 9;
const PAM_USER_UNKNOWN: i32 = 10;
const PAM_MAXTRIES: i32 = 11;
const PAM_NEW_AUTHTOK_REQD: i32 = 12;
const PAM_ACCT_EXPIRED: i32 = 13;
const PAM_CRED_EXPIRED: i32 = 16;
const PAM_AUTHTOK_EXPIRED: i32 = 27;

const PAM_NAMES: [(&str, i32); 10] = [
    ("PERM_DENIED", PAM_PERM_DENIED),
    ("AUTH_ERR", PAM_AUTH_ERR),
    ("CRED_INSUFFICIENT", PAM_CRED_INSUFFICIENT),
    ("AUTHINFO_UNAVAIL", PAM_AUTHINFO_UNAVAIL),
    ("USER_UNKNOWN", PAM_USER_UNKNOWN),
    ("MAXTRIES", PAM_MAXTRIES),
    ("NEW_AUTHTOK_REQD", PAM_NEW_AUTHTOK_REQD),
    ("ACCT_EXPIRED", PAM_ACCT_EXPIRED),
    ("CRED_EXPIRED", PAM_CRED_EXPIRED),
    ("AUTHTOK_EXPIRED", PAM_AUTHTOK_EXPIRED),
];

impl Refusal {
    pub fn from_pam(code: i32) -> Self {
        match code {
            PAM_AUTH_ERR | PAM_CRED_INSUFFICIENT => Self::WrongAnswer,
            PAM_USER_UNKNOWN => Self::Unknown,
            PAM_NEW_AUTHTOK_REQD | PAM_CRED_EXPIRED | PAM_AUTHTOK_EXPIRED => Self::PasswordExpired,
            PAM_ACCT_EXPIRED => Self::AccountExpired,
            PAM_PERM_DENIED => Self::Denied,
            PAM_MAXTRIES => Self::TooManyAttempts,
            _ => Self::Unavailable,
        }
    }

    pub fn from_pam_name(name: &str) -> Option<Self> {
        PAM_NAMES
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, code)| Self::from_pam(*code))
    }

    pub fn counts_as_attempt(self) -> bool {
        matches!(self, Self::WrongAnswer | Self::Unknown)
    }

    pub fn message(self) -> String {
        match self {
            Self::WrongAnswer => tr!("Incorrect password. Try again."),
            Self::Unknown => tr!("That user name or password is not right. Try again."),
            Self::PasswordExpired => tr!("Your password has expired. Choose a new one."),
            Self::AccountExpired => {
                tr!("This account has expired. Ask your administrator to renew it.")
            }
            Self::Denied => tr!("This account cannot sign in here."),
            Self::TooManyAttempts => tr!("Too many attempts. Wait a moment and try again."),
            Self::Unavailable => tr!("Signing in is not available right now."),
        }
    }
}

pub fn clip_message(text: &str) -> String {
    let trimmed = text.trim();
    match trimmed.char_indices().nth(MESSAGE_LIMIT) {
        Some((end, _)) => format!("{}…", &trimmed[..end]),
        None => trimmed.to_owned(),
    }
}

#[derive(Debug, Default)]
pub struct Messages(Vec<String>);

impl Messages {
    pub fn push(&mut self, message: String) {
        if message.is_empty() || self.0.last() == Some(&message) {
            return;
        }
        self.0.push(message);
        let extra = self.0.len().saturating_sub(SHOWN_MESSAGES);
        self.0.drain(..extra);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn text(&self) -> Option<String> {
        (!self.0.is_empty()).then(|| self.0.join("\n"))
    }
}

#[derive(Debug, Default)]
pub struct Backoff {
    failures: u32,
    until: Option<Instant>,
}

impl Backoff {
    pub fn fail(&mut self, now: Instant) {
        self.failures += 1;
        let over = self.failures.saturating_sub(FREE_ATTEMPTS);
        if over > 0 {
            let wait = FIRST_WAIT
                .saturating_mul(1 << (over - 1).min(8))
                .min(LONGEST_WAIT);
            self.until = Some(now + wait);
        }
    }

    pub fn succeed(&mut self) {
        *self = Self::default();
    }

    pub fn until(&self) -> Option<Instant> {
        self.until
    }

    pub fn waiting(&self, now: Instant) -> Option<Duration> {
        self.until
            .map(|until| until.saturating_duration_since(now))
            .filter(|left| !left.is_zero())
    }

    pub fn notice(&self, now: Instant) -> Option<String> {
        self.waiting(now).map(|left| {
            let seconds = left.as_secs() + u64::from(left.subsec_nanos() > 0);
            trn!(
                "Too many attempts. Try again in 1 second.",
                "Too many attempts. Try again in {count} seconds.",
                count = seconds
            )
        })
    }
}
