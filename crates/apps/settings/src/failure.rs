use bande_settings_panes::SettingId;
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

fn kind(error: &zbus::Error) -> String {
    let name = match error {
        zbus::Error::MethodError(name, _, _) => name.to_string(),
        zbus::Error::FDO(error) => error.name().to_string(),
        _ => String::new(),
    };
    name.rsplit('.').next().unwrap_or_default().to_owned()
}

pub(crate) fn detail(error: &zbus::Error) -> Option<String> {
    match error {
        zbus::Error::MethodError(_, description, _) => description.clone(),
        zbus::Error::FDO(error) => error.description().map(str::to_owned),
        _ => None,
    }
}

pub(crate) fn sentence_or(
    error: &zbus::Error,
    service: &str,
    otherwise: impl FnOnce(&str) -> String,
) -> String {
    tracing::warn!("{service}: {error}");
    let kind = kind(error);
    let kind = kind.as_str();
    if REFUSED.contains(&kind) {
        L10N.tr("administrator-only")
    } else if SILENT.contains(&kind) {
        L10N.tr_args("not-responding", [("service", L10N.tr(service).into())])
    } else {
        otherwise(kind)
    }
}

pub fn sentence(error: &zbus::Error, service: &str) -> String {
    sentence_or(error, service, |_| L10N.tr("couldnt-change"))
}

pub(crate) fn failed(setting: SettingId, error: &zbus::Error, service: &str) -> Update {
    Update::Failed(Some(setting), sentence(error, service))
}

pub(crate) fn unexpected(error: impl std::fmt::Display) -> Update {
    tracing::warn!("{error}");
    Update::Failed(None, L10N.tr("couldnt-change"))
}
