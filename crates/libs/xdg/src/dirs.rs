use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserDir {
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
    Templates,
}

impl UserDir {
    fn key(self) -> &'static str {
        match self {
            Self::Desktop => "XDG_DESKTOP_DIR",
            Self::Documents => "XDG_DOCUMENTS_DIR",
            Self::Downloads => "XDG_DOWNLOAD_DIR",
            Self::Music => "XDG_MUSIC_DIR",
            Self::Pictures => "XDG_PICTURES_DIR",
            Self::Videos => "XDG_VIDEOS_DIR",
            Self::Templates => "XDG_TEMPLATES_DIR",
        }
    }

    fn default_name(self) -> &'static str {
        match self {
            Self::Desktop => "Desktop",
            Self::Documents => "Documents",
            Self::Downloads => "Downloads",
            Self::Music => "Music",
            Self::Pictures => "Pictures",
            Self::Videos => "Videos",
            Self::Templates => "Templates",
        }
    }
}

pub fn user_dir(home: &Path, user_dirs: Option<&str>, dir: UserDir) -> PathBuf {
    let prefix = format!("{}=", dir.key());
    user_dirs
        .into_iter()
        .flat_map(str::lines)
        .filter_map(|line| line.trim().strip_prefix(prefix.as_str()))
        .filter_map(|value| value.strip_prefix('"')?.strip_suffix('"'))
        .filter_map(|value| match value.strip_prefix("$HOME") {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => {
                Some(home.join(rest.trim_start_matches('/')))
            }
            Some(_) => None,
            None => value.starts_with('/').then(|| PathBuf::from(value)),
        })
        .next_back()
        .unwrap_or_else(|| home.join(dir.default_name()))
}
