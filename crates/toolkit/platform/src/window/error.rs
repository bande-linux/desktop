use std::fmt;
use std::io;

use unbound_graphics::DrawError;
use wayland_client::backend::WaylandError;
use wayland_client::{ConnectError, DispatchError};

#[derive(Debug)]
pub enum WindowError {
    Connect(ConnectError),
    Dispatch(DispatchError),
    Wayland(WaylandError),
    Missing(&'static str),
    Io(io::Error),
    Render(DrawError),
    LockRefused,
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(error) => write!(f, "cannot connect to the compositor: {error}"),
            Self::Dispatch(error) => write!(f, "Wayland dispatch failed: {error}"),
            Self::Wayland(error) => write!(f, "Wayland connection failed: {error}"),
            Self::Missing(global) => write!(f, "the compositor does not offer {global}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Render(error) => write!(f, "{error}"),
            Self::LockRefused => write!(f, "the compositor refused to lock the session"),
        }
    }
}

impl std::error::Error for WindowError {}

impl From<DispatchError> for WindowError {
    fn from(error: DispatchError) -> Self {
        Self::Dispatch(error)
    }
}

impl From<WaylandError> for WindowError {
    fn from(error: WaylandError) -> Self {
        Self::Wayland(error)
    }
}

impl From<io::Error> for WindowError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<DrawError> for WindowError {
    fn from(error: DrawError) -> Self {
        Self::Render(error)
    }
}
