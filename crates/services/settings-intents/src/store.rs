use bande_intents::{
    Describe, Done, Effect, Find, Found, Job, Limits, Perform, Plan, Ref, Refusal, Row, RowKind,
    Store, Undo, Wanted,
};
use bande_l10n::tr;
use bande_settings_client::{SettingsClient, Stored};
use bande_settings_panes::{Mark, PaneId, SettingHit, Value, search};
use bande_xdg::{
    Locale, applications, current_desktops, data_dirs, find_app, launch_with_arguments,
};

use zbus::blocking::Connection;

use crate::intents::{self, OpenPane, Restore, Set, Setting};
use crate::registry::{Served, panes, served};

const DOMAIN: &str = "settings";
const VALUE: &str = "value";
const SETTINGS_APP: &str = "org.bande.Settings";

/// The settings, kept by the programs that apply them.
pub struct Settings {
    client: SettingsClient,
}

fn no_setting(token: &str, limits: &Limits) -> Refusal {
    let close = search(panes(), &token.replace('-', " "));
    let candidates = close
        .iter()
        .flat_map(|pane| pane.settings.iter())
        .map(|hit: &SettingHit| Ref::<Setting>::new(hit.setting.word()).to_string());
    let reachable: Vec<String> = served()
        .filter(|served| limits.covers(DOMAIN, served.pane.keyword()))
        .map(|served| served.reference().to_string())
        .collect();
    Refusal::not_found(tr!("There is no setting “{name}”", name = token))
        .candidates(candidates.filter(|candidate| reachable.contains(candidate)))
}

fn find_served(reference: &Ref<Setting>, limits: &Limits) -> Result<Served, Refusal> {
    served()
        .find(|served| {
            served.id().word() == reference.token() && limits.covers(DOMAIN, served.pane.keyword())
        })
        .ok_or_else(|| no_setting(reference.token(), limits))
}

fn effect(served: &Served, value: &Value) -> Effect {
    match served.id().mark(Some(value)) {
        Mark::Disrupt => Effect::Disrupt,
        Mark::Critical => Effect::Critical,
        Mark::Hidden | Mark::Read | Mark::Change => Effect::Change,
    }
}

fn pane(word: &str, limits: &Limits) -> Result<PaneId, Refusal> {
    PaneId::from_keyword(word)
        .filter(|pane| limits.covers(DOMAIN, pane.keyword()))
        .ok_or_else(|| {
            let known = panes()
                .iter()
                .map(|pane| pane.id.keyword().to_owned())
                .filter(|word| limits.covers(DOMAIN, word))
                .collect();
            Refusal::invalid("pane", known)
        })
}

/// What Perform writes, and what must still be stored then.
pub struct Change<T> {
    served: Served,
    before: Stored,
    after: T,
}

impl Settings {
    pub fn open(connection: &Connection) -> Result<Self, Refusal> {
        Ok(Self {
            client: SettingsClient::connect(connection)?,
        })
    }

    fn plan<T>(
        served: Served,
        before: Stored,
        effect: Effect,
        (from, to): (String, String),
        after: T,
    ) -> Plan<Change<T>> {
        let title = served.setting.label.clone();
        Plan {
            effects: vec![effect],
            summary: tr!(
                "{setting}: {from} to {to}",
                setting = title.clone(),
                from = from.clone(),
                to = to.clone()
            ),
            rows: vec![
                Row::new(RowKind::Thing, tr!("Setting"), title),
                Row::new(RowKind::Text, tr!("From"), from),
                Row::new(RowKind::Text, tr!("To"), to),
            ],
            references: vec![served.reference().to_string()],
            prepared: Change {
                served,
                before,
                after,
            },
        }
    }

    fn unchanged<T>(&self, change: &Change<T>) -> Result<(), Refusal> {
        if self.client.stored(change.served.id())? == change.before {
            return Ok(());
        }
        Err(Refusal::conflict(tr!(
            "{setting} was changed by something else",
            setting = change.served.setting.label.clone()
        )))
    }

    fn thing(&self, served: &Served, wanted: &Wanted) -> Result<Setting, Refusal> {
        let value = match wanted.has(VALUE) {
            true => Some(self.client.get(served.id())?),
            false => None,
        };
        Ok(served.thing(value.as_ref(), wanted))
    }
}

impl Store for Settings {
    fn look(&self, reference: &str, _properties: &[String]) -> Result<String, Refusal> {
        let reference = Ref::<Setting>::parse(reference)
            .ok_or_else(|| Refusal::not_found(reference.to_owned()))?;
        let served = find_served(&reference, &Limits::default())?;
        written(&self.client.stored(served.id())?)
    }

    fn recover(&self, undo: Undo) -> Result<Option<Undo>, Refusal> {
        let Some(restore) = undo.arguments::<Restore>() else {
            return Ok(None);
        };
        let now = self.look(restore.setting.as_str(), &[])?;
        Ok((now != restore.stored.0).then_some(undo))
    }
}

fn written(stored: &Stored) -> Result<String, Refusal> {
    serde_json::to_string(stored).map_err(|error| Refusal::failed(error.to_string()))
}

impl Describe<Setting> for Settings {
    fn describe(
        &self,
        thing: &Ref<Setting>,
        wanted: &Wanted,
        limits: &Limits,
    ) -> Result<Setting, Refusal> {
        self.thing(&find_served(thing, limits)?, wanted)
    }
}

impl Find<Settings> for intents::Find {
    type Thing = Setting;

    fn find(self, _owner: &Settings, limits: &Limits) -> Result<Found<Setting>, Refusal> {
        let within = match &self.pane {
            Some(word) => Some(pane(&word.0, limits)?),
            None => None,
        };
        let all: Vec<Served> = served()
            .filter(|served| {
                limits.covers(DOMAIN, served.pane.keyword())
                    && within.is_none_or(|within| within == served.pane)
            })
            .collect();
        let references = match self.text.as_ref().map(|text| text.0.trim()) {
            None | Some("") => all.iter().map(Served::reference).collect(),
            Some(words) => {
                let mut hits: Vec<SettingHit> = search(panes(), words)
                    .into_iter()
                    .flat_map(|pane| pane.settings)
                    .collect();
                hits.sort_by_key(|hit| std::cmp::Reverse(hit.score));
                hits.iter()
                    .filter_map(|hit| all.iter().find(|served| served.id() == hit.setting))
                    .map(Served::reference)
                    .collect()
            }
        };
        Ok(Found::new(references))
    }
}

impl Perform<Settings> for OpenPane {
    type Prepared = PaneId;
    type Output = ();

    fn prepare(self, _owner: &Settings, limits: &Limits) -> Result<Plan<PaneId>, Refusal> {
        let pane = pane(&self.pane.0, limits)?;
        Ok(Plan {
            effects: vec![Effect::Show],
            summary: tr!("Open {page} in Settings", page = pane.title()),
            rows: vec![Row::new(RowKind::Text, tr!("Page"), pane.title())],
            references: Vec::new(),
            prepared: pane,
        })
    }

    fn perform(pane: PaneId, _owner: &Settings, _job: &Job) -> Result<Done<()>, Refusal> {
        let entries = applications(
            &data_dirs(),
            Locale::from_env().as_ref(),
            &current_desktops(),
        );
        let entry = find_app(&entries, SETTINGS_APP)
            .ok_or_else(|| Refusal::unavailable(tr!("Settings is not installed")))?;
        launch_with_arguments(entry, &[pane.keyword().to_owned()], None)
            .map_err(|error| Refusal::failed(error.to_string()))?;
        Ok(Done::new(()))
    }
}

impl Perform<Settings> for Set {
    type Prepared = Change<Value>;
    type Output = Setting;

    fn prepare(self, owner: &Settings, limits: &Limits) -> Result<Plan<Change<Value>>, Refusal> {
        let served = find_served(&self.setting, limits)?;
        let value = served.parse(&self.value.0)?;
        let shown = served.label(&owner.client.get(served.id())?);
        let before = owner.client.stored(served.id())?;
        let effect = effect(&served, &value);
        let words = (shown, served.label(&value));
        Ok(Settings::plan(served, before, effect, words, value))
    }

    fn perform(
        change: Change<Value>,
        owner: &Settings,
        job: &Job,
    ) -> Result<Done<Setting>, Refusal> {
        let id = change.served.id();
        let undo = Undo::by(Restore {
            setting: change.served.reference(),
            stored: written(&change.before)?.into(),
        })?
        .touching(&change.served.reference(), &[VALUE]);
        owner.unchanged(&change)?;
        job.keep(undo.clone())?;
        owner.client.set(id, &change.after)?;
        let thing = owner.thing(&change.served, &Wanted::of(&[VALUE]))?;
        Ok(match owner.client.stored(id)? == change.before {
            true => Done::new(thing),
            false => Done::undone_by(thing, undo),
        })
    }
}

impl Perform<Settings> for Restore {
    type Prepared = Change<Stored>;
    type Output = Setting;

    fn prepare(self, owner: &Settings, limits: &Limits) -> Result<Plan<Change<Stored>>, Refusal> {
        let served = find_served(&self.setting, limits)?;
        let stored: Stored = serde_json::from_str(&self.stored.0)
            .map_err(|error| Refusal::failed(error.to_string()))?;
        let value = SettingsClient::shown(served.id(), &stored)
            .ok_or_else(|| Refusal::failed(tr!("The earlier value cannot be read")))?;
        let before = owner.client.stored(served.id())?;
        let words = (
            served.label(&owner.client.get(served.id())?),
            served.label(&value),
        );
        let effect = effect(&served, &value);
        Ok(Settings::plan(served, before, effect, words, stored))
    }

    fn perform(
        change: Change<Stored>,
        owner: &Settings,
        _job: &Job,
    ) -> Result<Done<Setting>, Refusal> {
        owner.unchanged(&change)?;
        owner.client.restore(change.served.id(), &change.after)?;
        Ok(Done::new(
            owner.thing(&change.served, &Wanted::of(&[VALUE]))?,
        ))
    }
}
