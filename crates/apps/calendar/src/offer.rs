use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use bande_search::{Offer, offer_in_background};

use crate::store::Calendar;

pub const DOMAIN: &str = "org.bande.Calendar";
pub const EVENT_ARGUMENT: &str = "event:";

pub fn offer_to_search(calendars: &[Calendar]) {
    let mut hasher = DefaultHasher::new();
    let mut items = Vec::new();
    for calendar in calendars {
        for stored in &calendar.objects {
            stored.data.hash(&mut hasher);
            let Some(event) = stored
                .parsed
                .as_ref()
                .and_then(|object| object.events.first())
            else {
                continue;
            };
            items.push(Offer {
                id: event.uid.clone(),
                kind: "event".to_owned(),
                title: event.summary.clone(),
                detail: format!("{} · {}", event.start.date(), calendar.info.name),
                target: format!("{EVENT_ARGUMENT}{}", event.uid),
                body: format!("{}\n{}", event.location, event.description),
            });
        }
    }
    offer_in_background(DOMAIN, format!("{:016x}", hasher.finish()), items);
}
