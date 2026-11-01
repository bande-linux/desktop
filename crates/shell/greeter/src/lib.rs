mod accounts;
mod greetd;
mod users;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use accounts::{last_user_file, login_picture, remember_user, remembered_user};
pub use greetd::{
    AuthMessageType, ErrorType, Login, Rejection, Request, Response, SESSION, SESSION_ENVIRONMENT,
    Step, read_message, write_message,
};
pub use users::login_users;

const ENGLISH: &str = include_str!("../locales/en-US/bande-greeter.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
