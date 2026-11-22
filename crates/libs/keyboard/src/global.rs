use crate::shortcuts::{Key, Modifiers, bound, chord_keys};

const FIELD: char = '\t';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trigger {
    pub modifiers: Modifiers,
    pub key: Key,
}

fn key_named(name: &str) -> Option<Key> {
    Some(match name {
        "Tab" => Key::Tab,
        "space" | "Space" => Key::Space,
        "Escape" => Key::Escape,
        "Print" => Key::Print,
        "Left" => Key::Left,
        "Right" => Key::Right,
        "Up" => Key::Up,
        "Down" => Key::Down,
        _ => match name.strip_prefix('F').map(str::parse::<u8>) {
            Some(Ok(number)) if (1..=12).contains(&number) => Key::Function(number),
            _ => {
                let mut characters = name.chars();
                match (characters.next(), characters.next()) {
                    (Some(character), None) if character.is_ascii_graphic() => {
                        Key::Character(character.to_ascii_lowercase())
                    }
                    _ => return None,
                }
            }
        },
    })
}

fn key_name(key: Key) -> Option<String> {
    Some(match key {
        Key::Tab => "Tab".to_owned(),
        Key::Space => "space".to_owned(),
        Key::Escape => "Escape".to_owned(),
        Key::Print => "Print".to_owned(),
        Key::Left => "Left".to_owned(),
        Key::Right => "Right".to_owned(),
        Key::Up => "Up".to_owned(),
        Key::Down => "Down".to_owned(),
        Key::Function(number) => format!("F{number}"),
        Key::Character(character) => character.to_string(),
        Key::Tap | Key::AnyFunction | Key::AnyArrow => return None,
    })
}

impl Trigger {
    /// Reads a trigger in the XDG shortcuts form, such as `CTRL+ALT+a` or `LOGO+F5`.
    pub fn parse(text: &str) -> Option<Self> {
        let mut modifiers = Modifiers::default();
        let mut parts: Vec<&str> = text.split('+').collect();
        let key = match parts.pop()? {
            "" if text.ends_with("++") => {
                parts.pop();
                "+"
            }
            last => last,
        };
        for part in parts {
            match part.to_ascii_uppercase().as_str() {
                "CTRL" | "CONTROL" => modifiers.ctrl = true,
                "ALT" => modifiers.alt = true,
                "SHIFT" => modifiers.shift = true,
                "LOGO" | "SUPER" => modifiers.logo = true,
                _ => return None,
            }
        }
        Some(Self {
            modifiers,
            key: key_named(key)?,
        })
    }

    /// The trigger in the XDG shortcuts form.
    pub fn text(self) -> Option<String> {
        let Modifiers {
            shift,
            ctrl,
            alt,
            logo,
        } = self.modifiers;
        let held = [
            (ctrl, "CTRL"),
            (alt, "ALT"),
            (shift, "SHIFT"),
            (logo, "LOGO"),
        ]
        .into_iter()
        .filter(|(held, _)| *held)
        .map(|(_, name)| name.to_owned());
        Some(
            held.chain([key_name(self.key)?])
                .collect::<Vec<_>>()
                .join("+"),
        )
    }

    /// The keys as Settings and the menus show them, such as `Ctrl+Alt+A`.
    pub fn keys(self) -> String {
        chord_keys(self.modifiers, self.key)
    }

    /// Whether an app may take the trigger: it is no system shortcut, and it holds Super or
    /// both Ctrl and Alt, so it never takes a key an app or text field uses.
    pub fn assignable(self) -> bool {
        let Modifiers {
            ctrl, alt, logo, ..
        } = self.modifiers;
        (logo || (ctrl && alt)) && bound(self.modifiers, self.key).is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalShortcut {
    pub app_id: String,
    pub id: String,
    pub description: String,
    pub trigger: Option<Trigger>,
}

/// Gives an app's shortcut the trigger, or takes its trigger away with `None`. Refuses a trigger
/// an app may not have or another shortcut has.
pub fn assign(
    shortcuts: &mut [GlobalShortcut],
    app_id: &str,
    id: &str,
    trigger: Option<Trigger>,
) -> bool {
    let is_it = |shortcut: &GlobalShortcut| shortcut.app_id == app_id && shortcut.id == id;
    if let Some(trigger) = trigger
        && (!trigger.assignable()
            || shortcuts
                .iter()
                .any(|shortcut| shortcut.trigger == Some(trigger) && !is_it(shortcut)))
    {
        return false;
    }
    match shortcuts.iter_mut().find(|shortcut| is_it(shortcut)) {
        Some(shortcut) => {
            shortcut.trigger = trigger;
            true
        }
        None => false,
    }
}

fn clean(field: &str) -> String {
    field.replace(['\t', '\n', '\r'], " ")
}

/// Reads the shortcuts apps bound through the GlobalShortcuts portal, one per line.
pub fn read_global_shortcuts(text: &str) -> Vec<GlobalShortcut> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(FIELD);
            let app_id = fields.next()?.to_owned();
            let id = fields.next()?.to_owned();
            let trigger = fields.next()?;
            let description = fields.next().unwrap_or_default().to_owned();
            Some(GlobalShortcut {
                app_id,
                id,
                description,
                trigger: Trigger::parse(trigger).filter(|trigger| trigger.assignable()),
            })
        })
        .collect()
}

pub fn write_global_shortcuts(shortcuts: &[GlobalShortcut]) -> String {
    shortcuts
        .iter()
        .map(|shortcut| {
            let trigger = shortcut.trigger.and_then(Trigger::text).unwrap_or_default();
            format!(
                "{}{FIELD}{}{FIELD}{trigger}{FIELD}{}\n",
                clean(&shortcut.app_id),
                clean(&shortcut.id),
                clean(&shortcut.description),
            )
        })
        .collect()
}
