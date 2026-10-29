use std::fmt;
use std::io;
use std::path::Path;
use std::sync::{Arc, PoisonError, RwLock};

use crate::{Config, ConfigError};

#[derive(Debug)]
pub enum ReloadError {
    Read(io::Error),
    Invalid(ConfigError),
}

impl fmt::Display for ReloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(f, "cannot read the configuration: {error}"),
            Self::Invalid(error) => write!(f, "invalid configuration at {error}"),
        }
    }
}

impl std::error::Error for ReloadError {}

#[derive(Debug, Default)]
pub struct ConfigStore {
    active: RwLock<Arc<Config>>,
}

impl ConfigStore {
    pub fn new(config: Config) -> Self {
        Self {
            active: RwLock::new(Arc::new(config)),
        }
    }

    pub fn current(&self) -> Arc<Config> {
        Arc::clone(&self.active.read().unwrap_or_else(PoisonError::into_inner))
    }

    pub fn apply(&self, source: &str) -> Result<Arc<Config>, ConfigError> {
        let shadow = Arc::new(Config::parse(source)?);
        *self.active.write().unwrap_or_else(PoisonError::into_inner) = Arc::clone(&shadow);
        Ok(shadow)
    }

    pub fn reload(&self, path: &Path) -> Result<Arc<Config>, ReloadError> {
        let source = std::fs::read_to_string(path).map_err(ReloadError::Read)?;
        self.apply(&source).map_err(ReloadError::Invalid)
    }
}
