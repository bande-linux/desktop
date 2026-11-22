use std::rc::Rc;

use bande_l10n::tr;
use bande_toolkit::ui::Alert;

use crate::crypto::{GnuPg, Keys, Sealed, Standard, Verdict, wkd};
use crate::message::Content;
use crate::store::{AccountId, MessageId};
use crate::ui::reader::Reading;
use crate::ui::toolbar::Signal;
use crate::ui::view::MailView;
use crate::worker::Event as MailEvent;

impl MailView {
    pub(crate) fn keys(&self) -> Keys {
        Keys {
            gnupg: GnuPg::default(),
            trusted: self
                .store
                .trusted_certificates()
                .inspect_err(|error| tracing::warn!("cannot read trusted certificates: {error}"))
                .unwrap_or_default(),
        }
    }

    pub(super) fn signs(&self, account: AccountId) -> bool {
        self.store
            .identity(account)
            .inspect_err(|error| tracing::warn!("cannot read an account's keys: {error}"))
            .is_ok_and(|identity| {
                identity.sign && !(identity.openpgp.is_empty() && identity.smime.is_empty())
            })
    }

    fn opened_content(&self, id: MessageId) -> Option<&Content> {
        self.reader
            .thread
            .as_ref()?
            .entry(id)?
            .reading
            .as_ref()
            .map(|reading| &reading.content)
    }

    pub(super) fn unseal(&mut self, id: MessageId) {
        let raw = match self.store.body(id) {
            Ok(Some(raw)) => raw,
            Ok(None) => return,
            Err(error) => return tracing::warn!("cannot read a message: {error}"),
        };
        let keys = self.keys();
        let notifier = self.mailroom.notifier();
        let spawned = std::thread::Builder::new()
            .name("unseal".to_owned())
            .spawn(move || {
                let content = Content::open(&raw, Some(&keys));
                notifier.send(MailEvent::Opened(id, Box::new(content)));
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot open a signed or encrypted message: {error}");
        }
    }

    pub(super) fn opened(&mut self, id: MessageId, content: Content) {
        if self.opened_content(id).is_none() {
            return;
        }
        let security = &content.security;
        if let Some(signed) = &security.signed
            && signed.standard == Standard::Smime
            && signed.verdict == Verdict::Verified
            && let Err(error) = self
                .store
                .remember_certificates(&content.envelope.sender().email, &signed.certificates)
        {
            tracing::warn!("cannot keep a sender's certificate: {error}");
        }
        let decrypted = security
            .encrypted
            .as_ref()
            .is_some_and(|encrypted| encrypted.state == Sealed::Decrypted);
        if decrypted
            && self.settings.index_decrypted
            && let Err(error) = self.store.index_decrypted(id, &content.text())
        {
            tracing::warn!("cannot add a decrypted message to search: {error}");
        }
        let reading = Reading::new(content, true, &self.settings);
        let remote = self.loads_remote(&reading);
        if let Some(entry) = self
            .reader
            .thread
            .as_mut()
            .and_then(|thread| thread.entry_mut(id))
        {
            entry.reading = Some(reading);
        }
        self.reader.layout = None;
        if remote {
            self.load_remote(id);
        }
    }

    pub(super) fn look_up_key(&mut self, id: MessageId) {
        let Some(email) = self
            .opened_content(id)
            .map(|content| content.envelope.sender().email)
            .filter(|email| !email.is_empty())
        else {
            return;
        };
        let network = self.network.clone();
        let notifier = self.mailroom.notifier();
        let spawned = std::thread::Builder::new()
            .name("key-lookup".to_owned())
            .spawn(move || {
                let found = wkd::look_up(&email, &network)
                    .and_then(|key| GnuPg::default().import(&key))
                    .map_err(|error| error.to_string());
                notifier.send(MailEvent::LookedUp(id, found));
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot look up a key: {error}");
        }
    }

    pub(super) fn looked_up(&mut self, id: MessageId, found: Result<(), String>) {
        match found {
            Ok(()) => self.unseal(id),
            Err(error) => self.ui.present_sheet(
                Alert::new(&tr!("The key could not be found"), &error, |_| {})
                    .primary(&tr!("OK"), "ok"),
            ),
        }
    }

    pub(super) fn confirm_trust_issuer(&mut self, id: MessageId) {
        let Some(issuer) = self
            .opened_content(id)
            .and_then(|content| content.security.signed.as_ref()?.issuer.clone())
        else {
            return;
        };
        let signals = Rc::clone(&self.signals);
        let alert = Alert::new(
            &tr!("Trust certificates from “{name}”?", name = issuer.name.clone()),
            &tr!(
                "Mail will accept every certificate this issuer signs as proof of a sender’s address. Only trust it if you have checked its fingerprint with someone you know: {fingerprint}",
                fingerprint = issuer.fingerprint.clone()
            ),
            move |choice| {
                if choice == "trust" {
                    signals.borrow_mut().push(Signal::TrustIssuer(id));
                }
            },
        )
        .primary(&tr!("Trust"), "trust")
        .cancel(&tr!("Cancel"), "cancel");
        self.ui.present_sheet(alert);
    }

    pub(super) fn trust_issuer(&mut self, id: MessageId) {
        let Some(issuer) = self
            .opened_content(id)
            .and_then(|content| content.security.signed.as_ref()?.issuer.clone())
        else {
            return;
        };
        match self.store.trust_certificate(&issuer.der) {
            Ok(()) => self.unseal(id),
            Err(error) => self.ui.present_sheet(
                Alert::new(
                    &tr!("The issuer could not be trusted"),
                    &error.to_string(),
                    |_| {},
                )
                .primary(&tr!("OK"), "ok"),
            ),
        }
    }
}
