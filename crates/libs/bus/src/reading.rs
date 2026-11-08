use std::fmt::Display;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reading<T> {
    Present(T),
    #[default]
    Absent,
    Unavailable,
}

impl<T> Reading<T> {
    pub fn from_answer<E: Display>(
        service: &str,
        answer: Result<Option<T>, E>,
        hardware: impl FnOnce() -> bool,
    ) -> Self {
        match answer {
            Ok(Some(value)) => Self::Present(value),
            Ok(None) => Self::Absent,
            Err(error) if hardware() => {
                tracing::warn!("{service} is unavailable: {error}");
                Self::Unavailable
            }
            Err(error) => {
                tracing::debug!("{service} is not running and nothing it reports exists: {error}");
                Self::Absent
            }
        }
    }

    pub fn present(&self) -> Option<&T> {
        match self {
            Self::Present(value) => Some(value),
            Self::Absent | Self::Unavailable => None,
        }
    }

    pub fn present_mut(&mut self) -> Option<&mut T> {
        match self {
            Self::Present(value) => Some(value),
            Self::Absent | Self::Unavailable => None,
        }
    }

    pub fn into_present(self) -> Option<T> {
        match self {
            Self::Present(value) => Some(value),
            Self::Absent | Self::Unavailable => None,
        }
    }

    pub fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable)
    }
}
