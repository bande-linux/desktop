use std::path::{Path, PathBuf};

use bande_l10n::tr;
use bande_network::{Eap, Enterprise};
use bande_toolkit::ui::{Button, SecureField, SegmentedControl, TextField, Widget};

use crate::pages::{FIELD_SIZE, Widgets, boxed, submit};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Certificate {
    Authority,
    Client,
    Key,
}

impl Certificate {
    pub fn title(self) -> String {
        match self {
            Self::Authority => tr!("Choose the Network’s Certificate Authority"),
            Self::Client => tr!("Choose Your Certificate"),
            Self::Key => tr!("Choose Your Private Key"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnterpriseField {
    Identity,
    AnonymousIdentity,
    Domain,
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

impl SettingsView {
    fn enterprise_text(
        &mut self,
        field: EnterpriseField,
        label: String,
        placeholder: String,
    ) -> Box<dyn Widget> {
        let text = match field {
            EnterpriseField::Identity => self.enterprise.identity.clone(),
            EnterpriseField::AnonymousIdentity => self.enterprise.anonymous_identity.clone(),
            EnterpriseField::Domain => self.enterprise.domain.clone(),
        };
        let mut typed = intent(&self.intents, move |text: String| {
            Intent::EnterpriseText(field, text)
        });
        let input = TextField::new(&placeholder, &text, move |text| typed(text.to_owned()))
            .without_text_checking();
        self.anchors
            .push((Anchor::Item(format!("enterprise-{field:?}")), input.id()));
        boxed(Row::new(
            Text::new(&label, Tone::Primary),
            Some(boxed(input)),
        ))
    }

    fn certificate_row(
        &mut self,
        certificate: Certificate,
        label: String,
        chosen: Option<&PathBuf>,
        unset: String,
    ) -> Box<dyn Widget> {
        let mut choose = intent(&self.intents, move |()| {
            Intent::ChooseCertificate(certificate)
        });
        let button = Button::new(&tr!("Choose…"), move || choose(()));
        self.anchors.push((
            Anchor::Item(format!("certificate-{certificate:?}")),
            button.id(),
        ));
        let shown = chosen.map_or(unset, |path| file_name(path));
        boxed(
            Row::new(Text::new(&label, Tone::Primary), Some(boxed(button)))
                .detail(Text::new(&shown, Tone::Muted).caption()),
        )
    }

    pub(crate) fn enterprise_rows(&mut self, join: fn(String) -> Intent) -> Widgets {
        let login = self.enterprise.clone();
        let labels: Vec<String> = Eap::ALL.iter().map(|eap| eap.label().to_owned()).collect();
        let selected = Eap::ALL.iter().position(|eap| *eap == login.eap);
        let method = SegmentedControl::new(
            &tr!("Authentication"),
            &labels,
            selected,
            intent(&self.intents, Intent::EnterpriseMethod),
        );
        self.anchors
            .push((Anchor::Item("enterprise-method".to_owned()), method.id()));
        let mut rows = vec![boxed(Row::new(
            Text::new(&tr!("Authentication"), Tone::Primary),
            Some(boxed(method)),
        ))];
        rows.push(self.enterprise_text(
            EnterpriseField::Identity,
            tr!("User name"),
            tr!("name@example.org"),
        ));
        if login.eap != Eap::Tls {
            rows.push(self.enterprise_text(
                EnterpriseField::AnonymousIdentity,
                tr!("Anonymous identity"),
                tr!("Optional"),
            ));
        }
        let authorities = [tr!("System Certificates"), tr!("Certificate File")];
        let authority = SegmentedControl::new(
            &tr!("Certificate authority"),
            &authorities,
            Some(usize::from(login.authority.is_some())),
            intent(&self.intents, Intent::EnterpriseAuthority),
        );
        self.anchors.push((
            Anchor::Item("enterprise-authority".to_owned()),
            authority.id(),
        ));
        rows.push(boxed(
            Row::new(
                Text::new(&tr!("Certificate authority"), Tone::Primary),
                Some(boxed(authority)),
            )
            .detail(
                Text::new(
                    &tr!("Settings checks that the network’s server is the one your organization runs"),
                    Tone::Muted,
                )
                .caption(),
            ),
        ));
        if login.authority.is_some() || self.enterprise_file {
            rows.push(self.certificate_row(
                Certificate::Authority,
                tr!("Certificate authority file"),
                login.authority.as_ref(),
                tr!("None chosen"),
            ));
        }
        let domain_hint = match login.authority {
            Some(_) => tr!("Optional"),
            None => tr!("radius.example.org"),
        };
        rows.push(self.enterprise_text(EnterpriseField::Domain, tr!("Server domain"), domain_hint));
        if login.eap == Eap::Tls {
            rows.push(self.certificate_row(
                Certificate::Client,
                tr!("Your certificate"),
                login.certificate.as_ref(),
                tr!("None chosen"),
            ));
            rows.push(self.certificate_row(
                Certificate::Key,
                tr!("Private key"),
                login.private_key.as_ref(),
                tr!("In the certificate file"),
            ));
        }
        let label = match login.eap {
            Eap::Tls => tr!("Private key password"),
            Eap::Peap | Eap::Ttls => tr!("Password"),
        };
        let field = SecureField::new(&label, FIELD_SIZE, submit(&self.intents, join));
        self.anchors.push((Anchor::Password, field.id()));
        rows.push(boxed(Row::new(
            Text::new(&label, Tone::Primary),
            Some(boxed(field)),
        )));
        rows
    }

    pub(crate) fn enterprise_login(&mut self, password: String) -> Option<Enterprise> {
        let login = Enterprise {
            password,
            ..self.enterprise.clone()
        };
        if login.is_complete() {
            return Some(login);
        }
        self.join.error = Some(if login.identity.trim().is_empty() {
            tr!("Type your user name.")
        } else if login.authority.is_none() && login.domain.trim().is_empty() {
            tr!("Type the server domain, or choose the network’s certificate authority file.")
        } else if login.eap == Eap::Tls {
            tr!("Choose your certificate.")
        } else {
            tr!("Type your password.")
        });
        None
    }

    pub(crate) fn enterprise_intent(&mut self, intent: Intent) {
        match intent {
            Intent::EnterpriseText(field, text) => match field {
                EnterpriseField::Identity => self.enterprise.identity = text,
                EnterpriseField::AnonymousIdentity => self.enterprise.anonymous_identity = text,
                EnterpriseField::Domain => self.enterprise.domain = text,
            },
            Intent::EnterpriseMethod(index) => {
                if let Some(eap) = Eap::ALL.get(index) {
                    self.enterprise.eap = *eap;
                }
            }
            Intent::EnterpriseAuthority(index) => {
                self.enterprise_file = index == 1;
                if !self.enterprise_file {
                    self.enterprise.authority = None;
                }
            }
            _ => {}
        }
    }

    pub(crate) fn choose_certificate(&mut self, certificate: Certificate, path: PathBuf) {
        match certificate {
            Certificate::Authority => self.enterprise.authority = Some(path),
            Certificate::Client => self.enterprise.certificate = Some(path),
            Certificate::Key => self.enterprise.private_key = Some(path),
        }
    }
}
