use bande_graphics::Rect;

const MAX_FOUND: usize = 12;
const PATH_SEPARATOR: &str = " › ";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Menu {
    pub items: Vec<MenuItem>,
    pub search: Option<Search>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Search {
    pub hint: String,
    pub nothing_found: String,
    pub query: String,
}

impl Menu {
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self {
            items,
            search: None,
        }
    }

    pub fn searchable(self, hint: &str, nothing_found: &str) -> Self {
        Self {
            search: Some(Search {
                hint: hint.to_owned(),
                nothing_found: nothing_found.to_owned(),
                query: String::new(),
            }),
            ..self
        }
    }

    pub fn matching(&self, query: &str) -> Vec<MenuItem> {
        let mut items = Vec::new();
        collect_found(&self.items, &[], &query.to_lowercase(), &mut items);
        items
    }

    pub fn found(&self, query: &str) -> Menu {
        let query = query.to_lowercase();
        let mut items = self.matching(&query);
        let search = self.search.clone().map(|search| Search {
            query: query.clone(),
            ..search
        });
        if items.is_empty() {
            let nothing = search
                .as_ref()
                .map(|search| search.nothing_found.clone())
                .unwrap_or_default();
            items.push(MenuItem::new(&nothing, "").enabled(false));
        }
        items.truncate(MAX_FOUND);
        Menu { items, search }
    }
}

pub(super) fn collect_found(
    items: &[MenuItem],
    path: &[&str],
    query: &str,
    found: &mut Vec<MenuItem>,
) {
    for item in items {
        if item.separator {
            continue;
        }
        match &item.submenu {
            Some(submenu) if item.enabled => {
                let mut inner = path.to_vec();
                inner.push(&item.title);
                collect_found(&submenu.items, &inner, query, found);
            }
            Some(_) => {}
            None if item.enabled && item.title.to_lowercase().contains(query) => {
                let mut title = path.join(PATH_SEPARATOR);
                if !title.is_empty() {
                    title.push_str(PATH_SEPARATOR);
                }
                title.push_str(&item.title);
                found.push(MenuItem {
                    title,
                    strong: false,
                    alternate: None,
                    shift_alternate: None,
                    ..item.clone()
                });
            }
            None => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub title: String,
    pub action: String,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub checked: bool,
    pub strong: bool,
    pub submenu: Option<Menu>,
    pub separator: bool,
    pub alternate: Option<Box<MenuItem>>,
    pub shift_alternate: Option<Box<MenuItem>>,
    pub layout: Option<Rect>,
}

pub fn placement_layout(action: &str) -> Option<Rect> {
    let half = 0.5;
    Some(match action {
        "place-left" => Rect::new(0.0, 0.0, half, 1.0),
        "place-right" => Rect::new(half, 0.0, half, 1.0),
        "place-top-left" => Rect::new(0.0, 0.0, half, half),
        "place-top-right" => Rect::new(half, 0.0, half, half),
        "place-bottom-left" => Rect::new(0.0, half, half, half),
        "place-bottom-right" => Rect::new(half, half, half, half),
        "place-fill" => Rect::new(0.0, 0.0, 1.0, 1.0),
        _ => return None,
    })
}

impl MenuItem {
    pub fn new(title: &str, action: &str) -> Self {
        Self {
            title: title.to_owned(),
            action: action.to_owned(),
            shortcut: None,
            enabled: true,
            checked: false,
            strong: false,
            submenu: None,
            separator: false,
            alternate: None,
            shift_alternate: None,
            layout: None,
        }
    }

    pub fn separator() -> Self {
        Self {
            separator: true,
            enabled: false,
            ..Self::new("", "")
        }
    }

    pub fn submenu(title: &str, menu: Menu) -> Self {
        Self {
            submenu: Some(menu),
            ..Self::new(title, "")
        }
    }

    pub fn shortcut(self, shortcut: &str) -> Self {
        Self {
            shortcut: Some(shortcut.to_owned()),
            ..self
        }
    }

    pub fn layout(self, layout: Option<Rect>) -> Self {
        Self { layout, ..self }
    }

    pub fn enabled(self, enabled: bool) -> Self {
        Self { enabled, ..self }
    }

    pub fn checked(self, checked: bool) -> Self {
        Self { checked, ..self }
    }

    pub fn strong(self) -> Self {
        Self {
            strong: true,
            ..self
        }
    }

    pub fn alternate(self, alternate: MenuItem) -> Self {
        Self {
            alternate: Some(Box::new(alternate)),
            ..self
        }
    }

    pub fn shift_alternate(self, alternate: MenuItem) -> Self {
        Self {
            shift_alternate: Some(Box::new(alternate)),
            ..self
        }
    }

    pub(super) fn selectable(&self) -> bool {
        self.enabled && !self.separator
    }
}
