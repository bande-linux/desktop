use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

const CONFIG_FOLDERS: [&str; 4] = [
    "/etc/systemd",
    "/run/systemd",
    "/usr/local/lib/systemd",
    "/usr/lib/systemd",
];

/// The value systemd uses for `key` in its configuration `file` (such as `sleep.conf`): the first
/// main file found, then every drop-in in filename order, the last assignment winning. `None`
/// when nothing sets it or the last assignment is empty, which restores systemd's default.
pub fn setting(file: &str, key: &str) -> Option<String> {
    setting_in(&CONFIG_FOLDERS.map(Path::new), file, key)
}

fn setting_in(folders: &[&Path], file: &str, key: &str) -> Option<String> {
    let main = folders
        .iter()
        .map(|folder| folder.join(file))
        .find(|path| path.is_file());
    let mut drop_ins = BTreeMap::new();
    for folder in folders.iter().rev() {
        drop_ins.extend(drop_ins_in(&folder.join(format!("{file}.d"))));
    }
    main.into_iter()
        .chain(drop_ins.into_values())
        .rev()
        .find_map(|path| assigned(&path, key))
        .filter(|value| !value.is_empty())
}

fn drop_ins_in(folder: &Path) -> Vec<(String, PathBuf)> {
    match fs::read_dir(folder) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "conf")
            })
            .filter_map(|path| Some((path.file_name()?.to_str()?.to_owned(), path)))
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            tracing::warn!("cannot read {}: {error}", folder.display());
            Vec::new()
        }
    }
}

fn assigned(path: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(path)
        .inspect_err(|error| tracing::warn!("cannot read {}: {error}", path.display()))
        .ok()?;
    text.lines()
        .rev()
        .map(str::trim)
        .filter(|line| !line.starts_with(['#', ';']))
        .filter_map(|line| line.split_once('='))
        .filter(|(name, _)| name.trim() == key)
        .map(|(_, value)| value.trim().to_owned())
        .next()
}

/// Reads a systemd time span such as `3h`, `2h 30min` or `90` (seconds); `None` for `infinity`
/// or text systemd would reject.
pub fn timespan(text: &str) -> Option<Duration> {
    let mut rest = text.trim();
    if rest.is_empty() || rest == "infinity" {
        return None;
    }
    let mut total = 0.0;
    while !rest.is_empty() {
        let number_ends = rest
            .find(|character: char| !character.is_ascii_digit() && character != '.')
            .unwrap_or(rest.len());
        let number: f64 = rest[..number_ends].parse().ok()?;
        rest = rest[number_ends..].trim_start();
        let unit_ends = rest
            .find(|character: char| !character.is_alphabetic())
            .unwrap_or(rest.len());
        let seconds = match &rest[..unit_ends] {
            "" | "s" | "sec" | "second" | "seconds" => 1.0,
            "ms" | "msec" => 1e-3,
            "us" | "usec" | "µs" => 1e-6,
            "m" | "min" | "minute" | "minutes" => 60.0,
            "h" | "hr" | "hour" | "hours" => 3_600.0,
            "d" | "day" | "days" => 86_400.0,
            "w" | "week" | "weeks" => 604_800.0,
            "M" | "month" | "months" => 2_629_800.0,
            "y" | "year" | "years" => 31_557_600.0,
            _ => return None,
        };
        total += number * seconds;
        rest = rest[unit_ends..].trim_start();
    }
    Duration::try_from_secs_f64(total).ok()
}

/// Reads a systemd boolean (`yes`, `no`, `true`, `off`, `1`…).
pub fn boolean(text: &str) -> Option<bool> {
    match text.to_ascii_lowercase().as_str() {
        "1" | "yes" | "y" | "true" | "t" | "on" => Some(true),
        "0" | "no" | "n" | "false" | "f" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_ins_override_the_main_file_and_etc_masks_a_vendor_drop_in() {
        let root = tempfile::tempdir().unwrap();
        let etc = root.path().join("etc");
        let vendor = root.path().join("usr");
        let write = |path: PathBuf, text: &str| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let folders = [etc.as_path(), vendor.as_path()];
        write(etc.join("sleep.conf"), "[Sleep]\nHibernateDelaySec=1h\n");
        write(vendor.join("sleep.conf"), "[Sleep]\nHibernateDelaySec=9h\n");
        assert_eq!(
            setting_in(&folders, "sleep.conf", "HibernateDelaySec").as_deref(),
            Some("1h")
        );
        write(
            vendor.join("sleep.conf.d/50-bande.conf"),
            "[Sleep]\nHibernateDelaySec=3h\n",
        );
        assert_eq!(
            setting_in(&folders, "sleep.conf", "HibernateDelaySec").as_deref(),
            Some("3h")
        );
        write(
            etc.join("sleep.conf.d/60-local.conf"),
            "[Sleep]\nHibernateDelaySec = 2h\n",
        );
        assert_eq!(
            setting_in(&folders, "sleep.conf", "HibernateDelaySec").as_deref(),
            Some("2h")
        );
        write(
            etc.join("sleep.conf.d/50-bande.conf"),
            "[Sleep]\n#HibernateDelaySec=5h\n",
        );
        write(
            etc.join("sleep.conf.d/60-local.conf"),
            "[Sleep]\nHibernateDelaySec=\n",
        );
        assert_eq!(
            setting_in(&folders, "sleep.conf", "HibernateDelaySec"),
            None
        );

        assert_eq!(timespan("2h 30min"), Some(Duration::from_secs(9_000)));
        assert_eq!(timespan("1d3h"), Some(Duration::from_secs(97_200)));
        assert_eq!(timespan("90"), Some(Duration::from_secs(90)));
        assert_eq!(timespan("infinity"), None);
        assert_eq!(timespan("3 parsecs"), None);
    }
}
