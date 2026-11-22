use crate::ical::{CalendarObject, Event, InstanceKey, Property};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Accepted,
    Tentative,
    Declined,
    Pending,
}

impl Answer {
    pub const CHOICES: [Self; 3] = [Self::Accepted, Self::Tentative, Self::Declined];

    fn of(value: Option<&str>) -> Self {
        match value.map(str::to_ascii_uppercase).as_deref() {
            Some("ACCEPTED") => Self::Accepted,
            Some("TENTATIVE") => Self::Tentative,
            Some("DECLINED") => Self::Declined,
            _ => Self::Pending,
        }
    }

    fn value(self) -> &'static str {
        match self {
            Self::Accepted => "ACCEPTED",
            Self::Tentative => "TENTATIVE",
            Self::Declined => "DECLINED",
            Self::Pending => "NEEDS-ACTION",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invitee {
    pub address: String,
    pub name: String,
    pub answer: Answer,
    pub organizer: bool,
}

fn address(value: &str) -> &str {
    let value = value.trim();
    match value.get(..7) {
        Some(scheme) if scheme.eq_ignore_ascii_case("mailto:") => value.get(7..).unwrap_or(value),
        _ => value,
    }
}

fn same(property: &Property, me: &[String]) -> bool {
    let found = address(&property.value);
    me.iter()
        .any(|mine| found.eq_ignore_ascii_case(mine.trim()))
}

fn invitee(property: &Property, organizer: bool) -> Invitee {
    Invitee {
        address: address(&property.value).to_owned(),
        name: property
            .param("CN")
            .map(|name| name.trim_matches('"').to_owned())
            .unwrap_or_default(),
        answer: match organizer {
            true => Answer::Accepted,
            false => Answer::of(property.param("PARTSTAT")),
        },
        organizer,
    }
}

pub fn invitees(event: &Event) -> Vec<Invitee> {
    let organizer = event.component.property("ORGANIZER");
    organizer
        .map(|property| invitee(property, true))
        .into_iter()
        .chain(
            event
                .component
                .all("ATTENDEE")
                .filter(|attendee| {
                    organizer.is_none_or(|organizer| {
                        !address(&attendee.value).eq_ignore_ascii_case(address(&organizer.value))
                    })
                })
                .map(|attendee| invitee(attendee, false)),
        )
        .collect()
}

pub fn my_answer(event: &Event, me: &[String]) -> Option<Answer> {
    let organizer = event.component.property("ORGANIZER")?;
    if same(organizer, me) {
        return None;
    }
    event
        .component
        .all("ATTENDEE")
        .find(|attendee| same(attendee, me))
        .map(|attendee| Answer::of(attendee.param("PARTSTAT")))
}

fn set_answer(event: &mut Event, me: &[String], answer: Answer) {
    for attendee in event
        .component
        .properties
        .iter_mut()
        .filter(|property| property.name.eq_ignore_ascii_case("ATTENDEE") && same(property, me))
    {
        attendee.params.retain(|(name, _)| {
            !name.eq_ignore_ascii_case("PARTSTAT") && !name.eq_ignore_ascii_case("RSVP")
        });
        attendee
            .params
            .push(("PARTSTAT".to_owned(), answer.value().to_owned()));
    }
}

pub fn answer(object: &CalendarObject, me: &[String], answer: Answer) -> CalendarObject {
    let mut object = object.clone();
    for event in &mut object.events {
        set_answer(event, me, answer);
    }
    object
}

pub fn parse_addresses(text: &str) -> Vec<String> {
    text.split([',', ';', ' ', '\n'])
        .map(|part| part.trim().trim_matches(['<', '>']))
        .filter(|part| {
            part.split_once('@')
                .is_some_and(|(user, host)| !user.is_empty() && host.contains('.'))
        })
        .map(str::to_owned)
        .collect()
}

pub fn invite(event: &mut Event, me: &[String], addresses: &[String]) {
    let Some(organizer) = me.first().filter(|_| !addresses.is_empty()) else {
        return;
    };
    let component = &mut event.component;
    if component.property("ORGANIZER").is_none() {
        component.push(Property::new("ORGANIZER", format!("mailto:{organizer}")));
        component.push(
            Property::new("ATTENDEE", format!("mailto:{organizer}"))
                .with_param("PARTSTAT", "ACCEPTED")
                .with_param("ROLE", "CHAIR"),
        );
    }
    for address in addresses {
        if component
            .all("ATTENDEE")
            .any(|attendee| same(attendee, std::slice::from_ref(address)))
        {
            continue;
        }
        component.push(
            Property::new("ATTENDEE", format!("mailto:{address}"))
                .with_param("PARTSTAT", "NEEDS-ACTION")
                .with_param("ROLE", "REQ-PARTICIPANT")
                .with_param("RSVP", "TRUE"),
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Publish,
    Request,
    Reply,
    Cancel,
}

pub fn method(object: &CalendarObject) -> Method {
    match object
        .calendar
        .value("METHOD")
        .map(|method| method.trim().to_ascii_uppercase())
        .as_deref()
    {
        Some("REQUEST") => Method::Request,
        Some("REPLY") => Method::Reply,
        Some("CANCEL") => Method::Cancel,
        _ => Method::Publish,
    }
}

pub fn without_method(mut object: CalendarObject) -> CalendarObject {
    object
        .calendar
        .properties
        .retain(|property| !property.name.eq_ignore_ascii_case("METHOD"));
    object
}

pub fn organizer(event: &Event) -> Option<Invitee> {
    event
        .component
        .property("ORGANIZER")
        .map(|property| invitee(property, true))
}

pub fn is_invited(event: &Event, me: &[String]) -> bool {
    event
        .component
        .all("ATTENDEE")
        .any(|attendee| same(attendee, me))
}

fn instance(event: &Event) -> Option<InstanceKey> {
    event.recurrence_id.as_ref().map(|moment| moment.key())
}

fn counterpart<'a>(object: &'a CalendarObject, event: &Event) -> Option<&'a Event> {
    let key = instance(event);
    object.events.iter().find(|stored| instance(stored) == key)
}

fn organizer_of(object: &CalendarObject) -> Option<&str> {
    object
        .events
        .iter()
        .find_map(|event| event.component.property("ORGANIZER"))
        .map(|property| address(&property.value))
}

pub fn organized_by(object: &CalendarObject, me: &[String]) -> bool {
    organizer_of(object).is_some_and(|organizer| {
        me.iter()
            .any(|mine| organizer.eq_ignore_ascii_case(mine.trim()))
    })
}

pub fn may_update(stored: &CalendarObject, incoming: &CalendarObject) -> bool {
    let same_organizer = match (organizer_of(stored), organizer_of(incoming)) {
        (Some(stored), Some(incoming)) => stored.eq_ignore_ascii_case(incoming),
        _ => false,
    };
    same_organizer
        && incoming.events.iter().all(|event| {
            counterpart(stored, event)
                .or_else(|| stored.master())
                .is_none_or(|stored| event.sequence >= stored.sequence)
        })
}

pub fn merge_update(stored: &CalendarObject, incoming: CalendarObject) -> CalendarObject {
    let alarms_for = |event: &Event| {
        counterpart(stored, event)
            .or_else(|| stored.master())
            .map(|stored| stored.alarms.clone())
    };
    if incoming.master().is_some() {
        let mut merged = incoming;
        for event in &mut merged.events {
            if let Some(alarms) = alarms_for(event) {
                event.alarms = alarms;
            }
        }
        return merged;
    }
    let mut merged = stored.clone();
    for mut event in incoming.events {
        if let Some(alarms) = alarms_for(&event) {
            event.alarms = alarms;
        }
        let key = instance(&event);
        match merged
            .events
            .iter_mut()
            .find(|stored| instance(stored) == key)
        {
            Some(slot) => *slot = event,
            None => merged.events.push(event),
        }
    }
    merged
}

pub fn take_reply(
    object: &CalendarObject,
    reply: &CalendarObject,
    me: &[String],
) -> Option<(CalendarObject, Invitee)> {
    if !organized_by(object, me) {
        return None;
    }
    let mut repliers = reply
        .events
        .iter()
        .flat_map(|event| event.component.all("ATTENDEE"));
    let replier = invitee(repliers.next()?, false);
    if repliers.any(|other| !address(&other.value).eq_ignore_ascii_case(&replier.address)) {
        return None;
    }
    let mut updated = object.clone();
    let mut taken = false;
    for answered in &reply.events {
        let Some(attendee) = answered
            .component
            .all("ATTENDEE")
            .next()
            .map(|attendee| invitee(attendee, false))
        else {
            continue;
        };
        let key = instance(answered);
        for event in updated.events.iter_mut().filter(|event| {
            (key.is_none() || instance(event) == key)
                && is_invited(event, std::slice::from_ref(&attendee.address))
        }) {
            set_answer(
                event,
                std::slice::from_ref(&attendee.address),
                attendee.answer,
            );
            taken = true;
        }
    }
    taken.then_some((updated, replier))
}
