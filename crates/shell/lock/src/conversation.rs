use std::time::Instant;

use bande_l10n::{tr, trn};
use bande_pam::{Backoff, Code};

pub const MESSAGE_LIMIT: usize = 400;
const SHOWN_MESSAGES: usize = 3;

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

impl Refusal {
    pub fn from_pam(code: Code) -> Self {
        match code {
            Code::AUTH_ERR | Code::CRED_INSUFFICIENT => Self::WrongAnswer,
            Code::USER_UNKNOWN => Self::Unknown,
            Code::NEW_AUTHTOK_REQD | Code::CRED_EXPIRED | Code::AUTHTOK_EXPIRED => {
                Self::PasswordExpired
            }
            Code::ACCT_EXPIRED => Self::AccountExpired,
            Code::PERM_DENIED => Self::Denied,
            Code::MAXTRIES => Self::TooManyAttempts,
            _ => Self::Unavailable,
        }
    }

    pub fn from_pam_name(name: &str) -> Option<Self> {
        Code::named(name).map(Self::from_pam)
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

pub fn backoff_notice(backoff: &Backoff, now: Instant) -> Option<String> {
    backoff.waiting(now).map(|left| {
        let seconds = left.as_secs() + u64::from(left.subsec_nanos() > 0);
        trn!(
            "Too many attempts. Try again in 1 second.",
            "Too many attempts. Try again in {count} seconds.",
            count = seconds
        )
    })
}
