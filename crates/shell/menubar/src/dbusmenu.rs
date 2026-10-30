use std::collections::HashMap;

use unbound_toolkit::ui::{Menu, MenuItem};
use zbus::zvariant::{OwnedValue, Value};

use crate::menus::Command;

pub const APP_MENU_PATH: &str = "/org/unbound/MenuBar";

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

fn node(id: i32, properties: &HashMap<String, OwnedValue>, children: Vec<Node>) -> Node {
    let toggled = property::<&str>(properties, "toggle-type").is_some_and(|kind| !kind.is_empty());
    Node {
        id,
        label: without_mnemonics(property::<&str>(properties, "label").unwrap_or_default()),
        enabled: property(properties, "enabled").unwrap_or(true),
        visible: property(properties, "visible").unwrap_or(true),
        separator: property::<&str>(properties, "type") == Some("separator"),
        checked: toggled && property::<i32>(properties, "toggle-state") == Some(1),
        children,
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
                    let action = Command::Item {
                        item: item.to_owned(),
                        id: child.id,
                    }
                    .action();
                    MenuItem::new(&child.label, &action)
                        .enabled(child.enabled)
                        .checked(child.checked)
                } else {
                    MenuItem::submenu(&child.label, menu(child, item)).enabled(child.enabled)
                }
            })
            .collect(),
    )
}
