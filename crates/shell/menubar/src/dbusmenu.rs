use std::collections::HashMap;

use unbound_app_menu::{SHELL_ACTION_PROPERTY, ShellAction, VERSION_PROPERTY};
use unbound_toolkit::ui::{Menu, MenuItem};
use zbus::zvariant::{OwnedValue, Value};

use crate::menus::Command;

pub use unbound_app_menu::MENU_PATH as APP_MENU_PATH;

pub type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

#[zbus::proxy(
    interface = "com.canonical.dbusmenu",
    gen_async = false,
    blocking_name = "DBusMenu"
)]
pub trait DBusMenu {
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: &[&str],
    ) -> zbus::Result<(u32, Layout)>;

    fn event(&self, id: i32, event_id: &str, data: &Value<'_>, timestamp: u32) -> zbus::Result<()>;

    fn about_to_show(&self, id: i32) -> zbus::Result<bool>;
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Node {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub visible: bool,
    pub separator: bool,
    pub checked: bool,
    pub shortcut: Option<String>,
    pub shell_action: Option<ShellAction>,
    pub version: Option<String>,
    pub children: Vec<Node>,
}

fn plain<'a>(value: &'a Value<'a>) -> &'a Value<'a> {
    match value {
        Value::Value(inner) => plain(inner),
        other => other,
    }
}

fn property<'a, T: TryFrom<&'a Value<'a>>>(
    properties: &'a HashMap<String, OwnedValue>,
    name: &str,
) -> Option<T> {
    properties
        .get(name)
        .and_then(|value| T::try_from(plain(value)).ok())
}

pub fn without_mnemonics(label: &str) -> String {
    let mut plain = String::with_capacity(label.len());
    let mut characters = label.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '_' {
            if characters.peek() == Some(&'_') {
                characters.next();
                plain.push('_');
            }
            continue;
        }
        plain.push(character);
    }
    plain
}

fn node_of_value(value: &Value<'_>) -> Option<Node> {
    match value {
        Value::Value(inner) => node_of_value(inner),
        Value::Structure(structure) => {
            let [Value::I32(id), properties, Value::Array(children)] = structure.fields() else {
                return None;
            };
            let properties =
                HashMap::<String, OwnedValue>::try_from(properties.try_clone().ok()?).ok()?;
            Some(node(
                *id,
                &properties,
                children.iter().filter_map(node_of_value).collect(),
            ))
        }
        _ => None,
    }
}

fn shortcut(properties: &HashMap<String, OwnedValue>) -> Option<String> {
    let Value::Array(chords) = plain(properties.get("shortcut")?) else {
        return None;
    };
    let Value::Array(keys) = plain(chords.first()?) else {
        return None;
    };
    let keys: Vec<&str> = keys
        .iter()
        .filter_map(|key| <&str>::try_from(plain(key)).ok())
        .collect();
    (!keys.is_empty()).then(|| keys.join("+"))
}

fn node(id: i32, properties: &HashMap<String, OwnedValue>, children: Vec<Node>) -> Node {
    let toggled = property::<&str>(properties, "toggle-type").is_some_and(|kind| !kind.is_empty());
    Node {
        id,
        label: without_mnemonics(property::<&str>(properties, "label").unwrap_or_default()),
        enabled: property(properties, "enabled").unwrap_or(true),
        visible: property(properties, "visible").unwrap_or(true),
        separator: property::<&str>(properties, "type") == Some("separator"),
        checked: toggled && property::<i32>(properties, "toggle-state") == Some(1),
        shortcut: shortcut(properties),
        shell_action: property::<&str>(properties, SHELL_ACTION_PROPERTY)
            .and_then(ShellAction::parse),
        version: property::<&str>(properties, VERSION_PROPERTY).map(str::to_owned),
        children,
    }
}

fn command(node: &Node, item: &str) -> Command {
    match node.shell_action {
        Some(ShellAction::About) => Command::AboutApp(node.version.clone().unwrap_or_default()),
        Some(ShellAction::Hide) => Command::HideApp,
        Some(ShellAction::HideOthers) => Command::HideOthers,
        Some(ShellAction::ShowAll) => Command::ShowAll,
        None => Command::Item {
            item: item.to_owned(),
            id: node.id,
        },
    }
}

pub fn tree((id, properties, children): Layout) -> Node {
    node(
        id,
        &properties,
        children
            .iter()
            .filter_map(|child| node_of_value(child))
            .collect(),
    )
}

pub fn menu(node: &Node, item: &str) -> Menu {
    Menu::new(
        node.children
            .iter()
            .filter(|child| child.visible)
            .map(|child| {
                if child.separator {
                    return MenuItem::separator();
                }
                if child.children.is_empty() {
                    let entry = MenuItem::new(&child.label, &command(child, item).action())
                        .enabled(child.enabled)
                        .checked(child.checked);
                    match &child.shortcut {
                        Some(shortcut) => entry.shortcut(shortcut),
                        None => entry,
                    }
                } else {
                    MenuItem::submenu(&child.label, menu(child, item)).enabled(child.enabled)
                }
            })
            .collect(),
    )
}
