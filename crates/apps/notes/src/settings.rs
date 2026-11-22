use std::sync::OnceLock;

use accesskit::TreeUpdate;
use bande_app::View;
use bande_inbox::Outbox;
use bande_l10n::tr;
use bande_mail_core::accounts;
use bande_toolkit::ui::{Context, Form, FormLine, Label, ScrollView, Toggle, Ui, Widget};
use bande_toolkit::{Appearance, Rect, Scene, TextSystem, Window, WindowEvent};

use crate::worker::Event;

pub const SETTINGS_SIZE: (u32, u32) = (520, 360);

pub static CHANGES: OnceLock<Outbox<Event>> = OnceLock::new();

pub struct AccountsSettings(Ui);

fn toggles() -> Vec<FormLine> {
    let mut lines = vec![FormLine::Heading(tr!("Accounts"))];
    let found = accounts::open_store()
        .and_then(|mail| Ok((accounts::list(&mail)?, accounts::notes(&mail)?)));
    let (all, used) = match found {
        Ok(found) => found,
        Err(error) => {
            let message = tr!(
                "The accounts cannot be read: {error}",
                error = error.to_string()
            );
            lines.push(FormLine::Wide(Box::new(Label::new(&message).secondary())));
            return lines;
        }
    };
    if all.is_empty() {
        lines.push(FormLine::Wide(Box::new(
            Label::new(&tr!(
                "Notes are kept on this computer. To keep them in a mail account as well, choose File ▸ Add Account…."
            ))
            .secondary(),
        )));
    }
    for account in all {
        let id = account.id;
        let toggle = Toggle::new(
            &tr!("Use {email} for Notes", email = account.email.clone()),
            used.contains(&id),
            move |on| {
                let changed =
                    accounts::open_store().and_then(|mail| accounts::set_notes(&mail, id, on));
                match changed {
                    Ok(()) => {
                        if let Some(changes) = CHANGES.get() {
                            changes.send(Event::AccountsChanged);
                        }
                    }
                    Err(error) => tracing::warn!("cannot change the account: {error}"),
                }
            },
        );
        let widget: Box<dyn Widget> = Box::new(toggle);
        lines.push(FormLine::Row(account.email, widget));
    }
    lines
}

impl AccountsSettings {
    pub fn new(appearance: Appearance) -> Self {
        let form = Form::new(&tr!("Notes Settings"), toggles());
        let mut ui = Ui::new(
            Box::new(ScrollView::new(form)),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        Self(ui)
    }
}

impl View for AccountsSettings {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let mut scene = self.0.scene(width, height);
        scene.background = window.appearance().palette().sheet;
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        View::event(&mut self.0, window, event)
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.0.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.0.set_window_active(active);
    }

    fn animating(&self) -> bool {
        self.0.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.0.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.0.text_input_area()
    }
}
