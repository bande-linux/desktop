use bande_bus::accounts::{AccountUser, Accounts};
use bande_lock::{User, display_name};
use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;

const FIRST_USER_ID: u32 = 1000;
const LAST_USER_ID: u32 = 59999;
const NO_LOGIN_SHELLS: [&str; 4] = [
    "/usr/sbin/nologin",
    "/sbin/nologin",
    "/bin/false",
    "/usr/bin/false",
];

fn can_log_in(shell: &str) -> bool {
    !NO_LOGIN_SHELLS.contains(&shell.trim())
}

pub fn login_users(passwd: &str) -> Vec<User> {
    passwd
        .lines()
        .filter_map(|line| {
            let fields = line.split(':').collect::<Vec<_>>();
            let [login, _, uid, _, gecos, _, shell] = fields.as_slice() else {
                return None;
            };
            let uid = uid.parse::<u32>().ok()?;
            ((FIRST_USER_ID..=LAST_USER_ID).contains(&uid) && can_log_in(shell)).then(|| User {
                uid,
                login: (*login).to_owned(),
                full_name: display_name(login, gecos),
            })
        })
        .collect()
}

fn listed_user(
    system: &Connection,
    path: zbus::zvariant::OwnedObjectPath,
) -> zbus::Result<Option<User>> {
    let user = AccountUser::builder(system)
        .path(path)?
        .cache_properties(CacheProperties::No)
        .build()?;
    if user.system_account()? || user.locked()? || !can_log_in(&user.shell()?) {
        return Ok(None);
    }
    let login = user.user_name()?;
    let uid =
        u32::try_from(user.uid()?).map_err(|error| zbus::Error::Failure(error.to_string()))?;
    let full_name = display_name(&login, &user.real_name()?);
    Ok(Some(User {
        uid,
        login,
        full_name,
    }))
}

pub fn account_users(system: &Connection) -> zbus::Result<Vec<User>> {
    let mut users = Vec::new();
    for path in Accounts::new(system)?.list_cached_users()? {
        let shown = path.to_string();
        match listed_user(system, path) {
            Ok(Some(user)) => users.push(user),
            Ok(None) => {}
            Err(error) => tracing::warn!("cannot read the account {shown}: {error}"),
        }
    }
    users.sort_by_key(|user| user.full_name.to_lowercase());
    Ok(users)
}
