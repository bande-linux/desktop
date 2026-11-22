use std::path::Path;

use bande_l10n::tr;
use bande_toolkit::ui::Alert;

use super::{CalendarView, Confirm, Request, inspector};
use crate::ical::{CalendarObject, Event};
use crate::invite::{self, Answer, Method};
use crate::store::AccountKind;

const ANSWERS: [(Answer, &str); 3] = [
    (Answer::Accepted, "accepted"),
    (Answer::Tentative, "tentative"),
    (Answer::Declined, "declined"),
];

fn query(text: &str) -> String {
    bande_xdg::percent_encode(Path::new(text))
}

fn title_of(event: &Event) -> String {
    match event.summary.trim() {
        "" => tr!("New Event"),
        summary => summary.to_owned(),
    }
}

impl CalendarView {
    pub fn offer(&mut self, objects: Vec<CalendarObject>, file: &str) -> bool {
        if self.confirm.is_some() {
            self.offers.push_back((objects, file.to_owned()));
            return true;
        }
        let Some(first) = objects.first().filter(|_| objects.len() == 1) else {
            return self.offer_import(objects, file);
        };
        match invite::method(first) {
            Method::Request if self.invited(first) => self.offer_invitation(first.clone()),
            Method::Reply => self.take_reply(first),
            Method::Cancel => self.offer_cancel(first),
            Method::Counter => self.offer_counter(first),
            Method::DeclineCounter => self.take_declined_counter(first),
            _ => self.offer_import(objects, file),
        }
    }

    pub(crate) fn offer_next(&mut self) {
        while self.confirm.is_none()
            && let Some((objects, file)) = self.offers.pop_front()
        {
            self.offer(objects, &file);
        }
    }

    fn my_addresses(&self) -> Vec<String> {
        self.library
            .accounts
            .iter()
            .flat_map(|account| match &account.kind {
                AccountKind::CalDav { addresses, .. } if !addresses.is_empty() => addresses.clone(),
                AccountKind::CalDav { user, .. } if user.contains('@') => vec![user.clone()],
                _ => Vec::new(),
            })
            .collect()
    }

    fn invited(&self, object: &CalendarObject) -> bool {
        let me = self.my_addresses();
        object
            .events
            .iter()
            .any(|event| invite::is_invited(event, &me))
    }

    fn stored_object(&self, uid: &str) -> Option<(usize, usize, &CalendarObject)> {
        let (calendar, index) = self.stored(uid)?;
        Some((calendar, index, self.library.object(calendar, index)?))
    }

    fn stored(&self, uid: &str) -> Option<(usize, usize)> {
        self.library
            .writable()
            .find_map(|(index, calendar)| calendar.find(uid).map(|object| (index, object)))
    }

    fn when(&self, event: &Event) -> String {
        let at = event.start.to_zoned(&self.zone);
        match event.start.is_date() {
            true => self.formats.date(&at, true),
            false => self.formats.date_time(&at, true),
        }
    }

    fn offer_invitation(&mut self, object: CalendarObject) -> bool {
        let Some(event) = object.master().or_else(|| object.events.first()) else {
            return false;
        };
        if let Some((_, _, stored)) = self.stored_object(object.uid())
            && !invite::may_update(stored, &object)
        {
            self.status = Some(tr!(
                "This invitation is older than the event in your calendar, or is not from its organizer."
            ));
            return true;
        }
        let organizer = invite::organizer(event).map_or_else(String::new, |organizer| {
            match organizer.name.is_empty() {
                true => organizer.address,
                false => organizer.name,
            }
        });
        let title = tr!("Invitation to “{title}”", title = title_of(event));
        let message = tr!(
            "From {organizer}, {when}",
            organizer = organizer,
            when = self.when(event)
        );
        self.go_to(event.start.date());
        self.confirm = Some(Confirm::Invitation(Box::new(object)));
        let mut alert = Alert::new(&title, &message, self.choice()).cancel(&tr!("Later"), "cancel");
        for (answer, action) in ANSWERS {
            let label = inspector::answer_title(answer);
            alert = match answer {
                Answer::Accepted => alert.primary(&label, action),
                _ => alert.other(&label, action),
            };
        }
        self.ui.present_dialog(alert);
        true
    }

    pub(crate) fn answer_invitation(&mut self, object: CalendarObject, action: &str) {
        let Some((answer, _)) = ANSWERS.into_iter().find(|(_, name)| *name == action) else {
            return;
        };
        let me = self.my_addresses();
        let answered = invite::without_method(invite::answer(&object, &me, answer));
        let Some(event) = answered
            .master()
            .or_else(|| answered.events.first())
            .cloned()
        else {
            return;
        };
        let whole = answered.master().is_some();
        let stored = match self.stored_object(answered.uid()) {
            Some((_, _, stored)) if !invite::may_update(stored, &answered) => return,
            Some((calendar, index, stored)) => Some((
                calendar,
                index,
                invite::merge_update(stored, answered.clone()),
            )),
            None => None,
        };
        match (answer, stored) {
            (Answer::Declined, Some((calendar, index, _))) if whole => {
                self.remember(calendar, index);
                self.library.remove_object(calendar, index);
            }
            (Answer::Declined, None) => {}
            (_, Some((calendar, _, merged))) => {
                self.library.put_object(calendar, merged);
            }
            (_, None) => {
                if let Some(calendar) = self.default_calendar() {
                    self.library.put_object(calendar, answered);
                }
            }
        }
        self.requests.push(Request::Changed);
        let Some(organizer) = invite::organizer(&event) else {
            return;
        };
        let (title, when) = (title_of(&event), self.when(&event));
        let (subject, body) = match answer {
            Answer::Accepted => (
                tr!("Accepted: {title}", title = title.clone()),
                tr!(
                    "I will attend “{title}”, {when}.",
                    title = title,
                    when = when
                ),
            ),
            Answer::Tentative => (
                tr!("Maybe: {title}", title = title.clone()),
                tr!(
                    "I might attend “{title}”, {when}.",
                    title = title,
                    when = when
                ),
            ),
            Answer::Declined | Answer::Pending => (
                tr!("Declined: {title}", title = title.clone()),
                tr!(
                    "I will not attend “{title}”, {when}.",
                    title = title,
                    when = when
                ),
            ),
        };
        self.requests.push(Request::Mail(format!(
            "mailto:{}?subject={}&body={}",
            query(&organizer.address),
            query(&subject),
            query(&body)
        )));
    }

    fn take_reply(&mut self, reply: &CalendarObject) -> bool {
        let me = self.my_addresses();
        let Some((calendar, _, object)) = self.stored_object(reply.uid()) else {
            self.status = Some(tr!(
                "This answer is to an event that is not in your calendars."
            ));
            return true;
        };
        let Some((updated, replier)) = invite::take_reply(object, reply, &me) else {
            self.status = Some(tr!(
                "This answer is not to an event you organize, or is not from one of its invitees."
            ));
            return true;
        };
        let title = updated.master().map(title_of).unwrap_or_default();
        self.library.put_object(calendar, updated);
        self.requests.push(Request::Changed);
        let name = match replier.name.is_empty() {
            true => replier.address,
            false => replier.name,
        };
        self.status = Some(tr!(
            "“{title}”: answer from {names}",
            title = title,
            names = name
        ));
        true
    }

    fn offer_counter(&mut self, counter: &CalendarObject) -> bool {
        let me = self.my_addresses();
        let taken = self
            .stored_object(counter.uid())
            .and_then(|(calendar, _, stored)| {
                let (updated, proposer) = invite::take_counter(stored, counter, &me, &self.zone)?;
                Some((calendar, stored.clone(), updated, proposer))
            });
        let Some((calendar, stored, updated, proposer)) = taken else {
            self.status = Some(tr!(
                "This proposal is not for an event you organize, or is not from one of its invitees."
            ));
            return true;
        };
        let Some(proposed) = counter.events.first() else {
            return false;
        };
        let current = invite::counterpart_of(&stored, proposed)
            .map_or_else(String::new, |event| self.when(event));
        let name = match proposer.name.is_empty() {
            true => proposer.address.clone(),
            false => proposer.name.clone(),
        };
        let title = title_of(proposed);
        let message = match current.is_empty() {
            true => self.when(proposed),
            false => tr!(
                "{when} instead of {current}",
                when = self.when(proposed),
                current = current
            ),
        };
        self.go_to(proposed.start.date());
        self.confirm = Some(Confirm::Counter {
            calendar: self.library.calendars[calendar].info.id.clone(),
            updated: Box::new(updated),
            proposer: proposer.address,
            title: title.clone(),
        });
        let alert = Alert::new(
            &tr!(
                "{name} proposes a new time for “{title}”",
                name = name,
                title = title
            ),
            &message,
            self.choice(),
        )
        .cancel(&tr!("Later"), "cancel")
        .other(&tr!("Decline"), "decline")
        .primary(&tr!("Accept Proposal"), "accept");
        self.ui.present_dialog(alert);
        true
    }

    pub(crate) fn answer_counter(
        &mut self,
        calendar: &str,
        updated: CalendarObject,
        proposer: &str,
        title: &str,
        action: &str,
    ) {
        match action {
            "accept" => {
                let Some(calendar) = self.library.calendar_index(calendar) else {
                    return;
                };
                if let Some(index) = self.library.calendars[calendar].find(updated.uid()) {
                    self.remember(calendar, index);
                }
                self.library.put_object(calendar, updated);
                self.requests.push(Request::Changed);
            }
            "decline" => {
                let subject = tr!("Proposal declined: {title}", title = title.to_owned());
                let body = tr!(
                    "“{title}” stays at the time it has.",
                    title = title.to_owned()
                );
                self.requests.push(Request::Mail(format!(
                    "mailto:{}?subject={}&body={}",
                    query(proposer),
                    query(&subject),
                    query(&body)
                )));
            }
            _ => {}
        }
    }

    fn take_declined_counter(&mut self, decline: &CalendarObject) -> bool {
        let title = decline.events.first().map(title_of).unwrap_or_default();
        self.status = Some(
            match self
                .stored_object(decline.uid())
                .is_some_and(|(_, _, stored)| invite::declined_counter(stored, decline))
            {
                true => tr!(
                    "The organizer of “{title}” declined the time you proposed.",
                    title = title
                ),
                false => tr!(
                    "This answer to a proposal is not from the organizer of an event in your calendars."
                ),
            },
        );
        true
    }

    fn offer_cancel(&mut self, cancel: &CalendarObject) -> bool {
        let Some((calendar, _, stored)) = self.stored_object(cancel.uid()) else {
            self.status = Some(tr!(
                "A cancelled event that is not in your calendars was opened."
            ));
            return true;
        };
        if !invite::may_update(stored, cancel) {
            self.status = Some(tr!(
                "This cancellation is older than the event in your calendar, or is not from its organizer."
            ));
            return true;
        }
        let title = cancel.events.first().map(title_of).unwrap_or_default();
        let occurrence = cancel
            .events
            .first()
            .and_then(|event| event.recurrence_id.as_ref())
            .map(|moment| moment.key());
        self.confirm = Some(Confirm::Cancelled {
            calendar: self.library.calendars[calendar].info.id.clone(),
            uid: cancel.uid().to_owned(),
            occurrence,
        });
        if let Some(day) = cancel.events.first().map(|event| event.start.date()) {
            self.go_to(day);
        }
        let alert = Alert::new(
            &tr!("“{title}” was cancelled", title = title),
            &tr!("Do you want to remove it from your calendar?"),
            self.choice(),
        )
        .cancel(&tr!("Keep"), "cancel")
        .destructive(&tr!("Remove"), "accept");
        self.ui.present_dialog(alert);
        true
    }

    pub(crate) fn remove_cancelled(
        &mut self,
        calendar: &str,
        uid: &str,
        occurrence: Option<crate::ical::InstanceKey>,
    ) {
        let Some(calendar) = self.library.calendar_index(calendar) else {
            return;
        };
        let Some(index) = self.library.calendars[calendar].find(uid) else {
            return;
        };
        let Some(object) = self.library.object(calendar, index).cloned() else {
            return;
        };
        let kept = occurrence.and_then(|key| {
            crate::edit::remove_occurrence(&object, key, crate::edit::Scope::Occurrence)
        });
        self.remember(calendar, index);
        match kept {
            Some(kept) => {
                self.library.put_object(calendar, kept);
            }
            None => self.library.remove_object(calendar, index),
        }
        self.selected = None;
        self.requests.push(Request::Changed);
    }
}
