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

#[derive(Debug, Clone)]
pub struct Loaded {
    pub config: Arc<Config>,
    pub problems: Vec<ConfigError>,
}

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

    pub fn apply(&self, source: &str) -> Result<Loaded, ConfigError> {
        let (config, problems) = Config::load(source)?;
        let shadow = Arc::new(config);
        *self.active.write().unwrap_or_else(PoisonError::into_inner) = Arc::clone(&shadow);
        Ok(Loaded {
            config: shadow,
            problems,
        })
    }

    pub fn reload(&self, path: &Path) -> Result<Loaded, ReloadError> {
        let source = std::fs::read_to_string(path).map_err(ReloadError::Read)?;
        self.apply(&source).map_err(ReloadError::Invalid)
    }
}
