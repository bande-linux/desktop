mod accounts;
mod greetd;
mod users;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use accounts::{last_user_file, login_picture, remember_user, remembered_user};
pub use greetd::{
    AuthMessageType, ErrorType, Login, Rejection, Request, Response, SESSION, SESSION_ENVIRONMENT,
    Step, read_message, write_message,
};
pub use users::login_users;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-greeter.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [include_str!("main.rs")];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
