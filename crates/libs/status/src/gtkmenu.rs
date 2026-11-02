use std::collections::HashMap;

use zbus::blocking::Connection;
use zbus::message::{Flags, Message};
use zbus::zvariant::{OwnedValue, Signature, Value};

use crate::dbusmenu::{Node, with_app_row, without_mnemonics};

pub const MENUS_INTERFACE: &str = "org.gtk.Menus";
pub const ACTIONS_INTERFACE: &str = "org.gtk.Actions";
const SECTION: &str = ":section";
const SUBMENU: &str = ":submenu";
const MAX_DEPTH: usize = 16;

type Item = HashMap<String, OwnedValue>;
type MenuId = (u32, u32);
type Described = HashMap<String, (bool, Signature, Vec<OwnedValue>)>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Address {
    bus: String,
    menubar: String,
    application: String,
    window: String,
}

impl Address {
    fn parse(key: &str) -> Option<Self> {
        let mut fields = key.split(' ');
        if fields.next()? != "gtk" {
            return None;
        }
        let address = Self {
            bus: fields.next()?.to_owned(),
            menubar: fields.next()?.to_owned(),
            application: fields.next()?.to_owned(),
            window: fields.next()?.to_owned(),
        };
        fields.next().is_none().then_some(address)
    }

    fn group(&self, prefix: &str) -> &str {
        match prefix {
            "app" => &self.application,
            "win" => &self.window,
            _ => "",
        }
    }
}

pub fn is_gtk(key: &str) -> bool {
    Address::parse(key).is_some()
}

pub fn changed_by(key: &str, sender: &str) -> bool {
    Address::parse(key).is_some_and(|address| address.bus == sender)
}

struct Activation {
    path: String,
    name: String,
    target: Option<OwnedValue>,
}

struct Subscription {
    bus: String,
    menubar: String,
    groups: Vec<u32>,
}

#[derive(Default)]
pub struct Importer {
    key: String,
    subscription: Option<Subscription>,
    activations: HashMap<i32, Activation>,
}

fn link(item: &Item, name: &str) -> Option<MenuId> {
    let Value::Structure(link) = &**item.get(name)? else {
        return None;
    };
    match link.fields() {
        [Value::U32(group), Value::U32(menu)] => Some((*group, *menu)),
        _ => None,
    }
}

fn text<'a>(item: &'a Item, name: &str) -> Option<&'a str> {
    item.get(name)
        .and_then(|value| <&str>::try_from(&**value).ok())
}

fn shortcut(accel: &str) -> Option<String> {
    let mut keys = Vec::new();
    let mut rest = accel;
    while let Some(after) = rest.strip_prefix('<') {
        let (modifier, tail) = after.split_once('>')?;
        keys.push(match modifier.to_ascii_lowercase().as_str() {
            "primary" | "control" | "ctrl" | "ctl" => "Control",
            "shift" => "Shift",
            "alt" | "mod1" => "Alt",
            "super" | "meta" | "hyper" => "Super",
            _ => return None,
        });
        rest = tail;
    }
    let key = match rest {
        "" => return None,
        "plus" | "KP_Add" => "+",
        "minus" | "KP_Subtract" => "-",
        "equal" => "=",
        "comma" => ",",
        "period" => ".",
        "slash" => "/",
        "question" => "?",
        "Page_Up" | "Prior" => "PageUp",
        "Page_Down" | "Next" => "PageDown",
        "BackSpace" => "Backspace",
        key => key,
    };
    keys.push(key);
    Some(keys.join("+"))
}

fn send_end(connection: &Connection, subscription: &Subscription) -> zbus::Result<()> {
    let end = Message::method_call(subscription.menubar.as_str(), "End")?
        .destination(subscription.bus.as_str())?
        .interface(MENUS_INTERFACE)?
        .with_flags(Flags::NoReplyExpected)?
        .build(&(subscription.groups.as_slice(),))?;
    connection.send(&end)
}

fn describe(connection: &Connection, bus: &str, path: &str) -> Described {
    if path.is_empty() {
        return Described::new();
    }
    connection
        .call_method(Some(bus), path, Some(ACTIONS_INTERFACE), "DescribeAll", &())
        .and_then(|reply| reply.body().deserialize::<Described>())
        .unwrap_or_default()
}

struct Builder<'a> {
    menus: &'a HashMap<MenuId, Vec<Item>>,
    address: &'a Address,
    actions: [(&'static str, Described); 2],
    next: i32,
    activations: HashMap<i32, Activation>,
}

impl Builder<'_> {
    fn id(&mut self) -> i32 {
        self.next += 1;
        self.next
    }

    fn separator(&mut self) -> Node {
        Node {
            id: self.id(),
            enabled: true,
            visible: true,
            separator: true,
            ..Node::default()
        }
    }

    fn menu(&mut self, menu: MenuId, depth: usize, top: bool) -> Vec<Node> {
        let Some(items) = self.menus.get(&menu).filter(|_| depth < MAX_DEPTH) else {
            return Vec::new();
        };
        let mut nodes: Vec<Node> = Vec::new();
        let mut after_section = false;
        for item in items {
            let shown = |nodes: &[Node]| nodes.iter().any(|node| node.visible);
            if let Some(section) = link(item, SECTION) {
                let inner = self.menu(section, depth + 1, top);
                if shown(&inner) {
                    if !top && shown(&nodes) {
                        nodes.push(self.separator());
                    }
                    nodes.extend(inner);
                    after_section = true;
                }
                continue;
            }
            if after_section && !top {
                nodes.push(self.separator());
            }
            after_section = false;
            let node = self.item(item, depth);
            nodes.push(node);
        }
        nodes
    }

    fn item(&mut self, item: &Item, depth: usize) -> Node {
        let id = self.id();
        let label = without_mnemonics(text(item, "label").unwrap_or_default());
        if let Some(submenu) = link(item, SUBMENU) {
            return Node {
                id,
                label,
                enabled: true,
                visible: true,
                children: self.menu(submenu, depth + 1, false),
                ..Node::default()
            };
        }
        let target = item.get("target").and_then(|value| value.try_clone().ok());
        let action = text(item, "action").and_then(|action| action.split_once('.'));
        let described = action.and_then(|(prefix, name)| {
            let (_, actions) = self.actions.iter().find(|(known, _)| *known == prefix)?;
            actions.get(name)
        });
        let enabled = described.is_some_and(|(enabled, _, _)| *enabled);
        let checked = match (described.and_then(|(_, _, state)| state.first()), &target) {
            (Some(state), Some(target)) => state == target,
            (Some(state), None) => bool::try_from(&**state).unwrap_or(false),
            _ => false,
        };
        let visible = match text(item, "hidden-when") {
            Some("action-disabled") => enabled,
            Some("action-missing") => described.is_some(),
            Some("macos-menubar") => false,
            _ => true,
        };
        if let Some((prefix, name)) = action
            && described.is_some()
        {
            self.activations.insert(
                id,
                Activation {
                    path: self.address.group(prefix).to_owned(),
                    name: name.to_owned(),
                    target,
                },
            );
        }
        Node {
            id,
            label,
            enabled,
            visible,
            checked,
            shortcut: text(item, "accel").and_then(shortcut),
            ..Node::default()
        }
    }
}

impl Importer {
    fn subscribe(
        &mut self,
        connection: &Connection,
        address: &Address,
    ) -> zbus::Result<HashMap<MenuId, Vec<Item>>> {
        let mut menus = HashMap::new();
        let mut groups: Vec<u32> = Vec::new();
        let mut pending = vec![0];
        while !pending.is_empty() {
            let reply = connection.call_method(
                Some(address.bus.as_str()),
                address.menubar.as_str(),
                Some(MENUS_INTERFACE),
                "Start",
                &(pending.as_slice(),),
            )?;
            groups.append(&mut pending);
            for (group, menu, items) in reply.body().deserialize::<Vec<(u32, u32, Vec<Item>)>>()? {
                for item in &items {
                    for (linked, _) in [SECTION, SUBMENU]
                        .iter()
                        .filter_map(|name| link(item, name))
                    {
                        if !groups.contains(&linked) && !pending.contains(&linked) {
                            pending.push(linked);
                        }
                    }
                }
                menus.insert((group, menu), items);
            }
        }
        let current = Subscription {
            bus: address.bus.clone(),
            menubar: address.menubar.clone(),
            groups,
        };
        if let Some(previous) = self.subscription.replace(current) {
            let _ = send_end(connection, &previous);
        }
        Ok(menus)
    }

    pub fn load(&mut self, connection: &Connection, key: &str) -> zbus::Result<Node> {
        let address = Address::parse(key)
            .ok_or_else(|| zbus::Error::Failure(format!("not a GTK menu: {key}")))?;
        let menus = self.subscribe(connection, &address)?;
        let mut builder = Builder {
            menus: &menus,
            address: &address,
            actions: [
                (
                    "app",
                    describe(connection, &address.bus, &address.application),
                ),
                ("win", describe(connection, &address.bus, &address.window)),
            ],
            next: 0,
            activations: HashMap::new(),
        };
        let children = builder.menu((0, 0), 0, true);
        self.activations = builder.activations;
        self.key = key.to_owned();
        Ok(with_app_row(Node {
            enabled: true,
            visible: true,
            children,
            ..Node::default()
        }))
    }

    pub fn activate(&self, connection: &Connection, key: &str, id: i32) -> zbus::Result<()> {
        let Some(activation) = self.activations.get(&id).filter(|_| self.key == key) else {
            return Ok(());
        };
        let Some(address) = Address::parse(key) else {
            return Ok(());
        };
        let parameter: Vec<Value<'_>> = activation
            .target
            .iter()
            .filter_map(|target| target.try_clone().ok().map(Value::from))
            .collect();
        connection.call_method(
            Some(address.bus.as_str()),
            activation.path.as_str(),
            Some(ACTIONS_INTERFACE),
            "Activate",
            &(
                activation.name.as_str(),
                parameter,
                HashMap::<&str, Value<'_>>::new(),
            ),
        )?;
        Ok(())
    }
}
