use std::os::fd::OwnedFd;

use crate::{Effect, Limits, Reason, Refusal, Store};

/// What a file is opened for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    /// A folder to write in.
    Folder,
}

impl Access {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Folder => "folder",
        }
    }

    pub(crate) fn from_word(word: &str) -> Option<Self> {
        [Self::Read, Self::Folder]
            .into_iter()
            .find(|access| access.word() == word)
    }
}

/// A file its owner opened for another one.
#[derive(Debug)]
pub struct Opened {
    pub file: OwnedFd,
    pub reference: String,
    pub name: String,
    pub media_type: String,
    pub size: u64,
}

/// A file another owner wrote in a folder it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// Its name until it is registered.
    pub temporary: String,
    pub name: String,
    /// The reference of the file it replaces.
    pub replaces: Option<String>,
}

/// Opens things for other owners as descriptors, and names what they write.
pub trait Files: Store {
    /// Opens what `reference` names, and tells the effects of what it was asked for.
    fn open(
        &self,
        reference: &str,
        access: Access,
        limits: &Limits,
    ) -> Result<(Opened, Vec<Effect>), Refusal>;

    /// Checks and names what was written in `folder`, and gives the references in the same order.
    fn register(
        &self,
        _folder: OwnedFd,
        _written: Vec<Written>,
        _limits: &Limits,
    ) -> Result<Vec<String>, Refusal> {
        Err(Refusal::new(
            Reason::UnknownIntent,
            bande_l10n::tr!("Files cannot be written here"),
        ))
    }
}
