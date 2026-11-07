use bande_settings_panes::{PaneId, SettingId};
use zbus::DBusError;

use crate::L10N;
use crate::follow::Update;

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

pub(crate) fn not_responding(service: &str) -> String {
    L10N.tr_args("not-responding", [("service", L10N.tr(service).into())])
}

pub(crate) fn explained_with(
    error: &zbus::Error,
    service: &str,
    otherwise: impl FnOnce(&str) -> String,
) -> String {
    tracing::warn!("{service}: {error}");
    let kind = kind(error);
    if REFUSED.contains(&kind.as_str()) {
        L10N.tr("administrator-only")
    } else if unresponsive(error) {
        not_responding(service)
    } else {
        otherwise(&kind)
    }
}

pub(crate) fn explained(error: &zbus::Error, service: &str, otherwise: &str) -> String {
    explained_with(error, service, |_| L10N.tr(otherwise))
}

pub fn sentence(error: &zbus::Error, service: &str) -> String {
    explained(error, service, "couldnt-change")
}

pub(crate) fn failed(setting: SettingId, error: &zbus::Error, service: &str) -> Update {
    Update::Failed(setting, sentence(error, service))
}

pub(crate) fn pane_failed(pane: PaneId, what: &str, error: impl std::fmt::Display) -> Update {
    tracing::warn!("{what}: {error}");
    Update::PaneFailed(pane, L10N.tr(what))
}
