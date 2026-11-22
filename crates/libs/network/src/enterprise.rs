use std::collections::HashMap;
use std::path::{Path, PathBuf};

use zbus::zvariant::{OwnedValue, Value};

use crate::connections::text;

pub(crate) const ENTERPRISE: &str = "802-1x";
const AGENT_OWNED: u32 = 0x1;
const FILE_SCHEME: &str = "file://";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Eap {
    #[default]
    Peap,
    Ttls,
    Tls,
}

impl Eap {
    pub const ALL: [Self; 3] = [Self::Peap, Self::Ttls, Self::Tls];

    fn name(self) -> &'static str {
        match self {
            Self::Peap => "peap",
            Self::Ttls => "ttls",
            Self::Tls => "tls",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Peap => "PEAP",
            Self::Ttls => "TTLS",
            Self::Tls => "TLS",
        }
    }

    fn inner(self) -> Option<&'static str> {
        match self {
            Self::Peap => Some("mschapv2"),
            Self::Ttls => Some("pap"),
            Self::Tls => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Enterprise {
    pub eap: Eap,
    pub identity: String,
    pub anonymous_identity: String,
    pub authority: Option<PathBuf>,
    pub domain: String,
    pub password: String,
    pub certificate: Option<PathBuf>,
    pub private_key: Option<PathBuf>,
}

fn file_value(path: &Path) -> Value<'static> {
    let mut bytes = FILE_SCHEME.as_bytes().to_vec();
    bytes.extend_from_slice(path.as_os_str().as_encoded_bytes());
    bytes.push(0);
    Value::from(bytes)
}

fn file_of(settings: &HashMap<String, OwnedValue>, key: &str) -> Option<PathBuf> {
    let bytes = Vec::<u8>::try_from(settings.get(key)?.try_clone().ok()?).ok()?;
    let path = bytes.strip_prefix(FILE_SCHEME.as_bytes())?;
    let path = path.strip_suffix(&[0]).unwrap_or(path);
    Some(PathBuf::from(String::from_utf8_lossy(path).into_owned()))
}

impl Enterprise {
    pub fn is_complete(&self) -> bool {
        let trusted = self.authority.is_some() || !self.domain.trim().is_empty();
        let credentials = match self.eap {
            Eap::Tls => self.certificate.is_some(),
            Eap::Peap | Eap::Ttls => !self.password.is_empty(),
        };
        trusted && credentials && !self.identity.trim().is_empty()
    }

    pub(crate) fn settings(&self) -> Vec<(&'static str, Value<'static>)> {
        let mut settings = vec![
            ("eap", Value::from(vec![self.eap.name()])),
            ("identity", Value::from(self.identity.trim().to_owned())),
        ];
        if !self.anonymous_identity.trim().is_empty() {
            settings.push((
                "anonymous-identity",
                Value::from(self.anonymous_identity.trim().to_owned()),
            ));
        }
        match &self.authority {
            Some(authority) => settings.push(("ca-cert", file_value(authority))),
            None => settings.push(("system-ca-certs", Value::from(true))),
        }
        if !self.domain.trim().is_empty() {
            settings.push((
                "domain-suffix-match",
                Value::from(self.domain.trim().to_owned()),
            ));
        }
        if let Some(inner) = self.eap.inner() {
            settings.push(("phase2-auth", Value::from(inner)));
            settings.push(("password", Value::from(self.password.clone())));
            settings.push(("password-flags", Value::from(AGENT_OWNED)));
        }
        if let (Eap::Tls, Some(certificate)) = (self.eap, &self.certificate) {
            let key = self.private_key.as_ref().unwrap_or(certificate);
            settings.push(("client-cert", file_value(certificate)));
            settings.push(("private-key", file_value(key)));
            settings.push(("private-key-password", Value::from(self.password.clone())));
            settings.push(("private-key-password-flags", Value::from(AGENT_OWNED)));
        }
        settings
    }

    pub(crate) fn owned_settings(&self) -> HashMap<String, OwnedValue> {
        self.settings()
            .into_iter()
            .filter_map(|(key, value)| Some((key.to_owned(), OwnedValue::try_from(value).ok()?)))
            .collect()
    }

    pub(crate) fn of(settings: &HashMap<String, OwnedValue>) -> Option<Self> {
        let methods = Vec::<String>::try_from(settings.get("eap")?.try_clone().ok()?).ok()?;
        let eap = match methods.first()?.as_str() {
            "peap" => Eap::Peap,
            "ttls" => Eap::Ttls,
            "tls" => Eap::Tls,
            _ => return None,
        };
        Some(Self {
            eap,
            identity: text(settings, "identity").unwrap_or_default(),
            anonymous_identity: text(settings, "anonymous-identity").unwrap_or_default(),
            authority: file_of(settings, "ca-cert"),
            domain: text(settings, "domain-suffix-match").unwrap_or_default(),
            password: String::new(),
            certificate: file_of(settings, "client-cert"),
            private_key: file_of(settings, "private-key"),
        })
    }
}
