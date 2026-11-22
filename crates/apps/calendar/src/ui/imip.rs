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
        let Some(first) = objects.first().filter(|_| objects.len() == 1) else {
            return self.offer_import(objects, file);
        };
        match invite::method(first) {
            Method::Request if self.invited(first) => self.offer_invitation(first.clone()),
            Method::Reply => self.take_reply(first),
            Method::Cancel => self.offer_cancel(first),
            _ => self.offer_import(objects, file),
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
            .master()
            .is_some_and(|event| invite::is_invited(event, &me))
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
        let Some(event) = object.master() else {
            return false;
        };
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
        let Some(event) = answered.master().cloned() else {
            return;
        };
        let stored = self.stored(answered.uid());
        match (answer, stored) {
            (Answer::Declined, Some((calendar, index))) => {
                self.remember(calendar, index);
                self.library.remove_object(calendar, index);
            }
            (Answer::Declined, None) => {}
            (_, Some((calendar, _))) => {
                self.library.put_object(calendar, answered);
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
        let found = self.stored(reply.uid()).and_then(|(calendar, index)| {
            let object = self.library.object(calendar, index)?;
            let (updated, answers) = invite::take_reply(object, reply)?;
            Some((calendar, updated, answers))
        });
        let Some((calendar, updated, answers)) = found else {
            self.status = Some(tr!(
                "This answer is to an event that is not in your calendars."
            ));
            return true;
        };
        let title = updated.master().map(title_of).unwrap_or_default();
        self.library.put_object(calendar, updated);
        self.requests.push(Request::Changed);
        let names: Vec<String> = answers
            .into_iter()
            .map(|answer| match answer.name.is_empty() {
                true => answer.address,
                false => answer.name,
            })
            .collect();
        self.status = Some(tr!(
            "“{title}”: answer from {names}",
            title = title,
            names = names.join(", ")
        ));
        true
    }

    fn offer_cancel(&mut self, cancel: &CalendarObject) -> bool {
        let Some((calendar, _)) = self.stored(cancel.uid()) else {
            self.status = Some(tr!(
                "A cancelled event that is not in your calendars was opened."
            ));
            return true;
        };
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
