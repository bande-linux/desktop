use bande_l10n::tr;

use crate::follow::{Update, Updates};
use crate::pages::{Widgets, boxed};
use crate::view::SettingsView;
use crate::widgets::{Row, Text, Tone};
use zbus::blocking::Connection;

const REMOTE_LOGIN_UNIT: &str = "ssh.service";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SecurityStatus {
    pub remote_login: bool,
}

fn read_remote_login(system: Option<&Connection>) -> bool {
    let Some(system) = system else {
        return false;
    };
    let state: zbus::Result<String> = system
        .call_method(
            Some("org.freedesktop.systemd1"),
            "/org/freedesktop/systemd1",
            Some("org.freedesktop.systemd1.Manager"),
            "GetUnitFileState",
            &(REMOTE_LOGIN_UNIT,),
        )
        .and_then(|reply| reply.body().deserialize());
    state.is_ok_and(|state| state == "enabled")
}

pub fn follow_security_status(system: Option<&Connection>, updates: &Updates) {
    updates.send(Update::Security(SecurityStatus {
        remote_login: read_remote_login(system),
    }));
}

impl SettingsView {
    pub(crate) fn security_status_rows(&mut self) -> Widgets {
        let status = self.security;
        let remote_detail = if status.remote_login {
            tr!("On. Others on your network can try to sign in over SSH.")
        } else {
            tr!("Off.")
        };
        vec![
            boxed(
                Row::new(Text::new(&tr!("Remote Login"), Tone::Primary), None)
                    .detail(Text::new(&remote_detail, Tone::Muted).caption()),
            ),
            boxed(
                Row::new(Text::new(&tr!("Apps you install"), Tone::Primary), None)
                    .detail(Text::new(&tr!("Apps from Flatpak are sandboxed. Other programs run with your full access to your files, and nothing here can contain them, so only run programs you trust."), Tone::Muted).caption()),
            ),
        ]
    }
}
