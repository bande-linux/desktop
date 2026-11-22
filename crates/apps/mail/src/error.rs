use std::fmt;
use std::io;

use bande_l10n::tr;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Timeout,
    Tls(String),
    Insecure,
    Protocol(String),
    Refused(String),
    Login(String),
    Store(rusqlite::Error),
    Secret(String),
    Closed,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn is_connection(&self) -> bool {
        matches!(
            self,
            Self::Io(_) | Self::Timeout | Self::Tls(_) | Self::Protocol(_) | Self::Closed
        )
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Io(error) => tr!("The connection failed: {error}", error = error.to_string()),
            Self::Timeout => tr!("The server did not answer in time."),
            Self::Tls(error) => tr!("The secure connection failed: {error}", error = error),
            Self::Insecure => tr!("The server offers no secure connection."),
            Self::Protocol(error) => {
                tr!(
                    "The server sent something unexpected: {error}",
                    error = error
                )
            }
            Self::Refused(text) => tr!("The server refused: {text}", text = text),
            Self::Login(text) => tr!("The server refused the login: {text}", text = text),
            Self::Store(error) => tr!(
                "The offline store failed: {error}",
                error = error.to_string()
            ),
            Self::Secret(error) => tr!("The keyring failed: {error}", error = error),
            Self::Closed => tr!("The server closed the connection."),
        };
        f.write_str(&text)
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Self::Timeout,
            io::ErrorKind::UnexpectedEof => Self::Closed,
            _ => Self::Io(error),
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(error)
    }
}

impl From<crate::imap::wire::WireError> for Error {
    fn from(error: crate::imap::wire::WireError) -> Self {
        Self::Protocol(error.0)
    }
}

impl From<zbus::Error> for Error {
    fn from(error: zbus::Error) -> Self {
        Self::Secret(error.to_string())
    }
}
