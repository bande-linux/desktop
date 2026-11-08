use bande_l10n::{Text, text, tr};

use crate::follow::Update;
use bande_settings_panes::{PaneId, SettingId};
use zbus::DBusError;

const REFUSED: [&str; 5] = [
    "RefusedByPolicy",
    "InteractiveAuthorizationRequired",
    "AccessDenied",
    "NotAuthorized",
    "AuthFailed",
];
const SILENT: [&str; 5] = [
    "ServiceUnknown",
    "NoReply",
    "TimedOut",
    "Timeout",
    "NameHasNoOwner",
];

pub(crate) fn kind(error: &zbus::Error) -> String {
    let name = match error {
        zbus::Error::MethodError(name, _, _) => name.to_string(),
        zbus::Error::FDO(error) => error.name().to_string(),
        _ => String::new(),
    };
    name.rsplit('.').next().unwrap_or_default().to_owned()
}

pub(crate) fn unresponsive(error: &zbus::Error) -> bool {
    SILENT.contains(&kind(error).as_str())
}

pub(crate) fn not_responding(service: Text) -> String {
    tr!(
        "{service} isn't responding. Try again in a moment.",
        service = service.tr()
    )
}

pub(crate) fn explained_with(
    error: &zbus::Error,
    service: Text,
    otherwise: impl FnOnce(&str) -> String,
) -> String {
    tracing::warn!("{service:?}: {error}");
    let kind = kind(error);
    if REFUSED.contains(&kind.as_str()) {
        tr!("Only an administrator can change this.")
    } else if unresponsive(error) {
        not_responding(service)
    } else {
        otherwise(&kind)
    }
}

pub(crate) fn explained(error: &zbus::Error, service: Text, otherwise: Text) -> String {
    explained_with(error, service, |_| otherwise.tr())
}

pub fn sentence(error: &zbus::Error, service: Text) -> String {
    explained(error, service, text!("Couldn't make this change."))
}

pub(crate) fn failed(setting: SettingId, error: &zbus::Error, service: Text) -> Update {
    Update::Failed(setting, sentence(error, service))
}

pub(crate) fn pane_failed(pane: PaneId, what: Text, error: impl std::fmt::Display) -> Update {
    tracing::warn!("{what:?}: {error}");
    Update::PaneFailed(pane, what.tr())
}
