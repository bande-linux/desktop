use std::path::PathBuf;

use bande_fs::{
    Access, Class, Entry, access, runnable, set_extension_hidden, set_mode, with_access,
    with_runnable,
};
use bande_toolkit::ui::{
    CheckState, Checkbox, Key, Modifiers, PopUpButton, TextField, Toggle, WidgetId,
};

use crate::L10N;
use crate::controls::{Signal, sender};
use crate::view::{FilesView, Rename};

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
}

impl InspectorControls {
    pub(crate) fn ids(&self) -> Vec<WidgetId> {
        self.name
            .iter()
            .flat_map(|name| [Some(name.field), name.hide.map(|(id, _)| id)])
            .flatten()
            .chain(self.runnable.iter().map(|(id, _)| *id))
            .chain(self.access.iter().map(|(_, id, _)| *id))
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
    L10N.tr(match class {
        Class::Owner => "access-you",
        Class::Group => "access-group",
        Class::Others => "access-everyone",
    })
}

fn access_titles() -> Vec<String> {
    Access::ALL
        .iter()
        .map(|access| {
            L10N.tr(match access {
                Access::ReadWrite => "access-read-write",
                Access::ReadOnly => "access-read-only",
                Access::NoAccess => "access-none",
            })
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
        let mut checkbox = Checkbox::new(&L10N.tr("hide-extension"), check(wanted.on), move |on| {
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
                field: self.insert(TextField::new("", &entry.name, |_| {})),
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
        match set_extension_hidden(&entry.path, hidden) {
            Ok(()) => {
                let folders = entry.path.parent().map(PathBuf::from);
                self.browser.refresh(folders.as_slice());
                self.selection_changed();
            }
            Err(error) => self.show_notice(
                L10N.tr_args("error-operation", [("error", error.to_string().into())]),
            ),
        }
    }

    fn runnable_toggle(&mut self, on: bool) -> WidgetId {
        let send = sender(&self.signals);
        self.insert(Toggle::new(&L10N.tr("allow-running"), on, move |on| {
            send(Signal::Runnable(on));
        }))
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
        self.inspector_controls = Some(controls);
    }

    fn change_mode(&mut self, change: impl FnOnce(u32, bool) -> u32) {
        let Some((path, folder, mode)) = self.editable() else {
            return;
        };
        let changed = change(mode, folder);
        match set_mode(&path, changed) {
            Ok(()) => {
                if let Some(info) = &mut self.info {
                    info.mode = Some(changed);
                }
            }
            Err(error) => self.show_notice(
                L10N.tr_args("error-permissions", [("error", error.to_string().into())]),
            ),
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
