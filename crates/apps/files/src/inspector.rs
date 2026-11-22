use bande_fs::Action;
use bande_l10n::tr;
use std::path::PathBuf;

use crate::controls::{Signal, sender};
use crate::servers::ServerDone;
use crate::view::{FilesView, Rename, Sheet};
use bande_fs::{
    Access, Class, Entry, access, apply_to_enclosed, own_groups, runnable, set_extension_hidden,
    set_group, set_mode, with_access, with_runnable,
};
use bande_toolkit::ui::{
    Button, CheckState, Checkbox, Key, Modifiers, PopUpButton, TextField, Toggle, WidgetId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HideChoice {
    on: bool,
    enabled: bool,
}

pub(crate) struct NameControls {
    pub(crate) field: WidgetId,
    pub(crate) hide: Option<(WidgetId, HideChoice)>,
    name: String,
}

pub(crate) struct InspectorControls {
    pub(crate) name: Option<NameControls>,
    pub(crate) runnable: Option<(WidgetId, bool)>,
    pub(crate) access: Vec<(Class, WidgetId, Access)>,
    pub(crate) group: Option<(WidgetId, u32)>,
    pub(crate) enclosed: Option<WidgetId>,
}

impl InspectorControls {
    pub(crate) fn ids(&self) -> Vec<WidgetId> {
        self.name
            .iter()
            .flat_map(|name| [Some(name.field), name.hide.map(|(id, _)| id)])
            .flatten()
            .chain(self.runnable.iter().map(|(id, _)| *id))
            .chain(self.access.iter().map(|(_, id, _)| *id))
            .chain(self.group.map(|(id, _)| id))
            .chain(self.enclosed)
            .collect()
    }
}

fn check(on: bool) -> CheckState {
    match on {
        true => CheckState::On,
        false => CheckState::Off,
    }
}

pub(crate) fn class_title(class: Class) -> String {
    match class {
        Class::Owner => tr!("You"),
        Class::Group => tr!("Group"),
        Class::Others => tr!("Everyone"),
    }
}

fn access_titles() -> Vec<String> {
    Access::ALL
        .iter()
        .map(|access| match access {
            Access::ReadWrite => tr!("Read & Write"),
            Access::ReadOnly => tr!("Read only"),
            Access::NoAccess => tr!("No Access"),
        })
        .collect()
}

impl FilesView {
    fn inspected(&self) -> Option<&Entry> {
        let info = self.info.as_ref().filter(|_| self.inspector)?;
        match info.entries.as_slice() {
            [entry] if entry.trashed.is_none() => Some(entry),
            _ => None,
        }
    }

    fn editable(&self) -> Option<(PathBuf, bool, u32)> {
        let info = self.info.as_ref()?;
        let entry = self.inspected().filter(|entry| info.owned && !entry.link)?;
        Some((entry.path.clone(), entry.is_folder(), info.mode?))
    }

    fn hide_wanted(&self, entry: &Entry) -> Option<HideChoice> {
        let enabled = !self.show_extensions
            && bande_fs::hidden_extension(&entry.name, None, Some(true)).is_some();
        (!entry.is_folder()).then(|| HideChoice {
            on: enabled && self.shown_name(entry) != entry.name,
            enabled,
        })
    }

    fn hide_checkbox(&mut self, wanted: HideChoice) -> WidgetId {
        let send = sender(&self.signals);
        let mut checkbox = Checkbox::new(&tr!("Hide extension"), check(wanted.on), move |on| {
            send(Signal::HideExtension(on));
        });
        checkbox.set_enabled(wanted.enabled);
        self.insert(checkbox)
    }

    fn sync_name_controls(&mut self, controls: &mut InspectorControls, entry: &Entry) {
        let wanted = self.hide_wanted(entry);
        let mut name = match controls.name.take() {
            Some(name) => name,
            None => NameControls {
                field: self.insert(TextField::new("", &entry.name, |_| {}).without_text_checking()),
                hide: None,
                name: entry.name.clone(),
            },
        };
        if name.name != entry.name {
            name.name.clone_from(&entry.name);
            self.show_name(name.field, &entry.name);
        }
        name.hide = match (name.hide, wanted) {
            (Some((id, shown)), Some(wanted)) => {
                if shown != wanted {
                    self.ui.update_widget::<Checkbox>(id, |checkbox, cx| {
                        checkbox.set_state(cx, check(wanted.on));
                        checkbox.set_enabled(wanted.enabled);
                    });
                }
                Some((id, wanted))
            }
            (Some((id, _)), None) => {
                self.remove(id);
                None
            }
            (None, wanted) => wanted.map(|wanted| (self.hide_checkbox(wanted), wanted)),
        };
        controls.name = Some(name);
    }

    fn show_name(&mut self, field: WidgetId, name: &str) {
        if let Some(field) = self.ui.widget_as::<TextField>(field) {
            field.replace_text(name);
        }
    }

    pub(crate) fn name_field_focused(&self) -> bool {
        let field = self
            .inspector_controls
            .as_ref()
            .and_then(|controls| controls.name.as_ref())
            .map(|name| name.field);
        field.is_some() && self.ui.focus() == field
    }

    pub(crate) fn name_field_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        match key {
            Key::Enter => self.commit_name_field(),
            Key::Escape => {
                self.reset_name_field();
                self.focus_content();
            }
            _ => {
                return self
                    .ui
                    .handle(&bande_toolkit::ui::Event::Key { key, modifiers });
            }
        }
        true
    }

    fn reset_name_field(&mut self) {
        let shown = self
            .inspector_controls
            .as_ref()
            .and_then(|controls| controls.name.as_ref())
            .map(|name| (name.field, name.name.clone()));
        if let Some((field, name)) = shown {
            self.show_name(field, &name);
        }
    }

    pub(crate) fn commit_name_field(&mut self) {
        if !self.name_field_focused() {
            return;
        }
        let Some((field, entry)) = self
            .inspector_controls
            .as_ref()
            .and_then(|controls| controls.name.as_ref())
            .map(|name| name.field)
            .zip(self.inspected().cloned())
        else {
            return;
        };
        let typed = self
            .ui
            .widget_as::<TextField>(field)
            .map(|field| field.text())
            .unwrap_or_default();
        self.reset_name_field();
        self.focus_content();
        if typed != entry.name {
            let rename = Rename {
                path: entry.path.clone(),
                field,
                hidden_extension: None,
                folder: entry.is_folder(),
            };
            self.rename_checked(rename, typed, true);
        }
    }

    pub(crate) fn set_extension_hidden(&mut self, hidden: bool) {
        let Some(entry) = self.inspected().cloned() else {
            return;
        };
        if let Some((_, shown)) = self
            .inspector_controls
            .as_mut()
            .and_then(|controls| controls.name.as_mut())
            .and_then(|name| name.hide.as_mut())
        {
            shown.on = hidden;
        }
        if bande_volumes::on_connected_volume(&entry.path) {
            let path = entry.path.clone();
            self.on_server(move || {
                let result = set_extension_hidden(&path, hidden);
                ServerDone::Changed(Action::Rename, Vec::new(), result.err())
            });
            return;
        }
        match set_extension_hidden(&entry.path, hidden) {
            Ok(()) => {
                let folders = entry.path.parent().map(PathBuf::from);
                self.browser.refresh(folders.as_slice());
                self.selection_changed();
            }
            Err(error) => self.show_notice(tr!(
                "The operation couldn’t be completed: {error}",
                error = error.to_string()
            )),
        }
    }

    fn runnable_toggle(&mut self, on: bool) -> WidgetId {
        let send = sender(&self.signals);
        self.insert(Toggle::new(
            &tr!("Allow running as a program"),
            on,
            move |on| {
                send(Signal::Runnable(on));
            },
        ))
    }

    fn access_pop_up(&mut self, class: Class, shown: Access) -> WidgetId {
        let send = sender(&self.signals);
        let selected = Access::ALL
            .iter()
            .position(|known| *known == shown)
            .unwrap_or(0);
        self.insert(PopUpButton::new(
            &class_title(class),
            &access_titles(),
            selected,
            move |index| send(Signal::Access(class, index)),
        ))
    }

    fn group_choices(&self, current: u32) -> Vec<(u32, String)> {
        let mut groups = own_groups();
        if !groups.iter().any(|(id, _)| *id == current) {
            groups.insert(0, (current, current.to_string()));
        }
        groups
    }

    fn group_pop_up(&mut self, current: u32) -> WidgetId {
        let send = sender(&self.signals);
        let groups = self.group_choices(current);
        let selected = groups
            .iter()
            .position(|(id, _)| *id == current)
            .unwrap_or(0);
        let titles: Vec<String> = groups.into_iter().map(|(_, name)| name).collect();
        self.insert(PopUpButton::new(
            &tr!("Owning group"),
            &titles,
            selected,
            move |index| send(Signal::OwningGroup(index)),
        ))
    }

    pub(crate) fn set_owning_group(&mut self, index: usize) {
        let Some((path, _, _)) = self.editable() else {
            return;
        };
        let Some(current) = self.info.as_ref().and_then(|info| info.group) else {
            return;
        };
        let Some((chosen, _)) = self.group_choices(current).get(index).cloned() else {
            return;
        };
        if let Some((_, shown)) = self
            .inspector_controls
            .as_mut()
            .and_then(|controls| controls.group.as_mut())
        {
            *shown = chosen;
        }
        match set_group(&path, chosen) {
            Ok(()) => {
                if let Some(info) = &mut self.info {
                    info.group = Some(chosen);
                }
            }
            Err(error) => self.show_notice(tr!(
                "The group couldn’t be changed: {error}",
                error = error.to_string()
            )),
        }
        self.sync_inspector_controls();
    }

    pub(crate) fn ask_to_apply_to_enclosed(&mut self) {
        let Some((folder, true, mode)) = self.editable() else {
            return;
        };
        let group = self.info.as_ref().and_then(|info| info.group);
        self.present(Sheet::ApplyToEnclosed {
            folder,
            mode,
            group,
        });
    }

    pub(crate) fn apply_to_enclosed(&mut self, folder: PathBuf, mode: u32, group: Option<u32>) {
        self.on_server(move || {
            ServerDone::Changed(
                Action::Rename,
                Vec::new(),
                apply_to_enclosed(&folder, mode, group).err(),
            )
        });
    }

    fn drop_inspector_controls(&mut self) {
        if let Some(controls) = self.inspector_controls.take() {
            for id in controls.ids() {
                self.remove(id);
            }
        }
    }

    pub(crate) fn sync_inspector_controls(&mut self) {
        let Some(entry) = self.inspected().cloned() else {
            self.drop_inspector_controls();
            return;
        };
        let mut controls = self
            .inspector_controls
            .take()
            .unwrap_or_else(|| InspectorControls {
                name: None,
                runnable: None,
                access: Vec::new(),
                group: None,
                enclosed: None,
            });
        self.sync_name_controls(&mut controls, &entry);
        let editable = self.editable();
        let wanted = editable
            .as_ref()
            .and_then(|(_, folder, mode)| (!folder).then(|| runnable(*mode)));
        match (controls.runnable, wanted) {
            (Some((_, shown)), Some(on)) if shown == on => {}
            (shown, on) => {
                if let Some((id, _)) = shown {
                    self.remove(id);
                }
                controls.runnable = on.map(|on| (self.runnable_toggle(on), on));
            }
        }
        let mut access_controls = Vec::new();
        for class in Class::ALL {
            let kept = controls
                .access
                .iter()
                .find(|(known, _, _)| *known == class)
                .copied();
            let Some((_, _, mode)) = editable else {
                if let Some((_, id, _)) = kept {
                    self.remove(id);
                }
                continue;
            };
            let wanted = access(mode, class);
            let id = match kept {
                Some((_, id, shown)) if shown == wanted => id,
                kept => {
                    if let Some((_, id, _)) = kept {
                        self.remove(id);
                    }
                    self.access_pop_up(class, wanted)
                }
            };
            access_controls.push((class, id, wanted));
        }
        controls.access = access_controls;
        let wanted_group = editable
            .as_ref()
            .and_then(|_| self.info.as_ref().and_then(|info| info.group));
        match (controls.group, wanted_group) {
            (Some((_, shown)), Some(group)) if shown == group => {}
            (shown, group) => {
                if let Some((id, _)) = shown {
                    self.remove(id);
                }
                controls.group = group.map(|group| (self.group_pop_up(group), group));
            }
        }
        let wants_enclosed = editable.as_ref().is_some_and(|(_, folder, _)| *folder);
        match (controls.enclosed, wants_enclosed) {
            (Some(id), false) => {
                self.remove(id);
                controls.enclosed = None;
            }
            (None, true) => {
                let send = sender(&self.signals);
                controls.enclosed = Some(
                    self.insert(Button::new(&tr!("Apply to Enclosed Items…"), move || {
                        send(Signal::ApplyToEnclosed)
                    })),
                );
            }
            _ => {}
        }
        self.inspector_controls = Some(controls);
    }

    fn change_mode(&mut self, change: impl FnOnce(u32, bool) -> u32) {
        let Some((path, folder, mode)) = self.editable() else {
            return;
        };
        let changed = change(mode, folder);
        if bande_volumes::on_connected_volume(&path) {
            if let Some(info) = &mut self.info {
                info.mode = Some(changed);
            }
            self.on_server(move || {
                ServerDone::Changed(Action::Rename, Vec::new(), set_mode(&path, changed).err())
            });
            self.sync_inspector_controls();
            return;
        }
        match set_mode(&path, changed) {
            Ok(()) => {
                if let Some(info) = &mut self.info {
                    info.mode = Some(changed);
                }
            }
            Err(error) => self.show_notice(tr!(
                "The permissions couldn’t be changed: {error}",
                error = error.to_string()
            )),
        }
        self.sync_inspector_controls();
    }

    pub(crate) fn set_runnable(&mut self, on: bool) {
        if let Some((_, shown)) = self
            .inspector_controls
            .as_mut()
            .and_then(|controls| controls.runnable.as_mut())
        {
            *shown = on;
        }
        self.change_mode(|mode, _| with_runnable(mode, on));
    }

    pub(crate) fn set_access(&mut self, class: Class, index: usize) {
        let Some(chosen) = Access::ALL.get(index).copied() else {
            return;
        };
        if let Some(controls) = &mut self.inspector_controls {
            for (known, _, shown) in &mut controls.access {
                if *known == class {
                    *shown = chosen;
                }
            }
        }
        self.change_mode(|mode, folder| with_access(mode, class, chosen, folder));
    }
}
