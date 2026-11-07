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

const LONGEST_LOGIN: usize = 32;

pub fn suggested_login(full_name: &str) -> String {
    let mut login = String::new();
    for character in full_name.to_lowercase().chars() {
        let folded = match character {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
            'æ' => "ae",
            'ç' | 'ć' | 'č' => "c",
            'ď' | 'đ' | 'ð' => "d",
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => "e",
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' => "i",
            'ł' | 'ľ' | 'ĺ' => "l",
            'ñ' | 'ń' | 'ň' => "n",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => "o",
            'œ' => "oe",
            'ř' | 'ŕ' => "r",
            'ś' | 'š' | 'ş' => "s",
            'ß' => "ss",
            'ť' | 'ţ' => "t",
            'þ' => "th",
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' | 'ų' => "u",
            'ý' | 'ÿ' => "y",
            'ź' | 'ż' | 'ž' => "z",
            character if character.is_ascii_alphanumeric() => {
                login.push(character);
                continue;
            }
            _ => continue,
        };
        login.push_str(folded);
    }
    login
        .trim_start_matches(|character: char| character.is_ascii_digit())
        .chars()
        .take(LONGEST_LOGIN)
        .collect()
}

pub fn valid_login(login: &str) -> bool {
    let mut characters = login.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
        && characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '_'
                || character == '-'
        })
        && login.len() <= LONGEST_LOGIN
}
