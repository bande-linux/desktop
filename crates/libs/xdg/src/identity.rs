use std::path::Path;

use crate::entry::DesktopEntry;

const LAUNCHERS: &[&str] = &[
    "bash", "dash", "env", "fish", "flatpak", "java", "node", "perl", "pkexec", "python",
    "python3", "ruby", "sh", "sudo", "zsh",
];

impl DesktopEntry {
    pub fn program(&self) -> Option<String> {
        if self.terminal {
            return None;
        }
        let command = self.command().ok()?;
        let program = Path::new(command.first()?).file_name()?.to_str()?;
        (!LAUNCHERS.contains(&program)).then(|| program.to_owned())
    }
}

fn last_part(id: &str) -> Option<&str> {
    id.rsplit_once('.').map(|(_, last)| last)
}

pub fn find_app<'a>(entries: &'a [DesktopEntry], app_id: &str) -> Option<&'a DesktopEntry> {
    if app_id.is_empty() {
        return None;
    }
    let same = |name: &str| name.eq_ignore_ascii_case(app_id);
    let reverse_dns = |entry: &DesktopEntry| {
        let id = entry.app_id();
        last_part(id).is_some_and(same)
            || (!id.contains('.')
                && last_part(app_id).is_some_and(|last| last.eq_ignore_ascii_case(id)))
    };
    let rules: [&dyn Fn(&DesktopEntry) -> bool; 5] = [
        &|entry| entry.app_id() == app_id,
        &|entry| same(entry.app_id()),
        &|entry| entry.startup_wm_class.as_deref().is_some_and(same),
        &reverse_dns,
        &|entry| entry.program().as_deref().is_some_and(same),
    ];
    rules
        .iter()
        .find_map(|rule| entries.iter().find(|entry| rule(entry)))
}

pub fn app_name(entries: &[DesktopEntry], app_id: &str) -> String {
    find_app(entries, app_id).map_or_else(
        || last_part(app_id).unwrap_or(app_id).to_owned(),
        |entry| entry.name.clone(),
    )
}

pub fn running_app<'a>(
    entries: &[DesktopEntry],
    running: &'a [String],
    app_id: &str,
) -> Option<&'a str> {
    let running = || running.iter().map(String::as_str);
    running().find(|window| *window == app_id).or_else(|| {
        let wanted = find_app(entries, app_id)?.app_id();
        running().find(|window| find_app(entries, window).map(DesktopEntry::app_id) == Some(wanted))
    })
}
