use std::path::PathBuf;

use bande_fs::{Access, Class, access, runnable, set_mode, with_access, with_runnable};
use bande_toolkit::ui::{PopUpButton, Toggle, WidgetId};

use crate::L10N;
use crate::controls::{Signal, sender};
use crate::view::FilesView;

pub(crate) struct InspectorControls {
    path: PathBuf,
    pub(crate) runnable: Option<(WidgetId, bool)>,
    pub(crate) access: Vec<(Class, WidgetId, Access)>,
}

impl InspectorControls {
    pub(crate) fn ids(&self) -> Vec<WidgetId> {
        self.runnable
            .iter()
            .map(|(id, _)| *id)
            .chain(self.access.iter().map(|(_, id, _)| *id))
            .collect()
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
    fn editable(&self) -> Option<(PathBuf, bool, u32)> {
        let info = self.info.as_ref().filter(|_| self.inspector)?;
        match (info.entries.as_slice(), info.mode) {
            ([entry], Some(mode)) if info.owned && !entry.link && entry.trashed.is_none() => {
                Some((entry.path.clone(), entry.is_folder(), mode))
            }
            _ => None,
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
        let Some((path, folder, mode)) = self.editable() else {
            self.drop_inspector_controls();
            return;
        };
        if self
            .inspector_controls
            .as_ref()
            .is_some_and(|controls| controls.path != path)
        {
            self.drop_inspector_controls();
        }
        let mut controls = self
            .inspector_controls
            .take()
            .unwrap_or_else(|| InspectorControls {
                path,
                runnable: None,
                access: Vec::new(),
            });
        let wanted = (!folder).then(|| runnable(mode));
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
            let wanted = access(mode, class);
            let kept = controls
                .access
                .iter()
                .find(|(known, _, _)| *known == class)
                .copied();
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
