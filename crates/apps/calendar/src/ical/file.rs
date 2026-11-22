use std::collections::HashSet;

use super::event::PRODUCT;
use bande_content_lines::{self as lines, Component, Property, escape};

pub fn export<'a>(name: &str, objects: impl IntoIterator<Item = &'a str>) -> String {
    let mut calendar = Component::new("VCALENDAR");
    calendar.push(Property::new("VERSION", "2.0"));
    calendar.push(Property::new("PRODID", PRODUCT));
    calendar.push(Property::new("NAME", escape(name)));
    calendar.push(Property::new("X-WR-CALNAME", escape(name)));
    let mut zones = HashSet::new();
    let roots = objects
        .into_iter()
        .filter_map(|data| lines::parse(data).ok())
        .flatten()
        .filter(|root| root.name == "VCALENDAR");
    for root in roots {
        for child in root.components {
            let keep = match child.name.as_str() {
                "VTIMEZONE" => zones.insert(child.value("TZID").unwrap_or_default().to_owned()),
                "VEVENT" | "VTODO" | "VJOURNAL" => true,
                _ => false,
            };
            if keep {
                calendar.components.push(child);
            }
        }
    }
    calendar.to_text()
}
