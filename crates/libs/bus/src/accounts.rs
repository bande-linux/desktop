use std::path::PathBuf;

use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;

pub const ACCOUNTS: &str = "org.freedesktop.Accounts";
pub const STANDARD: i32 = 0;
pub const ADMINISTRATOR: i32 = 1;

#[zbus::proxy(
    interface = "org.freedesktop.Accounts",
    default_service = "org.freedesktop.Accounts",
    default_path = "/org/freedesktop/Accounts",
    gen_async = false,
    blocking_name = "Accounts"
)]
pub trait AccountsService {
    fn find_user_by_name(&self, name: &str) -> zbus::Result<OwnedObjectPath>;

    fn create_user(
        &self,
        name: &str,
        full_name: &str,
        account_type: i32,
    ) -> zbus::Result<OwnedObjectPath>;

    fn delete_user(&self, id: i64, remove_files: bool) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.Accounts.User",
    default_service = "org.freedesktop.Accounts",
    gen_async = false,
    blocking_name = "AccountUser"
)]
pub trait AccountsServiceUser {
    #[zbus(property)]
    fn uid(&self) -> zbus::Result<u64>;

    #[zbus(property)]
    fn real_name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn account_type(&self) -> zbus::Result<i32>;

    #[zbus(property)]
    fn icon_file(&self) -> zbus::Result<String>;

    fn set_real_name(&self, name: &str) -> zbus::Result<()>;

    fn set_icon_file(&self, file: &str) -> zbus::Result<()>;

    fn set_account_type(&self, account_type: i32) -> zbus::Result<()>;

    fn set_password(&self, crypted: &str, hint: &str) -> zbus::Result<()>;
}

pub fn account<'a>(system: &Connection, login: &str) -> zbus::Result<AccountUser<'a>> {
    let path = Accounts::new(system)?.find_user_by_name(login)?;
    AccountUser::builder(system)
        .path(path)?
        .cache_properties(CacheProperties::No)
        .build()
}

pub fn picture(system: &Connection, login: &str) -> Option<PathBuf> {
    let file = account(system, login)
        .and_then(|user| user.icon_file())
        .inspect_err(|error| tracing::info!("no account picture for {login}: {error}"))
        .ok()?;
    let file = PathBuf::from(file);
    (file.is_absolute() && file.is_file()).then_some(file)
}

pub fn passwd_name(login: &str) -> Option<String> {
    std::fs::read_to_string("/etc/passwd")
        .ok()?
        .lines()
        .find_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            (fields.first() == Some(&login))
                .then(|| fields.get(4)?.split(',').next().map(str::to_owned))
                .flatten()
        })
        .filter(|name| !name.is_empty())
}
