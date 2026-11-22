use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use bande_address_book::store::Library;
use bande_address_book::vcard::{Kind, display_name};
use bande_search::{Offer, offer_in_background};

pub const DOMAIN: &str = "org.bande.Contacts";
pub const CONTACT_ARGUMENT: &str = "contact:";

fn fingerprint(library: &Library) -> String {
    let mut hasher = DefaultHasher::new();
    for book in &library.books {
        book.info.id.hash(&mut hasher);
        for stored in &book.cards {
            (
                &stored.href,
                &stored.etag,
                stored.data.len(),
                stored.pending,
            )
                .hash(&mut hasher);
        }
    }
    format!("{:016x}", hasher.finish())
}

/// Offers the contacts to the desktop's search when they changed since the last offer.
pub fn offer_to_search(library: &Library) {
    static LAST: Mutex<String> = Mutex::new(String::new());
    let fingerprint = fingerprint(library);
    match LAST.lock() {
        Ok(mut last) if *last != fingerprint => fingerprint.clone_into(&mut last),
        _ => return,
    }
    let items = library
        .contacts()
        .into_iter()
        .filter(|(_, card)| card.kind() == Kind::Individual)
        .map(|(_, card)| {
            let contact = card.contact();
            let detail = contact
                .emails
                .first()
                .map(|email| email.value.clone())
                .or_else(|| contact.phones.first().map(|phone| phone.value.clone()))
                .unwrap_or_else(|| contact.organisation.clone());
            let body = [contact.organisation.clone(), contact.title.clone()]
                .into_iter()
                .chain(contact.emails.iter().map(|email| email.value.clone()))
                .chain(contact.phones.iter().map(|phone| phone.value.clone()))
                .collect::<Vec<_>>()
                .join("\n");
            Offer {
                id: card.uid(),
                kind: "contact".to_owned(),
                title: display_name(&contact),
                detail,
                target: format!("{CONTACT_ARGUMENT}{}", card.uid()),
                body,
            }
        })
        .collect();
    offer_in_background(DOMAIN, fingerprint, items);
}
