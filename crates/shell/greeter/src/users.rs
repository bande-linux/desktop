use bande_lock::{User, display_name};

const FIRST_USER_ID: u32 = 1000;
const LAST_USER_ID: u32 = 59999;
const NO_LOGIN_SHELLS: [&str; 3] = ["/usr/sbin/nologin", "/sbin/nologin", "/bin/false"];

pub fn login_users(passwd: &str) -> Vec<User> {
    passwd
        .lines()
        .filter_map(|line| {
            let fields = line.split(':').collect::<Vec<_>>();
            let [login, _, uid, _, gecos, _, shell] = fields.as_slice() else {
                return None;
            };
            let uid = uid.parse::<u32>().ok()?;
            ((FIRST_USER_ID..=LAST_USER_ID).contains(&uid) && !NO_LOGIN_SHELLS.contains(shell))
                .then(|| User {
                    uid,
                    login: (*login).to_owned(),
                    full_name: display_name(login, gecos),
                })
        })
        .collect()
}
