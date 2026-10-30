use std::path::{Path, PathBuf};

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

const ACCOUNTS: &str = "org.freedesktop.Accounts";
const ACCOUNTS_PATH: &str = "/org/freedesktop/Accounts";
const USER: &str = "org.freedesktop.Accounts.User";
const LAST_USER: &str = "unbound-greeter/last-user";

fn background_file(system: &Connection, login: &str) -> zbus::Result<String> {
    let accounts = Proxy::new(system, ACCOUNTS, ACCOUNTS_PATH, ACCOUNTS)?;
    let user: OwnedObjectPath = accounts.call("FindUserByName", &(login,))?;
    Proxy::new(system, ACCOUNTS, user, USER)?.get_property("BackgroundFile")
}

pub fn login_picture(login: &str) -> Option<PathBuf> {
    let system = Connection::system()
        .inspect_err(|error| tracing::info!("no system bus for wallpapers: {error}"))
        .ok()?;
    let picture = background_file(&system, login)
        .inspect_err(|error| tracing::info!("no wallpaper shared by {login}: {error}"))
        .ok()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)?;
    std::fs::File::open(&picture)
        .inspect_err(|error| tracing::info!("{}: {error}", picture.display()))
        .ok()
        .map(|_| picture)
}

pub fn last_user_file(state_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let state = match state_home.filter(|path| path.is_absolute()) {
        Some(state) => state.to_path_buf(),
        None => home?.join(".local/state"),
    };
    Some(state.join(LAST_USER))
}

pub fn remember_user(file: &Path, login: &str) -> std::io::Result<()> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    std::fs::write(file, login)
}

pub fn remembered_user(file: &Path) -> Option<String> {
    std::fs::read_to_string(file)
        .ok()
        .map(|login| login.trim().to_owned())
        .filter(|login| !login.is_empty())
}
