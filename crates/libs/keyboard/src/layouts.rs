use roxmltree::{Document, Node};

pub const EVDEV_RULES: &str = "/usr/share/X11/xkb/rules/evdev.xml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub code: String,
    pub name: String,
    pub short: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layouts(Vec<Layout>);

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|child| child.has_tag_name(name))
}

fn text<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    child(node, name)?.text().map(str::trim)
}

fn layout(config: Node<'_, '_>, code: String, inherited_short: &str) -> Option<Layout> {
    Some(Layout {
        code,
        name: text(config, "description")?.to_owned(),
        short: text(config, "shortDescription")
            .unwrap_or(inherited_short)
            .to_uppercase(),
    })
}

impl Layouts {
    pub fn parse(xml: &str) -> Self {
        let Ok(document) = Document::parse(xml) else {
            return Self::default();
        };
        let mut layouts = Vec::new();
        for node in document
            .descendants()
            .filter(|node| node.has_tag_name("layout"))
        {
            let Some(config) = child(node, "configItem") else {
                continue;
            };
            let Some(name) = text(config, "name") else {
                continue;
            };
            let short = text(config, "shortDescription").unwrap_or(name).to_owned();
            layouts.extend(layout(config, name.to_owned(), &short));
            let variants = child(node, "variantList")
                .into_iter()
                .flat_map(|list| list.children().filter(|node| node.has_tag_name("variant")))
                .filter_map(|variant| child(variant, "configItem"));
            for variant in variants {
                if let Some(variant_name) = text(variant, "name") {
                    layouts.extend(layout(variant, format!("{name}({variant_name})"), &short));
                }
            }
        }
        layouts.sort_by_cached_key(|layout| layout.name.to_lowercase());
        Self(layouts)
    }

    pub fn load() -> Self {
        std::fs::read_to_string(EVDEV_RULES)
            .map(|xml| Self::parse(&xml))
            .unwrap_or_default()
    }

    pub fn all(&self) -> &[Layout] {
        &self.0
    }

    pub fn find(&self, code: &str) -> Option<&Layout> {
        self.0.iter().find(|layout| layout.code == code)
    }

    pub fn name(&self, code: &str) -> String {
        self.find(code)
            .map_or_else(|| code.to_owned(), |layout| layout.name.clone())
    }

    pub fn short(&self, code: &str) -> String {
        self.find(code).map_or_else(
            || {
                code.split('(')
                    .next()
                    .unwrap_or(code)
                    .chars()
                    .take(3)
                    .collect::<String>()
                    .to_uppercase()
            },
            |layout| layout.short.clone(),
        )
    }

    pub fn search(&self, query: &str) -> Vec<&Layout> {
        let query = query.trim().to_lowercase();
        let mut starting: Vec<&Layout> = Vec::new();
        let mut containing: Vec<&Layout> = Vec::new();
        for layout in &self.0 {
            let name = layout.name.to_lowercase();
            if name.starts_with(&query) || layout.code == query {
                starting.push(layout);
            } else if name.contains(&query) || layout.code.contains(&query) {
                containing.push(layout);
            }
        }
        starting.extend(containing);
        starting
    }
}
