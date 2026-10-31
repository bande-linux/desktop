use jiff::ToSpan;
use jiff::civil::{Date, Weekday};

pub const WEEKS: i64 = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Week {
    pub number: Option<i8>,
    pub days: [Date; 7],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Month {
    pub first: Date,
    pub weeks: Vec<Week>,
}

impl Month {
    pub fn containing(day: Date, first_weekday: Weekday) -> Self {
        let first = day.first_of_month();
        let lead = i64::from(first.weekday().since(first_weekday));
        let start = first.saturating_sub(lead.days());
        let weeks = (0..WEEKS)
            .map(|week| {
                let monday_or_first = start.saturating_add((7 * week).days());
                Week {
                    number: (first_weekday == Weekday::Monday)
                        .then(|| monday_or_first.iso_week_date().week()),
                    days: std::array::from_fn(|offset| {
                        monday_or_first.saturating_add((offset as i64).days())
                    }),
                }
            })
            .collect();
        Self { first, weeks }
    }

    pub fn contains(&self, day: Date) -> bool {
        day.year() == self.first.year() && day.month() == self.first.month()
    }

    pub fn shifted(&self, months: i32) -> Self {
        let first_weekday = self
            .weeks
            .first()
            .map_or(Weekday::Monday, |week| week.days[0].weekday());
        Self::containing(self.first.saturating_add(months.months()), first_weekday)
    }
}
