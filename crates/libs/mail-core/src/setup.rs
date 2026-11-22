use bande_l10n::tr;

use crate::discover::{Settings, Source};
use crate::error::Error;
use crate::imap::Session;
use crate::net::{Network, Server};
use crate::smtp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub domain: String,
    pub settings: Option<(Settings, Source)>,
}

impl Found {
    pub fn message(&self) -> String {
        let domain = self.domain.clone();
        match self.settings.as_ref().map(|(_, source)| *source) {
            Some(Source::Services) => tr!(
                "Mail found these servers in the DNS records of {domain}. Check that they belong to your mail provider, then sign in.",
                domain = domain
            ),
            Some(Source::Domain) => tr!(
                "Mail found these servers in the settings {domain} publishes. Check that they belong to your mail provider, then sign in.",
                domain = domain
            ),
            Some(Source::Guess) => tr!(
                "{domain} publishes no mail settings, so Mail guessed these servers. Check them against what your mail provider gives, then sign in.",
                domain = domain
            ),
            None => tr!(
                "Mail found no servers for {domain}. Enter the settings your mail provider gives for IMAP and SMTP.",
                domain = domain
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    Unreachable(String),
    Certificate(String),
    Secure(String),
    NoEncryption(String),
    WrongCredentials(String),
    Server(String, String),
    NoPassword,
    Saving(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ready(Settings),
    Failed(Settings, Problem),
}

impl Problem {
    fn of(server: &Server, error: Error) -> Self {
        let name = format!("{}:{}", server.host, server.port);
        match error {
            Error::Login(_) => Self::WrongCredentials(name),
            Error::Insecure => Self::NoEncryption(name),
            Error::Tls(text) if text.to_ascii_lowercase().contains("certificate") => {
                Self::Certificate(name)
            }
            Error::Tls(_) => Self::Secure(name),
            Error::Io(_) | Error::Timeout | Error::Closed => Self::Unreachable(name),
            other => Self::Server(name, other.to_string()),
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Unreachable(server) => tr!(
                "Mail could not reach {server}. Check the server name and port, and that this computer is online.",
                server = server.clone()
            ),
            Self::Certificate(server) => tr!(
                "The certificate of {server} could not be verified. Check that the server name is exactly the one your provider gives.",
                server = server.clone()
            ),
            Self::Secure(server) => tr!(
                "Mail could not make a secure connection to {server}. Check the port and the security setting.",
                server = server.clone()
            ),
            Self::NoEncryption(server) => tr!(
                "{server} offers no encrypted connection. Choose TLS or STARTTLS, with the port your provider gives for it.",
                server = server.clone()
            ),
            Self::WrongCredentials(server) => tr!(
                "{server} did not accept the user name or password. Check both; some providers need a separate app password for mail apps.",
                server = server.clone()
            ),
            Self::Server(server, detail) => tr!(
                "{server} answered with an error: {detail}. Check the server settings, or try again later.",
                server = server.clone(),
                detail = detail.clone()
            ),
            Self::NoPassword => {
                tr!("Mail has no saved password for this account. Enter it again.")
            }
            Self::Saving(error) => tr!(
                "Mail could not save the account: {error}",
                error = error.clone()
            ),
        }
    }
}

pub fn find(email: &str, network: &Network) -> Found {
    Found {
        domain: bande_discovery::domain_of(email).unwrap_or_default(),
        settings: crate::discover::discover(email, network),
    }
}

pub fn check(settings: &Settings, password: &str, network: &Network) -> Outcome {
    let imap = || -> Result<(), Error> {
        let mut session = Session::connect(&settings.imap, network)?;
        session.login(&settings.user, password)?;
        session.list()?;
        session.logout();
        Ok(())
    };
    let problem = match imap() {
        Err(error) => Some(Problem::of(&settings.imap, error)),
        Ok(()) => smtp::verify(&settings.smtp, network, &settings.user, password)
            .err()
            .map(|error| Problem::of(&settings.smtp, error)),
    };
    match problem {
        None => Outcome::Ready(settings.clone()),
        Some(problem) => Outcome::Failed(settings.clone(), problem),
    }
}
