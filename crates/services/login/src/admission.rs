use std::path::Path;

const DEFAULT_SHELL: &str = "/bin/sh";
const SHELLS_WHEN_UNLISTED: [&str; 2] = ["/bin/sh", "/bin/csh"];

/// The shell an account logs in with: its own, or `/bin/sh` when it names none.
pub fn login_shell(shell: &Path) -> &Path {
    match shell.as_os_str().is_empty() {
        true => Path::new(DEFAULT_SHELL),
        false => shell,
    }
}

/// Why an account that PAM let through still may not have a desktop session, if it may not:
/// root, or a login shell that `/etc/shells` (its text is `shells`, `None` when there is no such
/// file) does not list, such as `nologin`.
pub fn refusal(uid: u32, shell: &Path, shells: Option<&str>) -> Option<&'static str> {
    if uid == 0 {
        return Some("root may not sign in to the desktop");
    }
    let shell = login_shell(shell);
    let listed = match shells {
        Some(shells) => shells
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .any(|line| Path::new(line) == shell),
        None => SHELLS_WHEN_UNLISTED
            .iter()
            .any(|listed| Path::new(listed) == shell),
    };
    match listed {
        true => None,
        false => Some("the account's login shell is not in /etc/shells"),
    }
}
